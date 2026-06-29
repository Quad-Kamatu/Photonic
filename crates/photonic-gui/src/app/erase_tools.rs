//! Destructive path-edit tools: Knife (freehand slice) and the vector Eraser
//! (drag-to-subtract). Both commit on pointer release as a single undoable
//! `Command::Batch`, cutting real geometry via `photonic_core::ops` boolean and
//! stroke-outline operations. Methods on `PhotonicApp`.
#![allow(clippy::too_many_arguments)]
use super::*;
use photonic_core::ops::boolean::{boolean_op, BooleanOp};
use photonic_core::ops::stroke_outline::outline_stroke;
use photonic_core::style::{LineCap, LineJoin};

impl PhotonicApp {
    /// Vector Eraser: drag a circular head; on release, boolean-subtract the
    /// swept area from every visible, unlocked path node it overlaps.
    pub(crate) fn handle_eraser_tool(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        doc: &mut Document,
        view: &CanvasView,
        doc_modified: &mut bool,
        history: &mut CommandHistory,
    ) {
        let ctx = ui.ctx();
        let rect = response.rect;
        let radius = self.eraser_radius.max(0.5);

        // Hide the system cursor — we draw the eraser-head circle instead.
        ctx.set_cursor_icon(egui::CursorIcon::None);

        // Collect drag points in canvas space, throttled to ~½ radius apart so
        // the swept outline stays smooth without exploding the point count.
        if response.dragged_by(egui::PointerButton::Primary) {
            if let Some(pos) = response.interact_pointer_pos() {
                let (cx, cy) = view.screen_to_canvas(pos.x as f64, pos.y as f64);
                let add = match self.eraser_points.last() {
                    Some(&(lx, ly)) => {
                        let dx = cx - lx;
                        let dy = cy - ly;
                        let thresh = (radius * 0.5).max(1.0);
                        dx * dx + dy * dy >= thresh * thresh
                    }
                    None => true,
                };
                if add {
                    self.eraser_points.push((cx, cy));
                }
            }
        }

        // Swept-area preview while dragging (translucent red ribbon).
        if self.eraser_points.len() >= 2 && response.dragged_by(egui::PointerButton::Primary) {
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Tooltip,
                egui::Id::new("eraser_sweep_preview"),
            ));
            let stroke = egui::Stroke::new(
                (radius * 2.0 * view.zoom) as f32,
                egui::Color32::from_rgba_unmultiplied(255, 80, 80, 40),
            );
            let pts: Vec<egui::Pos2> = self
                .eraser_points
                .iter()
                .map(|&(cx, cy)| {
                    let (sx, sy) = view.canvas_to_screen(cx, cy);
                    egui::pos2(sx as f32, sy as f32)
                })
                .collect();
            for w in pts.windows(2) {
                painter.line_segment([w[0], w[1]], stroke);
            }
        }

        // Eraser-head circle cursor, following the pointer on hover and drag.
        if let Some(cursor) = ui.input(|i| i.pointer.hover_pos()) {
            if rect.contains(cursor) {
                let painter = ctx.layer_painter(egui::LayerId::new(
                    egui::Order::Foreground,
                    egui::Id::new("eraser_cursor"),
                ));
                let r_screen = (radius * view.zoom) as f32;
                painter.circle_stroke(
                    cursor,
                    r_screen,
                    egui::Stroke::new(1.5, egui::Color32::from_rgb(255, 70, 70)),
                );
                painter.circle_stroke(
                    cursor,
                    r_screen,
                    egui::Stroke::new(0.5, egui::Color32::WHITE),
                );
                // Keep the cursor preview smooth even when idle.
                ctx.request_repaint();
            }
        }

        // Commit on release.
        if response.drag_stopped() {
            let pts = std::mem::take(&mut self.eraser_points);
            self.apply_eraser(&pts, radius, doc, history, doc_modified);
        }
    }

    /// Build and commit the eraser boolean-subtract over all eligible nodes.
    fn apply_eraser(
        &mut self,
        points: &[(f64, f64)],
        radius: f64,
        doc: &mut Document,
        history: &mut CommandHistory,
        doc_modified: &mut bool,
    ) {
        if points.is_empty() {
            return;
        }
        let eraser_canvas = match build_stroke_area(points, radius * 2.0, true) {
            Some(p) => p,
            None => return,
        };
        let eraser_bbox = match eraser_canvas.bounding_box() {
            Some(b) => b,
            None => return,
        };

        let node_ids: Vec<NodeId> = doc.nodes.keys().copied().collect();
        let mut cmds: Vec<Command> = Vec::new();

        for nid in node_ids {
            let node = match doc.nodes.get(&nid) {
                Some(n) => n,
                None => continue,
            };
            if !node.visible || node.locked {
                continue;
            }
            let pn = match &node.kind {
                SceneNodeKind::Path(p) => p,
                _ => continue,
            };
            if pn.path_data.is_empty() {
                continue;
            }
            // Cheap reject: skip nodes whose canvas-space bbox misses the sweep.
            if let Some(node_bbox) = path_canvas_bbox(node, &pn.path_data) {
                if !rects_overlap(node_bbox, eraser_bbox) {
                    continue;
                }
            }

            // Subtract in the node's LOCAL space: bring the eraser outline into
            // local coords via the inverse node transform, then boolean-subtract.
            let inv = node.transform.to_kurbo().inverse();
            let eraser_local = transform_path(&eraser_canvas, inv);

            match boolean_op(&pn.path_data, &eraser_local, BooleanOp::Subtract) {
                Ok(result) => {
                    if result.is_empty() {
                        cmds.push(Command::RemoveNode { node_id: nid });
                    } else {
                        let mut new_node = node.clone();
                        if let SceneNodeKind::Path(ref mut p) = new_node.kind {
                            p.path_data = result;
                        }
                        cmds.push(Command::UpdateNode {
                            old: node.clone(),
                            new: new_node,
                        });
                    }
                }
                Err(e) => {
                    eprintln!("Eraser: boolean subtract failed on node {nid:?}: {e}");
                }
            }
        }

        if !cmds.is_empty() {
            let cmd = if cmds.len() == 1 {
                cmds.pop().unwrap()
            } else {
                Command::Batch(cmds)
            };
            history.execute(cmd, doc);
            *doc_modified = true;
        }
    }

    /// Knife: drag a freehand line; on release, slice each filled path it fully
    /// crosses into separate editable face nodes.
    pub(crate) fn handle_knife_tool(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        doc: &mut Document,
        view: &CanvasView,
        doc_modified: &mut bool,
        history: &mut CommandHistory,
    ) {
        let ctx = ui.ctx();
        ctx.set_cursor_icon(egui::CursorIcon::Crosshair);

        if response.dragged_by(egui::PointerButton::Primary) {
            if let Some(pos) = response.interact_pointer_pos() {
                let (cx, cy) = view.screen_to_canvas(pos.x as f64, pos.y as f64);
                let add = match self.knife_points.last() {
                    Some(&(lx, ly)) => {
                        let dx = cx - lx;
                        let dy = cy - ly;
                        let thresh = (3.0 / view.zoom).max(0.5);
                        dx * dx + dy * dy >= thresh * thresh
                    }
                    None => true,
                };
                if add {
                    self.knife_points.push((cx, cy));
                }
            }
        }

        // Cut-line preview.
        if !self.knife_points.is_empty() {
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Tooltip,
                egui::Id::new("knife_preview"),
            ));
            let stroke = egui::Stroke::new(1.5, egui::Color32::from_rgb(255, 60, 60));
            let mut pts: Vec<egui::Pos2> = self
                .knife_points
                .iter()
                .map(|&(cx, cy)| {
                    let (sx, sy) = view.canvas_to_screen(cx, cy);
                    egui::pos2(sx as f32, sy as f32)
                })
                .collect();
            if response.dragged_by(egui::PointerButton::Primary) {
                if let Some(cur) = ui.input(|i| i.pointer.hover_pos()) {
                    pts.push(cur);
                }
            }
            for w in pts.windows(2) {
                painter.line_segment([w[0], w[1]], stroke);
            }
        }

        if response.drag_stopped() {
            let pts = std::mem::take(&mut self.knife_points);
            // Keep the cut thin on screen regardless of zoom (~2px wide).
            let width = (2.0 / view.zoom).max(0.5);
            self.apply_knife(&pts, width, doc, history, doc_modified);
        }
    }

    /// Build and commit the knife slice over all eligible filled path nodes.
    fn apply_knife(
        &mut self,
        points: &[(f64, f64)],
        width: f64,
        doc: &mut Document,
        history: &mut CommandHistory,
        doc_modified: &mut bool,
    ) {
        if points.len() < 2 {
            return;
        }
        // Butt caps: the cutter must not bulge past the drawn endpoints, so the
        // line genuinely crosses the shape rather than nicking its interior.
        let cutter_canvas = match build_stroke_area(points, width, false) {
            Some(p) => p,
            None => return,
        };
        let cutter_bbox = match cutter_canvas.bounding_box() {
            Some(b) => b,
            None => return,
        };

        let node_ids: Vec<NodeId> = doc.nodes.keys().copied().collect();
        let mut cmds: Vec<Command> = Vec::new();
        let mut new_selection: Vec<NodeId> = Vec::new();

        for nid in node_ids {
            let node = match doc.nodes.get(&nid) {
                Some(n) => n,
                None => continue,
            };
            if !node.visible || node.locked {
                continue;
            }
            let pn = match &node.kind {
                SceneNodeKind::Path(p) => p,
                _ => continue,
            };
            // Knife M1 operates on filled paths only.
            if !pn.fill.enabled || pn.path_data.is_empty() {
                continue;
            }
            if let Some(node_bbox) = path_canvas_bbox(node, &pn.path_data) {
                if !rects_overlap(node_bbox, cutter_bbox) {
                    continue;
                }
            }

            let inv = node.transform.to_kurbo().inverse();
            let cutter_local = transform_path(&cutter_canvas, inv);

            // Subtracting a thin, fully-crossing sliver splits the filled area
            // into ≥2 disjoint polygons (returned as subpaths of one PathData).
            let sliced = match boolean_op(&pn.path_data, &cutter_local, BooleanOp::Subtract) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("Knife: boolean cut failed on node {nid:?}: {e}");
                    continue;
                }
            };
            let faces = split_subpaths(&sliced);
            // Only act when the cut actually separated the shape into pieces.
            if faces.len() < 2 {
                continue;
            }

            cmds.push(Command::RemoveNode { node_id: nid });
            let total = faces.len();
            for (i, face) in faces.into_iter().enumerate() {
                let mut new_node = node.clone();
                new_node.id = NodeId::new_v4();
                new_node.name = format!("{} ({}/{})", node.name, i + 1, total);
                if let SceneNodeKind::Path(ref mut p) = new_node.kind {
                    p.path_data = face;
                    p.is_compound = false;
                }
                new_selection.push(new_node.id);
                cmds.push(Command::AddNode {
                    node: new_node,
                    layer_id: Some(node.layer_id),
                });
            }
        }

        if !cmds.is_empty() {
            history.execute(Command::Batch(cmds), doc);
            doc.selection = photonic_core::Selection::from_ids(new_selection.iter().copied());
            *doc_modified = true;
        }
    }
}

