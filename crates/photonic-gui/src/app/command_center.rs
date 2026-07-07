//! Command dispatch + the Ctrl/Cmd+K searchable command palette (#140).
//!
//! [`PhotonicApp::dispatch_command`] is the single entry point that turns a
//! `commands::CommandId` into a real editor action (undo, group, flip, tool
//! activation, …). The palette and any keymap-driven shortcut both route through
//! it, so a remapped key and a palette click run identical code paths.
use super::*;
use crate::commands::{self, CommandId};

/// Sanitize a node name into a safe file stem (lowercase alnum + dashes).
fn sanitize_stem(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut prev_dash = true;
    for c in name.chars() {
        if c.is_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// Z-order move requested by an arrange command.
#[derive(Clone, Copy)]
enum ZMove {
    Forward,
    Backward,
    Front,
    Back,
}

impl PhotonicApp {
    /// True if the resolved binding for `id` was just pressed this frame.
    /// Consults `prefs.keymap` (user override) over the registry default.
    pub(crate) fn binding_pressed(&self, ctx: &egui::Context, id: CommandId) -> bool {
        match self.prefs.resolve_binding(id) {
            Some(b) => ctx.input(|i| i.key_pressed(b.key) && b.matches(i.modifiers)),
            None => false,
        }
    }

    /// Run a registered command. Returns `true` if the document changed.
    pub(crate) fn dispatch_command(
        &mut self,
        id: CommandId,
        doc: &mut Document,
        history: &mut CommandHistory,
    ) -> bool {
        let mut modified = false;
        match id {
            "edit.undo" => {
                if history.undo(doc) {
                    self.selected_id = doc.selection.ids().next().copied();
                    self.invalidate_point_edit(doc);
                    modified = true;
                }
            }
            "edit.redo" => {
                if history.redo(doc) {
                    self.selected_id = doc.selection.ids().next().copied();
                    self.invalidate_point_edit(doc);
                    modified = true;
                }
            }
            "edit.copy" => {
                let ids: Vec<NodeId> = doc.selection.ids().copied().collect();
                self.gui_clipboard.capture(doc, ids.iter());
            }
            "edit.paste" => modified = self.paste_clipboard(doc, history, 10.0),
            "edit.paste_in_place" => modified = self.paste_clipboard(doc, history, 0.0),
            "edit.duplicate" => modified = self.duplicate_selection(doc, history),
            "edit.delete" => {
                // Route the whole multi-select delete through history as one
                // undoable step so Ctrl+Z restores the removed nodes (#191).
                // `execute` hydrates each bare RemoveNode into RemoveNodeFull,
                // so undo re-adds the node into its original layer.
                let ids: Vec<NodeId> = doc.selection.ids().copied().collect();
                if !ids.is_empty() {
                    let cmds: Vec<Command> = ids
                        .iter()
                        .map(|&node_id| Command::RemoveNode { node_id })
                        .collect();
                    history.execute(Command::Batch(cmds), doc);
                    doc.selection.clear();
                    self.selected_id = None;
                    modified = true;
                }
            }
            "selection.select_all" => {
                let all: Vec<NodeId> = doc
                    .layer_order
                    .iter()
                    .filter_map(|lid| doc.layers.get(lid))
                    .flat_map(|l| l.node_ids.iter().copied())
                    .collect();
                if !all.is_empty() {
                    self.selected_id = all.first().copied();
                    doc.selection = Selection::from_ids(all);
                }
            }
            "selection.deselect" => {
                doc.selection.clear();
                self.selected_id = None;
            }
            "object.group" => self.do_group_selected(doc, history, &mut modified),
            "object.ungroup" => modified = self.ungroup_selection(doc, history),
            "object.ungroup_all" => {
                if let Some(id) = self.selected_id {
                    modified = self.ungroup_all_node(id, doc, history);
                }
            }
            "object.bring_forward" => {
                modified = self.reorder_selected(doc, history, ZMove::Forward)
            }
            "object.send_backward" => {
                modified = self.reorder_selected(doc, history, ZMove::Backward)
            }
            "object.bring_to_front" => modified = self.reorder_selected(doc, history, ZMove::Front),
            "object.send_to_back" => modified = self.reorder_selected(doc, history, ZMove::Back),
            "object.flip_horizontal" => modified = self.flip_selection(doc, history, true),
            "object.flip_vertical" => modified = self.flip_selection(doc, history, false),
            "view.outline_mode" => self.toggle_outline_mode(),
            "view.pixel_preview" => self.toggle_pixel_preview(),
            "view.overprint_preview" => self.toggle_overprint_preview(),
            "view.toggle_guides" => self.guides_visible = !self.guides_visible,
            "view.toggle_grid" => self.prefs.show_grid = !self.prefs.show_grid,
            "view.toggle_keyline_grid" => {
                self.prefs.show_keyline_grid = !self.prefs.show_keyline_grid
            }
            "view.toggle_snap_pixel" => self.prefs.snap_to_pixel = !self.prefs.snap_to_pixel,
            "assets.import_design_tokens" => modified = self.import_design_tokens_dialog(doc),
            "document.export_icon_set" => self.export_icon_set_dialog(doc),
            "view.fit" => self.fit_pending = true,
            "view.toggle_audit" => self.audit.panel_open = !self.audit.panel_open,
            "palette.open" => self.command_palette_open = true,
            // ── Mode switch (video-editor-module 04 §1.2) ────────────────────
            // All three route through the same helper (`app/monitor.rs`) so
            // the lazy-creation invariant (§1.3: `doc.timeline.is_some()`
            // whenever `self.mode == Video`) and the exit-pauses-playback
            // seam (§7) apply no matter which entry point fired.
            "mode.toggle_video" => self.enter_or_exit_video_mode(doc, history),
            "mode.enter_video" => {
                if self.mode != AppMode::Video {
                    self.enter_or_exit_video_mode(doc, history);
                }
            }
            "mode.exit_video" => {
                if self.mode == AppMode::Video {
                    self.enter_or_exit_video_mode(doc, history);
                }
            }
            // ── Video transport (04 §5.1, §3.2) — owned by this story; each
            // calls a real placeholder method on `PhotonicApp` (`app/monitor.rs`)
            // that moves `self.playhead` until the P3 engine lands.
            "video.play_pause" => self.video_play_pause(),
            "video.play_reverse" => self.video_play_reverse(),
            "video.pause" => self.video_pause(),
            "video.play_forward" => self.video_play_forward(),
            "video.step_back" => self.video_step_back(doc),
            "video.step_forward" => self.video_step_forward(doc),
            "video.set_in" => self.video_set_in(doc, history),
            "video.set_out" => self.video_set_out(doc, history),
            "video.playhead_home" => self.timeline_playhead_home(),
            "video.playhead_end" => self.timeline_playhead_end(doc),
            // ── Timeline-panel edit commands (04 §5.1) — owned by the P2-wave
            // timeline-panel story (`app/timeline/interact.rs`+`ops_bridge.rs`,
            // not yet landed in this tree). Calls are written against the
            // `pub(crate) fn <name>(&mut self, ...)` methods that story adds to
            // `PhotonicApp`; see `app/mode_fallbacks.rs` for the TEMP no-op
            // shims that make this compile until they land (delete that file
            // once they do — it's marked for the orchestrator).
            "video.prev_edit_point" => self.timeline_prev_edit_point(doc),
            "video.next_edit_point" => self.timeline_next_edit_point(doc),
            "video.split_at_playhead" => self.timeline_split_at_playhead(doc, history),
            "video.toggle_snap" => self.timeline_toggle_snap(),
            "video.zoom_in" => self.timeline_zoom_in(),
            "video.zoom_out" => self.timeline_zoom_out(),
            "video.zoom_fit" => self.timeline_zoom_fit(doc),
            _ => {
                if let Some(t) = commands::tool_for_command(id) {
                    // Clear stale point-edit state so entering Direct Select via the
                    // command palette re-seeds from the current selection (#164 finding 1).
                    self.clear_point_edit();
                    self.active_tool = t;
                }
            }
        }
        modified
    }

    /// #207 (GUI equivalent of the `import_design_tokens` MCP tool): pick a
    /// tokens file (CSS / JSON / Style Dictionary) and register named color
    /// swatches from it. Returns true if any swatch was added.
    pub(crate) fn import_design_tokens_dialog(&mut self, doc: &mut Document) -> bool {
        let Some(path) = run_file_dialog(|| {
            rfd::FileDialog::new()
                .add_filter("Design tokens", &["json", "css", "tokens", "txt"])
                .add_filter("All files", &["*"])
                .pick_file()
        }) else {
            return false;
        };
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                self.set_import_status(format!("Import failed: {e}"));
                return false;
            }
        };
        let hint = match path.extension().and_then(|e| e.to_str()) {
            Some("css") => Some("css"),
            Some("json") | Some("tokens") => Some("json"),
            _ => None,
        };
        let tokens = photonic_core::tokens::parse_token_colors(&text, hint);
        let mut added = 0usize;
        let mut updated = 0usize;
        for (name, hex) in tokens {
            let Some(c) = photonic_core::color::Color::from_hex(&hex) else {
                continue;
            };
            let norm = c.to_hex();
            if let Some(existing) = doc.color_swatches.iter_mut().find(|s| s.name == name) {
                existing.color_hex = norm;
                updated += 1;
            } else {
                doc.color_swatches
                    .push(photonic_core::ColorSwatch::new(&name, &norm));
                added += 1;
            }
        }
        self.set_import_status(format!(
            "Imported design tokens: {added} added, {updated} updated"
        ));
        added > 0 || updated > 0
    }

    /// #203 (GUI equivalent of `export_icon_set`): pick a folder and write every
    /// top-level group as a normalized (uniform square) `.svg`.
    fn export_icon_set_dialog(&mut self, doc: &Document) {
        let Some(dir) = run_file_dialog(|| rfd::FileDialog::new().pick_folder()) else {
            return;
        };
        use photonic_core::export::{SvgNormalize, SvgSelectionOptions};
        let opts = SvgSelectionOptions {
            precision: 4,
            optimize: true,
            normalize: SvgNormalize::Square { pad: 0.1 },
        };
        let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut count = 0usize;
        for layer_id in &doc.layer_order {
            let Some(layer) = doc.layers.get(layer_id) else {
                continue;
            };
            for nid in &layer.node_ids {
                let Some(node) = doc.nodes.get(nid) else {
                    continue;
                };
                if !matches!(node.kind, photonic_core::node::SceneNodeKind::Group(_)) {
                    continue;
                }
                let svg = photonic_core::export::export_nodes_as_svg_opts(doc, &[*nid], &opts);
                let mut base = sanitize_stem(&node.name);
                if base.is_empty() {
                    base = "icon".into();
                }
                let mut stem = base.clone();
                let mut n = 2;
                while !used.insert(stem.clone()) {
                    stem = format!("{base}-{n}");
                    n += 1;
                }
                if std::fs::write(dir.join(format!("{stem}.svg")), svg).is_ok() {
                    count += 1;
                }
            }
        }
        self.set_import_status(format!("Exported {count} icon(s) to {}", dir.display()));
    }

    /// Surface a short status line for import/export actions. The visible result
    /// is the updated swatch panel / written files; this logs a summary.
    fn set_import_status(&mut self, msg: String) {
        tracing::info!("{msg}");
    }

    /// Paste the in-process clipboard with an optional offset (10px = "paste",
    /// 0 = "paste in place"). Shared by both paste commands.
    fn paste_clipboard(
        &mut self,
        doc: &mut Document,
        history: &mut CommandHistory,
        offset: f64,
    ) -> bool {
        if self.gui_clipboard.is_empty() {
            return false;
        }
        let Some(target_layer) = doc
            .active_layer_id
            .or_else(|| doc.layer_order.first().copied())
        else {
            return false;
        };
        let Some((cmd, new_ids)) = self.gui_clipboard.paste_command(target_layer, offset, offset)
        else {
            return false;
        };
        history.execute(cmd, doc);
        doc.selection = Selection::from_ids(new_ids.iter().copied());
        self.selected_id = new_ids.first().copied();
        true
    }

    /// Duplicate every selected node in place (+10px), selecting the copies.
    /// Groups are duplicated as whole subtrees (descendants get fresh ids and
    /// remapped references), and each copy lands in its source layer.
    fn duplicate_selection(&mut self, doc: &mut Document, history: &mut CommandHistory) -> bool {
        use std::collections::HashMap;
        let sel: Vec<NodeId> = doc.selection.ids().copied().collect();
        // Bucket the selected roots by their layer so each copy stays put.
        let mut by_layer: HashMap<LayerId, Vec<NodeId>> = HashMap::new();
        for nid in &sel {
            if let Some(node) = doc.nodes.get(nid) {
                by_layer.entry(node.layer_id).or_default().push(*nid);
            }
        }
        let mut cmds: Vec<Command> = Vec::new();
        let mut new_ids: Vec<NodeId> = Vec::new();
        for (layer_id, roots) in by_layer {
            let (r, mut nodes) = photonic_core::ops::cloning::clone_subtrees(
                &doc.nodes, &roots, layer_id, 10.0, 10.0,
            );
            if r.is_empty() {
                continue;
            }
            for n in nodes.iter_mut() {
                if r.contains(&n.id) {
                    n.name = format!("{} copy", n.name);
                }
            }
            new_ids.extend(r.iter().copied());
            cmds.push(Command::AddSubtree {
                layer_id,
                roots: r,
                nodes,
            });
        }
        if cmds.is_empty() {
            return false;
        }
        history.execute(Command::Batch(cmds), doc);
        doc.selection = Selection::from_ids(new_ids.iter().copied());
        self.selected_id = new_ids.first().copied();
        true
    }

    /// Ungroup the selected node when it is a group.
    fn ungroup_selection(&mut self, doc: &mut Document, history: &mut CommandHistory) -> bool {
        let Some(sel_id) = self.selected_id else {
            return false;
        };
        let Some(node) = doc.get_node(&sel_id) else {
            return false;
        };
        let SceneNodeKind::Group(g) = &node.kind else {
            return false;
        };
        let children = g.children.clone();
        let node_clone = node.clone();
        let Some((layer_id, group_index)) = doc.node_layer_and_index(&sel_id) else {
            return false;
        };
        let first_child = children.first().copied();
        history.execute(
            Command::UngroupNodes {
                group: node_clone,
                layer_id,
                group_index,
                children,
            },
            doc,
        );
        self.selected_id = first_child;
        match first_child {
            Some(fc) => doc.selection = Selection::single(fc),
            None => doc.selection.clear(),
        }
        true
    }

    /// Ungroup the selected group recursively — flatten it and every nested
    /// group into their leaf nodes in one undoable step. Uses the same
    /// `UngroupNodes` primitive as single-level ungroup (so transform/z-order
    /// semantics match), applied breadth-first: each ungroup is simulated on a
    /// scratch document so every command carries the correct layer index for a
    /// clean undo.
    pub(crate) fn ungroup_all_node(
        &mut self,
        root: NodeId,
        doc: &mut Document,
        history: &mut CommandHistory,
    ) -> bool {
        let (cmds, leaves) = plan_ungroup_all(doc, root);
        if cmds.is_empty() {
            return false;
        }
        history.execute(Command::Batch(cmds), doc);
        if leaves.is_empty() {
            doc.selection.clear();
            self.selected_id = None;
        } else {
            doc.selection = Selection::from_ids(leaves.iter().copied());
            self.selected_id = leaves.first().copied();
        }
        true
    }

    /// Change the z-order of the selected node within its layer.
    fn reorder_selected(
        &mut self,
        doc: &mut Document,
        history: &mut CommandHistory,
        mv: ZMove,
    ) -> bool {
        let Some(sel_id) = self.selected_id else {
            return false;
        };
        let Some((layer_id, cur_idx)) = doc.node_layer_and_index(&sel_id) else {
            return false;
        };
        let layer_len = doc
            .layers
            .get(&layer_id)
            .map(|l| l.node_ids.len())
            .unwrap_or(0);
        if layer_len == 0 {
            return false;
        }
        let new_index = match mv {
            ZMove::Front => layer_len - 1,
            ZMove::Back => 0,
            ZMove::Forward => (cur_idx + 1).min(layer_len - 1),
            ZMove::Backward => cur_idx.saturating_sub(1),
        };
        if new_index == cur_idx {
            return false;
        }
        history.execute(
            Command::ReorderNode {
                layer_id,
                node_id: sel_id,
                old_index: cur_idx,
                new_index,
            },
            doc,
        );
        true
    }

    /// Mirror every selected path about its own bounding-box center.
    pub(crate) fn flip_selection(
        &mut self,
        doc: &mut Document,
        history: &mut CommandHistory,
        horizontal: bool,
    ) -> bool {
        use kurbo::Shape;
        let sel: Vec<NodeId> = doc.selection.ids().copied().collect();
        let mut changed = false;
        for nid in &sel {
            let Some(node) = doc.nodes.get(nid) else {
                continue;
            };
            let SceneNodeKind::Path(pn) = &node.kind else {
                continue;
            };
            let bez = pn.path_data.to_bez_path();
            let bbox = bez.bounding_box();
            let cx = bbox.x0 + bbox.width() / 2.0;
            let cy = bbox.y0 + bbox.height() / 2.0;
            let flip = |p: kurbo::Point| {
                if horizontal {
                    kurbo::Point::new(2.0 * cx - p.x, p.y)
                } else {
                    kurbo::Point::new(p.x, 2.0 * cy - p.y)
                }
            };
            let mut new_bez = BezPath::new();
            for el in bez.elements() {
                match *el {
                    PathEl::MoveTo(p) => new_bez.move_to(flip(p)),
                    PathEl::LineTo(p) => new_bez.line_to(flip(p)),
                    PathEl::CurveTo(c1, c2, p) => new_bez.curve_to(flip(c1), flip(c2), flip(p)),
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
            changed = true;
        }
        changed
    }

    /// Ctrl/Cmd+K toggle + the centered, fuzzy command palette overlay.
    /// Returns `true` if a command ran and changed the document.
    pub(crate) fn command_palette(
        &mut self,
        ctx: &egui::Context,
        doc: &mut Document,
        history: &mut CommandHistory,
    ) -> bool {
        // Global open/close toggle — works regardless of focus so it can be
        // summoned over any panel.
        if self.binding_pressed(ctx, "palette.open") {
            self.command_palette_open = !self.command_palette_open;
            self.command_palette_query.clear();
            self.command_palette_sel = 0;
            self.command_palette_focus = self.command_palette_open;
        }
        if !self.command_palette_open {
            return false;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.command_palette_open = false;
            return false;
        }

        // Fuzzy-filter the catalog by label subsequence (reuses global_search).
        let q = self.command_palette_query.trim().to_lowercase();
        let all = commands::all_commands();
        let mut filtered: Vec<&commands::CommandEntry> = if q.is_empty() {
            all.iter().collect()
        } else {
            let mut v: Vec<&commands::CommandEntry> = all
                .iter()
                .filter(|c| {
                    let l = c.label.to_lowercase();
                    l.contains(&q) || crate::global_search::fuzzy_subseq(&q, &l)
                })
                .collect();
            v.sort_by_key(|c| {
                let l = c.label.to_lowercase();
                (!l.starts_with(&q), !l.contains(&q), c.label.len())
            });
            v
        };
        filtered.truncate(60);
        if self.command_palette_sel >= filtered.len() {
            self.command_palette_sel = filtered.len().saturating_sub(1);
        }

        let (up, down, enter) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::ArrowUp),
                i.key_pressed(egui::Key::ArrowDown),
                i.key_pressed(egui::Key::Enter),
            )
        });
        if down && !filtered.is_empty() {
            self.command_palette_sel = (self.command_palette_sel + 1) % filtered.len();
        }
        if up && !filtered.is_empty() {
            self.command_palette_sel =
                (self.command_palette_sel + filtered.len() - 1) % filtered.len();
        }

        let mut chosen: Option<CommandId> = None;
        if enter {
            chosen = filtered.get(self.command_palette_sel).map(|c| c.id);
        }

        let screen = ctx.screen_rect();
        let width = 460.0_f32;
        let pos = egui::pos2(screen.center().x - width / 2.0, screen.top() + 120.0);

        // Dimmed backdrop that also closes the palette on a click outside.
        egui::Area::new(egui::Id::new("command_palette_backdrop"))
            .order(egui::Order::Foreground)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                let resp = ui.allocate_response(screen.size(), egui::Sense::click());
                ui.painter()
                    .rect_filled(screen, 0.0, Color32::from_black_alpha(120));
                if resp.clicked() {
                    self.command_palette_open = false;
                }
            });

        egui::Area::new(egui::Id::new("command_palette"))
            .order(egui::Order::Tooltip)
            .fixed_pos(pos)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style())
                    .inner_margin(egui::Margin::same(10.0))
                    .show(ui, |ui| {
                        ui.set_width(width);
                        let edit = ui.add(
                            egui::TextEdit::singleline(&mut self.command_palette_query)
                                .hint_text(format!(
                                    "{}  Run a command…",
                                    egui_phosphor::regular::MAGNIFYING_GLASS
                                ))
                                .desired_width(f32::INFINITY),
                        );
                        if self.command_palette_focus {
                            edit.request_focus();
                            self.command_palette_focus = false;
                        }
                        ui.add_space(6.0);
                        egui::ScrollArea::vertical()
                            .max_height(360.0)
                            .show(ui, |ui| {
                                if filtered.is_empty() {
                                    ui.label(RichText::new("No matching commands").weak());
                                }
                                for (i, c) in filtered.iter().enumerate() {
                                    let selected = i == self.command_palette_sel;
                                    let binding = if c.is_tool {
                                        None
                                    } else {
                                        self.prefs.resolve_binding(c.id)
                                    };
                                    let row = ui.horizontal(|ui| {
                                        ui.set_width(ui.available_width());
                                        let lbl = ui.selectable_label(
                                            selected,
                                            RichText::new(&c.label).strong(),
                                        );
                                        if let Some(b) = binding {
                                            ui.with_layout(
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui| {
                                                    ui.label(
                                                        RichText::new(b.display()).weak().small(),
                                                    );
                                                },
                                            );
                                        }
                                        lbl
                                    });
                                    if row.inner.clicked() {
                                        chosen = Some(c.id);
                                    }
                                }
                            });
                    });
            });

        if let Some(id) = chosen {
            self.command_palette_open = false;
            self.command_palette_query.clear();
            return self.dispatch_command(id, doc, history);
        }
        false
    }
}

