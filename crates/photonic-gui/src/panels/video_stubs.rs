//! Video-mode drawer section stubs (video-editor-module `04-ui-mode-timeline.md`
//! §4.1). Empty bodies — each is filled in by its P2-wave owner (noted per fn)
//! without touching `panels/mod.rs`'s `draw_drawer` dispatch or the right-drawer
//! match in `app/mod.rs`.

use super::PropPanelCtx;
use egui::Ui;

// ── Left rail (`DrawerGroup`) ─────────────────────────────────────────────────
//
// `MediaPool` graduated to `panels/media_pool.rs` (05 §2 interior).

/// `DrawerGroup::ClipInspector` shell. Owner: this doc (04); widgets source
/// from `prop_registry` (01 §6.2).
pub(crate) fn draw_clip_inspector(_ui: &mut Ui, _ctx: &mut PropPanelCtx) {
    // P2 wave fills this.
}

/// `DrawerGroup::Effects` shell. Owner: 08-fusion-node-flows.md.
pub(crate) fn draw_effects_browser(_ui: &mut Ui, _ctx: &mut PropPanelCtx) {
    // P2 wave fills this.
}

/// `DrawerGroup::Captions` shell. Owner: 06-captions-ai.md.
pub(crate) fn draw_captions_panel(_ui: &mut Ui, _ctx: &mut PropPanelCtx) {
    // P2 wave fills this.
}

/// `DrawerGroup::NodeEditor` shell (palette + inspector only — the graph
/// canvas lives in the central panel, 08 §6.1). Owner: 08-fusion-node-flows.md.
pub(crate) fn draw_node_editor_palette(_ui: &mut Ui, _ctx: &mut PropPanelCtx) {
    // P2 wave fills this.
}

// ── Right rail (`RightDrawerGroup`) ───────────────────────────────────────────
// Called directly from `app/mod.rs`'s right-drawer match, which doesn't build
// a `PropPanelCtx` (unlike the left drawer's `draw_drawer` dispatch).

/// `RightDrawerGroup::ColorControls` shell. Owner: 07-color-grading.md.
pub(crate) fn draw_color_controls(_ui: &mut Ui) {
    // P2 wave fills this.
}

/// `RightDrawerGroup::AudioMixer` shell. Owner: 09-audio-mixer.md.
pub(crate) fn draw_audio_mixer(_ui: &mut Ui) {
    // P2 wave fills this.
}
