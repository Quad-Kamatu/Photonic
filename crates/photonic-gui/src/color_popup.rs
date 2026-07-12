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
use photonic_core::style::{
    interpolate_stops_with, Fill, FillKind, FluidGradient, FluidGradientPoint, Gradient,
    GradientInterpolation, GradientKind, GradientStop, GradientUnits, MeshGradient,
};
use photonic_core::Color;

use crate::color_convert as cc;
use crate::panels::FillColorSlot;

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
                egui_phosphor::regular::X
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
                show(ui, "vs blk", [0.0; 3]);
                show(ui, "vs wht", [1.0; 3]);
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

// ══════════════════════════════════════════════════════════════════════════════
// Fill-aware picker: solid + gradients with a slide-out gradient drawer.
// ══════════════════════════════════════════════════════════════════════════════

const DRAWER_W: f32 = 214.0;

/// Config for the fill-aware picker.
pub struct FillPickerConfig<'a> {
    /// Config for the embedded color picker (recents, swatches, eyedropper…).
    pub color: PickerConfig<'a>,
    /// Saved gradients (name, fill) offered as one-click swatches.
    pub gradient_swatches: &'a [(String, Fill)],
    /// Show the "save gradient" button (caller must handle
    /// [`FillOutcome::save_gradient`]).
    pub allow_save_gradient: bool,
}

/// What the fill picker reported this frame.
#[derive(Default)]
pub struct FillOutcome {
    pub changed: bool,
    /// A stop/point wants the canvas eyedropper.
    pub eyedropper: Option<FillColorSlot>,
    /// Save the active solid color to swatches.
    pub add_swatch: Option<[f32; 4]>,
    /// Save the current gradient fill to the gradient library.
    pub save_gradient: Option<Fill>,
}

/// Fill type, including the two gradient sub-kinds.
#[derive(Clone, Copy, PartialEq, Eq)]
enum FillType {
    None,
    Solid,
    Linear,
    Radial,
    Fluid,
    Mesh,
    Pattern,
}

impl FillType {
    fn of(f: &Fill) -> Self {
        match &f.kind {
            FillKind::None => Self::None,
            FillKind::Solid(_) => Self::Solid,
            FillKind::Gradient(g) => match g.kind {
                GradientKind::Linear => Self::Linear,
                GradientKind::Radial => Self::Radial,
            },
            FillKind::FluidGradient(_) => Self::Fluid,
            FillKind::MeshGradient(_) => Self::Mesh,
            FillKind::Pattern(_) => Self::Pattern,
        }
    }
    fn is_gradient(self) -> bool {
        matches!(self, Self::Linear | Self::Radial | Self::Fluid | Self::Mesh)
    }
    /// Whether this fill type gets the slide-out drawer (gradients + pattern).
    fn has_drawer(self) -> bool {
        self.is_gradient() || self == Self::Pattern
    }
    fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Solid => "Solid",
            Self::Linear => "Linear",
            Self::Radial => "Radial",
            Self::Fluid => "Fluid",
            Self::Mesh => "Mesh",
            Self::Pattern => "Pattern",
        }
    }
}

fn col_to_arr(c: Color) -> [f32; 4] {
    [c.r, c.g, c.b, c.a]
}
fn arr_to_col(a: [f32; 4]) -> Color {
    Color {
        r: a[0],
        g: a[1],
        b: a[2],
        a: a[3],
    }
}

impl ColorPopup {
    /// The fill-aware picker: fill-type tabs + the color area (editing the active
    /// stop) + a gradient drawer that slides out for gradient fills.
    pub fn fill_picker(ui: &mut Ui, fill: &mut Fill, cfg: &FillPickerConfig) -> FillOutcome {
        let mut out = FillOutcome::default();
        let id = ui.make_persistent_id("fill_picker");
        let ftype = FillType::of(fill);

        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(WIDTH);
                // Fill-type tabs.
                if let Some(new_type) = fill_type_tabs(ui, ftype) {
                    if new_type != ftype {
                        *fill = build_default_fill(new_type, fill);
                        out.changed = true;
                    }
                }
                ui.add_space(4.0);
                // Active color area (edits the selected stop/point).
                fill_active_color(ui, fill, id, cfg, &mut out);
            });

            // Slide-out gradient/pattern drawer.
            let want = FillType::of(fill).has_drawer();
            let openf = ui
                .ctx()
                .animate_bool_with_time(id.with("drawer"), want, 0.15);
            if openf > 0.01 {
                ui.add_space(6.0);
                ui.vertical(|ui| {
                    ui.set_width(DRAWER_W * openf);
                    if openf > 0.55 {
                        gradient_drawer(ui, fill, id, cfg, &mut out);
                    }
                });
            }
        });
        out
    }
}

