use super::*;

pub(crate) fn draw_edit_history(ui: &mut Ui, ctx: &mut PropPanelCtx) {
    let history_entries = ctx.history_entries;
    let history_total = ctx.history_total;
    let matches = |label: &str| -> bool { ctx.matches(label) };
    let mut action: Option<PanelAction> = None;
    // ── Edit History ──────────────────────────────────────────────────────────
    if matches("History") {
        egui::CollapsingHeader::new("Edit History")
            .default_open(false)
            .id_salt("history_panel")
            .show(ui, |ui| {
                ui.label(
                    RichText::new(format!("Edit history ({} steps):", history_total))
                        .weak()
                        .small(),
                );
                ui.add_space(2.0);
                if history_entries.is_empty() {
                    ui.label(RichText::new("No edits yet.").weak().small());
                } else {
                    for (step, desc) in history_entries.iter().take(20) {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(format!("{}. {}", step, desc)).small().color(
                                if *step == 1 {
                                    Color32::from_rgb(180, 210, 255)
                                } else {
                                    Color32::from_rgb(130, 130, 150)
                                },
                            ));
                        });
                    }
                }
                if history_total > 0 {
                    ui.add_space(4.0);
                    ui.separator();
                    ui.add_space(2.0);
                    thread_local! {
                        static JUMP_INDEX: std::cell::RefCell<usize> = std::cell::RefCell::new(0);
                    }
                    JUMP_INDEX.with(|v| {
                        let mut val = (*v.borrow()).min(history_total);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Jump to step:").small());
                            ui.add(
                                egui::DragValue::new(&mut val)
                                    .range(0..=history_total)
                                    .speed(1.0),
                            );
                            if ui
                                .small_button("Jump")
                                .on_hover_text(format!(
                                    "Jump to undo depth {} (0=oldest, {}=current)",
                                    val, history_total
                                ))
                                .clicked()
                            {
                                action = Some(PanelAction::JumpToHistory { index: val });
                            }
                        });
                        *v.borrow_mut() = val;
                    });
                }
            });
        ui.add_space(4.0);
    }

    if action.is_some() {
        ctx.action = action;
    }
}

pub(crate) fn draw_branches(ui: &mut Ui, ctx: &mut PropPanelCtx) {
    let branch_names = ctx.branch_names;
    let branch_name_input = &mut *ctx.branch_name_input;
    let q = ctx.q.as_str();
    let matches = |label: &str| -> bool { q.is_empty() || label.to_lowercase().contains(q) };
    let forced_open = ctx.forced_open;
    let mut action: Option<PanelAction> = None;
    // ── Branches ──────────────────────────────────────────────────────────────
    if matches("Branches") {
        egui::CollapsingHeader::new("Branches")
            .default_open(false)
            .open(forced_open)
            .show(ui, |ui| {
                ui.label(
                    RichText::new("Fork the document state into named branches.")
                        .weak()
                        .small(),
                );
                ui.add_space(2.0);
                // Save new branch
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(branch_name_input)
                            .hint_text("Branch name…")
                            .desired_width(ui.available_width() - 60.0),
                    );
                    let can_save = !branch_name_input.trim().is_empty();
                    if ui
                        .add_enabled(can_save, egui::Button::new("Save").small())
                        .clicked()
                    {
                        let name = branch_name_input.trim().to_string();
                        action = Some(PanelAction::BranchCreate { name });
                        branch_name_input.clear();
                    }
                });
                ui.add_space(4.0);
                if branch_names.is_empty() {
                    ui.label(RichText::new("No branches yet.").weak().small());
                } else {
                    for name in branch_names {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(name).small());
                            if ui
                                .small_button("Switch")
                                .on_hover_text(format!("Restore branch '{}'", name))
                                .clicked()
                            {
                                action = Some(PanelAction::BranchSwitch { name: name.clone() });
                            }
                            if ui
                                .small_button(ph::X)
                                .on_hover_text(format!("Delete branch '{}'", name))
                                .clicked()
                            {
                                action = Some(PanelAction::BranchDelete { name: name.clone() });
                            }
                        });
                    }
                }
            });
        ui.add_space(4.0);
    }

    if action.is_some() {
        ctx.action = action;
    }
}

