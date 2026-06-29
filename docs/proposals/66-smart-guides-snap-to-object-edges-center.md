# Smart Guides & Snap-to-Object (#66)

> Status: **implemented (MVP)** — object-aware snapping is live on the Select-tool move drag, with dashed smart-guide overlays and pixel-distance labels. See "What this PR implements" / "Remaining work" below; the original design proposal follows unchanged.

## What this PR implements

- **`crates/photonic-gui/src/snap.rs`** (new): the snap engine.
  - `SnapAxis { Vertical, Horizontal }`, `SnapCandidate { node_id, axis, value, perp_min, perp_max }`, `ActiveGuide { axis, coord, target_node, distance }`, `SnapResult { corrected: (f64, f64), active: Vec<ActiveGuide> }`.
  - `collect_snap_candidates(doc, exclude) -> Vec<SnapCandidate>` — emits left/center/right (vertical) and top/center/bottom (horizontal) alignment lines for every visible, non-locked node (via `Document::nodes_in_draw_order`, which already flattens groups and skips hidden layers/nodes), excluding the dragged ids. Bounds are canvas-space (`local_bounds` projected through `node.transform`).
  - `resolve_snap(moving_bbox, candidates, tolerance) -> SnapResult` — per axis, picks the candidate closest to one of the object's three edges (min/center/max) within tolerance; returns the `(dx, dy)` correction plus up to two active guides. Each guide's `distance` is the genuine object-to-object gap along the perpendicular axis (for the label).
  - 6 unit tests covering edge snap, center-beats-edge, both-axes, closest-candidate selection, out-of-tolerance no-op, and the distance-label gap.
- **`AppPreferences`** (`preferences.rs`): `snap_to_objects: bool` (default `true`), `snap_tolerance_px: f32` (default `6.0`), `snap_show_guides: bool` (default `true`), all `#[serde(default = ...)]` for forward-compatible loading. Exposed in the View settings popover (toggle + tolerance slider).
- **Move-drag integration** (`app/tool_handlers.rs`, `handle_select_tool`): after grid snap, when `snap_to_objects` is on and Shift (axis-lock) is not held, the tentative selection bbox (original bbox + drag delta) is resolved against the candidates; the correction is **added to the applied delta**, so the dragged object actually moves to alignment. Tolerance is converted from screen px to canvas units via `view.zoom`. Object snap is additive with grid snap. New `move_snap_bbox` field caches the start bbox; `last_snap_result` is set each snapping frame and cleared on release.
- **Overlay rendering** (`app/mod.rs`, after the existing guide overlay): when `snap_show_guides` is on and a snap is active, dashed magenta full-canvas lines are drawn at each alignment, with a `{n}px` distance label. Guides vanish on pointer release (state cleared in the `drag_stopped` block).

Acceptance criteria mapping: dragging a node snaps to a nearby node's edge/center with a dashed guide ✓; guides disappear on release ✓; live pixel-distance labels ✓; `snap_to_objects` toggle coexisting with grid snap ✓; O(n) per-frame candidate scan, no spatial index needed at typical scene sizes ✓.

## Remaining work

- **Resize and shape-creation drags**: deferred. Snapping is wired into the move drag only. The resize (`self.resizing`) and `build_shape` create paths still use grid snap. The `snap.rs` API is drag-path-agnostic, so adding them is a follow-up that reuses `collect_snap_candidates` / `resolve_snap`.
- **Equal-spacing detection** (equidistant-between-two-neighbors snap + bracket labels): deferred. `resolve_snap` does single edge/center alignment per axis; distribution snapping is not implemented.
- **Snap to path anchor points / artboard edges / margins**: out of scope (M3 in the original design).
- **Group bounds**: `local_bounds()` returns `None` for group nodes, so a group's own combined bbox is not a snap target (its visible leaf children still are, via draw-order flattening). Text bounds use the core approximation, not glyphon layout.
- **Guide color theming**: fixed magenta rather than an accent-derived/preference color.

---

# Smart Guides & Snap-to-Object (#66) — Design Proposal

## Summary

The existing snap system (`App::snap()` at `app.rs:~10937`) only rounds coordinates to the pixel grid. This issue adds object-aware snapping: during move/resize/create drags, Photonic detects when the dragged object's edges or center align with those of nearby objects, snaps to them, and draws temporary guide lines with live distance labels.

## Scope

