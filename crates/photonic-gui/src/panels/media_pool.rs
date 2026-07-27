//! Media Pool drawer interior (video-editor-module `05-import-export.md` §2,
//! replacing the `video_stubs` shell): import button + window drag-drop →
//! import (with a background ffprobe worker), bins tree, list/grid asset views
//! with probe metadata, offline badge + relink flow, proxy status/toggle, and
//! drag-asset-to-timeline (payload consumed by `app/timeline/mod.rs`, which
//! inserts via the `ops_bridge` path).
//!
//! Import is asynchronous by construction: the click/drop enqueues paths on a
//! worker thread that probes (`photonic_video::media::probe`) and content-
//! hashes each file, then ships a fully-populated `MediaAsset` back over an
//! mpsc channel; the app drains the channel each frame and commits one
//! undoable `AddAsset` per finished asset. `EngineCmd::Probe` is still a P3
//! stub, so probing directly here (same code the engine uses) is the wiring
//! that works today — swap to engine-side probing when the decode-pool story
//! lands.
//!
//! Asset thumbnails are a documented seam (the engine exposes no per-asset
//! decoded-frame access yet); rows use kind glyphs instead.

use super::{PanelAction, PropPanelCtx};
use egui::Ui;
use egui_phosphor::regular as ph;
use photonic_core::timeline::{
    AssetId, AssetKind, AssetSource, BinId, MediaAsset, MediaBin, MediaPool, ProxyRef, ProxyStatus,
    Tick, TICKS_PER_SECOND,
};
use photonic_video::ProxyMode;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

// ── Import worker ─────────────────────────────────────────────────────────────

/// Session-side media pool state: view options, selection, and the background
/// import channel. Lives on `PhotonicApp`.
pub struct MediaPoolUi {
    pub grid_view: bool,
    pub selected: Option<AssetId>,
    /// One-shot: pool click wants single-monitor source peek (24 §3).
    /// Drained by `drive_playback` into `EngineCmd::SetPreviewTarget`.
    pub want_peek: Option<AssetId>,
    /// Bin filter; `None` = pool root (shows unfiled assets).
    pub current_bin: Option<BinId>,
    /// K-C2 filter: only show unused assets (usage == 0).
    pub filter_unused_only: bool,
    /// K-C2 filter: minimum star rating (0 = any).
    pub filter_min_rating: u8,
    pub new_bin_name: String,
    /// L1–L5 jobs still in flight (24-preview-media-load ladder).
    pub importing: usize,
    meta_tx: mpsc::Sender<ImportMetaResult>,
    meta_rx: mpsc::Receiver<ImportMetaResult>,
    /// Proxy transcodes currently running. Separate from importing: ffmpeg
    /// work must never block the editor thread or compete with asset probes.
    pub proxying: usize,
    /// Asset ids with a worker already generating a proxy (session-only).
    /// Prevents auto-L7 and manual "Build proxies" from double-queuing the
    /// same asset into concurrent ffmpeg writes of the same `.part` path.
    proxy_in_flight: std::collections::HashSet<AssetId>,
    proxy_tx: mpsc::Sender<ProxyJobResult>,
    proxy_rx: mpsc::Receiver<ProxyJobResult>,
    /// content_hash → poster PNG for grid/monitor paint (session).
    pub posters: std::collections::HashMap<String, PathBuf>,
    /// content_hash → L4 keyframe index ready.
    pub keyframe_ready: std::collections::HashSet<String>,
    /// content_hash → L5 waveform peak pyramid ready.
    pub waveform_ready: std::collections::HashSet<String>,
    /// Loaded egui textures for poster paths (path string → TextureHandle).
    poster_textures: std::collections::HashMap<String, egui::TextureHandle>,
}

/// Completed background proxy job, applied by the app through an undoable
/// `SetAssetProxy` command on its next UI frame.
pub struct ProxyJobResult {
    pub asset: AssetId,
    pub proxy: Option<ProxyRef>,
}

/// L1–L5 fill for an already-registered L0 asset (24 §2).
///
/// L1–L4 meta is sent **before** L5 runs so later stages never gate earlier
/// pool metadata (24 §2). A follow-up message may set only
/// [`ImportMetaResult::waveform_only`] after L5 completes.
pub struct ImportMetaResult {
    pub asset: AssetId,
    pub probe: Option<photonic_core::timeline::MediaProbe>,
    pub content_hash: Option<String>,
    pub poster_path: Option<PathBuf>,
    /// L4 keyframe index written (video only).
    pub keyframe_index: bool,
    /// L5 waveform pyramid written (audio / video-with-audio).
    pub waveform_ready: bool,
    /// When true, only session `waveform_ready` is updated — do not apply
    /// `set_asset_meta` (probe/hash already applied by the L1–L4 message).
    pub waveform_only: bool,
}

impl Default for MediaPoolUi {
    fn default() -> Self {
        let (meta_tx, meta_rx) = mpsc::channel();
        let (proxy_tx, proxy_rx) = mpsc::channel();
        MediaPoolUi {
            grid_view: false,
            selected: None,
            want_peek: None,
            current_bin: None,
            filter_unused_only: false,
            filter_min_rating: 0,
            new_bin_name: String::new(),
            importing: 0,
            meta_tx,
            meta_rx,
            proxying: 0,
            proxy_in_flight: std::collections::HashSet::new(),
            proxy_tx,
            proxy_rx,
            posters: std::collections::HashMap::new(),
            keyframe_ready: std::collections::HashSet::new(),
            waveform_ready: std::collections::HashSet::new(),
            poster_textures: std::collections::HashMap::new(),
        }
    }
}

