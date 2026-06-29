//! Interactive tool handlers (Select, Pen, Shape Builder) and shape-builder
//! geometry, extracted from app::mod. Methods on PhotonicApp.
#![allow(clippy::too_many_arguments)]
use super::*;

impl PhotonicApp {
    pub(crate) fn handle_select_tool(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        doc: &mut Document,
        view: &CanvasView,
        renderer: &mut PhotonicRenderer,
        doc_modified: &mut bool,
        history: &mut CommandHistory,
    ) {
        // ── Keyboard shortcuts (skipped when a text widget has focus) ─────────
        if viewport_kb(ui.ctx()) {
            if let Some(sel_id) = self.selected_id {
                let (delete, ctrl, shift, bracket_right, bracket_left, key_g) = ui.input(|i| {
                    (
                        i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace),
                        i.modifiers.ctrl,
                        i.modifiers.shift,
                        i.key_pressed(egui::Key::CloseBracket),
                        i.key_pressed(egui::Key::OpenBracket),
                        i.key_pressed(egui::Key::G),
                    )
                });

                // Delete / Backspace: remove all selected nodes
                if delete {
                    let ids_to_delete: Vec<NodeId> = doc.selection.ids().copied().collect();
                    for id in ids_to_delete {
                        doc.remove_node(&id);
                    }
                    doc.selection.clear();
                    self.selected_id = None;
                    *doc_modified = true;
                    return;
                }

                // Z-order shortcuts: Ctrl+] / Ctrl+[ (with Shift for extremes)
                if ctrl && (bracket_right || bracket_left) {
                    if let Some((layer_id, cur_idx)) = doc.node_layer_and_index(&sel_id) {
                        let layer_len = doc
                            .layers
                            .get(&layer_id)
                            .map(|l| l.node_ids.len())
                            .unwrap_or(0);
                        if layer_len > 0 {
                            let new_index = if bracket_right && shift {
                                layer_len - 1 // Bring to Front
                            } else if bracket_left && shift {
                                0 // Send to Back
                            } else if bracket_right {
                                (cur_idx + 1).min(layer_len - 1) // Bring Forward
                            } else {
                                cur_idx.saturating_sub(1) // Send Backward
                            };
                            if new_index != cur_idx {
                                let cmd = Command::ReorderNode {
                                    layer_id,
                                    node_id: sel_id,
                                    old_index: cur_idx,
                                    new_index,
                                };
                                history.execute(cmd, doc);
                                *doc_modified = true;
                            }
                        }
                    }
                }

                // Ctrl+Shift+G: ungroup (only if selected node is a group)
                if ctrl && shift && key_g {
                    if let Some(node) = doc.get_node(&sel_id) {
                        if let SceneNodeKind::Group(g) = &node.kind {
                            let children = g.children.clone();
                            let node_clone = node.clone();
                            if let Some((layer_id, group_index)) = doc.node_layer_and_index(&sel_id)
                            {
                                let first_child = children.first().copied();
                                let cmd = Command::UngroupNodes {
                                    group: node_clone,
                                    layer_id,
                                    group_index,
                                    children,
                                };
                                history.execute(cmd, doc);
                                self.selected_id = first_child;
                                if let Some(fc) = first_child {
                                    doc.selection = Selection::single(fc);
                                } else {
                                    doc.selection.clear();
                                }
                                *doc_modified = true;
                            }
                        }
                    }
                }
            }

            // Ctrl+G: group selected nodes (requires 2+ in selection)
            let (ctrl_g, shift_g) = ui.input(|i| {
                (
                    i.modifiers.ctrl && !i.modifiers.shift && i.key_pressed(egui::Key::G),
                    i.modifiers.ctrl && i.modifiers.shift && i.key_pressed(egui::Key::G),
                )
            });
            if ctrl_g && !shift_g && doc.selection.count() >= 2 {
                self.do_group_selected(doc, history, doc_modified);
            }

            // Ctrl+Y: toggle Outline Mode
            if ui.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::Y)) {
                self.outline_mode = !self.outline_mode;
            }

            // Ctrl+;: toggle guide visibility
            if ui.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::Semicolon)) {
                self.guides_visible = !self.guides_visible;
            }

            // Ctrl+C: copy selected nodes to in-process clipboard.
            if ui.input(|i| i.modifiers.ctrl && !i.modifiers.shift && i.key_pressed(egui::Key::C)) {
                self.gui_clipboard.clear();
                for nid in doc.selection.ids() {
                    if let Some(node) = doc.nodes.get(nid) {
                        self.gui_clipboard.push(node.clone());
                    }
                }
            }

            // Ctrl+V: paste from clipboard with +10px offset.
            // Ctrl+Shift+V: paste in place (exact original coordinates).
            let (paste, paste_in_place) = ui.input(|i| {
                (
                    i.modifiers.ctrl && !i.modifiers.shift && i.key_pressed(egui::Key::V),
                    i.modifiers.ctrl && i.modifiers.shift && i.key_pressed(egui::Key::V),
                )
            });
            if (paste || paste_in_place) && !self.gui_clipboard.is_empty() {
                let offset = if paste { 10.0_f64 } else { 0.0 };
                if let Some(target_layer) = doc
                    .active_layer_id
                    .or_else(|| doc.layer_order.first().copied())
                {
                    let mut cmds: Vec<Command> = Vec::new();
                    let mut new_ids: Vec<NodeId> = Vec::new();
                    for src in &self.gui_clipboard {
                        let mut new_node = src.clone();
                        new_node.id = uuid::Uuid::new_v4();
                        new_node.layer_id = target_layer;
                        if offset.abs() > 1e-9 {
                            new_node.transform.matrix[4] += offset;
                            new_node.transform.matrix[5] += offset;
                        }
                        new_ids.push(new_node.id);
                        cmds.push(Command::AddNode {
                            node: new_node,
                            layer_id: Some(target_layer),
                        });
                    }
                    if !cmds.is_empty() {
                        history.execute(Command::Batch(cmds), doc);
                        doc.selection = Selection::from_ids(new_ids.iter().copied());
                        if let Some(first) = new_ids.first() {
                            self.selected_id = Some(*first);
                        }
                        *doc_modified = true;
                    }
                }
            }

            // Ctrl+Shift+H: flip horizontal / Ctrl+Shift+V: flip vertical
            if ui.input(|i| i.modifiers.ctrl && i.modifiers.shift && i.key_pressed(egui::Key::H)) {
                let sel: Vec<NodeId> = doc.selection.node_ids.iter().copied().collect();
                for nid in &sel {
                    if let Some(node) = doc.nodes.get(nid) {
                        if let SceneNodeKind::Path(pn) = &node.kind {
                            use kurbo::Shape;
                            let bez = pn.path_data.to_bez_path();
                            let bbox = bez.bounding_box();
                            let cx = bbox.x0 + bbox.width() / 2.0;
                            let mut new_bez = BezPath::new();
                            for el in bez.elements() {
                                let flip = |p: kurbo::Point| kurbo::Point::new(2.0 * cx - p.x, p.y);
                                match *el {
                                    PathEl::MoveTo(p) => new_bez.move_to(flip(p)),
                                    PathEl::LineTo(p) => new_bez.line_to(flip(p)),
                                    PathEl::CurveTo(c1, c2, p) => {
                                        new_bez.curve_to(flip(c1), flip(c2), flip(p))
                                    }
                                    PathEl::QuadTo(c, p) => new_bez.quad_to(flip(c), flip(p)),
                                    PathEl::ClosePath => new_bez.close_path(),
                                }
                            }
                            let mut new_node = node.clone();
                            if let SceneNodeKind::Path(ref mut np) = new_node.kind {
                                np.path_data = PathData::from_bez_path(&new_bez);
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
                }
            }
            if ui.input(|i| i.modifiers.ctrl && i.modifiers.shift && i.key_pressed(egui::Key::J)) {
                let sel: Vec<NodeId> = doc.selection.node_ids.iter().copied().collect();
                for nid in &sel {
                    if let Some(node) = doc.nodes.get(nid) {
                        if let SceneNodeKind::Path(pn) = &node.kind {
                            use kurbo::Shape;
                            let bez = pn.path_data.to_bez_path();
                            let bbox = bez.bounding_box();
                            let cy = bbox.y0 + bbox.height() / 2.0;
                            let mut new_bez = BezPath::new();
                            for el in bez.elements() {
                                let flip = |p: kurbo::Point| kurbo::Point::new(p.x, 2.0 * cy - p.y);
                                match *el {
                                    PathEl::MoveTo(p) => new_bez.move_to(flip(p)),
                                    PathEl::LineTo(p) => new_bez.line_to(flip(p)),
                                    PathEl::CurveTo(c1, c2, p) => {
                                        new_bez.curve_to(flip(c1), flip(c2), flip(p))
                                    }
                                    PathEl::QuadTo(c, p) => new_bez.quad_to(flip(c), flip(p)),
                                    PathEl::ClosePath => new_bez.close_path(),
                                }
                            }
                            let mut new_node = node.clone();
                            if let SceneNodeKind::Path(ref mut np) = new_node.kind {
                                np.path_data = PathData::from_bez_path(&new_bez);
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
                }
            }

            // Ctrl+Z: undo / Ctrl+R: redo

            let (ctrl_z, ctrl_r) = ui.input(|i| {
                (
                    i.modifiers.ctrl && !i.modifiers.shift && i.key_pressed(egui::Key::Z),
                    i.modifiers.ctrl && i.key_pressed(egui::Key::R),
                )
            });
            if ctrl_z {
                if history.undo(doc) {
                    self.selected_id = doc.selection.ids().next().copied();
                    self.invalidate_point_edit(doc);
                    *doc_modified = true;
                }
            }
            if ctrl_r {
                if history.redo(doc) {
                    self.selected_id = doc.selection.ids().next().copied();
                    self.invalidate_point_edit(doc);
                    *doc_modified = true;
                }
            }
        } // end viewport_kb

        // ── Isolation Mode: Escape exits ─────────────────────────────────────
        if self.isolated_group.is_some() {
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.isolated_group = None;
                doc.selection.clear();
                self.selected_id = None;
            }
        }

        // ── Double-click: enter Isolation Mode on a group ─────────────────────
        if response.double_clicked_by(egui::PointerButton::Primary) {
            if let Some(pos) = response.interact_pointer_pos() {
                let (cx, cy) = view.screen_to_canvas(pos.x as f64, pos.y as f64);
                let hit = hit_test(doc, cx, cy, renderer);
                if let Some(id) = hit {
                    if let Some(node) = doc.nodes.get(&id) {
                        if matches!(node.kind, SceneNodeKind::Group(_)) {
                            self.isolated_group = Some(id);
                            // Select children of the group.
                            if let SceneNodeKind::Group(g) = &node.kind {
                                doc.selection.clear();
                                for cid in &g.children {
                                    doc.selection.add(*cid);
                                }
                                self.selected_id = g.children.first().copied();
                            }
                            *doc_modified = true;
                            return;
                        }
                    }
                }
                // Double-click on non-group or empty: exit isolation if active
                if self.isolated_group.is_some() {
                    self.isolated_group = None;
                    doc.selection.clear();
                    self.selected_id = None;
                }
            }
        }

        // Drag-to-move or resize selected node
        if response.drag_started_by(egui::PointerButton::Primary) {
            // Use press_origin (where the user first clicked) rather than
            // interact_pointer_pos (current position after drag threshold), so that
            // clicks near bounding-box edges still register as "on the selected node".
            if let Some(pos) = ui.input(|i| i.pointer.press_origin()) {
                let (cx, cy) = view.screen_to_canvas(pos.x as f64, pos.y as f64);
                let shift = ui.input(|i| i.modifiers.shift);

                // Compute effective selection bounds: combined bbox for multi, single for one.
                let sel_ids: Vec<NodeId> = doc.selection.ids().copied().collect();
                let effective_bounds = if sel_ids.len() > 1 {
                    selection_canvas_bounds(doc, &sel_ids, renderer)
                } else {
                    self.selected_id
                        .and_then(|id| doc.nodes.get(&id))
                        .and_then(|n| text_aware_canvas_bounds(n, renderer))
                };

                // Check if click lands on a corner resize handle.
                const HANDLE_HIT: f32 = 6.0;
                let resize_hit = effective_bounds.and_then(|(bx0, by0, bx1, by1)| {
                    let (sx0, sy0) = view.canvas_to_screen(bx0, by0);
                    let (sx1, sy1) = view.canvas_to_screen(bx1, by1);
                    let p = pos;
                    let corners = [
                        (egui::pos2(sx0 as f32, sy0 as f32), ResizeHandle::TopLeft),
                        (egui::pos2(sx1 as f32, sy0 as f32), ResizeHandle::TopRight),
                        (egui::pos2(sx0 as f32, sy1 as f32), ResizeHandle::BottomLeft),
                        (
                            egui::pos2(sx1 as f32, sy1 as f32),
                            ResizeHandle::BottomRight,
                        ),
                    ];
                    corners
                        .iter()
                        .find(|(c, _)| (p - *c).length() <= HANDLE_HIT)
                        .map(|(_, h)| *h)
                });

                if let Some(handle) = resize_hit {
                    self.resizing = Some(handle);
                    self.resize_origin_bounds = effective_bounds;
                    // Snapshot the nodes being resized so the drag can be recorded
                    // as a single undoable history step on release (#5).
                    self.resize_drag_origins = if sel_ids.len() > 1 {
                        sel_ids
                            .iter()
                            .filter_map(|id| doc.nodes.get(id).cloned())
                            .collect()
                    } else {
                        self.selected_id
                            .and_then(|id| doc.nodes.get(&id))
                            .cloned()
                            .into_iter()
                            .collect()
                    };
                    if sel_ids.len() > 1 {
                        // Multi-node resize: capture every selected node's transform
                        self.resize_multi_origins = sel_ids
                            .iter()
                            .filter_map(|&id| doc.nodes.get(&id).map(|n| (id, n.transform.matrix)))
                            .collect();
                        self.resize_origin_transform = None;
                        self.resize_origin_font_size = None;
                    } else {
                        // Single-node resize: existing behaviour (text gets font_size scaling)
                        self.resize_multi_origins.clear();
                        self.resize_origin_transform = self
                            .selected_id
                            .and_then(|id| doc.nodes.get(&id))
                            .map(|n| n.transform.matrix);
                        self.resize_origin_font_size = self
                            .selected_id
                            .and_then(|id| doc.nodes.get(&id))
                            .and_then(|n| {
                                if let SceneNodeKind::Text(t) = &n.kind {
                                    Some(t.font_size)
                                } else {
                                    None
                                }
                            });
                    }
                } else {
                    // Check if click is within the effective selection bounds (body).
                    let on_selected = match effective_bounds {
                        Some((x0, y0, x1, y1)) => cx >= x0 && cx <= x1 && cy >= y0 && cy <= y1,
                        None => self.selected_id.is_some(),
                    };

                    // Dragging within the selection bounds moves it — including
                    // with Shift (axis-lock) or Alt (duplicate). Shift only falls
                    // through to marquee/extend-select when NOT on the selection.
                    if on_selected {
                        self.moving = true;
                    } else {
                        // Try selecting a new node at the click point
                        let hit = {
                            let raw = hit_test(doc, cx, cy, renderer);
                            // In isolation mode, only accept hits that are children of the isolated group.
                            if let Some(iso_id) = self.isolated_group {
                                raw.filter(|id| {
                                    doc.nodes
                                        .get(&iso_id)
                                        .and_then(|n| {
                                            if let SceneNodeKind::Group(g) = &n.kind {
                                                Some(&g.children)
                                            } else {
                                                None
                                            }
                                        })
                                        .map(|children| children.contains(id))
                                        .unwrap_or(false)
                                })
                            } else {
                                raw
                            }
                        };
                        if shift {
                            if let Some(id) = hit {
                                doc.selection.toggle(id);
                                self.selected_id = Some(id);
                            } else {
                                // Shift+drag on empty space → additive marquee
                                self.marquee_start = Some(pos);
                            }
                        } else {
                            let alt = ui.input(|i| i.modifiers.alt);
                            // Alt+click: if the hit node is a group, select the
                            // topmost child of that group instead (Group Selection behavior).
                            let effective_hit = if alt {
                                hit.and_then(|id| {
                                    if let Some(SceneNodeKind::Group(g)) =
                                        doc.nodes.get(&id).map(|n| &n.kind)
                                    {
                                        // Return topmost (last) child that exists in the document.
                                        g.children
                                            .iter()
                                            .rev()
                                            .find(|cid| doc.nodes.contains_key(*cid))
                                            .copied()
                                    } else {
                                        Some(id)
                                    }
                                })
                            } else {
                                hit
                            };
                            self.selected_id = effective_hit;
                            self.moving = effective_hit.is_some() && !alt;
                            match self.selected_id {
                                Some(id) => doc.selection = Selection::single(id),
                                None => {
                                    doc.selection.clear();
                                    // Drag on empty space → begin marquee selection
                                    self.marquee_start = Some(pos);
                                }
                            }
                        }
                    }
                }
            }
        }

        if response.dragged_by(egui::PointerButton::Primary) {
            if self.resizing.is_some() {
                if let (Some(handle), Some((bx0, by0, bx1, by1))) =
                    (self.resizing, self.resize_origin_bounds)
                {
                    if let Some(pos) = response.interact_pointer_pos() {
                        let (px, py) = view.screen_to_canvas(pos.x as f64, pos.y as f64);
                        let orig_w = bx1 - bx0;
                        let orig_h = by1 - by0;
                        if orig_w.abs() > 1e-9 && orig_h.abs() > 1e-9 {
                            let (anchor_x, anchor_y, mut sx, mut sy) = match handle {
                                ResizeHandle::TopLeft => {
                                    (bx1, by1, (bx1 - px) / orig_w, (by1 - py) / orig_h)
                                }
                                ResizeHandle::TopRight => {
                                    (bx0, by1, (px - bx0) / orig_w, (by1 - py) / orig_h)
                                }
                                ResizeHandle::BottomLeft => {
                                    (bx1, by0, (bx1 - px) / orig_w, (py - by0) / orig_h)
                                }
                                ResizeHandle::BottomRight => {
                                    (bx0, by0, (px - bx0) / orig_w, (py - by0) / orig_h)
                                }
                            };

                            // Shift constrains the resize to a uniform scale so the
                            // selection keeps its aspect ratio (#4). The
                            // larger-magnitude axis wins; signs (flips across the
                            // anchor) are preserved.
                            if ui.input(|i| i.modifiers.shift) {
                                let s = sx.abs().max(sy.abs());
                                sx = s.copysign(sx);
                                sy = s.copysign(sy);
                            }

                            if !self.resize_multi_origins.is_empty() {
                                // Multi-node resize: apply the same scale to every node
                                use photonic_core::transform::Transform;
                                let t_scale = Transform::scale_around(sx, sy, anchor_x, anchor_y);
                                let origins = self.resize_multi_origins.clone();
                                for (id, orig_xf) in origins {
                                    if let Some(node) = doc.nodes.get_mut(&id) {
                                        // Scale is in canvas space, so it composes
                                        // AFTER the node's own transform.
                                        node.transform =
                                            t_scale.then(&Transform { matrix: orig_xf });
                                    }
                                }
                                *doc_modified = true;
                            } else if let (Some(orig_xf), Some(sel_id)) =
                                (self.resize_origin_transform, self.selected_id)
                            {
                                // Single-node resize (with text font_size special case)
                                if let Some(node) = doc.nodes.get_mut(&sel_id) {
                                    if let SceneNodeKind::Text(text) = &mut node.kind {
                                        if let Some(orig_fs) = self.resize_origin_font_size {
                                            let scale = sy.abs().max(0.01);
                                            text.font_size = (orig_fs * scale).max(1.0);
                                            let new_w = (bx1 - bx0) * scale;
                                            let new_h = (by1 - by0) * scale;
                                            let (tx, ty) = match handle {
                                                ResizeHandle::BottomRight => (bx0, by0),
                                                ResizeHandle::TopLeft => (bx1 - new_w, by1 - new_h),
                                                ResizeHandle::TopRight => (bx0, by1 - new_h),
                                                ResizeHandle::BottomLeft => (bx1 - new_w, by0),
                                            };
                                            node.transform.matrix = [1.0, 0.0, 0.0, 1.0, tx, ty];
                                        }
                                    } else {
                                        use photonic_core::transform::Transform;
                                        let t_orig = Transform { matrix: orig_xf };
                                        let t_scale =
                                            Transform::scale_around(sx, sy, anchor_x, anchor_y);
                                        // Canvas-space scale composes AFTER the
                                        // node's own transform (else a moved node
                                        // jumps instead of scaling in place).
                                        node.transform = t_scale.then(&t_orig);
                                    }
                                    *doc_modified = true;
                                }
                            }
                        }
                    }
                }
            } else if self.moving {
                // Capture the starting translations, reference point and press
                // position on the first move frame, so the move is applied
                // absolutely (origin + total delta) and can be snapped to grid
                // (#12). Also snapshot the full nodes so the whole drag becomes a
                // single undoable history step on release (#11).
                if self.move_snap_origins.is_empty() {
                    // Alt held at move start: duplicate the selection and drag the
                    // copies, leaving the originals in place.
                    if ui.input(|i| i.modifiers.alt) {
                        let src_ids: Vec<NodeId> = doc.selection.ids().copied().collect();
                        let mut new_ids: Vec<NodeId> = Vec::new();
                        for id in &src_ids {
                            if let Some(mut n) = doc.nodes.get(id).cloned() {
                                n.id = uuid::Uuid::new_v4();
                                let layer = n.layer_id;
                                let nid = n.id;
                                doc.add_node(n, Some(layer));
                                new_ids.push(nid);
                            }
                        }
                        if !new_ids.is_empty() {
                            doc.selection = Selection::from_ids(new_ids.iter().copied());
                            self.selected_id = new_ids.first().copied();
                            self.dup_drag = true;
                            *doc_modified = true;
                        }
                    }

                    let ids_to_move: Vec<NodeId> = doc.selection.ids().copied().collect();
                    self.move_drag_origins = ids_to_move
                        .iter()
                        .filter_map(|id| doc.nodes.get(id).cloned())
                        .collect();
                    self.move_snap_origins = ids_to_move
                        .iter()
                        .filter_map(|id| {
                            doc.nodes
                                .get(id)
                                .map(|n| (*id, n.transform.matrix[4], n.transform.matrix[5]))
                        })
                        .collect();
                    let start_bounds = selection_canvas_bounds(doc, &ids_to_move, renderer);
                    self.move_snap_ref = start_bounds.map(|(x0, y0, _, _)| (x0, y0));
                    self.move_snap_bbox = start_bounds;
                    self.move_snap_press = ui
                        .input(|i| i.pointer.press_origin())
                        .map(|p| view.screen_to_canvas(p.x as f64, p.y as f64));
                }

                if let (Some((px, py)), Some(cur)) =
                    (self.move_snap_press, response.interact_pointer_pos())
                {
                    let (curx, cury) = view.screen_to_canvas(cur.x as f64, cur.y as f64);
                    let raw_dx = curx - px;
                    let raw_dy = cury - py;
                    // Shift: lock the move to the nearest of 8 directions (takes
                    // precedence over grid snap). Otherwise snap the reference
                    // point's target to the grid (no-op when snap is off).
                    let shift = ui.input(|i| i.modifiers.shift);
                    let (mut dx, mut dy) = if shift {
                        axis_lock_8(raw_dx, raw_dy)
                    } else {
                        match self.move_snap_ref {
                            Some((rx, ry)) => {
                                (self.snap(rx + raw_dx) - rx, self.snap(ry + raw_dy) - ry)
                            }
                            None => (raw_dx, raw_dy),
                        }
                    };

                    // Object-aware snapping (#66): refine the grid-snapped delta
                    // so the dragged selection's edges/centers align to nearby
                    // nodes. Additive with grid snap; suppressed while Shift
                    // (axis-lock) is held. Tolerance is in screen px → canvas.
                    self.last_snap_result = None;
                    if self.prefs.snap_to_objects && !shift {
                        if let Some((bx0, by0, bx1, by1)) = self.move_snap_bbox {
                            let moving: Vec<NodeId> = doc.selection.ids().copied().collect();
                            let candidates =
                                crate::snap::collect_snap_candidates(doc, &moving);
                            let tol =
                                (self.prefs.snap_tolerance_px as f64) / view.zoom.max(1e-6);
                            let tentative = (bx0 + dx, by0 + dy, bx1 + dx, by1 + dy);
                            let snap =
                                crate::snap::resolve_snap(tentative, &candidates, tol);
                            dx += snap.corrected.0;
                            dy += snap.corrected.1;
                            if !snap.active.is_empty() {
                                self.last_snap_result = Some(snap);
                            }
                        }
                    }
                    for (id, ox, oy) in &self.move_snap_origins {
                        if let Some(node) = doc.nodes.get_mut(id) {
                            node.transform.matrix[4] = ox + dx;
                            node.transform.matrix[5] = oy + dy;
                            *doc_modified = true;
                        }
                    }
                }
            }
        }

        if response.drag_stopped_by(egui::PointerButton::Primary) {
            self.moving = false;
            // Record the completed move as a single undoable history step (#11).
            // The doc already holds the moved state, so re-applying UpdateNode is
            // a no-op; it just captures the inverse for undo/redo.
            if !self.move_drag_origins.is_empty() {
                if self.dup_drag {
                    // Alt-duplicate: the copies are already live in the doc. Remove
                    // them and re-add through history so the whole duplication is a
                    // single undoable step (undo deletes the copies).
                    let ids: Vec<NodeId> =
                        self.move_drag_origins.iter().map(|n| n.id).collect();
                    self.move_drag_origins.clear();
                    let finals: Vec<SceneNode> =
                        ids.iter().filter_map(|id| doc.nodes.get(id).cloned()).collect();
                    for id in &ids {
                        doc.remove_node(id);
                    }
                    let cmds: Vec<Command> = finals
                        .into_iter()
                        .map(|node| {
                            let layer_id = Some(node.layer_id);
                            Command::AddNode { node, layer_id }
                        })
                        .collect();
                    if !cmds.is_empty() {
                        history.execute(Command::Batch(cmds), doc);
                        *doc_modified = true;
                    }
                } else {
                    let cmds: Vec<Command> = std::mem::take(&mut self.move_drag_origins)
                        .into_iter()
                        .filter_map(|old| {
                            doc.nodes.get(&old.id).and_then(|cur| {
                                (cur.transform.matrix != old.transform.matrix).then(|| {
                                    Command::UpdateNode {
                                        old,
                                        new: cur.clone(),
                                    }
                                })
                            })
                        })
                        .collect();
                    if !cmds.is_empty() {
                        history.execute(Command::Batch(cmds), doc);
                        *doc_modified = true;
                    }
                }
            }
            self.dup_drag = false;
            self.move_snap_origins.clear();
            self.move_snap_ref = None;
            self.move_snap_bbox = None;
            self.last_snap_result = None;
            self.move_snap_press = None;
            self.resizing = None;
            self.resize_origin_bounds = None;
            self.resize_origin_transform = None;
            self.resize_origin_font_size = None;
            self.resize_multi_origins.clear();

            // Record the completed resize as a single undoable history step (#5).
            // The doc already holds the resized state, so re-applying UpdateNode
            // is a no-op; it just captures the inverse for undo/redo.
            if !self.resize_drag_origins.is_empty() {
                let cmds: Vec<Command> = std::mem::take(&mut self.resize_drag_origins)
                    .into_iter()
                    .filter_map(|old| {
                        doc.nodes.get(&old.id).and_then(|cur| {
                            let text_changed = matches!(
                                (&cur.kind, &old.kind),
                                (SceneNodeKind::Text(a), SceneNodeKind::Text(b))
                                    if a.font_size != b.font_size
                            );
                            (cur.transform.matrix != old.transform.matrix || text_changed).then(
                                || Command::UpdateNode {
                                    old,
                                    new: cur.clone(),
                                },
                            )
                        })
                    })
                    .collect();
                if !cmds.is_empty() {
                    history.execute(Command::Batch(cmds), doc);
                    *doc_modified = true;
                }
            }

            // Complete marquee selection if one was in progress
            if let Some(start_pos) = self.marquee_start.take() {
                let end_pos = response
                    .interact_pointer_pos()
                    .or_else(|| ui.input(|i| i.pointer.hover_pos()))
                    .unwrap_or(start_pos);
                let shift = ui.input(|i| i.modifiers.shift);
                let (cx0, cy0) = view.screen_to_canvas(start_pos.x as f64, start_pos.y as f64);
                let (cx1, cy1) = view.screen_to_canvas(end_pos.x as f64, end_pos.y as f64);
                let mx0 = cx0.min(cx1);
                let my0 = cy0.min(cy1);
                let mx1 = cx0.max(cx1);
                let my1 = cy0.max(cy1);

                // Collect nodes whose bounds intersect the marquee rect
                let to_select: Vec<NodeId> = {
                    let nodes = doc.nodes_in_draw_order();
                    let mut ids = Vec::new();
                    for node in nodes {
                        if let Some((nx0, ny0, nx1, ny1)) = text_aware_canvas_bounds(node, renderer)
                        {
                            if nx1 >= mx0 && nx0 <= mx1 && ny1 >= my0 && ny0 <= my1 {
                                ids.push(node.id);
                            }
                        }
                    }
                    ids
                };

                if !shift {
                    doc.selection.clear();
                    self.selected_id = None;
                }
                for id in to_select {
                    doc.selection.add(id);
                    self.selected_id = Some(id);
                }
            }
        }

        // Click on empty space to deselect (without shift)
        if response.clicked_by(egui::PointerButton::Primary) && !self.moving {
            if let Some(pos) = response.interact_pointer_pos() {
                let (cx, cy) = view.screen_to_canvas(pos.x as f64, pos.y as f64);
                let shift = ui.input(|i| i.modifiers.shift);
                let hit = hit_test(doc, cx, cy, renderer);
                if shift {
                    if let Some(id) = hit {
                        doc.selection.toggle(id);
                        self.selected_id = Some(id);
                    }
                } else {
                    self.selected_id = hit;
                    match self.selected_id {
                        Some(id) => doc.selection = Selection::single(id),
                        None => doc.selection.clear(),
                    }
                }
            }
        }

        // ── Selection overlay ────────────────────────────────────────────────
        let accent = Color32::from_rgb(110, 86, 207);
        let thick_stroke = egui::Stroke::new(1.5, accent);
        let sel_ids: Vec<NodeId> = doc.selection.ids().copied().collect();

        if sel_ids.len() > 1 {
            // Multi-select: one unified bounding box with resize handles over the
            // union of all selected nodes (no per-node boxes — they act as a unit).
            if let Some((cx0, cy0, cx1, cy1)) = selection_canvas_bounds(doc, &sel_ids, renderer) {
                let (sx0, sy0) = view.canvas_to_screen(cx0, cy0);
                let (sx1, sy1) = view.canvas_to_screen(cx1, cy1);
                let sel_rect = egui::Rect::from_min_max(
                    egui::pos2(sx0 as f32, sy0 as f32),
                    egui::pos2(sx1 as f32, sy1 as f32),
                );
                ui.painter().rect_stroke(sel_rect, 0.0, thick_stroke);
                for corner in [
                    sel_rect.left_top(),
                    sel_rect.right_top(),
                    sel_rect.left_bottom(),
                    sel_rect.right_bottom(),
                ] {
                    let handle = egui::Rect::from_center_size(corner, egui::Vec2::splat(7.0));
                    ui.painter().rect_filled(handle, 0.0, Color32::WHITE);
                    ui.painter().rect_stroke(handle, 0.0, thick_stroke);
                }
            }
        } else if let Some(sel_id) = self.selected_id {
            // Single-select: outline + resize handles on that node
            if let Some(node) = doc.nodes.get(&sel_id) {
                if let Some((cx0, cy0, cx1, cy1)) = text_aware_canvas_bounds(node, renderer) {
                    let (sx0, sy0) = view.canvas_to_screen(cx0, cy0);
                    let (sx1, sy1) = view.canvas_to_screen(cx1, cy1);
                    let sel_rect = egui::Rect::from_min_max(
                        egui::pos2(sx0 as f32, sy0 as f32),
                        egui::pos2(sx1 as f32, sy1 as f32),
                    );
                    ui.painter().rect_stroke(sel_rect, 0.0, thick_stroke);
                    for corner in [
                        sel_rect.left_top(),
                        sel_rect.right_top(),
                        sel_rect.left_bottom(),
                        sel_rect.right_bottom(),
                    ] {
                        let handle = egui::Rect::from_center_size(corner, egui::Vec2::splat(7.0));
                        ui.painter().rect_filled(handle, 0.0, Color32::WHITE);
                        ui.painter().rect_stroke(handle, 0.0, thick_stroke);
                    }
                }
            }
        }

        // ── Marquee selection overlay ────────────────────────────────────────
        if let Some(start_pos) = self.marquee_start {
            let current_pos = ui.input(|i| i.pointer.hover_pos()).unwrap_or(start_pos);
            let rect = egui::Rect::from_two_pos(start_pos, current_pos);
            let accent = Color32::from_rgb(110, 86, 207);
            ui.painter().rect(
                rect,
                0.0,
                Color32::from_rgba_unmultiplied(110, 86, 207, 30),
                egui::Stroke::new(1.0, accent),
            );
        }

        // ── Cursor icon ──────────────────────────────────────────────────────
        let cursor = if let Some(handle) = self.resizing {
            // Mid-drag: hold the resize cursor
            match handle {
                ResizeHandle::TopLeft | ResizeHandle::BottomRight => egui::CursorIcon::ResizeNwSe,
                ResizeHandle::TopRight | ResizeHandle::BottomLeft => egui::CursorIcon::ResizeNeSw,
            }
        } else if self.moving {
            egui::CursorIcon::Move
        } else if let Some(hover_pos) = ui.input(|i| i.pointer.hover_pos()) {
            // Use effective (combined) bounds for cursor feedback
            const HANDLE_HIT: f32 = 6.0;
            let hover_sel_ids: Vec<NodeId> = doc.selection.ids().copied().collect();
            let hover_bounds = if hover_sel_ids.len() > 1 {
                selection_canvas_bounds(doc, &hover_sel_ids, renderer)
            } else {
                self.selected_id
                    .and_then(|id| doc.nodes.get(&id))
                    .and_then(|n| text_aware_canvas_bounds(n, renderer))
            };

            let corner_hit = hover_bounds.and_then(|(bx0, by0, bx1, by1)| {
                let (sx0, sy0) = view.canvas_to_screen(bx0, by0);
                let (sx1, sy1) = view.canvas_to_screen(bx1, by1);
                let corners = [
                    (egui::pos2(sx0 as f32, sy0 as f32), ResizeHandle::TopLeft),
                    (egui::pos2(sx1 as f32, sy0 as f32), ResizeHandle::TopRight),
                    (egui::pos2(sx0 as f32, sy1 as f32), ResizeHandle::BottomLeft),
                    (
                        egui::pos2(sx1 as f32, sy1 as f32),
                        ResizeHandle::BottomRight,
                    ),
                ];
                corners
                    .iter()
                    .find(|(c, _)| (hover_pos - *c).length() <= HANDLE_HIT)
                    .map(|(_, h)| *h)
            });

            if let Some(handle) = corner_hit {
                match handle {
                    ResizeHandle::TopLeft | ResizeHandle::BottomRight => {
                        egui::CursorIcon::ResizeNwSe
                    }
                    ResizeHandle::TopRight | ResizeHandle::BottomLeft => {
                        egui::CursorIcon::ResizeNeSw
                    }
                }
            } else {
                let on_body = hover_bounds
                    .map(|(bx0, by0, bx1, by1)| {
                        let (sx0, sy0) = view.canvas_to_screen(bx0, by0);
                        let (sx1, sy1) = view.canvas_to_screen(bx1, by1);
                        egui::Rect::from_min_max(
                            egui::pos2(sx0 as f32, sy0 as f32),
                            egui::pos2(sx1 as f32, sy1 as f32),
                        )
                        .contains(hover_pos)
                    })
                    .unwrap_or(false);
                if on_body {
                    egui::CursorIcon::Move
                } else {
                    egui::CursorIcon::Default
                }
            }
        } else {
            egui::CursorIcon::Default
        };
        ui.ctx().set_cursor_icon(cursor);
    }

    // ── Pen tool handler ──────────────────────────────────────────────────────

    pub(crate) fn handle_pen_tool(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        doc: &mut Document,
        view: &CanvasView,
        doc_modified: &mut bool,
    ) {
        // Escape cancels the in-progress path
        if viewport_kb(ui.ctx()) && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.pen_points.clear();
            return;
        }

        // Double-click finalises the path (also fires clicked, so handle first)
        if response.double_clicked_by(egui::PointerButton::Primary) {
            if self.pen_points.len() >= 2 {
                if let Some(path) = self.build_pen_path() {
                    let stroke_arg = self.prefs.default_stroke_enabled.then(|| {
                        (
                            self.prefs.default_stroke_color,
                            self.prefs.default_stroke_width,
                        )
                    });
                    let node = make_node(
                        path,
                        self.fill_color,
                        stroke_arg,
                        "Pen",
                        doc.node_count() + 1,
                    );
                    doc.add_node(node, None);
                    *doc_modified = true;
                }
            }
            self.pen_points.clear();
            return;
        }

        // Single click: add an anchor point
        if response.clicked_by(egui::PointerButton::Primary) {
            if !ui.input(|i| i.modifiers.alt) {
                if let Some(pos) = response.interact_pointer_pos() {
                    let (cx, cy) = view.screen_to_canvas(pos.x as f64, pos.y as f64);
                    self.pen_points.push((cx, cy));
                }
            }
        }

        // ── Preview ──────────────────────────────────────────────────────────
        let painter = ui.painter();
        let path_stroke = egui::Stroke::new(1.5, Color32::from_rgb(110, 86, 207));
        let rubber_stroke =
            egui::Stroke::new(1.0, Color32::from_rgba_unmultiplied(110, 86, 207, 128));

        // Lines between placed points
        for i in 0..self.pen_points.len().saturating_sub(1) {
            let (x0, y0) = self.pen_points[i];
            let (x1, y1) = self.pen_points[i + 1];
            let (sx0, sy0) = view.canvas_to_screen(x0, y0);
            let (sx1, sy1) = view.canvas_to_screen(x1, y1);
            painter.line_segment(
                [
                    egui::pos2(sx0 as f32, sy0 as f32),
                    egui::pos2(sx1 as f32, sy1 as f32),
                ],
                path_stroke,
            );
        }

        // Anchor dots
        for &(cx, cy) in &self.pen_points {
            let (sx, sy) = view.canvas_to_screen(cx, cy);
            let center = egui::pos2(sx as f32, sy as f32);
            painter.rect_filled(
                egui::Rect::from_center_size(center, egui::Vec2::splat(6.0)),
                0.0,
                Color32::WHITE,
            );
            painter.rect_stroke(
                egui::Rect::from_center_size(center, egui::Vec2::splat(6.0)),
                0.0,
                path_stroke,
            );
        }

        // Rubber-band line from last point to cursor
        if let Some(&(lx, ly)) = self.pen_points.last() {
            if let Some(cursor) = ui.input(|i| i.pointer.hover_pos()) {
                let (sx, sy) = view.canvas_to_screen(lx, ly);
                painter.line_segment([egui::pos2(sx as f32, sy as f32), cursor], rubber_stroke);
            }
        }
    }

    /// Build a `PathData` polyline from the accumulated pen points.
    pub(crate) fn build_pen_path(&self) -> Option<PathData> {
        if self.pen_points.len() < 2 {
            return None;
        }
        let mut bez = BezPath::new();
        let (x0, y0) = self.pen_points[0];
        bez.move_to((x0, y0));
        for &(x, y) in &self.pen_points[1..] {
            bez.line_to((x, y));
        }
        Some(PathData::from_bez_path(&bez))
    }

    // ── Direct Selection tool handler ─────────────────────────────────────────

    // (Direct Selection tool handler moved to `mod direct_select` — direct_select.rs)

    // ── Shape Builder tool handler ────────────────────────────────────────────

    pub(crate) fn handle_shape_builder_tool(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        doc: &mut Document,
        view: &CanvasView,
        renderer: &mut PhotonicRenderer,
        doc_modified: &mut bool,
        history: &mut CommandHistory,
    ) {
        let alt_held = ui.input(|i| i.modifiers.alt);

        // Cursor: minus = subtract, crosshair = union
        ui.ctx().set_cursor_icon(if alt_held {
            egui::CursorIcon::NoDrop
        } else {
            egui::CursorIcon::Crosshair
        });

        // Canvas position under pointer
        let canvas_pos = ui
            .input(|i| i.pointer.hover_pos())
            .map(|p| view.screen_to_canvas(p.x as f64, p.y as f64));

        // Update hovered node
        self.shape_builder_hovered =
            canvas_pos.and_then(|(cx, cy)| hit_test(doc, cx, cy, renderer));

        // Drag start: record mode, reset collected set
        if response.drag_started_by(egui::PointerButton::Primary) {
            self.shape_builder_subtract_mode = alt_held;
            self.shape_builder_drag_ids.clear();
            // Add the initial shape under the cursor
            if let Some(id) = self.shape_builder_hovered {
                self.shape_builder_drag_ids.push(id);
            }
        }

        // During drag: accumulate every new shape the cursor enters
        if response.dragged_by(egui::PointerButton::Primary) {
            let pos = response
                .interact_pointer_pos()
                .map(|p| view.screen_to_canvas(p.x as f64, p.y as f64))
                .or(canvas_pos);
            if let Some((cx, cy)) = pos {
                if let Some(id) = hit_test(doc, cx, cy, renderer) {
                    if !self.shape_builder_drag_ids.contains(&id) {
                        self.shape_builder_drag_ids.push(id);
                    }
                }
            }
        }

        // Drag end: perform the boolean operation
        if response.drag_stopped_by(egui::PointerButton::Primary) {
            let ids = std::mem::take(&mut self.shape_builder_drag_ids);
            let subtract = self.shape_builder_subtract_mode;
            if !ids.is_empty() {
                self.execute_shape_builder(doc, history, &ids, subtract, doc_modified);
            }
        }

        // ── Visual feedback ───────────────────────────────────────────────────
        let painter = ui.painter();

        // Highlight shapes being collected in current drag
        for &id in &self.shape_builder_drag_ids {
            if let Some(node) = doc.nodes.get(&id) {
                if let SceneNodeKind::Path(pn) = &node.kind {
                    let baked = gui_apply_affine_to_path(&pn.path_data, node.transform.to_kurbo());
                    let pts = bez_to_screen_points(&baked.to_bez_path(), view);
                    if pts.len() >= 2 {
                        let fill = if self.shape_builder_subtract_mode {
                            Color32::from_rgba_unmultiplied(248, 113, 113, 100)
                        } else {
                            Color32::from_rgba_unmultiplied(52, 211, 153, 100)
                        };
                        painter.add(egui::Shape::Path(egui::epaint::PathShape {
                            points: pts,
                            closed: true,
                            fill,
                            stroke: egui::epaint::PathStroke::new(0.0, Color32::TRANSPARENT),
                        }));
                    }
                }
            }
        }

        // Highlight the hovered shape (if not already in drag set)
        if let Some(hovered_id) = self.shape_builder_hovered {
            if !self.shape_builder_drag_ids.contains(&hovered_id) {
                if let Some(node) = doc.nodes.get(&hovered_id) {
                    if let SceneNodeKind::Path(pn) = &node.kind {
                        let baked =
                            gui_apply_affine_to_path(&pn.path_data, node.transform.to_kurbo());
                        let pts = bez_to_screen_points(&baked.to_bez_path(), view);
                        if pts.len() >= 2 {
                            let (fill_color, stroke_color) = if alt_held {
                                (
                                    Color32::from_rgba_unmultiplied(248, 113, 113, 60),
                                    Color32::from_rgb(248, 113, 113),
                                )
                            } else {
                                (
                                    Color32::from_rgba_unmultiplied(52, 211, 153, 60),
                                    Color32::from_rgb(52, 211, 153),
                                )
                            };
                            painter.add(egui::Shape::Path(egui::epaint::PathShape {
                                points: pts,
                                closed: true,
                                fill: fill_color,
                                stroke: egui::epaint::PathStroke::new(2.0, stroke_color),
                            }));
                        }
                    }
                }
            }
        }
    }

    /// Execute a Shape Builder operation on `ids`.
    ///
    /// - Union mode (`subtract = false`): union all touched shapes into one.
    /// - Subtract mode (`subtract = true`, Alt held): subtract all touched shapes
    ///   (after the first) from the first one; if only one shape is touched, delete it.
    pub(crate) fn execute_shape_builder(
        &mut self,
        doc: &mut Document,
        history: &mut CommandHistory,
        ids: &[NodeId],
        subtract: bool,
        doc_modified: &mut bool,
    ) {
        use photonic_core::ops::boolean::{boolean_op, BooleanOp};

        // Gather (id, layer_id, z-index) for each touched node
        let mut indexed: Vec<(NodeId, photonic_core::layer::LayerId, usize)> = ids
            .iter()
            .filter_map(|&id| doc.node_layer_and_index(&id).map(|(l, i)| (id, l, i)))
            .collect();

        if indexed.is_empty() {
            return;
        }

        // All must be in the same layer
        let layer_id = indexed[0].1;
        if indexed.iter().any(|(_, l, _)| *l != layer_id) {
            return;
        }

        // Sort by ascending z-order
        indexed.sort_by_key(|(_, _, idx)| *idx);

        if subtract && indexed.len() == 1 {
            // Delete single alt-clicked shape
            let node_id = indexed[0].0;
            history.execute(photonic_core::history::Command::RemoveNode { node_id }, doc);
            self.shape_builder_hovered = None;
            *doc_modified = true;
            return;
        }

        if !subtract && indexed.len() < 2 {
            // Nothing to union
            return;
        }

        // Bake transforms for all shapes
        let baked_paths: Vec<_> = indexed
            .iter()
            .filter_map(|(id, _, _)| {
                let n = doc.get_node(id)?;
                if let SceneNodeKind::Path(pn) = &n.kind {
                    Some((
                        *id,
                        gui_apply_affine_to_path(&pn.path_data, n.transform.to_kurbo()),
                    ))
                } else {
                    None
                }
            })
            .collect();

        if baked_paths.is_empty() {
            return;
        }

        // Get style from the bottom-most shape (first in z-order)
        let (fill, stroke) = doc
            .get_node(&indexed[0].0)
            .and_then(|n| {
                if let SceneNodeKind::Path(pn) = &n.kind {
                    Some((pn.fill.clone(), pn.stroke.clone()))
                } else {
                    None
                }
            })
            .unwrap_or_default();

        // Compute result path
        let op = if subtract {
            BooleanOp::Subtract
        } else {
            BooleanOp::Union
        };
        let mut result_path = baked_paths[0].1.clone();
        for (_, path) in &baked_paths[1..] {
            match boolean_op(&result_path, path, op) {
                Ok(p) => result_path = p,
                Err(_) => return,
            }
        }

        // Build result node inheriting the first shape's style
        let mut result_pn = photonic_core::node::PathNode::new(result_path);
        result_pn.fill = fill;
        result_pn.stroke = stroke;
        let result_node = SceneNode::new("Shape", layer_id, SceneNodeKind::Path(result_pn));
        let result_id = result_node.id;

        // Place the result at the z-position of the lowest input shape
        let insert_z = indexed[0].2;
        let layer_len = doc
            .layers
            .get(&layer_id)
            .map(|l| l.node_ids.len())
            .unwrap_or(0);
        let result_pos = layer_len.saturating_sub(indexed.len()); // position after removes + add
        let new_index = insert_z.min(result_pos);

        let mut cmds: Vec<photonic_core::history::Command> = indexed
            .iter()
            .map(|(id, _, _)| photonic_core::history::Command::RemoveNode { node_id: *id })
            .collect();
        cmds.push(photonic_core::history::Command::AddNode {
            node: result_node,
            layer_id: Some(layer_id),
        });
        if new_index != result_pos {
            cmds.push(photonic_core::history::Command::ReorderNode {
                layer_id,
                node_id: result_id,
                old_index: result_pos,
                new_index,
            });
        }

        history.execute(photonic_core::history::Command::Batch(cmds), doc);
        self.selected_id = Some(result_id);
        doc.selection = Selection::single(result_id);
        *doc_modified = true;
    }

    // ── Console panel ─────────────────────────────────────────────────────────

    pub(crate) fn draw_console(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui
                .selectable_label(self.lua_console.tab == ConsoleTab::Lua, "Lua")
                .clicked()
            {
                self.lua_console.tab = ConsoleTab::Lua;
            }
            if ui
                .selectable_label(self.lua_console.tab == ConsoleTab::Claude, "Claude")
                .clicked()
            {
                self.lua_console.tab = ConsoleTab::Claude;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button(ph::X).clicked() {
                    self.lua_console.visible = false;
                }
                let expand_icon = if self.lua_console.expanded { ph::CARET_DOWN } else { ph::CARET_UP };
                if ui
                    .small_button(expand_icon)
                    .on_hover_text(if self.lua_console.expanded {
                        "Collapse"
                    } else {
                        "Expand"
                    })
                    .clicked()
                {
                    self.lua_console.expanded = !self.lua_console.expanded;
                }
                if ui.small_button("Clear").clicked() {
                    self.lua_console.log.clear();
                }
                if self.lua_console.tab == ConsoleTab::Claude {
                    if ui
                        .small_button("Copy")
                        .on_hover_text("Copy conversation to clipboard")
                        .clicked()
                    {
                        let mut text = String::new();
                        for (is_user, msg) in &self.claude_chat.messages {
                            let role = if *is_user { "You" } else { "Claude" };
                            text.push_str(role);
                            text.push_str(": ");
                            text.push_str(msg);
                            text.push_str("\n\n");
                        }
                        ui.output_mut(|o| o.copied_text = text);
                    }
                }
            });
        });
        ui.separator();

        match self.lua_console.tab {
            ConsoleTab::Lua => self.draw_lua_tab(ui),
            ConsoleTab::Claude => self.draw_claude_tab(ui),
        }
    }

    pub(crate) fn draw_lua_tab(&mut self, ui: &mut egui::Ui) {
        // Output scroll area
        let available = ui.available_height() - 32.0;
        egui::ScrollArea::vertical()
            .id_salt("console_out")
            .max_height(available.max(40.0))
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for (is_err, line) in &self.lua_console.log {
                    let color = if *is_err {
                        Color32::from_rgb(248, 113, 113)
                    } else {
                        Color32::from_rgb(187, 187, 210)
                    };
                    ui.label(egui::RichText::new(line).monospace().color(color));
                }
            });

        ui.separator();

        // Input row
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(">")
                    .monospace()
                    .color(Color32::from_rgb(144, 119, 224)),
            );
            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.lua_console.input)
                    .font(egui::TextStyle::Monospace)
                    .desired_width(ui.available_width() - 50.0)
                    .hint_text("photonic.create_rect(100, 100, 200, 150)"),
            );
            let submitted = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if ui.button("Run").clicked() || submitted {
                if !self.lua_console.input.trim().is_empty() {
                    let code = self.lua_console.input.clone();
                    self.lua_console.log.push((false, format!("> {code}")));
                    self.lua_console.pending = Some(code);
                    self.lua_console.input.clear();
                }
                resp.request_focus();
            }
        });
    }

    // ── Shape factory ─────────────────────────────────────────────────────────

    pub(crate) fn build_shape(&self, sx: f64, sy: f64, ex: f64, ey: f64) -> Option<PathData> {
        let min_x = sx.min(ex);
        let min_y = sy.min(ey);
        let max_x = sx.max(ex);
        let max_y = sy.max(ey);
        let w = max_x - min_x;
        let h = max_y - min_y;
        let cx = (min_x + max_x) / 2.0;
        let cy = (min_y + max_y) / 2.0;
        let radius = ((ex - sx).hypot(ey - sy)) / 2.0;

        let path = match self.active_tool {
            Tool::Rectangle => PathData::rect(min_x, min_y, w, h),
            Tool::Ellipse => PathData::ellipse(cx, cy, w / 2.0, h / 2.0),
            Tool::Polygon => PathData::regular_polygon(cx, cy, radius, self.polygon_sides as usize),
            Tool::Star => PathData::star(
                cx,
                cy,
                radius,
                radius * self.star_inner_ratio as f64,
                self.star_points as usize,
            ),
            Tool::Spiral => PathData::spiral(
                cx,
                cy,
                radius,
                (self.spiral_inner_radius as f64).min(radius),
                self.spiral_turns as f64,
                self.spiral_segs_per_turn as usize,
            ),
            // Line uses the raw drag start/end (not a bounding box).
            Tool::Line => PathData::line(sx, sy, ex, ey),
            Tool::Arc => PathData::arc(
                cx,
                cy,
                w / 2.0,
                h / 2.0,
                self.arc_start_angle,
                self.arc_end_angle,
                !self.arc_open,
            ),
            Tool::Grid => PathData::grid(min_x, min_y, w, h, self.grid_cols, self.grid_rows),
            Tool::PolarGrid => {
                let outer_r = (w.min(h)) / 2.0;
                let inner_r = outer_r * self.polar_grid_inner_ratio as f64;
                PathData::polar_grid(
                    cx,
                    cy,
                    outer_r,
                    inner_r,
                    self.polar_grid_rings,
                    self.polar_grid_sectors,
                )
            }
            _ => return None,
        };

        Some(path)
    }

    /// Like `build_shape` but takes an explicit `Tool` instead of reading `self.active_tool`.
    /// Used by `CreateShapeAtPos` so active tool state is not polluted.
    pub(crate) fn build_shape_with_tool(
        &self,
        tool: Tool,
        sx: f64,
        sy: f64,
        ex: f64,
        ey: f64,
    ) -> Option<PathData> {
        let min_x = sx.min(ex);
        let min_y = sy.min(ey);
        let max_x = sx.max(ex);
        let max_y = sy.max(ey);
        let w = max_x - min_x;
        let h = max_y - min_y;
        let cx = (min_x + max_x) / 2.0;
        let cy = (min_y + max_y) / 2.0;
        let radius = ((ex - sx).hypot(ey - sy)) / 2.0;

        let path = match tool {
            Tool::Rectangle => PathData::rect(min_x, min_y, w, h),
            Tool::RoundedRect => {
                PathData::rounded_rect(min_x, min_y, w, h, self.rounded_rect_radius)
            }
            Tool::Ellipse => PathData::ellipse(cx, cy, w / 2.0, h / 2.0),
            Tool::Polygon => PathData::regular_polygon(cx, cy, radius, self.polygon_sides as usize),
            Tool::Star => PathData::star(
                cx,
                cy,
                radius,
                radius * self.star_inner_ratio as f64,
                self.star_points as usize,
            ),
            Tool::Spiral => PathData::spiral(
                cx,
                cy,
                radius,
                (self.spiral_inner_radius as f64).min(radius),
                self.spiral_turns as f64,
                self.spiral_segs_per_turn as usize,
            ),
            Tool::Line => PathData::line(sx, sy, ex, ey),
            Tool::Arc => PathData::arc(
                cx,
                cy,
                w / 2.0,
                h / 2.0,
                self.arc_start_angle,
                self.arc_end_angle,
                !self.arc_open,
            ),
            Tool::Grid => PathData::grid(min_x, min_y, w, h, self.grid_cols, self.grid_rows),
            Tool::PolarGrid => {
                let outer_r = (w.min(h)) / 2.0;
                let inner_r = outer_r * self.polar_grid_inner_ratio as f64;
                PathData::polar_grid(
                    cx,
                    cy,
                    outer_r,
                    inner_r,
                    self.polar_grid_rings,
                    self.polar_grid_sectors,
                )
            }
            _ => return None,
        };

        Some(path)
    }
}
