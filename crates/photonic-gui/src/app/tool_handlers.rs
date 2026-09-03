//! Interactive tool handlers (Select, Pen, Shape Builder) and shape-builder
//! geometry, extracted from app::mod. Methods on PhotonicApp.
#![allow(clippy::too_many_arguments)]
use super::*;

/// Registry-backed commands handled in every viewport tool. Keeping this list
/// explicit allows the audit test to catch shortcut entries that have a label
/// and default binding but no runtime dispatch path.
const GLOBAL_SHORTCUT_COMMANDS: &[&str] = &[
    "edit.undo",
    "edit.redo",
    "edit.duplicate",
    "selection.select_all",
    "selection.deselect",
    "object.group",
    "object.ungroup",
    "object.bring_forward",
    "object.send_backward",
    "object.bring_to_front",
    "object.send_to_back",
    "object.flip_horizontal",
    "object.flip_vertical",
    "view.outline_mode",
    "view.pixel_preview",
    "view.overprint_preview",
    "view.toggle_guides",
];

/// Build the in-progress Pen geometry. `curvature_mode` chooses between
/// explicit anchor handles and a Catmull-Rom interpolation through positions.
fn build_pen_bez(anchors: &[PenAnchor], curvature_mode: bool, close: bool) -> Option<BezPath> {
    if anchors.len() < 2 {
        return None;
    }
    if curvature_mode {
        Some(curvature_pen_bez(anchors, close && anchors.len() >= 3))
    } else {
        Some(handled_pen_bez(anchors, close && anchors.len() >= 3))
    }
}

/// Emit cubic segments using the outgoing handle of the previous anchor and
/// incoming handle of the current anchor. With neither side pulled, preserve a
/// true straight `LineTo` rather than a degenerate cubic.
fn handled_pen_bez(anchors: &[PenAnchor], close: bool) -> BezPath {
    let mut bez = BezPath::new();
    bez.move_to(anchors[0].position);
    for pair in anchors.windows(2) {
        emit_pen_edge(&mut bez, pair[0], pair[1]);
    }
    if close {
        let last = anchors[anchors.len() - 1];
        let first = anchors[0];
        if last.out_handle.is_some() || first.in_handle.is_some() {
            bez.curve_to(
                last.out_handle.unwrap_or(last.position),
                first.in_handle.unwrap_or(first.position),
                first.position,
            );
        }
        bez.close_path();
    }
    bez
}

fn emit_pen_edge(bez: &mut BezPath, from: PenAnchor, to: PenAnchor) {
    match (from.out_handle, to.in_handle) {
        (None, None) => bez.line_to(to.position),
        (out_handle, in_handle) => bez.curve_to(
            out_handle.unwrap_or(from.position),
            in_handle.unwrap_or(to.position),
            to.position,
        ),
    }
}

/// Uniform Catmull-Rom interpolation converted to cubic Béziers. Every input
/// position is emitted as a segment endpoint, so the resulting path passes
/// through — rather than merely approaches — each placed anchor.
fn curvature_pen_bez(anchors: &[PenAnchor], closed: bool) -> BezPath {
    let points: Vec<Point> = anchors.iter().map(|anchor| anchor.position).collect();
    let n = points.len();
    let mut bez = BezPath::new();
    bez.move_to(points[0]);

    if n == 2 {
        bez.line_to(points[1]);
        return bez;
    }

    let point_at = |i: isize| -> Point {
        if closed {
            points[((i % n as isize) + n as isize) as usize % n]
        } else if i < 0 {
            Point::new(
                2.0 * points[0].x - points[1].x,
                2.0 * points[0].y - points[1].y,
            )
        } else if i >= n as isize {
            Point::new(
                2.0 * points[n - 1].x - points[n - 2].x,
                2.0 * points[n - 1].y - points[n - 2].y,
            )
        } else {
            points[i as usize]
        }
    };

    let segment_count = if closed { n } else { n - 1 };
    for i in 0..segment_count {
        let p0 = point_at(i as isize - 1);
        let p1 = point_at(i as isize);
        let p2 = point_at(i as isize + 1);
        let p3 = point_at(i as isize + 2);
        let c1 = Point::new(p1.x + (p2.x - p0.x) / 6.0, p1.y + (p2.y - p0.y) / 6.0);
        let c2 = Point::new(p2.x - (p3.x - p1.x) / 6.0, p2.y - (p3.y - p1.y) / 6.0);
        bez.curve_to(c1, c2, p2);
    }
    if closed {
        bez.close_path();
    }
    bez
}

/// Paint a kurbo path in canvas coordinates into egui screen coordinates.
/// Flattening is only for the transient overlay; committed geometry remains
/// cubic, and the tolerance tracks zoom so handles stay visually accurate.
fn paint_pen_bez(painter: &egui::Painter, view: &CanvasView, bez: &BezPath, stroke: egui::Stroke) {
    let mut subpaths: Vec<Vec<egui::Pos2>> = Vec::new();
    let tolerance = (0.35 / view.zoom.max(0.01)).clamp(0.01, 2.0);
    kurbo::flatten(
        bez.elements().iter().copied(),
        tolerance,
        |element| match element {
            PathEl::MoveTo(point) => {
                let (sx, sy) = view.canvas_to_screen(point.x, point.y);
                subpaths.push(vec![egui::pos2(sx as f32, sy as f32)]);
            }
            PathEl::LineTo(point) => {
                let (sx, sy) = view.canvas_to_screen(point.x, point.y);
                if let Some(subpath) = subpaths.last_mut() {
                    subpath.push(egui::pos2(sx as f32, sy as f32));
                }
            }
            PathEl::ClosePath => {
                if let Some(subpath) = subpaths.last_mut() {
                    if let Some(first) = subpath.first().copied() {
                        subpath.push(first);
                    }
                }
            }
            PathEl::QuadTo(..) | PathEl::CurveTo(..) => {}
        },
    );
    for points in subpaths.into_iter().filter(|points| points.len() >= 2) {
        painter.add(egui::Shape::line(points, stroke));
    }
}

