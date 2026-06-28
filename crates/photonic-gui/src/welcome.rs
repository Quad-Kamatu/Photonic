//! Welcome / opening flow — a guided, cinematic landing screen.
//!
//! Rather than a static two-column form, the opening is a small state machine:
//!
//!   Hub  ──▶  New Canvas   (blank-document setup, aspect-ratio tiles + preview)
//!    │
//!    └────▶  Open          (recent documents as live preview cards + browse)
//!
//! Entrance and panel transitions are animated off egui's frame clock
//! (`input.time` + `animate_bool_with_time`) — a subtle, premium feel: short
//! fades, small drifts, gentle easing, no overshoot. Recent documents render a
//! true preview thumbnail, generated off-thread by the pure-CPU compositor and
//! uploaded as an egui texture once ready (a stylized placeholder shows first).

use egui::{
    Align2, Color32, FontId, Margin, Mesh, Pos2, Rect, RichText, Rounding, Sense, Stroke,
    TextureHandle, TextureOptions, Vec2,
};
use egui_phosphor::regular as ph;
use photonic_core::Document;
use photonic_render::{canvas::CanvasView, compositor::composite_document};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};

const MAX_RECENT: usize = 8;

// ─── Palette (shared with the dark editor theme) ───────────────────────────────

const ACCENT: Color32 = Color32::from_rgb(110, 86, 207);
const ACCENT_BRIGHT: Color32 = Color32::from_rgb(150, 128, 240);
const ACCENT_DIM: Color32 = Color32::from_rgb(61, 48, 128);
const BG_BASE: Color32 = Color32::from_rgb(7, 7, 11);
const BG_PANEL: Color32 = Color32::from_rgb(12, 12, 21);
const BG_ELEVATED: Color32 = Color32::from_rgb(19, 19, 31);
const BG_WIDGET: Color32 = Color32::from_rgb(26, 26, 40);
const BORDER: Color32 = Color32::from_rgb(30, 30, 50);
const TEXT_PRIMARY: Color32 = Color32::from_rgb(232, 232, 242);
const TEXT_MUTED: Color32 = Color32::from_rgb(122, 122, 154);

// Canvas size presets, grouped by use-case: (group, &[(label, width, height)]).
// Dimensions are in pixels (print sizes at 96 DPI). Each tile draws its true
// aspect ratio, so wide/tall/square presets are visually distinguishable.
const PRESET_GROUPS: &[(&str, &[(&str, f64, f64)])] = &[
    (
        "Print",
        &[
            ("A3", 1587.0, 2245.0),
            ("A4", 1123.0, 794.0),
            ("A5", 794.0, 559.0),
            ("Letter", 1056.0, 816.0),
            ("Legal", 816.0, 1344.0),
            ("Tabloid", 1056.0, 1632.0),
            ("Postcard", 576.0, 384.0),
            ("Card", 336.0, 192.0),
        ],
    ),
    (
        "Screen",
        &[
            ("720p", 1280.0, 720.0),
            ("1080p", 1920.0, 1080.0),
            ("1440p", 2560.0, 1440.0),
            ("4K", 3840.0, 2160.0),
            ("Ultrawide", 3440.0, 1440.0),
            ("Web", 1366.0, 768.0),
        ],
    ),
    (
        "Social",
        &[
            ("IG Post", 1080.0, 1080.0),
            ("IG Story", 1080.0, 1920.0),
            ("IG Portrait", 1080.0, 1350.0),
            ("TikTok", 1080.0, 1920.0),
            ("X Post", 1600.0, 900.0),
            ("YT Thumb", 1280.0, 720.0),
            ("Pinterest", 1000.0, 1500.0),
        ],
    ),
    (
        "Banners",
        &[
            ("YouTube", 2560.0, 1440.0),
            ("OG Image", 1200.0, 630.0),
            ("FB Cover", 820.0, 312.0),
            ("FB Event", 1920.0, 1005.0),
            ("X Header", 1500.0, 500.0),
            ("LinkedIn", 1584.0, 396.0),
            ("LinkedIn Co", 1128.0, 191.0),
            ("Twitch", 1200.0, 480.0),
            ("Twitch Panel", 320.0, 100.0),
        ],
    ),
    (
        "Square",
        &[
            ("Favicon", 64.0, 64.0),
            ("256", 256.0, 256.0),
            ("512", 512.0, 512.0),
            ("1024", 1024.0, 1024.0),
            ("2048", 2048.0, 2048.0),
        ],
    ),
    (
        "Device",
        &[
            ("iPhone", 1170.0, 2532.0),
            ("iPad", 1640.0, 2360.0),
            ("Android", 1080.0, 2340.0),
            ("Watch", 396.0, 484.0),
        ],
    ),
];

// ─── Types ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecentEntry {
    pub path: PathBuf,
    pub name: String,
}

pub enum WelcomeAction {
    CreateNew {
        name: String,
        /// Final canvas dimensions in pixels.
        width: f64,
        height: f64,
        /// Print bleed in millimetres (0 = none).
        bleed_mm: f64,
        /// Print slug area in millimetres (0 = none).
        slug_mm: f64,
        /// Uniform safe-area margin inset on all four sides, in pixels (0 = none).
        margin: f64,
        /// Number of same-size artboards to create, laid out in a grid (>= 1).
        artboards: usize,
    },
    OpenFile(PathBuf),
    OpenBrowse,
}

