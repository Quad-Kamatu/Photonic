# 165 — Corner rounding on multi-selected vertices is unreliable + has an artificial cap

## Status: Implemented

**What this PR implements**

- `round_selected_corners` (`crates/photonic-gui/src/app/geometry.rs`) now uses a
  **neighbour-aware retreat clamp** instead of the unconditional per-corner
  `radius.min(lin/2).min(lout/2)`. Per corner `k`:
  - a per-position `rounded[k]` predicate (`selected` ∧ roundable straight corner
    ∧ has neighbours) is computed once for the subpath and drives both fillet
    emission and the neighbour-awareness;
  - `max_in  = if prev rounded { lin/2 } else { lin*(1-eps) }`;
  - `max_out = if next rounded { lout/2 } else { lout*(1-eps) }`;
  - `r = radius.min(max_in).min(max_out)`, `eps = 1e-3`.
  - Fillet emission (`line_to(fs)` / `quad_to(curr, fe)`) is unchanged.
  This removes the artificial half-edge cap on isolated corners (they now round
  until the arc reaches the adjacent vertex) and makes any set of simultaneously
  rounded adjacent corners split their shared edge 50/50 deterministically, with
  no overlap regardless of selection order or radius.
- Unit tests in `crates/photonic-gui/src/app/mod.rs` (module
  `direct_select_geometry_tests`): `rounding_isolated_corner_rounds_past_half_edge`,
  `rounding_two_adjacent_corners_never_overlap`,
  `rounding_non_adjacent_corners_round_independently`. All geometry/rounding
  tests pass; `cargo build --release`, `cargo test -p photonic-gui`, and
  `cargo check --workspace` are green.

**Remaining work / deferred (unchanged from the original scope's "Out")**

- The whole-path rounder `round_corners` in
  `crates/photonic-render/src/tessellator.rs` and the MCP `round_corners` handler
  carry the same class of half-edge clamp. Left as a related follow-up — this PR
  targets the GUI Direct Select flow only. No MCP tools changed, so
  `docs/mcp-api.md` needs no regeneration.
- Drag-radius model in `direct_select.rs` and widget hit-testing/selection UX are
  untouched (correct as-is; the visible inconsistency came from the clamp).

## Summary

Rounding straight corners with the Direct Select tool (Live-Corners widget) is
unreliable when two or more vertices are selected, and it enforces an artificial
cap: a corner can only be rounded to **half** its shorter adjacent edge, even
when the neighbouring vertex is not being rounded. The corner should be
roundable until the fillet arc reaches an adjacent vertex (the full edge), and
adjacent selected corners should share a common edge deterministically instead
of behaving inconsistently.

Root cause is a single line in the fillet math of `round_selected_corners`
(`crates/photonic-gui/src/app/geometry.rs`):

```rust
let r = radius.min(lin / 2.0).min(lout / 2.0);
```

This unconditional half-edge clamp is applied per-corner with no knowledge of
whether the neighbour on each side is also being rounded. Consequences:

- **Artificial cap** — even an isolated corner (neighbours not selected) is
  capped at half the edge length, so it can never round out to touch the
  adjacent vertex.
- **Multi-select inconsistency** — the same absolute `radius` is applied to
  every selected corner and clamped independently. When two *adjacent* selected
  corners share an edge, each retreats up to `L/2`, so they collide/behave
  inconsistently at large radii; and the half clamp is wrong whenever the
  neighbour is in fact *not* selected.

## Scope

### In
- `crates/photonic-gui/src/app/geometry.rs` — make the fillet retreat clamp in
  `round_selected_corners` neighbour-aware:
  - Along an edge whose *other* endpoint is **also** being rounded (its index is
    in `selected` and it is a roundable straight corner), split the edge — cap
    the retreat at `L/2` so the two fillets meet at most at the midpoint (no
    overlap, deterministic for multi-select).
  - Along an edge whose other endpoint is **not** being rounded, allow the
    retreat up to the full edge length (minus a small epsilon) so the arc can
    reach — but not pass — the adjacent vertex. This removes the artificial cap.
- Unit tests in `crates/photonic-gui/src/app/mod.rs` (alongside the existing
  `rounding_one_corner_*` tests) covering: isolated corner rounds past half-edge;
  two adjacent selected corners on a shared edge never overlap; a non-adjacent
  pair each round independently.

### Out
- The whole-path rounder `round_corners` in
  `crates/photonic-render/src/tessellator.rs` and the MCP `round_corners`
  handler — same class of clamp, but the issue is the GUI Direct Select flow
  (`area:gui`). Note as related follow-up, do not change here.
- The drag-radius model in `direct_select.rs` (cursor-distance-to-pivot). It is
  correct; the visible inconsistency comes from the clamp, not the radius
  source. No change.
- Any change to widget hit-testing / selection UX.

## Approach

In `round_selected_corners`, the fillet endpoints for corner `k` are computed
from its `prev`/`curr`/`next` points. Extend the per-corner logic so that the
retreat distance on each side is bounded by whether that side's neighbour is
itself being rounded:

1. Build the selected-corner set once (already available as `selected`), plus
   the roundable-corner set for the subpath (a neighbour only "shares the
   split" if it is a genuine straight corner that will actually be filleted).
2. For corner `k` with incoming edge `prev→curr` (length `lin`) and outgoing
   edge `curr→next` (length `lout`):
   - `max_in  = if prev_is_rounded { lin / 2.0 } else { lin * (1.0 - eps) }`
   - `max_out = if next_is_rounded { lout / 2.0 } else { lout * (1.0 - eps) }`
   - `r = radius.min(max_in).min(max_out)`
   with a small `eps` (e.g. `1e-3`) to avoid a degenerate zero-length segment
   at the adjacent vertex.
3. Emit the fillet as today (`line_to(fs)` / `quad_to(curr, fe)`), unchanged.

The neighbour lookup uses the same `CornerSub` / `neighbours(k)` machinery the
function already has; "is the neighbour rounded" = neighbour's element index is
in `selected` **and** it is a straight corner of this subpath. This keeps
single-corner rounding continuous up to the adjacent vertex, and makes any set
of simultaneously-rounded corners share their common edges 50/50 so the result
is deterministic regardless of selection order or radius.

House rule: `cargo build --release` after the edit; run the geometry unit tests.
