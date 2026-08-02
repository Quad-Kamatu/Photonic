//! Crash-recovery: restore untitled-document autosaves written to the recovery
//! folder by a previous session (see `autosave.rs`). On launch the recovery folder
//! is scanned in `draw`; if it holds any `.photon` files this prompt offers to
//! reopen them as tabs (still untitled + dirty, so the user saves them properly)
//! or discard them.

use super::*;
use std::path::PathBuf;

impl PhotonicApp {
    /// Replace the live document wholesale and rebuild the tab list to a single
    /// tab — used for the first restored document when coming from the welcome
    /// screen (where the live document is a throwaway blank).
    fn replace_live_document(
        &mut self,
        doc: &mut Document,
        history: &mut CommandHistory,
        new_doc: Document,
        new_history: CommandHistory,
        file: Option<PathBuf>,
    ) {
        *doc = new_doc;
        *history = new_history;
        self.current_file = file;
        self.selected_id = None;
        self.fit_pending = true;
        self.show_welcome = false;
        self.tabs.clear();
        self.active_tab = 0;
        self.ensure_initial_tab(doc, history);
    }

    /// Restore/Discard prompt for recovered autosaves. Returns `true` when the
    /// prompt was resolved by restoring (so `draw` re-enters cleanly next frame).
    pub(crate) fn draw_recovery_prompt(
        &mut self,
        ctx: &egui::Context,
        doc: &mut Document,
        view: &mut CanvasView,
        history: &mut CommandHistory,
    ) -> bool {
        let names: Vec<String> = self
            .recovery_candidates
            .iter()
            .map(|p| {
                p.file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "recovered".into())
            })
            .collect();

        super::close_guard::scrim(ctx, "recovery_scrim");
        #[derive(PartialEq)]
        enum Choice {
            None,
            Restore,
            Discard,
        }
        let mut choice = Choice::None;
        egui::Window::new(
            RichText::new(format!(
                "{}  Recover unsaved documents",
                ph::CLOCK_COUNTER_CLOCKWISE
            ))
            .size(16.0),
        )
        .id(egui::Id::new("recovery_prompt"))
        .order(egui::Order::Foreground)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.set_max_width(460.0);
            ui.label(format!(
                "{} unsaved document(s) from a previous session were found:",
                names.len()
            ));
            ui.add_space(4.0);
            egui::ScrollArea::vertical()
                .max_height(160.0)
                .show(ui, |ui| {
                    for n in &names {
                        ui.label(RichText::new(format!("  • {n}")).weak());
                    }
                });
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui.button("Restore all").clicked() {
                    choice = Choice::Restore;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Discard all").clicked() {
                        choice = Choice::Discard;
                    }
                });
            });
        });

        match choice {
            Choice::Restore => {
                let paths = std::mem::take(&mut self.recovery_candidates);
                for path in paths {
                    match load_document(&path) {
                        Ok((loaded, snap)) => {
                            let mut new_history = CommandHistory::default();
                            apply_opened_history(&mut new_history, snap);
                            if self.tabs.is_empty() {
                                self.replace_live_document(doc, history, loaded, new_history, None);
                            } else {
                                self.open_in_new_tab(doc, history, view, loaded, new_history, None);
                            }
                            // Keep it untitled + dirty and remember its recovery
                            // file so continued autosave reuses the same slot.
                            if let Some(t) = self.tabs.get_mut(self.active_tab) {
                                t.recovery_path = Some(path.clone());
                                t.dirty = true;
                                t.last_saved_node = None;
                            }
                        }
                        Err(_) => {
                            let _ = std::fs::remove_file(&path);
                        }
                    }
                }
                self.file_status = Some("Restored recovered documents".into());
                true
            }
            Choice::Discard => {
                let paths = std::mem::take(&mut self.recovery_candidates);
                for p in paths {
                    let _ = std::fs::remove_file(p);
                }
                false
            }
            Choice::None => false,
        }
    }
}
