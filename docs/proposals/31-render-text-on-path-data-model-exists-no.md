# Render text-on-path (data model exists, not rendered) (#31) — Design Proposal

> Status: design scaffold (not an implementation).

## Summary

`TextNode.path_spine_id: Option<NodeId>` and `path_offset: f64` are stored (node.rs
lines 328-331). `set_text_path` and `clear_text_path` MCP handlers exist in
`photonic-mcp/src/handlers/nodes.rs` (lines 15139, 15209). The renderer's
`render_text_pass` (renderer.rs lines 406-507) constructs a glyphon `Buffer` at a fixed
screen position and ignores `path_spine_id` entirely. SVG export also omits `<textPath>`.
This proposal adds arc-length parameterisation of the spine path and glyph placement/
orientation along it.

## Scope

**In:**
- Arc-length parameterisation of the spine `PathData` using kurbo's `BezPath` (which
  already provides `arclen()` and `eval()` at a parameter `t`).
- Glyph advance: accumulate glyph widths (from glyphon's `Buffer` layout) as arc-length
  positions, starting at `path_offset`.
- Glyph transform: at each glyph centre position, sample the tangent direction from the
  spine and rotate the glyph accordingly.
- Honour `letter_spacing` along the arc.
- `TextAlign` for the path: left = start at offset, center = centre the full run on the
  spine, right = end at the spine end.
- SVG export via `<defs><path id="spine_<id>" .../>` + `<text><textPath href="#spine_<id>"
  startOffset="...">`.
- Live update: when the spine path node is edited, the text re-renders at the new positions.
- Handle path direction reversal (flip the text baseline).

**Out:**
- Text above/below path toggle (stretch goal).
- Glyph skew to follow path curvature (only rotation, not shear).
- Multi-run text on path (only the full `content` string, as one run).

## Proposed approach

1. **Arc-length helper** (`photonic-core/src/path.rs`):
   ```rust
   /// Sample the point and tangent angle at arc-length `s` along `path`.
   pub fn sample_at_arc_length(path: &PathData, s: f64) -> Option<(f64, f64, f64)>
   //  returns (x, y, angle_radians)
   ```
   Implementation: use `kurbo::BezPath::arclen(acc)` to get total length, then binary
   search on `BezPath::eval(t).arclen(acc) ≈ s` to find the parameter `t`.
   `BezPath::eval(t)` gives the point; tangent is `BezPath::tangent(t)` or finite
   difference. This avoids adding new crate dependencies beyond kurbo (already present).

2. **Glyph positions** (`photonic-render/src/renderer.rs`, `render_text_pass`):
   When `text_node.path_spine_id.is_some()`:
   - Look up the spine `SceneNode` from `doc.nodes`.
   - Get its `PathData` from `SceneNodeKind::Path(p).path_data`.
   - Use glyphon's `Buffer` to obtain glyph advances (one-line layout at infinite width)
     — this gives each glyph's x advance in screen units.
   - For each glyph i: cumulative arc-length `s_i = path_offset + Σ advances[0..i]` (with
     `letter_spacing` added per gap).
   - Call `sample_at_arc_length(spine_path, s_i * zoom)` to get `(sx, sy, angle)`.
   - Emit the glyph as a `custom_glyph` entry with position `(sx, sy)` and a rotation
     transform applied to the glyph's wgpu quad, or use glyphon's
     `CustomGlyph` mechanism if it supports per-glyph transforms.
   - If glyphon does not support per-glyph rotation natively, fall back to
     tessellating each glyph outline as a path (expensive but correct).

3. **TextAlign on path**: before computing per-glyph positions, compute the total run
   width; for `Center`, subtract `total_width / 2` from `path_offset`; for `Right`,
   subtract `total_width`.

4. **Headless capture** (`photonic-render/src/headless.rs`): same code path as live
   render — no separate implementation needed if the render function is shared.

5. **SVG export** (`photonic-core/src/export.rs`):
   When `text_node.path_spine_id.is_some()`:
   - Look up the spine path, emit `<defs><path id="spine_{spine_id}" d="..."/></defs>`.
   - Emit `<text><textPath href="#spine_{spine_id}" startOffset="{path_offset}">
     {content}</textPath></text>`.
   - SVG `textPath` handles alignment via `text-anchor` attribute (maps from `TextAlign`).

6. **Live update wiring**: the spine is a separate `SceneNode`. When the spine is mutated
   via a `Command` in `history.rs`, the text nodes that reference it must be re-rendered.
   Since the renderer re-reads `doc.nodes` each frame, live canvas update is automatic
   once the render pass checks `path_spine_id`. No additional invalidation needed.

## Affected modules (real paths)

- `crates/photonic-core/src/path.rs` — `sample_at_arc_length()` helper
- `crates/photonic-render/src/renderer.rs` — `render_text_pass` (lines 406-507) and
  `collect_draw_nodes` equivalent for text
- `crates/photonic-render/src/headless.rs` — capture text pass (lines ~1421-1459)
- `crates/photonic-core/src/export.rs` — `<textPath>` SVG emission
- `crates/photonic-mcp/src/handlers/nodes.rs` — `set_text_path` (line 15139) already
  exists; no handler changes needed

## Risks & open questions

- **Glyphon per-glyph rotation**: glyphon's `TextArea` API does not expose per-glyph
  affine transforms. The `custom_glyphs` field in `TextArea` accepts a `CustomGlyph`
  slice — check if it supports rotation. If not, the fallback is to outline each glyph
  via swash and tessellate it as a `PathNode` (slow for large text; acceptable for MVP).
- **Arc-length accuracy**: kurbo's `arclen` uses a tolerance parameter. At low zoom,
  `acc = 0.5` is fine; at high zoom, tighter tolerance needed. Adaptive tolerance
  based on `zoom` is recommended.
- **Spine node visibility**: the spine path is typically invisible (no fill/stroke).
  The renderer currently skips invisible nodes but needs the spine path for sampling.
  Must look up the spine by `NodeId` directly from `doc.nodes`, not from the draw order.
- **Path direction reversal**: if the text appears upside-down, add a `flip_on_path:
  bool` field to `TextNode` (default false). For now, document that users should reverse
  path direction as a workaround.
- Open: should `path_offset` be in document units or as a proportion of total path
  length (0.0–1.0)? Current model stores it as document units (consistent with other
  offsets); SVG `startOffset` supports both — emit as absolute length.

## Acceptance criteria

- [ ] Text with `path_spine_id` set renders with each glyph positioned and rotated along
      the spine path on canvas and in headless PNG export.
- [ ] Editing the spine path (moving anchor points) immediately updates text placement
      on canvas.
- [ ] `path_offset` shifts the start of the text run along the spine.
- [ ] `letter_spacing` is honoured as additional arc-length between glyphs.
- [ ] SVG export emits a valid `<textPath>` element that browsers render correctly.
- [ ] `clear_text_path` detaches the spine and returns the text to point-type rendering.

## Effort estimate

**M** — Arc-length parameterisation with kurbo is well-defined; the main unknown is
glyphon's per-glyph transform support. If outlining is needed as a fallback, effort rises
to L.
