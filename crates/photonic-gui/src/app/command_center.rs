//! Command dispatch + the Ctrl/Cmd+K searchable command palette (#140).
//!
//! [`PhotonicApp::dispatch_command`] is the single entry point that turns a
//! `commands::CommandId` into a real editor action (undo, group, flip, tool
//! activation, …). The palette and any keymap-driven shortcut both route through
//! it, so a remapped key and a palette click run identical code paths.
use super::*;
use crate::app::timeline::{interact, ops_bridge};
use crate::commands::{self, CommandId};
use photonic_core::timeline::{ops, Clip, ClipTiming, Sequence, SequenceId, TrackKind};

/// One clip captured on the timeline clipboard (Ctrl+C / Ctrl+X, NLE parity
/// QW-3). Holds a full clone of the source clip — grade, effects, trim, speed
/// and transform all preserved — plus the kind of track it came from so paste
/// can target a compatible lane. Positions are stored source-relative: paste
/// maps the earliest clip's start onto the playhead and keeps the rest at their
/// original offsets.
#[derive(Clone)]
pub(crate) struct ClipboardClip {
    pub clip: Clip,
    pub kind: TrackKind,
}

/// Occupied `[start, end)` clip spans on one track (paste overlap-avoidance).
type Spans = Vec<(Tick, Tick)>;
/// A candidate paste track: id, kind, enabled, locked, and occupied spans.
struct PasteTrack {
    id: TrackId,
    kind: TrackKind,
    enabled: bool,
    locked: bool,
    spans: Spans,
}
/// A clipboard entry reduced to what the paste planner needs: kind, source
/// start, duration.
type PasteEntry = (TrackKind, Tick, Tick);
/// One planned paste placement: `(clipboard entry index, track index, new
/// start)`.
type Placement = (usize, usize, Tick);

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
            // ── Clip editing (NLE parity QW-1 / QW-3 / QW-4) ──────────────────
            // Delete/ripple-delete/copy/cut/paste of the timeline selection and
            // marker-at-playhead. Each routes through `ops_bridge` (or, for
            // paste's property-preserving insert, `ops::insert_clip`) so every
            // edit is a real undoable timeline command.
            "video.delete_clip" => self.timeline_delete_selection(doc, history, false),
            "video.ripple_delete" => self.timeline_delete_selection(doc, history, true),
            "video.copy" => {
                self.timeline_copy_selection(doc);
            }
            "video.cut" => self.timeline_cut_selection(doc, history),
            "video.paste" => {
                self.timeline_paste_clipboard(doc, history);
            }
            "video.add_marker" => self.timeline_add_marker_at_playhead(doc, history),
            // ── 3/4-point editing (spec 16 §4) ────────────────────────────────
            // Insert/Overwrite lay down the armed source at the playhead;
            // Lift/Extract clear the timeline in/out (`work_range`) on the
            // target track. Each is one undo step, built from the pure core ops
            // via `interact::do_*_edit`. Razor is a session-mode toggle whose
            // lane-click split is wired by the timeline-panel story.
            "video.insert_edit" => self.timeline_insert_edit(doc, history),
            "video.overwrite_edit" => self.timeline_overwrite_edit(doc, history),
            "video.lift_edit" => self.timeline_lift_edit(doc, history),
            "video.extract_edit" => self.timeline_extract_edit(doc, history),
            "video.toggle_razor" => self.timeline_toggle_razor(),
            "video.toggle_snap" => self.timeline_toggle_snap(),
            "video.zoom_in" => self.timeline_zoom_in(),
            "video.zoom_out" => self.timeline_zoom_out(),
            "video.zoom_fit" => self.timeline_zoom_fit(doc),
            // ── NLE parity round-2 (spec 17) ─────────────────────────────────
            // G1 Add-Edit-all-tracks / Close-Gap / Simplify; G2 Q/W/E trims +
            // Shift+Q/W rolls; G3 Match Frame / Reveal. Each is one undo step
            // (batches via `ops_bridge`); a no-op cleanly when nothing applies.
            "video.split_all_tracks" => self.timeline_split_all_tracks(doc, history),
            "video.close_gap" => self.timeline_close_gap_at_playhead(doc, history),
            "video.close_gaps" => self.timeline_close_all_gaps(doc, history),
            "video.simplify_sequence" => self.timeline_simplify_sequence(doc, history),
            "video.trim_start_to_playhead" => self.timeline_trim_to_playhead(doc, history, true),
            "video.trim_end_to_playhead" => self.timeline_trim_to_playhead(doc, history, false),
            "video.extend_edit" => self.timeline_extend_edit_to_playhead(doc, history),
            "video.roll_prev_to_playhead" => self.timeline_roll_to_playhead(doc, history, true),
            "video.roll_next_to_playhead" => self.timeline_roll_to_playhead(doc, history, false),
            "video.match_frame" => self.timeline_match_frame(doc),
            "video.reveal_in_project" => self.timeline_reveal_in_project(doc),
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
        let Some((cmd, new_ids)) = self
            .gui_clipboard
            .paste_command(target_layer, offset, offset)
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
            !doc.nodes
                .values()
                .any(|n| matches!(n.kind, SceneNodeKind::Group(_))),
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
        assert!(matches!(
            doc.nodes.get(&outer).map(|n| &n.kind),
            Some(SceneNodeKind::Group(_))
        ));
    }

    #[test]
    fn non_group_root_is_noop() {
        let (doc, _outer, leaves, _z) = nested_doc();
        let (cmds, planned) = plan_ungroup_all(&doc, leaves[0]);
        assert!(cmds.is_empty() && planned.is_empty());
    }
}