/// Plan a recursive ungroup of the group `root`: return the ordered list of
/// `UngroupNodes` commands that flatten it and every nested group, plus the leaf
/// node ids that remain. Pure — simulates each ungroup on a scratch clone so
/// each command's `group_index` is correct for a clean, single-step undo.
/// Returns empty when `root` is not a group.
pub(crate) fn plan_ungroup_all(doc: &Document, root: NodeId) -> (Vec<Command>, Vec<NodeId>) {
    use std::collections::VecDeque;
    if !matches!(
        doc.nodes.get(&root).map(|n| &n.kind),
        Some(SceneNodeKind::Group(_))
    ) {
        return (Vec::new(), Vec::new());
    }
    let mut work = doc.clone();
    let mut cmds: Vec<Command> = Vec::new();
    let mut leaves: Vec<NodeId> = Vec::new();
    let mut queue: VecDeque<NodeId> = VecDeque::new();
    queue.push_back(root);
    while let Some(gid) = queue.pop_front() {
        let Some(node) = work.nodes.get(&gid) else {
            continue;
        };
        let SceneNodeKind::Group(g) = &node.kind else {
            continue;
        };
        let children = g.children.clone();
        let node_clone = node.clone();
        let Some((layer_id, group_index)) = work.node_layer_and_index(&gid) else {
            continue;
        };
        let cmd = Command::UngroupNodes {
            group: node_clone,
            layer_id,
            group_index,
            children: children.clone(),
        };
        cmd.apply(&mut work);
        cmds.push(cmd);
        for c in &children {
            match work.nodes.get(c).map(|n| &n.kind) {
                Some(SceneNodeKind::Group(_)) => queue.push_back(*c),
                Some(_) => leaves.push(*c),
                None => {}
            }
        }
    }
    (cmds, leaves)
}

