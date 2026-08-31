//! Fontsource-backed font discovery and Photonic-local font installation.
//!
//! Network and disk work stays off the UI thread. Downloads are constrained to
//! Fontsource's jsDelivr namespace, size-limited, parsed as fonts before they are
//! persisted, and finalized atomically so an interrupted install is ignored.

use photonic_core::node::NodeId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

const CATALOG_URL: &str = "https://api.fontsource.org/v1/fonts";
const FONT_DETAILS_URL: &str = "https://api.fontsource.org/v1/fonts/";
const FONT_CDN_PREFIX: &str = "https://cdn.jsdelivr.net/fontsource/fonts/";
const MAX_CATALOG_BYTES: u64 = 16 * 1024 * 1024;
const MAX_FONT_BYTES: u64 = 64 * 1024 * 1024;
const MAX_FAMILY_BYTES: usize = 256 * 1024 * 1024;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogFont {
    pub id: String,
    pub family: String,
    #[serde(default)]
    pub subsets: Vec<String>,
    #[serde(default)]
    pub weights: Vec<u16>,
    #[serde(default)]
    pub styles: Vec<String>,
    #[serde(default)]
    pub def_subset: String,
    #[serde(default)]
    pub variable: bool,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub license: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FontDetails {
    id: String,
    family: String,
    #[serde(default)]
    def_subset: String,
    #[serde(default)]
    license: String,
    #[serde(default)]
    source: String,
    #[serde(default)]
    variants: BTreeMap<String, BTreeMap<String, BTreeMap<String, VariantFile>>>,
}

#[derive(Debug, Clone, Deserialize)]
struct VariantFile {
    url: VariantUrls,
}

#[derive(Debug, Clone, Deserialize)]
struct VariantUrls {
    ttf: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledFont {
    pub id: String,
    pub family: String,
    pub subset: String,
    pub license: String,
    pub source: String,
    pub files: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FontLibraryTab {
    #[default]
    Installed,
    Recent,
    Library,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogStatus {
    NotLoaded,
    Loading,
    Ready,
    Error(String),
}

pub struct FontInstallResult {
    pub node_id: Option<NodeId>,
    pub font: InstalledFont,
    pub paths: Vec<PathBuf>,
}

pub struct FontPreview {
    pub token: String,
    pub id: String,
    pub bytes: Vec<u8>,
}

struct InstallRequest {
    node_id: Option<NodeId>,
    id: String,
    family: String,
    subset: String,
}

pub struct FontLibraryState {
    pub tab: FontLibraryTab,
    pub search: String,
    pub preview_text: String,
    pub selected_id: Option<String>,
    pub selected_subset: String,
    pub catalog: Vec<CatalogFont>,
    pub catalog_status: CatalogStatus,
    pub installed_families: Vec<String>,
    pub recent_families: Vec<String>,
    pub managed_fonts: Vec<InstalledFont>,
    pub installing_id: Option<String>,
    pub preview_loading_token: Option<String>,
    pub preview_ready_token: Option<String>,
    pub preview_font_key: Option<String>,
    pub preview_error: Option<String>,
    pub picker_open: bool,
    installed_synced: bool,
    catalog_rx: Option<Receiver<Result<Vec<CatalogFont>, String>>>,
    install_rx: Option<Receiver<Result<FontInstallResult, String>>>,
    preview_rx: Option<Receiver<Result<FontPreview, String>>>,
}

impl Default for FontLibraryState {
    fn default() -> Self {
        Self {
            tab: FontLibraryTab::Installed,
            search: String::new(),
            preview_text: "The quick brown fox jumps over the lazy dog".into(),
            selected_id: None,
            selected_subset: String::new(),
            catalog: Vec::new(),
            catalog_status: CatalogStatus::NotLoaded,
            installed_families: Vec::new(),
            recent_families: Vec::new(),
            managed_fonts: scan_installed_fonts(&photonic_render::photonic_font_cache_dir()),
            installing_id: None,
            preview_loading_token: None,
            preview_ready_token: None,
            preview_font_key: None,
            preview_error: None,
            picker_open: false,
            installed_synced: false,
            catalog_rx: None,
            install_rx: None,
            preview_rx: None,
        }
    }
}

impl FontLibraryState {
    pub fn set_installed_families(&mut self, mut families: Vec<String>) {
        families.sort_by_key(|name| name.to_lowercase());
        families.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
        self.installed_families = families;
        self.installed_synced = true;
    }

    pub fn sync_installed_families(&mut self, renderer: &photonic_render::PhotonicRenderer) {
        if !self.installed_synced {
            self.set_installed_families(renderer.font_families());
        }
    }

    pub fn ensure_catalog(&mut self) {
        if !matches!(self.catalog_status, CatalogStatus::NotLoaded) {
            return;
        }
        self.catalog_status = CatalogStatus::Loading;
        let (tx, rx) = mpsc::channel();
        self.catalog_rx = Some(rx);
        std::thread::spawn(move || {
            let result = fetch_json(CATALOG_URL, MAX_CATALOG_BYTES)
                .and_then(|json| serde_json::from_str(&json).map_err(|error| error.to_string()));
            let _ = tx.send(result);
        });
    }

    pub fn retry_catalog(&mut self) {
        self.catalog_rx = None;
        self.catalog_status = CatalogStatus::NotLoaded;
        self.ensure_catalog();
    }

    pub fn poll_catalog(&mut self) {
        let Some(rx) = self.catalog_rx.take() else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok(mut catalog)) => {
                catalog.sort_by_key(|font| font.family.to_lowercase());
                self.catalog = catalog;
                self.catalog_status = CatalogStatus::Ready;
            }
            Ok(Err(error)) => self.catalog_status = CatalogStatus::Error(error),
            Err(mpsc::TryRecvError::Disconnected) => {
                self.catalog_status = CatalogStatus::Error("font catalog request stopped".into())
            }
            Err(mpsc::TryRecvError::Empty) => self.catalog_rx = Some(rx),
        }
    }

    pub fn start_install(
        &mut self,
        node_id: Option<NodeId>,
        id: String,
        family: String,
        subset: String,
    ) -> Result<(), String> {
        if self.install_rx.is_some() {
            return Err("another font is already being installed".into());
        }
        validate_slug(&id)?;
        validate_slug(&subset)?;
        let request = InstallRequest {
            node_id,
            id: id.clone(),
            family,
            subset,
        };
        let (tx, rx) = mpsc::channel();
        self.install_rx = Some(rx);
        self.installing_id = Some(id);
        std::thread::spawn(move || {
            let result = install_font(request, &photonic_render::photonic_font_cache_dir());
            let _ = tx.send(result);
        });
        Ok(())
    }

    pub fn start_preview(&mut self, id: String, family: String, subset: String) {
        let token = format!("{id}:{subset}");
        if self.preview_loading_token.as_deref() == Some(&token)
            || self.preview_ready_token.as_deref() == Some(&token)
        {
            return;
        }
        if validate_slug(&id).is_err() || validate_slug(&subset).is_err() {
            self.preview_error = Some("font preview metadata was invalid".into());
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.preview_rx = Some(rx);
        self.preview_loading_token = Some(token.clone());
        self.preview_error = None;
        std::thread::spawn(move || {
            let result = fetch_font_preview(&id, &family, &subset).map(|bytes| FontPreview {
                token,
                id,
                bytes,
            });
            let _ = tx.send(result);
        });
    }

    pub fn poll_install(&mut self) -> Option<Result<FontInstallResult, String>> {
        let rx = self.install_rx.take()?;
        match rx.try_recv() {
            Ok(result) => {
                self.installing_id = None;
                Some(result)
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.installing_id = None;
                Some(Err("font installation stopped unexpectedly".into()))
            }
            Err(mpsc::TryRecvError::Empty) => {
                self.install_rx = Some(rx);
                None
            }
        }
    }

    pub fn poll_preview(&mut self) -> Option<Result<FontPreview, String>> {
        let rx = self.preview_rx.take()?;
        match rx.try_recv() {
            Ok(result) => {
                self.preview_loading_token = None;
                Some(result)
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.preview_loading_token = None;
                Some(Err("font preview request stopped unexpectedly".into()))
            }
            Err(mpsc::TryRecvError::Empty) => {
                self.preview_rx = Some(rx);
                None
            }
        }
    }

    pub fn set_preview_ready(&mut self, token: String, font_key: String) {
        self.preview_ready_token = Some(token);
        self.preview_font_key = Some(font_key);
        self.preview_error = None;
    }

    pub fn is_busy(&self) -> bool {
        self.catalog_rx.is_some() || self.install_rx.is_some() || self.preview_rx.is_some()
    }

    pub fn refresh_managed_fonts(&mut self) {
        self.managed_fonts = scan_installed_fonts(&photonic_render::photonic_font_cache_dir());
    }

    pub fn is_family_managed(&self, family: &str) -> bool {
        self.managed_fonts
            .iter()
            .any(|font| font.family.eq_ignore_ascii_case(family))
    }

    pub fn is_subset_managed(&self, id: &str, subset: &str) -> bool {
        self.managed_fonts
            .iter()
            .any(|font| font.id == id && font.subset == subset)
    }

    pub fn record_recent_family(&mut self, family: &str) {
        self.recent_families
            .retain(|existing| !existing.eq_ignore_ascii_case(family));
        self.recent_families.insert(0, family.to_string());
        self.recent_families.truncate(12);
    }
}

fn http_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(30))
        .timeout_write(Duration::from_secs(10))
        .build()
}

fn fetch_json(url: &str, max_bytes: u64) -> Result<String, String> {
    let response = http_agent()
        .get(url)
        .call()
        .map_err(|error| format!("request failed: {error}"))?;
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(max_bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("reading response failed: {error}"))?;
    if bytes.len() as u64 > max_bytes {
        return Err("response exceeded the safe size limit".into());
    }
    String::from_utf8(bytes).map_err(|_| "response was not valid UTF-8".into())
}

fn download_font(url: &str) -> Result<Vec<u8>, String> {
    validate_font_url(url)?;
    let response = http_agent()
        .get(url)
        .call()
        .map_err(|error| format!("font download failed: {error}"))?;
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(MAX_FONT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("reading font failed: {error}"))?;
    if bytes.len() as u64 > MAX_FONT_BYTES {
        return Err("font file exceeded the 64 MB safety limit".into());
    }
    if bytes.len() < 512 {
        return Err("download did not contain a usable font file".into());
    }

    let source = glyphon::cosmic_text::fontdb::Source::Binary(std::sync::Arc::new(bytes.clone()));
    let mut database = glyphon::cosmic_text::fontdb::Database::new();
    if database.load_font_source(source).is_empty() {
        return Err("downloaded data was not a valid TTF/OTF font".into());
    }
    Ok(bytes)
}

fn fetch_font_preview(id: &str, family: &str, subset: &str) -> Result<Vec<u8>, String> {
    let local_dir = photonic_render::photonic_font_cache_dir()
        .join(id)
        .join(subset);
    if let Ok(json) = std::fs::read_to_string(local_dir.join("manifest.json")) {
        if let Ok(font) = serde_json::from_str::<InstalledFont>(&json) {
            if manifest_is_safe(&font, id, family, subset) {
                if let Some(path) = font.files.first().map(|file| local_dir.join(file)) {
                    if let Ok(bytes) = std::fs::read(path) {
                        let source = glyphon::cosmic_text::fontdb::Source::Binary(
                            std::sync::Arc::new(bytes.clone()),
                        );
                        let mut database = glyphon::cosmic_text::fontdb::Database::new();
                        if !database.load_font_source(source).is_empty() {
                            return Ok(bytes);
                        }
                    }
                }
            }
        }
    }

    let details_url = format!("{FONT_DETAILS_URL}{id}");
    let details: FontDetails = serde_json::from_str(&fetch_json(&details_url, MAX_CATALOG_BYTES)?)
        .map_err(|error| format!("invalid font metadata: {error}"))?;
    if details.id != id || !details.family.eq_ignore_ascii_case(family) {
        return Err("font preview metadata did not match the selected family".into());
    }
    if !is_supported_open_license(&details.license) {
        return Err("font preview did not carry an approved open-font license".into());
    }
    let plan = download_plan(&details, subset)?;
    let (_, _, url) = plan
        .iter()
        .find(|(weight, style, _)| weight == "400" && style == "normal")
        .or_else(|| plan.first())
        .ok_or_else(|| "font has no previewable TTF face".to_string())?;
    download_font(url)
}

fn install_font(request: InstallRequest, base: &Path) -> Result<FontInstallResult, String> {
    validate_slug(&request.id)?;
    validate_slug(&request.subset)?;
    let final_dir = base.join(&request.id).join(&request.subset);
    let manifest_path = final_dir.join("manifest.json");
    if let Ok(json) = std::fs::read_to_string(&manifest_path) {
        if let Ok(font) = serde_json::from_str::<InstalledFont>(&json) {
            let paths: Vec<PathBuf> = font.files.iter().map(|file| final_dir.join(file)).collect();
            if manifest_is_safe(&font, &request.id, &request.family, &request.subset)
                && !paths.is_empty()
                && paths.iter().all(|path| {
                    std::fs::metadata(path)
                        .map(|metadata| metadata.is_file() && metadata.len() >= 512)
                        .unwrap_or(false)
                })
            {
                return Ok(FontInstallResult {
                    node_id: request.node_id,
                    font,
                    paths,
                });
            }
        }
    }

    let details_url = format!("{FONT_DETAILS_URL}{}", request.id);
    let details: FontDetails = serde_json::from_str(&fetch_json(&details_url, MAX_CATALOG_BYTES)?)
        .map_err(|error| format!("invalid font metadata: {error}"))?;
    if details.id != request.id || !details.family.eq_ignore_ascii_case(&request.family) {
        return Err("font metadata did not match the requested family".into());
    }
    if details.family.is_empty()
        || details.family.len() > 200
        || details.family.chars().any(char::is_control)
    {
        return Err("font metadata contained an invalid family name".into());
    }
    if !is_supported_open_license(&details.license) {
        return Err(format!(
            "{} is not on Photonic's approved open-font license list",
            details.license
        ));
    }

    let subset = if request.subset.is_empty() {
        details.def_subset.clone()
    } else {
        request.subset.clone()
    };
    validate_slug(&subset)?;
    let plan = download_plan(&details, &subset)?;

    std::fs::create_dir_all(base).map_err(|error| format!("creating font cache: {error}"))?;
    // Stage beside (not inside) the recursively scanned font cache. A crash
    // during download can therefore never expose a half-installed face to
    // fontdb on the next launch.
    let staging_root = base.parent().unwrap_or(base).join(".photonic-font-staging");
    std::fs::create_dir_all(&staging_root)
        .map_err(|error| format!("creating font staging root: {error}"))?;
    let staging = staging_root.join(format!(
        ".{}-{}-{}.part",
        request.id,
        subset,
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&staging)
        .map_err(|error| format!("creating font staging directory: {error}"))?;

    let result = (|| {
        let mut files = Vec::with_capacity(plan.len());
        let mut total_bytes = 0usize;
        for (index, (weight, style, url)) in plan.iter().enumerate() {
            let bytes = download_font(url)?;
            total_bytes = total_bytes.saturating_add(bytes.len());
            if total_bytes > MAX_FAMILY_BYTES {
                return Err("font family exceeded the 256 MB safety limit".into());
            }
            let filename = format!(
                "{index:03}-{}-{}.ttf",
                safe_component(weight),
                safe_component(style)
            );
            let path = staging.join(&filename);
            std::fs::write(&path, bytes)
                .map_err(|error| format!("writing {}: {error}", path.display()))?;
            files.push(filename);
        }

        let font = InstalledFont {
            id: details.id,
            family: details.family,
            subset,
            license: details.license,
            source: details.source,
            files,
        };
        let manifest = serde_json::to_vec_pretty(&font)
            .map_err(|error| format!("serializing font manifest: {error}"))?;
        std::fs::write(staging.join("manifest.json"), manifest)
            .map_err(|error| format!("writing font manifest: {error}"))?;

        if final_dir.exists() {
            std::fs::remove_dir_all(&final_dir)
                .map_err(|error| format!("replacing prior font install: {error}"))?;
        }
        if let Some(parent) = final_dir.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("creating family cache: {error}"))?;
        }
        std::fs::rename(&staging, &final_dir)
            .map_err(|error| format!("finalizing font install: {error}"))?;
        let paths = font.files.iter().map(|file| final_dir.join(file)).collect();
        Ok(FontInstallResult {
            node_id: request.node_id,
            font,
            paths,
        })
    })();

    if result.is_err() {
        let _ = std::fs::remove_dir_all(&staging);
    }
    let _ = std::fs::remove_dir(&staging_root);
    result
}