/// Unit the dimension fields are expressed in. Pixels are canonical; mm/in are
/// converted through the chosen DPI when computing the final canvas size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SizeUnit {
    Px,
    Mm,
    In,
}

impl SizeUnit {
    fn label(self) -> &'static str {
        match self {
            SizeUnit::Px => "px",
            SizeUnit::Mm => "mm",
            SizeUnit::In => "in",
        }
    }
    /// Pixels per one unit at the given DPI (DPI = pixels per inch).
    fn px_per_unit(self, dpi: f64) -> f64 {
        match self {
            SizeUnit::Px => 1.0,
            SizeUnit::In => dpi,
            SizeUnit::Mm => dpi / 25.4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WelcomeView {
    Hub,
    NewCanvas,
    Open,
}

// ─── State ────────────────────────────────────────────────────────────────────

pub struct WelcomeState {
    pub doc_name: String,
    pub width: f64,
    pub height: f64,
    pub recent: Vec<RecentEntry>,
    view: WelcomeView,
    /// Frame-clock time (`input.time`) at which the screen first drew — anchors
    /// the one-shot entrance reveal.
    appeared_at: Option<f64>,
    thumbs: Thumbnailer,
    // ── New Canvas advanced options ──
    unit: SizeUnit,
    dpi: f64,
    bleed_mm: f64,
    slug_mm: f64,
    margin: f64,
    num_artboards: usize,
    advanced_open: bool,
}

impl WelcomeState {
    pub fn new() -> Self {
        Self {
            doc_name: "Untitled".to_string(),
            width: 1123.0,
            height: 794.0,
            recent: load_recent(),
            view: WelcomeView::Hub,
            appeared_at: None,
            thumbs: Thumbnailer::new(),
            unit: SizeUnit::Px,
            dpi: 300.0,
            bleed_mm: 0.0,
            slug_mm: 0.0,
            margin: 0.0,
            num_artboards: 1,
            advanced_open: false,
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
        let t = ctx.input(|i| i.time);
        let appeared = *self.appeared_at.get_or_insert(t);
        let elapsed = (t - appeared) as f32;
        // Drive the one-shot entrance reveal to completion.
        if elapsed < 1.4 {
            ctx.request_repaint();
        }
        // Upload any thumbnails that finished rendering since last frame.
        self.thumbs.pump(ctx);

        let mut action: Option<WelcomeAction> = None;
        let mut next_view: Option<WelcomeView> = None;

        // ── Global keyboard shortcuts ────────────────────────────────────────
        ctx.input(|i| {
            use egui::Key;
            match self.view {
                WelcomeView::Hub => {
                    if i.key_pressed(Key::N) {
                        next_view = Some(WelcomeView::NewCanvas);
                    }
                    if i.key_pressed(Key::O) {
                        next_view = Some(WelcomeView::Open);
                    }
                }
                _ => {
                    if i.key_pressed(Key::Escape) {
                        next_view = Some(WelcomeView::Hub);
                    }
                }
            }
        });

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(BG_BASE))
            .show(ctx, |ui| {
                let full = ui.max_rect();
                // Ambient accent glow rising from behind the wordmark.
                paint_radial_glow(
                    ui.painter(),
                    Pos2::new(full.center().x, full.top() + full.height() * 0.30),
                    full.width().max(full.height()) * 0.62,
                    ACCENT,
                    22,
                );

                match self.view {
                    WelcomeView::Hub => {
                        self.draw_hub(ui, ctx, elapsed, &mut next_view);
                    }
                    WelcomeView::NewCanvas => {
                        self.draw_new(ui, ctx, &mut action, &mut next_view);
                    }
                    WelcomeView::Open => {
                        self.draw_open(ui, ctx, &mut action, &mut next_view);
                    }
                }
            });

        if let Some(v) = next_view {
            self.view = v;
            ctx.request_repaint();
        }
        action
    }

    // ── Hub: the landing fork ────────────────────────────────────────────────

    fn draw_hub(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        elapsed: f32,
        next_view: &mut Option<WelcomeView>,
    ) {
        // Staggered reveal: each element fades + drifts up off one shared clock.
        let r_word = reveal(elapsed, 0.00, 0.55);
        let r_sub = reveal(elapsed, 0.10, 0.55);
        let r_card1 = reveal(elapsed, 0.20, 0.55);
        let r_card2 = reveal(elapsed, 0.28, 0.55);

        let avail = ui.available_height();
        ui.add_space(avail * 0.18);

        // Wordmark + subtitle.
        ui.vertical_centered(|ui| {
            let p = ui.painter();
            let cx = ui.max_rect().center().x;
            let y = ui.cursor().top();
            p.text(
                Pos2::new(cx, y + lift(r_word)),
                Align2::CENTER_TOP,
                "PHOTONIC",
                FontId::proportional(40.0),
                fade(ACCENT, r_word),
            );
        });
        ui.add_space(46.0);
        ui.vertical_centered(|ui| {
            let p = ui.painter();
            let cx = ui.max_rect().center().x;
            let y = ui.cursor().top();
            p.text(
                Pos2::new(cx, y + lift(r_sub)),
                Align2::CENTER_TOP,
                "a modern graphics studio",
                FontId::proportional(13.5),
                fade(TEXT_MUTED, r_sub),
            );
        });
        ui.add_space(54.0);

        // Two hero cards, centered.
        let card_w = 218.0;
        let card_h = 158.0;
        let gap = 22.0;
        let recent_hint = if self.recent.is_empty() {
            "Pick up where you left off".to_string()
        } else if self.recent.len() == 1 {
            "1 recent document".to_string()
        } else {
            format!("{} recent documents", self.recent.len())
        };

        ui.vertical_centered(|ui| {
            ui.set_max_width(card_w * 2.0 + gap);
            ui.horizontal(|ui| {
                if hero_card(
                    ui,
                    ctx,
                    "hub_new",
                    Vec2::new(card_w, card_h),
                    ph::SPARKLE,
                    "New Canvas",
                    "Start something blank",
                    r_card1,
                ) {
                    *next_view = Some(WelcomeView::NewCanvas);
                }
                ui.add_space(gap);
                if hero_card(
                    ui,
                    ctx,
                    "hub_open",
                    Vec2::new(card_w, card_h),
                    ph::CLOCK_COUNTER_CLOCKWISE,
                    "Open",
                    &recent_hint,
                    r_card2,
                ) {
                    *next_view = Some(WelcomeView::Open);
                }
            });
        });

        // Quiet hint strip at the very bottom.
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height().max(20.0) - 38.0);
            ui.scope(|ui| {
                ui.set_opacity(r_card2 * 0.8);
                ui.label(
                    RichText::new("N  new canvas      O  open")
                        .size(11.0)
                        .color(TEXT_MUTED),
                );
            });
        });
    }

    // ── New Canvas panel ─────────────────────────────────────────────────────

    fn draw_new(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        action: &mut Option<WelcomeAction>,
        next_view: &mut Option<WelcomeView>,
    ) {
        let anim = ctx.animate_bool_with_time(egui::Id::new("welcome_anim_new"), true, 0.18);
        panel_chrome(ui, ctx, "New canvas", next_view);

        let panel_w = 600.0_f32.min(ui.available_width() - 48.0);
        ui.add_space(8.0);
        ui.vertical_centered(|ui| {
            ui.set_max_width(panel_w);
            ui.scope(|ui| {
                ui.set_opacity(anim);
                let off = lift(anim);
                ui.add_space(off);

                let card = egui::Frame::none()
                    .fill(BG_PANEL)
                    .rounding(Rounding::same(10.0))
                    .stroke(Stroke::new(1.0, BORDER))
                    .inner_margin(Margin::same(22.0));
                card.show(ui, |ui| {
                    // Name.
                    ui.label(RichText::new("NAME").color(TEXT_MUTED).size(10.5));
                    ui.add_space(4.0);
                    ui.add(
                        egui::TextEdit::singleline(&mut self.doc_name)
                            .desired_width(f32::INFINITY)
                            .font(egui::TextStyle::Body),
                    );
                    ui.add_space(18.0);

                    // Aspect-ratio preset tiles, grouped by use-case, in a
                    // bounded scroll area. Selecting a tile sets W/H; typing in
                    // the always-visible dimension fields deselects all tiles.
                    ui.label(RichText::new("CANVAS SIZE").color(TEXT_MUTED).size(10.5));
                    ui.add_space(8.0);
                    let mut pick: Option<(f64, f64)> = None;
                    egui::ScrollArea::vertical()
                        .id_source("canvas_size_scroll")
                        .max_height(286.0)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            for (gi, (group, items)) in PRESET_GROUPS.iter().enumerate() {
                                if gi > 0 {
                                    ui.add_space(10.0);
                                }
                                ui.label(
                                    RichText::new(*group)
                                        .color(TEXT_MUTED)
                                        .size(10.0)
                                        .strong(),
                                );
                                ui.add_space(6.0);
                                ui.horizontal_wrapped(|ui| {
                                    ui.spacing_mut().item_spacing = Vec2::new(9.0, 9.0);
                                    for (label, pw, ph_) in *items {
                                        let selected = (self.width - pw).abs() < 0.5
                                            && (self.height - ph_).abs() < 0.5;
                                        if aspect_tile(ui, ctx, label, *pw, *ph_, selected) {
                                            pick = Some((*pw, *ph_));
                                        }
                                    }
                                });
                            }
                        });
                    if let Some((w, h)) = pick {
                        self.width = w;
                        self.height = h;
                    }
                    ui.add_space(14.0);

                    // Dimension fields, expressed in the chosen unit (px canonical).
                    // Editing them deselects all presets.
                    let ppu = self.unit.px_per_unit(self.dpi);
                    let decimals = if self.unit == SizeUnit::Px { 0 } else { 2 };
                    let speed = if self.unit == SizeUnit::Px { 1.0 } else { 0.1 };
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("W").color(TEXT_MUTED).size(12.0));
                        let mut w = self.width / ppu;
                        if ui
                            .add(
                                egui::DragValue::new(&mut w)
                                    .speed(speed)
                                    .range(0.01..=100_000.0)
                                    .max_decimals(decimals),
                            )
                            .changed()
                        {
                            self.width = (w * ppu).clamp(1.0, 16384.0);
                        }
                        ui.add_space(12.0);
                        ui.label(RichText::new("H").color(TEXT_MUTED).size(12.0));
                        let mut h = self.height / ppu;
                        if ui
                            .add(
                                egui::DragValue::new(&mut h)
                                    .speed(speed)
                                    .range(0.01..=100_000.0)
                                    .max_decimals(decimals),
                            )
                            .changed()
                        {
                            self.height = (h * ppu).clamp(1.0, 16384.0);
                        }

                        // Unit selector segmented control.
                        ui.add_space(16.0);
                        for u in [SizeUnit::Px, SizeUnit::Mm, SizeUnit::In] {
                            if mini_toggle(ui, u.label(), self.unit == u) {
                                self.unit = u;
                            }
                        }
                    });
                    ui.add_space(12.0);

                    // ── Advanced (DPI, bleed, slug, safe-area margin) ─────────
                    let caret = if self.advanced_open {
                        ph::CARET_DOWN
                    } else {
                        ph::CARET_RIGHT
                    };
                    let adv = ui.add(
                        egui::Label::new(
                            RichText::new(format!("{caret}  Advanced"))
                                .size(11.5)
                                .color(TEXT_MUTED),
                        )
                        .sense(Sense::click()),
                    );
                    if adv.hovered() {
                        ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    if adv.clicked() {
                        self.advanced_open = !self.advanced_open;
                    }

                    if self.advanced_open {
                        ui.add_space(8.0);
                        let inner = egui::Frame::none()
                            .fill(BG_BASE)
                            .rounding(Rounding::same(6.0))
                            .stroke(Stroke::new(1.0, BORDER))
                            .inner_margin(Margin::same(14.0));
                        inner.show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 10.0;

                            // Resolution (DPI/PPI) — drives mm/in ⇄ px conversion.
                            adv_row(ui, "Resolution", "Pixels per inch (print quality).", |ui| {
                                ui.add(
                                    egui::DragValue::new(&mut self.dpi)
                                        .speed(1.0)
                                        .range(1.0..=2400.0)
                                        .max_decimals(0)
                                        .suffix(" DPI"),
                                );
                                ui.add_space(8.0);
                                for d in [72.0, 96.0, 150.0, 300.0, 600.0] {
                                    if mini_toggle(ui, &format!("{}", d as i64), (self.dpi - d).abs() < 0.5) {
                                        self.dpi = d;
                                    }
                                }
                            });

                            // Bleed (print) — stored as Document::bleed_mm.
                            adv_row(ui, "Bleed", "Extra print area on all sides.", |ui| {
                                ui.add(
                                    egui::DragValue::new(&mut self.bleed_mm)
                                        .speed(0.1)
                                        .range(0.0..=50.0)
                                        .max_decimals(2)
                                        .suffix(" mm"),
                                );
                                ui.add_space(8.0);
                                if mini_toggle(ui, "3mm EU", (self.bleed_mm - 3.0).abs() < 0.05) {
                                    self.bleed_mm = 3.0;
                                }
                                if mini_toggle(ui, "1/8\" US", (self.bleed_mm - 3.175).abs() < 0.05) {
                                    self.bleed_mm = 3.175;
                                }
                            });

                            // Slug (print) — stored as Document::slug_mm.
                            adv_row(ui, "Slug", "Margin outside the bleed for marks.", |ui| {
                                ui.add(
                                    egui::DragValue::new(&mut self.slug_mm)
                                        .speed(0.1)
                                        .range(0.0..=50.0)
                                        .max_decimals(2)
                                        .suffix(" mm"),
                                );
                            });

                            // Safe-area margin — stored on all four Document margins.
                            adv_row(ui, "Safe margin", "Inset guide on all four sides.", |ui| {
                                ui.add(
                                    egui::DragValue::new(&mut self.margin)
                                        .speed(1.0)
                                        .range(0.0..=4096.0)
                                        .max_decimals(0)
                                        .suffix(" px"),
                                );
                            });

                            // Artboards — N copies laid out in a grid in one doc.
                            adv_row(
                                ui,
                                "Artboards",
                                "Create several same-size artboards in a grid.",
                                |ui| {
                                    let mut n = self.num_artboards as i64;
                                    if ui
                                        .add(
                                            egui::DragValue::new(&mut n)
                                                .speed(0.1)
                                                .range(1..=64)
                                                .max_decimals(0),
                                        )
                                        .changed()
                                    {
                                        self.num_artboards = n.clamp(1, 64) as usize;
                                    }
                                    ui.add_space(8.0);
                                    for c in [1_usize, 2, 4, 6] {
                                        if mini_toggle(ui, &format!("{c}"), self.num_artboards == c) {
                                            self.num_artboards = c;
                                        }
                                    }
                                },
                            );
                        });
                    }
                    ui.add_space(16.0);

                    // Create button — carries the chosen size + print options.
                    let mut sub = format!("{} × {} px", self.width as i64, self.height as i64);
                    if self.num_artboards > 1 {
                        sub.push_str(&format!("  ·  {} artboards", self.num_artboards));
                    }
                    if self.bleed_mm > 0.0 {
                        sub.push_str(&format!("  ·  bleed {}mm", trim_num(self.bleed_mm)));
                    }
                    let label = format!("{}  Create  ·  {}", ph::SPARKLE, sub);
                    let btn = egui::Button::new(
                        RichText::new(label).size(13.0).color(Color32::WHITE).strong(),
                    )
                    .fill(ACCENT)
                    .stroke(Stroke::new(1.0, ACCENT_BRIGHT))
                    .rounding(Rounding::same(6.0))
                    .min_size(Vec2::new(ui.available_width(), 40.0));
                    let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if ui.add(btn).clicked() || enter {
                        let name = self.doc_name.trim();
                        let name = if name.is_empty() { "Untitled" } else { name };
                        *action = Some(WelcomeAction::CreateNew {
                            name: name.to_string(),
                            width: self.width,
                            height: self.height,
                            bleed_mm: self.bleed_mm,
                            slug_mm: self.slug_mm,
                            margin: self.margin,
                            artboards: self.num_artboards,
                        });
                    }
                });
            });
        });
    }

    // ── Open panel ───────────────────────────────────────────────────────────

    fn draw_open(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        action: &mut Option<WelcomeAction>,
        next_view: &mut Option<WelcomeView>,
    ) {
        let anim = ctx.animate_bool_with_time(egui::Id::new("welcome_anim_open"), true, 0.18);
        panel_chrome(ui, ctx, "Open", next_view);

        let panel_w = 600.0_f32.min(ui.available_width() - 48.0);
        ui.add_space(8.0);
        ui.vertical_centered(|ui| {
            ui.set_max_width(panel_w);
            ui.scope(|ui| {
                ui.set_opacity(anim);
                ui.add_space(lift(anim));

                let card_w = 178.0;
                let card_h = 168.0;
                let gap = 16.0;
                let per_row = ((panel_w + gap) / (card_w + gap)).floor().max(1.0) as usize;

                egui::ScrollArea::vertical()
                    .max_height(ui.available_height() - 12.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing = Vec2::new(gap, gap);
                        // Build a flat list: the browse tile first, then recents.
                        let count = self.recent.len() + 1;
                        let mut idx = 0;
                        while idx < count {
                            ui.horizontal(|ui| {
                                for _ in 0..per_row {
                                    if idx >= count {
                                        break;
                                    }
                                    if idx == 0 {
                                        if browse_card(ui, ctx, Vec2::new(card_w, card_h)) {
                                            *action = Some(WelcomeAction::OpenBrowse);
                                        }
                                    } else {
                                        let path = self.recent[idx - 1].path.clone();
                                        let thumb = self.thumbs.get(ctx, &path);
                                        let entry = &self.recent[idx - 1];
                                        if recent_card(
                                            ui,
                                            ctx,
                                            idx,
                                            Vec2::new(card_w, card_h),
                                            entry,
                                            Some(&thumb),
                                        ) {
                                            *action = Some(WelcomeAction::OpenFile(path.clone()));
                                        }
                                    }
                                    idx += 1;
                                }
                            });
                        }
                        if self.recent.is_empty() {
                            ui.add_space(14.0);
                            ui.label(
                                RichText::new("No recent documents yet — open one to see it here.")
                                    .color(TEXT_MUTED)
                                    .size(12.0)
                                    .italics(),
                            );
                        }
                    });
            });
        });
    }
}