#[cfg(test)]
mod ungroup_all_tests {
    use super::plan_ungroup_all;
    use photonic_core::history::{Command, CommandHistory};
    use photonic_core::node::{GroupNode, NodeId, PathNode, SceneNode, SceneNodeKind};
    use photonic_core::{Document, PathData};

    fn leaf(doc: &Document) -> SceneNode {
        SceneNode::new(
            "p",
            doc.active_layer_id.unwrap(),
            SceneNodeKind::Path(PathNode::new(PathData::rect(0.0, 0.0, 10.0, 10.0))),
        )
    }

    fn group(doc: &Document, children: Vec<NodeId>) -> SceneNode {
        SceneNode::new(
            "g",
            doc.active_layer_id.unwrap(),
            SceneNodeKind::Group(GroupNode {
                children,
                clip_children: false,
                clip_node_id: None,
                blend_spine_id: None,
                live_boolean: None,
            }),
        )
    }

    /// Build a doc with a nested group tree: outer { a, mid { b, c } } plus a
    /// standalone leaf `z` at top level. Returns (doc, outer_id, [a,b,c], z).
    fn nested_doc() -> (Document, NodeId, Vec<NodeId>, NodeId) {
        let mut doc = Document::new("t", 100.0, 100.0);
        let layer = doc.active_layer_id.unwrap();
        let a = leaf(&doc);
        let b = leaf(&doc);
        let c = leaf(&doc);
        let z = leaf(&doc);
        let (ai, bi, ci, zi) = (a.id, b.id, c.id, z.id);
        for n in [a, b, c, z] {
            doc.nodes.insert(n.id, n);
        }
        let mid = group(&doc, vec![bi, ci]);
        let mid_id = mid.id;
        doc.nodes.insert(mid_id, mid);
        let outer = group(&doc, vec![ai, mid_id]);
        let outer_id = outer.id;
        doc.nodes.insert(outer_id, outer);
        // Layer top-level: [outer, z] (mid/a/b/c are nested, not top-level).
        doc.layers.get_mut(&layer).unwrap().node_ids = vec![outer_id, zi];
        (doc, outer_id, vec![ai, bi, ci], zi)
    }

