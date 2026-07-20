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
    AssetId, AssetKind, AssetSource, BinId, MediaAsset, MediaBin, MediaPool, ProxyRef,
    ProxyStatus, Tick, TICKS_PER_SECOND,
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
    pub new_bin_name: String,
    /// L1–L3 jobs still in flight (24-preview-media-load ladder).
    pub importing: usize,
    meta_tx: mpsc::Sender<ImportMetaResult>,
    meta_rx: mpsc::Receiver<ImportMetaResult>,
    /// Proxy transcodes currently running. Separate from importing: ffmpeg
    /// work must never block the editor thread or compete with asset probes.
    pub proxying: usize,
    proxy_tx: mpsc::Sender<ProxyJobResult>,
    proxy_rx: mpsc::Receiver<ProxyJobResult>,
    /// content_hash → poster PNG for grid/monitor paint (session).
    pub posters: std::collections::HashMap<String, PathBuf>,
    /// content_hash → L4 keyframe index ready.
    pub keyframe_ready: std::collections::HashSet<String>,
    /// Loaded egui textures for poster paths (path string → TextureHandle).
    poster_textures: std::collections::HashMap<String, egui::TextureHandle>,
}

/// Completed background proxy job, applied by the app through an undoable
/// `SetAssetProxy` command on its next UI frame.
pub struct ProxyJobResult {
    pub asset: AssetId,
    pub proxy: Option<ProxyRef>,
}

/// L1–L4 fill for an already-registered L0 asset (24 §2).
pub struct ImportMetaResult {
    pub asset: AssetId,
    pub probe: Option<photonic_core::timeline::MediaProbe>,
    pub content_hash: Option<String>,
    pub poster_path: Option<PathBuf>,
    /// L4 keyframe index written (video only).
    pub keyframe_index: bool,
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
            new_bin_name: String::new(),
            importing: 0,
            meta_tx,
            meta_rx,
            proxying: 0,
            proxy_tx,
            proxy_rx,
            posters: std::collections::HashMap::new(),
            keyframe_ready: std::collections::HashSet::new(),
            poster_textures: std::collections::HashMap::new(),
        }
    }
}

