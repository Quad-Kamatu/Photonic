# Inspector Point type Smooth must synthesize handles on straight corners (#188)

> Status: **implemented.** `bez_convert_anchors`
> (`crates/photonic-gui/src/app/geometry.rs`) was rewritten so the inspector's
> **Point type → Smooth** button now synthesizes tangent handles on straight
> (`LineTo`) corners, including the closed-subpath seam, instead of bailing.
> **Corner** stays a correct no-op on already-straight vertices. GUI dispatch
> and panel wiring were unchanged, as planned.

## What this PR implements

- **Decomposition-based rewrite** (the plan's recommended path). The function
  now decomposes each subpath into `(el_idx, point, in_handle: Option, out_handle:
  Option, arriving_element)` anchor records, applies the conversion on that list,
  and re-emits `MoveTo`/`LineTo`/`CurveTo`/`ClosePath`. This removes all
  `ClosePath`/seam index bookkeeping — synthesizing a handle on the `MoveTo`
  start or the last pre-`ClosePath` vertex simply materializes the implicit
  closing edge as an explicit `CurveTo` on re-emit.
- **Straight-corner synthesis** `(None, None)`: Catmull-Rom tangent
  `t = normalize(B - A)` from the seam-aware neighbours `A`/`B`, with
  `len_in = |P-A|/3`, `len_out = |B-P|/3`; the adjoining `LineTo` edges become
  `CurveTo` with the far control retracted to the neighbour so it stays straight.
- **One-sided cases** `(Some, None)` / `(None, Some)`: the existing handle is
  reflected collinearly onto the empty side (length = 1/3 of that edge). These
  previously fell into the bail arm.
- **`(Some, Some)`** legacy averaging behaviour is preserved byte-for-byte
  (direction averaged, per-handle lengths kept) — covered by a regression test.
- **Corner** retraction is reproduced exactly by the re-emit (a retracted side
  becomes `CurveTo(anchor, …)` when the neighbour is curved, or a `LineTo` when
  both sides are flat).
- **Tests** in `geometry.rs` (`convert_anchor_tests`): straight-corner synthesis,
  seam-corner materialization, corner no-op regression, smooth→corner roundtrip,
  and `(Some,Some)` averaging preservation. All pass; `cargo build --release`,
  `cargo test -p photonic-gui` (48 lib tests), and `cargo check --workspace`
  are green.

## Remaining work / deferred

- **Open-subpath endpoints** with no neighbour on one side are intentionally left
  unchanged (no well-defined tangent) — same as the plan.
- **`QuadTo` segments** are preserved verbatim and never smoothed across (the
  previous implementation ignored them too). Smoothing an anchor whose adjacent
  edge is a quadratic is out of scope.
- Everything under the original "Out (deferred)" list below (whole-path
  `convert_to_smooth`, the right-click point-type menu / on-canvas curvature
  handles #187, corner fillets #179/#165, and angle/length UI for synthesized
  handles) remains out of scope for this PR.

## Summary

`bez_convert_anchors` (`crates/photonic-gui/src/app/geometry.rs:868`) is the
per-anchor converter behind `PanelAction::ConvertAnchorType`
(`app/mod.rs:11208`), which the two inspector buttons in
`panels/mod.rs:1373/1384` dispatch.

- **Corner** (`geometry.rs:884-896`): only rewrites a segment already stored as
  `PathEl::CurveTo`. On a `LineTo` corner both `in_pt`/`out_pt` are `None`, so it
  correctly does nothing — a straight corner is *already* a corner. No behaviour
  change needed here (it already retracts handles when the point is curved).
- **Smooth** (`geometry.rs:901-932`): the `match (in_pt, out_pt)` only acts when
  at least one side is already curved. On a straight corner both are `None`, so it
  hits the `_ =>` arm (`geometry.rs:928-931`) that explicitly leaves the anchor
  untouched. Result: nothing happens on rectangles/polygons/most paths.

The comment there claims "whole-path To Smooth handles this", but
`PathData::convert_to_smooth` (`crates/photonic-core/src/path.rs:466`) only
smooths *junctions between two existing `CurveTo` segments* — it also leaves a
pure-`LineTo` polygon unchanged. So no existing code actually synthesizes handles
on a straight corner; this proposal adds that.

## Scope

**In:**
- Make **Smooth** synthesize tangent handles on a straight corner: compute a
  tangent from the two neighbouring anchors (auto-smooth / Catmull-Rom style) and
  convert the adjoining `LineTo` segments into `CurveTo` with control points along
  that tangent.
- Make it work for **all** vertices of a closed shape, including the seam
  (`MoveTo` start anchor and the anchor whose outgoing edge is the implicit
  `ClosePath` edge), not just the interior ones.
- Keep the existing `(Some, Some)` collinear-averaging path unchanged, and extend
  the one-sided cases `(Some, None)` / `(None, Some)` to reflect the existing
  handle to the empty side (currently they fall into the bail arm too).
- Unit tests in `geometry.rs`.

**Out (deferred):**
- Any change to `PathData::convert_to_smooth` / whole-path smoothing (path.rs).
- The right-click point-type menu / on-canvas curvature-handle rendering (#187).
- Corner-rounding fillets (#179, #165) — different feature, `fillet_corners`.
- Angle/length UI for the synthesized handles; we pick a deterministic default
  (1/3 of each adjacent edge length).

## Approach

Rewrite only the `smooth` branch of `bez_convert_anchors`. For each selected
anchor `i` with anchor point `P`:

1. **Resolve neighbours seam-aware.** Determine the subpath bounds and `closed`
   flag the same way `logical_handles` (`geometry.rs:466`) does. Find the previous
   anchor point `A` and next anchor point `B` within the subpath, wrapping across
   the seam when the subpath is closed. Endpoints of an *open* subpath (no prev or
   no next) have no well-defined tangent → leave unchanged.

2. **Compute the tangent.** `t = normalize(B - A)`. If `|B - A| < eps`, skip.
   Handle lengths: `len_in = |P - A| / 3`, `len_out = |B - P| / 3` (Catmull-Rom
   default; keeps the synthesized curve close to the original straight edges).
   New handles: `new_in = P - t*len_in`, `new_out = P + t*len_out`.

3. **Write the handles into the adjacent segments**, handling four segment
   shapes on each side:
   - Incoming segment (ends at `P`): if `LineTo(P)` → `CurveTo(A, new_in, P)`
     (far handle `A` retracted so the neighbour stays straight); if already
     `CurveTo(c1, _, P)` → replace `c2` with `new_in`.
   - Outgoing segment (leaves `P`): if `LineTo(B)` → `CurveTo(new_out, B, B)`; if
     already `CurveTo(_, c2, B)` → replace `c1` with `new_out`.
   - **Seam materialization.** When `closed` and the incoming/outgoing "segment"
     is the implicit `ClosePath` edge (i.e. `P` is the `MoveTo` start, or `P` is
     the last geometric anchor and `els[i+1]` is `ClosePath`), the closing edge
     has no explicit element. Materialize it: insert an explicit
     `CurveTo(new_out, B, B)` (or the incoming equivalent) immediately before the
     `ClosePath`, leaving `ClosePath` as the now zero-length seal. Because this
     changes element indices, process a snapshot of `els` and adjust the running
     index offset per subpath, or (cleaner) build the output path by
     decomposing each subpath into `(anchor, in_handle: Option<Point>,
     out_handle: Option<Point>, closed)` records, apply the synthesis on that
     record list, then re-emit `MoveTo`/`LineTo`/`CurveTo`/`ClosePath`. The
     decomposed form removes all the seam/`ClosePath` special-casing and is the
     recommended implementation — the element-splice version is acceptable only if
     it demonstrably preserves the existing `(Some,Some)` tests.

4. **Corner branch:** unchanged (already correct — retracts existing handles,
   no-op on already-straight corners).

The GUI dispatch (`app/mod.rs:11208`) and panel wiring (`panels/mod.rs:1373`)
need no changes — they already round-trip through `PathData::from_bez_path`.

## Tests (in `geometry.rs`)

- `smooth_straight_corner_synthesizes_handles`: a closed rectangle, select one
  interior vertex, Smooth → both adjacent segments become `CurveTo`, and the two
  new handles are collinear through the anchor (dot of the two tangent directions
  ≈ -1).
- `smooth_seam_corner_synthesizes_handles`: same but selecting the `MoveTo`
  vertex and the last (pre-`ClosePath`) vertex — asserts the closing edge is
  materialized and both corners smooth.
- `corner_is_noop_on_straight`: Corner on a straight rectangle vertex leaves the
  path byte-identical (regression guard for the deliberate no-op).
- `smooth_then_corner_roundtrips`: Smooth a corner then Corner → the anchor's
  adjacent segments are straight lines again (handles retracted).
- Keep/verify any existing `(Some,Some)` averaging behaviour unchanged.

## House rule

After the edit: `cargo build --release` must succeed; GPU headless render works
for any manual confirmation. Joseph launches/verifies the GUI himself.

## Fix round 1 (post-adversarial-review)

Two findings from the adversarial gate were addressed in the working tree; no
deferrals remain — both were in-scope correctness issues, not new scope.

1. **[major] Stale point selection on compound-path seam materialization.**
   The decomposed implementation can *grow* a subpath's element count (it
   materializes the implicit seam as an explicit `CurveTo`). For a compound
   path, growing an earlier subpath shifts every later subpath's element index,
   so the caller's `self.point_selected` (element-index based) would point at
   the wrong anchors. Unlike its siblings `RoundSelectedCorners` and
   `DeleteAnchors`, `PanelAction::ConvertAnchorType` did not clear the stale
   selection. **Fix (`app/mod.rs`):** compare the element count before/after
   `bez_convert_anchors` and `self.point_selected.clear()` only when it changed
   — preserving the common single-subpath selection while dropping it exactly
   when seam materialization shifted indices. This supersedes the earlier claim
   in this doc that `app/mod.rs` "needs no changes".
   Regression test: `seam_materialization_shifts_compound_indices` (asserts the
   count grows, i.e. the caller's change-detection signal is reliable).

2. **[blocker] Seam smoothing was not idempotent (degenerate zero-length
   `CurveTo`).** The feature's own seam materialization emits the closing edge
   as an explicit `CurveTo` whose endpoint equals the `MoveTo` start — the
   "start listed twice" explicit-close form. On a *second* smooth of the same
   seam anchor, the decomposition treated that duplicate start as a separate
   anchor, so the closed-wrap neighbour lookup picked the coincident duplicate
   (length-0 handle) and appended a degenerate zero-length cubic. **Fix
   (`geometry.rs`):** after decomposition, reunify the explicit-close seam the
   way `logical_handles` does — when a closed subpath's last anchor coincides
   with `anchors[0]`, drop the trailing duplicate and fold its incoming handle
   onto the start anchor, so the seam neighbours resolve to the true geometry.
   Additionally guard the re-emit's closing block to skip a zero-length closing
   cubic (endpoints + both controls all coincident). Regression test:
   `smooth_seam_is_idempotent` (asserts no degenerate `CurveTo`, `is_smooth_anchor`
   stays true, and the element count is stable across re-application).

Verification: `cargo build --release` clean; `cargo test -p photonic-gui` →
50 pass (2 new); the changed region has no new clippy warnings.

## Fix round 2 (post-adversarial-review)

One finding from the round-2 adversarial gate was addressed in the working
tree; no deferrals remain.

1. **[major] Round-1 count-change guard is unsound for compound paths.** The
   round-1 fix cleared `self.point_selected` only when the total element count
   changed across `bez_convert_anchors`, claiming this both detected
   seam-materialization index shifts and preserved single-subpath selections.
   But `bez_convert_anchors` unconditionally reunifies every closed subpath
   during decompose, so a subpath arriving in explicit-close `LINE` form
   (`M … L start Z`) re-emits in implicit form (`M … Z`), *shrinking* by one
   element. In a compound path, that shrink on an earlier subpath can exactly
   cancel a `+1` seam-materialization grow on a later subpath — total count
   unchanged, yet every later-subpath anchor index shifted, leaving
   `point_selected` stale and silently retargeting subsequent edits. Element
   count is not equivalent to "indices unchanged". **Fix (`app/mod.rs`):** drop
   the `old_len`/`count_changed` bookkeeping and clear `self.point_selected`
   **unconditionally**, matching the sibling `RoundSelectedCorners` /
   `DeleteAnchors` handlers. The convert op has no more need to retain the
   selection than its siblings; unconditional clearing is the only structurally
   sound choice given reunify can reshape any subpath. This supersedes the
   round-1 count-change guard described above.

Verification: `cargo build --release` clean; `cargo test -p photonic-gui`
passes; the changed region has no new clippy warnings.