// ─── Shared chrome: shrunk wordmark + back affordance ──────────────────────────

fn panel_chrome(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    title: &str,
    next_view: &mut Option<WelcomeView>,
) {
    ui.add_space(22.0);
    ui.horizontal(|ui| {
        ui.add_space(28.0);
        // Back chevron.
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(34.0, 28.0), Sense::click());
        let hot = resp.hovered();
        if hot {
            ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            ph::ARROW_LEFT,
            FontId::proportional(18.0),
            if hot { TEXT_PRIMARY } else { TEXT_MUTED },
        );
        if resp.clicked() {
            *next_view = Some(WelcomeView::Hub);
        }
        ui.add_space(2.0);
        ui.label(
            RichText::new("PHOTONIC")
                .size(13.0)
                .color(ACCENT)
                .strong(),
        );
        ui.add_space(10.0);
        ui.label(RichText::new("/").size(13.0).color(BORDER));
        ui.add_space(10.0);
        ui.label(RichText::new(title).size(13.0).color(TEXT_PRIMARY));
    });
    ui.add_space(10.0);
    let sep_y = ui.cursor().top();
    ui.painter().hline(
        ui.max_rect().x_range(),
        sep_y,
        Stroke::new(1.0, Color32::from_rgb(25, 25, 42)),
    );
    ui.add_space(14.0);
}