impl MediaPoolUi {
    /// L0-first import (24 §2): returns stubs for immediate `AddAsset`, then
    /// runs hash → probe → poster → keyframe index → waveform on a worker.
    pub fn spawn_import(
        &mut self,
        paths: Vec<PathBuf>,
        bin: Option<BinId>,
        project_path: Option<PathBuf>,
    ) -> Vec<MediaAsset> {
        // L0 first: stubs are placeable immediately (24 §2). Worker does L1–L5.
        let stubs = l0_register_stubs(&paths, bin);
        if stubs.is_empty() {
            return Vec::new();
        }
        let jobs: Vec<(MediaAsset, PathBuf)> = {
            let mut out = Vec::with_capacity(stubs.len());
            let mut si = 0;
            for path in paths {
                if guess_asset_kind(&path).is_none() {
                    continue;
                }
                if si < stubs.len() {
                    out.push((stubs[si].clone(), path));
                    si += 1;
                }
            }
            out
        };
        self.importing += stubs.len();
        let meta_tx = self.meta_tx.clone();
        std::thread::spawn(move || {
            let tools = photonic_video::media::ffmpeg_locate::locate().ok();
            let cache_dir =
                photonic_video::media::poster::poster_cache_dir(project_path.as_deref());
            for (asset, path) in jobs {
                let asset_id = asset.id;
                let hash = photonic_video::media::probe::content_hash(&path).ok();
                let probe = tools.as_ref().and_then(|t| {
                    match photonic_video::media::probe::probe_asset(t, &path) {
                        Ok(p) => Some(p),
                        Err(e) => {
                            tracing::warn!("media import: probe failed for {path:?}: {e}");
                            None
                        }
                    }
                });
                // L3 poster before L4 keyframe index (24 §2 priority).
                let poster_path = match (tools.as_ref(), hash.as_ref()) {
                    (Some(tools), Some(h))
                        if matches!(asset.kind, AssetKind::Video | AssetKind::Image) =>
                    {
                        let out = photonic_video::media::poster::poster_cache_path(&cache_dir, h);
                        match photonic_video::media::poster::ensure_poster(tools, &path, &out) {
                            Ok(p) => Some(p),
                            Err(e) => {
                                tracing::debug!("media import: poster failed for {path:?}: {e}");
                                None
                            }
                        }
                    }
                    _ => None,
                };
                // L4: keyframe index (video only) — warms scrub seeks.
                let keyframe_index = match (tools.as_ref(), hash.as_ref()) {
                    (Some(tools), Some(h)) if asset.kind == AssetKind::Video => {
                        match photonic_video::media::KeyframeIndex::load_or_build(
                            tools, &path, &cache_dir, h,
                        ) {
                            Ok(_) => true,
                            Err(e) => {
                                tracing::debug!(
                                    "media import: keyframe index failed for {path:?}: {e}"
                                );
                                false
                            }
                        }
                    }
                    _ => false,
                };
                // Send L1–L4 meta *before* L5 so long waveforms never gate
                // pool columns / L7 auto-queue (24 §2).
                if meta_tx
                    .send(ImportMetaResult {
                        asset: asset_id,
                        probe: probe.clone(),
                        content_hash: hash.clone(),
                        poster_path,
                        keyframe_index,
                        waveform_ready: false,
                        waveform_only: false,
                    })
                    .is_err()
                {
                    return;
                }
                // L5: waveform peak pyramid (audio + video-with-audio). Same
                // sidecar dir as timeline WaveformCache so reopen/paint hits.
                // Failures never fail import (24 §2 / D-PM-6).
                let waveform_ready = match (tools.as_ref(), hash.as_ref()) {
                    (Some(tools), Some(h))
                        if matches!(asset.kind, AssetKind::Audio | AssetKind::Video) =>
                    {
                        let try_wave = match asset.kind {
                            AssetKind::Audio => true,
                            AssetKind::Video => match &probe {
                                Some(p) => p.audio.is_some(),
                                None => true,
                            },
                            _ => false,
                        };
                        if !try_wave {
                            false
                        } else if matches!(
                            photonic_video::audio::waveform::load_from_dir(&cache_dir, h),
                            Ok(Some(_))
                        ) {
                            true
                        } else {
                            const WAVEFORM_SAMPLE_RATE: u32 = 48_000;
                            match photonic_video::playback::FfmpegPcmSource::spawn(
                                tools,
                                &path,
                                Tick::ZERO,
                                WAVEFORM_SAMPLE_RATE,
                            ) {
                                Ok(mut src) => {
                                    let pyramid = photonic_video::audio::waveform::build_pyramid(
                                        &mut src,
                                        h.clone(),
                                    );
                                    match photonic_video::audio::waveform::save_to_dir(
                                        &pyramid, &cache_dir,
                                    ) {
                                        Ok(_) => true,
                                        Err(e) => {
                                            tracing::debug!(
                                                "media import: waveform save failed for {path:?}: {e}"
                                            );
                                            false
                                        }
                                    }
                                }
                                Err(e) => {
                                    tracing::debug!(
                                        "media import: waveform decode failed for {path:?}: {e}"
                                    );
                                    false
                                }
                            }
                        }
                    }
                    _ => false,
                };
                if waveform_ready {
                    if meta_tx
                        .send(ImportMetaResult {
                            asset: asset_id,
                            probe: None,
                            content_hash: hash,
                            poster_path: None,
                            keyframe_index: false,
                            waveform_ready: true,
                            waveform_only: true,
                        })
                        .is_err()
                    {
                        return;
                    }
                }
            }
        });
        stubs
    }

    /// Drain L1–L5 completions. L1–L4 messages need `set_asset_meta`; waveform-only
    /// follow-ups only update session readiness.
    pub fn drain_meta(&mut self) -> Vec<ImportMetaResult> {
        let mut out = Vec::new();
        while let Ok(meta) = self.meta_rx.try_recv() {
            if !meta.waveform_only {
                self.importing = self.importing.saturating_sub(1);
            }
            if let Some(hash) = &meta.content_hash {
                if let Some(path) = &meta.poster_path {
                    self.posters.insert(hash.clone(), path.clone());
                }
                if meta.keyframe_index {
                    self.keyframe_ready.insert(hash.clone());
                }
                if meta.waveform_ready {
                    self.waveform_ready.insert(hash.clone());
                }
            }
            out.push(meta);
        }
        out
    }

