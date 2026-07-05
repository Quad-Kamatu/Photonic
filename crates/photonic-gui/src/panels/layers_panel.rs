use super::*;
use crate::multi_button::{multi_button, MultiButtonItem};

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

    // During the drawer's slide animation the panel is momentarily ~1px wide;
    // the nested top/bottom panels below dislike degenerate widths, so skip
    // layout entirely until there is room (content is ~invisible then anyway).
    if ui.available_width() < 40.0 {
        return None;
    }

    // Prune any stale selected_layer_ids (layers that no longer exist).
    selected_layer_ids.retain(|id| doc.layers.contains_key(id));

    // ── Pinned footer: slide-up adjustment tray + action bar + object count ──
    // Rendered as a bottom panel so it always sits at the base of the drawer,
    // independent of how far the layer list is scrolled.
    egui::TopBottomPanel::bottom("layers_footer")
        .frame(egui::Frame::none().inner_margin(egui::Margin::symmetric(2.0, 4.0)))
        .show_inside(ui, |ui| {
            draw_layers_footer(ui, doc, &mut action);
        });

    // ── Scrolling layer tree fills the remaining space above the footer ──────
    egui::CentralPanel::default()
        .frame(egui::Frame::none())
        .show_inside(ui, |ui| {
            ui.label(
                RichText::new("LAYERS")
                    .small()
                    .color(Color32::from_rgb(80, 80, 110)),
            );
            ui.add_space(2.0);
            egui::ScrollArea::vertical()
                .id_salt("layers_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    draw_layers_tree(ui, doc, selected_layer_ids, selected_id, &mut action);
                });
        });

    action
}

/// Render the scrolling layer stack (top→bottom) with per-layer right-click
/// menus, inline rename, drag-reorder, and the contextual Merge/Flatten rows.
fn draw_layers_tree(
    ui: &mut Ui,
    doc: &Document,
    selected_layer_ids: &mut Vec<LayerId>,
    selected_id: Option<NodeId>,
    action: &mut Option<PanelAction>,
) {
    // Which layer (if any) is currently being renamed inline, and its buffer.
    let rename_target: Option<LayerId> = ui.data(|d| d.get_temp(rename_target_id()));

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
                *action = Some(PanelAction::SetLayerColor {
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
                *action = Some(PanelAction::SetLayerTemplate {
                    layer_id: lid,
                    is_template: !layer.is_template,
                });
            }

            // Inline rename takes over the name slot while this layer is the
            // rename target; otherwise a disclosure header + right-click menu.
            if rename_target == Some(lid) {
                let mut buf: String = ui.data(|d| d.get_temp(rename_buf_id()).unwrap_or_default());
                let edit_id = egui::Id::new(("layer_rename_edit", lid));
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut buf)
                        .id(edit_id)
                        .desired_width(140.0),
                );
                if resp.lost_focus() {
                    // Commit on Enter (and on click-away); Escape cancels.
                    let escaped = ui.input(|i| i.key_pressed(egui::Key::Escape));
                    let trimmed = buf.trim();
                    if !escaped && !trimmed.is_empty() && trimmed != layer.name {
                        *action = Some(PanelAction::RenameLayer {
                            layer_id: lid,
                            name: trimmed.to_string(),
                        });
                    }
                    ui.data_mut(|d| {
                        d.remove::<LayerId>(rename_target_id());
                        d.remove::<String>(rename_buf_id());
                    });
                } else {
                    ui.data_mut(|d| d.insert_temp(rename_buf_id(), buf));
                }
            } else {
                let layer_label = if layer.is_template {
                    RichText::new(format!("{} [T]", layer.name))
                        .italics()
                        .weak()
                } else if layer.visible {
                    RichText::new(layer.name.to_string())
                } else {
                    RichText::new(format!("{} (hidden)", layer.name)).weak()
                };

                let ch = egui::CollapsingHeader::new(layer_label)
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
                                action,
                            );
                        }
                    });

                // Right-click the layer name for layer-level actions.
                draw_layer_context_menu(&ch.header_response, doc, lid, layer, action);
            }
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
                    *action = Some(PanelAction::ReorderLayers { new_order });
                }
            }
        }
        // Drop a NODE onto this layer row → reparent it to this layer's top (#210),
        // i.e. pull it out of a group / move it between layers.
        if let Some(payload) = row.response.dnd_release_payload::<NodeId>() {
            *action = Some(PanelAction::ReparentNode {
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
                *action = Some(PanelAction::MergeLayers {
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
                *action = Some(PanelAction::FlattenArtwork);
            }
        });
    }
}

// ── Footer / context-menu / adjustment-tray helpers ─────────────────────────

