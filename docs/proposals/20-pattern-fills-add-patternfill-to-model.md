# Pattern Fills: add `PatternFill` to the model and render it (#20)

> Status: IMPLEMENTED (raster-tile pattern fills). Tiles correctly in headless
> render, raster export, and SVG export; pattern transform is independent of the
> object transform. One piece is deferred — see "Remaining work".

## What this PR implements

- **Core model** (`crates/photonic-core/src/style.rs`): `FillKind::Pattern(PatternFill)`,
  a self-contained `PatternFill { tile: RasterImage, tile_type, scale, rotation,
  offset, spacing }`, the `PatternTileType` enum (`Grid` / `BrickByRow` /
  `BrickByColumn` / `Hex`), `PatternFill::sample_at` (inverse-transform → layout
  shift → wrap → bilinear sample, transparent in the gutter), the
  `FillKind::sample_at` arm, and `Fill::pattern`. Serializes as
  `{"type":"pattern", ...}` with the tile embedded as a base64 PNG. Seven unit
  tests cover wrap periodicity, gutter transparency, brick row-shift, rotation,
  transform independence, and serde round-trip.
- **Document registry** (`document.rs`): `Document::patterns: Vec<Pattern>` and the
  `Pattern { id, name, fill }` struct, mirroring `symbols` / `width_profiles`.
- **Rendering** — patterns are sampled **per pixel**. The CPU compositor
  (`compositor.rs`) already samples `FillKind::sample_at` per pixel, so it tiles
  with zero changes. The GPU render paths (headless vector path, live canvas)
  colour fills at their *mesh vertices* — fine for gradients, wrong for a tiled
  pattern. The headless renderer therefore routes any document containing a
  pattern fill through the CPU compositor (the same route raster documents
  already take), giving pixel-accurate tiling in headless render and in every
  GUI export (PNG/JPEG/WebP/…). A GPU headless test
  (`headless::blend_tests::pattern_fill_tiles_and_is_transform_independent`)
  renders a checker-filled rectangle and asserts both on-render tiling and that
  translating the shape by a whole tile leaves document pixels unchanged.
- **SVG export** (`export.rs`): a `<pattern patternUnits="userSpaceOnUse">` def
  wrapping a base64 `<image>`, with a `patternTransform` matching the pattern
  transform, referenced via `fill="url(#patN)"`.
- **MCP** (`protocol.rs`, `handlers/document.rs`, `server.rs`): `define_pattern`
  (tile from a file path or inline base64), `apply_pattern_fill` (resolve a
  registry entry, embed a clone of its tile on each node, with per-application
  transform overrides — undo-safe batch), `list_patterns`, `delete_pattern`, plus
  a self-contained `FillArg::Pattern { tile_base64, … }` variant so the existing
  fill-setting tools (create_shape/create_path/update_node/…) can set a pattern
  directly. `docs/mcp-api.md` regenerated (300 → 304 tools).
- **GUI** (`panels/mod.rs`): "Pattern" in the fill-type selector/classifier, a
  default checker tile when switching a fill to Pattern, and a pattern inspector
  (layout buttons, scale/rotation/offset/spacing controls). Forced match arms in
  the colour ops (invert / grayscale / adjust) operate on the tile pixels.

## Remaining work (deferred)

- **Live interactive GPU canvas tiling.** The GUI's *interactive* canvas
  (`renderer.rs`, GPU, per-vertex colour) shows a pattern fill sampled at the
  fill's mesh vertices (an averaged smear), not tiled. The canonical tiled result
  is correct in headless render and in all exports (which the GUI uses for
  PNG/SVG/etc.). True live GPU tiling needs a fragment-shader pattern sampler
  (upload the tile as a texture, per-node pattern-transform uniforms) — a
  self-contained pipeline addition left as a follow-up.
