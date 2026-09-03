//! Fontsource-backed font discovery and Photonic-local font installation.
//!
//! Network and disk work stays off the UI thread. Downloads are constrained to
//! Fontsource's jsDelivr namespace, size-limited, parsed as fonts before they are
//! persisted, and finalized with a recoverable directory replacement so an
//! interrupted install keeps the previous cache usable. See
//! `docs/font-library-provenance.md` for the upstream trust and cache policy.

use photonic_core::node::NodeId;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
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
const FONT_REPLACEMENT_BACKUP_SUFFIX: &str = ".photonic-font-backup";

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
    npm_version: String,
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
    /// Exact Fontsource package version used for this install.
    #[serde(default)]
    pub fontsource_version: String,
    /// SHA-256 digest for each file in `files`, keyed by its relative filename.
    #[serde(default)]
    pub artifact_sha256: BTreeMap<String, String>,
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

fn font_bytes_are_usable(bytes: &[u8]) -> bool {
    if bytes.len() < 512 || bytes.len() as u64 > MAX_FONT_BYTES {
        return false;
    }
    let source = glyphon::cosmic_text::fontdb::Source::Binary(std::sync::Arc::new(bytes.to_vec()));
    let mut database = glyphon::cosmic_text::fontdb::Database::new();
    !database.load_font_source(source).is_empty()
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn font_file_is_usable(path: &Path, expected_sha256: &str) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() || metadata.len() < 512 || metadata.len() > MAX_FONT_BYTES {
        return false;
    }
    let Ok(bytes) = std::fs::read(path) else {
        return false;
    };
    font_bytes_are_usable(&bytes) && sha256_hex(&bytes) == expected_sha256
}