/// egui memory id: which layer is being renamed inline (holds a `LayerId`).
fn rename_target_id() -> egui::Id {
    egui::Id::new("layers_rename_target")
}
/// egui memory id: the working text buffer for the inline rename (a `String`).
fn rename_buf_id() -> egui::Id {
    egui::Id::new("layers_rename_buf")
}
/// egui memory id: whether the adjustment-layer slide-up tray is open (a `bool`).
fn adjust_tray_open_id() -> egui::Id {
    egui::Id::new("layers_adjust_tray_open")
}

/// Right-click menu for a layer row: rename, add sublayer, show/hide, lock,
/// and delete. Emits the matching [`PanelAction`].
fn draw_layer_context_menu(
    header: &egui::Response,
    doc: &Document,
    lid: LayerId,
    layer: &photonic_core::layer::Layer,
    action: &mut Option<PanelAction>,
) {
    header.context_menu(|ui| {
        if ui.button(format!("{} Rename", ph::PENCIL_SIMPLE)).clicked() {
            // Arm inline rename and focus the edit box on the next frame.
            ui.data_mut(|d| {
                d.insert_temp(rename_target_id(), lid);
                d.insert_temp(rename_buf_id(), layer.name.clone());
            });
            ui.ctx()
                .memory_mut(|m| m.request_focus(egui::Id::new(("layer_rename_edit", lid))));
            ui.close_menu();
        }
        if ui
            .button(format!("{} Add Sublayer", ph::FOLDER_SIMPLE_PLUS))
            .clicked()
        {
            *action = Some(PanelAction::AddSublayer);
            ui.close_menu();
        }
        ui.separator();
        let (vis_icon, vis_label) = if layer.visible {
            (ph::EYE_SLASH, "Hide Layer")
        } else {
            (ph::EYE, "Show Layer")
        };
        if ui.button(format!("{} {}", vis_icon, vis_label)).clicked() {
            *action = Some(PanelAction::SetLayerVisible {
                layer_id: lid,
                visible: !layer.visible,
            });
            ui.close_menu();
        }
        let (lock_icon, lock_label) = if layer.locked {
            (ph::LOCK_SIMPLE_OPEN, "Unlock Layer")
        } else {
            (ph::LOCK_SIMPLE, "Lock Layer")
        };
        if ui.button(format!("{} {}", lock_icon, lock_label)).clicked() {
            *action = Some(PanelAction::SetLayerLocked {
                layer_id: lid,
                locked: !layer.locked,
            });
            ui.close_menu();
        }
        ui.separator();
        // Delete — refuse when this is the only remaining layer.
        let can_delete = doc.layer_order.len() > 1;
        if ui
            .add_enabled(
                can_delete,
                egui::Button::new(format!("{} Delete Layer", ph::TRASH)),
            )
            .clicked()
        {
            *action = Some(PanelAction::DeleteLayer { layer_id: lid });
            ui.close_menu();
        }
    });
}

/// The pinned footer: the adjustment slide-up tray, the four layer-action
/// buttons, and the object-count readout.
fn draw_layers_footer(ui: &mut Ui, doc: &Document, action: &mut Option<PanelAction>) {
    // Slide-up tray drawn first so it appears *above* the buttons, rising up.
    let open = ui.data(|d| d.get_temp::<bool>(adjust_tray_open_id()).unwrap_or(false));
    let t = ui
        .ctx()
        .animate_bool_with_time(egui::Id::new("layers_adjust_tray_anim"), open, 0.18);
    if t > 0.001 {
        draw_adjustment_tray(ui, t, action);
    }

    // Action bar — the four layer actions as a segmented multi-button pill:
    // icon-only at rest, each expands its label on hover.
    let items = [
        MultiButtonItem::new(
            ph::STACK_PLUS,
            "New Layer",
            "Add a new empty layer at the top of the stack",
        ),
        MultiButtonItem::new(
            ph::FOLDER_SIMPLE_PLUS,
            "Sublayer",
            "Add a nested group container to the active layer",
        ),
        MultiButtonItem::new(
            ph::SELECTION,
            "Mask",
            "Add a mask for the current selection:\n• a raster node → editable alpha mask\n• 2+ objects → clipping mask (top object clips)",
        ),
        MultiButtonItem::new(
            ph::SUN,
            "Adjust",
            "Add a non-destructive adjustment layer",
        ),
    ];
    // Even padding above/below, and centered horizontally in the drawer.
    ui.add_space(6.0);
    let clicked = ui
        .vertical_centered(|ui| multi_button(ui, "layers_action_bar", &items))
        .inner;
    ui.add_space(6.0);
    if let Some(idx) = clicked {
        match idx {
            0 => *action = Some(PanelAction::AddLayer),
            1 => *action = Some(PanelAction::AddSublayer),
            2 => *action = Some(PanelAction::AddLayerMaskSmart),
            // Adjust → toggle the slide-up tray of adjustment presets.
            _ => ui.data_mut(|d| {
                let cur = d.get_temp::<bool>(adjust_tray_open_id()).unwrap_or(false);
                d.insert_temp(adjust_tray_open_id(), !cur);
            }),
        }
    }

    ui.separator();
    ui.label(
        RichText::new(format!("{} objects", doc.node_count()))
            .weak()
            .small(),
    );
}

