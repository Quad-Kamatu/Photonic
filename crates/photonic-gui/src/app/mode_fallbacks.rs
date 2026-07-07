//! TEMP fallbacks for `PhotonicApp` methods the P2-wave timeline-panel story
//! (`app/timeline/interact.rs` + `ops_bridge.rs`, video-editor-module
//! `04-ui-mode-timeline.md` §2.3/§2.5/§5.1) is expected to add for real.
//! `app/command_center.rs`'s `video.*` command dispatch calls these exact
//! method names so the palette/keyboard wiring for the mode-switch story
//! compiles and is exercisable end-to-end today, ahead of the timeline
//! builder's own implementation landing on `PhotonicApp`.
//!
//! ORCHESTRATOR: once `prev_edit_point`/`next_edit_point`/`split_at_playhead`/
//! `toggle_snap`/`zoom_in`/`zoom_out`/`zoom_fit` exist for real on
//! `PhotonicApp` (wherever the timeline-panel story lands them), DELETE THIS
//! FILE and the `mod mode_fallbacks;` line in `app/mod.rs` — leaving both in
//! place would be a duplicate-inherent-method compile error. No other change
//! is needed in `command_center.rs`; the call sites there already call these
//! exact names, so the real methods take over transparently.
#![allow(dead_code)]

use super::*;

impl PhotonicApp {
    /// TEMP no-op. Real impl: move `self.playhead` to the previous clip edge
    /// or marker at/before the current playhead on the active track(s) (04
    /// §5.1 `Shift+←`).
    pub(crate) fn prev_edit_point(&mut self, _doc: &Document) {}

    /// TEMP no-op. Mirror of `prev_edit_point`, searching forward (`Shift+→`).
    pub(crate) fn next_edit_point(&mut self, _doc: &Document) {}

    /// TEMP no-op. Real impl: for each clip in `self.timeline_selection`
    /// covering `self.playhead`, call `photonic_core::timeline::ops::split_clip`
    /// and push the result(s) through `history` (04 §2.3 `S` key / §5.1
    /// `video.split_at_playhead`).
    pub(crate) fn split_at_playhead(&mut self, _doc: &mut Document, _history: &mut CommandHistory) {
    }

    /// TEMP no-op. Real impl: flip `self.timeline_snap_enabled` (mirrored to
    /// `prefs.timeline_snap_enabled` like the existing toggle-persistence
    /// pattern, 04 §2.5 `N` key).
    pub(crate) fn toggle_snap(&mut self) {}

    /// TEMP no-op. Real impl: adjust `self.timeline_view.pixels_per_tick`
    /// within its clamped range (04 §2.1 `+` key).
    pub(crate) fn zoom_in(&mut self) {}

    /// TEMP no-op. Mirror of `zoom_in` (`-` key).
    pub(crate) fn zoom_out(&mut self) {}

    /// TEMP no-op. Real impl: fit `work_range` (or the full sequence extent
    /// when unset) to the timeline lane width (04 §2.1, `Shift+Z`).
    pub(crate) fn zoom_fit(&mut self) {}
}