/// Fill-type tab row. Returns the newly chosen type.
fn fill_type_tabs(ui: &mut Ui, current: FillType) -> Option<FillType> {
    let mut chosen = None;
    ui.horizontal_wrapped(|ui| {
        for t in [
            FillType::Solid,
            FillType::Linear,
            FillType::Radial,
            FillType::Fluid,
            FillType::Mesh,
            FillType::Pattern,
        ] {
            let selected = current == t
                || (t == FillType::Solid && current == FillType::None);
            if ui
                .add(egui::SelectableLabel::new(selected, t.label()))
                .clicked()
            {
                chosen = Some(t);
            }
        }
    });
    chosen
}

/// Build a sensible default fill when switching type, inheriting a base color.
fn build_default_fill(t: FillType, old: &Fill) -> Fill {
    let base = match &old.kind {
        FillKind::Solid(c) => *c,
        FillKind::Gradient(g) => g.stops.first().map(|s| s.color).unwrap_or(Color::BLACK),
        FillKind::FluidGradient(fg) => fg.points.first().map(|p| p.color).unwrap_or(Color::BLACK),
        FillKind::MeshGradient(mg) => mg.cell_colors.first().copied().unwrap_or(Color::BLACK),
        _ => Color::BLACK,
    };
    let white = Color::WHITE;
    let mut f = match t {
        FillType::None => Fill::solid(base),
        FillType::Solid => Fill::solid(base),
        // New gradients use object-bounding-box units (0..1) so they fit the
        // filled object rather than the artboard.
        FillType::Linear => Fill::gradient(
            Gradient::linear(
                0.0,
                0.0,
                1.0,
                0.0,
                vec![GradientStop::new(0.0, base), GradientStop::new(1.0, white)],
            )
            .with_units(GradientUnits::ObjectBoundingBox),
        ),
        FillType::Radial => Fill::gradient(
            Gradient::radial(
                0.5,
                0.5,
                0.5,
                vec![GradientStop::new(0.0, base), GradientStop::new(1.0, white)],
            )
            .with_units(GradientUnits::ObjectBoundingBox),
        ),
        FillType::Fluid => Fill::fluid_gradient(
            FluidGradient::new(vec![
                FluidGradientPoint::new(0.25, 0.3, base),
                FluidGradientPoint::new(0.75, 0.3, white),
                FluidGradientPoint::new(0.5, 0.8, Color::new(1.0, 0.5, 0.0, 1.0)),
            ])
            .with_units(GradientUnits::ObjectBoundingBox),
        ),
        FillType::Mesh => Fill::mesh_gradient(
            MeshGradient::grid(
                2,
                2,
                vec![
                    base,
                    Color::new(1.0, 0.2, 0.2, 1.0),
                    Color::new(0.2, 1.0, 0.2, 1.0),
                    Color::new(0.2, 0.2, 1.0, 1.0),
                ],
            )
            .with_units(GradientUnits::ObjectBoundingBox),
        ),
        FillType::Pattern => {
            Fill::pattern(photonic_core::style::PatternFill::new(crate::panels::default_checker_tile()))
        }
    };
    f.opacity = old.opacity;
    f.enabled = true;
    f
}

/// egui memory id holding the active stop/point index for a fill.
fn active_index(ui: &Ui, id: egui::Id, count: usize) -> usize {
    let i: usize = ui.data(|d| d.get_temp(id.with("active")).unwrap_or(0));
    i.min(count.saturating_sub(1))
}
fn set_active_index(ui: &Ui, id: egui::Id, i: usize) {
    ui.data_mut(|d| d.insert_temp(id.with("active"), i));
}

