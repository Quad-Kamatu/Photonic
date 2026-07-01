# 185 — Object fill picker shows a shifted color (gamma-sRGB fed into egui linear Rgba picker)

## Status — implemented

**Shared helper (single correct code path)**
Every gamma-sRGB color picker now routes through one bridge in a new module
`crates/photonic-gui/src/color_edit.rs`:
- `srgb_f32_color_edit(ui, &mut [f32; 4]) -> Response` — for `[f32; 4]` stores that map
  1:1 into `Color`.
- `srgb_color_edit(ui, &mut photonic_core::Color) -> Response` — thin wrapper for
  `Color`-backed values.
Both convert to `[u8; 4]`, drive egui's `color_edit_button_srgba_unmultiplied` (which
interprets bytes as gamma sRGB — matching the renderer and document swatch), and write the
edited channels back only on `.changed()`. Returning the `Response` lets call sites still
chain `.on_hover_text(..)` and inspect `.changed()`. This eliminates every linear-`Rgba`
picker path for gamma-sRGB values, so no two co-bound pickers can disagree.

**Pickers converted (all Color-convention / gamma-sRGB stores)**
- `crates/photonic-gui/src/app/mod.rs`:
  - `:3276` always-visible rail active-fill swatch (#172), editing `self.fill_color`.
  - `:2628` Tool Defaults → Default Fill (`prefs.default_fill_color`, mirrored into
    `self.fill_color`).
  - `:2636` Tool Defaults → Stroke Color (`prefs.default_stroke_color`).
- `crates/photonic-gui/src/panels/mod.rs`:
  - `draw_tool_shape_options` New Shape Fill (`&mut [f32; 4]` wired to `self.fill_color`).
  - `draw_fill_editor` Solid fill (`FillKind::Solid`).
  - Gradient stop, fluid-gradient point, mesh-gradient vertex.
  - Stroke color, and both glow color pickers (drop-glow + gaussian-glow).
- `crates/photonic-core/src/color.rs` doc comment corrected from "linear sRGB" to an
  accurate gamma/sRGB-encoded storage contract (with a note on the picker convention).

All of these edit values that flow into `Color` (gamma sRGB) and therefore now agree with
the renderer, the document swatch, and each other. No change to `Color` storage,
`to_hex`/`from_hex`/`to_rgba8`, or the GPU boundary.

**Deliberately NOT converted (and why)**
- `app/mod.rs:2566` **Grid Color** (`prefs.grid_color`) stays on
  `color_edit_button_rgba_unmultiplied`. This value is NOT part of the #185 bug: it is
  rendered via `egui::Rgba::from_rgba_unmultiplied(..)` at `app/mod.rs:~3675`, i.e. it is
  treated as **linear** on both the picker side and the render side. It is self-consistent
  already; routing it through the sRGBA bridge would introduce a new mismatch. Left as-is
  intentionally.

**Remaining work / deferred**
- Re-plumbing the whole pipeline to true linear color is deliberately deferred (noted in
  the issue as the larger alternative); it is not needed to fix the swatch shift.
- #35 (full color-picker dialog) and #8 (eyedropper) are related but out of scope here.

## Summary
`photonic_core::Color` stores channels as **gamma sRGB** (`from_hex`/`to_hex`/`to_rgba8`
use a plain `u8/255.0` with no gamma decode — `crates/photonic-core/src/color.rs`). The
document swatch picker treats it correctly (via `Color32` + `color_picker_color32`, which
egui interprets as gamma sRGB). But a whole family of GUI pickers fed the same gamma-sRGB
values into `color_edit_button_rgba_unmultiplied`, whose `[f32;4]` egui interprets as
**linear** `Rgba`. Result: those pickers show/suggest a shifted swatch, and editing
round-trips the color to a slightly different value. This aligns every gamma-sRGB picker on
the single sRGB path the renderer already uses (shared helper in `color_edit.rs`).

The affected pickers (all now fixed): the active-fill rail swatch, Tool Defaults
Default Fill / Stroke Color, the New Shape Fill picker, the Solid fill editor, gradient
stop, fluid-gradient point, mesh-gradient vertex, stroke color, and both glow color
pickers. The only remaining `color_edit_button_rgba_unmultiplied` call is Grid Color, which
is linear on both the picker and render side and is therefore correct as-is (see Status).

## Scope

### In
- New shared helpers in `crates/photonic-gui/src/color_edit.rs`
  (`srgb_f32_color_edit` / `srgb_color_edit`) that bridge gamma-sRGB
  `[f32; 4]` / `Color` values through `color_edit_button_srgba_unmultiplied`.
- Every gamma-sRGB picker routed through those helpers: rail active-fill swatch, Tool
  Defaults Default Fill + Stroke Color (`app/mod.rs`); New Shape Fill, Solid fill, gradient
  stop, fluid-gradient point, mesh-gradient vertex, stroke color, drop-glow + gaussian-glow
  color (`panels/mod.rs`). See Status for the exact list.
- Fix the misleading doc comment `crates/photonic-core/src/color.rs:3` ("linear sRGB" →
  gamma / sRGB-encoded), so the storage contract is stated accurately.

### Out
- No change to `Color` storage, `to_hex`/`from_hex`, or the GPU/render boundary — the
  renderer and document swatch already agree on gamma sRGB; we make every stray picker match
  them rather than re-plumb the whole pipeline to linear (that larger option is noted in the
  issue but deliberately deferred).
- Grid Color (`app/mod.rs:2566`) is intentionally left on the linear picker — it is linear
  on both the picker and render side, so it is already self-consistent (see Status).
- #35 (full color-picker dialog) and #8 (eyedropper) — related but separate; not touched.

## Approach
egui exposes `Ui::color_edit_button_srgba_unmultiplied(&mut [u8;4])`, which interprets the
bytes as gamma sRGB — exactly how `Color` is stored and how the document swatch path already
behaves. Rather than duplicate the conversion at each of the 11 call sites, it is factored
into two helpers in `color_edit.rs`:

1. `srgb_f32_color_edit(ui, &mut [f32; 4])`: convert `(c*255.0).round() as u8`, drive the
   srgba picker, and on `.changed()` write each channel back as `u8 as f32 / 255.0`. Returns
   the `Response` so callers keep `.on_hover_text(..)` / `.changed()`.
2. `srgb_color_edit(ui, &mut Color)`: thin wrapper over (1) reading/writing the four `Color`
   channels in place.
3. Each former `color_edit_button_rgba_unmultiplied` call site (except Grid Color) now calls
   the appropriate helper, removing all per-site `[f32;4]`↔`Color` reconstruction.
4. **Doc comment** (`color.rs:3`): correct "linear sRGB" to describe gamma/sRGB-encoded
   storage.

## Verification
- `cargo build --release` succeeds.
- The object fill picker swatch now matches the rendered object color (no shift), and the
  New Shape Fill swatch matches the color of shapes it creates — both consistent with the
  document swatch picker.
