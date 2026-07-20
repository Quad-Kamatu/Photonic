use super::*;

/// Synthesize a short, stable, git-like commit id for a history entry so the
/// graph reads like a real commit log. Deterministic per (position, label).
fn short_hash(abs: usize, label: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    abs.hash(&mut h);
    label.hash(&mut h);
    format!("{:07x}", (h.finish() as u32) & 0x0fff_ffff)
}

/// VS Code–style **branching** commit graph of the edit history. The edit tree
/// is laid out into lanes: the trunk (root → HEAD) plus every divergent branch
/// created by editing after an undo. Each node is a clickable commit that jumps
/// the document to that point, crossing branches when needed.
pub(crate) fn draw_edit_history(ui: &mut Ui, ctx: &mut PropPanelCtx) {
    use std::collections::{HashMap, HashSet};
    let graph = ctx.history_graph;
    let current = ctx.history_current;
    let last_saved = ctx.history_last_saved;
    // The drawer search filters history entries (not property sections).
    let query = ctx.q.clone();
    let mut action: Option<PanelAction> = None;

    // Look-ups.
    let by_id: HashMap<u64, &photonic_core::history::HistoryGraphNode> =
        graph.iter().map(|n| (n.id, n)).collect();
    let cur_node = by_id.get(&current).copied();
    let edit_count = graph.len().saturating_sub(1); // minus the root

    // ── Header: title + total + quick undo/redo ──────────────────────────────
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("{}  History", ph::GIT_COMMIT)).strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                RichText::new(format!(
                    "{} edit{}",
                    edit_count,
                    if edit_count == 1 { "" } else { "s" }
                ))
                .weak()
                .small(),
            );
            ui.add_space(4.0);
            let redo_target = cur_node.and_then(|n| n.primary_child);
            if ui
                .add_enabled(
                    redo_target.is_some(),
                    egui::Button::new(ph::ARROW_ARC_RIGHT).small().frame(false),
                )
                .on_hover_text("Redo (step forward)")
                .clicked()
            {
                if let Some(id) = redo_target {
                    action = Some(PanelAction::JumpToHistoryNode { id });
                }
            }
            let undo_target = cur_node.and_then(|n| n.parent);
            if ui
                .add_enabled(
                    undo_target.is_some(),
                    egui::Button::new(ph::ARROW_ARC_LEFT).small().frame(false),
                )
                .on_hover_text("Undo (step back)")
                .clicked()
            {
                if let Some(id) = undo_target {
                    action = Some(PanelAction::JumpToHistoryNode { id });
                }
            }
        });
    });
    ui.add_space(6.0);

    // ── Name the current branch ───────────────────────────────────────────────
    // A branch is a name that rides with the tip: it advances onto each new commit
    // you make on this line. Naming and switching are non-destructive.
    ui.horizontal(|ui| {
        let hint = if cur_node.and_then(|n| n.label.as_ref()).is_some() {
            "Rename this branch…"
        } else {
            "Name this branch…"
        };
        ui.add(
            egui::TextEdit::singleline(&mut *ctx.branch_name_input)
                .hint_text(hint)
                .desired_width(ui.available_width() - 52.0),
        );
        let name = ctx.branch_name_input.trim().to_string();
        if ui
            .add_enabled(
                !name.is_empty(),
                egui::Button::new(ph::BOOKMARK_SIMPLE).small(),
            )
            .on_hover_text("Name this branch — the name follows your edits along this line")
            .clicked()
        {
            action = Some(PanelAction::BranchCreate { name });
            ctx.branch_name_input.clear();
        }
    });

    // ── Branches — quick, non-destructive switches (jump to the branch tip) ────
    let named: Vec<(&str, u64)> = graph
        .iter()
        .filter_map(|n| n.label.as_deref().map(|l| (l, n.id)))
        .collect();
    if !named.is_empty() {
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
            for (label, id) in &named {
                let is_here = *id == current;
                if ui
                    .selectable_label(is_here, format!("{} {label}", ph::BOOKMARK_SIMPLE))
                    .on_hover_text("Switch to this branch (jump to its tip)")
                    .clicked()
                {
                    action = Some(PanelAction::JumpToHistoryNode { id: *id });
                }
            }
        });
    }
    ui.add_space(6.0);

    if edit_count == 0 {
        ui.label(
            RichText::new("No edits yet — the graph fills in as you work.")
                .weak()
                .small(),
        );
        if action.is_some() {
            ctx.action = action;
        }
        return;
    }

    // ── Search mode: flat filtered list of matching commits ───────────────────
    // When the drawer search has a query, show a simple newest-first list of the
    // history entries whose description matches, instead of the lane graph — the
    // graph's shape isn't meaningful once nodes are filtered out.
    if !query.is_empty() {
        let mut any = false;
        egui::ScrollArea::vertical()
            .max_height(340.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for node in graph.iter() {
                    let hay = format!(
                        "{} {}",
                        node.description,
                        node.label.as_deref().unwrap_or("")
                    )
                    .to_lowercase();
                    if node.is_root || !hay.contains(&query) {
                        continue;
                    }
                    any = true;
                    let saved = last_saved == Some(node.id);
                    ui.horizontal(|ui| {
                        let mut lead = String::new();
                        if node.is_current {
                            lead.push_str(&format!("{} ", ph::GIT_COMMIT));
                        }
                        if saved {
                            lead.push_str(&format!("{} ", ph::FLOPPY_DISK));
                        }
                        if let Some(l) = &node.label {
                            lead.push_str(&format!("{} {l}  ", ph::BOOKMARK_SIMPLE));
                        }
                        if ui
                            .selectable_label(
                                node.is_current,
                                format!("{lead}{}", node.description),
                            )
                            .on_hover_text("Jump to this state")
                            .clicked()
                        {
                            action = Some(PanelAction::JumpToHistoryNode { id: node.id });
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(short_hash(node.id as usize, &node.description))
                                    .monospace()
                                    .weak()
                                    .small(),
                            );
                        });
                    });
                }
            });
        if !any {
            ui.label(
                RichText::new("No history entries match your search.")
                    .weak()
                    .small(),
            );
        }
        if action.is_some() {
            ctx.action = action;
        }
        ui.add_space(6.0);
        return;
    }

    // ── Lane assignment (git-graph style) ────────────────────────────────────
    // `graph` is newest-first (top → bottom). Each lane flows downward toward the
    // parent it is currently reserved for; when we reach that parent the reserved
    // lanes converge into its column and the column continues toward *its* parent.
    let index: HashMap<u64, usize> = graph.iter().enumerate().map(|(i, n)| (n.id, i)).collect();
    let mut lanes: Vec<Option<u64>> = Vec::new();
    let mut col: HashMap<u64, usize> = HashMap::new();
    for node in graph.iter() {
        let reserved: Vec<usize> = lanes
            .iter()
            .enumerate()
            .filter(|(_, l)| **l == Some(node.id))
            .map(|(i, _)| i)
            .collect();
        let c = if let Some(&m) = reserved.iter().min() {
            m
        } else if let Some(f) = lanes.iter().position(|l| l.is_none()) {
            f
        } else {
            lanes.push(None);
            lanes.len() - 1
        };
        col.insert(node.id, c);
        for &r in &reserved {
            if r != c {
                lanes[r] = None;
            }
        }
        lanes[c] = node.parent; // reserve this column for the parent (None frees it)
    }

    // Ancestors of HEAD (the trunk) render bright; branch nodes render dim.
    let mut onpath: HashSet<u64> = HashSet::new();
    {
        let mut id = Some(current);
        while let Some(i) = id {
            onpath.insert(i);
            id = by_id.get(&i).and_then(|n| n.parent);
        }
    }

    // ── Geometry & palette ───────────────────────────────────────────────────
    let lane_w = 14.0;
    let gutter = 10.0;
    let row_h = 26.0;
    let node_r = 4.5;
    let font = egui::TextStyle::Small.resolve(ui.style());
    let mono = egui::TextStyle::Monospace.resolve(ui.style());
    let panel_bg = ui.visuals().panel_fill;
    let lane_palette = [
        Color32::from_rgb(139, 116, 224),
        Color32::from_rgb(86, 172, 168),
        Color32::from_rgb(214, 170, 96),
        Color32::from_rgb(206, 122, 178),
        Color32::from_rgb(126, 176, 122),
    ];
    let dim = Color32::from_rgb(96, 98, 120);
    let text_bright = Color32::from_rgb(224, 228, 242);
    let text_head = Color32::from_rgb(240, 242, 250);
    let text_dim = Color32::from_rgb(126, 128, 152);
    let hash_col = Color32::from_rgb(102, 104, 132);
    let lane_color = |c: usize| lane_palette[c % lane_palette.len()];

    let n = graph.len();
    let (block, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), n as f32 * row_h),
        egui::Sense::hover(),
    );
    let painter = ui.painter_at(block);
    let left = block.left();
    let top = block.top();
    let x_of = |c: usize| left + gutter + c as f32 * lane_w;
    let y_of = |i: usize| top + i as f32 * row_h + row_h * 0.5;

    // Per-row rightmost occupied lane — each node's own column plus any edge that
    // runs vertically through that row. Labels anchor just past this, so each title
    // sits directly beside its own node instead of aligning to the single furthest
    // lane, while still clearing pass-through branch lines.
    let mut row_max_lane: Vec<usize> = graph
        .iter()
        .map(|node| *col.get(&node.id).unwrap_or(&0))
        .collect();
    for node in graph.iter() {
        let (Some(&i), Some(&c)) = (index.get(&node.id), col.get(&node.id)) else {
            continue;
        };
        let Some(p) = node.parent else { continue };
        let Some(&j) = index.get(&p) else { continue };
        let (lo, hi) = if i <= j { (i, j) } else { (j, i) };
        for slot in row_max_lane[lo..=hi].iter_mut() {
            *slot = (*slot).max(c);
        }
    }

    // Edges first (drawn under the nodes).
    for node in graph.iter() {
        let (Some(&i), Some(&c)) = (index.get(&node.id), col.get(&node.id)) else {
            continue;
        };
        let Some(p) = node.parent else { continue };
        let (Some(&j), Some(&pc)) = (index.get(&p), col.get(&p)) else {
            continue;
        };
        let bright = onpath.contains(&node.id);
        let stroke = egui::Stroke::new(1.7, if bright { lane_color(c) } else { dim });
        let (x1, x2) = (x_of(c), x_of(pc));
        let (yi, yj) = (y_of(i), y_of(j));
        if c == pc {
            painter.line_segment([egui::pos2(x1, yi), egui::pos2(x1, yj)], stroke);
        } else {
            // Run down this lane, then elbow across into the parent's lane just
            // above the parent node — reads as a branch merging back to trunk.
            let elbow_y = yj - row_h * 0.5;
            painter.line_segment([egui::pos2(x1, yi), egui::pos2(x1, elbow_y)], stroke);
            painter.line_segment([egui::pos2(x1, elbow_y), egui::pos2(x2, yj)], stroke);
        }
    }

    // Nodes + labels + interaction.
    for node in graph.iter() {
        let (Some(&i), Some(&c)) = (index.get(&node.id), col.get(&node.id)) else {
            continue;
        };
        let center = egui::pos2(x_of(c), y_of(i));
        let bright = onpath.contains(&node.id);
        let base = if bright { lane_color(c) } else { dim };

        let row_rect = egui::Rect::from_min_size(
            egui::pos2(left, y_of(i) - row_h * 0.5),
            egui::vec2(block.width(), row_h),
        );
        let resp = ui.interact(
            row_rect,
            ui.id().with(("hist_row", node.id)),
            egui::Sense::click(),
        );
        if node.is_current {
            painter.rect_filled(
                row_rect,
                egui::Rounding::same(4.0),
                Color32::from_rgba_unmultiplied(139, 116, 224, 30),
            );
        } else if resp.hovered() {
            painter.rect_filled(
                row_rect,
                egui::Rounding::same(4.0),
                Color32::from_rgba_unmultiplied(255, 255, 255, 12),
            );
        }

        // Marker: HEAD ring, root hollow, trunk filled, branch hollow-dim.
        painter.circle_filled(center, node_r + 2.0, panel_bg);
        if node.is_current {
            painter.circle_stroke(center, node_r + 1.0, egui::Stroke::new(2.0, base));
            painter.circle_filled(center, node_r - 1.5, Color32::WHITE);
        } else if node.is_root {
            painter.circle_stroke(center, node_r, egui::Stroke::new(1.4, base));
        } else if bright {
            painter.circle_filled(center, node_r, base);
        } else {
            painter.circle_stroke(center, node_r, egui::Stroke::new(1.4, base));
        }
        // "Last Save": teal outer ring on the node matching the on-disk file —
        // legible even when it coincides with the white HEAD marker.
        let saved_here = last_saved == Some(node.id);
        let save_col = Color32::from_rgb(88, 200, 160);
        if saved_here {
            painter.circle_stroke(center, node_r + 3.5, egui::Stroke::new(1.6, save_col));
        }

        // Right edge: HEAD pill or short commit id.
        let cy = center.y;
        let mut text_right = block.right() - 8.0;
        if node.is_current {
            let g = ui
                .painter()
                .layout_no_wrap("HEAD".to_string(), font.clone(), Color32::WHITE);
            let pad = egui::vec2(6.0, 2.0);
            let size = g.size() + pad * 2.0;
            let pr = egui::Rect::from_min_size(
                egui::pos2(block.right() - 8.0 - size.x, cy - size.y * 0.5),
                size,
            );
            painter.rect_filled(pr, egui::Rounding::same(3.0), lane_color(c));
            painter.galley(pr.min + pad, g, Color32::WHITE);
            text_right = pr.left() - 6.0;
        } else if !node.is_root {
            let g = ui.painter().layout_no_wrap(
                short_hash(node.id as usize, &node.description),
                mono.clone(),
                hash_col,
            );
            let pos = egui::pos2(block.right() - 8.0 - g.size().x, cy - g.size().y * 0.5);
            painter.galley(pos, g, hash_col);
            text_right = pos.x - 6.0;
        }

        // Commit message, ellipsized.
        let tcol = if node.is_current {
            text_head
        } else if bright {
            text_bright
        } else {
            text_dim
        };
        // A small floppy glyph precedes the message on the last-saved node.
        // Anchor the label just past this row's own graph column(s).
        let text_left = x_of(row_max_lane[i]) + node_r + 10.0;
        let mut msg_left = text_left;
        if saved_here {
            let fg =
                ui.painter()
                    .layout_no_wrap(ph::FLOPPY_DISK.to_string(), font.clone(), save_col);
            painter.galley(
                egui::pos2(text_left, cy - fg.size().y * 0.5),
                fg.clone(),
                save_col,
            );
            msg_left = text_left + fg.size().x + 4.0;
        }
        // Named states render a purple ref pill (like a git branch tag) before the
        // message, shifting it right.
        if let Some(l) = &node.label {
            let lab_col = Color32::from_rgb(150, 130, 235);
            let lg = ui.painter().layout_no_wrap(
                format!("{} {l}", ph::BOOKMARK_SIMPLE),
                font.clone(),
                Color32::WHITE,
            );
            let pad = egui::vec2(5.0, 1.5);
            let size = lg.size() + pad * 2.0;
            let pr = egui::Rect::from_min_size(egui::pos2(msg_left, cy - size.y * 0.5), size);
            painter.rect_filled(pr, egui::Rounding::same(3.0), lab_col);
            painter.galley(pr.min + pad, lg, Color32::WHITE);
            msg_left = pr.right() + 6.0;
        }
        let tw = (text_right - msg_left).max(10.0);
        let mut job = egui::text::LayoutJob::single_section(
            node.description.clone(),
            egui::TextFormat {
                font_id: font.clone(),
                color: tcol,
                ..Default::default()
            },
        );
        job.wrap.max_width = tw;
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        job.wrap.overflow_character = Some('…');
        let g = ui.fonts(|f| f.layout_job(job));
        painter.galley(egui::pos2(msg_left, cy - g.size().y * 0.5), g, tcol);

        let resp = resp.on_hover_text(if saved_here {
            "Last saved to disk here".to_string()
        } else if node.is_current {
            "Current state — right-click for options".to_string()
        } else {
            "Click to jump here · right-click for options".to_string()
        });
        // Left-click: jump straight to this commit (reversible — it's a tree, so
        // you can jump back). Doubles as click-to-preview per #174.
        if resp.clicked() {
            action = Some(PanelAction::JumpToHistoryNode { id: node.id });
        }
        // Right-click: jump + naming affordances. Branching is implicit in the
        // undo-tree — jumping to a commit and then editing forks a new branch —
        // and a "named branch" is just a label you attach to a commit here.
        resp.context_menu(|ui| {
            ui.label(
                RichText::new(node.description.clone())
                    .small()
                    .color(Color32::from_rgb(200, 204, 222)),
            );
            ui.separator();
            if !node.is_current
                && ui
                    .button("Jump to this state")
                    .on_hover_text("Move the document to this commit (reversible)")
                    .clicked()
            {
                action = Some(PanelAction::JumpToHistoryNode { id: node.id });
                ui.close_menu();
            }
            // ── Start / name a branch at this commit ──────────────────────
            if !node.is_root {
                let buf_id = ui.id().with(("hist_name", node.id));
                let mut buf = ui.data_mut(|d| {
                    d.get_temp::<String>(buf_id)
                        .unwrap_or_else(|| node.label.clone().unwrap_or_default())
                });
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut buf)
                            .hint_text("Name a branch here…")
                            .desired_width(150.0),
                    );
                    let can = !buf.trim().is_empty();
                    if ui
                        .add_enabled(can, egui::Button::new(ph::BOOKMARK_SIMPLE))
                        .on_hover_text("Name a branch at this commit (it follows future edits)")
                        .clicked()
                    {
                        action = Some(PanelAction::LabelHistoryNode {
                            id: node.id,
                            name: Some(buf.trim().to_string()),
                        });
                        ui.close_menu();
                    }
                });
                ui.data_mut(|d| d.insert_temp(buf_id, buf));
                if node.label.is_some()
                    && ui
                        .button("Delete branch name")
                        .on_hover_text("Remove this branch name (keeps the commit)")
                        .clicked()
                {
                    action = Some(PanelAction::LabelHistoryNode {
                        id: node.id,
                        name: None,
                    });
                    ui.close_menu();
                }
                ui.separator();
                if ui.button("Copy commit id").clicked() {
                    let id = short_hash(node.id as usize, &node.description);
                    ui.output_mut(|o| o.copied_text = id);
                    ui.close_menu();
                }
            }
        });
    }
    ui.add_space(6.0);

    if action.is_some() {
        ctx.action = action;
    }
}