/// The color area — edits the fill's solid color, or the selected gradient
/// stop/point.
fn fill_active_color(
    ui: &mut Ui,
    fill: &mut Fill,
    id: egui::Id,
    cfg: &FillPickerConfig,
    out: &mut FillOutcome,
) {
    // Extract the active color + a slot for eyedropper targeting.
    let (mut rgba, slot): (Option<[f32; 4]>, FillColorSlot) = match &fill.kind {
        FillKind::None => (None, FillColorSlot::Solid),
        FillKind::Solid(c) => (Some(col_to_arr(*c)), FillColorSlot::Solid),
        FillKind::Gradient(g) => {
            let ai = active_index(ui, id, g.stops.len());
            (
                g.stops.get(ai).map(|s| col_to_arr(s.color)),
                FillColorSlot::GradientStop(ai),
            )
        }
        FillKind::FluidGradient(fg) => {
            let ai = active_index(ui, id, fg.points.len());
            (
                fg.points.get(ai).map(|p| col_to_arr(p.color)),
                FillColorSlot::FluidPoint(ai),
            )
        }
        FillKind::MeshGradient(mg) => {
            let ai = active_index(ui, id, mg.cell_colors.len());
            (
                mg.cell_colors.get(ai).map(|c| col_to_arr(*c)),
                FillColorSlot::MeshVertex(ai),
            )
        }
        FillKind::Pattern(_) => (None, FillColorSlot::Solid),
    };

    match &mut rgba {
        Some(rgba) => {
            let p = ColorPopup::picker_body(ui, rgba, &cfg.color);
            if p.eyedropper_clicked {
                out.eyedropper = Some(slot.clone());
            }
            if p.add_swatch.is_some() {
                out.add_swatch = p.add_swatch;
            }
            if p.changed {
                let c = arr_to_col(*rgba);
                match (&mut fill.kind, &slot) {
                    (FillKind::Solid(sc), _) => *sc = c,
                    (FillKind::Gradient(g), FillColorSlot::GradientStop(i)) => {
                        if let Some(s) = g.stops.get_mut(*i) {
                            s.color = c;
                        }
                    }
                    (FillKind::FluidGradient(fg), FillColorSlot::FluidPoint(i)) => {
                        if let Some(pt) = fg.points.get_mut(*i) {
                            pt.color = c;
                        }
                    }
                    (FillKind::MeshGradient(mg), FillColorSlot::MeshVertex(i)) => {
                        if let Some(cell) = mg.cell_colors.get_mut(*i) {
                            *cell = c;
                        }
                    }
                    _ => {}
                }
                out.changed = true;
            }
        }
        None => {
            if matches!(fill.kind, FillKind::Pattern(_)) {
                ui.label(egui::RichText::new("Pattern fill — see drawer").weak().small());
            } else {
                ui.label(egui::RichText::new("No fill").weak());
                if ui.button("Add solid color").clicked() {
                    *fill = Fill::solid(Color::BLACK);
                    out.changed = true;
                }
            }
        }
    }
}

/// The slide-out gradient drawer: stop bar / point list, geometry, interpolation,
/// and gradient swatches.
fn gradient_drawer(
    ui: &mut Ui,
    fill: &mut Fill,
    id: egui::Id,
    cfg: &FillPickerConfig,
    out: &mut FillOutcome,
) {
    let is_pattern = matches!(fill.kind, FillKind::Pattern(_));
    ui.label(
        egui::RichText::new(if is_pattern { "PATTERN" } else { "GRADIENT" })
            .small()
            .color(ACCENT),
    );
    match &mut fill.kind {
        FillKind::Gradient(g) => {
            gradient_stop_drawer(ui, g, id, out);
        }
        FillKind::FluidGradient(fg) => {
            if fluid_drawer(ui, fg, id) {
                out.changed = true;
            }
        }
        FillKind::MeshGradient(mg) => {
            if mesh_drawer(ui, mg, id) {
                out.changed = true;
            }
        }
        FillKind::Pattern(p) => {
            if pattern_drawer(ui, p) {
                out.changed = true;
            }
        }
        _ => {}
    }

    // Gradient library.
    if cfg.allow_save_gradient || !cfg.gradient_swatches.is_empty() {
        ui.add_space(4.0);
        ui.separator();
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("SAVED").small().weak());
            if cfg.allow_save_gradient
                && matches!(fill.kind, FillKind::Gradient(_))
                && ui
                    .small_button(egui_phosphor::regular::PLUS)
                    .on_hover_text("Save this gradient")
                    .clicked()
            {
                out.save_gradient = Some(fill.clone());
            }
        });
        ui.horizontal_wrapped(|ui| {
            for (name, gfill) in cfg.gradient_swatches {
                if gradient_swatch_button(ui, gfill).on_hover_text(name).clicked() {
                    *fill = gfill.clone();
                    out.changed = true;
                }
            }
        });
    }
}

/// Linear/Radial drawer: interpolation toggle, stop bar, stop actions, geometry.
/// A "rotate with object" checkbox for object-bounding-box gradients: toggles
/// between axis-aligned and rotation-following units. Returns true on change.
fn rotate_toggle(ui: &mut Ui, units: &mut GradientUnits) -> bool {
    if !units.is_object_box() {
        return false;
    }
    let mut rot = units.follows_rotation();
    if ui
        .checkbox(&mut rot, "Rotate with object")
        .on_hover_text("Gradient rotates & shears with the object (local space)")
        .changed()
    {
        *units = if rot {
            GradientUnits::ObjectBoundingBoxRotated
        } else {
            GradientUnits::ObjectBoundingBox
        };
        return true;
    }
    false
}