    #[test]
    fn flattens_nested_groups_to_leaves_in_one_step() {
        let (mut doc, outer, leaves, z) = nested_doc();
        let layer = doc.active_layer_id.unwrap();
        let (cmds, planned_leaves) = plan_ungroup_all(&doc, outer);
        assert_eq!(cmds.len(), 2, "outer + mid = two ungroup commands");
        assert_eq!(planned_leaves.len(), 3);

        let mut history = CommandHistory::new(100);
        history.execute(Command::Batch(cmds), &mut doc);

        // No group nodes remain anywhere.
        assert!(
            !doc.nodes.values().any(|n| matches!(n.kind, SceneNodeKind::Group(_))),
            "all groups dissolved"
        );
        // All three leaves + z are now top-level in the layer, no dangling ids.
        let top = &doc.layers.get(&layer).unwrap().node_ids;
        for l in &leaves {
            assert!(top.contains(l), "leaf promoted to top level");
        }
        assert!(top.contains(&z));
        assert_eq!(top.len(), 4);

        // Single undo restores the whole nested structure.
        assert!(history.undo(&mut doc));
        let top = &doc.layers.get(&layer).unwrap().node_ids;
        assert_eq!(top, &vec![outer, z], "undo restored original top level");
        assert!(matches!(doc.nodes.get(&outer).map(|n| &n.kind), Some(SceneNodeKind::Group(_))));
    }

    #[test]
    fn non_group_root_is_noop() {
        let (doc, _outer, leaves, _z) = nested_doc();
        let (cmds, planned) = plan_ungroup_all(&doc, leaves[0]);
        assert!(cmds.is_empty() && planned.is_empty());
    }
}
