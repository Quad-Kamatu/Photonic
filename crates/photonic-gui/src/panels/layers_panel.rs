use super::*;

/// Draw the left layers panel. Returns an optional action triggered by context menus.
/// Recursively render one node row in the Layers tree. Groups get a disclosure
/// header that expands to their children at any depth; every row is selectable
/// (emits `SelectNode`) and carries the z-order / collect context menu.
fn draw_layer_node_row(
    ui: &mut Ui,
    doc: &Document,
    node_id: NodeId,
    selected_id: Option<NodeId>,
    action: &mut Option<PanelAction>,
) {
    let Some(node) = doc.nodes.get(&node_id) else {
        return;
    };
    let is_selected = selected_id == Some(node_id);

    let response = match &node.kind {
        SceneNodeKind::Group(g) => {
            let label = RichText::new(node.name.clone()).color(if is_selected {
                Color32::from_rgb(184, 164, 255)
            } else {
                Color32::from_rgb(144, 119, 224)
            });
            let header = egui::CollapsingHeader::new(label)
                .id_salt(node_id)
                .default_open(true)
                .show(ui, |ui| {
                    let children: Vec<NodeId> = g.children.iter().rev().copied().collect();
                    if children.is_empty() {
                        ui.label(RichText::new("(empty group)").weak());
                    }
                    for child_id in children {
                        draw_layer_node_row(ui, doc, child_id, selected_id, action);
                    }
                });
            let hr = header.header_response;
            if hr.clicked() {
                *action = Some(PanelAction::SelectNode { node_id });
            }
            hr
        }
        _ => {
            let resp = ui.selectable_label(is_selected, format!("• {}", node.name));
            if resp.clicked() {
                *action = Some(PanelAction::SelectNode { node_id });
            }
            resp
        }
    };

    response.context_menu(|ui| {
        if ui.button("Bring to Front").clicked() {
            *action = Some(PanelAction::ReorderNode {
                node_id,
                op: ZOrderOp::BringToFront,
            });
            ui.close_menu();
        }
        if ui.button("Bring Forward").clicked() {
            *action = Some(PanelAction::ReorderNode {
                node_id,
                op: ZOrderOp::BringForward,
            });
            ui.close_menu();
        }
        if ui.button("Send Backward").clicked() {
            *action = Some(PanelAction::ReorderNode {
                node_id,
                op: ZOrderOp::SendBackward,
            });
            ui.close_menu();
        }
        if ui.button("Send to Back").clicked() {
            *action = Some(PanelAction::ReorderNode {
                node_id,
                op: ZOrderOp::SendToBack,
            });
            ui.close_menu();
        }
        ui.separator();
        if ui.button("Collect in New Layer").clicked() {
            *action = Some(PanelAction::CollectInNewLayer {
                node_ids: vec![node_id],
            });
            ui.close_menu();
        }
    });
}



