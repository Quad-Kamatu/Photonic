use super::*;

/// Draw the vertical tools panel. Returns the newly selected tool if changed.
pub fn draw_tools_panel(ui: &mut Ui, active: Tool, pinned_tools: &[Tool]) -> Option<Tool> {
    let mut chosen = None;

    // ── Hotbar (pinned tools) ─────────────────────────────────────────────
    if !pinned_tools.is_empty() {
        ui.label(
            RichText::new(format!("{} HOTBAR", egui_phosphor::regular::PUSH_PIN))
                .small()
                .color(Color32::from_rgb(110, 86, 207)),
        );
        ui.add_space(2.0);
        for tool in pinned_tools {
            let label = format!("{} {}", tool.icon(), tool.label());
            if ui.selectable_label(*tool == active, label).clicked() {
                chosen = Some(*tool);
            }
        }
        ui.separator();
        ui.add_space(2.0);
    }

    ui.label(
        RichText::new("TOOLS")
            .small()
            .color(Color32::from_rgb(80, 80, 110)),
    );
    ui.add_space(2.0);

    // ── Selection & navigation ────────────────────────────────────────────
    for tool in [Tool::Select, Tool::DirectSelect, Tool::Pan] {
        let label = format!("{} {}", tool.icon(), tool.label());
        if ui.selectable_label(tool == active, label).clicked() {
            chosen = Some(tool);
        }
    }

    // ── Shapes group (Rectangle / Ellipse / Polygon / Star) ──────────────
    // A single button that shows the active shape's icon and opens a
    // hover popover for switching between shape types.
    {
        let popup_id = ui.make_persistent_id("shapes_popover");
        let is_shape_active = active.is_shape_creator();

        // Active shape's icon/label, or Rectangle as the default
        let (group_icon, group_label) = if is_shape_active {
            (active.icon(), active.label())
        } else {
            (Tool::Rectangle.icon(), "Shapes")
        };

        // "›" indicator signals that sub-tools are available
        let btn_text = format!("{} {}  ›", group_icon, group_label);
        let response = ui.selectable_label(is_shape_active, &btn_text);

        // Open the popover on hover
        if response.hovered() {
            ui.memory_mut(|m| m.open_popup(popup_id));
        }

        // Direct click (without hovering into the popover) activates the
        // currently-shown shape, or Rectangle when no shape is active.
        if response.clicked() && !is_shape_active {
            chosen = Some(Tool::Rectangle);
        }

        // Render the popover to the right of the button
        if ui.memory(|m| m.is_popup_open(popup_id)) {
            let pos = egui::pos2(response.rect.right() + 4.0, response.rect.top());

            let area_resp = egui::Area::new(popup_id)
                .kind(egui::UiKind::Popup)
                .order(egui::Order::Foreground)
                .pivot(egui::Align2::LEFT_TOP)
                .fixed_pos(pos)
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style())
                        .show(ui, |ui| {
                            ui.set_min_width(110.0);
                            let mut picked: Option<Tool> = None;
                            for shape in [
                                Tool::Rectangle,
                                Tool::Ellipse,
                                Tool::Polygon,
                                Tool::Star,
                                Tool::Spiral,
                            ] {
                                let label = format!("{} {}", shape.icon(), shape.label());
                                if ui.selectable_label(shape == active, label).clicked() {
                                    picked = Some(shape);
                                }
                            }
                            picked
                        })
                        .inner
                });

            // Close when the pointer leaves both the button and the popover
            let popup_rect = area_resp.response.rect;
            let pointer_in_popup = ui
                .ctx()
                .pointer_latest_pos()
                .map(|p| popup_rect.contains(p))
                .unwrap_or(false);

            if !response.hovered() && !pointer_in_popup {
                ui.memory_mut(|m| m.close_popup());
            }

            if let Some(tool) = area_resp.inner {
                chosen = Some(tool);
                ui.memory_mut(|m| m.close_popup());
            }
        }
    }

    // ── Drawing tools ─────────────────────────────────────────────────────
    for tool in [Tool::Pen, Tool::ShapeBuilder, Tool::Text] {
        let label = format!("{} {}", tool.icon(), tool.label());
        if ui.selectable_label(tool == active, label).clicked() {
            chosen = Some(tool);
        }
    }

    // ── Path editing tools ─────────────────────────────────────────────────
    ui.add_space(4.0);
    ui.separator();
    ui.add_space(2.0);
    for tool in [
        Tool::Scissors,
        Tool::Knife,
        Tool::Eraser,
        Tool::MagicWand,
        Tool::Lasso,
        Tool::Pencil,
    ] {
        let label = format!("{} {}", tool.icon(), tool.label());
        if ui.selectable_label(tool == active, label).clicked() {
            chosen = Some(tool);
        }
    }

    chosen
}