    /// Ensure a poster texture is loaded for `hash` (if a path is known).
    fn poster_texture(&mut self, ctx: &egui::Context, hash: &str) -> Option<egui::TextureHandle> {
        if let Some(tex) = self.poster_textures.get(hash) {
            return Some(tex.clone());
        }
        let path = self.posters.get(hash)?;
        let bytes = std::fs::read(path).ok()?;
        let img = photonic_core::raster::image::RasterImage::from_encoded(&bytes).ok()?;
        let color = egui::ColorImage::from_rgba_unmultiplied(
            [img.width as usize, img.height as usize],
            &img.pixels,
        );
        let name = format!("media_poster_{hash}");
        let handle = ctx.load_texture(name, color, egui::TextureOptions::LINEAR);
        self.poster_textures
            .insert(hash.to_string(), handle.clone());
        Some(handle)
    }

    /// True while a background proxy job is already covering this asset.
    pub fn proxy_job_in_flight(&self, asset: AssetId) -> bool {
        self.proxy_in_flight.contains(&asset)
    }

    /// Build all supplied file-backed video proxies sequentially on one worker.
    /// The worker yields CPU priority inside `generate_proxy`; completion is
    /// handed back to the UI so document/history mutation stays on the main
    /// thread. Existing ready cache files are reused without re-encoding.
    ///
    /// Skips assets already in flight (session) so auto-L7 and manual Build
    /// cannot double-write the same `.part` proxy path.
    pub fn spawn_proxy_generation(
        &mut self,
        assets: Vec<MediaAsset>,
        project_path: Option<PathBuf>,
    ) {
        let assets: Vec<MediaAsset> = assets
            .into_iter()
            .filter(|asset| {
                asset.kind == AssetKind::Video
                    && matches!(asset.source, AssetSource::File { .. })
                    && !self.proxy_in_flight.contains(&asset.id)
                    // Never overwrite a user-attached proxy with a generated one
                    // (G-15A; in-flight generate completion must not clobber Attach).
                    && !matches!(
                        asset.proxy.as_ref(),
                        Some(p) if p.origin == photonic_core::timeline::ProxyOrigin::Attached
                    )
            })
            .collect();
        if assets.is_empty() {
            return;
        }
        for a in &assets {
            self.proxy_in_flight.insert(a.id);
        }
        self.proxying += assets.len();
        let tx = self.proxy_tx.clone();
        std::thread::spawn(move || {
            let tools = photonic_video::media::ffmpeg_locate::locate().ok();
            let cache_dir = photonic_video::media::proxy::proxy_cache_dir(project_path.as_deref());
            for asset in assets {
                let AssetSource::File { path, .. } = &asset.source else {
                    let _ = tx.send(ProxyJobResult {
                        asset: asset.id,
                        proxy: asset.proxy.clone(),
                    });
                    continue;
                };
                let hash = asset
                    .content_hash
                    .clone()
                    .or_else(|| photonic_video::media::probe::content_hash(path).ok());
                let proxy = match (tools.as_ref(), hash) {
                    (Some(tools), Some(hash)) => {
                        let output =
                            photonic_video::media::proxy::proxy_cache_path(&cache_dir, &hash);
                        let status = if output.is_file()
                            || photonic_video::media::proxy::generate_proxy(
                                tools,
                                path,
                                &output,
                                &|| false,
                            )
                            .is_ok()
                        {
                            ProxyStatus::Ready
                        } else {
                            ProxyStatus::Failed
                        };
                        Some(ProxyRef {
                            path: output,
                            status,
                            origin: photonic_core::timeline::ProxyOrigin::Generated,
                        })
                    }
                    // A missing toolchain/hash must not detach a previously
                    // ready proxy; preserve the asset's existing attachment.
                    _ => asset.proxy.clone(),
                };
                if tx
                    .send(ProxyJobResult {
                        asset: asset.id,
                        proxy,
                    })
                    .is_err()
                {
                    return;
                }
            }
        });
    }

    /// Drain proxy completions. The caller applies them through timeline
    /// commands so undo/redo and the engine mirror stay coherent.
    /// Each completion is a single document update (Ready/Failed) — no
    /// intermediate Pending history entries (G-15C).
    pub fn drain_finished_proxies(&mut self) -> Vec<ProxyJobResult> {
        let mut out = Vec::new();
        while let Ok(result) = self.proxy_rx.try_recv() {
            self.proxying = self.proxying.saturating_sub(1);
            self.proxy_in_flight.remove(&result.asset);
            out.push(result);
        }
        out
    }
}

/// Drag payload for pool-row → timeline drops (consumed in
/// `app/timeline/mod.rs`).
#[derive(Clone, Copy, Debug)]
pub struct AssetDrag {
    pub asset: AssetId,
}

// ── Pure helpers (unit-tested below) ─────────────────────────────────────────

/// L0 register only: build placeable stubs with **no** probe/hash (24 §2).
/// Pure and synchronous — the same path `spawn_import` uses before the worker.
/// Multi-select import must return N stubs before any L1–L2 completes.
pub fn l0_register_stubs(paths: &[PathBuf], bin: Option<BinId>) -> Vec<MediaAsset> {
    paths
        .iter()
        .filter_map(|path| {
            let kind = guess_asset_kind(path)?;
            let mut asset = MediaAsset::from_file(kind, path);
            asset.bin = bin;
            // Contract: L0 has no probe/hash yet.
            debug_assert!(asset.probe.is_none());
            debug_assert!(asset.content_hash.is_none());
            Some(asset)
        })
        .collect()
}

/// Extension → asset kind (mirrors 05 §1's accepted-format table).
pub fn guess_asset_kind(path: &Path) -> Option<AssetKind> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "mp4" | "mov" | "mkv" | "avi" | "webm" | "m4v" | "mts" | "mxf" => AssetKind::Video,
        "mp3" | "wav" | "aac" | "flac" | "ogg" | "m4a" | "opus" => AssetKind::Audio,
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "tiff" | "tif" | "exr" => {
            AssetKind::Image
        }
        "svg" | "photon" => AssetKind::VectorDoc,
        "cube" => AssetKind::Lut3d,
        _ => return None,
    })
}

