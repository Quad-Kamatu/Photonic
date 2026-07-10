//! `DrawerGroup::ClipInspector` panel (04 §4.1) — the selected clip's
//! transform / speed / effects-stack / transition params, the `Clip`/`ClipEffect`
//! analogue of the vector Inspector. Panel shell owned by 04; widgets source
//! from `prop_registry` (01 §6.2). Keyframe-editor targeting lives on
//! [`super::VideoPanelUi::keyframe_editor_target`].
//!
//! Stub — filled by the clip-inspector (04) panel story.

use crate::panels::PropPanelCtx;
use egui::Ui;

/// Left-rail Clip Inspector drawer. Reads the timeline selection and clip data
/// via `ctx` (`ctx.video.selection`, `ctx.doc`).
pub(crate) fn draw_clip_inspector(_ui: &mut Ui, _ctx: &mut PropPanelCtx) {
    // Clip-inspector (04) panel story fills this.
}
