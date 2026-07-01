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
        self.point_context_anchor = None;
        self.point_drag_origin = None;
        self.point_drag_mode = None;
        self.point_marquee_start = None;
        if let Some(nid) = self.point_edit_node {
            if !doc.nodes.contains_key(&nid) {
                self.point_edit_node = None;
            }
        }
    }

    /// Fully drop Direct Select's point-edit state. Called on every tool switch
    /// so re-entering Direct Select re-seeds from the *current* object selection
    /// rather than resurrecting a stale `point_edit_node` (#164 finding 1).
    /// Centralizes the block that was copy-pasted across the tool-switch sites.
    pub(crate) fn clear_point_edit(&mut self) {
        self.point_edit_node = None;
        self.point_selected.clear();
        self.point_context_anchor = None;
        self.point_drag_origin = None;
        self.point_drag_mode = None;
        self.point_marquee_start = None;
    }

    /// Seed Direct Select's point-edit state from the current object selection
    /// so switching into the tool shows every anchor of the selected path
    /// (rendered filled, as if the whole path were selected). #164 requirement 1.
    /// No-op if a node is already being point-edited or nothing suitable is
    /// selected. Only single Path nodes are seeded (see proposal "Out/deferred").
    pub(crate) fn seed_direct_select_from_selection(&mut self, doc: &Document) {
        if self.point_edit_node.is_some() {
            return;
        }
        // Prefer the primary selection id, else the first node in the selection.
        let candidate = self
            .selected_id
            .or_else(|| doc.selection.ids().next().copied());
        if let Some(nid) = candidate {
            if let Some(node) = doc.nodes.get(&nid) {
                if let SceneNodeKind::Path(pn) = &node.kind {
                    let bez = pn.path_data.to_bez_path();
                    self.point_edit_node = Some(nid);
                    self.select_all_anchors(&bez);
                }
            }
        }
    }

    /// Fill `point_selected` with every anchor element index of `bez` so the
    /// whole path renders as selected (filled). #164 requirement 1.
    fn select_all_anchors(&mut self, bez: &kurbo::BezPath) {
        self.point_selected = path_anchor_points(bez).iter().map(|(i, _)| *i).collect();
    }

    /// Convert a single directly-selected anchor between corner and smooth
    /// (curved) via the right-click context menu (#187). Builds one
    /// `Command::UpdateNode` and pushes it through history — the same inline
    /// pattern the delete-anchor block uses — so it undoes/redoes atomically.
    ///
    /// After the convert, the anchor is re-selected by matching its (unchanged)
    /// local position: `bez_convert_anchors` can materialize a closed subpath's
    /// implicit seam into an explicit `CurveTo`, renumbering element indices, so
    /// the pre-convert index is not a reliable handle. Re-selecting keeps the
    /// converted anchor highlighted and — for Smooth — makes its freshly
    /// synthesized in/out handles render and drag immediately.
    fn ds_convert_context_anchor(
        &mut self,
        idx: usize,
        smooth: bool,
        doc: &mut Document,
        view: &CanvasView,
        doc_modified: &mut bool,
        history: &mut CommandHistory,
    ) {
        let Some(nid) = self.point_edit_node else {
            return;
        };
        let Some(old_node) = doc.nodes.get(&nid).cloned() else {
            return;
        };
        let SceneNodeKind::Path(pn) = &old_node.kind else {
            return;
        };
        let bez = pn.path_data.to_bez_path();
        // Snapshot the anchor's local point so it can be re-found after a possible
        // element renumber (seam materialization).
        let anchor_local: Option<Point> = path_anchor_points(&bez)
            .iter()
            .find(|(i, _)| *i == idx)
            .map(|(_, p)| *p);
        let sel: std::collections::HashSet<usize> = std::iter::once(idx).collect();
        let new_bez = bez_convert_anchors(&bez, &sel, smooth);
        let mut new_node = old_node.clone();
        if let SceneNodeKind::Path(np) = &mut new_node.kind {
            np.path_data = PathData::from_bez_path(&new_bez);
        }
        history.execute(
            Command::UpdateNode {
                old: old_node,
                new: new_node,
            },
            doc,
        );
        *doc_modified = true;

        // Re-select the (possibly renumbered) anchor at its unchanged position so
        // its curvature handles surface and become draggable on the next frame.
        self.point_selected.clear();
        if let (Some(p), Some(node)) = (anchor_local, doc.nodes.get(&nid)) {
            if let SceneNodeKind::Path(pn) = &node.kind {
                let nb = pn.path_data.to_bez_path();
                let (acx, acy) = node.transform.apply(p.x, p.y);
                if let Some(new_idx) =
                    nearest_anchor_screen(&nb, &node.transform, view, acx, acy, 12.0)
                {
                    self.point_selected = vec![new_idx];
                }
            }
        }
    }

    /// Fillet a single directly-selected straight corner via the right-click
    /// context menu (#187). Reuses `round_selected_corners` — the same helper the
    /// inspector "Round corners" buttons and the on-canvas Live-Corners widget
    /// use — restricted to genuine straight corners (rounding a curve-adjacent
    /// anchor would flatten the curve). Rounding restructures element indices, so
    /// the selection is dropped afterwards, matching the shared handler.
    fn ds_round_context_anchor(
        &mut self,
        idx: usize,
        radius: f64,
        doc: &mut Document,
        doc_modified: &mut bool,
        history: &mut CommandHistory,
    ) {
        let Some(nid) = self.point_edit_node else {
            return;
        };
        let Some(old_node) = doc.nodes.get(&nid).cloned() else {
            return;
        };
        let SceneNodeKind::Path(pn) = &old_node.kind else {
            return;
        };
        let bez = pn.path_data.to_bez_path();
        let straight = straight_corners(&bez);
        if !straight.contains_key(&idx) {
            // Not a straight corner — nothing to round.
            return;
        }
        let sel: std::collections::HashSet<usize> = std::iter::once(idx).collect();
        let new_bez = round_selected_corners(&bez, &sel, radius);
        let mut new_node = old_node.clone();
        if let SceneNodeKind::Path(np) = &mut new_node.kind {
            np.path_data = PathData::from_bez_path(&new_bez);
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

    /// Hit-test the current point-edit node's anchors at canvas position
    /// `(cx, cy)`, returning the nearest anchor's element index within the
    /// grab radius. Shared by the tool handler and by the radial-wheel
    /// suppression guard in `app/mod.rs` (#187): a right-click on a directly-
    /// selected anchor must reach the point-type context menu rather than pop
    /// the global selection wheel, so both paths must agree on "is there an
    /// anchor here?" using the identical hit-test.
    pub(crate) fn ds_anchor_at(
        &self,
        cx: f64,
        cy: f64,
        doc: &Document,
        view: &CanvasView,
    ) -> Option<usize> {
        // Same generous radius the tool handler uses for anchor grabs.
        const ANCHOR_RADIUS_PX: f64 = 12.0;
        let nid = self.point_edit_node?;
        let node = doc.nodes.get(&nid)?;
        if let SceneNodeKind::Path(pn) = &node.kind {
            let bez = pn.path_data.to_bez_path();
            nearest_anchor_screen(&bez, &node.transform, view, cx, cy, ANCHOR_RADIUS_PX)
        } else {
            None
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
            self.point_context_anchor = None;
            self.point_drag_origin = None;
            self.point_drag_mode = None;
            self.point_marquee_start = None;
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
        let hover_cursor = match (
            self.point_edit_node.and_then(|nid| doc.nodes.get(&nid)),
            hover_canvas,
        ) {
            (Some(node), Some((hx, hy))) => {
                if ds_find_handle(node, view, &self.point_selected, hx, hy, HANDLE_RADIUS_PX)
                    .is_some()
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
                } else if self.ds_anchor_at(hx, hy, doc, view).is_some() {
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

        // ── Right-click an anchor → point-type / round context menu (#187) ────
        // Discoverability: the geometry (bez_convert_anchors, round_selected_corners)
        // and the on-canvas handle render/drag already exist; the missing piece was
        // a right-click entry point. On secondary click, hit-test the anchor at the
        // press location, select it, and record it as the menu target.
        if response.secondary_clicked() {
            if let Some(press_pos) = response.interact_pointer_pos() {
                let (cx, cy) = view.screen_to_canvas(press_pos.x as f64, press_pos.y as f64);
                let hit = self.ds_anchor_at(cx, cy, doc, view);
                if let Some(idx) = hit {
                    // Select the right-clicked anchor (replace unless Shift/Ctrl is
                    // held to add) so the menu acts on a visible selection.
                    if add_sel {
                        if !self.point_selected.contains(&idx) {
                            self.point_selected.push(idx);
                        }
                    } else if !self.point_selected.contains(&idx) {
                        self.point_selected = vec![idx];
                    }
                    self.point_context_anchor = Some(idx);
                } else {
                    // Right-click missed every anchor — no anchor menu this time.
                    self.point_context_anchor = None;
                }
            }
        }

        // Registered every frame so egui can keep the menu open across frames; the
        // closure renders items only when an anchor is the active context target.
        response.context_menu(|ui| {
            let (Some(ctx_idx), Some(_nid)) = (self.point_context_anchor, self.point_edit_node)
            else {
                // No anchor was right-clicked (empty space / non-path) — close so no
                // empty menu lingers.
                ui.close_menu();
                return;
            };
            ui.label(egui::RichText::new("Anchor point").weak().small());
            if ui
                .button("Corner")
                .on_hover_text("Retract this anchor's handles → sharp corner")
                .clicked()
            {
                self.ds_convert_context_anchor(ctx_idx, false, doc, view, doc_modified, history);
                self.point_context_anchor = None;
                ui.close_menu();
            }
            if ui
                .button("Smooth / Curved")
                .on_hover_text("Add collinear handles → smooth curve; drag the handles to shape it")
                .clicked()
            {
                self.ds_convert_context_anchor(ctx_idx, true, doc, view, doc_modified, history);
                self.point_context_anchor = None;
                ui.close_menu();
            }
            ui.menu_button("Round corner", |ui| {
                for r in [4.0_f64, 8.0, 16.0, 32.0] {
                    if ui
                        .button(format!("{r:.0} px"))
                        .on_hover_text("Fillet this straight corner by this radius")
                        .clicked()
                    {
                        self.ds_round_context_anchor(ctx_idx, r, doc, doc_modified, history);
                        self.point_context_anchor = None;
                        ui.close_menu();
                    }
                }
            });
        });

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
                let anchor_hit = self.ds_anchor_at(cx, cy, doc, view);
                let origin_bez = edit_node.and_then(|node| match &node.kind {
                    SceneNodeKind::Path(pn) => Some(pn.path_data.to_bez_path()),
                    _ => None,
                });

                if let Some((anchor, kind)) = handle_hit {
                    self.point_drag_mode = Some(DirectDrag::Handle { anchor, kind });
                    self.point_drag_origin = self
                        .point_edit_node
                        .and_then(|nid| doc.nodes.get(&nid).cloned());
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
                    self.point_drag_origin = self
                        .point_edit_node
                        .and_then(|nid| doc.nodes.get(&nid).cloned());
                } else if let Some(anchor_idx) = anchor_hit {
                    if add_sel {
                        // Shift/Ctrl adds the pressed anchor to the selection.
                        if !self.point_selected.contains(&anchor_idx) {
                            self.point_selected.push(anchor_idx);
                        }
                    } else if !self.point_selected.contains(&anchor_idx) {
                        // Only collapse to a single anchor when grabbing an
                        // UNselected one; grabbing a member of the current
                        // multi-selection keeps the whole set so the drag moves
                        // every selected anchor together via DirectDrag::Anchors
                        // + bez_move_anchors (#181 requirement 2).
                        self.point_selected = vec![anchor_idx];
                    }
                    self.point_drag_mode = Some(DirectDrag::Anchors);
                    self.point_drag_origin = self
                        .point_edit_node
                        .and_then(|nid| doc.nodes.get(&nid).cloned());
                } else {
                    // Missed anchors/handles/corners — this is a body press.
                    // direct_select_hit selects on a body click even when unfilled.
                    let hit_shape = direct_select_hit(doc, cx, cy, renderer);
                    if hit_shape.is_some() && hit_shape == self.point_edit_node {
                        // Pressing the fill of the already-selected shape starts a
                        // whole-shape move (#164 requirement 2). Capture the node
                        // and its original translation so the drag is stable.
                        if let Some(nid) = self.point_edit_node {
                            if let Some(node) = doc.nodes.get(&nid) {
                                let start_e = node.transform.matrix[4];
                                let start_f = node.transform.matrix[5];
                                self.point_drag_origin = Some(node.clone());
                                // Capture a canvas-space reference point (bbox
                                // top-left) so grid snap aligns the shape's edge
                                // to the grid, matching the Move tool's
                                // move_snap_ref (#181 requirement 3).
                                let ref_pt = selection_canvas_bounds(doc, &[nid], renderer)
                                    .map(|(x0, y0, _, _)| (x0, y0));
                                self.point_drag_mode = Some(DirectDrag::Shape {
                                    start_e,
                                    start_f,
                                    ref_pt,
                                });
                            }
                        }
                    } else if let Some(nid) = hit_shape {
                        // A different (or first) shape — select it and show all of
                        // its anchors filled. No move on this press: select first,
                        // then a subsequent fill drag moves it (#164 requirement 1).
                        self.point_edit_node = Some(nid);
                        let bez = match doc.nodes.get(&nid).map(|n| &n.kind) {
                            Some(SceneNodeKind::Path(pn)) => Some(pn.path_data.to_bez_path()),
                            _ => None,
                        };
                        match bez {
                            Some(bez) => self.select_all_anchors(&bez),
                            None => self.point_selected.clear(),
                        }
                        self.point_drag_origin = None;
                        self.point_drag_mode = None;
                    } else if self.point_edit_node.is_some() {
                        // Empty canvas while a path is being point-edited: begin a
                        // rubber-band marquee to select the enclosed anchors of the
                        // edit path (#181 requirement 1). Tracked separately from
                        // point_drag_mode — a marquee changes only the selection, so
                        // no drag origin / undo is recorded.
                        self.point_marquee_start = Some(press_pos);
                        self.point_drag_origin = None;
                        self.point_drag_mode = None;
                    } else {
                        // Empty canvas with nothing being edited — deselect.
                        self.point_edit_node = None;
                        self.point_selected.clear();
                        self.point_drag_origin = None;
                        self.point_drag_mode = None;
                    }
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
                                let new_bez =
                                    bez_set_handle(&bez, anchor, kind, Point::new(lx, ly), mirror);
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
                                let dist =
                                    ((lx - corner.x).powi(2) + (ly - corner.y).powi(2)).sqrt();
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
                Some(DirectDrag::Shape {
                    start_e,
                    start_f,
                    ref_pt,
                }) => {
                    let (start_e, start_f, ref_pt) = (*start_e, *start_f, *ref_pt);
                    // Total canvas-space delta from the press point, mirroring the
                    // Move tool (tool_handlers.rs). Writes translation directly.
                    if let (Some(nid), Some(press), Some(cursor)) = (
                        nid,
                        ui.input(|i| i.pointer.press_origin()),
                        response.interact_pointer_pos(),
                    ) {
                        let raw_dx = (cursor.x - press.x) as f64 / view.zoom;
                        let raw_dy = (cursor.y - press.y) as f64 / view.zoom;
                        // Shift locks the move to 8 directions (axis-lock beats
                        // grid snap); otherwise snap a canvas-space reference
                        // point (the shape's bbox top-left) to the grid so the
                        // shape's edge lands ON grid lines, exactly like the Move
                        // tool's `self.snap(rx + raw_dx) - rx` (#181 requirement
                        // 3). Snapping the reference point instead of the raw
                        // target is what makes the shape align to the grid rather
                        // than merely stepping in grid-sized increments. No-op
                        // unless prefs.snap_to_grid is enabled.
                        let (dx, dy) = if shift {
                            axis_lock_8(raw_dx, raw_dy)
                        } else {
                            match ref_pt {
                                Some((rx, ry)) => {
                                    (self.snap(rx + raw_dx) - rx, self.snap(ry + raw_dy) - ry)
                                }
                                None => (raw_dx, raw_dy),
                            }
                        };
                        let (te, tf) = (start_e + dx, start_f + dy);
                        if let Some(node) = doc.nodes.get_mut(&nid) {
                            node.transform.matrix[4] = te;
                            node.transform.matrix[5] = tf;
                            *doc_modified = true;
                        }
                    }
                }
                None => {}
            }
        }

        // ── Marquee complete: select the enclosed anchors ─────────────────────
        // Runs independently of the geometry-drag history block below: a marquee
        // only changes the anchor selection, records no undo (#181 requirement 1).
        if response.drag_stopped_by(egui::PointerButton::Primary) {
            if let Some(start_pos) = self.point_marquee_start.take() {
                let end_pos = response
                    .interact_pointer_pos()
                    .or_else(|| ui.input(|i| i.pointer.hover_pos()))
                    .unwrap_or(start_pos);
                let rect = egui::Rect::from_two_pos(start_pos, end_pos);
                // Collect every anchor of the edit path whose screen position lies
                // inside the marquee rect. Mapping matches the anchor-square overlay:
                // node.transform.apply (local→canvas) then view.canvas_to_screen.
                let hits: Vec<usize> = self
                    .point_edit_node
                    .and_then(|nid| doc.nodes.get(&nid))
                    .and_then(|node| match &node.kind {
                        SceneNodeKind::Path(pn) => {
                            let bez = pn.path_data.to_bez_path();
                            Some(
                                path_anchor_points(&bez)
                                    .into_iter()
                                    .filter_map(|(idx, p)| {
                                        let (cx, cy) = node.transform.apply(p.x, p.y);
                                        let (sx, sy) = view.canvas_to_screen(cx, cy);
                                        rect.contains(egui::pos2(sx as f32, sy as f32))
                                            .then_some(idx)
                                    })
                                    .collect::<Vec<usize>>(),
                            )
                        }
                        _ => None,
                    })
                    .unwrap_or_default();

                if rect.area() < 1.0 && hits.is_empty() {
                    // Zero-area marquee (a click-through on empty canvas) with no
                    // enclosed anchors falls back to a plain deselect.
                    if !add_sel {
                        self.point_selected.clear();
                    }
                } else if add_sel {
                    // Shift/Ctrl unions the marquee hits with the current selection.
                    for idx in hits {
                        if !self.point_selected.contains(&idx) {
                            self.point_selected.push(idx);
                        }
                    }
                } else {
                    // Plain marquee replaces the selection with the enclosed anchors.
                    self.point_selected = hits;
                }
                // A marquee never carries a geometry drag; make sure the drag-end
                // history block below sees no stale origin/mode.
                self.point_drag_origin = None;
                self.point_drag_mode = None;
            }
        }

        // ── Drag end: push undo command ───────────────────────────────────────
        if response.drag_stopped_by(egui::PointerButton::Primary) {
            let was_corner = matches!(self.point_drag_mode, Some(DirectDrag::Corner { .. }));
            if let Some(old_node) = self.point_drag_origin.take() {
                if let Some(nid) = self.point_edit_node {
                    if let Some(new_node) = doc.nodes.get(&nid).cloned() {
                        // A whole-shape move changes only the transform, so the
                        // path_data-only check would miss it — compare both (#164).
                        let changed = match (&old_node.kind, &new_node.kind) {
                            (SceneNodeKind::Path(op), SceneNodeKind::Path(np)) => {
                                op.path_data != np.path_data
                            }
                            _ => false,
                        } || old_node.transform.matrix != new_node.transform.matrix;
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

                let hit_anchor = self.ds_anchor_at(cx, cy, doc, view);

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
                            // A plain click on a shape body newly selects it — fill
                            // every anchor so it renders "whole path selected", same
                            // as the drag_started body branch (#164 finding 2). Without
                            // this a clean click leaves anchors white/unfilled.
                            self.point_edit_node = Some(nid);
                            let bez = match doc.nodes.get(&nid).map(|n| &n.kind) {
                                Some(SceneNodeKind::Path(pn)) => Some(pn.path_data.to_bez_path()),
                                _ => None,
                            };
                            match bez {
                                Some(bez) => self.select_all_anchors(&bez),
                                None => self.point_selected.clear(),
                            }
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

        // ── Marquee rubber-band overlay ───────────────────────────────────────
        // Mirrors the Move tool's marquee (tool_handlers.rs): a translucent accent
        // rectangle from the press point to the current cursor while dragging (#181).
        if let Some(start_pos) = self.point_marquee_start {
            let current_pos = ui.input(|i| i.pointer.hover_pos()).unwrap_or(start_pos);
            let rect = egui::Rect::from_two_pos(start_pos, current_pos);
            ui.painter().rect(
                rect,
                0.0,
                Color32::from_rgba_unmultiplied(110, 86, 207, 30),
                egui::Stroke::new(1.0, accent),
            );
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
                            painter
                                .line_segment([a_center, h_center], egui::Stroke::new(1.0, accent));
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
