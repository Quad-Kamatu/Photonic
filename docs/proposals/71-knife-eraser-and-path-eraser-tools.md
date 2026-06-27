# Knife, Eraser, and Path-Eraser Tools (#71) — Design Proposal

> Status: design scaffold (not an implementation).

## Summary

The Scissors tool (`app.rs:2260`) cuts a path at a single point. This issue adds three destructive-edit tools that operate over regions or strokes: **Knife** (freehand line through filled shapes → two closed paths), **Eraser** (drag-to-erase any art using boolean subtraction), and **Path-Eraser** (drag along a selected path to delete a segment). All rely on the existing `ops::boolean` module (`crates/photonic-core/src/ops/boolean.rs`).

## Scope

**In**
- `Tool::Knife`: freehand polyline drawn across filled `SceneNodeKind::Path` nodes; on release, each intersected path is split into two or more closed paths; original node replaced by new nodes
- `Tool::Eraser`: drag a circular eraser head; on release, subtract the swept stroke area (outline of the drag path at eraser radius) from all path nodes it intersects; result is `boolean_op(original, swept_outline, BooleanOp::Subtract)`
- `Tool::PathEraser`: with a path node selected, drag along its length to mark a segment for deletion; the segment between the two nearest path points at stroke start/end is removed, leaving open sub-paths
- All three produce clean, editable `PathData` results
- Each operation is a single undoable `Command::Batch` (remove old node(s) + add new node(s))
- Knife and Eraser operate on all visible, unlocked path nodes (no pre-selection needed); PathEraser requires a selected path

**Out**
- Erasing raster/image content
- Erasing text or group nodes (only `SceneNodeKind::Path` nodes for M1)
- Soft/feathered eraser edges
- Knife cutting open paths (first pass: closed/filled paths only)

## Proposed Approach

### Knife

1. **`Tool::Knife` state in `App`**: `knife_points: Vec<(f64, f64)>` collects canvas-space points as the user drags. Render as a dashed polyline on canvas during drag.

2. **On release**: for each path node that the knife polyline intersects:
   a. Build a `PathData` for the knife stroke (thin zero-width path).
   b. Use `ops::boolean::boolean_op(shape, knife_stroke_area, BooleanOp::Divide)` — note `BooleanOp::Divide` already exists in `boolean.rs` and splits an area by a cutting line.
   c. Each resulting sub-polygon becomes a new `SceneNode` inheriting the original's style and transform.
   d. Bundle as `Command::Batch([RemoveNode(original), AddNode(result_a), AddNode(result_b), ...])`.

3. **Intersection test**: before running boolean, quick-check if the knife polyline's bounding box overlaps the node's bounding box.

### Eraser

1. **`Tool::Eraser` state**: `eraser_radius: f64` (settable in tool options, default 10px canvas-space), `eraser_path: Vec<(f64, f64)>` collecting drag points.

2. **On release**: 
   a. Construct the swept eraser outline: `PathData::stroke_outline(&eraser_path, eraser_radius)` — create this helper in `photonic-core/src/path.rs`, building a capsule-rounded stroke from the polyline using `ops::stroke_outline` (the module already exists at `crates/photonic-core/src/ops/stroke_outline.rs`).
   b. For each intersecting path node: `boolean_op(node_path, eraser_outline, BooleanOp::Subtract)`.
   c. If result is empty (fully erased), emit `Command::RemoveNode`. Otherwise emit `Command::Batch([UpdateNode(old, new)])` with the subtracted path.

3. **Cursor preview**: render a circle of radius `eraser_radius` in screen space following the pointer.

### Path-Eraser

1. **`Tool::PathEraser` state**: `path_eraser_start: Option<f64>` and `path_eraser_end: Option<f64>` — normalized arc-length positions [0,1] on the selected path.

2. **On drag**: find the nearest arc-length t on the selected path for the pointer position (same sample-point approach as Scissors). Drag from t_start to t_end defines the segment to erase.

3. **On release**: trim the selected path's `PathData` to remove the arc from t_start to t_end. If the path was closed, the result is one open path. If open, the result may be two sub-paths. Implement `PathData::remove_segment(t_start, t_end) -> Vec<PathData>` in `crates/photonic-core/src/path.rs`.

## Affected Modules

- `crates/photonic-gui/src/tools/mod.rs` — add `Tool::Knife`, `Tool::Eraser`, `Tool::PathEraser` variants; update label/icon/toolgroup
- `crates/photonic-gui/src/app.rs` — `App`: state fields for each tool; dispatch in the main event loop; canvas preview rendering; `app.rs:1217` tool group (add to "Path Editing" group)
- `crates/photonic-core/src/ops/boolean.rs` — `BooleanOp::Divide` already exists; verify it handles the knife use-case correctly
- `crates/photonic-core/src/ops/stroke_outline.rs` — reuse for eraser swept-stroke construction; may need a polyline-input overload
- `crates/photonic-core/src/path.rs` — new: `PathData::remove_segment(t_start, t_end) -> Vec<PathData>` for PathEraser
- `crates/photonic-core/src/history.rs` — `Command::Batch` / `Command::RemoveNode` / `Command::AddNode` (all existing)

## Risks & Open Questions

- **`BooleanOp::Divide` robustness**: the `geo` crate's boolean ops can fail on self-intersecting or degenerate paths, returning `Err(String)`. All three tools must handle the error case gracefully (log a warning, leave the original node unchanged).
- **Stroke-outlined paths**: the boolean approach treats a stroked path as its filled outline. A path with only a stroke and no fill may produce unexpected results with `Subtract` — consider expanding the stroke to a filled shape first using `stroke_outline`.
- **Knife on open paths**: `BooleanOp::Divide` is defined for filled polygons. Cutting an open path requires a different approach (find intersection point, split at t). Limit M1 to closed, filled paths and document this restriction.
- **Performance**: each tool commits on `drag_released` — no per-frame boolean ops. Should be acceptable.
- **Eraser radius in canvas vs screen units**: `eraser_radius` should be in canvas units so it scales correctly with zoom. The cursor circle preview must convert to screen units for rendering.

## Acceptance Criteria

- [ ] `Tool::Knife` freehand cut through a closed filled path produces two separate, editable closed path nodes
- [ ] `Tool::Eraser` drag removes a region from path nodes, producing clean anchors at cut edges
- [ ] `Tool::PathEraser` erases a segment of a selected path; result is a clean open or split path
- [ ] Each operation is a single undo step
- [ ] Failed boolean ops (degenerate geometry) leave the original node unchanged with a console warning
- [ ] All three tools appear in the "Path Editing" group in the toolbox

## Effort Estimate

**L** — three distinct interaction models; PathEraser requires a new `PathData::remove_segment` primitive; boolean error handling needs care across all three tools.
