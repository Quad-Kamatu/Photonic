# Smart Guides & Snap-to-Object (#66) — Design Proposal

> Status: design scaffold (not an implementation).

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