/// Display name: the file stem for file-backed assets, a fixed label for
/// embedded vector refs.
pub fn asset_display_name(asset: &MediaAsset) -> String {
    match &asset.source {
        AssetSource::File { path, .. } => path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string()),
        AssetSource::EmbeddedVector { .. } => "Embedded vector".to_string(),
    }
}

/// Is the file-backed source currently reachable? Embedded sources are always
/// online.
pub fn asset_is_offline(asset: &MediaAsset) -> bool {
    match &asset.source {
        AssetSource::File { path, .. } => !path.exists(),
        AssetSource::EmbeddedVector { .. } => false,
    }
}

/// `M:SS.d` duration readout for list columns.
pub fn format_duration(t: Tick) -> String {
    let total_ds = (t.0.max(0) * 10) / TICKS_PER_SECOND; // deciseconds
    let m = total_ds / 600;
    let s = (total_ds % 600) / 10;
    let d = total_ds % 10;
    format!("{m}:{s:02}.{d}")
}

/// One-line probe summary (dimensions / fps / audio) for the metadata column.
pub fn probe_summary(asset: &MediaAsset) -> String {
    let Some(probe) = &asset.probe else {
        return "—".to_string();
    };
    let mut parts: Vec<String> = Vec::new();
    parts.push(format_duration(probe.duration));
    if let Some(v) = &probe.video {
        let fps = v.frame_rate.num as f64 / v.frame_rate.den.max(1) as f64;
        parts.push(format!("{}x{}", v.width, v.height));
        parts.push(format!("{fps:.3}fps").replace(".000fps", "fps"));
        // K-G6: surface interlaced scan so the pool shows what probe found.
        match v.scan {
            photonic_core::timeline::ScanType::InterlacedTopFirst => {
                parts.push("interlaced (TFF)".into());
            }
            photonic_core::timeline::ScanType::InterlacedBottomFirst => {
                parts.push("interlaced (BFF)".into());
            }
            photonic_core::timeline::ScanType::Progressive
            | photonic_core::timeline::ScanType::Unknown => {}
        }
    }
    if let Some(a) = &probe.audio {
        parts.push(format!("{}ch {}Hz", a.channels, a.sample_rate));
    }
    parts.push(probe.codec.clone());
    parts.join(" · ")
}

/// Child bins of `parent`, in name order.
pub fn bin_children(bins: &[MediaBin], parent: Option<BinId>) -> Vec<&MediaBin> {
    let mut out: Vec<&MediaBin> = bins.iter().filter(|b| b.parent == parent).collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Assets filed under `bin` (root = unfiled), sorted by display name for a
/// stable list.
pub fn assets_in_bin(pool: &MediaPool, bin: Option<BinId>) -> Vec<&MediaAsset> {
    let mut out: Vec<&MediaAsset> = pool.assets.values().filter(|a| a.bin == bin).collect();
    out.sort_by_key(|a| asset_display_name(a));
    out
}

/// K-C2: how many timeline clips reference `asset` across all sequences
/// (derived query — not stored). Pure over the project graph.
pub fn asset_usage_count(
    project: &photonic_core::timeline::TimelineProject,
    asset: AssetId,
) -> usize {
    let mut n = 0usize;
    for seq in project.sequences.values() {
        for track in seq.tracks() {
            for clip in &track.clips {
                if matches!(
                    &clip.source,
                    photonic_core::timeline::ClipSource::Asset { asset: a } if *a == asset
                ) {
                    n += 1;
                }
            }
        }
    }
    n
}

fn kind_glyph(kind: AssetKind) -> &'static str {
    match kind {
        AssetKind::Video => ph::FILM_STRIP,
        AssetKind::Audio => ph::SPEAKER_HIGH,
        AssetKind::Image => ph::IMAGE,
        AssetKind::VectorDoc => ph::BEZIER_CURVE,
        AssetKind::Lut3d => ph::PALETTE,
    }
}

// ── Drawer UI ─────────────────────────────────────────────────────────────────