fn gradient_stop_drawer(ui: &mut Ui, g: &mut Gradient, id: egui::Id, out: &mut FillOutcome) {
    let mut changed = false;

    // Interpolation space (T2).
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Blend").small().weak());
        let mut interp = g.interpolation;
        if ui
            .selectable_value(&mut interp, GradientInterpolation::Srgb, "sRGB")
            .clicked()
            | ui.selectable_value(&mut interp, GradientInterpolation::Oklab, "OKLab")
                .on_hover_text("Perceptual blend — avoids the muddy gray midpoint")
                .clicked()
        {
            g.interpolation = interp;
            changed = true;
        }
    });
    changed |= rotate_toggle(ui, &mut g.units);

    // Stop bar.
    let mut active = active_index(ui, id, g.stops.len());
    changed |= gradient_bar(ui, g, &mut active);
    set_active_index(ui, id, active);

    // Active-stop controls: offset, midpoint, actions.
    ui.horizontal(|ui| {
        if let Some(s) = g.stops.get_mut(active) {
            let mut off = s.offset;
            if ui
                .add(egui::DragValue::new(&mut off).speed(0.005).range(0.0..=1.0).prefix("pos "))
                .changed()
            {
                s.offset = off;
                changed = true;
            }
            let mut mid = s.midpoint * 100.0;
            if ui
                .add(
                    egui::DragValue::new(&mut mid)
                        .speed(0.5)
                        .range(2.0..=98.0)
                        .suffix("%")
                        .prefix("mid "),
                )
                .on_hover_text("Midpoint of the blend to the next stop")
                .changed()
            {
                s.midpoint = mid / 100.0;
                changed = true;
            }
        }
    });
    ui.horizontal(|ui| {
        if ui.small_button(egui_phosphor::regular::PLUS).on_hover_text("Add stop").clicked() {
            let off = g.stops.get(active).map(|s| s.offset).unwrap_or(0.5);
            let c = interpolate_stops_with(&g.stops, off, g.interpolation);
            g.stops.insert(active + 1, GradientStop::new((off + 0.1).min(1.0), arr_to_col(c)));
            changed = true;
        }
        if ui.small_button(egui_phosphor::regular::COPY).on_hover_text("Duplicate stop").clicked() {
            if let Some(s) = g.stops.get(active).cloned() {
                g.stops.insert(active + 1, s);
                changed = true;
            }
        }
        if g.stops.len() > 2
            && ui.small_button(egui_phosphor::regular::TRASH).on_hover_text("Delete stop").clicked()
        {
            g.stops.remove(active);
            changed = true;
        }
        if ui
            .small_button(egui_phosphor::regular::ARROWS_LEFT_RIGHT)
            .on_hover_text("Reverse")
            .clicked()
        {
            for s in g.stops.iter_mut() {
                s.offset = 1.0 - s.offset;
            }
            g.stops.reverse();
            changed = true;
        }
        if ui
            .small_button(egui_phosphor::regular::DOTS_THREE_OUTLINE)
            .on_hover_text("Distribute evenly")
            .clicked()
        {
            let n = g.stops.len();
            if n > 1 {
                for (i, s) in g.stops.iter_mut().enumerate() {
                    s.offset = i as f32 / (n - 1) as f32;
                }
                changed = true;
            }
        }
    });

    // Geometry.
    ui.add_space(2.0);
    match g.kind {
        GradientKind::Linear => changed |= linear_geometry(ui, g),
        GradientKind::Radial => changed |= radial_geometry(ui, g),
    }

    if changed {
        out.changed = true;
    }
}

