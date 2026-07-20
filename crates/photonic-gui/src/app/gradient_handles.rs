//! On-canvas gradient control handles (the "gradient annotator").
//!
//! Shown only while the movable fill color popup is open (editing a path node's
//! fill). Draggable handles appear on the object: linear endpoints + midpoint,
//! radial center + radius ring, and one node per fluid point / mesh vertex.
//! Handles map through the same space the renderer uses — the object's world
//! AABB for axis-aligned object gradients, or the object's *local* frame
//! (rotating/shearing with it) for rotation-following gradients — so they sit
//! exactly where the gradient renders.

use super::*;
use photonic_core::style::{FillKind, GradientKind, GradientUnits};
use photonic_core::transform::Transform;

/// Which control point is being dragged.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum GradHandle {
    LinStart,
    LinEnd,
    LinMid,
    RadCenter,
    RadRadius,
    Fluid(usize),
    /// Interior vertical grid line `i` (drag horizontally).
    MeshXLine(usize),
    /// Interior horizontal grid line `j` (drag vertically).
    MeshYLine(usize),
}

/// Coordinate mapper between gradient-coordinate space and document space for a
/// specific object + gradient units.
struct Frame<'a> {
    units: GradientUnits,
    /// World/document AABB: (min_x, min_y, w, h).
    wbb: (f64, f64, f64, f64),
    /// Local (pre-transform) bounds.
    lb: kurbo::Rect,
    t: &'a Transform,
}

impl Frame<'_> {
    /// Gradient coordinate → document space.
    fn c2d(&self, u: f64, v: f64) -> (f64, f64) {
        match self.units {
            GradientUnits::UserSpaceOnUse => (u, v),
            GradientUnits::ObjectBoundingBox => {
                (self.wbb.0 + u * self.wbb.2, self.wbb.1 + v * self.wbb.3)
            }
            GradientUnits::ObjectBoundingBoxRotated => {
                let lx = self.lb.x0 + u * self.lb.width();
                let ly = self.lb.y0 + v * self.lb.height();
                self.t.apply(lx, ly)
            }
        }
    }

    /// Document space → gradient coordinate.
    fn d2c(&self, x: f64, y: f64) -> (f64, f64) {
        match self.units {
            GradientUnits::UserSpaceOnUse => (x, y),
            GradientUnits::ObjectBoundingBox => {
                ((x - self.wbb.0) / self.wbb.2, (y - self.wbb.1) / self.wbb.3)
            }
            GradientUnits::ObjectBoundingBoxRotated => {
                let (lx, ly) = inverse(self.t, x, y);
                (
                    (lx - self.lb.x0) / self.lb.width(),
                    (ly - self.lb.y0) / self.lb.height(),
                )
            }
        }
    }

    /// The reference length that a radial `r` coordinate scales by.
    fn radius_ref(&self) -> f64 {
        match self.units {
            GradientUnits::UserSpaceOnUse => 1.0,
            GradientUnits::ObjectBoundingBox => self.wbb.2.max(self.wbb.3),
            GradientUnits::ObjectBoundingBoxRotated => self.lb.width().max(self.lb.height()),
        }
    }

    /// Document position of a radial radius handle (center + r along local x).
    fn radius_handle_doc(&self, cx: f64, cy: f64, r: f64) -> (f64, f64) {
        let rr = r * self.radius_ref();
        match self.units {
            GradientUnits::ObjectBoundingBoxRotated => {
                let lx = self.lb.x0 + cx * self.lb.width() + rr;
                let ly = self.lb.y0 + cy * self.lb.height();
                self.t.apply(lx, ly)
            }
            _ => {
                let (dcx, dcy) = self.c2d(cx, cy);
                (dcx + rr, dcy)
            }
        }
    }

    /// Recover a radial `r` coordinate from a dragged document point.
    fn radius_from_doc(&self, cx: f64, cy: f64, x: f64, y: f64) -> f64 {
        let dist = match self.units {
            GradientUnits::ObjectBoundingBoxRotated => {
                let (lx, ly) = inverse(self.t, x, y);
                let clx = self.lb.x0 + cx * self.lb.width();
                let cly = self.lb.y0 + cy * self.lb.height();
                (lx - clx).hypot(ly - cly)
            }
            _ => {
                let (dcx, dcy) = self.c2d(cx, cy);
                (x - dcx).hypot(y - dcy)
            }
        };
        (dist / self.radius_ref()).max(1e-4)
    }
}