fn download_plan(
    details: &FontDetails,
    subset: &str,
) -> Result<Vec<(String, String, String)>, String> {
    let mut plan = Vec::new();
    for (weight, styles) in &details.variants {
        for (style, subsets) in styles {
            if let Some(file) = subsets.get(subset) {
                validate_font_url(&file.url.ttf)?;
                plan.push((weight.clone(), style.clone(), file.url.ttf.clone()));
            }
        }
    }
    if plan.is_empty() {
        return Err(format!(
            "no TTF files are available for the {subset} character set"
        ));
    }
    Ok(plan)
}

fn validate_font_url(url: &str) -> Result<(), String> {
    let Some(path) = url.strip_prefix(FONT_CDN_PREFIX) else {
        return Err("font download URL was outside the trusted Fontsource CDN".into());
    };
    if path.is_empty() || path.contains("..") || path.contains('\\') || path.contains('#') {
        return Err("font download URL contained an unsafe path".into());
    }
    Ok(())
}

fn validate_slug(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 100
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err("font identifier contained unsupported characters".into());
    }
    Ok(())
}

pub fn is_supported_open_license(license: &str) -> bool {
    matches!(
        license.to_ascii_lowercase().as_str(),
        "ofl-1.1" | "apache-2.0" | "ufl-1.0" | "cc0-1.0" | "unlicense" | "mit"
    )
}

