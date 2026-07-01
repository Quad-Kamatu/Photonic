# 189 — Merge Vertices by Distance (Weld)

## Status: Implemented

### What this PR implements
- **Core** — `crates/photonic-core/src/ops/merge.rs`:
  `merge_vertices_by_distance(path: &PathData, threshold: f64) -> PathData`,
  registered in `ops/mod.rs`. Subpath-walk over `path.to_bez_path()` (split on
  `MoveTo`/`ClosePath`), Bézier segments flattened to on-curve anchor endpoints,
  greedy running-centroid clustering welding anchors within `threshold`,
  wrap-around weld of last-into-first for closed subpaths, degenerate
  zero-length segment removal, rebuild via `BezPath` → `PathData::from_bez_path`.
  `threshold <= 0` returns `path.clone()`. 5 unit tests cover threshold-zero,
  near-coincident welding, far-anchor preservation, closed wrap-around, and
  degenerate-subpath drop — all passing.
- **GUI** — `crates/photonic-gui/src/app/mod.rs`: `MergeVerticesDialog` struct,
  `draw_merge_vertices_dialog` (distance slider + live `Points: N → M` readout
  via `simplify::count_points`, Cancel/Apply committing one `Command::UpdateNode`
  step reusing the per-threshold cached preview), canvas live-preview overlay
  (accent wireframe + anchor dots, cached per threshold) beside the Simplify
  overlay, and the `OpenMergeVerticesDialog` `PanelAction` handler.
- **Wiring** — `panels/mod.rs`: `PanelAction::OpenMergeVerticesDialog { node_id }`
  + `From<WheelAction>` arm. `radial_wheel.rs`: `WheelAction::MergeVertices(NodeId)`
  + a "Merge Verts" `RadialMenuItem` in the Path category.

Verified: `cargo build --release`, `cargo test -p photonic-core` (312+ passing),
`cargo test -p photonic-gui` (all passing), `cargo check --workspace` — all green.

### Remaining work (deferred, unchanged from original scope)
- **Curve preservation** — weld result is emitted as polyline segments; Bézier
  handles are not reconstructed (same behavior as `simplify_path`).
- **Sub-selection welding** (#181 Direct Select) — merge acts on the whole path;
  restricting to a multi-vertex sub-selection is deferred.
- **MCP tool** — no `photonic-mcp` surface this pass (issue scopes Core + GUI only;
  no MCP docs regeneration needed).

## Summary
Add a **Merge Vertices by Distance** (weld) operation for path-edit mode. It collapses
anchors that are spatially near/coincident into a single anchor, driven by a **distance
threshold slider** with a **live vertex-count readout** and **on-canvas live preview**
(revert on Cancel, one history step on Apply). This is cleanup tooling for the aftermath
of boolean ops, imports, or hand-drawing — distinct in intent and algorithm from Simplify
Path (#166), which curve-fits to preserve shape.

The feature is a direct structural clone of the shipped Simplify Path dialog (#166): one new
core primitive next to `simplify.rs`, plus a GUI dialog + canvas overlay that mirror the
existing `SimplifyDialog` flow verbatim.

## Scope

### In
- **Core:** `photonic-core/src/ops/merge.rs` — new module with
  `merge_vertices_by_distance(path: &PathData, threshold: f64) -> PathData`. Registered in
  `ops/mod.rs`. Reuse the existing `count_points` from `simplify.rs` for the readout (no new
  count fn needed).
- **GUI:** new `MergeVerticesDialog` struct + `draw_merge_vertices_dialog` mirroring
  `SimplifyDialog` / `draw_simplify_dialog`; a `MergeVertices` canvas preview overlay next to
  the Simplify overlay; `PanelAction::OpenMergeVerticesDialog { node_id }`; radial-wheel
  `WheelAction::MergeVertices(NodeId)` under the "Path" category; wiring in `panels/mod.rs`
  `From<WheelAction>`.
- Live preview: slider (threshold) drives a per-threshold cached preview; Apply commits one
  `Command::UpdateNode` step; Cancel/close discards.

### Out (deferred)
- **Curve preservation** — like `simplify_path`, the weld result is emitted as polyline
  segments (Bézier handles are not reconstructed). Curve-aware welding is out of scope.
- **Sub-selection welding** (#181 Direct Select) — merge acts on the whole path this pass;
  restricting to a multi-vertex sub-selection is deferred.
- **MCP tool** — issue names Core + GUI only; no `photonic-mcp` surface this pass.

## Approach

### Core algorithm (`merge.rs`)
Mirror `simplify_path`'s subpath-walk structure over `path.to_bez_path()`:
1. Split into subpaths on `MoveTo` / `ClosePath`; flatten Bézier segments to their on-curve
   anchor endpoints (curves already collapse to lines, consistent with `simplify.rs`).
2. Per subpath, greedily weld: keep a running cluster; while the next anchor lies within
   `threshold` of the running cluster centroid, absorb it and update the centroid (running
   mean, akin to `PathData::average_anchor_points` at `core/src/path.rs:961`); otherwise emit
   the centroid and start a new cluster.
3. For closed subpaths, weld the final cluster into the first if within `threshold`
   (wrap-around), then drop degenerate zero-length segments.
4. Rebuild via `BezPath` `move_to`/`line_to`(/`close_path`) → `PathData::from_bez_path`.
   Guard: subpaths that would collapse below 2 points are dropped or left as a single point.

`threshold <= 0` returns `path.clone()`.

### GUI (`app/mod.rs`)
Copy the Simplify wiring 1:1, renaming tolerance→threshold:
- `MergeVerticesDialog { node_id, node_name, threshold, orig_points, preview, cached_thr }`
  (cf. `SimplifyDialog`, `app/mod.rs:226`).
- `draw_merge_vertices_dialog` (cf. `draw_simplify_dialog`, `app/mod.rs:12990`): slider +
  `Points: {orig} → {new}` readout via `simplify::count_points`, Cancel/Apply with cached
  preview reuse and single `UpdateNode` history step.
- Canvas overlay next to the Simplify overlay (`app/mod.rs:3641`) painting the welded preview
  wireframe + anchor dots per-threshold-cached.
- `PanelAction::OpenMergeVerticesDialog` handler (cf. `app/mod.rs:7259`) captures
  `orig_points`.
- `panels/mod.rs`: enum variant `OpenMergeVerticesDialog { node_id }` (cf. line 96) +
  `From<WheelAction>` arm (cf. line 751).
- `radial_wheel.rs`: `WheelAction::MergeVertices(NodeId)` (cf. line 78) + a "Merge Verts"
  `RadialMenuItem` in the Path category (cf. line 234).

### Build / verify
`cargo build --release`; open the wheel on a path node → Merge Verts, drag the slider, confirm
the vertex-count readout updates, preview repaints, Cancel reverts, Apply produces one undoable
step.