fn font_cache_is_usable(font: &InstalledFont, base: &Path) -> bool {
    !font.files.is_empty()
        && font.files.len() == font.artifact_sha256.len()
        && font.files.iter().all(|file| {
            let Some(expected_sha256) = font.artifact_sha256.get(file) else {
                return false;
            };
            font_file_is_usable(&base.join(file), expected_sha256)
        })
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

    if !font_bytes_are_usable(&bytes) {
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
            if manifest_is_safe(&font, id, family, subset)
                && font_cache_is_usable(&font, &local_dir)
            {
                if let Some(path) = font.files.first().map(|file| local_dir.join(file)) {
                    if let Ok(bytes) = std::fs::read(path) {
                        return Ok(bytes);
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

fn replacement_backup_path(final_dir: &Path) -> Result<PathBuf, String> {
    let file_name = final_dir
        .file_name()
        .ok_or_else(|| "font cache path had no final directory name".to_string())?;
    let mut backup_name = std::ffi::OsString::from(".");
    backup_name.push(file_name);
    backup_name.push(FONT_REPLACEMENT_BACKUP_SUFFIX);
    Ok(final_dir.with_file_name(backup_name))
}

fn remove_path_if_exists(path: &Path) -> std::io::Result<()> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if metadata.file_type().is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    }
}

fn rename_font_path(from: &Path, to: &Path) -> std::io::Result<()> {
    // The replacement protocol moves onto paths that are guaranteed to be
    // absent, so this works on both Unix and Windows without deleting a
    // non-empty destination directory first.
    std::fs::rename(from, to)
}

/// Recover the one backup associated with a font subset. A backup can be left
/// behind if the process exits after moving the old install but before the new
/// directory is in place; keeping this path deterministic lets the next scan
/// finish that interrupted replacement without another download.
fn recover_font_replacement(final_dir: &Path) -> Result<(), String> {
    let backup = replacement_backup_path(final_dir)?;
    let backup_exists = match std::fs::symlink_metadata(&backup) {
        Ok(_) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(format!("checking previous font install backup: {error}")),
    };
    if !backup_exists {
        return Ok(());
    }

    match std::fs::symlink_metadata(final_dir) {
        Ok(_) => remove_path_if_exists(&backup)
            .map_err(|error| format!("cleaning up previous font install backup: {error}")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            rename_font_path(&backup, final_dir)
                .map_err(|error| format!("restoring previous font install: {error}"))
        }
        Err(error) => Err(format!("checking existing font install: {error}")),
    }
}

fn recover_stale_font_replacements(base: &Path) {
    let Ok(families) = std::fs::read_dir(base) else {
        return;
    };
    for family in families.flatten() {
        let family_dir = family.path();
        if !family_dir.is_dir() {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&family_dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let Some(subset) = name
                .strip_prefix('.')
                .and_then(|name| name.strip_suffix(FONT_REPLACEMENT_BACKUP_SUFFIX))
                .filter(|subset| validate_slug(subset).is_ok())
            else {
                continue;
            };
            let final_dir = family_dir.join(subset);
            let _ = recover_font_replacement(&final_dir);
        }
    }
}

fn replace_staged_directory_with<F>(
    staging: &Path,
    final_dir: &Path,
    rename: F,
) -> Result<(), String>
where
    F: Fn(&Path, &Path) -> std::io::Result<()>,
{
    let backup = replacement_backup_path(final_dir)?;
    recover_font_replacement(final_dir)?;
    let had_previous = match std::fs::symlink_metadata(final_dir) {
        Ok(_) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(format!("checking existing font install: {error}")),
    };

    if had_previous {
        rename(final_dir, &backup)
            .map_err(|error| format!("staging previous font install for replacement: {error}"))?;
    }

    match rename(staging, final_dir) {
        Ok(()) => {
            if had_previous {
                let _ = remove_path_if_exists(&backup);
            }
            Ok(())
        }
        Err(error) => {
            let finalization_error = format!("finalizing font install: {error}");
            if !had_previous {
                return Err(finalization_error);
            }
            match rename(&backup, final_dir) {
                Ok(()) => Err(finalization_error),
                Err(restore_error) => Err(format!(
                    "{finalization_error}; restoring previous font install failed: {restore_error}"
                )),
            }
        }
    }
}

fn finalize_staged_directory(
    staging: &Path,
    staging_root: &Path,
    final_dir: &Path,
) -> Result<(), String> {
    finalize_staged_directory_with(staging, staging_root, final_dir, rename_font_path)
}

fn finalize_staged_directory_with<F>(
    staging: &Path,
    staging_root: &Path,
    final_dir: &Path,
    rename: F,
) -> Result<(), String>
where
    F: Fn(&Path, &Path) -> std::io::Result<()>,
{
    let result = replace_staged_directory_with(staging, final_dir, rename);
    if result.is_err() {
        let _ = remove_path_if_exists(staging);
    }
    let _ = std::fs::remove_dir(staging_root);
    result
}

fn install_font(request: InstallRequest, base: &Path) -> Result<FontInstallResult, String> {
    validate_slug(&request.id)?;
    validate_slug(&request.subset)?;
    let final_dir = base.join(&request.id).join(&request.subset);
    recover_font_replacement(&final_dir)?;
    let manifest_path = final_dir.join("manifest.json");
    if let Ok(json) = std::fs::read_to_string(&manifest_path) {
        if let Ok(font) = serde_json::from_str::<InstalledFont>(&json) {
            if manifest_is_safe(&font, &request.id, &request.family, &request.subset)
                && font_cache_is_usable(&font, &final_dir)
            {
                let paths: Vec<PathBuf> =
                    font.files.iter().map(|file| final_dir.join(file)).collect();
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
        let mut artifact_sha256 = BTreeMap::new();
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
            artifact_sha256.insert(filename.clone(), sha256_hex(&bytes));
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
            fontsource_version: details.npm_version,
            artifact_sha256,
        };
        let manifest = serde_json::to_vec_pretty(&font)
            .map_err(|error| format!("serializing font manifest: {error}"))?;
        std::fs::write(staging.join("manifest.json"), manifest)
            .map_err(|error| format!("writing font manifest: {error}"))?;

        if let Some(parent) = final_dir.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("creating family cache: {error}"))?;
        }
        finalize_staged_directory(&staging, &staging_root, &final_dir)?;
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
                let url = pin_font_url(&file.url.ttf, &details.id, &details.npm_version)?;
                plan.push((weight.clone(), style.clone(), url));
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
    if path.is_empty()
        || path.contains("..")
        || path.contains('\\')
        || path.contains('#')
        || path.contains('?')
    {
        return Err("font download URL contained an unsafe path".into());
    }
    Ok(())
}

fn validate_fontsource_version(version: &str) -> Result<(), String> {
    if version.is_empty() || version.len() > 64 || semver::Version::parse(version).is_err() {
        return Err("font metadata did not contain an exact Fontsource semver".into());
    }
    Ok(())
}

fn pin_font_url(url: &str, expected_id: &str, version: &str) -> Result<String, String> {
    validate_slug(expected_id)?;
    validate_fontsource_version(version)?;
    let Some(path) = url.strip_prefix(FONT_CDN_PREFIX) else {
        return Err("font download URL was outside the trusted Fontsource CDN".into());
    };
    let (package, filename) = path
        .split_once('/')
        .ok_or_else(|| "font download URL did not contain a font filename".to_string())?;
    if package != format!("{expected_id}@latest")
        || filename.is_empty()
        || filename.contains('/')
        || !filename.ends_with(".ttf")
    {
        return Err("font download URL did not match the selected Fontsource package".into());
    }
    let pinned = format!("{FONT_CDN_PREFIX}{expected_id}@{version}/{filename}");
    validate_font_url(&pinned)?;
    Ok(pinned)
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
        && validate_fontsource_version(&font.fontsource_version).is_ok()
        && !font.files.is_empty()
        && font.files.len() == font.artifact_sha256.len()
        && font.files.iter().all(|file| {
            let path = Path::new(file);
            path.components().count() == 1
                && path.extension().and_then(|ext| ext.to_str()) == Some("ttf")
                && !file.chars().any(char::is_control)
                && font
                    .artifact_sha256
                    .get(file)
                    .is_some_and(|digest| is_sha256(digest))
        })
}

fn scan_installed_fonts(base: &Path) -> Vec<InstalledFont> {
    recover_stale_font_replacements(base);
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
                    if manifest_is_safe(&font, &expected_id, &expected_family, &expected_subset)
                        && font_cache_is_usable(&font, &subset.path())
                    {
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
      "id":"test-sans","family":"Test Sans","defSubset":"latin","license":"OFL-1.1","npmVersion":"5.3.0",
      "source":"https://example.invalid/source","variants":{
        "400":{"normal":{"latin":{"url":{"ttf":"https://cdn.jsdelivr.net/fontsource/fonts/test-sans@latest/latin-400-normal.ttf"}}}},
        "700":{"italic":{"latin":{"url":{"ttf":"https://cdn.jsdelivr.net/fontsource/fonts/test-sans@latest/latin-700-italic.ttf"}}}}
      }
    }"#;

    fn usable_system_font_bytes() -> Option<Vec<u8>> {
        let mut database = glyphon::cosmic_text::fontdb::Database::new();
        database.load_system_fonts();
        let ids: Vec<_> = database.faces().map(|face| face.id).collect();
        ids.into_iter()
            .find_map(|id| {
                database.with_face_data(id, |data, _| (data.len() >= 512).then(|| data.to_vec()))
            })
            .flatten()
    }

    fn test_manifest(family: &str, files: &[&str], bytes: &[u8]) -> InstalledFont {
        let files: Vec<String> = files.iter().map(|file| (*file).into()).collect();
        InstalledFont {
            id: "test-font".into(),
            family: family.into(),
            subset: "latin".into(),
            license: "OFL-1.1".into(),
            source: "https://example.invalid".into(),
            artifact_sha256: files
                .iter()
                .map(|file| (file.clone(), sha256_hex(bytes)))
                .collect(),
            files,
            fontsource_version: "5.3.0".into(),
        }
    }

    fn set_file_digest(font: &mut InstalledFont, file: &str, bytes: &[u8]) {
        font.artifact_sha256
            .insert(file.to_owned(), sha256_hex(bytes));
    }

    fn changed_valid_font_bytes(bytes: &[u8]) -> Vec<u8> {
        let mut changed = bytes.to_vec();
        let table_count = u16::from_be_bytes([changed[4], changed[5]]) as usize;
        for index in 0..table_count {
            let entry = 12 + index * 16;
            if &changed[entry..entry + 4] == b"head" {
                let offset = u32::from_be_bytes([
                    changed[entry + 8],
                    changed[entry + 9],
                    changed[entry + 10],
                    changed[entry + 11],
                ]) as usize;
                changed[offset + 8] ^= 1;
                return changed;
            }
        }
        panic!("system font did not contain a head table")
    }

    fn write_test_install(path: &Path, font: &InstalledFont, bytes: &[u8]) {
        std::fs::create_dir_all(path).unwrap();
        std::fs::write(
            path.join("manifest.json"),
            serde_json::to_vec_pretty(font).unwrap(),
        )
        .unwrap();
        for file in &font.files {
            std::fs::write(path.join(file), bytes).unwrap();
        }
    }

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
        assert_eq!(
            plan[0].2,
            "https://cdn.jsdelivr.net/fontsource/fonts/test-sans@5.3.0/latin-400-normal.ttf"
        );
        assert_eq!(
            plan[1].2,
            "https://cdn.jsdelivr.net/fontsource/fonts/test-sans@5.3.0/latin-700-italic.ttf"
        );
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
    fn rejects_missing_or_non_semver_fontsource_versions() {
        let mut details: FontDetails = serde_json::from_str(DETAILS).unwrap();
        details.npm_version.clear();
        assert!(download_plan(&details, "latin").is_err());

        details.npm_version = "latest".into();
        assert!(download_plan(&details, "latin").is_err());
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
    fn failed_font_replacement_restores_the_previous_install() {
        let root = std::env::temp_dir().join(format!(
            "photonic-font-replace-failure-{}",
            uuid::Uuid::new_v4()
        ));
        let final_dir = root.join("test-font").join("latin");
        let staging_root = root.join(".photonic-font-staging");
        let staging = staging_root.join(".test-font-latin.part");
        let old = test_manifest("Old Font", &["old.ttf"], b"old font bytes");
        let new = test_manifest("New Font", &["new.ttf"], b"new font bytes");
        let old_manifest = serde_json::to_vec_pretty(&old).unwrap();

        write_test_install(&final_dir, &old, b"old font bytes");
        write_test_install(&staging, &new, b"new font bytes");

        let source = staging.clone();
        let destination = final_dir.clone();
        let error =
            finalize_staged_directory_with(&staging, &staging_root, &final_dir, move |from, to| {
                if from == source.as_path() && to == destination.as_path() {
                    Err(std::io::Error::other("injected finalization failure"))
                } else {
                    std::fs::rename(from, to)
                }
            })
            .unwrap_err();

        assert!(error.contains("injected finalization failure"));
        assert_eq!(
            std::fs::read(final_dir.join("manifest.json")).unwrap(),
            old_manifest
        );
        assert_eq!(
            std::fs::read(final_dir.join("old.ttf")).unwrap(),
            b"old font bytes"
        );
        assert!(!final_dir.join("new.ttf").exists());
        assert!(!staging.exists());
        assert!(!staging_root.exists());
        assert!(!replacement_backup_path(&final_dir).unwrap().exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn successful_font_replacement_selects_new_files_and_cleans_staging() {
        let Some(font_bytes) = usable_system_font_bytes() else {
            return;
        };
        let root = std::env::temp_dir().join(format!(
            "photonic-font-replace-success-{}",
            uuid::Uuid::new_v4()
        ));
        let final_dir = root.join("test-font").join("latin");
        let staging_root = root.join(".photonic-font-staging");
        let staging = staging_root.join(".test-font-latin.part");
        let old = test_manifest("Old Font", &["old.ttf"], &font_bytes);
        let new = test_manifest("New Font", &["new.ttf"], &font_bytes);

        write_test_install(&final_dir, &old, &font_bytes);
        write_test_install(&staging, &new, &font_bytes);
        finalize_staged_directory(&staging, &staging_root, &final_dir).unwrap();

        let found = scan_installed_fonts(&root);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].family, "New Font");
        assert_eq!(found[0].files, vec!["new.ttf"]);
        assert!(!final_dir.join("old.ttf").exists());
        assert!(!staging.exists());
        assert!(!staging_root.exists());
        assert!(!replacement_backup_path(&final_dir).unwrap().exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn interrupted_font_replacement_is_restored_before_discovery() {
        let Some(font_bytes) = usable_system_font_bytes() else {
            return;
        };
        let root = std::env::temp_dir().join(format!(
            "photonic-font-replace-recovery-{}",
            uuid::Uuid::new_v4()
        ));
        let final_dir = root.join("test-font").join("latin");
        let old = test_manifest("Old Font", &["old.ttf"], &font_bytes);
        write_test_install(&final_dir, &old, &font_bytes);
        let backup = replacement_backup_path(&final_dir).unwrap();
        std::fs::rename(&final_dir, &backup).unwrap();

        let found = scan_installed_fonts(&root);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].family, "Old Font");
        assert!(final_dir.join("manifest.json").is_file());
        assert!(final_dir.join("old.ttf").is_file());
        assert!(!backup.exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn installed_manifests_are_discovered_only_when_all_font_files_are_usable() {
        let root =
            std::env::temp_dir().join(format!("photonic-font-test-{}", uuid::Uuid::new_v4()));
        let staging = root.join(".unfinished.part");
        std::fs::create_dir_all(&staging).unwrap();

        fn write_manifest(root: &Path, font: &InstalledFont) -> PathBuf {
            let dir = root.join(&font.id).join(&font.subset);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("manifest.json"), serde_json::to_vec(font).unwrap()).unwrap();
            dir
        }

        let make_manifest = |id: &str, family: &str, files: Vec<&str>| InstalledFont {
            id: id.into(),
            family: family.into(),
            subset: "latin".into(),
            license: "OFL-1.1".into(),
            source: "https://example.invalid".into(),
            files: files.into_iter().map(str::to_owned).collect(),
            fontsource_version: "5.3.0".into(),
            artifact_sha256: BTreeMap::new(),
        };

        let valid_bytes = usable_system_font_bytes();
        if let Some(bytes) = valid_bytes.as_ref() {
            let mut valid = make_manifest("valid-font", "Valid Font", vec!["000-400-normal.ttf"]);
            let valid_file = valid.files[0].clone();
            set_file_digest(&mut valid, &valid_file, bytes);
            let dir = write_manifest(&root, &valid);
            std::fs::write(dir.join(&valid.files[0]), bytes).unwrap();
        }

        let missing = make_manifest("missing-font", "Missing Font", vec!["000-400-normal.ttf"]);
        write_manifest(&root, &missing);

        let undersized = make_manifest(
            "undersized-font",
            "Undersized Font",
            vec!["000-400-normal.ttf"],
        );
        let dir = write_manifest(&root, &undersized);
        std::fs::write(dir.join(&undersized.files[0]), [0_u8; 511]).unwrap();

        let invalid = make_manifest("invalid-font", "Invalid Font", vec!["000-400-normal.ttf"]);
        let dir = write_manifest(&root, &invalid);
        std::fs::write(dir.join(&invalid.files[0]), [0_u8; 512]).unwrap();

        if let Some(bytes) = valid_bytes.as_ref() {
            let partial = make_manifest(
                "partial-font",
                "Partial Font",
                vec!["000-400-normal.ttf", "001-700-normal.ttf"],
            );
            let first_file = partial.files[0].clone();
            let second_file = partial.files[1].clone();
            let mut partial = partial;
            set_file_digest(&mut partial, &first_file, bytes);
            set_file_digest(&mut partial, &second_file, &[0_u8; 512]);
            let dir = write_manifest(&root, &partial);
            std::fs::write(dir.join(&partial.files[0]), bytes).unwrap();
            std::fs::write(dir.join(&partial.files[1]), [0_u8; 512]).unwrap();
        }

        let unsafe_manifest =
            make_manifest("unsafe-font", "Unsafe Font", vec!["../../outside.ttf"]);
        write_manifest(&root, &unsafe_manifest);

        let found = scan_installed_fonts(&root);
        if valid_bytes.is_some() {
            assert_eq!(found.len(), 1);
            assert_eq!(found[0].family, "Valid Font");
        } else {
            assert!(found.is_empty());
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn changed_valid_font_bytes_are_not_reused_from_a_managed_cache() {
        let Some(original_bytes) = usable_system_font_bytes() else {
            return;
        };
        let changed_bytes = changed_valid_font_bytes(&original_bytes);
        assert_ne!(original_bytes, changed_bytes);
        assert!(font_bytes_are_usable(&changed_bytes));

        let root = std::env::temp_dir().join(format!(
            "photonic-font-digest-test-{}",
            uuid::Uuid::new_v4()
        ));
        let final_dir = root.join("test-font").join("latin");
        let manifest = test_manifest("Test Font", &["face.ttf"], &original_bytes);
        write_test_install(&final_dir, &manifest, &changed_bytes);

        assert!(scan_installed_fonts(&root).is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn legacy_manifests_without_provenance_are_not_managed() {
        let Some(bytes) = usable_system_font_bytes() else {
            return;
        };
        let mut manifest = test_manifest("Test Font", &["face.ttf"], &bytes);
        manifest.fontsource_version.clear();
        assert!(!manifest_is_safe(
            &manifest,
            "test-font",
            "Test Font",
            "latin"
        ));

        manifest.fontsource_version = "5.3.0".into();
        manifest.artifact_sha256.clear();
        assert!(!manifest_is_safe(
            &manifest,
            "test-font",
            "Test Font",
            "latin"
        ));
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