/// `DrawerGroup::MediaPool` interior (04 §4.1 / 05 §2).
pub(crate) fn draw_media_pool(ui: &mut Ui, ctx: &mut PropPanelCtx) {
    let Some(project) = ctx.doc.timeline.as_ref() else {
        ui.label("No timeline project yet — enter video mode to create one.");
        return;
    };
    let pool = &project.media;

    // ── Toolbar: import, new bin, view toggle ───────────────────────────────
    ui.horizontal(|ui| {
        if ui
            .button(format!("{} Import…", ph::DOWNLOAD_SIMPLE))
            .on_hover_text("Import media files (or drop files on the window)")
            .clicked()
        {
            ctx.action = Some(PanelAction::MediaImportDialog {
                bin: ctx.media_ui.current_bin,
            });
        }
        let grid = ctx.media_ui.grid_view;
        if ui
            .selectable_label(!grid, ph::LIST)
            .on_hover_text("List view")
            .clicked()
        {
            ctx.media_ui.grid_view = false;
        }
        if ui
            .selectable_label(grid, ph::SQUARES_FOUR)
            .on_hover_text("Grid view")
            .clicked()
        {
            ctx.media_ui.grid_view = true;
        }
    });

    // ── Proxy playback mode (engine-wide toggle, 05 §4) + L7 ingest (G-15C) ─
    ui.horizontal(|ui| {
        ui.label("Proxies:");
        let mut mode = ctx.proxy_mode;
        egui::ComboBox::from_id_salt("media_proxy_mode")
            .selected_text(match mode {
                ProxyMode::Auto => "Auto",
                ProxyMode::ForceProxy => "Force proxy",
                ProxyMode::ForceOriginal => "Force original",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut mode, ProxyMode::Auto, "Auto");
                ui.selectable_value(&mut mode, ProxyMode::ForceProxy, "Force proxy");
                ui.selectable_value(&mut mode, ProxyMode::ForceOriginal, "Force original");
            });
        if mode != ctx.proxy_mode {
            ctx.action = Some(PanelAction::MediaSetProxyMode { mode });
        }
        let generate_on_import = ctx
            .doc
            .timeline
            .as_ref()
            .map(|p| p.settings.generate_proxies)
            .unwrap_or(false);
        let mut gen = generate_on_import;
        if ui
            .checkbox(&mut gen, "On import")
            .on_hover_text(
                "Automatically build half-res editing proxies after import (L7). \
                 Playback uses them when Proxy mode is Auto or Force proxy.",
            )
            .changed()
        {
            ctx.action = Some(PanelAction::MediaSetGenerateProxiesOnImport { enabled: gen });
        }
        if ui
            .add_enabled(
                ctx.media_ui.proxying == 0,
                egui::Button::new("Build proxies"),
            )
            .on_hover_text("Create reusable low-resolution editing proxies in the background")
            .clicked()
        {
            ctx.action = Some(PanelAction::MediaGenerateProxies);
        }
        if ctx.media_ui.proxying > 0 {
            ui.label(
                egui::RichText::new(format!("{} building…", ctx.media_ui.proxying))
                    .weak()
                    .small(),
            );
        }
        if !ctx.engine_online {
            ui.label(egui::RichText::new("(engine offline)").weak().small());
        }
    });

    // ── K-C2 filters + K-C5 remove-unused ───────────────────────────────────
    ui.horizontal(|ui| {
        ui.label("Filter:");
        ui.checkbox(&mut ctx.media_ui.filter_unused_only, "Unused only")
            .on_hover_text("Show only assets not placed on any sequence");
        ui.label("Min ★");
        for r in 0u8..=5 {
            let label = if r == 0 {
                "Any".to_string()
            } else {
                format!("{r}")
            };
            if ui
                .selectable_label(ctx.media_ui.filter_min_rating == r, label)
                .clicked()
            {
                ctx.media_ui.filter_min_rating = r;
            }
        }
        ui.separator();
        let unused_n = project
            .media
            .assets
            .keys()
            .filter(|id| asset_usage_count(project, **id) == 0)
            .count();
        if ui
            .add_enabled(
                unused_n > 0,
                egui::Button::new(format!("Remove unused ({unused_n})")),
            )
            .on_hover_text("Delete every media pool asset with zero timeline references (undoable)")
            .clicked()
        {
            ctx.action = Some(PanelAction::MediaRemoveUnused);
        }
        // K-C5: cache size report (read-only).
        if ui
            .button("Cache…")
            .on_hover_text("Show project sidecar cache sizes (proxies, posters, …)")
            .clicked()
        {
            // Project path is not on PropPanelCtx; global proxy cache + any
            // open project's cache is enough for a size readout (K-C5 slice).
            let report = photonic_video::media::summarize_cache(None);
            let mut lines = vec![format!(
                "Cache root: {}\n{:.2} MB total",
                report.root.display(),
                report.total_mb()
            )];
            for c in &report.categories {
                if c.files > 0 {
                    lines.push(format!(
                        "  {}: {:.2} MB ({} files)",
                        c.name,
                        c.bytes as f64 / (1024.0 * 1024.0),
                        c.files
                    ));
                }
            }
            // Surface via the same status path as imports.
            // PropPanelCtx may not have set_import_status — use action-less toast via selection of label.
            ui.memory_mut(|m| {
                m.data.insert_temp(
                    egui::Id::new("media_cache_report"),
                    lines.join("\n"),
                );
            });
        }
    });
    if let Some(report) = ui.memory(|m| {
        m.data
            .get_temp::<String>(egui::Id::new("media_cache_report"))
    }) {
        ui.collapsing("Cache report", |ui| {
            ui.monospace(&report);
            if ui.button("Dismiss").clicked() {
                ui.memory_mut(|m| {
                    m.data
                        .remove::<String>(egui::Id::new("media_cache_report"));
                });
            }
        });
    }

    ui.separator();

    // ── Bins tree (flat with parent refs → indented tree) ───────────────────
    ui.horizontal(|ui| {
        let root_selected = ctx.media_ui.current_bin.is_none();
        if ui
            .selectable_label(root_selected, format!("{} Media", ph::HOUSE))
            .clicked()
        {
            ctx.media_ui.current_bin = None;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let name_ok = !ctx.media_ui.new_bin_name.trim().is_empty();
            if ui
                .add_enabled(name_ok, egui::Button::new(ph::FOLDER_PLUS))
                .on_hover_text("New bin")
                .clicked()
            {
                ctx.action = Some(PanelAction::MediaCreateBin {
                    name: std::mem::take(&mut ctx.media_ui.new_bin_name),
                    parent: ctx.media_ui.current_bin,
                });
            }
            ui.add(
                egui::TextEdit::singleline(&mut ctx.media_ui.new_bin_name)
                    .hint_text("New bin…")
                    .desired_width(110.0),
            );
        });
    });
    draw_bin_tree(ui, ctx, &pool.bins, None, 0);

    ui.separator();

    // ── Pending imports ─────────────────────────────────────────────────────
    if ctx.media_ui.importing > 0 {
        ui.horizontal(|ui| {
            ui.add(egui::Spinner::new().size(14.0));
            ui.label(format!(
                "Importing {} file{}…",
                ctx.media_ui.importing,
                if ctx.media_ui.importing == 1 { "" } else { "s" }
            ));
        });
    }

    // ── Asset list / grid ───────────────────────────────────────────────────
    let mut assets = assets_in_bin(pool, ctx.media_ui.current_bin);
    // K-C2 usage counts: pure derived query over timeline clips.
    let usage: std::collections::HashMap<AssetId, usize> = assets
        .iter()
        .map(|a| (a.id, asset_usage_count(project, a.id)))
        .collect();
    // Apply K-C2 filters (unused-only / min rating).
    let min_r = ctx.media_ui.filter_min_rating;
    let unused_only = ctx.media_ui.filter_unused_only;
    assets.retain(|a| {
        let n = usage.get(&a.id).copied().unwrap_or(0);
        if unused_only && n > 0 {
            return false;
        }
        if min_r > 0 {
            match a.rating {
                Some(r) if r >= min_r => {}
                _ => return false,
            }
        }
        true
    });
    if assets.is_empty() && ctx.media_ui.importing == 0 {
        ui.label(
            egui::RichText::new("No media here yet. Import files or drop them on the window.")
                .weak(),
        );
        return;
    }

    egui::ScrollArea::vertical()
        .auto_shrink([false, true])
        .show(ui, |ui| {
            if ctx.media_ui.grid_view {
                ui.horizontal_wrapped(|ui| {
                    for asset in &assets {
                        let n = usage.get(&asset.id).copied().unwrap_or(0);
                        draw_asset_cell(ui, ctx, asset, true, &pool.bins, n);
                    }
                });
            } else {
                for asset in &assets {
                    let n = usage.get(&asset.id).copied().unwrap_or(0);
                    draw_asset_cell(ui, ctx, asset, false, &pool.bins, n);
                }
            }
        });
}

