//! `DrawerGroup::Transcript` panel (17-nle-parity-round2.md §G-18) —
//! text-based (transcript) editing: select a word range in the transcript and
//! ripple-delete/reorder the matching timeline clip range, plus a one-click
//! filler-word ("um"/"uh") filter. Builds on the existing auto-caption infra
//! (`captions/` — the transcript text itself already exists); the new part is
//! wiring transcript spans to timeline ripple ops, out of this panel's reach.
//! This left-rail entry exists so the drawer is reachable and its session
//! state (`VideoPanelUi::transcript_panel_open` / `transcript_scroll`) has a
//! stable home while that story is unwritten.
//!
//! Stub — filled by the transcript (17 G-18) panel story.

use crate::panels::PropPanelCtx;
use egui::Ui;

/// Left-rail Transcript drawer.
pub(crate) fn draw_transcript(_ui: &mut Ui, _ctx: &mut PropPanelCtx) {
    // Transcript (17 G-18) panel story fills this.
}