// ── Timeline clip editing (NLE parity QW-1 / QW-3 / QW-4) ────────────────────
//
// Delete/copy/cut/paste of the timeline selection + marker-at-playhead. Every
// mutation routes through `ops_bridge` (the sanctioned intent→ops→history
// bridge) so it lands as a real undoable timeline command; paste additionally
// uses `ops::insert_clip` directly (like `monitor.rs::ensure_timeline_project`)
// because there is no asset-agnostic "insert this exact clip" bridge helper and
// paste must preserve the copied clip's grade/effects/trim.
impl PhotonicApp {
    /// Resolve `timeline_selection` (clip ids) to concrete `(track, clip)` pairs
    /// on the active sequence, in timeline order.
    fn resolve_timeline_selection(
        &self,
        doc: &Document,
        seq_id: SequenceId,
    ) -> Vec<(TrackId, ClipId)> {
        match doc.timeline.as_ref().and_then(|p| p.sequences.get(&seq_id)) {
            Some(s) => resolve_selection_in(s, &self.timeline_selection),
            None => Vec::new(),
        }
    }

    /// Delete the timeline-selected clip(s) (`Delete`/`Backspace`, QW-1). With
    /// `ripple`, downstream clips on the same track shift left to close the gap
    /// (`Shift+Delete`); otherwise the clips are lifted and leave a gap. No-op on
    /// an empty selection or missing timeline.
    pub(crate) fn timeline_delete_selection(
        &mut self,
        doc: &mut Document,
        history: &mut CommandHistory,
        ripple: bool,
    ) {
        let Some(seq_id) = doc.timeline.as_ref().and_then(|p| p.active_sequence) else {
            return;
        };
        let targets = self.resolve_timeline_selection(doc, seq_id);
        if targets.is_empty() {
            return;
        }
        for (track, clip) in &targets {
            if ripple {
                ops_bridge::ripple_delete(doc, history, seq_id, *track, *clip);
            } else {
                ops_bridge::remove_clip(doc, history, seq_id, *track, *clip);
            }
        }
        // Drop the now-gone clips from the selection.
        self.timeline_selection
            .retain(|id| !targets.iter().any(|(_, c)| c == id));
    }

    /// Copy the timeline-selected clip(s) into `timeline_clipboard` (`Ctrl+C`,
    /// QW-3), full-cloned with their source track kind. Returns `true` if
    /// anything was copied.
    pub(crate) fn timeline_copy_selection(&mut self, doc: &Document) -> bool {
        let Some(seq_id) = doc.timeline.as_ref().and_then(|p| p.active_sequence) else {
            return false;
        };
        let Some(s) = doc.timeline.as_ref().and_then(|p| p.sequences.get(&seq_id)) else {
            return false;
        };
        let mut buf: Vec<ClipboardClip> = Vec::new();
        for t in s.tracks() {
            for c in &t.clips {
                if self.timeline_selection.contains(&c.id) {
                    buf.push(ClipboardClip {
                        clip: c.clone(),
                        kind: t.kind,
                    });
                }
            }
        }
        if buf.is_empty() {
            return false;
        }
        self.timeline_clipboard = buf;
        true
    }

    /// Cut = copy + ripple-delete (`Ctrl+X`, QW-3). No-op if the selection is
    /// empty (nothing copied → nothing deleted).
    pub(crate) fn timeline_cut_selection(
        &mut self,
        doc: &mut Document,
        history: &mut CommandHistory,
    ) {
        if self.timeline_copy_selection(doc) {
            self.timeline_delete_selection(doc, history, true);
        }
    }

    /// Paste the clipboard at the playhead (`Ctrl+V`, QW-3): each clip lands on
    /// the patched compatible-kind track when available, else the first one
    /// with room, at `playhead + (its source start − earliest source start)` so
    /// a multi-clip paste keeps its spacing. Committed as one undo step; the
    /// pasted clips become the new selection. Returns `true` if at least one
    /// clip was inserted.
    pub(crate) fn timeline_paste_clipboard(
        &mut self,
        doc: &mut Document,
        history: &mut CommandHistory,
    ) -> bool {
        if self.timeline_clipboard.is_empty() {
            return false;
        }
        let Some(seq_id) = doc.timeline.as_ref().and_then(|p| p.active_sequence) else {
            return false;
        };
        let anchor = self
            .timeline_clipboard
            .iter()
            .map(|e| e.clip.start)
            .min()
            .unwrap_or(Tick::ZERO);
        let playhead = self.playhead;

        // Candidate tracks, in timeline order.
        let cand: Vec<PasteTrack> = {
            let Some(s) = doc.timeline.as_ref().and_then(|p| p.sequences.get(&seq_id)) else {
                return false;
            };
            s.tracks()
                .map(|t| PasteTrack {
                    id: t.id,
                    kind: t.kind,
                    enabled: t.enabled,
                    locked: t.locked,
                    spans: t.clips.iter().map(|c| (c.start, c.end())).collect(),
                })
                .collect()
        };

        let entries: Vec<PasteEntry> = self
            .timeline_clipboard
            .iter()
            .map(|e| (e.kind, e.clip.start, e.clip.duration))
            .collect();
        let placements = plan_paste_placements(
            &entries,
            playhead,
            anchor,
            &cand,
            self.target_video_track,
            self.target_audio_track,
        );
        if placements.is_empty() {
            return false;
        }

        let mut cmds: Vec<Command> = Vec::new();
        let mut new_sel: Vec<ClipId> = Vec::new();
        {
            let Some(p) = doc.timeline.as_ref() else {
                return false;
            };
            for (ei, ti, start) in &placements {
                let track_id = cand[*ti].id;
                let mut clip = self.timeline_clipboard[*ei].clip.clone();
                clip.id = ClipId::new();
                clip.start = *start;
                let new_id = clip.id;
                if let Ok(cmd) = ops::insert_clip(p, seq_id, track_id, clip) {
                    cmds.push(Command::Timeline(cmd));
                    new_sel.push(new_id);
                }
            }
        }
        if cmds.is_empty() {
            return false;
        }
        history.execute_discrete(Command::Batch(cmds), doc);
        self.timeline_selection = new_sel;
        true
    }

