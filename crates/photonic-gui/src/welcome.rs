use egui::{Color32, Margin, RichText, Rounding, Stroke, Vec2};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const MAX_RECENT: usize = 8;

// ─── Types ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecentEntry {
    pub path: PathBuf,
    pub name: String,
}

pub enum WelcomeAction {
    CreateNew {
        name: String,
        width: f64,
        height: f64,
    },
    OpenFile(PathBuf),
    OpenBrowse,
}

// Canvas size presets: (label, width, height)
const PRESETS: &[(&str, f64, f64)] = &[
    ("A4", 1123.0, 794.0),
    ("Letter", 1056.0, 816.0),
    ("HD", 1280.0, 720.0),
    ("4K", 3840.0, 2160.0),
    ("Square", 1024.0, 1024.0),
];

// ─── State ────────────────────────────────────────────────────────────────────

pub struct WelcomeState {
    pub doc_name: String,
    pub width: f64,
    pub height: f64,
    pub recent: Vec<RecentEntry>,
}

impl WelcomeState {
    pub fn new() -> Self {
        Self {
            doc_name: "Untitled".to_string(),
            width: 1123.0,
            height: 794.0,
            recent: load_recent(),
        }
    }

    /// Record a file in the recent list (modifies in-memory + saves to disk).
    pub fn add_recent(&mut self, path: PathBuf, name: String) {
        self.recent.retain(|e| e.path != path);
        self.recent.insert(0, RecentEntry { path, name });
        self.recent.truncate(MAX_RECENT);
        save_recent(&self.recent);
    }

    /// Draw the welcome screen, returning an action if the user made a choice.
    pub fn draw(&mut self, ctx: &egui::Context) -> Option<WelcomeAction> {
        // ── Colours (match the dark theme) ───────────────────────────────────
        let accent = Color32::from_rgb(110, 86, 207);
        let accent_dim = Color32::from_rgb(61, 48, 128);
        let bg_base = Color32::from_rgb(7, 7, 11);
        let bg_panel = Color32::from_rgb(12, 12, 21);
        let bg_elevated = Color32::from_rgb(19, 19, 31);
        let bg_widget = Color32::from_rgb(26, 26, 40);
        let border = Color32::from_rgb(30, 30, 50);
        let border_focus = Color32::from_rgb(110, 86, 207);
        let text_primary = Color32::from_rgb(232, 232, 242);
        let text_muted = Color32::from_rgb(122, 122, 154);

        let card_frame = egui::Frame::none()
            .fill(bg_panel)
            .rounding(Rounding::same(8.0))
            .stroke(Stroke::new(1.0, border))
            .inner_margin(Margin::same(24.0));

        let mut action: Option<WelcomeAction> = None;

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(bg_base))
            .show(ctx, |ui| {
                let avail_w = ui.available_width();

                // ── App title pinned to the top ───────────────────────────────
                ui.add_space(36.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("PHOTONIC").size(38.0).color(accent).strong());
                    ui.add_space(5.0);
                    ui.label(
                        RichText::new("Vector Graphics Editor")
                            .size(13.0)
                            .color(text_muted),
                    );
                });
                ui.add_space(32.0);

                // ── Divider between header and cards ──────────────────────────
                let sep_color = Color32::from_rgb(25, 25, 42);
                ui.painter().hline(
                    ui.available_rect_before_wrap().x_range(),
                    ui.cursor().top(),
                    Stroke::new(1.0, sep_color),
                );
                ui.add_space(32.0);