impl MediaPoolUi {
    /// L0-first import (24 §2): returns stubs for immediate `AddAsset`, then
    /// runs hash → probe → poster → keyframe index on a worker.
    pub fn spawn_import(
        &mut self,
        paths: Vec<PathBuf>,
        bin: Option<BinId>,
        project_path: Option<PathBuf>,
    ) -> Vec<MediaAsset> {
        let jobs: Vec<(MediaAsset, PathBuf)> = paths
            .into_iter()
            .filter_map(|path| {
                let kind = guess_asset_kind(&path)?;
                let mut asset = MediaAsset::from_file(kind, &path);
                asset.bin = bin;
                Some((asset, path))
            })
            .collect();
        if jobs.is_empty() {
            return Vec::new();
        }
        self.importing += jobs.len();
        let stubs: Vec<MediaAsset> = jobs.iter().map(|(a, _)| a.clone()).collect();
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
                        let out =
                            photonic_video::media::poster::poster_cache_path(&cache_dir, h);
                        match photonic_video::media::poster::ensure_poster(tools, &path, &out)
                        {
                            Ok(p) => Some(p),
                            Err(e) => {
                                tracing::debug!(
                                    "media import: poster failed for {path:?}: {e}"
                                );
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
                if meta_tx
                    .send(ImportMetaResult {
                        asset: asset_id,
                        probe,
                        content_hash: hash,
                        poster_path,
                        keyframe_index,
                    })
                    .is_err()
                {
                    return;
                }
            }
        });
        stubs
    }

    /// Drain L1–L4 completions. Caller applies `set_asset_meta`.
    pub fn drain_meta(&mut self) -> Vec<ImportMetaResult> {
        let mut out = Vec::new();
        while let Ok(meta) = self.meta_rx.try_recv() {
            self.importing = self.importing.saturating_sub(1);
            if let Some(hash) = &meta.content_hash {
                if let Some(path) = &meta.poster_path {
                    self.posters.insert(hash.clone(), path.clone());
                }
                if meta.keyframe_index {
                    self.keyframe_ready.insert(hash.clone());
                }
            }
            out.push(meta);
        }
        out
    }

    /// Ensure a poster texture is loaded for `hash` (if a path is known).
    fn poster_texture(
        &mut self,
        ctx: &egui::Context,
        hash: &str,
    ) -> Option<egui::TextureHandle> {
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
        self.poster_textures.insert(hash.to_string(), handle.clone());
        Some(handle)
    }

    /// Build all supplied file-backed video proxies sequentially on one worker.
    /// The worker yields CPU priority inside `generate_proxy`; completion is
    /// handed back to the UI so document/history mutation stays on the main
    /// thread. Existing ready cache files are reused without re-encoding.
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
            })
            .collect();
        if assets.is_empty() {
            return;
        }
        self.proxying += assets.len();
        let tx = self.proxy_tx.clone();
        std::thread::spawn(move || {
            let tools = photonic_video::media::ffmpeg_locate::locate().ok();
            let cache_dir = photonic_video::media::proxy::proxy_cache_dir(project_path.as_deref());
            for asset in assets {
                let AssetSource::File { path, .. } = &asset.source else {
                    continue;
                };
                let hash = asset.content_hash.clone().or_else(|| {
                    photonic_video::media::probe::content_hash(path).ok()
                });
                let proxy = match (tools.as_ref(), hash) {
                    (Some(tools), Some(hash)) => {
                        let output = photonic_video::media::proxy::proxy_cache_path(&cache_dir, &hash);
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
                        Some(ProxyRef { path: output, status })
                    }
                    // A missing toolchain/hash must not detach a previously
                    // ready proxy; preserve the asset's existing attachment.
                    _ => asset.proxy.clone(),
                };
                if tx.send(ProxyJobResult { asset: asset.id, proxy }).is_err() {
                    return;
                }
            }
        });
    }

    /// Drain proxy completions. The caller applies them through timeline
    /// commands so undo/redo and the engine mirror stay coherent.
    pub fn drain_finished_proxies(&mut self) -> Vec<ProxyJobResult> {
        let mut out = Vec::new();
        while let Ok(result) = self.proxy_rx.try_recv() {
            self.proxying = self.proxying.saturating_sub(1);
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

    // ── Proxy playback mode (engine-wide toggle, 05 §4) ─────────────────────
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
        if ui
            .add_enabled(ctx.media_ui.proxying == 0, egui::Button::new("Build proxies"))
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
    let assets = assets_in_bin(pool, ctx.media_ui.current_bin);
    // Assets referenced by any clip on any sequence — badged "ON TL" so the user
    // can tell which imported media is actually placed on the timeline.
    let used_on_timeline: std::collections::HashSet<_> = project
        .sequences
        .values()
        .flat_map(|s| s.video_tracks.iter().chain(s.audio_tracks.iter()))
        .flat_map(|t| t.clips.iter())
        .filter_map(|c| c.source.asset())
        .collect();
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
                        let used = used_on_timeline.contains(&asset.id);
                        draw_asset_cell(ui, ctx, asset, true, &pool.bins, used);
                    }
                });
            } else {
                for asset in &assets {
                    let used = used_on_timeline.contains(&asset.id);
                    draw_asset_cell(ui, ctx, asset, false, &pool.bins, used);
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
fn draw_asset_cell(
    ui: &mut Ui,
    ctx: &mut PropPanelCtx,
    asset: &MediaAsset,
    grid: bool,
    bins: &[MediaBin],
    used: bool,
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
                        badges(ui, asset, offline, used);
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
                            badges(ui, asset, offline, used);
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

fn badges(ui: &mut Ui, asset: &MediaAsset, offline: bool, used: bool) {
    if used {
        ui.label(
            egui::RichText::new("ON TL")
                .small()
                .strong()
                .color(egui::Color32::from_rgb(120, 200, 140)),
        )
        .on_hover_text("Used on the timeline");
    }
    if offline {
        ui.label(
            egui::RichText::new("OFFLINE")
                .small()
                .strong()
                .color(egui::Color32::from_rgb(235, 100, 90)),
        );
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
}