- **Text + pattern in one document, headless.** The CPU compositor does not paint
  glyph text (a long-standing limitation that already affects raster+text
  documents in headless). A document mixing pattern fills and text follows the
  same limitation when rendered headless.
- The original "Out" items still stand: live vector/symbol tiling, GPU-instanced
  tiling for very large fills, pattern fills on strokes, and seamless/auto-tile
  synthesis. SVG brick/hex staggers are approximated by the grid cell
  (on-canvas/headless remain the source of truth for exact stagger).

## Summary

`FillKind` (`crates/photonic-core/src/style.rs:61-92`) has five variants — `None`,
`Solid`, `Gradient`, `FluidGradient`, `MeshGradient` — but no pattern variant. Every
fill in Photonic is painted through one mechanism: `FillKind::sample_at(x, y, opacity)`
is called per-pixel in document space by the compositor (`compositor.rs:162`), the
headless renderer (`headless.rs:700`), and the GPU CPU-sample path (`renderer.rs:1054`).
A pattern fill that tiles a raster tile across the filled path fits this model exactly —
if the variant is self-contained (carries its own tile pixels), it works in all three
render paths with zero plumbing, the same way `Gradient` carries its own stops.

This proposal adds a **raster-tile** pattern fill: an embedded RGBA tile repeated across
the filled path with an independent pattern transform (scale/rotate/offset), inter-tile
spacing, and a tile layout (grid / brick-by-row / brick-by-column / hex). Vector/symbol
tiles are rasterized to the tile buffer at definition time (or deferred — see Out).

## Scope

**In:**
- New `FillKind::Pattern(PatternFill)` variant in `style.rs`, self-contained: holds an
  embedded `RasterImage` tile plus tile-space metadata.
- `PatternFill::sample_at(x, y, opacity)`: map document-space `(x, y)` through the
  inverse pattern transform into tile space, apply the layout offset (brick/hex row
  shift), wrap into `[0, tile_w+spacing) × [0, tile_h+spacing)`, and bilinearly sample
  the tile (transparent in the spacing gutter). Wire it into `FillKind::sample_at`.
- Pattern transform independent of the object transform: stored as scale, rotation
  (radians), and offset on `PatternFill`, applied in document space — satisfies the
  second acceptance criterion directly.
- Tile layouts: `Grid`, `BrickByRow`, `BrickByColumn`, `Hex` (a `PatternTileType` enum).
- SVG export: emit a `<pattern>` in `<defs>` (the tile encoded as a base64 `<image>`,
  reusing the same base64 path the raster export already uses) with
  `patternUnits="userSpaceOnUse"` and a `patternTransform` matching the pattern
  transform; reference it via `fill="url(#patN)"` in `fill_attrs`
  (`crates/photonic-core/src/export.rs:436`).
- MCP tools: `define_pattern` (build a tile from an image path / a baked symbol-or-group
  ID / a generated swatch, returning a pattern id), `apply_pattern_fill` (set
  `FillKind::Pattern` on target nodes with a transform), `list_patterns`,
  `delete_pattern`. Patterns stored in a document-level registry
  `Document::patterns: Vec<Pattern>` (mirrors `width_profiles`/`symbols` at
  `document.rs:610-624`); `apply_pattern_fill` resolves the registry entry and embeds a
  clone of its tile into the `PatternFill` so the fill stays self-contained for rendering.
- GUI: pattern shows up in the fill-type classifier (`panels/mod.rs:6170`) and renders
  in the canvas through the existing `sample_at` path; a minimal pattern row in the fill
  inspector (transform sliders) so it is editable.

**Out (deferred):**
- Live vector/symbol tiling that re-renders the source group every frame (instanced GPU
  tiling). Baseline bakes the symbol/group to a raster tile once at `define_pattern`
  time; live re-tiling on master edit is a follow-up tied to #29 (symbol propagation).
- GPU-instanced tiling pipeline for large fills (CPU bilinear sample is the baseline,
  consistent with how gradients already render on the CPU sample path).
