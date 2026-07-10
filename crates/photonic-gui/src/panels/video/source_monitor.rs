//! `DrawerGroup::SourceMonitor` panel (17-nle-parity-round2.md §G-10) — a
//! second preview surface showing the raw armed asset (`PendingSource`, spec
//! 16 §1) with its own scrub bar and in/out marks, separate from the program
//! monitor. Rated **Larger** in the round-2 gap list; the real surface is a
//! `monitor`-territory build (`app/monitor.rs`, mode-adaptive per 04 §D-02),
//! out of this panel's reach — this left-rail entry exists so the drawer is
//! reachable and its session state (`VideoPanelUi::source_monitor_scrub`) has
//! a stable home while that story is unwritten.
//!
//! Stub — filled by the source-monitor (17 G-10) panel story.

use crate::panels::PropPanelCtx;
use egui::Ui;

/// Left-rail Source Monitor drawer.
pub(crate) fn draw_source_monitor(_ui: &mut Ui, _ctx: &mut PropPanelCtx) {
    // Source-monitor (17 G-10) panel story fills this.
}