/// The adjustment-layer tray: a wrapping grid of preview-square tiles whose
/// revealed height animates with `t` (0→1) for a slide-up effect. Selecting a
/// tile emits [`PanelAction::AddAdjustmentLayer`] and closes the tray.
fn draw_adjustment_tray(ui: &mut Ui, t: f32, action: &mut Option<PanelAction>) {
    egui::Frame::group(ui.style())
        .inner_margin(egui::Margin::same(4.0))
        .show(ui, |ui| {
            ui.label(
                RichText::new("ADJUSTMENT LAYER")
                    .small()
                    .color(Color32::from_rgb(110, 86, 207)),
            );
            egui::ScrollArea::vertical()
                .id_salt("layers_adjust_tray_scroll")
                .max_height(168.0 * t)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        for (kind, label) in crate::app::layer_ops::ADJUSTMENT_TILES {
                            if adjustment_tile(ui, kind, label) {
                                *action = Some(PanelAction::AddAdjustmentLayer {
                                    kind: (*kind).to_string(),
                                });
                                ui.data_mut(|d| d.insert_temp(adjust_tray_open_id(), false));
                            }
                        }
                    });
                });
        });
}

/// One adjustment tile: a painted preview square with a caption. Returns true
/// when clicked.
fn adjustment_tile(ui: &mut Ui, kind: &str, label: &str) -> bool {
    let sq = 44.0;
    let resp = ui
        .allocate_ui_with_layout(
            egui::vec2(58.0, sq + 18.0),
            egui::Layout::top_down(egui::Align::Center),
            |ui| {
                let (rect, resp) =
                    ui.allocate_exact_size(egui::vec2(sq, sq), egui::Sense::click());
                paint_adjustment_preview(ui.painter(), rect, kind);
                let border = if resp.hovered() {
                    egui::Stroke::new(2.0, Color32::from_rgb(110, 86, 207))
                } else {
                    egui::Stroke::new(1.0, Color32::from_gray(70))
                };
                ui.painter()
                    .rect_stroke(rect, egui::Rounding::same(4.0), border);
                ui.add(egui::Label::new(RichText::new(label).small()).truncate());
                resp
            },
        )
        .inner;
    resp.on_hover_text(format!("Add {label} adjustment layer"))
        .clicked()
}

/// Paint a small representative preview of an adjustment as a horizontal ramp of
/// colour strips (plus a diagonal guide for curve/level-type adjustments).
fn paint_adjustment_preview(painter: &egui::Painter, rect: egui::Rect, kind: &str) {
    let n = 14;
    for i in 0..n {
        let f = i as f32 / (n - 1) as f32;
        let x0 = rect.left() + rect.width() * (i as f32 / n as f32);
        let x1 = rect.left() + rect.width() * ((i + 1) as f32 / n as f32);
        let strip =
            egui::Rect::from_min_max(egui::pos2(x0, rect.top()), egui::pos2(x1, rect.bottom()));
        painter.rect_filled(strip, egui::Rounding::ZERO, preview_color(kind, f));
    }
    if matches!(kind, "curves" | "levels") {
        painter.line_segment(
            [
                egui::pos2(rect.left(), rect.bottom()),
                egui::pos2(rect.right(), rect.top()),
            ],
            egui::Stroke::new(1.5, Color32::from_white_alpha(200)),
        );
    }
}

/// Sample colour at position `f` (0..1) for an adjustment's preview ramp.
fn preview_color(kind: &str, f: f32) -> Color32 {
    let gray = |v: f32| {
        let c = (v.clamp(0.0, 1.0) * 255.0) as u8;
        Color32::from_rgb(c, c, c)
    };
    match kind {
        "hue_saturation" => Color32::from(egui::ecolor::Hsva::new(f, 1.0, 1.0, 1.0)),
        "vibrance" => Color32::from(egui::ecolor::Hsva::new(f, 0.55, 0.95, 1.0)),
        "invert" => gray(1.0 - f),
        "posterize" => gray((f * 4.0).floor() / 3.0),
        "threshold" => {
            if f < 0.5 {
                Color32::BLACK
            } else {
                Color32::WHITE
            }
        }
        "exposure" => gray((f + 0.15).min(1.0)),
        // brightness_contrast, levels, curves, black_and_white → grayscale ramp.
        _ => gray(f),
    }
}

