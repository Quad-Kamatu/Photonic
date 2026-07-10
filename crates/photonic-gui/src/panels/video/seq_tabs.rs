//! Sequence tab strip (17-nle-parity-round2.md §G-17) — lets multiple
//! sequences stay open as tabs (and a nested sequence, §G-16, open as one),
//! mirroring the document tab bar (`app/tabs.rs`) one level down. Rated
//! **Larger** in the round-2 gap list; the real surface is a
//! `timeline-panel` build embedded in the timeline panel's own header
//! (`app/timeline/mod.rs`), out of this panel's reach.
//!
//! Unlike the other round-2 stubs this is **not** a `DrawerGroup` panel — no
//! rail icon owns it, since it lives inline in the timeline header rather
//! than behind a drawer toggle. It still takes [`VideoPanelUi`] directly
//! (like `audio_mixer.rs`) so its session state
//! (`open_sequence_tabs`/`nested_sequence_breadcrumbs`) has a stable home
//! while the timeline-panel story is unwritten.
//!
//! Stub — filled by the sequence-tabs (17 G-17) timeline-panel story.

use super::VideoPanelUi;
use egui::Ui;

/// Sequence tab strip, meant to be called from the timeline panel's header.
// Not yet called anywhere (no rail icon owns it — see module doc); the
// timeline-panel story wires the call site in `app/timeline/mod.rs`, out of
// this skeleton's territory.
#[allow(dead_code)]
pub(crate) fn draw_seq_tabs(_ui: &mut Ui, _vid: &mut VideoPanelUi) {
    // Sequence-tabs (17 G-17) timeline-panel story fills this.
}
