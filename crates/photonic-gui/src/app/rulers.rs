//! Interactive rulers: drag-to-create guides, guide editing (move / delete /
//! exact-position popup), a live cursor readout on both rulers, and a unit
//! selector in the corner box. See proposal #70 / PR #141.
//!
//! All guide mutations go through `Command::SetGuides { old, new }` so every
//! create / move / delete / exact-edit is a single undoable step.

use super::*;
use photonic_core::{DocumentUnit, Guide, GuideOrientation};

/// Assumed screen resolution for px↔unit conversion. Document-level DPI metadata
/// is out of scope here (see proposal "Remaining work").
pub(crate) const RULER_DPI: f64 = 96.0;

/// Height/width of the ruler strips and the corner box, in screen pixels.
const RULER_H: f32 = 18.0;

/// Hit tolerance (screen px) for grabbing a guide line on the canvas.
const GUIDE_GRAB_PX: f32 = 4.0;

/// State for the exact-position editor popup opened by double-clicking a guide.
#[derive(Debug, Clone)]
pub(crate) struct GuideEditPopup {
    /// Stable id of the guide being edited.
    pub guide_id: uuid::Uuid,
    /// Orientation, used to label the field (X for vertical, Y for horizontal).
    pub orientation: GuideOrientation,
    /// Edited value, expressed in the current document unit.
    pub value: f64,
}

impl PhotonicApp {
    /// Format a canvas-pixel position as a ruler label string in the current
    /// document unit (integer for px, one decimal otherwise).
    pub(crate) fn format_ruler_value(&self, px: f64) -> String {
        let unit = self.prefs.document_units;
        let v = photonic_core::from_px(px, unit, RULER_DPI);
        if matches!(unit, DocumentUnit::Px) {
            format!("{}", v.round() as i64)
        } else {
            format!("{:.1}", v)
        }
    }

    /// Drive all ruler/guide interaction for one frame. Called from the canvas
    /// render pass after the ruler strips + ticks have been painted, so the
    /// readout, previews, and corner selector draw on top of them.
    ///
    /// `canvas_rect` is the full central-panel rect (rulers occupy the top and
    /// left `RULER_H` strips of it).
    pub(crate) fn handle_ruler_interaction(
        &mut self,
        ui: &egui::Ui,
        canvas_rect: egui::Rect,
        view: &CanvasView,
        doc: &mut Document,
        history: &mut CommandHistory,
    ) {
        if !self.prefs.show_rulers {
            // Drop any stale interaction state if rulers were just hidden.
            self.ruler_drag = None;
            self.guide_dragging = None;
            self.guide_drag_old = None;
            return;
        }

        let painter = ui.painter_at(canvas_rect);
        let unit = self.prefs.document_units;

        let top = canvas_rect.min.y;
        let left = canvas_rect.min.x;
        let body_left = left + RULER_H;
        let body_top = top + RULER_H;

        let h_ruler = egui::Rect::from_min_max(
            egui::pos2(body_left, top),
            egui::pos2(canvas_rect.max.x, body_top),
        );
        let v_ruler = egui::Rect::from_min_max(
            egui::pos2(left, body_top),
            egui::pos2(body_left, canvas_rect.max.y),
        );
        let corner = egui::Rect::from_min_size(canvas_rect.min, egui::vec2(RULER_H, RULER_H));

        // ── Live cursor readout on both rulers ───────────────────────────────
        if self.ruler_drag.is_none() && self.guide_dragging.is_none() {
            if let Some(p) = ui.input(|i| i.pointer.hover_pos()) {
                if canvas_rect.contains(p) {
                    let (cx, cy) = view.screen_to_canvas(p.x as f64, p.y as f64);
                    let marker = egui::Color32::from_rgb(255, 110, 70);
                    let stroke = egui::Stroke::new(1.0, marker);
                    let font = egui::FontId::proportional(8.0);
                    if p.x > body_left {
                        painter.line_segment(
                            [egui::pos2(p.x, top), egui::pos2(p.x, body_top)],
                            stroke,
                        );
                        painter.text(
                            egui::pos2(p.x + 2.0, top + 9.0),
                            egui::Align2::LEFT_CENTER,
                            self.format_ruler_value(cx),
                            font.clone(),
                            marker,
                        );
                    }
                    if p.y > body_top {
                        painter.line_segment(
                            [egui::pos2(left, p.y), egui::pos2(body_left, p.y)],
                            stroke,
                        );
                        painter.text(
                            egui::pos2(left + 1.0, p.y + 1.0),
                            egui::Align2::LEFT_TOP,
                            self.format_ruler_value(cy),
                            font,
                            marker,
                        );
                    }
                }
            }
        }

        // ── Drag out of a ruler strip to create a new guide ──────────────────
        let h_resp = ui.interact(
            h_ruler,
            egui::Id::new("ruler_strip_h"),
            egui::Sense::click_and_drag(),
        );
        let v_resp = ui.interact(
            v_ruler,
            egui::Id::new("ruler_strip_v"),
            egui::Sense::click_and_drag(),
        );
        if h_resp.hovered() || v_resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        }
        if h_resp.drag_started() && self.guide_dragging.is_none() {
            self.ruler_drag = Some(GuideOrientation::Horizontal);
            self.guide_drag_old = Some(doc.guides.clone());
        }
        if v_resp.drag_started() && self.guide_dragging.is_none() {
            self.ruler_drag = Some(GuideOrientation::Vertical);
            self.guide_drag_old = Some(doc.guides.clone());
        }
        if let Some(orient) = self.ruler_drag {
            let resp = match orient {
                GuideOrientation::Horizontal => &h_resp,
                GuideOrientation::Vertical => &v_resp,
            };
            if let Some(p) = ui.input(|i| i.pointer.interact_pos()) {
                let (cx, cy) = view.screen_to_canvas(p.x as f64, p.y as f64);
                let preview = egui::Stroke::new(
                    1.0,
                    egui::Color32::from_rgba_unmultiplied(0, 200, 200, 200),
                );
                match orient {
                    GuideOrientation::Horizontal => {
                        self.ruler_drag_pos = cy;
                        painter.line_segment(
                            [egui::pos2(canvas_rect.min.x, p.y), egui::pos2(canvas_rect.max.x, p.y)],
                            preview,
                        );
                    }
                    GuideOrientation::Vertical => {
                        self.ruler_drag_pos = cx;
                        painter.line_segment(
                            [egui::pos2(p.x, canvas_rect.min.y), egui::pos2(p.x, canvas_rect.max.y)],
                            preview,
                        );
                    }
                }
                self.draw_drag_label(&painter, p, self.ruler_drag_pos, unit);
            }
            if resp.drag_stopped() {
                let pos = self.ruler_drag_pos;
                let release = ui.input(|i| i.pointer.interact_pos());
                let in_canvas = release
                    .map(|p| p.x > body_left && p.y > body_top)
                    .unwrap_or(false);
                if let Some(old) = self.guide_drag_old.take() {
                    if in_canvas {
                        let mut new_guides = old.clone();
                        new_guides.push(Guide::new(orient, pos));
                        history.execute(Command::SetGuides { old, new: new_guides }, doc);
                    }
                }
                self.ruler_drag = None;
            }
        }

