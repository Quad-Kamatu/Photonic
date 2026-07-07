//! Bottom timeline panel (video-editor-module `04-ui-mode-timeline.md` §2).
//!
//! New module family mirroring the existing `panels/` split-by-concern
//! pattern. This skeleton only lands `mod.rs` (the bottom-panel entry point)
//! and `layout.rs` (zoom/scroll session state) — `ruler.rs`, `tracks.rs`,
//! `clips.rs`, `interact.rs`, `ops_bridge.rs` are P2-wave additions per §2.

pub mod layout;

pub use layout::TimelineView;

/// The bottom timeline panel's entry point. Stub: renders an empty panel
/// frame with a placeholder label — no ruler/tracks/clips yet.
///
/// The real signature will grow to `(ctx, app, doc, engine_status)` per §2
/// once `photonic_video`'s engine status type exists (02-engine.md, not yet
/// built); kept to just `ui` here so this skeleton has no dependency on
/// not-yet-committed engine types. Callers register the enclosing
/// `egui::TopBottomPanel` themselves (`app/mod.rs`, gated on
/// `self.mode == AppMode::Video`, §1.1).
pub(crate) fn draw_timeline_panel(ui: &mut egui::Ui) {
    ui.weak("Timeline — P2 wave fills this (04-ui-mode-timeline.md §2).");
}
