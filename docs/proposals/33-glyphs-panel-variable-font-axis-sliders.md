# Glyphs panel + variable-font axis sliders (width/slant) (#33) — Design Proposal

> Status: design scaffold (not an implementation).

## Summary

`TextNode` supports `font_weight (u16)` and `font_style: FontStyle` but has no fields for
variable-font axes beyond weight. `glyphon` 0.6 (used in `photonic-render`) wraps `rustybuzz`
and `swash`, both of which support variable fonts via `fontdb`. The missing pieces are: axis
metadata introspection, model storage, live sliders in the GUI, and a Glyphs panel for browsing /
inserting arbitrary codepoints.

## Scope

**In scope**
- Read available `fvar` axes from the currently-selected font (Width, Slant, Optical Size, and
  any named axis beyond Weight).
- Store axis values on `TextNode` as a map, rendered live via glyphon/swash.
- Glyphs panel: grid of glyphs for the active font, searchable by Unicode name/category,
  click-to-insert at cursor or as a new text node.
- Font picker with live preview (text sample rendered at current size).

**Out of scope**
- Custom axis naming / user-defined axes.
- Glyph palette persistence between sessions (deferred).
- Rich inline cursor placement within existing text runs (no rich-text model yet).

## Proposed approach

1. **Model** (`crates/photonic-core/src/node.rs`): add to `TextNode`:
   ```rust
   /// Variable-font axis overrides, e.g. {"wdth": 75.0, "slnt": -10.0}.
   pub var_axes: std::collections::HashMap<String, f32>,
   ```
   Serde: `#[serde(default)]` so existing files are unaffected.

2. **Axis introspection**: `photonic-render` (or a new `photonic-fonts` utility crate) exposes a
   function `fn query_fvar_axes(font_family: &str) -> Vec<FontAxis>` where `FontAxis` carries
   `{ tag: [u8;4], name: String, min: f32, default: f32, max: f32 }`. The implementation reads
   from glyphon's `FontSystem` (which wraps `fontdb`) using `swash`'s font introspection API.
   Result is cached keyed by family name.

3. **Renderer** (`crates/photonic-render/src/renderer.rs`): pass `var_axes` into glyphon `Attrs`
   when building each `Buffer`. `glyphon::Attrs` already accepts custom variation coordinates via
   `Attrs::stretch`/`Attrs::style` for known axes; arbitrary axes need the underlying
   `swash::CacheKey` variation tuple — verify glyphon 0.6 public API surface before committing.

4. **GUI panels** (`crates/photonic-gui/src/`):
   - **Variable axes sliders**: in the Properties / Typography panel, after Font Family + Weight,
     call `query_fvar_axes` on the current node's `font_family`; for each axis beyond `wght`
     render a labeled `egui::Slider`. On change emit `PanelAction::UpdateNode`.
   - **Glyphs panel** (`panels/mod.rs`): new `DrawerKind::Glyphs` variant (DrawerKind is the
     enum at `app.rs:106`). Panel state holds current font family + search string. For each glyph
     in the font's `cmap`, render a 40×40 cell with the glyph rendered via `photonic-render`'s
     headless text measurement path. Click emits `PanelAction::CreateShapeAtPos` with `ShapeKind::Text`
     (or extends an existing text selection).
   - **Font picker**: egui combo box with a type-ahead filter over system fonts from
     `fontdb::Database`; each entry renders a small preview via `measure_text` / a tiny glyphon
     buffer.

5. **MCP** (`crates/photonic-mcp/src/handlers/nodes.rs`): `CreateTextArgs` gains `var_axes:
   Option<HashMap<String, f32>>`. New tool `list_font_axes(font_family)` returns axis metadata.

## Affected modules

- `crates/photonic-core/src/node.rs` — `TextNode.var_axes`
- `crates/photonic-render/src/renderer.rs` — axis coordinates in `TextSnapshot` + glyphon `Attrs`
- `crates/photonic-render/src/lib.rs` — export `query_fvar_axes`
- `crates/photonic-gui/src/app.rs` — `DrawerKind::Glyphs`, font picker widget
- `crates/photonic-gui/src/panels/mod.rs` — Glyphs panel state + rendering
- `crates/photonic-mcp/src/handlers/nodes.rs` — `CreateTextArgs.var_axes`, `list_font_axes`

## Risks & open questions

- **glyphon 0.6 variable-axis API**: glyphon's public `Attrs` API may not expose arbitrary
  variation coordinates; may require calling into the underlying `swash` font cache directly or
  patching glyphon. Investigate before starting renderer work.
- **Glyph grid performance**: rendering hundreds of glyph cells via a GPU path per frame is
  expensive. Consider rendering to a texture atlas on font change and caching.
- **System font enumeration**: `fontdb::Database::load_system_fonts()` behaviour differs per OS;
  test on Linux (fontconfig) and Windows.
- **Insert-at-cursor**: without a rich-text cursor model, inserting a glyph mid-string requires
  either replacing the full content or deferring to a basic append. Spec the interaction clearly.

## Acceptance criteria

- [ ] A variable font (e.g. Inter Variable) shows Width / Slant sliders when selected.
- [ ] Moving a slider updates the canvas render live.
- [ ] Glyphs panel lists all glyphs in the active font; search by Unicode name narrows results.
- [ ] Clicking a glyph creates a single-character text node (or appends to selection).
- [ ] `var_axes` round-trips through save/load and SVG export (via `font-variation-settings`).
- [ ] MCP `list_font_axes` returns correct min/max/default for a known variable font.

## Effort estimate

**L** — axis introspection requires diving into glyphon internals; Glyphs panel is a substantial
new UI component; font picker with live preview is non-trivial egui work.