fn safe_component(value: &str) -> String {
    let value: String = value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '-')
        .take(32)
        .collect();
    if value.is_empty() {
        "face".into()
    } else {
        value
    }
}

fn manifest_is_safe(
    font: &InstalledFont,
    expected_id: &str,
    expected_family: &str,
    expected_subset: &str,
) -> bool {
    font.id == expected_id
        && font.subset == expected_subset
        && font.family.eq_ignore_ascii_case(expected_family)
        && !font.family.is_empty()
        && font.family.len() <= 200
        && !font.family.chars().any(char::is_control)
        && validate_slug(&font.id).is_ok()
        && validate_slug(&font.subset).is_ok()
        && is_supported_open_license(&font.license)
        && !font.files.is_empty()
        && font.files.iter().all(|file| {
            let path = Path::new(file);
            path.components().count() == 1
                && path.extension().and_then(|ext| ext.to_str()) == Some("ttf")
                && !file.chars().any(char::is_control)
        })
}

fn scan_installed_fonts(base: &Path) -> Vec<InstalledFont> {
    let mut fonts = Vec::new();
    let Ok(families) = std::fs::read_dir(base) else {
        return fonts;
    };
    for family in families.flatten().filter(|entry| entry.path().is_dir()) {
        let Ok(subsets) = std::fs::read_dir(family.path()) else {
            continue;
        };
        for subset in subsets.flatten().filter(|entry| entry.path().is_dir()) {
            let expected_id = family.file_name().to_string_lossy().into_owned();
            let expected_subset = subset.file_name().to_string_lossy().into_owned();
            let path = subset.path().join("manifest.json");
            if let Ok(json) = std::fs::read_to_string(path) {
                if let Ok(font) = serde_json::from_str::<InstalledFont>(&json) {
                    let expected_family = font.family.clone();
                    if manifest_is_safe(&font, &expected_id, &expected_family, &expected_subset) {
                        fonts.push(font);
                    }
                }
            }
        }
    }
    fonts.sort_by_key(|font: &InstalledFont| (font.family.to_lowercase(), font.subset.clone()));
    fonts
}