    /// Add a marker at the playhead on the active sequence (`M`, QW-4).
    pub(crate) fn timeline_add_marker_at_playhead(
        &mut self,
        doc: &mut Document,
        history: &mut CommandHistory,
    ) {
        let Some(seq_id) = doc.timeline.as_ref().and_then(|p| p.active_sequence) else {
            return;
        };
        ops_bridge::add_marker(doc, history, seq_id, self.playhead, "Marker");
    }

    // ── 3/4-point source editing (spec 16) ───────────────────────────────────

    /// Arm `pending_source` from the first selected timeline clip (spec 16 §4
    /// minimal arming — no source monitor yet: selecting a clip makes it the
    /// source). Returns the source now in effect — the freshly-armed selection
    /// when one exists, else whatever was previously armed.
    fn arm_pending_source(&mut self, doc: &Document) -> Option<interact::PendingSource> {
        if let Some(seq) = doc
            .timeline
            .as_ref()
            .and_then(|p| p.active_sequence)
            .and_then(|id| doc.timeline.as_ref().and_then(|p| p.sequences.get(&id)))
        {
            if let Some(ps) = interact::pending_source_from_selection(seq, &self.timeline_selection)
            {
                self.pending_source = Some(ps);
            }
        }
        self.pending_source.clone()
    }

    /// The session source-patch target for `kind` (spec 16 §1 M-3).
    fn target_track_for(&self, kind: TrackKind) -> Option<TrackId> {
        match kind {
            TrackKind::Video | TrackKind::Text => self.target_video_track,
            TrackKind::Audio => self.target_audio_track,
        }
    }

    /// Shared Insert/Overwrite driver: arm the source, resolve the patch track
    /// for its kind, and lay it down at the playhead. `insert` selects ripple
    /// (Insert `,`) over replace-in-place (Overwrite `.`). No-op without an
    /// armed source or a target track of the source's kind.
    fn timeline_source_edit(
        &mut self,
        doc: &mut Document,
        history: &mut CommandHistory,
        insert: bool,
    ) {
        let Some(seq_id) = doc.timeline.as_ref().and_then(|p| p.active_sequence) else {
            return;
        };
        let Some(source) = self.arm_pending_source(doc) else {
            return;
        };
        let explicit = self.target_track_for(source.kind);
        let Some(track) = doc
            .timeline
            .as_ref()
            .and_then(|p| p.sequences.get(&seq_id))
            .and_then(|s| interact::resolve_target_track(s, source.kind, explicit))
        else {
            return;
        };
        let at = self.playhead;
        let clip = source.to_clip(at);
        if insert {
            interact::do_insert_edit(doc, history, seq_id, track, at, clip);
        } else {
            interact::do_overwrite_edit(doc, history, seq_id, track, at, clip);
        }
    }

    /// Insert the armed source at the playhead, rippling downstream right (`,`).
    pub(crate) fn timeline_insert_edit(
        &mut self,
        doc: &mut Document,
        history: &mut CommandHistory,
    ) {
        self.timeline_source_edit(doc, history, true);
    }

    /// Overwrite at the playhead with the armed source, no ripple (`.`).
    pub(crate) fn timeline_overwrite_edit(
        &mut self,
        doc: &mut Document,
        history: &mut CommandHistory,
    ) {
        self.timeline_source_edit(doc, history, false);
    }

    /// Shared Lift/Extract driver over the timeline in/out (`work_range`) on the
    /// video patch track. `ripple` closes the gap (Extract `'`) over leaving it
    /// (Lift `;`). No-op without a timeline in/out or a video track.
    fn timeline_range_edit(
        &mut self,
        doc: &mut Document,
        history: &mut CommandHistory,
        ripple: bool,
    ) {
        let Some(seq_id) = doc.timeline.as_ref().and_then(|p| p.active_sequence) else {
            return;
        };
        let Some((range, track)) = doc
            .timeline
            .as_ref()
            .and_then(|p| p.sequences.get(&seq_id))
            .and_then(|s| {
                let range = s.work_range?;
                let track =
                    interact::resolve_target_track(s, TrackKind::Video, self.target_video_track)?;
                Some((range, track))
            })
        else {
            return;
        };
        if ripple {
            interact::do_extract_edit(doc, history, seq_id, track, range);
        } else {
            interact::do_lift_edit(doc, history, seq_id, track, range);
        }
    }

    /// Lift the timeline in/out on the video patch track, leaving a gap (`;`).
    pub(crate) fn timeline_lift_edit(&mut self, doc: &mut Document, history: &mut CommandHistory) {
        self.timeline_range_edit(doc, history, false);
    }

