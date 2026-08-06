# Advanced character metrics: kerning, tracking, baseline shift, super/subscript (#32) — Design Proposal

> Status: design scaffold (not an implementation).

## Summary

`TextNode` already stores `letter_spacing` (tracking) and `opentype_features: Vec<String>`, but
pair kerning is not applied (renderer uses `Shaping::Basic` throughout), and baseline shift /
super/subscript have no model representation or UI. This issue wires those gaps end-to-end: model
→ renderer → GUI panel → MCP.

## Scope

**In scope**
- Switch glyphon shaping from `Shaping::Basic` to `Shaping::Advanced` to engage rustybuzz for
  `kern` and `calt` OpenType features already stored in `TextNode.opentype_features`.
- Add `baseline_shift: f64` and `vertical_align: VerticalAlign` (enum: Normal / Superscript /
  Subscript) to `TextNode` in `photonic-core/src/node.rs`.
- Extend `CharacterStyle` (`photonic-core/src/document.rs`, lines 410–431) with the same fields
  so presets carry them.
- Expose both attributes in the Character panel and via MCP set_text / update_node.

**Out of scope**
- Per-glyph manual kern pairs (only font-built pairs via OpenType `kern`).
- Optical kerning (algorithmic spacing not in the font).
- Rich-text runs (character-level formatting within a single node).

## Proposed approach

1. **Model** (`crates/photonic-core/src/node.rs`): add to `TextNode`:
   ```rust
   pub baseline_shift: f64,         // document units; positive = up
   pub vertical_align: VerticalAlign, // enum Normal / Superscript / Subscript
   ```
   Default both to zero / Normal. Update `TextNode::new()` defaults.

2. **`CharacterStyle`** (`document.rs` lines 410–431): mirror the two new fields as `Option`.

3. **Renderer** (`crates/photonic-render/src/renderer.rs`):
   - Change all three `Shaping::Basic` calls (lines 322, 437, 1395) to `Shaping::Advanced`. This
     activates rustybuzz (already a transitive dep of glyphon 0.6) for `kern`, `liga`, `calt`.
   - For `baseline_shift`: after computing the text block origin, translate the glyphon
     `TextArea.top` by `-baseline_shift * zoom` (positive shifts up).
   - For Superscript / Subscript: apply a conventional size factor (0.583×) and sign-matched
     vertical offset (approx. ±0.33 × font_size) to the snapshot before building the glyphon
     `Buffer`.
   - Extend `TextSnapshot` (the renderer-internal struct, around line 102) with the new fields
     so the pipeline receives them alongside existing `font_size`, `font_weight`, etc.

4. **GUI** (`crates/photonic-gui/src/app.rs` / panels): add numeric fields for "Baseline Shift"
   and a Superscript / Subscript toggle to the Character sub-section of the Properties panel.
   Emit `PanelAction::UpdateNode` on change.

5. **MCP** (`crates/photonic-mcp/src/handlers/nodes.rs`): extend `CreateTextArgs` and the
   existing `update_node` path to accept `baseline_shift` and `vertical_align`.

## Affected modules

- `crates/photonic-core/src/node.rs` — `TextNode`, new `VerticalAlign` enum
- `crates/photonic-core/src/document.rs` — `CharacterStyle` (lines 410–431)
- `crates/photonic-render/src/renderer.rs` — `TextSnapshot`, three `Shaping::Basic` sites, text
  layout pass
- `crates/photonic-gui/src/app.rs` — Properties panel Character section
- `crates/photonic-mcp/src/handlers/nodes.rs` — `CreateTextArgs`, `update_node`

## Risks & open questions

- **`Shaping::Advanced` throughput**: rustybuzz is slower than Basic; measure frame time on large
  text-heavy documents before landing. May need to cache shaped buffers.
- **Superscript/Subscript vs. OpenType `sups`/`subs` features**: size+shift approximation is
  safe for fonts without those features; for fonts that have them, prefer the feature over the
  manual scale. Decide which strategy to default to.
- **Export fidelity**: SVG export (`export.rs`) writes `font-size` and `letter-spacing` but
  has no baseline-shift attribute today. Needs a `dy` / `baseline-shift` attribute on the `<text>`
  element. PDF (future) would need similar handling.
- **Tracking vs. kerning terminology**: UI should label the existing `letter_spacing` as
  "Tracking" and the new font-kerning toggle as "Kerning" to match industry convention.

## Acceptance criteria

- [ ] Known kern pairs ("AV", "To", "WA") visually tighten when Kerning is on.
- [ ] Baseline shift moves glyphs up/down in the canvas and in SVG export.
- [ ] Superscript / subscript render at reduced size with correct vertical offset.
- [ ] `CharacterStyle` presets round-trip the new fields.
- [ ] MCP `create_text` and `update_node` accept and apply the new attributes.
- [ ] No measurable frame-rate regression on a 50-node text document.

## Effort estimate

**M** — model and export changes are small; renderer shaping swap is medium risk; GUI and MCP
wiring is straightforward.
