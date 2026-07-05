//! The single, industry-grade color picker used across Photonic.
//!
//! Every color affordance — inline swatch buttons, the floating radial-menu
//! picker, and the inspector's "recolor matching" popover — routes through
//! [`ColorPopup`]. One correct color path, one visual language.
//!
//! Features: custom SV square + hue bar/wheel + alpha; hex entry; RGB/HSB/HSL/
//! **OKLCH** numeric models; recent + document swatches; inline eyedropper;
//! perceptual tint/shade ramp; color-harmony chips; WCAG contrast readout and
//! color-blindness preview. Keyboard-navigable.
//!
//! ## Color convention (issue #185)
//! Photonic stores **gamma-encoded sRGB** `[f32; 4]`. All public entry points
//! speak that; conversions live in [`crate::color_convert`].

use egui::{
    epaint::Vertex, Color32, Mesh, Pos2, Rect, Response, Sense, Shape, Stroke, Ui, Vec2,
};
use photonic_core::Color;

use crate::color_convert as cc;

const ACCENT: Color32 = Color32::from_rgb(130, 105, 225);

// ── Public config / outcome ───────────────────────────────────────────────────

/// Optional context that lights up the richer picker features. Defaults to a
/// self-contained "lite" picker (SV/hue/alpha, hex, models, ramp, harmony,
/// contrast, CVD) with no app integration.
#[derive(Clone, Copy)]
pub struct PickerConfig<'a> {
    /// Show the alpha bar + alpha inputs.
    pub alpha: bool,
    /// Recently-used colors, most-recent first (rendered as a strip).
    pub recents: &'a [[f32; 4]],
    /// Document/palette swatches (rendered as a strip with an "add" button).
    pub swatches: &'a [[f32; 4]],
    /// Show the eyedropper button (caller wires the click to its sampler).
    pub eyedropper: bool,
    /// Show the "SWATCHES" section with an add button (caller must handle
    /// [`PickerOutcome::add_swatch`]); the existing `swatches` strip still shows
    /// regardless when non-empty.
    pub allow_add_swatch: bool,
    /// Reference color to report WCAG contrast against (e.g. the artboard).
    pub contrast_ref: Option<[f32; 3]>,
}

impl Default for PickerConfig<'_> {
    fn default() -> Self {
        Self {
            alpha: true,
            recents: &[],
            swatches: &[],
            eyedropper: false,
            allow_add_swatch: false,
            contrast_ref: None,
        }
    }
}

/// What the picker reported this frame.
#[derive(Default, Clone, Copy)]
pub struct PickerOutcome {
    /// The color changed.
    pub changed: bool,
    /// The eyedropper button was clicked.
    pub eyedropper_clicked: bool,
    /// The user asked to save this color to swatches.
    pub add_swatch: Option<[f32; 4]>,
}

/// The numeric-entry model shown under the hex field.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ColorModel {
    Rgb,
    Hsb,
    Hsl,
    Oklch,
}

impl ColorModel {
    const ALL: [ColorModel; 4] = [Self::Rgb, Self::Hsb, Self::Hsl, Self::Oklch];
    fn label(self) -> &'static str {
        match self {
            Self::Rgb => "RGB",
            Self::Hsb => "HSB",
            Self::Hsl => "HSL",
            Self::Oklch => "OKLCH",
        }
    }
}

/// HSV state persisted per-picker so hue survives value/saturation hitting zero.
#[derive(Clone, Copy)]
struct PickerState {
    h: f32,
    s: f32,
    v: f32,
}

const WIDTH: f32 = 264.0;
const SV_H: f32 = 150.0;
const BAR_W: f32 = 14.0;
const GAP: f32 = 6.0;

/// Photonic's one color picker. Stateless helpers; callers own the color store.
pub struct ColorPopup;

impl ColorPopup {
    // ── conversions ──
    fn to_c32(rgba: [f32; 4]) -> Color32 {
        let b = cc::rgba_to_bytes(rgba);
        Color32::from_rgba_unmultiplied(b[0], b[1], b[2], b[3])
    }

