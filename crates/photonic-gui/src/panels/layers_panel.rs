use super::*;

/// Draw the left layers panel. Returns an optional action triggered by context menus.
/// Recursively render one node row in the Layers tree. Groups get a disclosure
/// header that expands to their children at any depth; every row is selectable
/// (emits `SelectNode`) and carries the z-order / collect context menu.
fn draw_layer_node_row(
    ui: &mut Ui,
    doc: &Document,
    node_id: NodeId,
    parent: photonic_core::document::NodeContainer,
    index_in_parent: usize,
    selected_id: Option<NodeId>,
    action: &mut Option<PanelAction>,
) {
    use photonic_core::document::NodeContainer;
    use photonic_core::node::SceneNodeKind as K;
    let Some(node) = doc.nodes.get(&node_id) else {
        return;
    };
    let is_selected = selected_id == Some(node_id);
    let grip = |ui: &mut Ui, id: NodeId| {
        ui.dnd_drag_source(egui::Id::new(("node_drag", id)), id, |ui| {
            ui.add(
                egui::Label::new(RichText::new(ph::DOTS_SIX_VERTICAL).weak()).selectable(false),
            )
            .on_hover_text("Drag to reparent");
        })
        .response
    };

    let response = match &node.kind {
        SceneNodeKind::Group(g) => {
            // Manual disclosure (persisted open flag) so we control the header
            // row layout — a grip + folder label — and still get an indented body.
            let open_id = egui::Id::new(("layer_group_open", node_id));
            let mut open = ui.data_mut(|d| d.get_temp::<bool>(open_id).unwrap_or(true));
            let clip = g.clip_node_id.is_some();
            let row = ui.horizontal(|ui| {
                grip(ui, node_id);
                let tri = if open { ph::CARET_DOWN } else { ph::CARET_RIGHT };
                if ui
                    .add(egui::Label::new(RichText::new(tri).weak()).sense(egui::Sense::click()))
                    .clicked()
                {
                    open = !open;
                }
                let name = if clip {
                    format!("{} {} {}", ph::FOLDER_SIMPLE, node.name, ph::SCISSORS)
                } else {
                    format!("{} {}", ph::FOLDER_SIMPLE, node.name)
                };
                let label = RichText::new(name).color(if is_selected {
                    Color32::from_rgb(184, 164, 255)
                } else {
                    Color32::from_rgb(144, 119, 224)
                });
                if ui.selectable_label(is_selected, label).clicked() {
                    *action = Some(PanelAction::SelectNode { node_id });
                }
            });
            ui.data_mut(|d| d.insert_temp(open_id, open));
            let row_resp = row.response;
            // Drop a node ONTO this group → reparent into it (at the top of the
            // folder). Highlight while hovering with a payload.
            if row_resp.dnd_hover_payload::<NodeId>().is_some() {
                ui.painter().rect_stroke(
                    row_resp.rect,
                    egui::Rounding::same(3.0),
                    egui::Stroke::new(1.5, Color32::from_rgb(110, 86, 207)),
                );
            }
            if let Some(p) = row_resp.dnd_release_payload::<NodeId>() {
                if *p != node_id {
                    *action = Some(PanelAction::ReparentNode {
                        node_id: *p,
                        new: NodeContainer::Group(node_id),
                        new_index: g.children.len(),
                    });
                }
            }
            // Indented children.
            if open {
                ui.indent(egui::Id::new(("layer_group_body", node_id)), |ui| {
                    if g.children.is_empty() {
                        ui.label(RichText::new("(empty group)").weak());
                    }
                    let n = g.children.len();
                    for (ui_i, child_id) in g.children.iter().rev().copied().enumerate() {
                        let cidx = n - 1 - ui_i;
                        draw_layer_node_row(
                            ui,
                            doc,
                            child_id,
                            NodeContainer::Group(node_id),
                            cidx,
                            selected_id,
                            action,
                        );
                    }
                });
            }
            row_resp
        }
        _ => {
            let compound = matches!(&node.kind, K::Path(p) if p.is_compound);
            let src = ui.dnd_drag_source(egui::Id::new(("node_drag", node_id)), node_id, |ui| {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::Label::new(RichText::new(ph::DOTS_SIX_VERTICAL).weak())
                            .selectable(false),
                    );
                    let name = if compound {
                        format!("• {} {}", node.name, ph::INTERSECT)
                    } else {
                        format!("• {}", node.name)
                    };
                    ui.selectable_label(is_selected, name)
                })
                .inner
            });
            if src.inner.clicked() {
                *action = Some(PanelAction::SelectNode { node_id });
            }
            let row_resp = src.response;
            // Drop onto a leaf → reparent to this leaf's container at its slot
            // (basic between-rows placement).
            if let Some(p) = row_resp.dnd_release_payload::<NodeId>() {
                if *p != node_id {
                    *action = Some(PanelAction::ReparentNode {
                        node_id: *p,
                        new: parent,
                        new_index: index_in_parent,
                    });
                }
            }
            row_resp
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

    ui.horizontal(|ui| {
        ui.label(
            RichText::new("LAYERS")
                .small()
                .color(Color32::from_rgb(80, 80, 110)),
        );
        // Push the "add layer" button to the right edge of the header.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add(
                    egui::Button::new(RichText::new(ph::PLUS).small())
                        .small()
                        .frame(true),
                )
                .on_hover_text("Add a new empty layer")
                .clicked()
            {
                action = Some(PanelAction::AddLayer);
            }
        });
    });
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
                    use photonic_core::document::NodeContainer;
                    if layer.node_ids.is_empty() {
                        ui.label(RichText::new("  (empty)").weak());
                    }
                    let n = layer.node_ids.len();
                    for (ui_i, node_id) in layer.node_ids.iter().rev().copied().enumerate() {
                        let idx = n - 1 - ui_i;
                        draw_layer_node_row(
                            ui,
                            doc,
                            node_id,
                            NodeContainer::Layer(lid),
                            idx,
                            selected_id,
                            &mut action,
                        );
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
        // Drop a NODE onto this layer row → reparent it to this layer's top (#210),
        // i.e. pull it out of a group / move it between layers.
        if let Some(payload) = row.response.dnd_release_payload::<NodeId>() {
            action = Some(PanelAction::ReparentNode {
                node_id: *payload,
                new: photonic_core::document::NodeContainer::Layer(lid),
                new_index: doc.layers.get(&lid).map_or(0, |l| l.node_ids.len()),
            });
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