#[cfg(test)]
mod tests {
    use super::*;

    const DETAILS: &str = r#"{
      "id":"test-sans","family":"Test Sans","defSubset":"latin","license":"OFL-1.1",
      "source":"https://example.invalid/source","variants":{
        "400":{"normal":{"latin":{"url":{"ttf":"https://cdn.jsdelivr.net/fontsource/fonts/test-sans@latest/latin-400-normal.ttf"}}}},
        "700":{"italic":{"latin":{"url":{"ttf":"https://cdn.jsdelivr.net/fontsource/fonts/test-sans@latest/latin-700-italic.ttf"}}}}
      }
    }"#;

    #[test]
    fn parses_catalog_fields_used_by_the_picker() {
        let json = r#"[{"id":"abel","family":"Abel","subsets":["latin"],"weights":[400],"styles":["normal"],"defSubset":"latin","variable":false,"category":"sans-serif","license":"OFL-1.1"}]"#;
        let fonts: Vec<CatalogFont> = serde_json::from_str(json).unwrap();
        assert_eq!(fonts[0].family, "Abel");
        assert_eq!(fonts[0].def_subset, "latin");
        assert_eq!(fonts[0].license, "OFL-1.1");
    }

    #[test]
    fn plans_every_available_style_for_the_selected_subset() {
        let details: FontDetails = serde_json::from_str(DETAILS).unwrap();
        let plan = download_plan(&details, "latin").unwrap();
        assert_eq!(plan.len(), 2);
        assert_eq!(plan[0].0, "400");
        assert_eq!(plan[1].1, "italic");
    }

    #[test]
    fn rejects_untrusted_download_hosts_and_path_traversal() {
        assert!(validate_font_url("https://example.com/font.ttf").is_err());
        assert!(
            validate_font_url("https://cdn.jsdelivr.net/fontsource/fonts/../../secret.ttf")
                .is_err()
        );
        assert!(validate_font_url(
            "https://cdn.jsdelivr.net/fontsource/fonts/abel@latest/latin-400-normal.ttf"
        )
        .is_ok());
    }

    #[test]
    fn only_known_open_font_licenses_are_accepted() {
        assert!(is_supported_open_license("OFL-1.1"));
        assert!(is_supported_open_license("Apache-2.0"));
        assert!(is_supported_open_license("mit"));
        assert!(!is_supported_open_license("Commercial-EULA"));
        assert!(!is_supported_open_license(""));
    }

    #[test]
    fn recently_used_fonts_are_deduplicated_and_moved_to_the_front() {
        let mut state = FontLibraryState::default();
        state.record_recent_family("Inter");
        state.record_recent_family("Abel");
        state.record_recent_family("inter");
        assert_eq!(state.recent_families, vec!["inter", "Abel"]);
    }

    #[test]
    fn installed_manifests_are_discovered_without_partial_staging_dirs() {
        let root =
            std::env::temp_dir().join(format!("photonic-font-test-{}", uuid::Uuid::new_v4()));
        let installed = root.join("test-sans").join("latin");
        let staging = root.join(".unfinished.part");
        std::fs::create_dir_all(&installed).unwrap();
        std::fs::create_dir_all(&staging).unwrap();
        let manifest = InstalledFont {
            id: "test-sans".into(),
            family: "Test Sans".into(),
            subset: "latin".into(),
            license: "OFL-1.1".into(),
            source: "https://example.invalid".into(),
            files: vec!["000-400-normal.ttf".into()],
        };
        std::fs::write(
            installed.join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        let unsafe_dir = root.join("unsafe-font").join("latin");
        std::fs::create_dir_all(&unsafe_dir).unwrap();
        let mut unsafe_manifest = manifest.clone();
        unsafe_manifest.id = "unsafe-font".into();
        unsafe_manifest.family = "Unsafe Font".into();
        unsafe_manifest.files = vec!["../../outside.ttf".into()];
        std::fs::write(
            unsafe_dir.join("manifest.json"),
            serde_json::to_vec(&unsafe_manifest).unwrap(),
        )
        .unwrap();

        let found = scan_installed_fonts(&root);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].family, "Test Sans");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[ignore = "requires the live Fontsource API and CDN"]
    fn live_fontsource_family_installs_into_an_isolated_cache() {
        let root = std::env::temp_dir().join(format!(
            "photonic-font-download-test-{}",
            uuid::Uuid::new_v4()
        ));
        let result = install_font(
            InstallRequest {
                node_id: Some(uuid::Uuid::new_v4()),
                id: "abel".into(),
                family: "Abel".into(),
                subset: "latin".into(),
            },
            &root,
        )
        .unwrap();
        assert_eq!(result.font.family, "Abel");
        assert!(!result.paths.is_empty());
        assert!(result.paths.iter().all(|path| path.is_file()));
        assert_eq!(scan_installed_fonts(&root).len(), 1);

        let previous_cache = std::env::var_os("PHOTONIC_FONT_CACHE_DIR");
        std::env::set_var("PHOTONIC_FONT_CACHE_DIR", &root);
        let font_system = photonic_render::new_font_system();
        assert!(font_system.db().faces().any(|face| {
            face.families
                .iter()
                .any(|(family, _)| family.eq_ignore_ascii_case("Abel"))
        }));
        let preview = fetch_font_preview("abel", "Abel", "latin").unwrap();
        assert!(preview.len() >= 512);
        match previous_cache {
            Some(path) => std::env::set_var("PHOTONIC_FONT_CACHE_DIR", path),
            None => std::env::remove_var("PHOTONIC_FONT_CACHE_DIR"),
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
