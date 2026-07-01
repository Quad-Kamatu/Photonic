//! Shared color-picker widgets that bridge gamma-sRGB values into egui's
//! sRGBA (`[u8; 4]`) picker.
//!
//! `photonic_core::Color` — and every `[f32; 4]` GUI store that maps 1:1 into
//! it — holds channels as **gamma-encoded sRGB** (plain `u8 / 255.0`, no gamma
//! decode; see `photonic_core::color`). egui's
//! `color_edit_button_rgba_unmultiplied` interprets its `[f32; 4]` as **linear**
//! `Rgba`, which renders and round-trips a shifted swatch (issue #185).
//! `color_edit_button_srgba_unmultiplied` interprets its `[u8; 4]` as gamma
//! sRGB — matching the renderer and the document swatch path — so every fill
//! picker is routed through these helpers to keep one correct code path.

/// Gamma-sRGB color picker over a `[f32; 4]` store.
///
/// Converts the float channels to `[u8; 4]`, drives egui's sRGBA picker, and
/// writes the (possibly edited) bytes back as `u8 / 255.0` when the picker
/// reports a change. Returns the picker `Response` so callers can chain
/// `.on_hover_text(..)` / inspect `.changed()`; the write-back has already
/// happened by the time this returns.
pub fn srgb_f32_color_edit(ui: &mut egui::Ui, rgba: &mut [f32; 4]) -> egui::Response {
    let mut srgba = [
        (rgba[0] * 255.0).round() as u8,
        (rgba[1] * 255.0).round() as u8,
        (rgba[2] * 255.0).round() as u8,
        (rgba[3] * 255.0).round() as u8,
    ];
    let resp = ui.color_edit_button_srgba_unmultiplied(&mut srgba);
    if resp.changed() {
        rgba[0] = srgba[0] as f32 / 255.0;
        rgba[1] = srgba[1] as f32 / 255.0;
        rgba[2] = srgba[2] as f32 / 255.0;
        rgba[3] = srgba[3] as f32 / 255.0;
    }
    resp
}

/// Gamma-sRGB color picker over a `photonic_core::Color`.
///
/// Thin wrapper over [`srgb_f32_color_edit`] that reads/writes the four
/// channels of a `Color` in place. Returns the picker `Response`.
pub fn srgb_color_edit(ui: &mut egui::Ui, color: &mut photonic_core::Color) -> egui::Response {
    let mut rgba = [color.r, color.g, color.b, color.a];
    let resp = srgb_f32_color_edit(ui, &mut rgba);
    if resp.changed() {
        color.r = rgba[0];
        color.g = rgba[1];
        color.b = rgba[2];
        color.a = rgba[3];
    }
    resp
}