fn draw_bin_tree(
    ui: &mut Ui,
    ctx: &mut PropPanelCtx,
    bins: &[MediaBin],
    parent: Option<BinId>,
    depth: usize,
) {
    if depth > 16 {
        return; // corrupt parent cycles shouldn't hang the UI
    }
    // Collect ids up front: `ctx` is mutably borrowed inside the loop.
    let children: Vec<(BinId, String)> = bin_children(bins, parent)
        .into_iter()
        .map(|b| (b.id, b.name.clone()))
        .collect();
    for (id, name) in children {
        ui.horizontal(|ui| {
            ui.add_space(12.0 * (depth + 1) as f32);
            let selected = ctx.media_ui.current_bin == Some(id);
            let resp = ui.selectable_label(selected, format!("{} {name}", ph::FOLDER));
            if resp.clicked() {
                ctx.media_ui.current_bin = Some(id);
            }
            resp.context_menu(|ui| {
                if ui.button("Delete bin").clicked() {
                    ctx.action = Some(PanelAction::MediaRemoveBin { bin: id });
                    ui.close_menu();
                }
            });
        });
        draw_bin_tree(ui, ctx, bins, Some(id), depth + 1);
    }
}

/// One asset row (list) or tile (grid): kind glyph (thumbnail seam), name,
/// probe metadata, offline/proxy badges, drag source + context menu.
/// `usage` is the K-C2 clip-reference count (0 = unused).
fn draw_asset_cell(
    ui: &mut Ui,
    ctx: &mut PropPanelCtx,
    asset: &MediaAsset,
    grid: bool,
    bins: &[MediaBin],
    usage: usize,
) {
    let name = asset_display_name(asset);
    let offline = asset_is_offline(asset);
    let selected = ctx.media_ui.selected == Some(asset.id);
    let drag_id = ui.id().with(("media_asset", asset.id));

    let response = ui
        .dnd_drag_source(drag_id, AssetDrag { asset: asset.id }, |ui| {
            let frame = egui::Frame::none()
                .inner_margin(egui::Margin::symmetric(6.0, 4.0))
                .rounding(4.0)
                .fill(if selected {
                    ui.visuals().selection.bg_fill.gamma_multiply(0.4)
                } else {
                    egui::Color32::TRANSPARENT
                });
            frame.show(ui, |ui| {
                if grid {
                    ui.set_width(96.0);
                    ui.vertical_centered(|ui| {
                        // L3 poster when ready; kind glyph otherwise (24 §2).
                        let poster = asset
                            .content_hash
                            .as_deref()
                            .and_then(|h| ctx.media_ui.poster_texture(ui.ctx(), h));
                        if let Some(tex) = poster {
                            let size = egui::vec2(88.0, 50.0);
                            ui.add(egui::Image::new((tex.id(), size)).maintain_aspect_ratio(true));
                        } else {
                            ui.label(egui::RichText::new(kind_glyph(asset.kind)).size(28.0));
                        }
                        ui.label(egui::RichText::new(&name).small());
                        badges(ui, asset, offline, usage);
                    });
                } else {
                    ui.horizontal(|ui| {
                        let poster = asset
                            .content_hash
                            .as_deref()
                            .and_then(|h| ctx.media_ui.poster_texture(ui.ctx(), h));
                        if let Some(tex) = poster {
                            ui.add(
                                egui::Image::new((tex.id(), egui::vec2(40.0, 24.0)))
                                    .maintain_aspect_ratio(true),
                            );
                        } else {
                            ui.label(kind_glyph(asset.kind));
                        }
                        // Right cluster first (stats + badges hug the right
                        // edge); the name then fills the remaining left space and
                        // truncates, so a long name never overruns into the stats
                        // (they used to draw into the same pixels).
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(egui::RichText::new(probe_summary(asset)).weak().small());
                            badges(ui, asset, offline, usage);
                            ui.add(egui::Label::new(&name).truncate());
                        });
                    });
                }
            });
        })
        .response;

    if response.clicked() {
        ctx.media_ui.selected = Some(asset.id);
        // Single-monitor source peek (24 §3) — play-wins still enforced engine-side.
        ctx.media_ui.want_peek = Some(asset.id);
    }
    if response.double_clicked() {
        ctx.action = Some(PanelAction::MediaInsertAtPlayhead { asset: asset.id });
    }
    response
        .on_hover_text(match &asset.source {
            AssetSource::File { path, .. } => path.display().to_string(),
            AssetSource::EmbeddedVector { .. } => "Embedded vector content".to_string(),
        })
        .context_menu(|ui| {
            if ui.button("Insert at playhead").clicked() {
                ctx.action = Some(PanelAction::MediaInsertAtPlayhead { asset: asset.id });
                ui.close_menu();
            }
            if offline && ui.button("Relink…").clicked() {
                ctx.action = Some(PanelAction::MediaRelink { asset: asset.id });
                ui.close_menu();
            }
            // K-C2 star rating.
            ui.menu_button("Rating", |ui| {
                if ui
                    .selectable_label(asset.rating.is_none(), "Unrated")
                    .clicked()
                {
                    ctx.action = Some(PanelAction::MediaSetRating {
                        asset: asset.id,
                        rating: None,
                    });
                    ui.close_menu();
                }
                for r in 1u8..=5 {
                    let stars = "★".repeat(r as usize);
                    if ui
                        .selectable_label(asset.rating == Some(r), format!("{stars} ({r})"))
                        .clicked()
                    {
                        ctx.action = Some(PanelAction::MediaSetRating {
                            asset: asset.id,
                            rating: Some(r),
                        });
                        ui.close_menu();
                    }
                }
            });
            // G-15A: attach a user-owned proxy without re-encoding; detach never
            // deletes Attached files (handler / set_asset_proxy only clears ref).
            if asset.kind == AssetKind::Video
                && matches!(asset.source, AssetSource::File { .. })
                && ui.button("Attach Proxy…").clicked()
            {
                ctx.action = Some(PanelAction::MediaAttachProxy { asset: asset.id });
                ui.close_menu();
            }
            if asset.proxy.is_some() && ui.button("Detach Proxy").clicked() {
                ctx.action = Some(PanelAction::MediaDetachProxy { asset: asset.id });
                ui.close_menu();
            }
            ui.menu_button("Move to bin", |ui| {
                if ui.button("(root)").clicked() {
                    ctx.action = Some(PanelAction::MediaAssignBin {
                        asset: asset.id,
                        bin: None,
                    });
                    ui.close_menu();
                }
                for b in bin_children(bins, None) {
                    move_to_bin_items(ui, ctx, asset.id, bins, b, 0);
                }
            });
            if ui.button("Remove from pool").clicked() {
                ctx.action = Some(PanelAction::MediaRemoveAsset { asset: asset.id });
                ui.close_menu();
            }
        });
}