    /// Extract the timeline in/out on the video patch track, closing the gap (`'`).
    pub(crate) fn timeline_extract_edit(
        &mut self,
        doc: &mut Document,
        history: &mut CommandHistory,
    ) {
        self.timeline_range_edit(doc, history, true);
    }

    /// Toggle the razor/blade tool (spec 16 §4 M-4, `C`). The lane-click split it
    /// arms is applied by the timeline-panel story's `self_interact` (its
    /// territory) — this owns the mode bit and its keybinding.
    pub(crate) fn timeline_toggle_razor(&mut self) {
        self.timeline_razor_active = !self.timeline_razor_active;
    }

    // ── NLE parity round-2 (spec 17 G1/G2/G3) ────────────────────────────────
    //
    // Keyboard-velocity editing riding on the shipped split/trim/roll ops. The
    // per-frame poll is `handle_timeline_shortcuts` (called from
    // `draw_timeline_panel`); every mutation batches through `ops_bridge` as a
    // single undo step and no-ops cleanly when nothing applies.

    /// **Add Edit to All Tracks** (G1, Premiere Ctrl+Shift+K): split every
    /// unlocked track's clip under the playhead in one undo step.
    pub(crate) fn timeline_split_all_tracks(
        &mut self,
        doc: &mut Document,
        history: &mut CommandHistory,
    ) {
        let Some(seq_id) = doc.timeline.as_ref().and_then(|p| p.active_sequence) else {
            return;
        };
        ops_bridge::split_all_tracks(doc, history, seq_id, self.playhead);
    }

    /// **Close Gap at Playhead** (G1): on every unlocked track, close the gap the
    /// playhead sits in, all in one undo step.
    pub(crate) fn timeline_close_gap_at_playhead(
        &mut self,
        doc: &mut Document,
        history: &mut CommandHistory,
    ) {
        let Some(seq_id) = doc.timeline.as_ref().and_then(|p| p.active_sequence) else {
            return;
        };
        ops_bridge::close_gaps_at_playhead(doc, history, seq_id, self.playhead);
    }

    /// **Close All Gaps** (G1): repack every unlocked track left-contiguous
    /// (keeping each track's first clip position), one undo step.
    pub(crate) fn timeline_close_all_gaps(
        &mut self,
        doc: &mut Document,
        history: &mut CommandHistory,
    ) {
        let Some(seq_id) = doc.timeline.as_ref().and_then(|p| p.active_sequence) else {
            return;
        };
        ops_bridge::close_all_gaps(doc, history, seq_id);
    }

    /// **Simplify Sequence** (G1): merge back through-edits — adjacent clips that
    /// are the same source played continuously (a split with no change) — across
    /// every unlocked track, one undo step.
    pub(crate) fn timeline_simplify_sequence(
        &mut self,
        doc: &mut Document,
        history: &mut CommandHistory,
    ) {
        let Some(seq_id) = doc.timeline.as_ref().and_then(|p| p.active_sequence) else {
            return;
        };
        ops_bridge::simplify_sequence(doc, history, seq_id);
    }

    /// **Ripple-trim to playhead** (G2, Premiere Q/W): trim the target clip's
    /// start (`start_edge`, Q) or end (W) to the playhead, rippling downstream
    /// clips to close the gap. The target is the selected clip the playhead is
    /// strictly inside, else any clip it is inside (locked tracks skipped). No-op
    /// when the playhead is on an edge or in a gap.
    pub(crate) fn timeline_trim_to_playhead(
        &mut self,
        doc: &mut Document,
        history: &mut CommandHistory,
        start_edge: bool,
    ) {
        let Some(seq_id) = doc.timeline.as_ref().and_then(|p| p.active_sequence) else {
            return;
        };
        let at = self.playhead;
        // Carry the source-tick advance for a head trim as a plain `Tick` (never
        // the `SpeedMap` itself — a concurrent speed-ramp story makes it non-Copy).
        let Some((track, clip, start, duration, source_in, head_advance)) = doc
            .timeline
            .as_ref()
            .and_then(|p| p.sequences.get(&seq_id))
            .and_then(|s| {
                let (track, clip) = interact::trim_target_at(s, &self.timeline_selection, at)?;
                let c = s.track(track)?.clips.iter().find(|c| c.id == clip)?;
                let head_advance = c.speed.source_delta(at - c.start);
                Some((track, clip, c.start, c.duration, c.source_in, head_advance))
            })
        else {
            return;
        };
        // Playhead must be strictly interior for either edge to make sense.
        if !(start < at && at < start + duration) {
            return;
        }
        let new = if start_edge {
            // Q: remove [start, playhead) from the head — keep the timeline start,
            // advance source_in, shrink duration; downstream ripples left.
            let delta = at - start;
            let new_source_in = source_in + head_advance;
            if new_source_in.0 < 0 {
                return;
            }
            ClipTiming {
                start,
                duration: duration - delta,
                source_in: new_source_in,
            }
        } else {
            // W: move the out-point to the playhead; downstream ripples left.
            ClipTiming {
                start,
                duration: at - start,
                source_in,
            }
        };
        if new.duration.0 <= 0 {
            return;
        }
        ops_bridge::ripple_trim(doc, history, seq_id, track, clip, new);
    }