**In**
- Snap targets: edges (left/right/top/bottom) and centers (horizontal/vertical) of all visible, non-locked nodes in the active layer
- Equal-spacing hints: detect when the dragged object is equidistant between two neighbors and snap to that gap
- Visual guide overlays: colored dashed lines extending across the canvas at each active snap alignment
- Live distance labels: pixel distance from dragged object to snap target, rendered next to each guide
- Snap tolerance configurable in `AppPreferences`
- Snap respects the existing `snap_to_grid` toggle (grid snap and object snap are additive)
- Guide lines disappear on pointer release

**Out**
- Snap to path anchor points or path intersections (M3)
- Snap to artboard edges / margins (M3)
- Persistent measurement annotations (Issue #70 rulers)
- Equal-spacing snap for more than one axis simultaneously (first pass: one axis at a time)

## Proposed Approach

1. **Snap candidate collection**: add `fn collect_snap_candidates(doc: &Document, exclude: &[NodeId]) -> Vec<SnapCandidate>` in a new file `crates/photonic-gui/src/snap.rs`. `SnapCandidate` holds the node id, a `SnapAxis` (H/V), and the canvas-space coordinate value (e.g. left edge x = 120.0). Runs once per drag frame over all non-excluded visible nodes; cheap for typical scene sizes.

2. **Snap resolution**: `fn resolve_snap(cx: f64, cy: f64, bbox: BBox, candidates: &[SnapCandidate], tolerance: f64) -> SnapResult` returns the closest alignment(s) within tolerance and the corrected (cx, cy). Returns up to 2 active snaps (one per axis).

3. **Integration into drag paths**: the three drag paths that need it are move (`self.moving` block), resize (`self.resizing` block), and shape creation drag (`build_shape` region). Each currently calls `self.snap(v)` for grid snap. Replace that with a combined call that first checks object snap, then falls back to grid snap.

4. **`AppPreferences` additions** (`crates/photonic-gui/src/preferences.rs`): `snap_to_objects: bool` (default true), `snap_tolerance_px: f32` (default 6.0), `snap_show_guides: bool` (default true).

5. **Guide rendering**: after the main node paint pass, if `snap_result.active` is non-empty, draw dashed `Stroke` lines across the full canvas rect and a small text label using egui `Painter`. Store `last_snap_result: SnapResult` in `App` so the paint pass can read it without recomputing.

6. **Equal-spacing detection**: after edge/center snapping, if two snap candidates straddle the dragged object on the same axis, check if `gap_a ≈ gap_b`; if so, add a secondary snap nudge and render bracket-style distance labels between each pair.

## Affected Modules

- `crates/photonic-gui/src/snap.rs` — new file: `SnapCandidate`, `SnapResult`, `collect_snap_candidates`, `resolve_snap`
- `crates/photonic-gui/src/app.rs` — `App` struct gains `last_snap_result`, drag blocks call `resolve_snap`, paint block renders guide overlays; `AppPreferences` fields added
- `crates/photonic-gui/src/preferences.rs` — `AppPreferences`: `snap_to_objects`, `snap_tolerance_px`, `snap_show_guides`
- `crates/photonic-core/src/document.rs` — no changes; `Guide` struct already exists for guide-list snap target (reuse bounding-box logic from `Document`)

## Risks & Open Questions

- **Performance at large scene sizes**: `collect_snap_candidates` is O(n) over all nodes. For n > ~500 a spatial index (quadtree) may be needed. Start without one; profile before adding complexity.
- **Coordinate system**: snap candidates must be in canvas space, not screen space. All bounding-box queries must go through `node.transform` — confirm `photonic-render` exposes a canvas-space bounds call, or recompute from `PathData` + transform directly.
- **Existing `snap()` function** (`app.rs:10937`): currently private to `App` with a single grid-snap behavior. Replacing it inline at all call sites is safer than changing its signature, since it is called in several branches.
- **Guide color theming**: guides should contrast against both dark and light canvas backgrounds — make the color a preference or derive from the accent color.

## Acceptance Criteria

- [ ] Dragging a node snaps to the edge or center of a nearby node with a dashed guide line appearing
- [ ] Guide lines disappear immediately on pointer release
- [ ] Live pixel-distance labels display next to each active guide
- [ ] Equal-spacing snap fires when the dragged object is equidistant between two neighbors
- [ ] `snap_to_objects` can be toggled in preferences; grid snap and object snap coexist
- [ ] No perceptible lag on a scene with ~200 nodes

## Effort Estimate

**M** — the snap math is straightforward; the main investment is integrating into all three drag paths and getting the overlay rendering pixel-accurate.