impl PhotonicApp {
    /// Draw + interact with on-canvas gradient handles for the node whose fill
    /// popup is open. Returns `true` when it consumed the pointer drag.
    pub(crate) fn handle_gradient_on_canvas(
        &mut self,
        ui: &mut egui::Ui,
        response: &egui::Response,
        doc: &mut Document,
        view: &CanvasView,
        doc_modified: &mut bool,
        history: &mut CommandHistory,
    ) -> bool {
        // Only while the movable fill popup is open (not stroke).
        let Some(popup) = self.color_popup else {
            self.gradient_drag = None;
            return false;
        };
        if popup.stroke {
            self.gradient_drag = None;
            return false;
        }
        let nid = popup.node_id;
        let Some(node) = doc.nodes.get(&nid) else {
            self.gradient_drag = None;
            return false;
        };
        let SceneNodeKind::Path(pn) = &node.kind else {
            self.gradient_drag = None;
            return false;
        };
        if !matches!(
            pn.fill.kind,
            FillKind::Gradient(_) | FillKind::FluidGradient(_) | FillKind::MeshGradient(_)
        ) {
            self.gradient_drag = None;
            return false;
        }
        let Some(lb) = node.local_bounds() else {
            return false;
        };
        let frame = Frame {
            units: gradient_units(&pn.fill.kind),
            wbb: object_doc_aabb(lb, &node.transform),
            lb,
            t: &node.transform,
        };

        let handles = collect_handles(&pn.fill.kind, &frame);
        let accent = Color32::from_rgb(130, 105, 225);
        let painter = ui.painter();
        let to_screen = |d: (f64, f64)| {
            let s = view.canvas_to_screen(d.0, d.1);
            egui::pos2(s.0 as f32, s.1 as f32)
        };

        // ── Connecting geometry (linear line / radial ring) ──
        match &pn.fill.kind {
            FillKind::Gradient(g) if g.kind == GradientKind::Linear && g.coords.len() >= 4 => {
                let a = to_screen(frame.c2d(g.coords[0], g.coords[1]));
                let b = to_screen(frame.c2d(g.coords[2], g.coords[3]));
                painter.line_segment(
                    [a, b],
                    egui::Stroke::new(3.0, Color32::from_black_alpha(120)),
                );
                painter.line_segment([a, b], egui::Stroke::new(1.5, Color32::WHITE));
            }
            FillKind::Gradient(g) if g.kind == GradientKind::Radial && g.coords.len() >= 5 => {
                let center = to_screen(frame.c2d(g.coords[0], g.coords[1]));
                let rim = to_screen(frame.radius_handle_doc(g.coords[0], g.coords[1], g.coords[4]));
                let r_screen = center.distance(rim);
                painter.circle_stroke(center, r_screen, egui::Stroke::new(1.5, Color32::WHITE));
                painter.circle_stroke(
                    center,
                    r_screen + 1.0,
                    egui::Stroke::new(1.0, Color32::from_black_alpha(120)),
                );
            }
            FillKind::MeshGradient(mg) if mg.x_lines.len() >= 2 && mg.y_lines.len() >= 2 => {
                let x0 = mg.x_lines[0];
                let x1 = *mg.x_lines.last().unwrap();
                let y0 = mg.y_lines[0];
                let y1 = *mg.y_lines.last().unwrap();
                let line = |a, b, painter: &egui::Painter| {
                    painter.line_segment(
                        [a, b],
                        egui::Stroke::new(3.0, Color32::from_black_alpha(110)),
                    );
                    painter.line_segment([a, b], egui::Stroke::new(1.25, Color32::WHITE));
                };
                for &x in &mg.x_lines {
                    line(
                        to_screen(frame.c2d(x, y0)),
                        to_screen(frame.c2d(x, y1)),
                        painter,
                    );
                }
                for &y in &mg.y_lines {
                    line(
                        to_screen(frame.c2d(x0, y)),
                        to_screen(frame.c2d(x1, y)),
                        painter,
                    );
                }
            }
            _ => {}
        }

        // Generous grab radius; find the closest handle to a screen point.
        const HIT: f32 = 16.0;
        let closest = |pt: egui::Pos2| -> Option<GradHandle> {
            handles
                .iter()
                .map(|(h, dx, dy, _)| (*h, pt.distance(to_screen((*dx, *dy)))))
                .filter(|(_, d)| *d <= HIT)
                .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
                .map(|(h, _)| h)
        };

        // The handle to emphasise: the one being dragged, else the hovered one.
        let emphasised = self
            .gradient_drag
            .or_else(|| response.hover_pos().and_then(closest));

        // ── Handle dots ──
        for (h, dx, dy, col) in &handles {
            let p = to_screen((*dx, *dy));
            let active = emphasised == Some(*h);
            let r = if active { 7.5 } else { 5.5 };
            painter.circle_filled(p, r + 2.0, Color32::from_black_alpha(130));
            painter.circle_filled(p, r, col.unwrap_or(Color32::WHITE));
            painter.circle_stroke(
                p,
                r,
                egui::Stroke::new(2.0, if active { accent } else { Color32::WHITE }),
            );
        }

        // Cursor feedback so handles read as grabbable.
        if self.gradient_drag.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        } else if emphasised.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }

        // ── Interaction ──
        // Grab from the exact press origin, not the delayed `drag_started`
        // position (egui only reports a drag after the pointer has drifted past
        // its drag threshold — that drift used to miss the small handle and let
        // the Select tool move the object instead).
        if response.drag_started() {
            if let Some(press) = ui.input(|i| i.pointer.press_origin()) {
                self.gradient_drag = closest(press);
            }
        }
        let dragging = self.gradient_drag.is_some();
        if dragging && response.dragged() {
            if let Some(pp) = response.interact_pointer_pos() {
                let (cx, cy) = view.screen_to_canvas(pp.x as f64, pp.y as f64);
                let mut new_node = node.clone();
                if let SceneNodeKind::Path(pnm) = &mut new_node.kind {
                    apply_handle_drag(
                        &mut pnm.fill.kind,
                        self.gradient_drag.unwrap(),
                        cx,
                        cy,
                        &frame,
                    );
                }
                history.execute(
                    Command::UpdateNode {
                        old: node.clone(),
                        new: new_node,
                    },
                    doc,
                );
                *doc_modified = true;
            }
        }
        if response.drag_stopped() {
            self.gradient_drag = None;
        }
        dragging
    }
}