/// The interactive gradient preview bar with draggable, selectable stops.
fn gradient_bar(ui: &mut Ui, g: &mut Gradient, active: &mut usize) -> bool {
    let h = 26.0;
    let (rect, resp) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), h), Sense::click_and_drag());
    paint_checker(ui.painter(), rect);
    let interp = g.interpolation;
    let stops = g.stops.clone();
    paint_gradient(ui.painter(), rect, 48, 1, |fx, _| {
        let c = interpolate_stops_with(&stops, fx, interp);
        Color32::from_rgba_unmultiplied(
            (c[0] * 255.0) as u8,
            (c[1] * 255.0) as u8,
            (c[2] * 255.0) as u8,
            (c[3] * 255.0) as u8,
        )
    });
    ui.painter()
        .rect_stroke(rect, egui::Rounding::same(3.0), Stroke::new(1.0, Color32::from_gray(90)));

    let x_of = |off: f32| rect.left() + off.clamp(0.0, 1.0) * rect.width();
    let frac_of = |x: f32| ((x - rect.left()) / rect.width()).clamp(0.0, 1.0);

    let mut changed = false;

    // Select nearest stop on press; add a stop on double-click.
    if resp.double_clicked() {
        if let Some(p) = resp.interact_pointer_pos() {
            let off = frac_of(p.x);
            let c = interpolate_stops_with(&g.stops, off, interp);
            g.stops.push(GradientStop::new(off, arr_to_col(c)));
            g.stops.sort_by(|a, b| a.offset.partial_cmp(&b.offset).unwrap());
            *active = g
                .stops
                .iter()
                .position(|s| (s.offset - off).abs() < 1e-4)
                .unwrap_or(0);
            changed = true;
        }
    } else if resp.drag_started() || resp.clicked() {
        if let Some(p) = resp.interact_pointer_pos() {
            *active = g
                .stops
                .iter()
                .enumerate()
                .min_by(|(_, a), (_, b)| {
                    (a.offset - frac_of(p.x))
                        .abs()
                        .partial_cmp(&(b.offset - frac_of(p.x)).abs())
                        .unwrap()
                })
                .map(|(i, _)| i)
                .unwrap_or(0);
        }
    }
    if resp.dragged() {
        if let Some(p) = resp.interact_pointer_pos() {
            if let Some(s) = g.stops.get_mut(*active) {
                s.offset = frac_of(p.x);
                changed = true;
            }
        }
    }

    // Draw stop handles.
    for (i, s) in g.stops.iter().enumerate() {
        let x = x_of(s.offset);
        let top = rect.top();
        let bot = rect.bottom();
        let sel = i == *active;
        let stroke = if sel {
            Stroke::new(2.0, Color32::WHITE)
        } else {
            Stroke::new(1.0, Color32::from_black_alpha(180))
        };
        ui.painter().line_segment([Pos2::new(x, top), Pos2::new(x, bot)], stroke);
        let b = cc::rgba_to_bytes(col_to_arr(s.color));
        ui.painter().circle(
            Pos2::new(x, bot + 4.0),
            if sel { 5.0 } else { 4.0 },
            Color32::from_rgb(b[0], b[1], b[2]),
            stroke,
        );
    }
    ui.add_space(8.0); // room for the handle circles below the bar
    changed
}

fn linear_geometry(ui: &mut Ui, g: &mut Gradient) -> bool {
    if g.coords.len() < 4 {
        return false;
    }
    // Object-bounding-box coords are 0..1 fractions; user-space are pixels.
    let obb = g.units.is_object_box();
    let (len_speed, len_max, min_len) = if obb {
        (0.005, 4.0, 0.01)
    } else {
        (1.0, 100000.0, 1.0)
    };
    let (x0, y0, x1, y1) = (g.coords[0], g.coords[1], g.coords[2], g.coords[3]);
    let dx = x1 - x0;
    let dy = y1 - y0;
    let mut angle = dy.atan2(dx).to_degrees();
    let len = (dx * dx + dy * dy).sqrt().max(min_len);
    let mut changed = false;
    let apply = |g: &mut Gradient, angle: f64, l: f64| {
        let cx = (x0 + x1) * 0.5;
        let cy = (y0 + y1) * 0.5;
        let r = angle.to_radians();
        g.coords[0] = cx - r.cos() * l * 0.5;
        g.coords[1] = cy - r.sin() * l * 0.5;
        g.coords[2] = cx + r.cos() * l * 0.5;
        g.coords[3] = cy + r.sin() * l * 0.5;
    };
    ui.horizontal(|ui| {
        if ui
            .add(egui::DragValue::new(&mut angle).speed(1.0).suffix("°").prefix("angle "))
            .changed()
        {
            apply(g, angle, len);
            changed = true;
        }
        let mut l = len;
        if ui
            .add(
                egui::DragValue::new(&mut l)
                    .speed(len_speed)
                    .range(min_len..=len_max)
                    .prefix("len "),
            )
            .changed()
        {
            apply(g, angle, l);
            changed = true;
        }
    });
    changed
}

