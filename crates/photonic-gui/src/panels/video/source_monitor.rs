//! Source marks panel (G-10 / 24 D-PM-1–3) — **not** a second viewer.
//!
//! The single central monitor peeks source when armed (`PreviewTarget::Asset`).
//! This drawer surfaces mark In/Out state and shortcuts for 3-point edit.

use crate::panels::PropPanelCtx;
use egui::Ui;
use egui_phosphor::regular as ph;
use photonic_core::timeline::Tick;

/// Left-rail Source Marks drawer (session state only).
pub(crate) fn draw_source_monitor(ui: &mut Ui, ctx: &mut PropPanelCtx) {
    ui.label(egui::RichText::new("Source marks").strong().size(13.0));
    ui.label(
        egui::RichText::new(
            "Single monitor: pool click peeks SOURCE; I/O set marks; \
             , / . Insert/Overwrite at playhead.",
        )
        .weak()
        .small(),
    );
    ui.separator();

    // PropPanelCtx does not hold source_marks directly — show guidance when
    // the panel is open without the full app. Marks chrome lives on the
    // transport bar; this drawer is a quick reference + clear action via
    // command id if wired later.
    let _ = ctx;
    ui.horizontal(|ui| {
        ui.label(ph::MONITOR_PLAY);
        ui.label("Peek: click media pool row");
    });
    ui.horizontal(|ui| {
        ui.label(ph::FLAG);
        ui.label("I / O — source In/Out when SOURCE badge shows");
    });
    ui.horizontal(|ui| {
        ui.label(ph::ARROW_RIGHT);
        ui.label(", Insert · . Overwrite · F Match Frame");
    });
    ui.separator();
    ui.label(
        egui::RichText::new(format!(
            "Work range (sequence) uses I/O while SEQUENCE is focused. \
             Source ticks are session-only (not undoable). Tick unit: {} µs.",
            Tick(1).0
        ))
        .weak()
        .small(),
    );
}