fn gradient_units(kind: &FillKind) -> GradientUnits {
    match kind {
        FillKind::Gradient(g) => g.units,
        FillKind::FluidGradient(fg) => fg.units,
        FillKind::MeshGradient(mg) => mg.units,
        _ => GradientUnits::UserSpaceOnUse,
    }
}

/// The object's document-space AABB from its local bounds + transform.
fn object_doc_aabb(lb: kurbo::Rect, t: &Transform) -> (f64, f64, f64, f64) {
    let corners = [
        (lb.x0, lb.y0),
        (lb.x1, lb.y0),
        (lb.x1, lb.y1),
        (lb.x0, lb.y1),
    ];
    let (mut minx, mut miny, mut maxx, mut maxy) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for (x, y) in corners {
        let (cx, cy) = t.apply(x, y);
        minx = minx.min(cx);
        miny = miny.min(cy);
        maxx = maxx.max(cx);
        maxy = maxy.max(cy);
    }
    (minx, miny, (maxx - minx).max(1e-6), (maxy - miny).max(1e-6))
}

/// Inverse affine transform (document → local).
fn inverse(t: &Transform, x: f64, y: f64) -> (f64, f64) {
    let m = t.matrix;
    let det = m[0] * m[3] - m[1] * m[2];
    if det.abs() < 1e-12 {
        return (x, y);
    }
    let dx = x - m[4];
    let dy = y - m[5];
    (
        (m[3] * dx - m[2] * dy) / det,
        (-m[1] * dx + m[0] * dy) / det,
    )
}

