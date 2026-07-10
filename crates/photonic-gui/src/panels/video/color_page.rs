//! Color page (04 §4.1 / 07 §6), split across two surfaces:
//! - the right-drawer `RightDrawerGroup::ColorControls` group — wheels / curves /
//!   HSL qualifier / LUT browser for the selected clip's grade
//!   ([`draw_color_controls`]);
//! - the floating, dockable scopes panel — waveform / parade / vectorscope /
//!   histogram, parked beside the program monitor ([`draw_scopes_panel`]).
//!
//! Both interiors are owned by 07-color-grading.md. Grade-op selection, active
//! tab, scopes visibility, and scope kind live on [`super::VideoPanelUi`] (and
//! the [`super::ColorPageTab`] / [`super::ScopeKind`] enums).
//!
//! Stub — filled by the color-page (07) panel story.

use super::VideoPanelUi;
use egui::Ui;

/// Right-rail Color Controls drawer: wheels / curves / qualifier / LUT for the
/// selected clip's grade, plus the scopes-panel toggle. Called directly from
/// `app/mod.rs`'s right-drawer match (no `PropPanelCtx`).
pub(crate) fn draw_color_controls(_ui: &mut Ui, _vid: &mut VideoPanelUi) {
    // Color-page (07) controls story fills this.
}

/// Floating / dockable scopes panel (07 §6), shown while
/// [`VideoPanelUi::scopes_panel_open`] is set. Its own window close button
/// clears that flag; the active scope is [`VideoPanelUi::scope_kind`].
pub(crate) fn draw_scopes_panel(_ctx: &egui::Context, _vid: &mut VideoPanelUi) {
    // Color-page (07) scopes story fills this.
}
