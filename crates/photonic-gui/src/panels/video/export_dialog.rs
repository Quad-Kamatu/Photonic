//! Video export dialog (04 §4.1 / 05 §3) — a floating window (like the vector
//! `ExportDialog`, distinct from it) for picking a preset + reframe options and
//! launching an export. Interior owned by 05-import-export.md. Open state and
//! the last-used preset live on [`super::VideoPanelUi::export_dialog_open`] /
//! [`super::VideoPanelUi::last_export_preset`]; the menu/toolbar entry that sets
//! `export_dialog_open` is wired in `app/mod.rs`.
//!
//! Stub — filled by the export-dialog (05) panel story.

use super::VideoPanelUi;

/// Floating video export dialog, shown while
/// [`VideoPanelUi::export_dialog_open`] is set. Its own window close button
/// clears that flag.
pub(crate) fn draw_export_dialog(_ctx: &egui::Context, _vid: &mut VideoPanelUi) {
    // Export-dialog (05) panel story fills this (preset picker + reframe body).
}
