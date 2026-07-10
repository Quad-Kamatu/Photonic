//! Video-mode panel skeletons (video-editor-module `04-ui-mode-timeline.md`
//! §4.1 panel map). This module is the choke-point split of the former
//! `panels/video_stubs.rs`: one file per panel, each with a stub `draw` fn and
//! a "filled by <phase> panel story" marker so the six panel builders work
//! disjointly without touching `panels/mod.rs`'s `draw_drawer` dispatch, the
//! `PhotonicApp` struct, or each other.
//!
//! Panel → owning spec:
//! - [`clip_inspector`]  — 04-ui-mode-timeline.md (shell) / 01 §6.2 (widgets)
//! - [`effects_browser`] — 08-fusion-node-flows.md
//! - [`caption_editor`]  — 06-captions-ai.md
//! - [`color_page`]      — 07-color-grading.md (right-drawer controls + scopes)
//! - [`node_editor`]     — 08-fusion-node-flows.md (left palette + central canvas)
//! - [`audio_mixer`]     — 09-audio-mixer.md
//! - [`export_dialog`]   — 05-import-export.md

use std::collections::HashSet;

use photonic_core::timeline::{ClipId, CueId, GradeOpId, GraphId, GraphNodeId, TrackId};

pub(crate) mod audio_mixer;
pub(crate) mod caption_editor;
pub(crate) mod clip_inspector;
pub(crate) mod color_page;
pub(crate) mod effects_browser;
pub(crate) mod export_dialog;
pub(crate) mod keyframe_editor;
pub(crate) mod node_editor;

/// Which sub-section of the right-drawer Color Controls group is active
/// (07 §6). Owned by the color page story; defined here so the shared
/// [`VideoPanelUi`] / `PhotonicApp` field type is stable while that story is
/// unwritten.
#[allow(dead_code)] // variants beyond the default are constructed by the color-page story.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub(crate) enum ColorPageTab {
    /// Lift/gamma/gain wheels (07 §3.5).
    #[default]
    Wheels,
    /// Master + per-channel curve editor (07 §3.6).
    Curves,
    /// HSL qualifier secondary (07 §3.7).
    Qualifier,
    /// 3D LUT browser / intensity (07 §3.8).
    Lut,
}

/// Which scope the floating scopes panel renders (07 §6, `scopes.rs`). Owned by
/// the color page story; defined here for field-type stability (see
/// [`ColorPageTab`]).
#[allow(dead_code)] // variants beyond the default are constructed by the color-page story.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub(crate) enum ScopeKind {
    /// Per-column intensity waveform (07 §6 waveform).
    #[default]
    Waveform,
    /// RGB parade (three side-by-side waveforms).
    Parade,
    /// Cb/Cr vectorscope with skin-tone line (07 §6 vectorscope).
    Vectorscope,
    /// Luma/RGB histogram (07 §6 histogram).
    Histogram,
}

/// The slice of `PhotonicApp` video-editor session state the video panels read
/// and mutate, threaded to every video panel stub so panel builders never touch
/// the `PhotonicApp` struct or the `draw_*` call sites.
///
/// Left-rail drawers reach it through [`super::PropPanelCtx::video`]; the
/// right-rail drawers, the floating scopes / export panels, and the central
/// node canvas receive `&mut VideoPanelUi` directly (built by
/// `PhotonicApp::video_panel_ui`).
///
/// Every field carries a `[panel]` tag naming its owning story.
#[allow(dead_code)] // fields are the wired API surface consumed as each panel story is filled in.
pub(crate) struct VideoPanelUi<'a> {
    /// Clips selected in the timeline (04 §2.6) — read-only targeting for the
    /// clip inspector / effects / color panels.
    pub(crate) selection: &'a [ClipId],
    /// [color_page] selected grade op in the clip's grade stack (07 §1).
    pub(crate) selected_grade_op: &'a mut Option<GradeOpId>,
    /// [color_page] active Color Controls sub-tab (07 §6).
    pub(crate) color_page_tab: &'a mut ColorPageTab,
    /// [color_page] floating scopes panel visibility (07 §6).
    pub(crate) scopes_panel_open: &'a mut bool,
    /// [color_page] which scope the floating panel shows (07 §6).
    pub(crate) scope_kind: &'a mut ScopeKind,
    /// [node_editor] graph open in the central node canvas, if any (08 §6.1).
    pub(crate) open_graph: &'a mut Option<GraphId>,
    /// [node_editor] selected node in the open graph (08 §6.1 inspector).
    pub(crate) selected_graph_node: &'a mut Option<GraphNodeId>,
    /// [node_editor] whether the central panel shows the node canvas instead of
    /// the program monitor (08 §6.1 central-panel content state).
    pub(crate) node_canvas_active: &'a mut bool,
    /// [audio_mixer] track strips whose EQ/comp/automation are expanded (09).
    pub(crate) mixer_expanded_tracks: &'a mut HashSet<TrackId>,
    /// [clip_inspector] clip whose animated props the keyframe editor targets
    /// (04 §4.1 / 01 §6).
    pub(crate) keyframe_editor_target: &'a mut Option<ClipId>,
    /// [caption_editor] cue currently being edited (06).
    pub(crate) caption_edit_cue: &'a mut Option<CueId>,
    /// [export_dialog] whether the video export dialog is open (05 §3).
    pub(crate) export_dialog_open: &'a mut bool,
    /// [export_dialog] name of the last-used export preset, seed for the dialog
    /// (05 §3). Session-only here; the export story persists it to prefs.
    pub(crate) last_export_preset: &'a mut String,
}
