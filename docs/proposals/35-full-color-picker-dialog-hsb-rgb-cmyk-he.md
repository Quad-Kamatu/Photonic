# Full color picker dialog (HSB / RGB / CMYK / hex, swatches, eyedropper) (#35) — Design Proposal

> Status: design scaffold (not an implementation).

## Summary

Color input today is a basic solid-fill field. The `Color` struct (`photonic-core/src/color.rs`)
is linear sRGB (f32 r,g,b,a) with `from_hsl()` and `harmony()` helpers but no HSB or CMYK
conversion methods. The `Document` already tracks `recent_colors: Vec<Color>`, `color_swatches:
Vec<ColorSwatch>`, and `gradient_swatches`. `panels/mod.rs` has `EyedropperTarget` (with
`NodeFillSolid`, `NodeStroke`, etc.) and `FillColorSlot`. The task is a unified, reusable picker
component wired to all color-input sites.

## Scope

**In scope**
- Single `ColorPickerWidget` (egui custom widget) with: SV square + hue strip + alpha strip;
  HSB / RGB / CMYK / Hex numeric entry tabs; recent colors bar; save-to-swatch button.
- Reused everywhere a color is chosen: solid fill, stroke, gradient stops, fluid gradient points,
  mesh gradient vertices, glow/effect colors.
- Integrates `Document.color_swatches` and `Document.recent_colors`.
- Eyedropper via existing `EyedropperTarget` enum (fix Wayland sampling — tracked by #8 but
  wired here).

**Out of scope**
- CMYK *storage* / round-trip (depends on #36 document CMYK mode); CMYK entry converts to sRGB
  via a naive formula until #36 lands.
- Gradient swatch management (already present; picker only needs to display them, not edit).
- New swatch library format changes.

## Proposed approach

1. **Color model helpers** (`crates/photonic-core/src/color.rs`):
   Add conversion methods to `Color`:
   - `fn to_hsb(&self) -> (f32, f32, f32)` and `fn from_hsb(h, s, b, a) -> Self`
   - `fn to_cmyk_naive(&self) -> (f32, f32, f32, f32)` and `fn from_cmyk_naive(c, m, y, k, a) -> Self`
   These live on `Color` (linear sRGB); callers are responsible for gamut awareness.

2. **`ColorPickerWidget`** (new file `crates/photonic-gui/src/widgets/color_picker.rs`):
   A self-contained egui widget with internal state `ColorPickerState`:
   - `current: Color`
   - `mode: ColorMode` (HSB | RGB | CMYK | Hex)
   - `hue: f32` (kept separately so SV square doesn't collapse on achromatic colors)
   - Renders: SV 2D gradient texture (updated when hue changes, cached as egui texture),
     vertical hue bar, vertical alpha bar, tab row for mode, numeric inputs per mode,
     hex `#RRGGBBAA` text field, recent-colors strip (last 12 from `Document.recent_colors`),
     swatch grid (from `Document.color_swatches`), "Save swatch" button.
   - Returns `Option<Color>` on each frame (Some when changed).

3. **Integration** (`crates/photonic-gui/src/app.rs`, `panels/mod.rs`):
   - Replace all existing inline color inputs (fill panel, stroke panel, gradient stop editor,
     effect color fields) with `ColorPickerWidget`.
   - When picker returns `Some(color)`, emit the appropriate `PanelAction` variant:
     `UpdateNodeFill`, `UpdateNodeStroke`, `NodeFillGradStop`, etc.
   - "Save swatch" emits `PanelAction::ApplyColorSwatch` (already exists).
   - `record_recent_color` (`document.rs:667`) is called on every confirmed color pick.

4. **Eyedropper** (`panels/mod.rs` `EyedropperTarget`): the picker shows an eyedropper icon
   button; clicking sets `EyedropperState.target` to the relevant `EyedropperTarget` variant.
   Wayland fix (#8) is a prerequisite for pixel sampling to work on Wayland; the picker's
   eyedropper button should be disabled with a tooltip when running on Wayland until #8 is
   resolved, rather than appearing broken.

5. **Headless / MCP**: no changes needed; MCP already accepts hex colors as strings.

## Affected modules

- `crates/photonic-core/src/color.rs` — `to_hsb`, `from_hsb`, `to_cmyk_naive`, `from_cmyk_naive`
- `crates/photonic-gui/src/widgets/color_picker.rs` — new: `ColorPickerWidget`, `ColorPickerState`
- `crates/photonic-gui/src/panels/mod.rs` — replace inline color inputs; wire `EyedropperTarget`
- `crates/photonic-gui/src/app.rs` — eyedropper state integration, picker placement in panels

## Risks & open questions

- **SV gradient texture generation**: must regenerate a 256×256 texture when hue changes.
  egui's `TextureHandle` + `ColorImage` is the right path; measure frame time impact.
- **CMYK entry accuracy**: naive sRGB↔CMYK conversion is incorrect for print; values entered
  in CMYK will not round-trip faithfully until #36 (ICC-managed CMYK) lands. Make this explicit
  in the UI ("CMYK preview — accurate only in CMYK document mode").
- **Linear vs. perceptual display**: `Color` is linear sRGB; the picker UI must gamma-correct
  display values (convert to sRGB 2.2 for display) so the hue wheel looks correct.
- **Eyedropper on Wayland**: #8 is a prerequisite; the picker design must not block on it —
  show the button disabled if platform detection returns Wayland.
- **Component reuse**: ensure the widget is truly stateless/driven from outside (no hidden
  document mutation inside the widget) to keep the panel action pattern intact.

## Acceptance criteria

- [ ] Picker appears at every color-input site (fill, stroke, gradient stops, glow).
- [ ] HSB, RGB, and Hex modes each update the same live color coherently.
- [ ] CMYK tab shows values and a disclaimer; changing values updates the color display.
- [ ] Recent colors strip shows last 12 picked colors from `Document.recent_colors`.
- [ ] Saving a swatch appends to `Document.color_swatches` and appears in the swatch grid.
- [ ] Eyedropper button is disabled on Wayland with explanatory tooltip.
- [ ] No regression on existing color-swatch or gradient-swatch panel actions.

## Effort estimate

**M** — the widget itself is self-contained egui work; the SV gradient texture and conversion
math are well-defined; the main cost is wiring every existing color-input site.