fn radial_geometry(ui: &mut Ui, g: &mut Gradient) -> bool {
    if g.coords.len() < 5 {
        return false;
    }
    let obb = g.units.is_object_box();
    let (speed, rmin, rmax) = if obb {
        (0.005, 0.01, 4.0)
    } else {
        (1.0, 1.0, 100000.0)
    };
    let mut changed = false;
    ui.horizontal(|ui| {
        let mut cx = g.coords[0];
        let mut cy = g.coords[1];
        if ui.add(egui::DragValue::new(&mut cx).speed(speed).prefix("cx ")).changed() {
            g.coords[0] = cx;
            g.coords[2] = cx;
            changed = true;
        }
        if ui.add(egui::DragValue::new(&mut cy).speed(speed).prefix("cy ")).changed() {
            g.coords[1] = cy;
            g.coords[3] = cy;
            changed = true;
        }
        let mut r = g.coords[4];
        if ui
            .add(egui::DragValue::new(&mut r).speed(speed).range(rmin..=rmax).prefix("r "))
            .changed()
        {
            g.coords[4] = r;
            changed = true;
        }
    });
    changed
}

fn fluid_drawer(ui: &mut Ui, fg: &mut FluidGradient, id: egui::Id) -> bool {
    let mut changed = false;
    changed |= rotate_toggle(ui, &mut fg.units);
    let obb = fg.units.is_object_box();
    let xy_speed = if obb { 0.005 } else { 1.0 };
    let add_pos = if obb { 0.5 } else { 100.0 };
    let mut active = active_index(ui, id, fg.points.len());
    let mut remove = None;
    ui.label(egui::RichText::new("Points").small().weak());
    egui::ScrollArea::vertical().max_height(150.0).id_salt("fluid_pts").show(ui, |ui| {
        for i in 0..fg.points.len() {
            ui.horizontal(|ui| {
                let sel = i == active;
                if mini_swatch(ui, [fg.points[i].color.r, fg.points[i].color.g, fg.points[i].color.b], fg.points[i].color.a, true).clicked() {
                    active = i;
                }
                if sel {
                    ui.label(egui::RichText::new("●").small().color(ACCENT));
                }
                let mut x = fg.points[i].x as f32;
                let mut y = fg.points[i].y as f32;
                if ui.add(egui::DragValue::new(&mut x).speed(xy_speed).prefix("x")).changed() {
                    fg.points[i].x = x as f64;
                    changed = true;
                }
                if ui.add(egui::DragValue::new(&mut y).speed(xy_speed).prefix("y")).changed() {
                    fg.points[i].y = y as f64;
                    changed = true;
                }
                if fg.points.len() > 1 && ui.small_button(egui_phosphor::regular::X).clicked() {
                    remove = Some(i);
                }
            });
        }
    });
    if let Some(i) = remove {
        fg.points.remove(i);
        changed = true;
    }
    ui.horizontal(|ui| {
        if ui.small_button("+ Point").clicked() {
            fg.points.push(FluidGradientPoint::new(add_pos, add_pos, Color::WHITE));
            active = fg.points.len() - 1;
            changed = true;
        }
        let mut power = fg.power;
        if ui.add(egui::DragValue::new(&mut power).speed(0.1).range(0.5..=8.0).prefix("power ")).changed() {
            fg.power = power;
            changed = true;
        }
    });
    set_active_index(ui, id, active);
    changed
}

fn mesh_drawer(ui: &mut Ui, mg: &mut MeshGradient, id: egui::Id) -> bool {
    let mut changed = false;
    changed |= rotate_toggle(ui, &mut mg.units);

    // Blend: hard cells (0) → smooth (1).
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Blend").small().weak());
        let mut b = mg.blend;
        if ui
            .add(egui::Slider::new(&mut b, 0.0..=1.0).show_value(false))
            .on_hover_text("Cell transition: hard edges (0) → smooth blend (1)")
            .changed()
        {
            mg.blend = b;
            changed = true;
        }
    });

    let mut active = active_index(ui, id, mg.cell_colors.len());
    ui.label(
        egui::RichText::new(format!("{}×{} cells — click to recolor", mg.rows, mg.cols))
            .small()
            .weak(),
    );
    egui::ScrollArea::vertical().max_height(150.0).id_salt("mesh_grid").show(ui, |ui| {
        for row in 0..mg.rows {
            ui.horizontal(|ui| {
                for col in 0..mg.cols {
                    let idx = (row * mg.cols + col) as usize;
                    if let Some(c) = mg.cell_colors.get(idx) {
                        let sel = idx == active;
                        let r = mini_swatch(ui, [c.r, c.g, c.b], c.a, false);
                        if r.clicked() {
                            active = idx;
                        }
                        if sel {
                            ui.painter().rect_stroke(
                                r.rect.expand(1.5),
                                egui::Rounding::same(3.0),
                                Stroke::new(2.0, ACCENT),
                            );
                        }
                    }
                }
            });
        }
    });
    ui.horizontal(|ui| {
        if mg.rows < 8 && ui.small_button("+ Row").clicked() {
            mesh_resize(mg, mg.rows + 1, mg.cols);
            changed = true;
        }
        if mg.rows > 1 && ui.small_button("− Row").clicked() {
            mesh_resize(mg, mg.rows - 1, mg.cols);
            changed = true;
        }
        if mg.cols < 8 && ui.small_button("+ Col").clicked() {
            mesh_resize(mg, mg.rows, mg.cols + 1);
            changed = true;
        }
        if mg.cols > 1 && ui.small_button("− Col").clicked() {
            mesh_resize(mg, mg.rows, mg.cols - 1);
            changed = true;
        }
    });
    set_active_index(ui, id, active);
    changed
}