fn move_to_bin_items(
    ui: &mut Ui,
    ctx: &mut PropPanelCtx,
    asset: AssetId,
    bins: &[MediaBin],
    bin: &MediaBin,
    depth: usize,
) {
    if depth > 16 {
        return;
    }
    if ui
        .button(format!("{}{}", "  ".repeat(depth), bin.name))
        .clicked()
    {
        ctx.action = Some(PanelAction::MediaAssignBin {
            asset,
            bin: Some(bin.id),
        });
        ui.close_menu();
    }
    for child in bin_children(bins, Some(bin.id)) {
        move_to_bin_items(ui, ctx, asset, bins, child, depth + 1);
    }
}

fn badges(ui: &mut Ui, asset: &MediaAsset, offline: bool, usage: usize) {
    if let Some(r) = asset.rating {
        ui.label(
            egui::RichText::new(format!("{}★", r.min(5)))
                .small()
                .color(egui::Color32::from_rgb(235, 200, 80)),
        )
        .on_hover_text(format!("Rating: {r}/5"));
    }
    if usage > 0 {
        let label = if usage == 1 {
            "ON TL".to_string()
        } else {
            format!("×{usage}")
        };
        ui.label(
            egui::RichText::new(label)
                .small()
                .strong()
                .color(egui::Color32::from_rgb(120, 200, 140)),
        )
        .on_hover_text(format!(
            "Used on the timeline ({usage} clip{})",
            if usage == 1 { "" } else { "s" }
        ));
    }
    if offline {
        ui.label(
            egui::RichText::new("OFFLINE")
                .small()
                .strong()
                .color(egui::Color32::from_rgb(235, 100, 90)),
        );
    }
    // K-G6: interlaced badge + triage consequence on hover (detection is live;
    // deinterlace node is still open — badge makes the risk visible now).
    if let Some(v) = asset.probe.as_ref().and_then(|p| p.video.as_ref()) {
        if v.scan.is_interlaced() {
            let order = match v.scan {
                photonic_core::timeline::ScanType::InterlacedTopFirst => "top-field first",
                photonic_core::timeline::ScanType::InterlacedBottomFirst => "bottom-field first",
                _ => "interlaced",
            };
            let consequence = photonic_video::media::probe::interlaced_consequence(v.scan)
                .unwrap_or("Interlaced media may comb on progressive timelines.");
            ui.label(
                egui::RichText::new("INTERLACED")
                    .small()
                    .strong()
                    .color(egui::Color32::from_rgb(235, 180, 90)),
            )
            .on_hover_text(format!("{order}: {consequence}"));
        }
    }
    if let Some(proxy) = &asset.proxy {
        use photonic_core::timeline::ProxyStatus;
        let (txt, color) = match proxy.status {
            ProxyStatus::Ready => ("PROXY", egui::Color32::from_rgb(120, 200, 140)),
            ProxyStatus::Pending => ("PROXY…", egui::Color32::from_gray(150)),
            ProxyStatus::Failed => ("PROXY FAILED", egui::Color32::from_rgb(235, 150, 90)),
        };
        ui.label(egui::RichText::new(txt).small().color(color));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guess_asset_kind_covers_the_format_table() {
        assert_eq!(
            guess_asset_kind(Path::new("/a/clip.MP4")),
            Some(AssetKind::Video)
        );
        assert_eq!(
            guess_asset_kind(Path::new("music.flac")),
            Some(AssetKind::Audio)
        );
        assert_eq!(
            guess_asset_kind(Path::new("frame.JPeG")),
            Some(AssetKind::Image)
        );
        assert_eq!(
            guess_asset_kind(Path::new("art.photon")),
            Some(AssetKind::VectorDoc)
        );
        assert_eq!(
            guess_asset_kind(Path::new("look.cube")),
            Some(AssetKind::Lut3d)
        );
        assert_eq!(guess_asset_kind(Path::new("notes.txt")), None);
        assert_eq!(guess_asset_kind(Path::new("no_extension")), None);
    }

    #[test]
    fn display_name_is_file_stem() {
        let a = MediaAsset::from_file(AssetKind::Video, "/some/dir/My Clip.final.mp4");
        assert_eq!(asset_display_name(&a), "My Clip.final");
    }

    #[test]
    fn offline_is_missing_file() {
        let a = MediaAsset::from_file(AssetKind::Video, "/definitely/not/here.mp4");
        assert!(asset_is_offline(&a));
    }

    #[test]
    fn duration_formatting() {
        assert_eq!(format_duration(Tick(0)), "0:00.0");
        assert_eq!(format_duration(Tick(TICKS_PER_SECOND * 65)), "1:05.0");
        assert_eq!(
            format_duration(Tick(TICKS_PER_SECOND * 3 + TICKS_PER_SECOND / 2)),
            "0:03.5"
        );
        // Negative (corrupt) durations clamp rather than underflow.
        assert_eq!(format_duration(Tick(-5)), "0:00.0");
    }

    #[test]
    fn usage_count_counts_clip_refs() {
        use photonic_core::timeline::{
            Clip, ClipSource, FrameRate, Sequence, TimelineProject, Track, TrackKind, Tick,
        };
        let mut project = TimelineProject::new();
        let id = AssetId::new();
        let other = AssetId::new();
        let mut seq = Sequence::new("S", FrameRate::FPS_30, 320, 180);
        let mut track = Track::new(TrackKind::Video, "V1");
        track.clips.push(Clip::new(
            ClipSource::Asset { asset: id },
            Tick::ZERO,
            Tick(100),
        ));
        track.clips.push(Clip::new(
            ClipSource::Asset { asset: id },
            Tick(100),
            Tick(100),
        ));
        track.clips.push(Clip::new(
            ClipSource::Asset { asset: other },
            Tick(200),
            Tick(100),
        ));
        seq.video_tracks.push(track);
        let sid = seq.id;
        project.sequences.insert(sid, seq);
        assert_eq!(asset_usage_count(&project, id), 2);
        assert_eq!(asset_usage_count(&project, other), 1);
        assert_eq!(asset_usage_count(&project, AssetId::new()), 0);
    }

    #[test]
    fn probe_summary_surfaces_interlaced_scan() {
        use photonic_core::timeline::{
            FrameRate, MediaProbe, ProbedColor, ScanType, VideoStreamInfo,
        };
        let mut asset = MediaAsset::new(
            AssetKind::Video,
            AssetSource::File {
                path: PathBuf::from("/tmp/interlaced.mov"),
                rel_path: None,
            },
        );
        asset.probe = Some(MediaProbe {
            duration: Tick(TICKS_PER_SECOND),
            video: Some(VideoStreamInfo {
                width: 720,
                height: 480,
                frame_rate: FrameRate::new(30, 1),
                pixel_aspect: 1.0,
                color: ProbedColor::default(),
                keyframe_index_cached: false,
                scan: ScanType::InterlacedTopFirst,
            }),
            audio: None,
            container: "mov".into(),
            codec: "prores".into(),
        });
        let s = probe_summary(&asset);
        assert!(
            s.contains("interlaced (TFF)"),
            "probe_summary should surface K-G6 scan type: {s}"
        );
        assert!(v_scan_is_interlaced(&asset));
    }

    fn v_scan_is_interlaced(asset: &MediaAsset) -> bool {
        asset
            .probe
            .as_ref()
            .and_then(|p| p.video.as_ref())
            .is_some_and(|v| v.scan.is_interlaced())
    }

    #[test]
    fn bins_and_assets_filter_and_sort() {
        let mut pool = MediaPool::new();
        let root_bin = MediaBin::new("B", None);
        let child = MediaBin::new("A-child", Some(root_bin.id));
        let bins = vec![root_bin.clone(), child.clone(), MediaBin::new("A", None)];

        let kids = bin_children(&bins, None);
        assert_eq!(
            kids.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(),
            vec!["A", "B"]
        );
        assert_eq!(bin_children(&bins, Some(root_bin.id)).len(), 1);

        let mut filed = MediaAsset::from_file(AssetKind::Video, "/x/zzz.mp4");
        filed.bin = Some(root_bin.id);
        let unfiled_b = MediaAsset::from_file(AssetKind::Audio, "/x/bbb.wav");
        let unfiled_a = MediaAsset::from_file(AssetKind::Audio, "/x/aaa.wav");
        pool.insert(filed.clone());
        pool.insert(unfiled_b);
        pool.insert(unfiled_a);

        let root_assets = assets_in_bin(&pool, None);
        assert_eq!(
            root_assets
                .iter()
                .map(|a| asset_display_name(a))
                .collect::<Vec<_>>(),
            vec!["aaa", "bbb"]
        );
        let binned = assets_in_bin(&pool, Some(root_bin.id));
        assert_eq!(binned.len(), 1);
        assert_eq!(binned[0].id, filed.id);
    }

    #[test]
    fn probe_summary_handles_missing_probe() {
        let a = MediaAsset::from_file(AssetKind::Video, "/x/clip.mp4");
        assert_eq!(probe_summary(&a), "—");
    }

    /// 24 §2 / checklist §11.8: multi-select import yields **N L0 stubs** with
    /// no probe/hash before any L1–L2 work runs.
    #[test]
    fn l0_register_n_stubs_before_any_probe() {
        let t0 = std::time::Instant::now();
        let paths = vec![
            PathBuf::from("/media/a.mp4"),
            PathBuf::from("/media/b.mov"),
            PathBuf::from("/media/skip.txt"), // not a media kind
            PathBuf::from("/media/c.wav"),
        ];
        let stubs = l0_register_stubs(&paths, None);
        let l0_ms = t0.elapsed().as_millis();
        assert_eq!(stubs.len(), 3, "three media paths → three L0 rows");
        assert!(stubs.iter().all(|a| a.probe.is_none()));
        assert!(stubs.iter().all(|a| a.content_hash.is_none()));
        assert!(
            l0_ms < 100,
            "L0 multi-register must be UI-cheap (got {l0_ms} ms)"
        );
        // Distinct ids so concurrent AddAsset is safe.
        let mut ids: Vec<_> = stubs.iter().map(|a| a.id).collect();
        ids.sort_by_key(|i| i.0);
        ids.dedup();
        assert_eq!(ids.len(), 3);
    }
}
