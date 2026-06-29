# Pattern Fills: Add PatternFill to the Model and Render It (#20) — Design Proposal

> Status: design scaffold (not an implementation).

## Summary

`FillKind` (`style.rs:62-68`) has variants for `None`, `Solid`, `Gradient`,
`FluidGradient`, and `MeshGradient` but no `Pattern` variant. Pattern fills are a
baseline professional feature. This proposal adds the data model, GPU rendering path,
SVG export, and MCP tools.

## Scope

**In:**
- `FillKind::Pattern` referencing a document-level pattern definition (tile geometry +
  transform + tile type + spacing).
- Tile types: grid, brick-by-row, brick-by-column (hex is a stretch goal).
- Vector tile defined as a group of scene nodes (symbol-style reference).
- Pattern transform (scale/rotate/translate) independent of the object transform.
- Render via stencil-clipped instanced tiling on GPU.
- SVG export via `<pattern>` in `<defs>`.
- MCP tools: `define_pattern`, `apply_pattern_fill`, `list_patterns`, `delete_pattern`.

**Out:**
- Raster image tiles (separate feature; requires embedded image support).
- Animated or procedural patterns.
- Pattern along path (separate).

## Proposed Approach

1. **Core model** — add to `crates/photonic-core/src/document.rs`:
   ```rust
   pub struct PatternDef {
       pub id: Uuid,
       pub name: String,
       pub tile_width: f64,
       pub tile_height: f64,
       pub tile_type: TileType,     // Grid | BrickRow | BrickCol
       pub transform: Transform,    // independent pattern space transform
       pub tile_node_ids: Vec<NodeId>, // group of nodes forming one tile
   }
   // added to Document:
   pub patterns: Vec<PatternDef>,
   ```
   Add `FillKind::Pattern(PatternFill)` to `style.rs`:
   ```rust
   pub struct PatternFill {
       pub pattern_id: Uuid,
       /// Additional per-object transform layered on top of PatternDef::transform.
       pub local_transform: Transform,
   }
   ```

2. **Rendering — stencil clip approach:**
   - In `build_geometry`, when a `PathNode` has `FillKind::Pattern`, tessellate the
     path's fill into a stencil mask (render to `wgpu::TextureFormat` with stencil).
   - Tessellate the tile nodes (via existing `tessellate_fill` / `tessellate_stroke`)
     and instance them across the object bounding box using the tile type's offset
     formula. Write instanced vertex data into the vertex buffer.
   - During the fill draw pass, use the stencil to clip tile instances to the shape.
   - Alternative (simpler for MVP): bake the tiled pattern into a repeating texture
     at the tile's native resolution and use a UV-based shader; trades GPU memory for
     simplicity. Evaluate which is feasible with the current single-vertex-buffer
     `Vertex { position, color }` (`pipeline.rs:3-9`).

3. **`NodeSnapshot` extension** (`renderer.rs:703`) — add:
   ```rust
   pattern_fill: Option<PatternFillSnapshot>, // pre-baked tile instances
   ```

4. **SVG export** (`export.rs`) — emit `<pattern id="..." patternUnits="userSpaceOnUse"
   patternTransform="...">` in `<defs>`, populate with the tile node SVG, and reference
   via `fill="url(#...)"` on the object.

5. **MCP handlers** — add `define_pattern`, `apply_pattern_fill`, `list_patterns`,
   `delete_pattern` in `crates/photonic-mcp/src/handlers/document.rs` or a new
   `patterns.rs`.

## Affected Modules

- `crates/photonic-core/src/style.rs` — `FillKind::Pattern`, `PatternFill`
- `crates/photonic-core/src/document.rs` — `PatternDef`, `Document::patterns`
- `crates/photonic-render/src/renderer.rs` — pattern detection, tile instancing
- `crates/photonic-render/src/pipeline.rs` — stencil or UV shader (new pipeline variant)
- `crates/photonic-render/src/tessellator.rs` — tile tessellation call site
- `crates/photonic-core/src/export.rs` — `<pattern>` in SVG defs
- `crates/photonic-mcp/src/handlers/` — new MCP tools

## Risks & Open Questions

- **Shader complexity:** the current `Vertex` struct carries only position + color; UV
  tiling needs either a UV attribute or a transform uniform. Adding a UV channel changes
  the vertex buffer layout and the pipeline.
- **Tile node rendering:** tile nodes must be rendered without their layer/node transforms
  being applied relative to the document origin; need a tile-local coordinate transform.
- **Performance:** instancing many small tiles for large objects can generate large vertex
  buffers. A texture-bake approach is faster but loses crispness at high zoom.
- **Stencil buffer:** wgpu surface config does not currently allocate a stencil. Must add
  `depth_stencil: Some(TextureFormat::Depth24PlusStencil8)` to the pipeline, a non-trivial
  renderer change.

## Acceptance Criteria

- [ ] A path filled with a grid pattern tiles correctly on-canvas, headless, and SVG.
- [ ] Pattern transform (scale/rotate/offset) is independent of the object transform.
- [ ] Brick-by-row and brick-by-column tile types offset correctly.
- [ ] `define_pattern` / `apply_pattern_fill` MCP tools round-trip through `.photonic`.
- [ ] SVG export renders correctly in a browser (Firefox, Chrome).

## Effort Estimate

**L** — new data model + shader changes + instancing + SVG export + MCP tools; the stencil
buffer addition alone is a significant renderer change.