    /// Inline **swatch button** over a gamma-sRGB `[f32; 4]`. Clicking opens the
    /// full picker in a popup. Returns the button `Response` (`.changed()` fires
    /// when the popup edits the color).
    pub fn swatch_f32(ui: &mut Ui, rgba: &mut [f32; 4]) -> Response {
        Self::swatch_button(ui, rgba, PickerConfig::default())
    }

    /// Inline **swatch button** over a [`photonic_core::Color`].
    pub fn swatch_color(ui: &mut Ui, color: &mut Color) -> Response {
        let mut rgba = [color.r, color.g, color.b, color.a];
        let resp = Self::swatch_button(ui, &mut rgba, PickerConfig::default());
        if resp.changed() {
            color.r = rgba[0];
            color.g = rgba[1];
            color.b = rgba[2];
            color.a = rgba[3];
        }
        resp
    }

    /// The swatch button + popup implementation shared by the inline entry points.
    fn swatch_button(ui: &mut Ui, rgba: &mut [f32; 4], cfg: PickerConfig) -> Response {
        let size = Vec2::splat(ui.spacing().interact_size.y.max(16.0));
        let (rect, mut resp) = ui.allocate_exact_size(size, Sense::click());
        paint_checker(ui.painter(), rect);
        ui.painter()
            .rect_filled(rect, egui::Rounding::same(3.0), Self::to_c32(*rgba));
        ui.painter().rect_stroke(
            rect,
            egui::Rounding::same(3.0),
            Stroke::new(1.0, Color32::from_gray(90)),
        );

        // Per-widget id (resp.id is unique) so sibling swatches don't share a
        // popup — e.g. multiple gradient-stop swatches in one row.
        let popup_id = resp.id.with("cp_popup");
        if resp.clicked() {
            ui.memory_mut(|m| m.toggle_popup(popup_id));
        }
        let inner = egui::popup::popup_below_widget(
            ui,
            popup_id,
            &resp,
            egui::PopupCloseBehavior::CloseOnClickOutside,
            |ui| {
                ui.set_min_width(WIDTH);
                Self::picker_body(ui, rgba, &cfg)
            },
        );
        if let Some(out) = inner {
            if out.changed {
                resp.mark_changed();
            }
        }
        resp
    }

    /// The full picker body, for embedding in any container you already have.
    /// See [`PickerConfig`] / [`PickerOutcome`].
    pub fn picker_body(ui: &mut Ui, rgba: &mut [f32; 4], cfg: &PickerConfig) -> PickerOutcome {
        let mut out = PickerOutcome::default();
        ui.vertical(|ui| {
            ui.set_width(WIDTH);
            out = picker_ui(ui, rgba, cfg);
        });
        out
    }

    /// Back-compat convenience: picker body with only an alpha toggle.
    pub fn picker_body_simple(ui: &mut Ui, rgba: &mut [f32; 4], alpha: bool) -> bool {
        Self::picker_body(
            ui,
            rgba,
            &PickerConfig {
                alpha,
                ..Default::default()
            },
        )
        .changed
    }

    /// A floating, draggable **window** picker anchored at `pos`. Bind `open` to
    /// your popup-visible flag. Returns the full [`PickerOutcome`].
    pub fn window(
        ctx: &egui::Context,
        id_salt: impl std::hash::Hash,
        title: &str,
        pos: Pos2,
        rgba: &mut [f32; 4],
        open: &mut bool,
        cfg: &PickerConfig,
    ) -> PickerOutcome {
        let mut out = PickerOutcome::default();
        egui::Window::new(title)
            .id(egui::Id::new(id_salt))
            .collapsible(false)
            .resizable(false)
            .default_pos(pos)
            .constrain(true)
            .open(open)
            .show(ctx, |ui| {
                out = Self::picker_body(ui, rgba, cfg);
            });
        out
    }
}

// ── The picker body implementation ────────────────────────────────────────────

