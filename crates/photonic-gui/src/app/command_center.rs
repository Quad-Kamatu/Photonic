//! Command dispatch + the Ctrl/Cmd+K searchable command palette (#140).
//!
//! [`PhotonicApp::dispatch_command`] is the single entry point that turns a
//! `commands::CommandId` into a real editor action (undo, group, flip, tool
//! activation, …). The palette and any keymap-driven shortcut both route through
//! it, so a remapped key and a palette click run identical code paths.
use super::*;
use crate::commands::{self, CommandId};

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
                self.gui_clipboard.clear();
                for nid in doc.selection.ids() {
                    if let Some(node) = doc.nodes.get(nid) {
                        self.gui_clipboard.push(node.clone());
                    }
                }
            }
            "edit.paste" => modified = self.paste_clipboard(doc, history, 10.0),
            "edit.paste_in_place" => modified = self.paste_clipboard(doc, history, 0.0),
            "edit.duplicate" => modified = self.duplicate_selection(doc, history),
            "edit.delete" => {
                let ids: Vec<NodeId> = doc.selection.ids().copied().collect();
                if !ids.is_empty() {
                    for nid in ids {
                        doc.remove_node(&nid);
                    }
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
            "object.bring_forward" => modified = self.reorder_selected(doc, history, ZMove::Forward),
            "object.send_backward" => {
                modified = self.reorder_selected(doc, history, ZMove::Backward)
            }
            "object.bring_to_front" => modified = self.reorder_selected(doc, history, ZMove::Front),
            "object.send_to_back" => modified = self.reorder_selected(doc, history, ZMove::Back),
            "object.flip_horizontal" => modified = self.flip_selection(doc, history, true),
            "object.flip_vertical" => modified = self.flip_selection(doc, history, false),
            "view.outline_mode" => self.outline_mode = !self.outline_mode,
            "view.toggle_guides" => self.guides_visible = !self.guides_visible,
            "view.toggle_grid" => self.prefs.show_grid = !self.prefs.show_grid,
            "view.fit" => self.fit_pending = true,
            "view.toggle_audit" => self.audit.panel_open = !self.audit.panel_open,
            "palette.open" => self.command_palette_open = true,
            _ => {
                if let Some(t) = commands::tool_for_command(id) {
                    self.active_tool = t;
                }
            }
        }
        modified
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
        if cmds.is_empty() {
            return false;
        }
        history.execute(Command::Batch(cmds), doc);
        doc.selection = Selection::from_ids(new_ids.iter().copied());
        self.selected_id = new_ids.first().copied();
        true
    }

    /// Duplicate every selected node in place (+10px), selecting the copies.
    fn duplicate_selection(&mut self, doc: &mut Document, history: &mut CommandHistory) -> bool {
        let sel: Vec<NodeId> = doc.selection.ids().copied().collect();
        let mut cmds: Vec<Command> = Vec::new();
        let mut new_ids: Vec<NodeId> = Vec::new();
        for nid in &sel {
            if let Some(node) = doc.nodes.get(nid) {
                let mut copy = node.clone();
                copy.id = uuid::Uuid::new_v4();
                copy.name = format!("{} copy", copy.name);
                copy.transform.matrix[4] += 10.0;
                copy.transform.matrix[5] += 10.0;
                let lid = copy.layer_id;
                new_ids.push(copy.id);
                cmds.push(Command::AddNode {
                    node: copy,
                    layer_id: Some(lid),
                });
            }
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
        let layer_len = doc.layers.get(&layer_id).map(|l| l.node_ids.len()).unwrap_or(0);
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
                        egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
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
                                                ui.label(RichText::new(b.display()).weak().small());
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