pub fn draw_layers_panel(
    ui: &mut Ui,
    doc: &Document,
    selected_layer_ids: &mut Vec<LayerId>,
    selected_id: Option<NodeId>,
) -> Option<PanelAction> {
    let mut action: Option<PanelAction> = None;

    ui.label(
        RichText::new("LAYERS")
            .small()
            .color(Color32::from_rgb(80, 80, 110)),
    );
    ui.add_space(2.0);

    // Prune any stale selected_layer_ids (layers that no longer exist).
    selected_layer_ids.retain(|id| doc.layers.contains_key(id));

    // Layers from top to bottom in UI (reversed from draw order)
    for layer_id in doc.layer_order.iter().rev() {
        let Some(layer) = doc.layers.get(layer_id) else {
            continue;
        };
        let lid = *layer_id;

        let row = ui.horizontal(|ui| {
            // Drag handle — grab to reorder the layer stack (#169).
            ui.dnd_drag_source(egui::Id::new(("layer_grip", lid)), lid, |ui| {
                ui.add(
                    egui::Label::new(RichText::new(ph::DOTS_SIX_VERTICAL).weak())
                        .selectable(false),
                )
                .on_hover_text("Drag to reorder layer");
            });
            // Checkbox for multi-layer selection (used by Merge Layers).
            let mut checked = selected_layer_ids.contains(&lid);
            if ui.checkbox(&mut checked, "").changed() {
                if checked {
                    if !selected_layer_ids.contains(&lid) {
                        selected_layer_ids.push(lid);
                    }
                } else {
                    selected_layer_ids.retain(|id| id != &lid);
                }
            }

            // Color swatch — shows layer color tag; click cycles through preset colors.
            let swatch_color = match layer.color {
                Some([r, g, b, a]) => Color32::from_rgba_unmultiplied(
                    (r * 255.0) as u8,
                    (g * 255.0) as u8,
                    (b * 255.0) as u8,
                    (a * 255.0) as u8,
                ),
                None => Color32::from_gray(60),
            };
            // Cycle: None → Red → Orange → Yellow → Green → Blue → Purple → None
            const LAYER_COLORS: &[Option<[f32; 4]>] = &[
                None,
                Some([0.85, 0.20, 0.20, 1.0]),
                Some([0.90, 0.55, 0.10, 1.0]),
                Some([0.85, 0.80, 0.10, 1.0]),
                Some([0.20, 0.70, 0.25, 1.0]),
                Some([0.15, 0.45, 0.85, 1.0]),
                Some([0.60, 0.20, 0.80, 1.0]),
            ];
            let swatch_btn = egui::Button::new("")
                .fill(swatch_color)
                .min_size(egui::vec2(10.0, 10.0));
            if ui
                .add(swatch_btn)
                .on_hover_text("Click to cycle layer color tag")
                .clicked()
            {
                // Find the current color in the preset list and advance to next.
                let cur_idx = LAYER_COLORS
                    .iter()
                    .position(|c| *c == layer.color)
                    .unwrap_or(0);
                let next_color = LAYER_COLORS[(cur_idx + 1) % LAYER_COLORS.len()];
                action = Some(PanelAction::SetLayerColor {
                    layer_id: lid,
                    color: next_color,
                });
            }

            // Template toggle — "T" button; dimmed when not a template layer.
            let t_btn = egui::Button::new(RichText::new("T").small().color(if layer.is_template {
                Color32::from_rgb(255, 180, 60)
            } else {
                Color32::from_gray(90)
            }))
            .min_size(egui::vec2(14.0, 14.0));
            if ui
                .add(t_btn)
                .on_hover_text(if layer.is_template {
                    "Template layer (locked, dimmed) — click to disable"
                } else {
                    "Click to make this a template layer (locked, dimmed reference)"
                })
                .clicked()
            {
                action = Some(PanelAction::SetLayerTemplate {
                    layer_id: lid,
                    is_template: !layer.is_template,
                });
            }

            let layer_label = if layer.is_template {
                RichText::new(format!("{} [T]", layer.name))
                    .italics()
                    .weak()
            } else if layer.visible {
                RichText::new(format!("{}", layer.name))
            } else {
                RichText::new(format!("{} (hidden)", layer.name)).weak()
            };

            egui::CollapsingHeader::new(layer_label)
                .id_salt(lid)
                .default_open(true)
                .show(ui, |ui| {
                    let node_ids: Vec<NodeId> = layer.node_ids.iter().rev().copied().collect();
                    if node_ids.is_empty() {
                        ui.label(RichText::new("  (empty)").weak());
                    }
                    for node_id in node_ids {
                        draw_layer_node_row(ui, doc, node_id, selected_id, &mut action);
                    }
                });
        });

        // ── Drag-to-reorder drop handling (#169) ─────────────────────────────
        // Show an insertion line while a layer is dragged over this row, and on
        // release rebuild the stack order and emit a single undoable reorder.
        if row.response.dnd_hover_payload::<LayerId>().is_some() {
            let rect = row.response.rect;
            let top = ui
                .input(|i| i.pointer.hover_pos())
                .is_none_or(|p| p.y < rect.center().y);
            let y = if top { rect.top() } else { rect.bottom() };
            ui.painter().hline(
                rect.x_range(),
                y,
                egui::Stroke::new(2.0, Color32::from_rgb(110, 86, 207)),
            );
        }
        if let Some(payload) = row.response.dnd_release_payload::<LayerId>() {
            let dragged = *payload;
            if dragged != lid {
                let before = ui
                    .input(|i| i.pointer.hover_pos())
                    .is_none_or(|p| p.y < row.response.rect.center().y);
                // Work in UI order (top→bottom = layer_order reversed).
                let mut ui_order: Vec<LayerId> = doc.layer_order.iter().rev().copied().collect();
                ui_order.retain(|&x| x != dragged);
                if let Some(idx) = ui_order.iter().position(|&x| x == lid) {
                    let at = (if before { idx } else { idx + 1 }).min(ui_order.len());
                    ui_order.insert(at, dragged);
                    let new_order: Vec<LayerId> = ui_order.into_iter().rev().collect();
                    action = Some(PanelAction::ReorderLayers { new_order });
                }
            }
        }
    }

    // Show "Merge Selected" when 2+ layers are checked.
    if selected_layer_ids.len() >= 2 {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui
                .button(format!("Merge {} Layers", selected_layer_ids.len()))
                .on_hover_text(
                    "Merge selected layers into one (bottom-most in stack order is kept)",
                )
                .clicked()
            {
                action = Some(PanelAction::MergeLayers {
                    layer_ids: selected_layer_ids.clone(),
                });
                selected_layer_ids.clear();
            }
        });
    }

    // Flatten Artwork button (always shown when > 1 layer)
    if doc.layer_order.len() > 1 {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui
                .button("Flatten Artwork")
                .on_hover_text("Merge all layers into one; bottom-most layer is kept")
                .clicked()
            {
                action = Some(PanelAction::FlattenArtwork);
            }
        });
    }

    ui.separator();
    ui.label(RichText::new(format!("{} objects", doc.node_count())).weak());

    action
}

