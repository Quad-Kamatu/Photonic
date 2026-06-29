//! Direct Selection tool — anchor / bezier-handle / Live-Corners editing.
//! Methods extracted from app::mod; they drive PhotonicApp's point-edit state.
#![allow(clippy::too_many_arguments)]
use super::*;

impl PhotonicApp {
    /// Drop point-edit selection state that an external document change (undo,
    /// redo, deletion) may have invalidated: the anchor indices in
    /// `point_selected` are positional, so a restructured path makes them stale.
    /// Also clears `point_edit_node` if its node no longer exists.
    pub(crate) fn invalidate_point_edit(&mut self, doc: &Document) {
        self.point_selected.clear();
        self.point_drag_origin = None;
        self.point_drag_mode = None;
        if let Some(nid) = self.point_edit_node {
            if !doc.nodes.contains_key(&nid) {
                self.point_edit_node = None;
            }
        }
    }

    pub(crate) fn handle_direct_select_tool(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        doc: &mut Document,
        view: &CanvasView,
        renderer: &mut PhotonicRenderer,
        doc_modified: &mut bool,
        history: &mut CommandHistory,
    ) {
        // Generous hit radii — make anchors, handles and corner widgets easy to grab.
        const ANCHOR_RADIUS_PX: f64 = 12.0;
        const HANDLE_RADIUS_PX: f64 = 10.0;
        const CORNER_WIDGET_PX: f64 = 9.0;
        const CORNER_INSET_PX: f64 = 22.0;
        let accent = Color32::from_rgb(110, 86, 207);

        // Escape: exit point-edit mode
        if viewport_kb(ui.ctx()) && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.point_edit_node = None;
            self.point_selected.clear();
            self.point_drag_origin = None;
            self.point_drag_mode = None;
            return;
        }

        // The edit node may have been removed (delete, undo) since last frame.
        if let Some(nid) = self.point_edit_node {
            if !doc.nodes.contains_key(&nid) {
                self.invalidate_point_edit(doc);
            }
        }

        let (shift, ctrl, alt) =
            ui.input(|i| (i.modifiers.shift, i.modifiers.ctrl, i.modifiers.alt));
        // Shift is Illustrator's add-to-vertex-selection modifier; Ctrl kept for parity.
        let add_sel = shift || ctrl;

        // hover_pos is used ONLY for the visual highlight — NOT for hit-testing on
        // click/drag events.  All interaction positions come from interact_pointer_pos()
        // so that the test point is at the press location, not the current cursor position
        // (by the time drag_started fires the cursor may have moved off the anchor).
        let hover_canvas = ui
            .input(|i| i.pointer.hover_pos())
            .map(|p| view.screen_to_canvas(p.x as f64, p.y as f64));

        // Helper closure: hit-test anchors of the current edit node at canvas pos (cx, cy)
        let find_anchor = |nid: NodeId, cx: f64, cy: f64, doc: &Document| -> Option<usize> {
            doc.nodes.get(&nid).and_then(|node| {
                if let SceneNodeKind::Path(pn) = &node.kind {
                    let bez = pn.path_data.to_bez_path();
                    nearest_anchor_screen(&bez, &node.transform, view, cx, cy, ANCHOR_RADIUS_PX)
                } else {
                    None
                }
            })
        };

        // Roundable straight corners of the edit node, recomputed each frame for
        // Live-Corners widget hit-testing and rendering.
        let corner_map: std::collections::HashMap<usize, (Point, Point, Point)> = self
            .point_edit_node
            .and_then(|nid| doc.nodes.get(&nid))
            .and_then(|node| match &node.kind {
                SceneNodeKind::Path(pn) => Some(straight_corners(&pn.path_data.to_bez_path())),
                _ => None,
            })
            .unwrap_or_default();