                // ── Two-column card area, inset from window edges ─────────────
                // vertical_centered + set_max_width is the reliable way in egui to
                // centre a fixed-width block; Frame::inner_margin does NOT constrain
                // child max_rect, so buttons with large min_size would escape it.
                let card_total_w = (avail_w - 120.0).min(800.0).max(400.0);
                ui.vertical_centered(|ui| {
                    ui.set_max_width(card_total_w);
                    // Widen the inter-column gap; ui.columns() reads item_spacing.x
                    // once when it computes each column's rect.
                    ui.spacing_mut().item_spacing.x = 20.0;

                    ui.columns(2, |cols| {
                        // ── Left: New Document ────────────────────────────
                        card_frame.show(&mut cols[0], |ui| {
                            ui.label(
                                RichText::new("New Document")
                                    .size(14.0)
                                    .color(text_primary)
                                    .strong(),
                            );
                            ui.add(egui::Separator::default().spacing(10.0));

                            ui.label(RichText::new("Name").color(text_muted).size(11.0));
                            ui.add_space(2.0);
                            ui.add(
                                egui::TextEdit::singleline(&mut self.doc_name)
                                    .desired_width(f32::INFINITY)
                                    .font(egui::TextStyle::Body),
                            );
                            ui.add_space(16.0);

                            ui.label(RichText::new("Canvas").color(text_muted).size(11.0));
                            ui.add_space(4.0);
                            ui.horizontal_wrapped(|ui| {
                                for (label, pw, ph) in PRESETS {
                                    let selected = (self.width - pw).abs() < 0.5
                                        && (self.height - ph).abs() < 0.5;
                                    let btn = egui::Button::new(
                                        RichText::new(*label).size(11.0).color(if selected {
                                            accent
                                        } else {
                                            text_primary
                                        }),
                                    )
                                    .fill(if selected { accent_dim } else { bg_elevated })
                                    .stroke(if selected {
                                        Stroke::new(1.0, accent)
                                    } else {
                                        Stroke::new(1.0, border)
                                    })
                                    .rounding(Rounding::same(3.0))
                                    .min_size(Vec2::new(52.0, 22.0));
                                    if ui.add(btn).clicked() {
                                        self.width = *pw;
                                        self.height = *ph;
                                    }
                                }
                            });
                            ui.add_space(16.0);

                            ui.label(
                                RichText::new("Dimensions (px)")
                                    .color(text_muted)
                                    .size(11.0),
                            );
                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("W").color(text_muted).size(12.0));
                                ui.add(
                                    egui::DragValue::new(&mut self.width)
                                        .speed(1.0)
                                        .range(1.0..=16384.0)
                                        .max_decimals(0),
                                );
                                ui.add_space(12.0);
                                ui.label(RichText::new("H").color(text_muted).size(12.0));
                                ui.add(
                                    egui::DragValue::new(&mut self.height)
                                        .speed(1.0)
                                        .range(1.0..=16384.0)
                                        .max_decimals(0),
                                );
                            });
                            ui.add_space(20.0);

                            let btn = egui::Button::new(
                                RichText::new("Create Document")
                                    .size(13.0)
                                    .color(Color32::WHITE)
                                    .strong(),
                            )
                            .fill(accent)
                            .stroke(Stroke::new(1.0, border_focus))
                            .rounding(Rounding::same(5.0))
                            .min_size(Vec2::new(ui.available_width(), 36.0));
                            if ui.add(btn).clicked() {
                                let name = self.doc_name.trim().to_string();
                                let name = if name.is_empty() {
                                    "Untitled".to_string()
                                } else {
                                    name
                                };
                                action = Some(WelcomeAction::CreateNew {
                                    name,
                                    width: self.width,
                                    height: self.height,
                                });
                            }
                        });

                        // ── Right: Recent Documents ───────────────────────
                        card_frame.show(&mut cols[1], |ui| {
                            ui.label(
                                RichText::new("Recent")
                                    .size(14.0)
                                    .color(text_primary)
                                    .strong(),
                            );
                            ui.add(egui::Separator::default().spacing(10.0));

                            if self.recent.is_empty() {
                                ui.add_space(16.0);
                                ui.label(
                                    RichText::new("No recent documents")
                                        .color(text_muted)
                                        .size(12.0)
                                        .italics(),
                                );
                                ui.add_space(16.0);
                            } else {
                                egui::ScrollArea::vertical()
                                    .max_height(240.0)
                                    .auto_shrink([false; 2])
                                    .show(ui, |ui| {
                                        let mut open_idx: Option<usize> = None;
                                        for (i, entry) in self.recent.iter().enumerate() {
                                            let file_name = entry
                                                .path
                                                .file_name()
                                                .map(|n| n.to_string_lossy().into_owned())
                                                .unwrap_or_else(|| entry.name.clone());
                                            let dir = entry
                                                .path
                                                .parent()
                                                .map(|p| p.to_string_lossy().into_owned())
                                                .unwrap_or_default();

                                            let row = egui::Frame::none()
                                                .fill(bg_elevated)
                                                .rounding(Rounding::same(4.0))
                                                .stroke(Stroke::new(1.0, border))
                                                .inner_margin(Margin::symmetric(10.0, 8.0));

                                            let resp = row.show(ui, |ui| {
                                                ui.vertical(|ui| {
                                                    ui.label(
                                                        RichText::new(&file_name)
                                                            .color(text_primary)
                                                            .size(12.0),
                                                    );
                                                    ui.label(
                                                        RichText::new(&dir)
                                                            .color(text_muted)
                                                            .size(10.0),
                                                    );
                                                });
                                            });

                                            let row_resp = ui.interact(
                                                resp.response.rect,
                                                ui.make_persistent_id(format!("recent_{i}")),
                                                egui::Sense::click(),
                                            );
                                            if row_resp.hovered() {
                                                ui.ctx().set_cursor_icon(
                                                    egui::CursorIcon::PointingHand,
                                                );
                                            }
                                            if row_resp.clicked() {
                                                open_idx = Some(i);
                                            }
                                            ui.add_space(4.0);
                                        }
                                        if let Some(i) = open_idx {
                                            action = Some(WelcomeAction::OpenFile(
                                                self.recent[i].path.clone(),
                                            ));
                                        }
                                    });
                                ui.add_space(12.0);
                            }

                            // Open File button — placed naturally in top-down flow,
                            // not with bottom_up (which bleeds out of the card).
                            let btn = egui::Button::new(
                                RichText::new("Open File…").size(13.0).color(text_primary),
                            )
                            .fill(bg_widget)
                            .stroke(Stroke::new(1.0, border))
                            .rounding(Rounding::same(5.0))
                            .min_size(Vec2::new(ui.available_width(), 36.0));
                            if ui.add(btn).clicked() {
                                action = Some(WelcomeAction::OpenBrowse);
                            }
                        });
                    });
                });
            });

        action
    }
}

// ─── Recent-docs persistence ──────────────────────────────────────────────────

fn recent_docs_path() -> Option<PathBuf> {
    std::env::var("APPDATA")
        .ok()
        .map(|a| PathBuf::from(a).join("Photonic").join("recent_docs.json"))
}

fn load_recent() -> Vec<RecentEntry> {
    recent_docs_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_recent(docs: &[RecentEntry]) {
    let Some(path) = recent_docs_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(docs) {
        let _ = std::fs::write(path, json);
    }
}
