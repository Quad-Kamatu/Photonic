# Knife, Eraser, and Path-Eraser Tools (#71)

> Status: **implemented (M1)** — Knife and vector Eraser are shipped and wired.
> Path-Eraser is deferred (see Remaining work).

## What this PR implements

Two genuine destructive-edit tools that cut real geometry via the existing
`photonic_core::ops` boolean and stroke-outline operations:

- **`Tool::Eraser` (vector eraser)** — drag a circular eraser head across the
  canvas. A screen-space circle preview (radius scales with zoom) follows the
  pointer on hover and during the drag, plus a translucent swept-area ribbon
  while dragging. On release, the drag polyline is outlined at the eraser radius
  (`outline_stroke` with a `2*radius`-wide round-cap/round-join stroke) to build
  the swept area, which is then `boolean_op(node_path, swept_outline, Subtract)`
  against **every visible, unlocked `SceneNodeKind::Path` node** whose
  canvas-space bbox overlaps the sweep. The subtraction is done in each node's
  **local space** (the swept outline is transformed by the node's inverse
  transform, so rotated/scaled/translated art erases correctly). A fully-erased
  node becomes `Command::RemoveNode`; a partially-erased one becomes
  `Command::UpdateNode` with the subtracted path. All edits across all nodes are
  bundled into a **single `Command::Batch`** (one undo step). Failed boolean ops
  on degenerate geometry are logged and leave that node unchanged.

- **`Tool::Knife` (freehand slice)** — drag a freehand line; a red cut-line
  preview tracks the drag (including a live segment to the cursor). On release,
  for each **filled**, visible, unlocked path the line crosses, the cut line is
  outlined into a thin (~2px-on-screen, zoom-independent) butt-capped sliver and
  `boolean_op(shape, sliver, Subtract)` splits the filled area into ≥2 disjoint
  polygons. The result is split per-subpath into **separate editable path
  nodes** (each inheriting the original's style, transform, opacity, blend mode,
  effects, and layer). Original node removed + new face nodes added, all in one
  `Command::Batch`; the new faces become the selection. Cuts that don't fully
  cross a shape (fewer than 2 resulting faces) leave it untouched.

Both tools live in a new handler module **`crates/photonic-gui/src/app/erase_tools.rs`**,
with the swept-outline construction (`build_stroke_area`), the canvas→local
cutter transform, the bbox-overlap reject, and the subpath splitter unit-tested
(`eraser_sweep_outline_spans_radius`, `single_point_eraser_makes_a_disc`,
`knife_subtract_splits_a_square_into_two_faces`).

The eraser radius is a tool option (default 10px canvas), surfaced in the
properties panel under **Eraser Options**. Both tools appear in the **Path
Editing** toolbox group and the global command search.

### Files changed / created

- `crates/photonic-gui/src/app/erase_tools.rs` — **new**: `handle_eraser_tool`,
  `handle_knife_tool`, `apply_eraser`, `apply_knife`, `build_stroke_area`,
  `transform_path`, `path_canvas_bbox`, `rects_overlap`, `split_subpaths`, tests.
- `crates/photonic-gui/src/tools/mod.rs` — `Tool::Knife` + `Tool::Eraser`
  variants; `label`/`icon`/`description`/`is_shape_creator` arms.
- `crates/photonic-gui/src/app/mod.rs` — `mod erase_tools`; state fields
  (`eraser_points`, `eraser_radius`, `knife_points`) + `Default`; dispatch in the
  update loop; "Path Editing" toolbar group; `eraser_radius` threaded into the
  properties panel.
- `crates/photonic-gui/src/panels/mod.rs` — `eraser_radius` param; "Eraser
  Options" radius slider + "Knife Options" hint; path-editing tool picker.
- `crates/photonic-gui/src/global_search.rs` — search catalog + keywords.

## Remaining work

- **`Tool::PathEraser`** is deferred. It requires a new
  `PathData::remove_segment(t_start, t_end) -> Vec<PathData>` arc-length
  primitive in `photonic-core` and a different interaction model (drag along a
  *selected* open/closed path to trim a segment). None of that is faked here —
  the variant is intentionally not added.
- Eraser/Knife operate on `SceneNodeKind::Path` only — raster, text, and group
  nodes are skipped (out of scope for M1, per the design).
- Knife requires a **filled** path and only acts when the cut fully crosses the
  shape (≥2 resulting faces); open/stroke-only paths are left unchanged.
- Stroke-only erasing: the eraser subtracts from the path's *fill* geometry, not
  its rendered stroke outline. Erasing a stroke-only path has no geometric area
  to subtract from and is left unchanged.

---

# Design Proposal (original)

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

- [x] `Tool::Knife` freehand cut through a closed filled path produces two separate, editable closed path nodes
- [x] `Tool::Eraser` drag removes a region from path nodes, producing clean anchors at cut edges
- [ ] `Tool::PathEraser` erases a segment of a selected path; result is a clean open or split path — *deferred*
- [x] Each operation is a single undo step
- [x] Failed boolean ops (degenerate geometry) leave the original node unchanged with a console warning
- [x] All three tools appear in the "Path Editing" group in the toolbox — *Knife + Eraser shipped; PathEraser deferred*

## Effort Estimate

**L** — three distinct interaction models; PathEraser requires a new `PathData::remove_segment` primitive; boolean error handling needs care across all three tools.