impl PhotonicApp {
    /// Finalize a completed object-move drag by recording it as a single,
    /// **discrete** undoable History step (#11 / #183).
    ///
    /// Called on drag release from both the normal `drag_stopped_by(Primary)`
    /// path and the #183 fallback (for when a competing overlay swallowed the
    /// canvas response so `drag_stopped_by` never fired). The completed move is
    /// pushed through [`CommandHistory::execute_discrete`] rather than
    /// `execute`, so it is guaranteed to land as its own undo entry regardless
    /// of any coalescing gesture (#182) that is still open on the shared history
    /// — Ctrl+Z and the History timeline therefore always see exactly one step
    /// per move.
    ///
    /// Idempotent: once `move_drag_origins` has been consumed this only clears
    /// the transient drag/snap state, so calling it from either release path is
    /// safe — whichever fires first records the move exactly once.
    pub(crate) fn finalize_move(
        &mut self,
        doc: &mut Document,
        history: &mut CommandHistory,
        doc_modified: &mut bool,
    ) {
        if !self.move_drag_origins.is_empty() {
            if self.dup_drag {
                // Alt-duplicate: the copies are already live in the doc. Remove
                // them and re-add through history so the whole duplication is a
                // single undoable step (undo deletes the copies).
                let ids: Vec<NodeId> = self.move_drag_origins.iter().map(|n| n.id).collect();
                self.move_drag_origins.clear();
                let finals: Vec<SceneNode> = ids
                    .iter()
                    .filter_map(|id| doc.nodes.get(id).cloned())
                    .collect();
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
                    history.execute_discrete(Command::Batch(cmds), doc);
                    *doc_modified = true;
                }
            } else {
                // The doc already holds the moved state, so re-applying
                // UpdateNode is a no-op; it just captures the inverse for
                // undo/redo. Only nodes whose transform actually changed are
                // recorded.
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
                    history.execute_discrete(Command::Batch(cmds), doc);
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
    }

    /// Tool-independent keyboard shortcuts that must fire regardless of which
    /// tool is active (#192). Extracted from [`Self::handle_select_tool`] so
    /// undo/redo, copy/paste, duplicate, select-all/deselect, flip H/V,
    /// group/ungroup, z-order and the view-preview/guide toggles work while
    /// Scissors, Pen, Knife, Eraser, MagicWand, Lasso, Pencil, Text, Direct
    /// Select (any non-Select tool) is active — previously these were dead
    /// unless the Select tool happened to be current.
    ///
    /// Dispatched unconditionally from the frame loop before per-tool handling.
    /// Guarded by `viewport_kb` so typing into a focused text widget is never
    /// intercepted. Returns whether the document was modified this frame.
    pub(crate) fn handle_global_shortcuts(
        &mut self,
        ctx: &egui::Context,
        doc: &mut Document,
        history: &mut CommandHistory,
    ) -> bool {
        let mut doc_modified = false;

        // A layer can have been locked from the sidebar since the last canvas
        // frame. Prune its contents before shortcuts such as Delete, Duplicate,
        // Group, and Z-order inspect the current selection.
        self.prune_locked_selection(doc);

        // The native host queues image/SVG clipboard data because egui's
        // clipboard event can only carry text. Consume it even when a text
        // widget has focus so a paste intended for that widget cannot leak
        // into the canvas on a later frame.
        let pending_native_paste = self.pending_native_clipboard_paste.take();

        // Skip entirely when a text widget has focus so typing is unaffected.
        if !viewport_kb(ctx) {
            return doc_modified;
        }

        // Copy / paste are driven by egui's clipboard **events** plus the
        // native host queue for non-text formats. egui-winit intercepts
        // Ctrl+C / Ctrl+V (and Ctrl+Shift+V) and emits `Event::Copy` /
        // `Event::Paste`, swallowing the underlying key event — so
        // `key_pressed(Key::C / Key::V)` never fires for them. `Event::Paste`
        // does not carry modifier state, so read `shift` from the same frame.
        let remapped_copy =
            self.binding_is_remapped("edit.copy") && self.binding_pressed(ctx, "edit.copy");
        let remapped_paste =
            self.binding_is_remapped("edit.paste") && self.binding_pressed(ctx, "edit.paste");
        let remapped_paste_in_place = self.binding_is_remapped("edit.paste_in_place")
            && self.binding_pressed(ctx, "edit.paste_in_place");
        let (native_copy, paste_text, paste_in_place) = ctx.input(|i| {
            (
                i.events.iter().any(|e| matches!(e, egui::Event::Copy)),
                i.events.iter().find_map(|e| match e {
                    egui::Event::Paste(text) => Some(text.clone()),
                    _ => None,
                }),
                i.modifiers.shift,
            )
        });
        let want_copy = native_copy || remapped_copy;

        // Ctrl+C: copy the selected objects (each as a full subtree) to the
        // in-process clipboard, so a group of paths/images pastes intact and
        // survives switching to another open document.
        if want_copy {
            let ids: Vec<NodeId> = doc.selection.ids().copied().collect();
            if !ids.is_empty() {
                self.gui_clipboard.capture(doc, ids.iter());
                // Keep the OS clipboard non-empty so future Ctrl+V emits a paste
                // event (see note above). The text is an internal marker only.
                ctx.copy_text(INTERNAL_OBJECT_CLIPBOARD_MARKER.to_string());
            }
        }

        // Ctrl+V: paste with +10px offset. Ctrl+Shift+V: paste in place. A
        // native payload takes precedence over egui's text fallback so an SVG
        // remains editable and a clipboard image is not reduced to text.
        let pasted = if let Some((payload, queued_in_place)) = pending_native_paste {
            self.paste_native_clipboard(doc, history, payload, queued_in_place)
        } else if let Some(text) = paste_text {
            self.paste_native_clipboard(
                doc,
                history,
                NativeClipboardPaste::Text(text),
                paste_in_place,
            )
        } else if remapped_paste_in_place {
            self.dispatch_command("edit.paste_in_place", doc, history)
        } else if remapped_paste {
            self.dispatch_command("edit.paste", doc, history)
        } else {
            false
        };
        if pasted {
            doc_modified = true;
        }

        // Every non-clipboard global action is resolved through the keymap and
        // the same dispatcher used by the command palette.
        for &id in GLOBAL_SHORTCUT_COMMANDS {
            if self.binding_pressed(ctx, id) && self.dispatch_command(id, doc, history) {
                doc_modified = true;
            }
        }

        // Delete is global except in tools where it removes a sub-object (an
        // anchor or width sample) rather than the selected scene node.
        if !matches!(self.active_tool, Tool::DirectSelect | Tool::Width)
            && self.binding_pressed(ctx, "edit.delete")
            && self.dispatch_command("edit.delete", doc, history)
        {
            doc_modified = true;
        }

        doc_modified
    }

    /// The node ids a Select-tool click on `hit_id` should act on: every member
    /// of `hit_id`'s outermost group (Illustrator-style group selection), or
    /// just `hit_id` itself when `alt` is held, when editing inside an isolated
    /// group, or when `hit_id` isn't in a group at all. Never returns an empty
    /// vec (falls back to `[hit_id]`).
    pub(crate) fn select_group_members(
        &self,
        doc: &Document,
        hit_id: NodeId,
        alt: bool,
    ) -> Vec<NodeId> {
        if alt || self.isolated_group.is_some() {
            return vec![hit_id];
        }
        match doc.outermost_group_of(&hit_id) {
            Some(gid) => {
                let members: Vec<NodeId> = doc
                    .group_member_ids(&gid)
                    .into_iter()
                    .filter(|id| {
                        doc.nodes
                            .get(id)
                            .is_some_and(|node| !doc.is_node_locked(node))
                    })
                    .collect();
                if members.is_empty() {
                    vec![hit_id]
                } else {
                    members
                }
            }
            None => vec![hit_id],
        }
    }

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
        self.prune_locked_selection(doc);

        // ── Keyboard shortcuts (skipped when a text widget has focus) ─────────
        // Tool-independent shortcuts (undo/redo, copy/paste, duplicate,
        // select-all/deselect, flip, group/ungroup, z-order, view toggles) live
        // in `handle_global_shortcuts`, dispatched unconditionally from the
        // frame loop (#192). Only Delete/Backspace of the live Select-tool
        // selection remains here — it acts on the Select tool's selection UI and
        // must short-circuit the rest of this handler.
        if viewport_kb(ui.ctx()) && self.selected_id.is_some() {
            // Delete / Backspace: remove all selected nodes as one undoable
            // history step so Ctrl+Z restores them (#191). `execute` hydrates
            // each bare RemoveNode into RemoveNodeFull, so undo re-adds every
            // node into its original layer.
            let delete = ui
                .input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace));
            if delete {
                let ids_to_delete: Vec<NodeId> = doc.selection.ids().copied().collect();
                if !ids_to_delete.is_empty() {
                    let cmds: Vec<Command> = ids_to_delete
                        .iter()
                        .map(|&node_id| Command::RemoveNode { node_id })
                        .collect();
                    history.execute(Command::Batch(cmds), doc);
                    doc.selection.clear();
                    self.selected_id = None;
                    *doc_modified = true;
                }
                return;
            }
        }

        // ── Isolation Mode: Escape exits ─────────────────────────────────────
        if self.isolated_group.is_some() {
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.isolated_group = None;
                doc.selection.clear();
                self.selected_id = None;
            }
        }

        // ── Double-click: enter Isolation Mode on a group ─────────────────────
        // `hit_test` returns the LEAF under the cursor (groups are flattened in
        // draw order), so resolve that leaf up to the group it belongs to. Each
        // double-click drills one level deeper (Illustrator-style): first into
        // the outermost group, then into nested subgroups.
        if response.double_clicked_by(egui::PointerButton::Primary) {
            if let Some(pos) = response.interact_pointer_pos() {
                let (cx, cy) = view.screen_to_canvas(pos.x as f64, pos.y as f64);
                match hit_test(doc, cx, cy, renderer) {
                    Some(leaf) => {
                        // Ancestor group chain, outermost first.
                        let mut chain: Vec<NodeId> = Vec::new();
                        let mut cur = doc.parent_group_of(&leaf);
                        while let Some(g) = cur {
                            chain.push(g);
                            if chain.len() > 64 {
                                break;
                            }
                            cur = doc.parent_group_of(&g);
                        }
                        chain.reverse();

                        if chain.is_empty() {
                            // Ungrouped object: leave isolation (if any).
                            if self.isolated_group.take().is_some() {
                                doc.selection.clear();
                                self.selected_id = None;
                                *doc_modified = true;
                            }
                            return;
                        }

                        // Drill one level past the current isolation.
                        let target = match self.isolated_group {
                            None => chain[0],
                            Some(iso) => match chain.iter().position(|g| *g == iso) {
                                Some(i) if i + 1 < chain.len() => chain[i + 1],
                                Some(_) => iso,   // already deepest for this leaf
                                None => chain[0], // isolated elsewhere → reset
                            },
                        };

                        self.isolated_group = Some(target);
                        if let Some(SceneNodeKind::Group(g)) =
                            doc.nodes.get(&target).map(|n| &n.kind)
                        {
                            let children = g.children.clone();
                            doc.selection.clear();
                            for cid in &children {
                                doc.selection.add(*cid);
                            }
                            self.selected_id = children.first().copied();
                        }
                        *doc_modified = true;
                        return;
                    }
                    None => {
                        // Double-click on empty canvas: exit isolation if active.
                        if self.isolated_group.take().is_some() {
                            doc.selection.clear();
                            self.selected_id = None;
                            *doc_modified = true;
                        }
                    }
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
                // Thickness (screen px) of the rotate frame just outside the
                // selection box: pressing anywhere in this band rotates.
                const ROTATE_ZONE: f32 = 44.0;
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

                // Rotation zone: just OUTSIDE a corner handle (not on the body,
                // not on a resize handle) → rotate in place about the selection
                // centre, mirroring the Illustrator/Photoshop corner-rotate cursor.
                let rotate_pivot = if resize_hit.is_some() {
                    None
                } else {
                    effective_bounds.and_then(|(bx0, by0, bx1, by1)| {
                        let (sx0, sy0) = view.canvas_to_screen(bx0, by0);
                        let (sx1, sy1) = view.canvas_to_screen(bx1, by1);
                        let rect = egui::Rect::from_two_pos(
                            egui::pos2(sx0 as f32, sy0 as f32),
                            egui::pos2(sx1 as f32, sy1 as f32),
                        );
                        // A thick frame hugging the selection box: inside the
                        // inflated rect but outside the box itself.
                        let outer = rect.expand(ROTATE_ZONE);
                        (outer.contains(pos) && !rect.contains(pos))
                            .then_some(((bx0 + bx1) / 2.0, (by0 + by1) / 2.0))
                    })
                };

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
                } else if let Some(pivot) = rotate_pivot {
                    // Start a rotate-in-place drag about the selection centre.
                    self.rotating = true;
                    self.rotate_pivot = pivot;
                    self.rotate_start_angle = (cy - pivot.1).atan2(cx - pivot.0);
                    let ids: Vec<NodeId> = if sel_ids.len() > 1 {
                        sel_ids.clone()
                    } else {
                        self.selected_id.into_iter().collect()
                    };
                    self.rotate_origins = ids
                        .iter()
                        .filter_map(|id| doc.nodes.get(id).map(|n| (*id, n.transform.matrix)))
                        .collect();
                    // Reuse the resize snapshot vec so the release path records the
                    // rotation as one undoable UpdateNode batch.
                    self.resize_drag_origins = ids
                        .iter()
                        .filter_map(|id| doc.nodes.get(id).cloned())
                        .collect();
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
                        let alt = ui.input(|i| i.modifiers.alt);
                        // Group selection: clicking any member of a group acts
                        // on every object in its outermost group (Illustrator
                        // behavior). Alt+click bypasses this to grab just the
                        // clicked member; isolation mode is already editing
                        // inside one group, so no expansion there either. See
                        // `select_group_members`.
                        if shift {
                            if let Some(id) = hit {
                                // Toggle the whole group as a unit: any member
                                // already selected → deselect all of them,
                                // otherwise add all.
                                let members = self.select_group_members(doc, id, alt);
                                if members.iter().any(|m| doc.selection.contains(m)) {
                                    for m in &members {
                                        doc.selection.remove(m);
                                    }
                                    self.selected_id = doc.selection.ids().next().copied();
                                } else {
                                    for m in &members {
                                        doc.selection.add(*m);
                                    }
                                    self.selected_id = Some(id);
                                }
                            } else {
                                // Shift+drag on empty space → additive marquee
                                self.marquee_start = Some(pos);
                            }
                        } else {
                            match hit {
                                Some(id) => {
                                    let members = self.select_group_members(doc, id, alt);
                                    doc.selection = Selection::from_ids(members.iter().copied());
                                    self.selected_id = Some(id);
                                    self.moving = !alt;
                                }
                                None => {
                                    self.selected_id = None;
                                    self.moving = false;
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
            if self.rotating {
                if let Some(pos) = response.interact_pointer_pos() {
                    use photonic_core::transform::Transform;
                    let (cx, cy) = view.screen_to_canvas(pos.x as f64, pos.y as f64);
                    let (pxc, pyc) = self.rotate_pivot;
                    let ang = (cy - pyc).atan2(cx - pxc);
                    let mut delta = ang - self.rotate_start_angle;
                    // Shift snaps rotation to 15° increments.
                    if ui.input(|i| i.modifiers.shift) {
                        let step = std::f64::consts::PI / 12.0;
                        delta = (delta / step).round() * step;
                    }
                    let rot = Transform::rotate_around(delta, pxc, pyc);
                    let origins = self.rotate_origins.clone();
                    for (id, orig) in &origins {
                        if let Some(node) = doc.nodes.get_mut(id) {
                            node.transform = rot.then(&Transform { matrix: *orig });
                            *doc_modified = true;
                        }
                    }
                }
            } else if self.resizing.is_some() {
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
                        let mut cmds: Vec<Command> = Vec::new();
                        for id in &src_ids {
                            if let Some(mut n) = doc.nodes.get(id).cloned() {
                                n.id = uuid::Uuid::new_v4();
                                let layer = n.layer_id;
                                new_ids.push(n.id);
                                // Route the duplicate through history so it is
                                // undoable (was a direct doc.add_node bypass).
                                cmds.push(Command::AddNode {
                                    node: n,
                                    layer_id: Some(layer),
                                });
                            }
                        }
                        if !cmds.is_empty() {
                            history.execute_discrete(Command::Batch(cmds), doc);
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
                    if (self.prefs.snap_to_objects
                        || self.prefs.snap_to_artboard
                        || self.prefs.snap_to_anchors)
                        && !shift
                    {
                        if let Some((bx0, by0, bx1, by1)) = self.move_snap_bbox {
                            let moving: Vec<NodeId> = doc.selection.ids().copied().collect();
                            let mut candidates = if self.prefs.snap_to_objects {
                                crate::snap::collect_snap_candidates(doc, &moving)
                            } else {
                                Vec::new()
                            };
                            // Artboard/canvas edges + margins (#211).
                            if self.prefs.snap_to_artboard {
                                candidates.extend(crate::snap::collect_artboard_candidates(doc));
                            }
                            // Path anchor points (#211).
                            if self.prefs.snap_to_anchors {
                                candidates
                                    .extend(crate::snap::collect_anchor_candidates(doc, &moving));
                            }
                            let tol = (self.prefs.snap_tolerance_px as f64) / view.zoom.max(1e-6);
                            let tentative = (bx0 + dx, by0 + dy, bx1 + dx, by1 + dy);
                            let mut snap = crate::snap::resolve_snap(tentative, &candidates, tol);
                            dx += snap.corrected.0;
                            dy += snap.corrected.1;
                            // Equal-spacing distribution hints (#66) — only between
                            // objects, and only on an axis edge snapping didn't claim.
                            let others = if self.prefs.snap_to_objects {
                                crate::snap::collect_node_aabbs(doc, &moving)
                            } else {
                                Vec::new()
                            };
                            let post = (bx0 + dx, by0 + dy, bx1 + dx, by1 + dy);
                            let sp = crate::snap::resolve_equal_spacing(post, &others, tol);
                            if snap.corrected.0 == 0.0 {
                                if let Some(d) = sp.dx {
                                    dx += d;
                                    if let Some(h) = sp.hint_x {
                                        snap.spacing.push(h);
                                    }
                                }
                            }
                            if snap.corrected.1 == 0.0 {
                                if let Some(d) = sp.dy {
                                    dy += d;
                                    if let Some(h) = sp.hint_y {
                                        snap.spacing.push(h);
                                    }
                                }
                            }
                            if !snap.active.is_empty() || !snap.spacing.is_empty() {
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
            let move_pending = !self.move_drag_origins.is_empty();
            let was_moving = self.moving;
            self.moving = false;
            // Record the completed move as a single, discrete undoable history
            // step (#11 / #183). See `finalize_move`.
            //
            // Instrumentation (#183 root-cause A2 vs A1, see proposal): log which
            // release branch actually recovered the move so the A2 hypothesis can
            // be confirmed live. If we were in move mode but NO origins were
            // captured, that is the A1 signature (origin capture / hit-test never
            // ran) — the A2 fallback cannot help and a separate fix is required.
            if move_pending {
                tracing::debug!(
                    target: "photonic::move",
                    nodes = self.move_drag_origins.len(),
                    "#183 move recorded via drag_stopped_by(Primary) path"
                );
            } else if was_moving {
                tracing::warn!(
                    target: "photonic::move",
                    "#183 root-cause A1: drag stopped in move mode but no origins were captured \
                     (origin capture / hit-test never ran) — the A2 release fallback cannot recover \
                     this move; a hit-test / origin-capture fix is needed"
                );
            }
            self.finalize_move(doc, history, doc_modified);
            self.rotating = false;
            self.rotate_origins.clear();
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
                        if doc.is_node_locked(node) {
                            continue;
                        }
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
        // Fallback move recorder (#183). A competing overlay allocated later in
        // the frame — the artboard drag handle / name hit-target
        // (`app/mod.rs`), or a full-canvas modal scrim — can consume the canvas
        // `response`, so `drag_stopped_by(Primary)` never fires on it and the
        // move above is never recorded (the regression of #11). If a move is
        // still pending but the primary button is no longer held (and we are not
        // mid-drag), finalize it here so a move always lands as exactly one
        // undoable History step, undoable with Ctrl+Z and visible in the
        // timeline. Idempotent with the `drag_stopped_by` path: whichever fires
        // first consumes `move_drag_origins`, so the move is recorded once.
        //
        // The release decision itself lives in the pure, unit-tested predicate
        // `should_finalize_move_fallback` (see tests at the bottom of this file)
        // so the #183 fix path is exercised in CI, not only by manual GUI drags.
        else if should_finalize_move_fallback(
            !self.move_drag_origins.is_empty(),
            ui.input(|i| i.pointer.primary_down()),
            response.dragged_by(egui::PointerButton::Primary),
        ) {
            self.moving = false;
            tracing::debug!(
                target: "photonic::move",
                nodes = self.move_drag_origins.len(),
                "#183 move recorded via fallback path (canvas response swallowed; \
                 drag_stopped_by(Primary) never fired)"
            );
            self.finalize_move(doc, history, doc_modified);
        }

        // Click on empty space to deselect (without shift)
        if response.clicked_by(egui::PointerButton::Primary) && !self.moving {
            if let Some(pos) = response.interact_pointer_pos() {
                let (cx, cy) = view.screen_to_canvas(pos.x as f64, pos.y as f64);
                let shift = ui.input(|i| i.modifiers.shift);
                let alt = ui.input(|i| i.modifiers.alt);
                let hit = hit_test(doc, cx, cy, renderer);
                if shift {
                    if let Some(id) = hit {
                        // Toggle the whole group as a unit (see the drag path).
                        let members = self.select_group_members(doc, id, alt);
                        if members.iter().any(|m| doc.selection.contains(m)) {
                            for m in &members {
                                doc.selection.remove(m);
                            }
                            self.selected_id = doc.selection.ids().next().copied();
                        } else {
                            for m in &members {
                                doc.selection.add(*m);
                            }
                            self.selected_id = Some(id);
                        }
                    }
                } else {
                    match hit {
                        Some(id) => {
                            let members = self.select_group_members(doc, id, alt);
                            doc.selection = Selection::from_ids(members.iter().copied());
                            self.selected_id = Some(id);
                        }
                        None => {
                            self.selected_id = None;
                            doc.selection.clear();
                        }
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
            // Closed (grabbing) hand only while actively dragging a move
            egui::CursorIcon::Grabbing
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
                // Rotate frame: within the band just outside the selection box.
                const ROTATE_ZONE: f32 = 44.0;
                let (near_corner, on_body) = hover_bounds
                    .map(|(bx0, by0, bx1, by1)| {
                        let (sx0, sy0) = view.canvas_to_screen(bx0, by0);
                        let (sx1, sy1) = view.canvas_to_screen(bx1, by1);
                        let rect = egui::Rect::from_two_pos(
                            egui::pos2(sx0 as f32, sy0 as f32),
                            egui::pos2(sx1 as f32, sy1 as f32),
                        );
                        let inside = rect.contains(hover_pos);
                        let in_frame = rect.expand(ROTATE_ZONE).contains(hover_pos) && !inside;
                        (in_frame, inside)
                    })
                    .unwrap_or((false, false));
                if near_corner {
                    // egui has no rotate cursor — paint a small clockwise-arrow
                    // affordance at the pointer so the rotate zone reads clearly.
                    ui.painter().text(
                        hover_pos + egui::vec2(16.0, -16.0),
                        egui::Align2::CENTER_CENTER,
                        ph::ARROW_CLOCKWISE,
                        egui::FontId::proportional(18.0),
                        egui::Color32::from_rgb(110, 86, 207),
                    );
                    egui::CursorIcon::Grab
                } else if on_body {
                    // Open (grab) hand on hover to signal a draggable move
                    egui::CursorIcon::Grab
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
        history: &mut CommandHistory,
        doc_modified: &mut bool,
    ) {
        let curvature_mode = self.active_tool == Tool::CurvaturePen;

        // Escape cancels the in-progress path
        if viewport_kb(ui.ctx()) && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.clear_pen_path();
            return;
        }

        // Cursor: reflect the Pen's state while active and hovering the canvas.
        // egui has no dedicated pen glyph, so reuse existing variants: Crosshair for
        // normal point-placing, and PointingHand when hovering the first anchor with
        // enough points to close (signals "click to close the path").
        if response.hovered() {
            let icon = ui
                .input(|i| i.pointer.hover_pos())
                .filter(|&pos| self.pen_over_first_anchor(view, pos))
                .map(|_| egui::CursorIcon::PointingHand)
                .unwrap_or(egui::CursorIcon::Crosshair);
            ui.ctx().set_cursor_icon(icon);
        }

        // Double-click finalises the path, closing it (also fires clicked, so first)
        if response.double_clicked_by(egui::PointerButton::Primary) {
            if let Some(path) = self.build_pen_path(true) {
                self.finalize_pen_node(path, doc, history, doc_modified);
            }
            self.clear_pen_path();
            return;
        }

        // A drag places its anchor at the press origin, then pulls a symmetric
        // direction line from it. The segment arriving at the anchor uses the
        // opposite handle; the segment leaving it uses the pointer-side handle.
        // Curvature Pen deliberately ignores manual handles — its smoothing is
        // recomputed from the on-curve positions after every placement.
        if response.drag_started_by(egui::PointerButton::Primary) && !ui.input(|i| i.modifiers.alt)
        {
            let press = ui
                .input(|i| i.pointer.press_origin())
                .or_else(|| response.interact_pointer_pos());
            if let Some(pos) = press.filter(|&pos| !self.pen_over_first_anchor(view, pos)) {
                let (cx, cy) = view.screen_to_canvas(pos.x as f64, pos.y as f64);
                self.push_pen_anchor(PenAnchor::corner(cx, cy));
                self.pen_drag_anchor = Some(self.pen_anchors.len() - 1);
            }
        }

        if response.dragged_by(egui::PointerButton::Primary) && !curvature_mode {
            if let (Some(index), Some(pos)) =
                (self.pen_drag_anchor, response.interact_pointer_pos())
            {
                let (hx, hy) = view.screen_to_canvas(pos.x as f64, pos.y as f64);
                if let Some(anchor) = self.pen_anchors.get_mut(index) {
                    anchor.pull_handle_to(Point::new(hx, hy));
                }
            }
        }

        if response.drag_stopped_by(egui::PointerButton::Primary)
            || (self.pen_drag_anchor.is_some()
                && !ui.input(|i| i.pointer.primary_down())
                && !response.dragged_by(egui::PointerButton::Primary))
        {
            self.pen_drag_anchor = None;
        }

        // Single click: add an anchor point — or close the path if the click lands
        // on the first anchor (Illustrator-style click-to-close).
        if response.clicked_by(egui::PointerButton::Primary) {
            if !ui.input(|i| i.modifiers.alt) {
                if let Some(pos) = response.interact_pointer_pos() {
                    if self.pen_over_first_anchor(view, pos) {
                        if let Some(path) = self.build_pen_path(true) {
                            self.finalize_pen_node(path, doc, history, doc_modified);
                        }
                        self.clear_pen_path();
                        return;
                    }
                    let (cx, cy) = view.screen_to_canvas(pos.x as f64, pos.y as f64);
                    self.push_pen_anchor(PenAnchor::corner(cx, cy));
                }
            }
        }

        // ── Preview ──────────────────────────────────────────────────────────
        let painter = ui.painter();
        let path_stroke = egui::Stroke::new(1.5, Color32::from_rgb(110, 86, 207));
        let rubber_stroke =
            egui::Stroke::new(1.0, Color32::from_rgba_unmultiplied(110, 86, 207, 128));

        // Draw the actual cubic preview, not a straight approximation between
        // anchors, so the gesture reads exactly like the path that will commit.
        if let Some(bez) = build_pen_bez(&self.pen_anchors, curvature_mode, false) {
            paint_pen_bez(painter, view, &bez, path_stroke);
        }

        // Anchor dots
        for anchor in &self.pen_anchors {
            let (sx, sy) = view.canvas_to_screen(anchor.position.x, anchor.position.y);
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

            // Direction lines make click-drag-created curvature explicit and
            // provide immediate feedback for both sides of the smooth anchor.
            if !curvature_mode {
                for handle in [anchor.in_handle, anchor.out_handle].into_iter().flatten() {
                    let (hsx, hsy) = view.canvas_to_screen(handle.x, handle.y);
                    let handle_pos = egui::pos2(hsx as f32, hsy as f32);
                    painter.line_segment([center, handle_pos], rubber_stroke);
                    painter.circle_filled(handle_pos, 2.5, Color32::WHITE);
                    painter.circle_stroke(handle_pos, 2.5, rubber_stroke);
                }
            }
        }

        // Rubber-band the next *actual* segment. In Curvature Pen mode the whole
        // provisional spline is recalculated with the cursor as its next anchor.
        if self.pen_drag_anchor.is_none() {
            if let (Some(last), Some(cursor)) = (
                self.pen_anchors.last().copied(),
                ui.input(|i| i.pointer.hover_pos()),
            ) {
                let (cx, cy) = view.screen_to_canvas(cursor.x as f64, cursor.y as f64);
                let mut preview = if curvature_mode {
                    self.pen_anchors.clone()
                } else {
                    vec![last]
                };
                preview.push(PenAnchor::corner(cx, cy));
                if let Some(bez) = build_pen_bez(&preview, curvature_mode, false) {
                    paint_pen_bez(painter, view, &bez, rubber_stroke);
                }
            }
        }
    }

    /// Clear every piece of transient Pen state. Shared by Escape and all tool
    /// switching surfaces so an unfinished path can never leak between modes.
    pub(crate) fn clear_pen_path(&mut self) {
        self.pen_anchors.clear();
        self.pen_redo_anchors.clear();
        self.pen_drag_anchor = None;
    }

    /// Add a fresh anchor and invalidate the transient redo chain, exactly like
    /// a new document edit after Undo.
    fn push_pen_anchor(&mut self, anchor: PenAnchor) {
        self.pen_anchors.push(anchor);
        self.pen_redo_anchors.clear();
    }

    /// Undo one uncommitted Pen placement before falling back to document
    /// history. Returns whether a transient anchor was consumed.
    pub(crate) fn undo_pen_anchor(&mut self) -> bool {
        if !matches!(self.active_tool, Tool::Pen | Tool::CurvaturePen) {
            return false;
        }
        let Some(anchor) = self.pen_anchors.pop() else {
            return false;
        };
        self.pen_redo_anchors.push(anchor);
        self.pen_drag_anchor = None;
        true
    }

    /// Redo one placement removed by [`Self::undo_pen_anchor`].
    pub(crate) fn redo_pen_anchor(&mut self) -> bool {
        if !matches!(self.active_tool, Tool::Pen | Tool::CurvaturePen) {
            return false;
        }
        let Some(anchor) = self.pen_redo_anchors.pop() else {
            return false;
        };
        self.pen_anchors.push(anchor);
        true
    }

    /// Build `PathData` from the accumulated anchors. Normal Pen uses explicit
    /// direction handles; Curvature Pen interpolates smoothly through positions.
    ///
    /// When `close` is set and there are at least 3 points, the path is closed
    /// (`close_path`), producing a filled region rather than an open polyline. A
    /// closed 2-point path is degenerate, so closing is skipped below the threshold.
    pub(crate) fn build_pen_path(&self, close: bool) -> Option<PathData> {
        let curvature_mode = self.active_tool == Tool::CurvaturePen;
        build_pen_bez(&self.pen_anchors, curvature_mode, close)
            .map(|bez| PathData::from_bez_path(&bez))
    }

    /// Screen-space hit test: is `screen` within the close radius of the first
    /// anchor, with enough points placed to close the path? Drives both the
    /// close-state cursor and click-to-close finalisation.
    fn pen_over_first_anchor(&self, view: &CanvasView, screen: egui::Pos2) -> bool {
        const CLOSE_RADIUS: f32 = 8.0;
        if self.pen_anchors.len() < 3 {
            return false;
        }
        let first = self.pen_anchors[0].position;
        let (sfx, sfy) = view.canvas_to_screen(first.x, first.y);
        (screen - egui::pos2(sfx as f32, sfy as f32)).length() <= CLOSE_RADIUS
    }

    /// Central chokepoint for a tool creating a node (#190): always routes
    /// through history so the edit is undoable. Cross-tool per-creation behaviour
    /// (tagging, logging, default-style hooks) belongs here so it applies to
    /// every tool at once — the reason tools now share a [`CanvasTool`] parent.
    pub(crate) fn tool_commit_add(
        &self,
        node: SceneNode,
        doc: &mut Document,
        history: &mut CommandHistory,
        doc_modified: &mut bool,
    ) {
        let target_layer = doc
            .active_layer_id
            .or_else(|| doc.layer_order.last().copied());
        if target_layer.is_none_or(|id| doc.is_layer_locked(&id)) {
            return;
        }
        history.execute(
            Command::AddNode {
                node,
                layer_id: None,
            },
            doc,
        );
        *doc_modified = true;
    }

    /// Commit a finalised pen `path` as a new document node (fill + optional
    /// default stroke). Shared by the double-click and click-to-close paths.
    fn finalize_pen_node(
        &self,
        path: PathData,
        doc: &mut Document,
        history: &mut CommandHistory,
        doc_modified: &mut bool,
    ) {
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
            if self.active_tool == Tool::CurvaturePen {
                "Curvature Pen"
            } else {
                "Pen"
            },
            doc.node_count() + 1,
        );
        self.tool_commit_add(node, doc, history, doc_modified);
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
        use photonic_core::history::Command;
        use photonic_core::ops::boolean::{boolean_op, BooleanOp};

        // Keep the touched real path nodes (de-duplicated), sorted bottom-to-top
        // by the document's global draw order — works across layers, matching
        // the Union/boolean handler. (The old code required all touched shapes to
        // live in one layer and silently did nothing otherwise, so Shape Builder
        // appeared broken whenever the shapes weren't in the same layer.)
        let order: Vec<NodeId> = doc.nodes_in_draw_order().iter().map(|n| n.id).collect();
        let order_of = |id: &NodeId| order.iter().position(|x| x == id).unwrap_or(usize::MAX);
        let mut path_ids: Vec<NodeId> = Vec::new();
        for &id in ids {
            if !path_ids.contains(&id)
                && matches!(
                    doc.get_node(&id).map(|n| &n.kind),
                    Some(SceneNodeKind::Path(_))
                )
            {
                path_ids.push(id);
            }
        }
        path_ids.sort_by_key(order_of);

        if path_ids.is_empty() {
            return; // dragged over nothing paintable — stay silent
        }

        // Subtract mode over a single shape deletes it (Illustrator Alt-click).
        if subtract && path_ids.len() == 1 {
            let node_id = path_ids[0];
            history.execute(Command::RemoveNode { node_id }, doc);
            self.shape_builder_hovered = None;
            self.selected_id = None;
            doc.selection.clear();
            *doc_modified = true;
            self.file_status = Some("Shape Builder: deleted shape".into());
            return;
        }
        if path_ids.len() < 2 {
            // The drag only caught one shape — guide the user (the interior of
            // each shape must be passed through; unfilled shapes only register
            // on their outline).
            self.file_status =
                Some("Shape Builder: drag across 2+ overlapping shapes to merge".into());
            return;
        }

        // Bake each transform into geometry, then fold the op with the bottom
        // shape as the base.
        let baked: Vec<PathData> = path_ids
            .iter()
            .filter_map(|id| {
                doc.get_node(id).and_then(|n| match &n.kind {
                    SceneNodeKind::Path(p) => Some(gui_apply_affine_to_path(
                        &p.path_data,
                        n.transform.to_kurbo(),
                    )),
                    _ => None,
                })
            })
            .collect();
        let op = if subtract {
            BooleanOp::Subtract
        } else {
            BooleanOp::Union
        };
        let mut acc = baked[0].clone();
        for p in &baked[1..] {
            match boolean_op(&acc, p, op) {
                Ok(r) => acc = r,
                Err(e) => {
                    self.file_status = Some(format!("Shape Builder failed: {e}"));
                    return;
                }
            }
        }
        if acc.to_bez_path().elements().is_empty() {
            self.file_status = Some("Shape Builder produced an empty shape".into());
            return;
        }

        // Result inherits the bottom shape's layer, fill and stroke.
        let base_id = path_ids[0];
        let Some(base) = doc.get_node(&base_id) else {
            return;
        };
        let base_layer = base.layer_id;
        let (fill, stroke) = match &base.kind {
            SceneNodeKind::Path(p) => (p.fill.clone(), p.stroke.clone()),
            _ => Default::default(),
        };
        let mut result_pn = photonic_core::node::PathNode::new(acc);
        result_pn.fill = fill;
        result_pn.stroke = stroke;
        let result_node = SceneNode::new("Shape", base_layer, SceneNodeKind::Path(result_pn));
        let result_id = result_node.id;
        let mut cmds: Vec<Command> = path_ids
            .iter()
            .map(|id| Command::RemoveNode { node_id: *id })
            .collect();
        cmds.push(Command::AddNode {
            node: result_node,
            layer_id: Some(base_layer),
        });
        history.execute(Command::Batch(cmds), doc);
        self.selected_id = Some(result_id);
        doc.selection = Selection::single(result_id);
        *doc_modified = true;
        self.file_status = Some(format!(
            "Shape Builder: {} {} shapes",
            if subtract { "subtracted" } else { "merged" },
            path_ids.len()
        ));
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
                let expand_icon = if self.lua_console.expanded {
                    ph::CARET_DOWN
                } else {
                    ph::CARET_UP
                };
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

#[cfg(test)]
mod pen_path_tests {
    use super::*;

    fn corner(x: f64, y: f64) -> PenAnchor {
        PenAnchor::corner(x, y)
    }

    #[test]
    fn dragging_pulls_symmetric_handles_and_can_retract_them() {
        let mut anchor = corner(10.0, 20.0);
        anchor.pull_handle_to(Point::new(25.0, 12.0));
        assert_eq!(anchor.out_handle, Some(Point::new(25.0, 12.0)));
        assert_eq!(anchor.in_handle, Some(Point::new(-5.0, 28.0)));

        anchor.pull_handle_to(anchor.position);
        assert_eq!(anchor.in_handle, None);
        assert_eq!(anchor.out_handle, None);
    }

    #[test]
    fn dragged_anchors_emit_their_exact_cubic_handles() {
        let mut first = corner(0.0, 0.0);
        first.in_handle = Some(Point::new(-10.0, 0.0));
        first.out_handle = Some(Point::new(10.0, 0.0));
        let mut second = corner(30.0, 20.0);
        second.in_handle = Some(Point::new(20.0, 20.0));
        second.out_handle = Some(Point::new(40.0, 20.0));

        let bez = build_pen_bez(&[first, second], false, false).unwrap();
        assert_eq!(bez.elements()[0], PathEl::MoveTo(first.position));
        assert_eq!(
            bez.elements()[1],
            PathEl::CurveTo(
                first.out_handle.unwrap(),
                second.in_handle.unwrap(),
                second.position,
            )
        );
    }

    #[test]
    fn clicked_anchors_remain_true_line_segments() {
        let anchors = [corner(0.0, 0.0), corner(20.0, 10.0)];
        let bez = build_pen_bez(&anchors, false, false).unwrap();
        assert_eq!(bez.elements()[1], PathEl::LineTo(anchors[1].position));
    }

    #[test]
    fn curvature_pen_passes_through_every_open_anchor() {
        let anchors = [
            corner(0.0, 0.0),
            corner(20.0, 30.0),
            corner(50.0, 10.0),
            corner(80.0, 40.0),
        ];
        let bez = build_pen_bez(&anchors, true, false).unwrap();
        assert_eq!(bez.elements()[0], PathEl::MoveTo(anchors[0].position));
        let endpoints: Vec<Point> = bez.elements()[1..]
            .iter()
            .filter_map(|element| match element {
                PathEl::CurveTo(_, _, endpoint) => Some(*endpoint),
                _ => None,
            })
            .collect();
        assert_eq!(
            endpoints,
            anchors[1..]
                .iter()
                .map(|anchor| anchor.position)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn closed_curvature_pen_smoothly_returns_to_first_anchor() {
        let anchors = [
            corner(0.0, 0.0),
            corner(40.0, 0.0),
            corner(40.0, 40.0),
            corner(0.0, 40.0),
        ];
        let bez = build_pen_bez(&anchors, true, true).unwrap();
        assert!(matches!(bez.elements().last(), Some(PathEl::ClosePath)));
        assert!(matches!(
            bez.elements()[bez.elements().len() - 2],
            PathEl::CurveTo(_, _, endpoint) if endpoint == anchors[0].position
        ));
    }

    #[test]
    fn pen_anchor_undo_and_redo_are_individual() {
        let mut app = PhotonicApp::default();
        app.active_tool = Tool::Pen;
        app.push_pen_anchor(corner(1.0, 2.0));
        app.push_pen_anchor(corner(3.0, 4.0));

        assert!(app.undo_pen_anchor());
        assert_eq!(app.pen_anchors, vec![corner(1.0, 2.0)]);
        assert!(app.redo_pen_anchor());
        assert_eq!(app.pen_anchors, vec![corner(1.0, 2.0), corner(3.0, 4.0)]);
    }

    #[test]
    fn fresh_pen_anchor_clears_transient_redo() {
        let mut app = PhotonicApp::default();
        app.active_tool = Tool::CurvaturePen;
        app.push_pen_anchor(corner(1.0, 2.0));
        app.push_pen_anchor(corner(3.0, 4.0));
        assert!(app.undo_pen_anchor());

        app.push_pen_anchor(corner(5.0, 6.0));
        assert!(!app.redo_pen_anchor());
        assert_eq!(app.pen_anchors, vec![corner(1.0, 2.0), corner(5.0, 6.0)]);
    }
}

#[cfg(test)]
mod shortcut_audit_tests {
    use super::*;

    #[test]
    fn every_default_binding_has_a_runtime_handler() {
        const SPECIAL: &[&str] = &[
            "edit.copy",
            "edit.paste",
            "edit.paste_in_place",
            "edit.delete",
            "palette.open",
        ];
        for def in crate::commands::REGISTRY
            .iter()
            .filter(|def| def.default.is_some())
        {
            assert!(
                GLOBAL_SHORTCUT_COMMANDS.contains(&def.id) || SPECIAL.contains(&def.id),
                "default shortcut {} has no keyboard dispatch path",
                def.id
            );
        }
    }
}

/// Release-decision predicate for the #183 fallback move recorder.
///
/// On a frame where the normal `response.drag_stopped_by(Primary)` release did
/// **not** fire — because a competing overlay allocated later in the frame
/// (artboard drag-handle / name hit-target, or a full-canvas modal scrim)
/// swallowed the canvas `response` (root-cause A2) — the completed move would
/// otherwise be silently dropped (the regression of #11). Returns `true` when
/// the pending move should still be finalized here:
///
/// * `move_pending` — origins were captured (`move_drag_origins` non-empty), so
///   an object actually moved and there is something to record;
/// * `!primary_down` — the primary button is no longer held, i.e. the gesture
///   really has ended (not merely paused mid-drag with the button still down);
/// * `!dragged_by_primary` — no primary drag is in progress this frame, so we do
///   not fire while the `drag_stopped_by` path still owns the release.
///
/// Extracted as a pure function so the exact #183 fix condition is unit-tested
/// (this crate cannot exercise a live egui drag headlessly).
pub(crate) fn should_finalize_move_fallback(
    move_pending: bool,
    primary_down: bool,
    dragged_by_primary: bool,
) -> bool {
    move_pending && !primary_down && !dragged_by_primary
}

#[cfg(test)]
mod move_fallback_tests {
    use super::should_finalize_move_fallback;

    /// The core #183 recovery case: a move is pending, the primary button has
    /// been released, and no drag is in progress this frame (the canvas response
    /// was swallowed so `drag_stopped_by` never fired). The fallback MUST select
    /// finalize — this is the branch that recovers the otherwise-lost move.
    #[test]
    fn swallowed_response_frame_finalizes() {
        assert!(should_finalize_move_fallback(
            /* move_pending */ true, /* primary_down */ false,
            /* dragged_by_primary */ false,
        ));
    }

    /// An in-progress drag (button held, dragging this frame) must NOT finalize.
    #[test]
    fn active_drag_does_not_finalize() {
        assert!(!should_finalize_move_fallback(true, true, true));
    }

    /// Button still held but momentarily not dragging (a pause): the gesture is
    /// not over, so do not finalize yet.
    #[test]
    fn paused_but_button_held_does_not_finalize() {
        assert!(!should_finalize_move_fallback(true, true, false));
    }

    /// The `drag_stopped_by(Primary)` frame reports the drag as still ongoing on
    /// the owning widget while the button is up; the normal release path handles
    /// it, so the fallback must stand down to avoid double-recording.
    #[test]
    fn drag_stopped_frame_defers_to_primary_path() {
        assert!(!should_finalize_move_fallback(true, false, true));
    }

    /// No move pending (nothing was captured / nothing moved): never finalize,
    /// regardless of button or drag state — including the A1 root-cause shape
    /// (origins empty at release), which this fallback intentionally cannot and
    /// must not paper over.
    #[test]
    fn no_pending_move_never_finalizes() {
        for &primary_down in &[false, true] {
            for &dragging in &[false, true] {
                assert!(!should_finalize_move_fallback(
                    false,
                    primary_down,
                    dragging
                ));
            }
        }
    }
}

#[cfg(test)]
mod clipboard_shortcut_tests {
    use super::*;
    use photonic_core::history::CommandHistory;
    use photonic_core::node::{GroupNode, PathNode};
    use photonic_core::{Document, PathData, SceneNode, SceneNodeKind};

    /// egui delivers Ctrl+C / Ctrl+V as `Event::Copy` / `Event::Paste` (the raw
    /// `Key::C` / `Key::V` events are swallowed), so the handler keys off those
    /// events. This proves they are visible in `i.events` for the frame — the
    /// mechanism the copy/paste shortcut depends on.
    #[test]
    fn egui_exposes_clipboard_events() {
        let ctx = egui::Context::default();
        let mut input = egui::RawInput::default();
        input.events.push(egui::Event::Copy);
        input.events.push(egui::Event::Paste("ignored".to_string()));
        let mut seen = (false, false);
        let _ = ctx.run(input, |ctx| {
            seen = ctx.input(|i| {
                (
                    i.events.iter().any(|e| matches!(e, egui::Event::Copy)),
                    i.events.iter().any(|e| matches!(e, egui::Event::Paste(_))),
                )
            });
        });
        assert!(seen.0, "Event::Copy must be visible in i.events");
        assert!(seen.1, "Event::Paste must be visible in i.events");
    }

    fn path(layer: photonic_core::layer::LayerId) -> SceneNode {
        SceneNode::new(
            "p",
            layer,
            SceneNodeKind::Path(PathNode::new(PathData::rect(0.0, 0.0, 10.0, 10.0))),
        )
    }

    /// Copying a group and pasting reproduces the whole subtree (fresh ids) and
    /// does not double-list its children in draw order.
    #[test]
    fn gui_clipboard_pastes_group_as_subtree() {
        let mut doc = Document::new("t", 100.0, 100.0);
        let layer = doc.active_layer_id.unwrap();
        let c1 = path(layer);
        let c2 = path(layer);
        let (id1, id2) = (c1.id, c2.id);
        doc.nodes.insert(id1, c1);
        doc.nodes.insert(id2, c2);
        let group = SceneNode::new(
            "g",
            layer,
            SceneNodeKind::Group(GroupNode {
                children: vec![id1, id2],
                clip_children: false,
                clip_node_id: None,
                blend_spine_id: None,
                live_boolean: None,
            }),
        );
        let group_id = group.id;
        doc.nodes.insert(group_id, group);
        doc.layers.get_mut(&layer).unwrap().node_ids.push(group_id);

        let before = doc.nodes_in_draw_order().len(); // two leaves

        let mut clip = GuiClipboard::default();
        clip.capture(&doc, [group_id].iter());
        assert!(!clip.is_empty(), "capture stored the group");

        let (cmd, roots) = clip
            .paste_command(layer, 10.0, 10.0)
            .expect("paste command built");
        assert_eq!(roots.len(), 1, "one fresh root");
        assert_ne!(roots[0], group_id, "root id is fresh, not the original");

        let mut history = CommandHistory::new(100);
        history.execute(cmd, &mut doc);
        assert_eq!(
            doc.nodes_in_draw_order().len(),
            before * 2,
            "pasted group must not double-list a child"
        );
        // Pasting again from the same buffer yields yet another distinct root.
        let (cmd2, roots2) = clip.paste_command(layer, 20.0, 20.0).unwrap();
        assert_ne!(roots2[0], roots[0], "repeat paste gets fresh ids");
        history.execute(cmd2, &mut doc);
        assert_eq!(doc.nodes_in_draw_order().len(), before * 3);
    }
}