/// Resize the cell grid, preserving overlapping cell colors and re-spacing the
/// lines of the changed axis evenly (the unchanged axis keeps its positions).
fn mesh_resize(mg: &mut MeshGradient, new_rows: u32, new_cols: u32) {
    let new_rows = new_rows.clamp(1, 8);
    let new_cols = new_cols.clamp(1, 8);
    let (or, oc) = (mg.rows, mg.cols);
    let old = std::mem::take(&mut mg.cell_colors);
    let mut colors = Vec::with_capacity((new_rows * new_cols) as usize);
    for r in 0..new_rows {
        for c in 0..new_cols {
            colors.push(if r < or && c < oc {
                old.get((r * oc + c) as usize).copied().unwrap_or(Color::WHITE)
            } else {
                Color::WHITE
            });
        }
    }
    mg.cell_colors = colors;
    if new_cols != oc {
        mg.x_lines = (0..=new_cols).map(|i| i as f64 / new_cols as f64).collect();
    }
    if new_rows != or {
        mg.y_lines = (0..=new_rows).map(|i| i as f64 / new_rows as f64).collect();
    }
    mg.rows = new_rows;
    mg.cols = new_cols;
}

/// A small gradient preview swatch button.
fn gradient_swatch_button(ui: &mut Ui, fill: &Fill) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(28.0, 16.0), Sense::click());
    if let FillKind::Gradient(g) = &fill.kind {
        let stops = g.stops.clone();
        let interp = g.interpolation;
        paint_gradient(ui.painter(), rect, 24, 1, |fx, _| {
            let c = interpolate_stops_with(&stops, fx, interp);
            Color32::from_rgb((c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8)
        });
    } else {
        ui.painter().rect_filled(rect, egui::Rounding::same(3.0), Color32::from_gray(80));
    }
    let stroke = if resp.hovered() {
        Stroke::new(2.0, ACCENT)
    } else {
        Stroke::new(1.0, Color32::from_gray(80))
    };
    ui.painter().rect_stroke(rect, egui::Rounding::same(3.0), stroke);
    resp
}

fn pattern_drawer(ui: &mut Ui, p: &mut photonic_core::style::PatternFill) -> bool {
    use photonic_core::style::PatternTileType;
    let mut changed = false;
    ui.label(
        egui::RichText::new(format!("Tile {}×{}px", p.tile.width, p.tile.height))
            .small()
            .weak(),
    );
    ui.horizontal_wrapped(|ui| {
        for (label, t) in [
            ("Grid", PatternTileType::Grid),
            ("Brick", PatternTileType::BrickByRow),
            ("Brick↕", PatternTileType::BrickByColumn),
            ("Hex", PatternTileType::Hex),
        ] {
            if ui.selectable_label(p.tile_type == t, label).clicked() {
                p.tile_type = t;
                changed = true;
            }
        }
    });
    ui.horizontal(|ui| {
        let mut scale = p.scale;
        if ui
            .add(egui::DragValue::new(&mut scale).speed(0.01).range(0.05..=20.0).prefix("scale "))
            .changed()
        {
            p.scale = scale;
            changed = true;
        }
        let mut rot = p.rotation.to_degrees();
        if ui
            .add(egui::DragValue::new(&mut rot).speed(1.0).suffix("°").prefix("rot "))
            .changed()
        {
            p.rotation = rot.to_radians();
            changed = true;
        }
    });
    ui.horizontal(|ui| {
        let mut spacing = p.spacing;
        if ui
            .add(egui::DragValue::new(&mut spacing).speed(0.5).range(0.0..=200.0).prefix("gap "))
            .changed()
        {
            p.spacing = spacing;
            changed = true;
        }
    });
    changed
}