fn picker_ui(ui: &mut Ui, rgba: &mut [f32; 4], cfg: &PickerConfig) -> PickerOutcome {
    let mut out = PickerOutcome::default();
    let id = ui.make_persistent_id("colorpopup_state");

    // Sync HSV state: reuse stored HSV when it still maps to the current color
    // (keeps hue stable when the user drags value/saturation to zero).
    let derived = cc::rgb_to_hsv([rgba[0], rgba[1], rgba[2]]);
    let mut st: PickerState = match ui.data(|d| d.get_temp::<PickerState>(id)) {
        Some(s) if approx(cc::hsv_to_rgb([s.h, s.s, s.v]), [rgba[0], rgba[1], rgba[2]]) => s,
        _ => PickerState {
            h: derived[0],
            s: derived[1],
            v: derived[2],
        },
    };
    let mut a = rgba[3];
    let mut changed = false;

    // ── SV square + hue bar (+ alpha bar) ──
    ui.horizontal(|ui| {
        let rgb_now = cc::hsv_to_rgb([st.h, st.s, st.v]);
        changed |= sv_square(ui, Vec2::new(sv_width(cfg), SV_H), &mut st);
        changed |= hue_bar(ui, Vec2::new(BAR_W, SV_H), &mut st.h);
        if cfg.alpha {
            changed |= alpha_bar(ui, Vec2::new(BAR_W, SV_H), rgb_now, &mut a);
        }
    });

    // Derive the working rgb from HSV for the numeric/hex rows.
    let mut rgb = cc::hsv_to_rgb([st.h, st.s, st.v]);
    let mut rgb_edited = false;

    ui.add_space(6.0);

    // ── Preview • eyedropper • hex • copy ──
    ui.horizontal(|ui| {
        let (prect, _) = ui.allocate_exact_size(Vec2::new(26.0, 20.0), Sense::hover());
        paint_checker(ui.painter(), prect);
        ui.painter().rect_filled(
            prect,
            egui::Rounding::same(3.0),
            ColorPopup::to_c32([rgb[0], rgb[1], rgb[2], a]),
        );
        ui.painter().rect_stroke(
            prect,
            egui::Rounding::same(3.0),
            Stroke::new(1.0, Color32::from_gray(90)),
        );

        if cfg.eyedropper
            && ui
                .button(egui_phosphor::regular::EYEDROPPER)
                .on_hover_text("Sample a color from the canvas")
                .clicked()
        {
            out.eyedropper_clicked = true;
        }

        rgb_edited |= hex_field(ui, id, &mut rgb, &mut a, cfg.alpha);

        if ui
            .button(egui_phosphor::regular::COPY)
            .on_hover_text("Copy hex")
            .clicked()
        {
            ui.ctx()
                .copy_text(cc::format_hex([rgb[0], rgb[1], rgb[2], a], cfg.alpha));
        }
    });

    // ── Model selector + numeric fields ──
    ui.horizontal(|ui| {
        rgb_edited |= model_fields(ui, id, &mut rgb, &mut a, cfg.alpha);
    });

    // ── Perceptual tint/shade ramp (OKLCH lightness) ──
    if let Some(picked) = ramp_row(ui, rgb) {
        rgb = picked;
        rgb_edited = true;
    }

    // ── Harmony chips (OKLCH hue rotations) ──
    if let Some(picked) = harmony_row(ui, rgb) {
        rgb = picked;
        rgb_edited = true;
    }

    // ── Recent colors ──
    if !cfg.recents.is_empty() {
        ui.add_space(2.0);
        ui.label(egui::RichText::new("RECENT").small().weak());
        if let Some(c) = swatch_strip(ui, cfg.recents, "recent") {
            rgb = [c[0], c[1], c[2]];
            if cfg.alpha {
                a = c[3];
            }
            rgb_edited = true;
        }
    }

    // ── Document swatches + add (only where the caller manages a palette) ──
    if cfg.allow_add_swatch || !cfg.swatches.is_empty() {
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("SWATCHES").small().weak());
            if cfg.allow_add_swatch
                && ui
                    .small_button(egui_phosphor::regular::PLUS)
                    .on_hover_text("Save this color to swatches")
                    .clicked()
            {
                out.add_swatch = Some([rgb[0], rgb[1], rgb[2], a]);
            }
        });
        if !cfg.swatches.is_empty() {
            if let Some(c) = swatch_strip(ui, cfg.swatches, "doc") {
                rgb = [c[0], c[1], c[2]];
                if cfg.alpha {
                    a = c[3];
                }
                rgb_edited = true;
            }
        }
    }

    // ── WCAG contrast + color-blindness preview ──
    ui.add_space(4.0);
    ui.separator();
    contrast_and_cvd(ui, rgb, cfg.contrast_ref);

    // ── Reconcile edits back into HSV state + the caller's store ──
    if rgb_edited {
        let h = cc::rgb_to_hsv(rgb);
        // Keep the previous hue when the new color is a pure gray (hue is
        // otherwise meaningless and would snap to 0).
        st.h = if h[1] <= 1e-4 { st.h } else { h[0] };
        st.s = h[1];
        st.v = h[2];
        changed = true;
    }

    if changed {
        let final_rgb = cc::hsv_to_rgb([st.h, st.s, st.v]);
        rgba[0] = final_rgb[0];
        rgba[1] = final_rgb[1];
        rgba[2] = final_rgb[2];
        if cfg.alpha {
            rgba[3] = a;
        }
        out.changed = true;
    }
    ui.data_mut(|d| d.insert_temp(id, st));
    out
}