- Pattern fills on strokes (fill-only for this pass).
- Seamless/auto-tileable tile synthesis and per-tile color randomization.

## Proposed Approach

1. **Core model (`style.rs`).** Add:
   ```rust
   pub enum PatternTileType { Grid, BrickByRow, BrickByColumn, Hex }
   pub struct PatternFill {
       pub tile: RasterImage,        // embedded RGBA8 tile (self-contained)
       pub tile_type: PatternTileType,
       pub scale: f64,               // pattern transform — independent of object
       pub rotation: f64,            // radians
       pub offset: [f64; 2],         // document-space anchor
       pub spacing: f64,             // gutter between tiles, tile px
   }
   ```
   Add `Pattern(PatternFill)` to `FillKind` and a `PatternFill::sample_at` that does the
   inverse-transform → layout-shift → wrap → bilinear-sample described above. Extend
   `FillKind::sample_at` with the new arm. `RasterImage` already lives in core
   (`crates/photonic-core/src/raster/image.rs:13`), so the variant adds no new dep.
   Keep `#[serde(tag = "type", rename_all = "snake_case")]` so it serializes as
   `{"type":"pattern", ...}`.

2. **Document registry (`document.rs`).** Add `pub patterns: Vec<Pattern>` (with
   `#[serde(default)]`) and a `Pattern { id: Uuid, name: String, fill: PatternFill }`
   struct next to `Symbol`/`WidthProfile`. Initialize to empty in `Document::new`.

3. **Render paths.** No edits needed beyond the `match` arms the compiler forces in
   `compositor.rs:155`, `headless.rs:692`, and `renderer.rs:871/901/1054` — the
   per-pixel `sample_at` dispatch already covers pattern once the variant exists. Verify
   each non-exhaustive match is updated.

4. **SVG export (`export.rs`).** In `fill_attrs`, add a `FillKind::Pattern` arm that
   pushes a `<pattern>` def (unique id via the existing `counter`) wrapping a base64
   `<image>` sized to the tile, sets `patternTransform="rotate(..) scale(..)
   translate(..)"`, and returns `fill="url(#patN)"`. Brick/hex layouts are approximated
   by tile-size doubling + offset (documented limitation; on-canvas/headless remain the
   source of truth for exact hex).

5. **MCP (`protocol.rs` + `handlers/`).** Add arg structs (`DefinePatternArgs`,
   `ApplyPatternFillArgs`, `ListPatternsArgs`, `DeletePatternArgs`) and a
   `FillArg::Pattern { pattern_id, scale, rotation, offset, spacing }` so existing
   fill-setting tools can also select a pattern. Implement handlers in
   `handlers/nodes.rs` (apply) and a small registry helper (define/list/delete),
   following the gradient handler precedent. `define_pattern` accepts an image file path
   (decode via the existing image loader used by raster import) or a node id to bake.

6. **GUI (`panels/mod.rs`).** Extend the fill-type classifier (`6170`) and add a
   `Pattern` branch to the fill inspector with transform sliders; canvas rendering is
   automatic via `sample_at`.

## Acceptance Criteria Mapping

- *Tiles correctly on-canvas, headless, and SVG* → step 3 (shared `sample_at` covers
  canvas + headless) and step 4 (`<pattern>` def).
- *Pattern transform independent of object transform* → step 1: scale/rotation/offset
  live on `PatternFill` and are applied in document space inside `sample_at`, never
  composed with the node transform.

## Tests

- Unit: `PatternFill::sample_at` wrap math and brick/hex row-shift at known coords;
  rotated/scaled transform maps a known doc point to the expected tile texel.
- Round-trip: serialize a `FillKind::Pattern` document and reload; SVG export contains a
  `<pattern>` def referenced by `fill="url(#...)"`.
- Headless: render a rectangle with a 2-color checker tile; assert tiling period and
  that the pattern does not move when the rectangle is translated by a whole tile.
