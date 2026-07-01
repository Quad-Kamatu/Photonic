# Simplify Path — live preview before Apply (#166)

> Status: **implemented**. Small, self-contained GUI-behavior change in the
> Simplify Path dialog. area:gui, priority:p2, type:ux.

## What this PR implements

All in `crates/photonic-gui/src/app/mod.rs` (no other crate touched):

- **`SimplifyDialog` state extended** with `orig_points: usize`,
  `preview: Option<PathData>`, and `cached_tol: f64` (a per-tolerance RDP
  cache). `orig_points` is captured from `count_points(&pn.path_data)` in the
  `PanelAction::OpenSimplifyDialog` handler; `cached_tol` starts as `NaN` so the
  first comparison always forces a build.
- **Live canvas overlay** in the `CentralPanel` draw closure (right after the
  Outline-Mode wireframe block). While the dialog is open and the node is a
  `SceneNodeKind::Path`, it rebuilds the cached preview only when
  `cached_tol != tolerance`, converts it with `bez_to_screen_points_xf(&preview
  .to_bez_path(), view, &node.transform)`, and paints it as a ~1.5 px accent
  (`rgb(110, 86, 207)`) `egui::Shape::line` plus 2 px anchor dots at each vertex.
  Non-destructive — pure egui painting, no document mutation.
- **`Points: N → M` readout** in `draw_simplify_dialog`, driven by `orig_points`
  and `count_points` on the cached preview. The dialog refreshes the same cache
  itself (same guard) so the readout is correct regardless of draw order.
- **Apply reuses the cached preview** (`preview.take()`) instead of re-running
  RDP, falling back to a fresh `simplify_path` call if no cache exists. Still
  records `Command::UpdateNode` (undoable). Cancel/close just drops the dialog —
  no doc mutation, so no revert and no render-cache invalidation.

Verification: `cargo build --release`, `cargo test -p photonic-gui`, and
`cargo check --workspace` all pass (pre-existing deprecation warnings only).

## Remaining work

None for this issue. The deferrals below are intentional scope boundaries, not
unfinished work:
- No change to the `simplify_path` algorithm or its sampling parameters.
- Preview is a wireframe overlay (Outline-Mode convention), not a live GPU
  re-render of fill/stroke.
- Simplify-only; no preview for other ops (boolean, erase, width).
- No MCP surface change.

## Summary

The Simplify Path dialog (`SimplifyDialog`, `photonic-gui`) currently applies
blind: the user drags the **Tolerance** value and only sees the reduced path
*after* pressing **Apply**. This makes the tolerance a guess-and-check —
apply, inspect, undo, re-open, repeat.

It should show a **live preview** of the simplified result overlaid on the
canvas, updating as the tolerance changes, *before* the user commits with
Apply. Add a numeric point-count readout (before → after) for extra feedback.

## Scope

### In
- Draw a non-destructive **preview overlay** of the simplified path on the
  canvas while the Simplify dialog is open, updating live as Tolerance changes.
- Show a **"Points: N → M"** readout in the dialog so the reduction is legible
  as a number, not just visually.
- Cache the simplified `PathData` per-tolerance so we do not re-run
  Ramer-Douglas-Peucker every frame (only on tolerance change / dialog open).
- Apply / Cancel behavior unchanged: Apply still commits via
  `Command::UpdateNode` (undoable); Cancel/close discards.

### Out / deferred
- No change to the `simplify_path` algorithm (`photonic-core`) or its
  polyline-sampling parameters.
- No preview for other ops (boolean, erase, width) — this is Simplify-only.
- No live GPU re-render of fill/stroke; the preview is a wireframe overlay
  (same convention Outline Mode uses), which is enough to judge tolerance.
- No MCP surface change; the MCP `simplify` path already returns the result.

## Approach

The dialog state and its handler already exist. Two touch points, both in
`photonic-gui`:

**1. `SimplifyDialog` state (`app/mod.rs`, struct at ~L226).** Add a small
preview cache so RDP does not run every frame:
```rust
struct SimplifyDialog {
    node_id: NodeId,
    node_name: String,
    tolerance: f64,
    orig_points: usize,          // count_points(&pn.path_data) at open
    preview: Option<PathData>,   // simplified result for `cached_tol`
    cached_tol: f64,             // tolerance `preview` was built for
}
```
Populate `orig_points` when the dialog is opened (`PanelAction::OpenSimplifyDialog`
handler, ~L7107), where the node is already in scope.

**2. Preview overlay (`app/mod.rs`, CentralPanel closure inside `draw`, near
the Outline-Mode path drawing ~L3480-3523).** `view: &CanvasView` and the
canvas `painter`/`rect` are in scope there. When `self.simplify_dialog` is
`Some` and its node is a `SceneNodeKind::Path`:
- If `cached_tol != tolerance` (or `preview` is `None`), recompute
  `photonic_core::ops::simplify::simplify_path(&pn.path_data, tolerance)` and
  store it in `preview` / `cached_tol`.
- Convert the cached preview via the existing
  `bez_to_screen_points_xf(&preview.to_bez_path(), view, &node.transform)`
  (`app/geometry.rs`) and paint it with a distinct accent stroke (e.g. the
  theme accent, ~1.5 px) over the artwork, plus small anchor dots at each
  vertex so the point reduction is visible. This mirrors how Outline Mode
  already paints path wireframes with `egui::Shape::line`.

Because the overlay reads the cached `preview`, it costs one RDP run per
tolerance change, not per frame.

**3. Dialog readout (`draw_simplify_dialog`, ~L12799).** Under the Tolerance
row, add `ui.label("Points: {orig} → {new}")` using `orig_points` and
`count_points(&preview)` (recomputing/using the same cache). Keeps the numeric
feedback in sync with the overlay.

**4. Apply / Cancel (unchanged logic).** Apply still runs `simplify_path` and
records `Command::UpdateNode` (can reuse the cached `preview` to avoid a
redundant recompute). Cancel/close just drops `simplify_dialog`; since the
overlay is non-destructive and the doc was never mutated, nothing to revert.

No undo-history or render-cache risk: the preview is a pure egui overlay, never
a document mutation, so it needs no invalidation and cannot corrupt state if
the dialog is dismissed.

Build: `cargo build --release` must pass after the edit.