impl ColorPopup {
    /// A floating fill picker window (fill-type tabs + gradient drawer).
    pub fn fill_window(
        ctx: &egui::Context,
        id_salt: impl std::hash::Hash,
        title: &str,
        pos: Pos2,
        fill: &mut Fill,
        open: &mut bool,
        cfg: &FillPickerConfig,
    ) -> FillOutcome {
        let mut out = FillOutcome::default();
        let frame = egui::Frame::window(&ctx.style()).inner_margin(egui::Margin::same(12.0));
        egui::Window::new(title)
            .id(egui::Id::new(id_salt))
            .collapsible(false)
            .resizable(false)
            .default_pos(pos)
            .constrain(true)
            .frame(frame)
            .open(open)
            .show(ctx, |ui| {
                out = Self::fill_picker(ui, fill, cfg);
            });
        out
    }

    /// A fill preview **swatch button** that opens the fill picker in a popup.
    /// Renders a wide swatch showing the current fill.
    pub fn fill_swatch_popup(
        ui: &mut Ui,
        fill: &mut Fill,
        cfg: &FillPickerConfig,
    ) -> FillOutcome {
        let mut out = FillOutcome::default();
        let size = Vec2::new(48.0, ui.spacing().interact_size.y.max(18.0));
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
        paint_fill_preview(ui.painter(), rect, fill);
        ui.painter()
            .rect_stroke(rect, egui::Rounding::same(3.0), Stroke::new(1.0, Color32::from_gray(90)));

        let popup_id = resp.id.with("fill_popup");
        if resp.clicked() {
            ui.memory_mut(|m| m.toggle_popup(popup_id));
        }
        egui::popup::popup_below_widget(
            ui,
            popup_id,
            &resp,
            egui::PopupCloseBehavior::CloseOnClickOutside,
            |ui| {
                out = Self::fill_picker(ui, fill, cfg);
            },
        );
        out
    }
}

impl ColorPopup {
    /// A fill-preview **swatch button** (no popup). The caller opens its own
    /// editor (e.g. the movable fill window) on click.
    pub fn fill_preview(ui: &mut Ui, fill: &Fill, size: Vec2) -> Response {
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
        paint_fill_preview(ui.painter(), rect, fill);
        ui.painter().rect_stroke(
            rect,
            egui::Rounding::same(3.0),
            Stroke::new(1.0, Color32::from_gray(90)),
        );
        resp
    }
}

/// Paint a preview of any fill kind into `rect` (checkerboard under alpha).
fn paint_fill_preview(painter: &egui::Painter, rect: Rect, fill: &Fill) {
    paint_checker(painter, rect);
    match &fill.kind {
        FillKind::None => {}
        FillKind::Solid(c) => {
            let b = cc::rgba_to_bytes(col_to_arr(*c));
            painter.rect_filled(
                rect,
                egui::Rounding::same(3.0),
                Color32::from_rgba_unmultiplied(b[0], b[1], b[2], b[3]),
            );
        }
        FillKind::Gradient(g) => {
            let stops = g.stops.clone();
            let interp = g.interpolation;
            paint_gradient(painter, rect, 32, 1, |fx, _| {
                let c = interpolate_stops_with(&stops, fx, interp);
                Color32::from_rgba_unmultiplied(
                    (c[0] * 255.0) as u8,
                    (c[1] * 255.0) as u8,
                    (c[2] * 255.0) as u8,
                    (c[3] * 255.0) as u8,
                )
            });
        }
        FillKind::FluidGradient(fg) => {
            let c = fg.points.first().map(|p| col_to_arr(p.color)).unwrap_or([0.5; 4]);
            let b = cc::rgba_to_bytes(c);
            painter.rect_filled(rect, egui::Rounding::same(3.0), Color32::from_rgb(b[0], b[1], b[2]));
        }
        FillKind::MeshGradient(mg) => {
            let c = mg.cell_colors.first().map(|c| col_to_arr(*c)).unwrap_or([0.5; 4]);
            let b = cc::rgba_to_bytes(c);
            painter.rect_filled(rect, egui::Rounding::same(3.0), Color32::from_rgb(b[0], b[1], b[2]));
        }
        FillKind::Pattern(_) => {
            painter.rect_filled(rect, egui::Rounding::same(3.0), Color32::from_gray(120));
        }
    }
}