    /// **Extend Edit to Playhead** (G2, Premiere E): extend the selected clip's
    /// out-point to the playhead (a plain trim, no ripple), clamped to the next
    /// clip's start so it never overlaps. No-op with no selection or when the
    /// playhead is not to the right of the clip's start.
    pub(crate) fn timeline_extend_edit_to_playhead(
        &mut self,
        doc: &mut Document,
        history: &mut CommandHistory,
    ) {
        let Some(seq_id) = doc.timeline.as_ref().and_then(|p| p.active_sequence) else {
            return;
        };
        let at = self.playhead;
        let Some((track, clip, start, source_in, next_start)) = doc
            .timeline
            .as_ref()
            .and_then(|p| p.sequences.get(&seq_id))
            .and_then(|s| {
                let (track, clip) = interact::first_selected(s, &self.timeline_selection)?;
                let t = s.track(track)?;
                let c = t.clips.iter().find(|c| c.id == clip)?;
                let end = c.end();
                let next_start = t
                    .clips
                    .iter()
                    .filter(|o| o.id != clip && o.start >= end)
                    .map(|o| o.start)
                    .min();
                Some((track, clip, c.start, c.source_in, next_start))
            })
        else {
            return;
        };
        let mut new_end = at;
        if let Some(ns) = next_start {
            new_end = new_end.min(ns);
        }
        if new_end <= start {
            return;
        }
        let new = ClipTiming {
            start,
            duration: new_end - start,
            source_in,
        };
        ops_bridge::trim(doc, history, seq_id, track, clip, new);
    }

    /// **Roll edit to playhead** (G2, Premiere Shift+Q / Shift+W): roll the cut
    /// immediately before (`prev`) / after the playhead on the target track to
    /// the playhead. Target track = the selected/under-playhead clip's track,
    /// else the first unlocked track with a cut. No-op when no such cut exists or
    /// the roll would collapse a clip (rejected by `ops::roll_edit`).
    pub(crate) fn timeline_roll_to_playhead(
        &mut self,
        doc: &mut Document,
        history: &mut CommandHistory,
        prev: bool,
    ) {
        let Some(seq_id) = doc.timeline.as_ref().and_then(|p| p.active_sequence) else {
            return;
        };
        let at = self.playhead;
        let Some((track, left, right, delta)) = doc
            .timeline
            .as_ref()
            .and_then(|p| p.sequences.get(&seq_id))
            .and_then(|s| {
                let track = interact::roll_target_track(s, &self.timeline_selection, at)?;
                let t = s.track(track)?;
                // The flush-adjacent cut nearest before/after the playhead.
                let mut best: Option<(Tick, ClipId, ClipId)> = None;
                for w in t.clips.windows(2) {
                    let (l, r) = (&w[0], &w[1]);
                    if l.end() != r.start {
                        continue; // a gap, not a cut
                    }
                    let b = l.end();
                    let take = if prev { b < at } else { b > at };
                    if !take {
                        continue;
                    }
                    let better = match best {
                        None => true,
                        Some((bb, _, _)) => {
                            if prev {
                                b > bb
                            } else {
                                b < bb
                            }
                        }
                    };
                    if better {
                        best = Some((b, l.id, r.id));
                    }
                }
                let (b, l, r) = best?;
                Some((track, l, r, at - b))
            })
        else {
            return;
        };
        if delta.0 == 0 {
            return;
        }
        ops_bridge::roll(doc, history, seq_id, track, left, right, delta);
    }

    /// **Match Frame** (G3, Premiere F): from the clip under the playhead, arm
    /// its source at the matching source tick (feeds Insert/Overwrite/Replace).
    /// Reports the armed source tick via tracing (no source monitor yet — G10).
    pub(crate) fn timeline_match_frame(&mut self, doc: &Document) {
        let Some(seq_id) = doc.timeline.as_ref().and_then(|p| p.active_sequence) else {
            return;
        };
        let at = self.playhead;
        let Some(ps) = doc
            .timeline
            .as_ref()
            .and_then(|p| p.sequences.get(&seq_id))
            .and_then(|s| {
                let (track, clip) = interact::clip_at_playhead(s, &self.timeline_selection, at)?;
                let t = s.track(track)?;
                let c = t.clips.iter().find(|c| c.id == clip)?;
                let matched = c.source_in + c.speed.source_delta(at - c.start);
                let source_end = c.source_in + c.duration;
                let src_out = if source_end > matched {
                    source_end
                } else {
                    matched + c.duration
                };
                Some(interact::PendingSource {
                    source: c.source.clone(),
                    src_in: matched,
                    src_out,
                    name: c.name.clone(),
                    kind: t.kind,
                })
            })
        else {
            return;
        };
        tracing::info!(
            "Match Frame: armed source \"{}\" at source tick {}",
            ps.name,
            ps.src_in.0
        );
        self.pending_source = Some(ps);
    }

    /// **Reveal in Media Pool** (G3): select the source asset of the clip under
    /// the playhead in the media pool. No-op for generator clips (no asset).
    pub(crate) fn timeline_reveal_in_project(&mut self, doc: &Document) {
        let Some(seq_id) = doc.timeline.as_ref().and_then(|p| p.active_sequence) else {
            return;
        };
        let at = self.playhead;
        let asset = doc
            .timeline
            .as_ref()
            .and_then(|p| p.sequences.get(&seq_id))
            .and_then(|s| {
                let (track, clip) = interact::clip_at_playhead(s, &self.timeline_selection, at)?;
                s.track(track)?
                    .clips
                    .iter()
                    .find(|c| c.id == clip)?
                    .source
                    .asset()
            });
        if let Some(asset) = asset {
            self.media_pool_ui.selected = Some(asset);
        }
    }