        // ── Hover cursor feedback ─────────────────────────────────────────────
        // A grab cursor over anchors / handles / corner widgets is the primary
        // discoverability cue for what a press will grab.
        let hover_cursor = match (self.point_edit_node.and_then(|nid| doc.nodes.get(&nid)), hover_canvas)
        {
            (Some(node), Some((hx, hy))) => {
                if ds_find_handle(node, view, &self.point_selected, hx, hy, HANDLE_RADIUS_PX).is_some()
                    || ds_find_corner_widget(
                        node,
                        view,
                        &self.point_selected,
                        &corner_map,
                        hx,
                        hy,
                        CORNER_INSET_PX,
                        CORNER_WIDGET_PX,
                    )
                    .is_some()
                {
                    egui::CursorIcon::Grab
                } else if self
                    .point_edit_node
                    .and_then(|nid| find_anchor(nid, hx, hy, doc))
                    .is_some()
                {
                    egui::CursorIcon::PointingHand
                } else {
                    egui::CursorIcon::Default
                }
            }
            _ => egui::CursorIcon::Default,
        };
        ui.ctx().set_cursor_icon(hover_cursor);

        // ── Delete selected anchor points ─────────────────────────────────────
        let delete =
            ui.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace));
        if delete && !self.point_selected.is_empty() && viewport_kb(ui.ctx()) {
            if let Some(nid) = self.point_edit_node {
                if let Some(node) = doc.nodes.get(&nid) {
                    let old_node = node.clone();
                    if let SceneNodeKind::Path(pn) = &node.kind {
                        let bez = pn.path_data.to_bez_path();
                        let new_bez = bez_remove_elements(&bez, &self.point_selected);
                        let mut new_node = old_node.clone();
                        if let SceneNodeKind::Path(new_pn) = &mut new_node.kind {
                            new_pn.path_data = PathData::from_bez_path(&new_bez);
                        }
                        history.execute(
                            Command::UpdateNode {
                                old: old_node,
                                new: new_node,
                            },
                            doc,
                        );
                        self.point_selected.clear();
                        *doc_modified = true;
                    }
                }
            }
            return;
        }

        // ── Drag start: use interact_pointer_pos() — the press location ───────
        // Priority: bezier handle > corner widget > anchor point > shape body.
        if response.drag_started_by(egui::PointerButton::Primary) {
            if let Some(press_pos) = response.interact_pointer_pos() {
                let (cx, cy) = view.screen_to_canvas(press_pos.x as f64, press_pos.y as f64);
                let edit_node = self.point_edit_node.and_then(|nid| doc.nodes.get(&nid));

                let handle_hit = edit_node.and_then(|node| {
                    ds_find_handle(node, view, &self.point_selected, cx, cy, HANDLE_RADIUS_PX)
                });
                let corner_hit = edit_node.and_then(|node| {
                    ds_find_corner_widget(
                        node,
                        view,
                        &self.point_selected,
                        &corner_map,
                        cx,
                        cy,
                        CORNER_INSET_PX,
                        CORNER_WIDGET_PX,
                    )
                });
                let anchor_hit = self.point_edit_node.and_then(|nid| find_anchor(nid, cx, cy, doc));
                let origin_bez = edit_node.and_then(|node| match &node.kind {
                    SceneNodeKind::Path(pn) => Some(pn.path_data.to_bez_path()),
                    _ => None,
                });

                if let Some((anchor, kind)) = handle_hit {
                    self.point_drag_mode = Some(DirectDrag::Handle { anchor, kind });
                    self.point_drag_origin =
                        self.point_edit_node.and_then(|nid| doc.nodes.get(&nid).cloned());
                } else if let Some(pivot) = corner_hit {
                    let ob = origin_bez.unwrap_or_default();
                    // Distance from the pivot corner to the press point, so the
                    // radius starts at 0 rather than snapping to the widget inset.
                    let grab_dist = match (edit_node, straight_corners(&ob).get(&pivot)) {
                        (Some(node), Some((_, corner, _))) => {
                            let (lx, ly) = canvas_to_local(&node.transform, cx, cy);
                            ((lx - corner.x).powi(2) + (ly - corner.y).powi(2)).sqrt()
                        }
                        _ => 0.0,
                    };
                    self.point_drag_mode = Some(DirectDrag::Corner {
                        pivot,
                        origin_bez: ob,
                        grab_dist,
                    });
                    self.point_drag_origin =
                        self.point_edit_node.and_then(|nid| doc.nodes.get(&nid).cloned());
                } else if let Some(anchor_idx) = anchor_hit {
                    // Select this anchor (replace unless Shift/Ctrl is held)
                    if add_sel {
                        if !self.point_selected.contains(&anchor_idx) {
                            self.point_selected.push(anchor_idx);
                        }
                    } else if !self.point_selected.contains(&anchor_idx) {
                        self.point_selected = vec![anchor_idx];
                    }
                    self.point_drag_mode = Some(DirectDrag::Anchors);
                    self.point_drag_origin =
                        self.point_edit_node.and_then(|nid| doc.nodes.get(&nid).cloned());
                } else {
                    // Missed everything — select the shape under the cursor.
                    // direct_select_hit selects on a body click even when unfilled.
                    let hit_shape = direct_select_hit(doc, cx, cy, renderer);
                    self.point_edit_node = hit_shape;
                    self.point_selected.clear();
                    self.point_drag_origin = None;
                    self.point_drag_mode = None;
                }
            }
        }

        // ── During drag ───────────────────────────────────────────────────────
        if response.dragged_by(egui::PointerButton::Primary) && self.point_drag_origin.is_some() {
            let nid = self.point_edit_node;
            // Borrow the mode (don't clone — Corner carries a whole BezPath).
            match &self.point_drag_mode {
                Some(DirectDrag::Anchors) => {
                    if let Some(nid) = nid {
                        if !self.point_selected.is_empty() {
                            let delta = response.drag_delta();
                            let dcx = delta.x as f64 / view.zoom;
                            let dcy = delta.y as f64 / view.zoom;
                            if let Some(node) = doc.nodes.get_mut(&nid) {
                                // Invert the node's linear transform to get a local-space delta
                                let [a, b, c, d, _, _] = node.transform.matrix;
                                let det = a * d - b * c;
                                let (dlx, dly) = if det.abs() > 1e-10 {
                                    ((d * dcx - c * dcy) / det, (-b * dcx + a * dcy) / det)
                                } else {
                                    (dcx, dcy)
                                };
                                if let SceneNodeKind::Path(pn) = &mut node.kind {
                                    let bez = pn.path_data.to_bez_path();
                                    let new_bez =
                                        bez_move_anchors(&bez, &self.point_selected, dlx, dly);
                                    pn.path_data = PathData::from_bez_path(&new_bez);
                                    *doc_modified = true;
                                }
                            }
                        }
                    }
                }
                Some(DirectDrag::Handle { anchor, kind }) => {
                    let (anchor, kind) = (*anchor, *kind);
                    if let (Some(nid), Some(cursor)) = (nid, response.interact_pointer_pos()) {
                        let (ccx, ccy) = view.screen_to_canvas(cursor.x as f64, cursor.y as f64);
                        if let Some(node) = doc.nodes.get_mut(&nid) {
                            let (lx, ly) = canvas_to_local(&node.transform, ccx, ccy);
                            if let SceneNodeKind::Path(pn) = &mut node.kind {
                                let bez = pn.path_data.to_bez_path();
                                // Mirror only on a genuine smooth point (collinear
                                // handles); cusps stay independent. Alt always breaks.
                                let mirror = !alt && is_smooth_anchor(&bez, anchor);
                                let new_bez = bez_set_handle(
                                    &bez,
                                    anchor,
                                    kind,
                                    Point::new(lx, ly),
                                    mirror,
                                );
                                pn.path_data = PathData::from_bez_path(&new_bez);
                                *doc_modified = true;
                            }
                        }
                    }
                }
                Some(DirectDrag::Corner {
                    pivot,
                    origin_bez,
                    grab_dist,
                }) => {
                    let (pivot, grab_dist) = (*pivot, *grab_dist);
                    if let (Some(nid), Some(cursor)) = (nid, response.interact_pointer_pos()) {
                        let (ccx, ccy) = view.screen_to_canvas(cursor.x as f64, cursor.y as f64);
                        let corners = straight_corners(origin_bez);
                        // Radius = how far the cursor has pulled the widget away from
                        // the corner since grab (zero-based, so no snap on grab).
                        let radius = match (doc.nodes.get(&nid), corners.get(&pivot)) {
                            (Some(node), Some((_, corner, _))) => {
                                let (lx, ly) = canvas_to_local(&node.transform, ccx, ccy);
                                let dist = ((lx - corner.x).powi(2) + (ly - corner.y).powi(2)).sqrt();
                                (dist - grab_dist).max(0.0)
                            }
                            _ => 0.0,
                        };
                        // Apply the same radius to every selected straight corner.
                        let mut sel: std::collections::HashSet<usize> = self
                            .point_selected
                            .iter()
                            .copied()
                            .filter(|i| corners.contains_key(i))
                            .collect();
                        if sel.is_empty() {
                            sel.insert(pivot);
                        }
                        let new_bez = round_selected_corners(origin_bez, &sel, radius);
                        if let Some(node) = doc.nodes.get_mut(&nid) {
                            if let SceneNodeKind::Path(pn) = &mut node.kind {
                                pn.path_data = PathData::from_bez_path(&new_bez);
                                *doc_modified = true;
                            }
                        }
                    }
                }
                None => {}
            }
        }

        // ── Drag end: push undo command ───────────────────────────────────────
        if response.drag_stopped_by(egui::PointerButton::Primary) {
            let was_corner = matches!(self.point_drag_mode, Some(DirectDrag::Corner { .. }));
            if let Some(old_node) = self.point_drag_origin.take() {
                if let Some(nid) = self.point_edit_node {
                    if let Some(new_node) = doc.nodes.get(&nid).cloned() {
                        let changed = match (&old_node.kind, &new_node.kind) {
                            (SceneNodeKind::Path(op), SceneNodeKind::Path(np)) => {
                                op.path_data != np.path_data
                            }
                            _ => false,
                        };
                        if changed {
                            history.execute(
                                Command::UpdateNode {
                                    old: old_node,
                                    new: new_node,
                                },
                                doc,
                            );
                        }
                    }
                }
            }
            // Rounding restructures the path, so element indices no longer map to
            // the prior anchor selection — clear it to avoid stale highlights.
            if was_corner {
                self.point_selected.clear();
            }
            self.point_drag_mode = None;
        }

        // ── Click (no drag): select anchor or pick shape ──────────────────────
        // Use interact_pointer_pos() here too — same reasoning as drag_started.
        if response.clicked_by(egui::PointerButton::Primary) {
            if let Some(click_pos) = response.interact_pointer_pos() {
                let (cx, cy) = view.screen_to_canvas(click_pos.x as f64, click_pos.y as f64);

                let hit_anchor = self
                    .point_edit_node
                    .and_then(|nid| find_anchor(nid, cx, cy, doc));

                if let Some(anchor_idx) = hit_anchor {
                    if add_sel {
                        // Toggle
                        if let Some(pos) = self.point_selected.iter().position(|&i| i == anchor_idx)
                        {
                            self.point_selected.remove(pos);
                        } else {
                            self.point_selected.push(anchor_idx);
                        }
                    } else {
                        self.point_selected = vec![anchor_idx];
                    }
                } else {
                    let hit_shape = direct_select_hit(doc, cx, cy, renderer);
                    if let Some(nid) = hit_shape {
                        if Some(nid) != self.point_edit_node {
                            self.point_edit_node = Some(nid);
                            self.point_selected.clear();
                        } else if !add_sel {
                            self.point_selected.clear();
                        }
                    } else {
                        self.point_edit_node = None;
                        self.point_selected.clear();
                    }
                }
            }
        }

        // ── Visual overlay ────────────────────────────────────────────────────
        if let Some(nid) = self.point_edit_node {
            if let Some(node) = doc.nodes.get(&nid) {
                if let SceneNodeKind::Path(pn) = &node.kind {
                    let bez = pn.path_data.to_bez_path();
                    let painter = ui.painter();

                    // Path outline (accent, no fill)
                    let outline_pts = bez_to_screen_points_xf(&bez, view, &node.transform);
                    if outline_pts.len() >= 2 {
                        painter.add(egui::Shape::Path(egui::epaint::PathShape {
                            points: outline_pts,
                            closed: true,
                            fill: Color32::TRANSPARENT,
                            stroke: egui::epaint::PathStroke::new(1.5, accent),
                        }));
                    }

                    // Bezier control handles for selected curved anchors (seam-aware).
                    // Drawn before the anchor squares so anchors sit on top.
                    let anchors = path_anchor_points(&bez);
                    for &i in &self.point_selected {
                        let Some(ap) = anchors.iter().find(|(idx, _)| *idx == i).map(|(_, p)| *p)
                        else {
                            continue;
                        };
                        let (asx, asy) = local_to_screen(&node.transform, view, ap);
                        let a_center = egui::pos2(asx as f32, asy as f32);
                        let (in_h, out_h) = anchor_handle_pair(&bez, i);
                        for h in [in_h, out_h].into_iter().flatten() {
                            let (hsx, hsy) = local_to_screen(&node.transform, view, h.1);
                            let h_center = egui::pos2(hsx as f32, hsy as f32);
                            painter.line_segment([a_center, h_center], egui::Stroke::new(1.0, accent));
                            painter.circle_filled(h_center, 3.5, Color32::WHITE);
                            painter.circle_stroke(h_center, 3.5, egui::Stroke::new(1.5, accent));
                        }
                    }

                    // Which anchor is nearest the hover cursor (for grab highlight)
                    let hovered_anchor = hover_canvas.and_then(|(hx, hy)| {
                        nearest_anchor_screen(&bez, &node.transform, view, hx, hy, ANCHOR_RADIUS_PX)
                    });

                    // Anchor point squares
                    for (idx, local_pt) in &anchors {
                        let (cx, cy) = node.transform.apply(local_pt.x, local_pt.y);
                        let (sx, sy) = view.canvas_to_screen(cx, cy);
                        let center = egui::pos2(sx as f32, sy as f32);
                        let half = 4.5f32;
                        let rect =
                            egui::Rect::from_center_size(center, egui::Vec2::splat(half * 2.0));
                        let selected = self.point_selected.contains(idx);
                        let hovered = hovered_anchor == Some(*idx);
                        if selected {
                            painter.rect_filled(rect, 0.0, accent);
                        } else if hovered {
                            let big = egui::Rect::from_center_size(
                                center,
                                egui::Vec2::splat((half + 2.0) * 2.0),
                            );
                            painter.rect_filled(
                                big,
                                0.0,
                                Color32::from_rgba_unmultiplied(110, 86, 207, 60),
                            );
                            painter.rect_stroke(big, 0.0, egui::Stroke::new(1.5, accent));
                        } else {
                            painter.rect_filled(rect, 0.0, Color32::WHITE);
                            painter.rect_stroke(rect, 0.0, egui::Stroke::new(1.5, accent));
                        }
                    }

                    // Live-Corners rounding widgets for selected straight corners.
                    for &i in &self.point_selected {
                        if let Some((prev, curr, next)) = corner_map.get(&i) {
                            let (wsx, wsy) = ds_corner_widget_screen(
                                &node.transform,
                                view,
                                *prev,
                                *curr,
                                *next,
                                CORNER_INSET_PX,
                            );
                            let c = egui::pos2(wsx as f32, wsy as f32);
                            let hov = hover_canvas
                                .map(|(hx, hy)| {
                                    let (csx, csy) = view.canvas_to_screen(hx, hy);
                                    ((csx - wsx).powi(2) + (csy - wsy).powi(2)).sqrt()
                                        < CORNER_WIDGET_PX
                                })
                                .unwrap_or(false);
                            let r = if hov { 6.0 } else { 5.0 };
                            painter.circle_filled(c, r, Color32::WHITE);
                            painter.circle_stroke(c, r, egui::Stroke::new(1.5, accent));
                            painter.circle_stroke(c, r - 2.5, egui::Stroke::new(1.0, accent));
                        }
                    }
                }
            }
        }
    }
}