// ─── Small advanced-panel widgets ──────────────────────────────────────────────

/// A compact pill toggle used for unit / DPI / bleed quick-picks.
fn mini_toggle(ui: &mut egui::Ui, label: &str, selected: bool) -> bool {
    let btn = egui::Button::new(
        RichText::new(label)
            .size(11.0)
            .color(if selected { Color32::WHITE } else { TEXT_MUTED }),
    )
    .fill(if selected { ACCENT } else { BG_ELEVATED })
    .stroke(Stroke::new(
        1.0,
        if selected { ACCENT_BRIGHT } else { BORDER },
    ))
    .rounding(Rounding::same(4.0))
    .min_size(Vec2::new(0.0, 22.0));
    ui.add(btn).clicked()
}

/// One labeled row in the advanced panel: a fixed-width title (with a hover
/// hint) followed by its controls.
fn adv_row(ui: &mut egui::Ui, title: &str, hint: &str, content: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(116.0, 22.0), Sense::hover());
        ui.painter().text(
            Pos2::new(rect.left(), rect.center().y),
            Align2::LEFT_CENTER,
            title,
            FontId::proportional(12.0),
            TEXT_PRIMARY,
        );
        resp.on_hover_text(hint);
        content(ui);
    });
}

/// Format a number without trailing zeros (e.g. 3.0 → "3", 3.175 → "3.18").
fn trim_num(x: f64) -> String {
    let r = (x * 100.0).round() / 100.0;
    if r.fract().abs() < 1e-9 {
        format!("{}", r as i64)
    } else {
        format!("{r:.2}").trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

// ─── Hero card (Hub) ───────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn hero_card(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    id: &str,
    size: Vec2,
    icon: &str,
    title: &str,
    subtitle: &str,
    reveal: f32,
) -> bool {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let hov = ctx.animate_bool_with_time(egui::Id::new(id), resp.hovered(), 0.12);
    if resp.hovered() {
        ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    // Reveal: fade + a small upward drift that settles to zero.
    let draw = rect.translate(Vec2::new(0.0, lift(reveal)));
    // Hover micro-lift: a hair of vertical rise.
    let draw = draw.translate(Vec2::new(0.0, -3.0 * hov));
    let p = ui.painter();

    let fill = lerp_color(BG_PANEL, BG_ELEVATED, hov);
    let stroke = lerp_color(BORDER, ACCENT, hov);
    p.rect(
        draw,
        Rounding::same(12.0),
        fade(fill, reveal),
        Stroke::new(1.0 + hov, fade(stroke, reveal)),
    );

    // Icon in an accent chip near the top.
    let icon_c = lerp_color(ACCENT, ACCENT_BRIGHT, hov);
    p.text(
        Pos2::new(draw.left() + 22.0, draw.top() + 26.0),
        Align2::LEFT_TOP,
        icon,
        FontId::proportional(30.0),
        fade(icon_c, reveal),
    );
    // Title + subtitle anchored to the lower-left.
    p.text(
        Pos2::new(draw.left() + 22.0, draw.bottom() - 50.0),
        Align2::LEFT_TOP,
        title,
        FontId::proportional(18.0),
        fade(TEXT_PRIMARY, reveal),
    );
    p.text(
        Pos2::new(draw.left() + 22.0, draw.bottom() - 26.0),
        Align2::LEFT_TOP,
        subtitle,
        FontId::proportional(12.0),
        fade(TEXT_MUTED, reveal),
    );

    resp.clicked()
}

// ─── Aspect-ratio preset tile (New Canvas) ─────────────────────────────────────

fn aspect_tile(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    label: &str,
    pw: f64,
    ph_: f64,
    selected: bool,
) -> bool {
    let size = Vec2::new(88.0, 84.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let hov = ctx.animate_bool_with_time(
        egui::Id::new(("aspect", label, pw as i64, ph_ as i64)),
        resp.hovered(),
        0.1,
    );
    if resp.hovered() {
        ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let p = ui.painter();

    let fill = if selected {
        ACCENT_DIM
    } else {
        lerp_color(BG_ELEVATED, BG_WIDGET, hov)
    };
    let stroke = if selected {
        ACCENT
    } else {
        lerp_color(BORDER, ACCENT, hov * 0.6)
    };
    p.rect(rect, Rounding::same(6.0), fill, Stroke::new(1.0, stroke));

    // Proportional rectangle representing the aspect ratio, sat in the top
    // third of the tile so the label + dimensions have clear room below it.
    let max_box = Vec2::new(40.0, 26.0);
    let ar = (pw / ph_) as f32;
    let (rw, rh) = if ar >= max_box.x / max_box.y {
        (max_box.x, max_box.x / ar)
    } else {
        (max_box.y * ar, max_box.y)
    };
    let center = Pos2::new(rect.center().x, rect.top() + 22.0);
    let prect = Rect::from_center_size(center, Vec2::new(rw, rh));
    let pr_fill = if selected { ACCENT_BRIGHT } else { TEXT_MUTED };
    p.rect(prect, Rounding::same(2.0), pr_fill, Stroke::NONE);

    p.text(
        Pos2::new(rect.center().x, rect.top() + 45.0),
        Align2::CENTER_TOP,
        label,
        FontId::proportional(12.5),
        TEXT_PRIMARY,
    );
    p.text(
        Pos2::new(rect.center().x, rect.top() + 62.0),
        Align2::CENTER_TOP,
        format!("{}×{}", pw as i64, ph_ as i64),
        FontId::proportional(8.5),
        TEXT_MUTED,
    );

    resp.clicked()
}

// ─── Recent + browse cards (Open) ──────────────────────────────────────────────

fn browse_card(ui: &mut egui::Ui, ctx: &egui::Context, size: Vec2) -> bool {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let hov = ctx.animate_bool_with_time(egui::Id::new("open_browse"), resp.hovered(), 0.12);
    if resp.hovered() {
        ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let p = ui.painter();
    let draw = rect.translate(Vec2::new(0.0, -3.0 * hov));
    let stroke = lerp_color(BORDER, ACCENT, hov);
    // Ghost card — subtle fill, dashed-feel via lighter stroke.
    p.rect(
        draw,
        Rounding::same(10.0),
        lerp_color(BG_BASE, BG_PANEL, 0.5 + hov * 0.5),
        Stroke::new(1.0, stroke),
    );
    p.text(
        draw.center() - Vec2::new(0.0, 12.0),
        Align2::CENTER_CENTER,
        ph::FOLDER_OPEN,
        FontId::proportional(34.0),
        lerp_color(TEXT_MUTED, ACCENT_BRIGHT, hov),
    );
    p.text(
        Pos2::new(draw.center().x, draw.bottom() - 26.0),
        Align2::CENTER_TOP,
        "Open file…",
        FontId::proportional(13.0),
        TEXT_PRIMARY,
    );
    resp.clicked()
}

fn recent_card(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    idx: usize,
    size: Vec2,
    entry: &RecentEntry,
    thumb: Option<&Thumb>,
) -> bool {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let hov = ctx.animate_bool_with_time(egui::Id::new(("recent", idx)), resp.hovered(), 0.12);
    if resp.hovered() {
        ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let draw = rect.translate(Vec2::new(0.0, -3.0 * hov));
    let p = ui.painter();

    // Card body.
    p.rect(
        draw,
        Rounding::same(10.0),
        lerp_color(BG_PANEL, BG_ELEVATED, hov),
        Stroke::new(1.0, lerp_color(BORDER, ACCENT, hov)),
    );

    // Thumbnail well (top portion).
    let pad = 8.0;
    let well = Rect::from_min_max(
        draw.left_top() + Vec2::new(pad, pad),
        Pos2::new(draw.right() - pad, draw.top() + size.y - 42.0),
    );
    p.rect(well, Rounding::same(6.0), Color32::from_rgb(15, 15, 24), Stroke::NONE);

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

    match thumb {
        Some(Thumb::Ready(tex)) => {
            // Fit the preview inside the well, preserving aspect.
            let isz = tex.size_vec2();
            let ar = isz.x / isz.y.max(1.0);
            let (mut w, mut h) = (well.width() - 8.0, (well.width() - 8.0) / ar);
            if h > well.height() - 8.0 {
                h = well.height() - 8.0;
                w = h * ar;
            }
            let irect = Rect::from_center_size(well.center(), Vec2::new(w, h));
            // Soft drop shadow behind the artboard.
            p.rect(
                irect.translate(Vec2::new(0.0, 2.0)),
                Rounding::same(2.0),
                Color32::from_black_alpha(120),
                Stroke::NONE,
            );
            p.image(
                tex.id(),
                irect,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        _ => {
            // Stylized placeholder: accent wash + the file's initial.
            placeholder_thumb(p, well, &file_name, matches!(thumb, Some(Thumb::Failed)));
        }
    }

    // Filename + directory.
    p.text(
        Pos2::new(draw.left() + pad + 2.0, draw.bottom() - 32.0),
        Align2::LEFT_TOP,
        elide(&file_name, 22),
        FontId::proportional(12.5),
        TEXT_PRIMARY,
    );
    p.text(
        Pos2::new(draw.left() + pad + 2.0, draw.bottom() - 16.0),
        Align2::LEFT_TOP,
        elide(&dir, 26),
        FontId::proportional(9.5),
        TEXT_MUTED,
    );

    resp.clicked()
}

fn placeholder_thumb(p: &egui::Painter, well: Rect, name: &str, failed: bool) {
    // Faint accent gradient via a vertical two-tri mesh.
    let mut mesh = Mesh::default();
    let top = fade(ACCENT_DIM, 0.55);
    let bot = fade(BG_WIDGET, 0.9);
    mesh.colored_vertex(well.left_top(), top);
    mesh.colored_vertex(well.right_top(), top);
    mesh.colored_vertex(well.right_bottom(), bot);
    mesh.colored_vertex(well.left_bottom(), bot);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    // Clip the mesh to the rounded well by painting it then a frame.
    p.add(mesh);
    if failed {
        p.text(
            well.center(),
            Align2::CENTER_CENTER,
            ph::IMAGE,
            FontId::proportional(22.0),
            TEXT_MUTED,
        );
    } else {
        let initial = name
            .chars()
            .find(|c| c.is_alphanumeric())
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_else(|| "·".to_string());
        p.text(
            well.center(),
            Align2::CENTER_CENTER,
            initial,
            FontId::proportional(30.0),
            fade(TEXT_PRIMARY, 0.85),
        );
    }
}

// ─── Animation + paint helpers ─────────────────────────────────────────────────

/// Cubic ease-out, 0 → 1.
fn ease_out_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// Reveal progress (0 → 1) for an element starting at `delay` over `dur`.
fn reveal(elapsed: f32, delay: f32, dur: f32) -> f32 {
    ease_out_cubic((elapsed - delay) / dur)
}

/// Vertical drift for a reveal: 10px → 0 as progress 0 → 1.
fn lift(reveal: f32) -> f32 {
    10.0 * (1.0 - reveal)
}

/// Multiply a colour's alpha by `a` (0 → 1).
fn fade(c: Color32, a: f32) -> Color32 {
    let a = a.clamp(0.0, 1.0);
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (c.a() as f32 * a) as u8)
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t) as u8;
    Color32::from_rgba_unmultiplied(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()), l(a.a(), b.a()))
}

/// Soft radial glow as a triangle fan: opaque-ish centre → transparent edge.
fn paint_radial_glow(p: &egui::Painter, center: Pos2, radius: f32, color: Color32, center_alpha: u8) {
    let mut mesh = Mesh::default();
    let c = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), center_alpha);
    let edge = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 0);
    mesh.colored_vertex(center, c);
    let n = 48;
    for i in 0..=n {
        let ang = (i as f32 / n as f32) * std::f32::consts::TAU;
        mesh.colored_vertex(center + Vec2::new(ang.cos(), ang.sin()) * radius, edge);
        if i > 0 {
            mesh.add_triangle(0, i, i + 1);
        }
    }
    p.add(mesh);
}

fn elide(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let kept: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{kept}…")
    }
}

// ─── Thumbnail generation (off-thread CPU compositor → egui texture) ────────────

#[derive(Clone)]
enum Thumb {
    Pending,
    Ready(TextureHandle),
    Failed,
}

type ThumbResult = (PathBuf, Option<(u32, u32, Vec<u8>)>);

struct Thumbnailer {
    req_tx: Sender<PathBuf>,
    res_rx: Receiver<ThumbResult>,
    state: HashMap<PathBuf, Thumb>,
}

impl Thumbnailer {
    fn new() -> Self {
        let (req_tx, req_rx) = channel::<PathBuf>();
        let (res_tx, res_rx) = channel::<ThumbResult>();
        std::thread::Builder::new()
            .name("photonic-thumbs".into())
            .spawn(move || {
                while let Ok(path) = req_rx.recv() {
                    let out = render_thumb(&path);
                    if res_tx.send((path, out)).is_err() {
                        break;
                    }
                }
            })
            .ok();
        Self {
            req_tx,
            res_rx,
            state: HashMap::new(),
        }
    }

    /// Upload any thumbnails that finished rendering since the last frame.
    fn pump(&mut self, ctx: &egui::Context) {
        while let Ok((path, out)) = self.res_rx.try_recv() {
            let thumb = match out {
                Some((w, h, rgba)) => {
                    let img =
                        egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba);
                    let name = format!("thumb_{}", path.to_string_lossy());
                    Thumb::Ready(ctx.load_texture(name, img, TextureOptions::LINEAR))
                }
                None => Thumb::Failed,
            };
            self.state.insert(path, thumb);
        }
    }

    /// Get the thumbnail for `path`, requesting generation on first sight.
    fn get(&mut self, _ctx: &egui::Context, path: &Path) -> Thumb {
        if let Some(t) = self.state.get(path) {
            return t.clone();
        }
        let _ = self.req_tx.send(path.to_path_buf());
        self.state.insert(path.to_path_buf(), Thumb::Pending);
        Thumb::Pending
    }
}