        // ── Existing-guide interaction: move / delete / double-click edit ────
        // Snapshot the (index, id, orientation, position) of each draggable
        // guide so we can mutate `doc.guides` by index inside the loop.
        let guide_hits: Vec<(usize, uuid::Uuid, GuideOrientation, f64)> = doc
            .guides
            .iter()
            .enumerate()
            .filter(|(_, g)| g.angle_degrees.is_none() && !g.locked)
            .map(|(i, g)| (i, g.id, g.orientation, g.position))
            .collect();

        for (idx, id, orient, pos) in guide_hits {
            // Don't put a grab handle on the guide currently being created.
            let hit_rect = match orient {
                GuideOrientation::Horizontal => {
                    let (_, sy) = view.canvas_to_screen(0.0, pos);
                    let sy = sy as f32;
                    if sy < body_top || sy > canvas_rect.max.y {
                        continue;
                    }
                    egui::Rect::from_min_max(
                        egui::pos2(body_left, sy - GUIDE_GRAB_PX),
                        egui::pos2(canvas_rect.max.x, sy + GUIDE_GRAB_PX),
                    )
                }
                GuideOrientation::Vertical => {
                    let (sx, _) = view.canvas_to_screen(pos, 0.0);
                    let sx = sx as f32;
                    if sx < body_left || sx > canvas_rect.max.x {
                        continue;
                    }
                    egui::Rect::from_min_max(
                        egui::pos2(sx - GUIDE_GRAB_PX, body_top),
                        egui::pos2(sx + GUIDE_GRAB_PX, canvas_rect.max.y),
                    )
                }
            };

            let resp = ui.interact(
                hit_rect,
                egui::Id::new(("guide_hit", id)),
                egui::Sense::click_and_drag(),
            );
            if resp.hovered() || self.guide_dragging == Some(idx) {
                ui.ctx().set_cursor_icon(match orient {
                    GuideOrientation::Horizontal => egui::CursorIcon::ResizeVertical,
                    GuideOrientation::Vertical => egui::CursorIcon::ResizeHorizontal,
                });
            }
            if resp.double_clicked() {
                self.guide_edit_popup = Some(GuideEditPopup {
                    guide_id: id,
                    orientation: orient,
                    value: photonic_core::from_px(pos, unit, RULER_DPI),
                });
            }
            if resp.drag_started() && self.ruler_drag.is_none() {
                self.guide_dragging = Some(idx);
                self.guide_drag_old = Some(doc.guides.clone());
            }
            if self.guide_dragging == Some(idx) {
                if let Some(p) = ui.input(|i| i.pointer.interact_pos()) {
                    let (cx, cy) = view.screen_to_canvas(p.x as f64, p.y as f64);
                    let new_pos = match orient {
                        GuideOrientation::Horizontal => cy,
                        GuideOrientation::Vertical => cx,
                    };
                    if let Some(g) = doc.guides.get_mut(idx) {
                        g.position = new_pos;
                    }
                    self.draw_drag_label(&painter, p, new_pos, unit);
                }
                if resp.drag_stopped() {
                    let release = ui.input(|i| i.pointer.interact_pos());
                    let on_ruler = release
                        .map(|p| match orient {
                            GuideOrientation::Horizontal => p.y <= body_top,
                            GuideOrientation::Vertical => p.x <= body_left,
                        })
                        .unwrap_or(false);
                    if let Some(old) = self.guide_drag_old.take() {
                        let mut new_guides = doc.guides.clone();
                        if on_ruler && idx < new_guides.len() {
                            // Dragged back onto the ruler → delete.
                            new_guides.remove(idx);
                        }
                        history.execute(Command::SetGuides { old, new: new_guides }, doc);
                    }
                    self.guide_dragging = None;
                }
            }
        }