    /// Per-frame poll for the timeline-panel keyboard commands (spec 17 G1/G2/G3),
    /// called from `draw_timeline_panel`. The monitor's `handle_video_keyboard`
    /// owns the transport / 3-4-point keys; these editing keys are owned here so
    /// no out-of-territory file changes are needed. Gated on
    /// `!wants_keyboard_input` so a key never fires while a rename field has
    /// focus, and each id routes through the shared keymap so a user rebind
    /// applies.
    pub(crate) fn handle_timeline_shortcuts(
        &mut self,
        ctx: &egui::Context,
        doc: &mut Document,
        history: &mut CommandHistory,
    ) {
        if ctx.wants_keyboard_input() {
            return;
        }
        const KEYS: &[CommandId] = &[
            "video.split_all_tracks",
            "video.close_gap",
            "video.close_gaps",
            "video.simplify_sequence",
            "video.trim_start_to_playhead",
            "video.trim_end_to_playhead",
            "video.extend_edit",
            "video.roll_prev_to_playhead",
            "video.roll_next_to_playhead",
            "video.match_frame",
            "video.reveal_in_project",
        ];
        for &id in KEYS {
            if self.binding_pressed(ctx, id) {
                self.dispatch_command(id, doc, history);
            }
        }
    }
}

/// Resolve selected clip ids to `(track, clip)` pairs on `seq`, in timeline
/// order (video lanes then audio, each in clip order). Pure core of
/// [`PhotonicApp::resolve_timeline_selection`], split out for testing.
fn resolve_selection_in(seq: &Sequence, selection: &[ClipId]) -> Vec<(TrackId, ClipId)> {
    let mut out = Vec::new();
    for t in seq.tracks() {
        for c in &t.clips {
            if selection.contains(&c.id) {
                out.push((t.id, c.id));
            }
        }
    }
    out
}

/// True if the half-open spans `[a0, a1)` and `[b0, b1)` overlap.
fn spans_overlap(a0: Tick, a1: Tick, b0: Tick, b1: Tick) -> bool {
    a0 < b1 && b0 < a1
}

/// Plan where clipboard clips land on paste. Each entry's new start is
/// `playhead + (src_start − anchor)` (source-relative, clamped ≥ 0), so a
/// multi-clip paste keeps its spacing and the earliest clip lands on the
/// playhead. For each entry in order, the first track of a matching kind with
/// room — no overlap against existing spans or already-placed pastes — is
/// chosen. A matching, enabled, unlocked source-patch target gets first refusal;
/// if it cannot take the entry, the existing timeline-order fallback is used
/// without retrying that blocked target. Entries that fit nowhere are skipped.
/// Returns `(entry index, track index into `tracks`, new start)`.
fn plan_paste_placements(
    entries: &[PasteEntry],
    playhead: Tick,
    anchor: Tick,
    tracks: &[PasteTrack],
    target_video: Option<TrackId>,
    target_audio: Option<TrackId>,
) -> Vec<Placement> {
    let mut occ: Vec<Spans> = tracks.iter().map(|track| track.spans.clone()).collect();
    let mut out: Vec<Placement> = Vec::new();
    for (ei, (kind, src_start, dur)) in entries.iter().enumerate() {
        let raw = playhead + (*src_start - anchor);
        let start = if raw.0 < 0 { Tick::ZERO } else { raw };
        let end = start + *dur;
        let explicit = match kind {
            TrackKind::Video | TrackKind::Text => target_video,
            TrackKind::Audio => target_audio,
        };
        let explicit_index = explicit.and_then(|id| tracks.iter().position(|track| track.id == id));
        let preferred = explicit_index.filter(|&ti| {
            let track = &tracks[ti];
            track.kind == *kind
                && track.enabled
                && !track.locked
                && occ[ti]
                    .iter()
                    .all(|(s0, s1)| !spans_overlap(start, end, *s0, *s1))
        });
        if let Some(ti) = preferred {
            occ[ti].push((start, end));
            out.push((ei, ti, start));
            continue;
        }

        for (ti, track) in tracks.iter().enumerate() {
            if Some(ti) == explicit_index {
                continue;
            }
            if track.kind != *kind || !track.enabled || track.locked {
                continue;
            }
            let fits = occ[ti]
                .iter()
                .all(|(s0, s1)| !spans_overlap(start, end, *s0, *s1));
            if fits {
                occ[ti].push((start, end));
                out.push((ei, ti, start));
                break;
            }
        }
    }
    out
}

#[cfg(test)]
mod clip_edit_tests {
    use super::*;
    use photonic_core::timeline::{ClipSource, FrameRate, TimelineProject, Track};

    fn paste_track(kind: TrackKind, spans: Spans) -> PasteTrack {
        PasteTrack {
            id: TrackId::new(),
            kind,
            enabled: true,
            locked: false,
            spans,
        }
    }

    fn clip_at(start: i64, dur: i64) -> Clip {
        Clip::new(ClipSource::Adjustment, Tick(start), Tick(dur))
    }

    fn seq_with_clips() -> (Sequence, TrackId, TrackId, Vec<ClipId>) {
        let mut seq = Sequence::new("s", FrameRate::FPS_30, 1920, 1080);
        let mut v = Track::new(TrackKind::Video, "V1");
        let mut a = Track::new(TrackKind::Audio, "A1");
        let vc0 = clip_at(0, 100);
        let vc1 = clip_at(200, 100);
        let ac0 = clip_at(50, 100);
        let ids = vec![vc0.id, vc1.id, ac0.id];
        v.clips.push(vc0);
        v.clips.push(vc1);
        a.clips.push(ac0);
        let (vid, aid) = (v.id, a.id);
        seq.video_tracks.push(v);
        seq.audio_tracks.push(a);
        (seq, vid, aid, ids)
    }