/// Construct a filled area from a polyline by outlining it at `width` using
/// kurbo's stroke expansion. `round` selects round caps/joins (eraser head);
/// otherwise butt caps with round joins (knife blade). A single point is
/// expanded to a hair-segment so round caps still form a full disc.
///
/// Returns `None` if the polyline is empty or `width <= 0`.
pub(crate) fn build_stroke_area(
    points: &[(f64, f64)],
    width: f64,
    round: bool,
) -> Option<PathData> {
    if points.is_empty() || width <= 0.0 {
        return None;
    }
    let mut bez = BezPath::new();
    if points.len() == 1 {
        let (x, y) = points[0];
        bez.move_to((x - 0.01, y));
        bez.line_to((x + 0.01, y));
    } else {
        bez.move_to(points[0]);
        for &(x, y) in &points[1..] {
            bez.line_to((x, y));
        }
    }
    let polyline = PathData::from_bez_path(&bez);

    let mut stroke = Stroke::solid(Color::BLACK, width);
    stroke.line_join = LineJoin::Round;
    stroke.line_cap = if round { LineCap::Round } else { LineCap::Butt };

    outline_stroke(&polyline, &stroke).ok()
}

/// Apply an affine transform to every point of a path (used to move the cutter
/// from canvas space into a node's local space).
fn transform_path(path: &PathData, affine: kurbo::Affine) -> PathData {
    let bez = path.to_bez_path();
    PathData::from_bez_path(&(affine * bez))
}