fn sv_width(cfg: &PickerConfig) -> f32 {
    let bars = if cfg.alpha { 2.0 } else { 1.0 };
    WIDTH - bars * (BAR_W + GAP)
}

// ── 2D SV square ──────────────────────────────────────────────────────────────

fn sv_square(ui: &mut Ui, size: Vec2, st: &mut PickerState) -> bool {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
    // Gradient: x = saturation (0→1), y = value (1→0).
    let h = st.h;
    paint_gradient(ui.painter(), rect, 12, 12, |fx, fy| {
        let rgb = cc::hsv_to_rgb([h, fx, 1.0 - fy]);
        to_c32_rgb(rgb)
    });
    ui.painter().rect_stroke(
        rect,
        egui::Rounding::ZERO,
        Stroke::new(1.0, Color32::from_gray(90)),
    );

    let mut changed = false;
    if resp.dragged() || resp.clicked() {
        if let Some(p) = resp.interact_pointer_pos() {
            st.s = ((p.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
            st.v = 1.0 - ((p.y - rect.top()) / rect.height()).clamp(0.0, 1.0);
            changed = true;
        }
    }
    // Keyboard nudges (arrow keys; Shift = coarse).
    if resp.has_focus() {
        let (dx, dy) = arrow_delta(ui);
        if dx != 0.0 || dy != 0.0 {
            st.s = (st.s + dx).clamp(0.0, 1.0);
            st.v = (st.v + dy).clamp(0.0, 1.0);
            changed = true;
        }
    }
    if resp.has_focus() {
        ui.painter()
            .rect_stroke(rect, egui::Rounding::ZERO, Stroke::new(2.0, ACCENT));
    }

    // Cursor ring.
    let cx = rect.left() + st.s * rect.width();
    let cy = rect.top() + (1.0 - st.v) * rect.height();
    let center = Pos2::new(cx, cy);
    ui.painter()
        .circle_stroke(center, 5.0, Stroke::new(2.0, Color32::WHITE));
    ui.painter()
        .circle_stroke(center, 6.0, Stroke::new(1.0, Color32::from_black_alpha(160)));
    changed
}

fn hue_bar(ui: &mut Ui, size: Vec2, hue: &mut f32) -> bool {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
    paint_gradient(ui.painter(), rect, 1, 12, |_fx, fy| {
        to_c32_rgb(cc::hsv_to_rgb([fy * 360.0, 1.0, 1.0]))
    });
    ui.painter().rect_stroke(
        rect,
        egui::Rounding::ZERO,
        Stroke::new(1.0, Color32::from_gray(90)),
    );

    let mut changed = false;
    if resp.dragged() || resp.clicked() {
        if let Some(p) = resp.interact_pointer_pos() {
            *hue = (((p.y - rect.top()) / rect.height()).clamp(0.0, 1.0) * 360.0).rem_euclid(360.0);
            changed = true;
        }
    }
    if resp.has_focus() {
        let (_, dy) = arrow_delta(ui);
        if dy != 0.0 {
            *hue = (*hue - dy * 360.0).rem_euclid(360.0);
            changed = true;
        }
        ui.painter()
            .rect_stroke(rect, egui::Rounding::ZERO, Stroke::new(2.0, ACCENT));
    }
    let y = rect.top() + (*hue / 360.0) * rect.height();
    marker_line(ui.painter(), rect, y);
    changed
}

fn alpha_bar(ui: &mut Ui, size: Vec2, rgb: [f32; 3], a: &mut f32) -> bool {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
    paint_checker(ui.painter(), rect);
    paint_gradient(ui.painter(), rect, 1, 8, |_fx, fy| {
        let alpha = ((1.0 - fy) * 255.0).round() as u8;
        let b = cc::rgba_to_bytes([rgb[0], rgb[1], rgb[2], 1.0]);
        Color32::from_rgba_unmultiplied(b[0], b[1], b[2], alpha)
    });
    ui.painter().rect_stroke(
        rect,
        egui::Rounding::ZERO,
        Stroke::new(1.0, Color32::from_gray(90)),
    );

    let mut changed = false;
    if resp.dragged() || resp.clicked() {
        if let Some(p) = resp.interact_pointer_pos() {
            *a = 1.0 - ((p.y - rect.top()) / rect.height()).clamp(0.0, 1.0);
            changed = true;
        }
    }
    if resp.has_focus() {
        let (_, dy) = arrow_delta(ui);
        if dy != 0.0 {
            *a = (*a + dy).clamp(0.0, 1.0);
            changed = true;
        }
    }
    let y = rect.top() + (1.0 - *a) * rect.height();
    marker_line(ui.painter(), rect, y);
    changed
}

// ── Hex + numeric models ──────────────────────────────────────────────────────

fn hex_field(ui: &mut Ui, id: egui::Id, rgb: &mut [f32; 3], a: &mut f32, alpha: bool) -> bool {
    let buf_id = id.with("hexbuf");
    let editing_id = id.with("hexediting");
    let editing: bool = ui.data(|d| d.get_temp(editing_id).unwrap_or(false));
    let current = cc::format_hex([rgb[0], rgb[1], rgb[2], *a], alpha);
    let mut buf: String = if editing {
        ui.data(|d| d.get_temp(buf_id).unwrap_or_else(|| current.clone()))
    } else {
        current.clone()
    };

    let resp = ui.add(
        egui::TextEdit::singleline(&mut buf)
            .desired_width(if alpha { 84.0 } else { 68.0 })
            .font(egui::TextStyle::Monospace)
            .hint_text("#RRGGBB"),
    );

    let mut changed = false;
    if resp.has_focus() {
        ui.data_mut(|d| {
            d.insert_temp(editing_id, true);
            d.insert_temp(buf_id, buf.clone());
        });
    }
    if resp.lost_focus() {
        if let Some(c) = cc::parse_hex(&buf) {
            rgb[0] = c[0];
            rgb[1] = c[1];
            rgb[2] = c[2];
            if alpha {
                *a = c[3];
            }
            changed = true;
        }
        ui.data_mut(|d| {
            d.insert_temp(editing_id, false);
            d.remove::<String>(buf_id);
        });
    }
    changed
}

fn model_fields(ui: &mut Ui, id: egui::Id, rgb: &mut [f32; 3], a: &mut f32, alpha: bool) -> bool {
    let model_id = egui::Id::new("colorpopup_model");
    let mut model: ColorModel = ui
        .data(|d| d.get_temp(model_id))
        .unwrap_or(ColorModel::Rgb);

    egui::ComboBox::from_id_salt(id.with("model"))
        .selected_text(model.label())
        .width(64.0)
        .show_ui(ui, |ui| {
            for m in ColorModel::ALL {
                ui.selectable_value(&mut model, m, m.label());
            }
        });
    ui.data_mut(|d| d.insert_temp(model_id, model));

    let mut changed = false;
    match model {
        ColorModel::Rgb => {
            let mut b = cc::rgba_to_bytes([rgb[0], rgb[1], rgb[2], *a]);
            changed |= byte_drag(ui, "R", &mut b[0]);
            changed |= byte_drag(ui, "G", &mut b[1]);
            changed |= byte_drag(ui, "B", &mut b[2]);
            if alpha {
                changed |= byte_drag(ui, "A", &mut b[3]);
            }
            if changed {
                rgb[0] = b[0] as f32 / 255.0;
                rgb[1] = b[1] as f32 / 255.0;
                rgb[2] = b[2] as f32 / 255.0;
                if alpha {
                    *a = b[3] as f32 / 255.0;
                }
            }
        }
        ColorModel::Hsb => {
            let mut hsv = cc::rgb_to_hsv(*rgb);
            changed |= deg_drag(ui, "H", &mut hsv[0]);
            changed |= pct_drag(ui, "S", &mut hsv[1]);
            changed |= pct_drag(ui, "B", &mut hsv[2]);
            if changed {
                *rgb = cc::hsv_to_rgb(hsv);
            }
        }
        ColorModel::Hsl => {
            let mut hsl = cc::rgb_to_hsl(*rgb);
            changed |= deg_drag(ui, "H", &mut hsl[0]);
            changed |= pct_drag(ui, "S", &mut hsl[1]);
            changed |= pct_drag(ui, "L", &mut hsl[2]);
            if changed {
                *rgb = cc::hsl_to_rgb(hsl);
            }
        }
        ColorModel::Oklch => {
            let mut lch = cc::rgb_to_oklch(*rgb);
            changed |= unit_drag(ui, "L", &mut lch[0], 0.0..=1.0, 0.005);
            changed |= unit_drag(ui, "C", &mut lch[1], 0.0..=0.4, 0.002);
            changed |= deg_drag(ui, "H", &mut lch[2]);
            if changed {
                *rgb = cc::oklch_to_rgb(lch);
            }
        }
    }
    changed
}

// ── Ramp / harmony / swatch strips ────────────────────────────────────────────

/// A perceptually-even lightness ramp in OKLCH (chroma & hue held). Returns a
/// clicked color.
fn ramp_row(ui: &mut Ui, rgb: [f32; 3]) -> Option<[f32; 3]> {
    let [_, c, h] = cc::rgb_to_oklch(rgb);
    let mut picked = None;
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        for i in 0..9 {
            let l = 0.10 + 0.80 * (i as f32 / 8.0);
            let col = cc::oklch_to_rgb([l, c, h]);
            if mini_swatch(ui, col, 1.0, false).clicked() {
                picked = Some(col);
            }
        }
    });
    picked
}