/// Render a document to a fit-to-artboard RGBA8 thumbnail using the pure-CPU
/// compositor. Returns `(width, height, rgba)` or `None` on any load/parse error.
fn render_thumb(path: &Path) -> Option<(u32, u32, Vec<u8>)> {
    let doc = load_doc(path).ok()?;
    let dw = doc.width.max(1.0);
    let dh = doc.height.max(1.0);
    const MAX_EDGE: f64 = 360.0;
    let scale = (MAX_EDGE / dw.max(dh)).min(2.0);
    let tw = (dw * scale).round().clamp(1.0, 720.0) as u32;
    let th = (dh * scale).round().clamp(1.0, 720.0) as u32;

    let mut view = CanvasView::new(tw, th);
    view.zoom = scale;
    view.pan_x = 0.0;
    view.pan_y = 0.0;

    // Pre-fill with a near-white artboard so previews read as documents.
    let mut buf = vec![0u8; (tw as usize) * (th as usize) * 4];
    for px in buf.chunks_exact_mut(4) {
        px[0] = 250;
        px[1] = 250;
        px[2] = 252;
        px[3] = 255;
    }
    composite_document(&mut buf, tw, th, &doc, &view);
    Some((tw, th, buf))
}

fn load_doc(path: &Path) -> Result<Document, String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    if ext == "svg" {
        photonic_core::import_svg(&content).map_err(|e| e.to_string())
    } else {
        Document::from_json(&content).map_err(|e| e.to_string())
    }
}

// ─── Recent-docs persistence ──────────────────────────────────────────────────

/// Cross-platform Photonic config directory.
///
/// Prefers `%APPDATA%\Photonic` on Windows (matching the rest of the app), then
/// falls back to `$XDG_CONFIG_HOME/Photonic` / `~/.config/Photonic` so recent
/// documents load on Linux and macOS too.
fn config_dir() -> Option<PathBuf> {
    if let Ok(appdata) = std::env::var("APPDATA") {
        return Some(PathBuf::from(appdata).join("Photonic"));
    }
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(xdg).join("Photonic"));
    }
    if let Ok(home) = std::env::var("HOME") {
        return Some(PathBuf::from(home).join(".config").join("Photonic"));
    }
    None
}

fn recent_docs_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("recent_docs.json"))
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