/// Canvas-space axis-aligned bbox of a node's path (local bbox through transform).
fn path_canvas_bbox(node: &SceneNode, path: &PathData) -> Option<kurbo::Rect> {
    let local = path.bounding_box()?;
    let a = node.transform.to_kurbo();
    let corners = [
        a * Point::new(local.x0, local.y0),
        a * Point::new(local.x1, local.y0),
        a * Point::new(local.x0, local.y1),
        a * Point::new(local.x1, local.y1),
    ];
    let min_x = corners.iter().map(|p| p.x).fold(f64::INFINITY, f64::min);
    let min_y = corners.iter().map(|p| p.y).fold(f64::INFINITY, f64::min);
    let max_x = corners.iter().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max);
    let max_y = corners.iter().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max);
    Some(kurbo::Rect::new(min_x, min_y, max_x, max_y))
}

/// AABB overlap test (inclusive).
fn rects_overlap(a: kurbo::Rect, b: kurbo::Rect) -> bool {
    a.x0 <= b.x1 && b.x0 <= a.x1 && a.y0 <= b.y1 && b.y0 <= a.y1
}

/// Split a `PathData` into one `PathData` per subpath (each `MoveTo` starts a
/// new subpath). Used to turn a multi-polygon knife result into separate nodes.
fn split_subpaths(path: &PathData) -> Vec<PathData> {
    let bez = path.to_bez_path();
    let mut out: Vec<PathData> = Vec::new();
    let mut cur = BezPath::new();
    for el in bez.elements() {
        match el {
            PathEl::MoveTo(_) => {
                if !cur.elements().is_empty() {
                    out.push(PathData::from_bez_path(&cur));
                }
                cur = BezPath::new();
                cur.push(*el);
            }
            other => cur.push(*other),
        }
    }
    if !cur.elements().is_empty() {
        out.push(PathData::from_bez_path(&cur));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eraser_sweep_outline_spans_radius() {
        // A horizontal drag from (0,0) to (100,0) with a 10px radius head should
        // outline a capsule ~120 wide (×2 round caps) and ~20 tall.
        let pts = [(0.0, 0.0), (100.0, 0.0)];
        let area = build_stroke_area(&pts, 20.0, true).expect("outline built");
        assert!(!area.is_empty(), "swept outline must not be empty");
        let bbox = area.bounding_box().expect("outline has a bbox");
        assert!(
            (bbox.width() - 120.0).abs() < 2.0,
            "width ~120 (got {})",
            bbox.width()
        );
        assert!(
            (bbox.height() - 20.0).abs() < 2.0,
            "height ~20 (got {})",
            bbox.height()
        );
    }

    #[test]
    fn single_point_eraser_makes_a_disc() {
        // A click (single point) should still produce a non-empty round area.
        let pts = [(5.0, 5.0)];
        let area = build_stroke_area(&pts, 8.0, true).expect("outline built");
        assert!(!area.is_empty());
        let bbox = area.bounding_box().unwrap();
        // Diameter ≈ 8 (radius 4 each side); allow caps tolerance.
        assert!(bbox.width() >= 6.0 && bbox.width() <= 10.0, "got {}", bbox.width());
    }

    #[test]
    fn knife_subtract_splits_a_square_into_two_faces() {
        // A unit-ish square, cut by a thin vertical sliver straight down the
        // middle, must subtract into two disjoint subpaths.
        let square = PathData::from_svg("M 0 0 L 100 0 L 100 100 L 0 100 Z").unwrap();
        // Vertical cutter crossing top-to-bottom through x=50.
        let cutter = build_stroke_area(&[(50.0, -10.0), (50.0, 110.0)], 2.0, false).unwrap();
        let sliced = boolean_op(&square, &cutter, BooleanOp::Subtract).expect("cut ok");
        let faces = split_subpaths(&sliced);
        assert_eq!(faces.len(), 2, "expected two faces, got {}", faces.len());
        for f in &faces {
            assert!(!f.is_empty());
        }
    }
}