/// Color-harmony chips (OKLCH hue rotations). Returns a clicked color.
fn harmony_row(ui: &mut Ui, rgb: [f32; 3]) -> Option<[f32; 3]> {
    let [l, c, h] = cc::rgb_to_oklch(rgb);
    let harmonies: [(&str, f32); 5] = [
        ("Comp", 180.0),
        ("Analog +", 30.0),
        ("Analog −", -30.0),
        ("Triad +", 120.0),
        ("Split", 150.0),
    ];
    let mut picked = None;
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        for (name, dh) in harmonies {
            let col = cc::oklch_to_rgb([l, c, (h + dh).rem_euclid(360.0)]);
            if mini_swatch(ui, col, 1.0, false)
                .on_hover_text(name)
                .clicked()
            {
                picked = Some(col);
            }
        }
    });
    picked
}

/// A wrapping strip of clickable swatches. Returns a clicked color (rgba).
fn swatch_strip(ui: &mut Ui, colors: &[[f32; 4]], salt: &str) -> Option<[f32; 4]> {
    let mut picked = None;
    ui.push_id(salt, |ui| {
        ui.horizontal_wrapped(|ui| {
            for c in colors.iter().take(24) {
                if mini_swatch(ui, [c[0], c[1], c[2]], c[3], true).clicked() {
                    picked = Some(*c);
                }
            }
        });
    });
    picked
}