        // ── Unit selector in the corner box (click to cycle px→mm→in→pt) ─────
        let corner_resp = ui
            .interact(corner, egui::Id::new("ruler_corner_unit"), egui::Sense::click())
            .on_hover_text("Click to change ruler units");
        let corner_fg = if self.prefs.dark_mode {
            egui::Color32::from_gray(190)
        } else {
            egui::Color32::from_gray(70)
        };
        painter.text(
            corner.center(),
            egui::Align2::CENTER_CENTER,
            unit.label(),
            egui::FontId::proportional(9.0),
            corner_fg,
        );
        if corner_resp.clicked() {
            self.prefs.document_units = unit.next();
            self.prefs.save();
        }

        // ── Exact-position editor popup ──────────────────────────────────────
        self.draw_guide_edit_popup(ui, doc, history, unit);
    }

    /// Floating label showing the live guide position (in the current unit)
    /// next to the pointer during a ruler-create or guide-move drag.
    fn draw_drag_label(
        &self,
        painter: &egui::Painter,
        pointer: egui::Pos2,
        pos_px: f64,
        unit: DocumentUnit,
    ) {
        let text = format!("{} {}", self.format_ruler_value(pos_px), unit.label());
        let font = egui::FontId::proportional(11.0);
        let galley = painter.layout_no_wrap(text, font, egui::Color32::WHITE);
        let anchor = pointer + egui::vec2(12.0, 12.0);
        let bg = egui::Rect::from_min_size(anchor, galley.size())
            .expand2(egui::vec2(4.0, 2.0));
        painter.rect_filled(
            bg,
            3.0,
            egui::Color32::from_rgba_unmultiplied(0, 0, 0, 200),
        );
        painter.galley(anchor, galley, egui::Color32::WHITE);
    }

    /// Render the double-click "set exact position" popup, if open, and commit
    /// the new position as a single undoable `Command::SetGuides`.
    fn draw_guide_edit_popup(
        &mut self,
        ui: &egui::Ui,
        doc: &mut Document,
        history: &mut CommandHistory,
        unit: DocumentUnit,
    ) {
        let Some(mut popup) = self.guide_edit_popup.take() else {
            return;
        };
        let mut open = true;
        let mut apply = false;
        let axis = match popup.orientation {
            GuideOrientation::Horizontal => "Y",
            GuideOrientation::Vertical => "X",
        };
        egui::Window::new("Guide position")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ui.ctx(), |ui| {
                ui.horizontal(|ui| {
                    ui.label(format!("{axis}:"));
                    ui.add(
                        egui::DragValue::new(&mut popup.value)
                            .speed(0.5)
                            .suffix(format!(" {}", unit.label())),
                    );
                    if ui.button("Set").clicked() {
                        apply = true;
                    }
                });
            });

        if apply {
            if let Some(idx) = doc.guides.iter().position(|g| g.id == popup.guide_id) {
                let old = doc.guides.clone();
                let mut new_guides = old.clone();
                new_guides[idx].position = photonic_core::to_px(popup.value, unit, RULER_DPI);
                history.execute(Command::SetGuides { old, new: new_guides }, doc);
            }
            // Closing: leave `self.guide_edit_popup` as None.
        } else if open {
            // Still open and not applied → keep the popup state for next frame.
            self.guide_edit_popup = Some(popup);
        }
    }
}