    #[test]
    fn resolve_selection_maps_ids_to_track_clip_pairs_in_order() {
        let (seq, vid, aid, ids) = seq_with_clips();
        // Select the second video clip and the audio clip.
        let sel = vec![ids[2], ids[1]];
        let res = resolve_selection_in(&seq, &sel);
        // Video lanes come before audio; within a lane, clip order is preserved.
        assert_eq!(res, vec![(vid, ids[1]), (aid, ids[2])]);
    }

    #[test]
    fn resolve_selection_ignores_unknown_ids_and_empty_selection() {
        let (seq, _vid, _aid, _ids) = seq_with_clips();
        assert!(resolve_selection_in(&seq, &[]).is_empty());
        assert!(resolve_selection_in(&seq, &[ClipId::new()]).is_empty());
    }

    #[test]
    fn paste_single_clip_lands_at_playhead() {
        // One video clip copied from start=200; anchor is its own start, so it
        // pastes exactly at the playhead regardless of its original position.
        let entries = [(TrackKind::Video, Tick(200), Tick(100))];
        let tracks = [paste_track(TrackKind::Video, Vec::new())];
        let out = plan_paste_placements(&entries, Tick(1000), Tick(200), &tracks, None, None);
        assert_eq!(out, vec![(0, 0, Tick(1000))]);
    }

    #[test]
    fn paste_preserves_relative_spacing_across_clips() {
        // Two clips at 200 and 500 (gap 300) paste at playhead 1000 keeping the
        // gap: first at 1000, second at 1300. Anchor = earliest start (200).
        let entries = [
            (TrackKind::Video, Tick(200), Tick(100)),
            (TrackKind::Video, Tick(500), Tick(100)),
        ];
        let tracks = [paste_track(TrackKind::Video, Vec::new())];
        let out = plan_paste_placements(&entries, Tick(1000), Tick(200), &tracks, None, None);
        assert_eq!(out, vec![(0, 0, Tick(1000)), (1, 0, Tick(1300))]);
    }

    #[test]
    fn paste_skips_kind_mismatch_and_finds_next_fitting_track() {
        // An audio entry must skip the video track and land on the audio one.
        let entries = [(TrackKind::Audio, Tick(0), Tick(100))];
        let tracks = [
            paste_track(TrackKind::Video, Vec::new()),
            paste_track(TrackKind::Audio, Vec::new()),
        ];
        let out = plan_paste_placements(&entries, Tick(500), Tick(0), &tracks, None, None);
        assert_eq!(out, vec![(0, 1, Tick(500))]);
    }

    #[test]
    fn paste_overflows_to_second_track_when_first_is_occupied() {
        // Playhead lands the clip over an existing span on V1 → it goes to V2.
        let entries = [(TrackKind::Video, Tick(0), Tick(100))];
        let tracks = [
            paste_track(TrackKind::Video, vec![(Tick(450), Tick(600))]),
            paste_track(TrackKind::Video, Vec::new()),
        ];
        let out = plan_paste_placements(&entries, Tick(500), Tick(0), &tracks, None, None);
        assert_eq!(out, vec![(0, 1, Tick(500))]);
    }

    #[test]
    fn paste_is_skipped_when_no_track_has_room() {
        let entries = [(TrackKind::Video, Tick(0), Tick(100))];
        // The only video track is fully blocked across the paste span.
        let tracks = [paste_track(TrackKind::Video, vec![(Tick(400), Tick(700))])];
        let out = plan_paste_placements(&entries, Tick(500), Tick(0), &tracks, None, None);
        assert!(out.is_empty());
    }

    #[test]
    fn paste_explicit_video_target_wins() {
        let entries = [(TrackKind::Video, Tick(0), Tick(100))];
        let tracks = [
            paste_track(TrackKind::Video, Vec::new()),
            paste_track(TrackKind::Video, Vec::new()),
        ];
        let out = plan_paste_placements(
            &entries,
            Tick(500),
            Tick(0),
            &tracks,
            Some(tracks[1].id),
            None,
        );
        assert_eq!(out, vec![(0, 1, Tick(500))]);
    }

    #[test]
    fn paste_explicit_target_falls_back_after_planned_occupancy() {
        let entries = [
            (TrackKind::Video, Tick(0), Tick(100)),
            (TrackKind::Video, Tick(0), Tick(100)),
        ];
        let tracks = [
            paste_track(TrackKind::Video, Vec::new()),
            paste_track(TrackKind::Video, Vec::new()),
        ];
        let out = plan_paste_placements(
            &entries,
            Tick(500),
            Tick(0),
            &tracks,
            Some(tracks[1].id),
            None,
        );
        assert_eq!(out, vec![(0, 1, Tick(500)), (1, 0, Tick(500))]);
    }

    #[test]
    fn paste_explicit_audio_target_wins() {
        let entries = [(TrackKind::Audio, Tick(0), Tick(100))];
        let tracks = [
            paste_track(TrackKind::Audio, Vec::new()),
            paste_track(TrackKind::Audio, Vec::new()),
        ];
        let out = plan_paste_placements(
            &entries,
            Tick(500),
            Tick(0),
            &tracks,
            None,
            Some(tracks[1].id),
        );
        assert_eq!(out, vec![(0, 1, Tick(500))]);
    }