// ── Contrast + CVD ────────────────────────────────────────────────────────────

fn contrast_and_cvd(ui: &mut Ui, rgb: [f32; 3], reference: Option<[f32; 3]>) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Contrast").small().weak());
        let show = |ui: &mut Ui, label: &str, other: [f32; 3]| {
            let cr = cc::contrast_ratio(rgb, other);
            let ok_aa = cr >= 4.5;
            let ok_aaa = cr >= 7.0;
            let color = if ok_aaa {
                Color32::from_rgb(80, 200, 120)
            } else if ok_aa {
                Color32::from_rgb(220, 190, 90)
            } else {
                Color32::from_rgb(220, 110, 110)
            };
            let tag = if ok_aaa {
                "AAA"
            } else if ok_aa {
                "AA"
            } else {
                "✗"
            };
            ui.label(
                egui::RichText::new(format!("{label} {cr:.1} {tag}"))
                    .small()
                    .color(color),
            )
            .on_hover_text("WCAG: AA ≥ 4.5:1, AAA ≥ 7:1");
        };
        match reference {
            Some(r) => show(ui, "vs bg", r),
            None => {
                show(ui, "▲", [0.0; 3]);
                show(ui, "△", [1.0; 3]);
            }
        }
    });
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("CVD").small().weak());
        for (kind, tip) in [
            (cc::ColorVisionDeficiency::Protanopia, "Protanopia (red-blind)"),
            (cc::ColorVisionDeficiency::Deuteranopia, "Deuteranopia (green-blind)"),
            (cc::ColorVisionDeficiency::Tritanopia, "Tritanopia (blue-blind)"),
        ] {
            let sim = cc::simulate_cvd(rgb, kind);
            mini_swatch(ui, sim, 1.0, false).on_hover_text(tip);
        }
    });
}