/// Collect draggable handles as (handle, doc_x, doc_y, optional color).
fn collect_handles(kind: &FillKind, f: &Frame) -> Vec<(GradHandle, f64, f64, Option<Color32>)> {
    let mut out = Vec::new();
    match kind {
        FillKind::Gradient(g) => match g.kind {
            GradientKind::Linear if g.coords.len() >= 4 => {
                let (sx, sy) = f.c2d(g.coords[0], g.coords[1]);
                let (ex, ey) = f.c2d(g.coords[2], g.coords[3]);
                out.push((GradHandle::LinStart, sx, sy, None));
                out.push((GradHandle::LinEnd, ex, ey, None));
                out.push((GradHandle::LinMid, (sx + ex) * 0.5, (sy + ey) * 0.5, None));
            }
            GradientKind::Radial if g.coords.len() >= 5 => {
                let (cx, cy) = f.c2d(g.coords[0], g.coords[1]);
                let (rx, ry) = f.radius_handle_doc(g.coords[0], g.coords[1], g.coords[4]);
                out.push((GradHandle::RadCenter, cx, cy, None));
                out.push((GradHandle::RadRadius, rx, ry, None));
            }
            _ => {}
        },
        FillKind::FluidGradient(fg) => {
            for (i, p) in fg.points.iter().enumerate() {
                let (x, y) = f.c2d(p.x, p.y);
                out.push((GradHandle::Fluid(i), x, y, Some(color32(p.color))));
            }
        }
        FillKind::MeshGradient(mg) if mg.x_lines.len() >= 2 && mg.y_lines.len() >= 2 => {
            // A grab handle on each interior grid line, at the line's midpoint.
            let midy = (mg.y_lines[0] + mg.y_lines.last().unwrap()) * 0.5;
            let midx = (mg.x_lines[0] + mg.x_lines.last().unwrap()) * 0.5;
            for i in 1..mg.x_lines.len() - 1 {
                let (x, y) = f.c2d(mg.x_lines[i], midy);
                out.push((GradHandle::MeshXLine(i), x, y, None));
            }
            for j in 1..mg.y_lines.len() - 1 {
                let (x, y) = f.c2d(midx, mg.y_lines[j]);
                out.push((GradHandle::MeshYLine(j), x, y, None));
            }
        }
        _ => {}
    }
    out
}

/// Apply a drag (document coords) to the gradient's control point.
fn apply_handle_drag(kind: &mut FillKind, handle: GradHandle, x: f64, y: f64, f: &Frame) {
    match kind {
        FillKind::Gradient(g) => {
            let (u, v) = f.d2c(x, y);
            match handle {
                GradHandle::LinStart if g.coords.len() >= 4 => {
                    g.coords[0] = u;
                    g.coords[1] = v;
                }
                GradHandle::LinEnd if g.coords.len() >= 4 => {
                    g.coords[2] = u;
                    g.coords[3] = v;
                }
                GradHandle::LinMid if g.coords.len() >= 4 => {
                    let dx = u - (g.coords[0] + g.coords[2]) * 0.5;
                    let dy = v - (g.coords[1] + g.coords[3]) * 0.5;
                    g.coords[0] += dx;
                    g.coords[1] += dy;
                    g.coords[2] += dx;
                    g.coords[3] += dy;
                }
                GradHandle::RadCenter if g.coords.len() >= 5 => {
                    g.coords[0] = u;
                    g.coords[1] = v;
                    g.coords[2] = u;
                    g.coords[3] = v;
                }
                GradHandle::RadRadius if g.coords.len() >= 5 => {
                    g.coords[4] = f.radius_from_doc(g.coords[0], g.coords[1], x, y);
                }
                _ => {}
            }
        }
        FillKind::FluidGradient(fg) => {
            if let GradHandle::Fluid(i) = handle {
                if let Some(p) = fg.points.get_mut(i) {
                    let (u, v) = f.d2c(x, y);
                    p.x = u;
                    p.y = v;
                }
            }
        }
        FillKind::MeshGradient(mg) => {
            let (u, v) = f.d2c(x, y);
            match handle {
                // Keep an interior line strictly between its neighbours.
                GradHandle::MeshXLine(i) if i >= 1 && i + 1 < mg.x_lines.len() => {
                    let lo = mg.x_lines[i - 1] + 1e-3;
                    let hi = mg.x_lines[i + 1] - 1e-3;
                    mg.x_lines[i] = u.clamp(lo, hi);
                }
                GradHandle::MeshYLine(j) if j >= 1 && j + 1 < mg.y_lines.len() => {
                    let lo = mg.y_lines[j - 1] + 1e-3;
                    let hi = mg.y_lines[j + 1] - 1e-3;
                    mg.y_lines[j] = v.clamp(lo, hi);
                }
                _ => {}
            }
        }
        _ => {}
    }
}

fn color32(c: photonic_core::Color) -> Color32 {
    Color32::from_rgb(
        (c.r * 255.0) as u8,
        (c.g * 255.0) as u8,
        (c.b * 255.0) as u8,
    )
}
