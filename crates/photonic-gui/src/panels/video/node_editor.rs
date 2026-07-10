//! Node editor (04 §4.1 / 08 §6.1), split across two surfaces:
//! - the left-rail `DrawerGroup::NodeEditor` drawer — add-node palette, selected
//!   node inspector, graph info ([`draw_node_editor_palette`]);
//! - the central-panel node canvas content state — the egui-snarl graph canvas +
//!   viewer inset that replaces the program monitor while a composition is being
//!   edited ([`draw_node_canvas`]).
//!
//! Both interiors are owned by 08-fusion-node-flows.md; the full canvas lands in
//! `app/node_editor/` per 08 §7. Open-graph / selected-node / active state lives
//! on [`super::VideoPanelUi`].
//!
//! Stub — filled by the node-editor (08) panel story.

use super::VideoPanelUi;
use crate::panels::PropPanelCtx;
use egui::Ui;
use photonic_core::Document;

/// Left-rail Node Editor drawer: palette + selected-node inspector + graph info.
/// NOT the graph canvas (that is [`draw_node_canvas`]).
pub(crate) fn draw_node_editor_palette(_ui: &mut Ui, _ctx: &mut PropPanelCtx) {
    // Node-editor (08) palette/inspector story fills this.
}

/// Central-panel node canvas content state (08 §6.1), drawn in place of the
/// program monitor while [`VideoPanelUi::node_canvas_active`] is set. The
/// "Back to Timeline" escape (button + `Esc`) clears that flag.
pub(crate) fn draw_node_canvas(_ui: &mut Ui, _doc: &Document, _vid: &mut VideoPanelUi) {
    // Node-editor (08) canvas story fills this (egui-snarl canvas + viewer inset).
}