// ── Small painting / widget helpers ───────────────────────────────────────────

fn to_c32_rgb(rgb: [f32; 3]) -> Color32 {
    let b = cc::rgba_to_bytes([rgb[0], rgb[1], rgb[2], 1.0]);
    Color32::from_rgb(b[0], b[1], b[2])
}

/// A tiny clickable swatch (checkerboard under semi-transparent colors).
fn mini_swatch(ui: &mut Ui, rgb: [f32; 3], alpha: f32, show_alpha: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(16.0, 16.0), Sense::click());
    if show_alpha && alpha < 1.0 {
        paint_checker(ui.painter(), rect);
    }
    let b = cc::rgba_to_bytes([rgb[0], rgb[1], rgb[2], alpha]);
    ui.painter().rect_filled(
        rect,
        egui::Rounding::same(3.0),
        Color32::from_rgba_unmultiplied(b[0], b[1], b[2], b[3]),
    );
    let stroke = if resp.hovered() {
        Stroke::new(2.0, ACCENT)
    } else {
        Stroke::new(1.0, Color32::from_gray(80))
    };
    ui.painter()
        .rect_stroke(rect, egui::Rounding::same(3.0), stroke);
    resp
}

/// Paint a smooth 2D color gradient as a triangle mesh over `rect`.
fn paint_gradient(
    painter: &egui::Painter,
    rect: Rect,
    nx: usize,
    ny: usize,
    color_at: impl Fn(f32, f32) -> Color32,
) {
    let mut mesh = Mesh::default();
    let stride = (nx + 1) as u32;
    for iy in 0..=ny {
        for ix in 0..=nx {
            let fx = ix as f32 / nx as f32;
            let fy = iy as f32 / ny as f32;
            let pos = Pos2::new(
                rect.left() + fx * rect.width(),
                rect.top() + fy * rect.height(),
            );
            mesh.vertices.push(Vertex {
                pos,
                uv: egui::epaint::WHITE_UV,
                color: color_at(fx, fy),
            });
        }
    }
    for iy in 0..ny as u32 {
        for ix in 0..nx as u32 {
            let i = iy * stride + ix;
            mesh.add_triangle(i, i + 1, i + stride);
            mesh.add_triangle(i + 1, i + stride + 1, i + stride);
        }
    }
    painter.add(Shape::mesh(mesh));
}

