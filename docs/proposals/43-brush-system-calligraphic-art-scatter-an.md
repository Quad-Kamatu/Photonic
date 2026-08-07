# Brush System: Calligraphic, Art, Scatter, and Pattern Brushes (+ Expand) (#43) — Design Proposal

> Status: design scaffold (not an implementation).

## Summary

No brush system exists. The current `Stroke` struct in `crates/photonic-core/src/style.rs` handles only uniform-width solid strokes (plus `StrokeAlign`, `LineCap`, `LineJoin`, `ArrowheadStyle`). A `WidthProfile` type is already defined in `document.rs` (for variable-width strokes), and `ops/stroke_outline.rs` can expand a stroke to a filled outline path. The brush system builds on top of these: a `BrushDefinition` is stored in the document's brush library; a `PathNode`'s stroke may reference a brush by ID; the renderer instantiates the brush geometry along the path at draw time. "Expand Appearance" calls into `ops/stroke_outline.rs` or the brush-specific expand logic to convert to plain `PathNode`s.

## Scope (in / out)

**In:**
- **Data model**: `BrushKind` enum with four variants; `BrushDefinition` stored in `Document::brushes`; `Stroke::brush_id: Option<BrushId>` to attach a brush to a path node.
- **Calligraphic**: angle, roundness, size; optional pressure-mapped size/opacity.
- **Art brush**: stretch/tile a `PathData` artwork along the stroke path; options for stretch-to-fit, tile along, stretch between guides.
- **Scatter brush**: distribute copies of a `PathData` along the stroke; controls for size, spacing, scatter offset, rotation, colorization.
- **Pattern brush**: separate `PathData` tiles for side, corner-outer, corner-inner, and end caps.
- **Paintbrush tool** in `photonic-gui`: new entry in `tools/mod.rs`; draws a new `PathNode` with the active brush. Smooth pressure curve if the input device reports pressure.
- **Blob Brush**: merges strokes into a single filled compound path (uses `BooleanOp::Union` from `ops/boolean.rs`).
- **Expand Appearance** command: converts a brushed `PathNode` to a set of plain `PathNode`s using the brush geometry (art/scatter) or `ops/stroke_outline.rs` (calligraphic).
- **Brushes panel**: list, preview, create, delete brushes; drag to path.
- **MCP tools**: `define_brush(kind, params)`, `apply_brush(node_id, brush_id)`, `expand_brush(node_id)`.
- **SVG export** (#39): calligraphic brush → expand to outlined path on export (since SVG has no brush primitive); art/scatter/pattern → expand similarly.

**Out:**
- Pressure input beyond a simple linear size-to-pressure map — advanced tilt/azimuth handling deferred.
- Bristle brushes (raster simulation) — out of scope.
- Real-time tablet pressure during freehand drawing in the GUI (requires input device API integration; mark as enhancement).
- Raster brush strokes (Blob Brush producing pixel layers).

## Proposed Approach

1. **Data model** (`crates/photonic-core/src/node.rs` or a new `brush.rs`):

```rust
pub type BrushId = Uuid;

pub enum BrushKind {
    Calligraphic {
        angle: f64,          // degrees
        roundness: f64,      // 0.0–1.0
        size: f64,           // pt
        pressure_size: bool,
        pressure_opacity: bool,
    },
    Art {
        artwork: PathData,
        stretch_mode: ArtStretchMode, // StretchToFit | TileAlong | StretchBetweenGuides
        flip_x: bool,
        flip_y: bool,
    },
    Scatter {
        artwork: PathData,
        size_range: (f64, f64),
        spacing_range: (f64, f64),
        scatter_range: (f64, f64),
        rotation_range: (f64, f64),
        rotation_relative: bool,
    },
    Pattern {
        side_tile: PathData,
        corner_outer: Option<PathData>,
        corner_inner: Option<PathData>,
        start_cap: Option<PathData>,
        end_cap: Option<PathData>,
    },
}

pub struct BrushDefinition {
    pub id: BrushId,
    pub name: String,
    pub kind: BrushKind,
}
```

2. **Document integration**: Add `pub brushes: Vec<BrushDefinition>` to `Document` in `document.rs`. Add `pub brush_id: Option<BrushId>` to `Stroke` in `style.rs`.

3. **Expand logic** (`crates/photonic-core/src/ops/brush_expand.rs` — new file):
   - `pub fn expand_brush(path: &PathData, brush: &BrushDefinition, stroke: &Stroke) -> Vec<PathData>` — returns a list of filled paths.
   - **Calligraphic**: sample `path` at equal arc-length intervals; at each sample, compute an ellipse oriented at `angle` with `roundness`; union all ellipses into a compound path using `BooleanOp::Union` (`ops/boolean.rs`).
   - **Art / Scatter**: parameterize `path` by arc length; at each placement point, apply a `Transform` (scale + rotation + translation) to the artwork `PathData`.
   - **Pattern**: split `path` into segments; tile the side tile along each segment; place corner tiles at joins.

4. **Blob Brush** (`ops/blob_brush.rs` — new file): accumulate strokes as overlapping filled paths; after each new stroke, `BooleanOp::Union` with the current compound path on the same layer.

5. **Renderer integration** (`crates/photonic-render/src/renderer.rs`): When rendering a `PathNode` with `stroke.brush_id.is_some()`, call `expand_brush` to get geometry and feed it to the tessellator/fill pipeline. Cache expanded geometry per (node_id, transform_hash) to avoid re-expansion every frame.

6. **GUI tool** (`crates/photonic-gui/src/tools/mod.rs`): Add `PaintbrushTool` state machine; on pointer-down begin a new freehand `PathData`; on pointer-up emit `Command::AddNode` with the brush applied. Add a `BlobBrushTool` that calls the blob union path.

7. **History**: Expand Appearance emits `Command::Batch` of `Command::RemoveNode` (the brushed path) + `Command::AddNode` for each expanded path.

## Affected Modules

- `crates/photonic-core/src/node.rs` or new `brush.rs` — `BrushDefinition`, `BrushKind`, `BrushId`
- `crates/photonic-core/src/style.rs` — `Stroke::brush_id` field
- `crates/photonic-core/src/document.rs` — `Document::brushes: Vec<BrushDefinition>`
- `crates/photonic-core/src/ops/brush_expand.rs` — new: expand logic
- `crates/photonic-core/src/ops/blob_brush.rs` — new: blob union logic
- `crates/photonic-core/src/ops/mod.rs` — re-export new modules
- `crates/photonic-render/src/renderer.rs` — brush geometry caching + render path
- `crates/photonic-gui/src/tools/mod.rs` — `PaintbrushTool`, `BlobBrushTool`
- `crates/photonic-gui/src/panels/mod.rs` — Brushes panel, Expand Appearance action
- `crates/photonic-mcp/src/server.rs` + `protocol.rs` — new MCP tools
- `crates/photonic-core/src/export.rs` — expand brushed strokes on SVG export

## Risks & Open Questions

- **Performance of arc-length parameterization**: Accurate placement along a Bézier path requires adaptive subdivision. Use `kurbo::ParamCurveArclen` from the existing `kurbo` dependency. Cache arc-length tables per `PathData`.
- **Calligraphic union cost**: Unioning many small ellipses via `geo::BooleanOps` per frame is expensive. Expand once on commit; render the expanded geometry.
- **Pressure input availability**: `winit` 0.30 has `DeviceEvent::Motion` but surface-level pressure is only available on some platforms. Default to no-pressure (fixed size); add pressure as an enhancement.
- **Art brush artwork storage**: Should artwork be a `PathData` embedded in `BrushDefinition`, or a reference to an existing document node? Embedded is simpler and self-contained; reference allows reuse but complicates deletion. Propose embedded for V1.
- **Pattern brush corner tiles**: Corner tile fitting (especially for acute angles) is algorithmically complex. Defer to auto-generated corners (scale the side tile) for the first pass; user-defined corners as an enhancement.

## Acceptance Criteria

- [ ] Each of the four brush types renders visually along an arbitrary `PathData` in the canvas and in the headless renderer.
- [ ] SVG export expands brushed strokes to outlined paths with no data loss.
- [ ] "Expand Appearance" command produces flat `PathNode`s that match the brush rendering.
- [ ] Blob Brush merges overlapping strokes into a single compound path.
- [ ] Brushes panel lists brushes, allows create/delete, and drag-to-apply.
- [ ] MCP `define_brush` / `apply_brush` / `expand_brush` tools are functional.
- [ ] Undo/redo covers all brush operations.

## Effort Estimate

**XL** — Four brush types, a new arc-length parameterization system, renderer integration with caching, two new GUI tools, a full panel, and MCP wiring. Each brush type is independently significant. Calligraphic + Art alone would be **L**; all four together plus Blob and Expand push this to XL.