    #[test]
    fn paste_missing_wrong_or_blocked_target_falls_back() {
        let entries = [(TrackKind::Video, Tick(0), Tick(100))];
        let mut tracks = [
            paste_track(TrackKind::Video, Vec::new()),
            paste_track(TrackKind::Video, Vec::new()),
            paste_track(TrackKind::Audio, Vec::new()),
        ];

        for target in [Some(TrackId::new()), Some(tracks[2].id)] {
            let out = plan_paste_placements(&entries, Tick(500), Tick(0), &tracks, target, None);
            assert_eq!(out, vec![(0, 0, Tick(500))]);
        }

        tracks[0].enabled = false;
        let out = plan_paste_placements(
            &entries,
            Tick(500),
            Tick(0),
            &tracks,
            Some(tracks[0].id),
            None,
        );
        assert_eq!(out, vec![(0, 1, Tick(500))]);

        tracks[0].enabled = true;
        tracks[0].locked = true;
        let out = plan_paste_placements(
            &entries,
            Tick(500),
            Tick(0),
            &tracks,
            Some(tracks[0].id),
            None,
        );
        assert_eq!(out, vec![(0, 1, Tick(500))]);

        tracks[0].locked = false;
        tracks[0].spans.push((Tick(450), Tick(600)));
        let out = plan_paste_placements(
            &entries,
            Tick(500),
            Tick(0),
            &tracks,
            Some(tracks[0].id),
            None,
        );
        assert_eq!(out, vec![(0, 1, Tick(500))]);
    }

    #[test]
    fn paste_automatic_fallback_skips_disabled_and_locked_tracks() {
        let entries = [(TrackKind::Video, Tick(0), Tick(100))];
        let mut tracks = [
            paste_track(TrackKind::Video, Vec::new()),
            paste_track(TrackKind::Video, Vec::new()),
            paste_track(TrackKind::Video, Vec::new()),
        ];
        tracks[0].enabled = false;
        tracks[1].locked = true;

        let out = plan_paste_placements(&entries, Tick(500), Tick(0), &tracks, None, None);
        assert_eq!(out, vec![(0, 2, Tick(500))]);
    }

    #[test]
    fn paste_mixed_video_audio_routes_to_independent_targets() {
        let entries = [
            (TrackKind::Video, Tick(0), Tick(100)),
            (TrackKind::Audio, Tick(20), Tick(50)),
        ];
        let tracks = [
            paste_track(TrackKind::Video, Vec::new()),
            paste_track(TrackKind::Audio, Vec::new()),
            paste_track(TrackKind::Video, Vec::new()),
            paste_track(TrackKind::Audio, Vec::new()),
        ];
        let out = plan_paste_placements(
            &entries,
            Tick(500),
            Tick(0),
            &tracks,
            Some(tracks[2].id),
            Some(tracks[3].id),
        );
        assert_eq!(out, vec![(0, 2, Tick(500)), (1, 3, Tick(520))]);
    }

    #[test]
    fn timeline_paste_applies_targets_and_undoes_as_one_step() {
        let mut project = TimelineProject::new();
        let mut sequence = Sequence::new("s", FrameRate::FPS_30, 1920, 1080);
        let video_fallback = Track::new(TrackKind::Video, "V1");
        let video_target = Track::new(TrackKind::Video, "V2");
        let audio_fallback = Track::new(TrackKind::Audio, "A1");
        let audio_target = Track::new(TrackKind::Audio, "A2");
        let video_target_id = video_target.id;
        let audio_target_id = audio_target.id;
        sequence.video_tracks.extend([video_fallback, video_target]);
        sequence.audio_tracks.extend([audio_fallback, audio_target]);
        let sequence_id = project.insert_sequence(sequence);

        let mut doc = Document::new("paste", 1920.0, 1080.0);
        doc.timeline = Some(project);
        let mut history = CommandHistory::new(32);
        let mut app = PhotonicApp::default();
        app.playhead = Tick(1_000);
        app.target_video_track = Some(video_target_id);
        app.target_audio_track = Some(audio_target_id);
        app.timeline_clipboard = vec![
            ClipboardClip {
                clip: clip_at(100, 200),
                kind: TrackKind::Video,
            },
            ClipboardClip {
                clip: clip_at(125, 150),
                kind: TrackKind::Audio,
            },
        ];

        assert!(app.timeline_paste_clipboard(&mut doc, &mut history));
        let sequence = &doc.timeline.as_ref().unwrap().sequences[&sequence_id];
        let pasted_video = &sequence.track(video_target_id).unwrap().clips;
        let pasted_audio = &sequence.track(audio_target_id).unwrap().clips;
        assert_eq!(pasted_video.len(), 1);
        assert_eq!(pasted_audio.len(), 1);
        assert_eq!(pasted_video[0].start, Tick(1_000));
        assert_eq!(pasted_audio[0].start, Tick(1_025));
        assert_eq!(app.timeline_selection.len(), 2);
        assert_eq!(history.undo_depth(), 1);

        assert!(history.undo(&mut doc));
        let sequence = &doc.timeline.as_ref().unwrap().sequences[&sequence_id];
        assert!(sequence.track(video_target_id).unwrap().clips.is_empty());
        assert!(sequence.track(audio_target_id).unwrap().clips.is_empty());
        assert_eq!(history.undo_depth(), 0);
    }
}