/// Alpha checkerboard backing.
fn paint_checker(painter: &egui::Painter, rect: Rect) {
    const S: f32 = 5.0;
    let light = Color32::from_gray(160);
    let dark = Color32::from_gray(110);
    painter.rect_filled(rect, egui::Rounding::ZERO, dark);
    let cols = (rect.width() / S).ceil() as i32;
    let rows = (rect.height() / S).ceil() as i32;
    for r in 0..rows {
        for c in 0..cols {
            if (r + c) % 2 == 0 {
                let x = rect.left() + c as f32 * S;
                let y = rect.top() + r as f32 * S;
                let cell = Rect::from_min_size(Pos2::new(x, y), Vec2::splat(S)).intersect(rect);
                painter.rect_filled(cell, egui::Rounding::ZERO, light);
            }
        }
    }
}

/// A horizontal marker line across a vertical bar.
fn marker_line(painter: &egui::Painter, rect: Rect, y: f32) {
    let y = y.clamp(rect.top(), rect.bottom());
    painter.hline(
        rect.x_range(),
        y,
        Stroke::new(2.0, Color32::WHITE),
    );
    painter.hline(
        rect.x_range(),
        y + 1.5,
        Stroke::new(1.0, Color32::from_black_alpha(150)),
    );
}

/// Arrow-key delta for keyboard nudging (fine 1/255, coarse with Shift).
fn arrow_delta(ui: &Ui) -> (f32, f32) {
    ui.input(|i| {
        let step = if i.modifiers.shift { 0.05 } else { 1.0 / 255.0 };
        let mut dx = 0.0;
        let mut dy = 0.0;
        if i.key_pressed(egui::Key::ArrowRight) {
            dx += step;
        }
        if i.key_pressed(egui::Key::ArrowLeft) {
            dx -= step;
        }
        if i.key_pressed(egui::Key::ArrowUp) {
            dy += step;
        }
        if i.key_pressed(egui::Key::ArrowDown) {
            dy -= step;
        }
        (dx, dy)
    })
}

fn byte_drag(ui: &mut Ui, prefix: &str, v: &mut u8) -> bool {
    ui.add(egui::DragValue::new(v).speed(0.5).prefix(format!("{prefix} ")))
        .changed()
}

fn deg_drag(ui: &mut Ui, prefix: &str, v: &mut f32) -> bool {
    let mut x = *v;
    let changed = ui
        .add(
            egui::DragValue::new(&mut x)
                .speed(1.0)
                .range(0.0..=360.0)
                .suffix("°")
                .prefix(format!("{prefix} "))
                .max_decimals(0),
        )
        .changed();
    if changed {
        *v = x;
    }
    changed
}

/// A 0..1 value shown as a 0..100 percentage.
fn pct_drag(ui: &mut Ui, prefix: &str, v: &mut f32) -> bool {
    let mut pct = *v * 100.0;
    let changed = ui
        .add(
            egui::DragValue::new(&mut pct)
                .speed(0.5)
                .range(0.0..=100.0)
                .suffix("%")
                .prefix(format!("{prefix} "))
                .max_decimals(0),
        )
        .changed();
    if changed {
        *v = pct / 100.0;
    }
    changed
}

fn unit_drag(
    ui: &mut Ui,
    prefix: &str,
    v: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    speed: f32,
) -> bool {
    ui.add(
        egui::DragValue::new(v)
            .speed(speed)
            .range(range)
            .prefix(format!("{prefix} "))
            .max_decimals(3),
    )
    .changed()
}

fn approx(a: [f32; 3], b: [f32; 3]) -> bool {
    (0..3).all(|i| (a[i] - b[i]).abs() < 1.0 / 512.0)
}
