# 179 — Corner rounding: same half-edge clamp bug in whole-path rounder + MCP round_corners handler

## Status: Implemented

### What this PR implements
- **Tessellator** (`crates/photonic-render/src/tessellator.rs`, `round_subpath` →
  `emit_line_run`): the `signed_area` / `winding` / `is_convex` block was moved
  above `clamped_r`, a local `is_rounded(j)` predicate was added, and `clamped_r`
  is now neighbour-aware — `max_in`/`max_out` are `seg * 0.5` toward a rounded
  neighbour and `seg * (1 - eps)` (eps = 1e-3) toward an unrounded one
  (open-run endpoint, concave corner, or curve-junction vertex). `retreat` /
  `advance` inherit the new bound; the convex-only emission loop is unchanged.
- **MCP** (`crates/photonic-mcp/src/handlers/nodes.rs`, `apply_round_corners` →
  `flush`): the unconditional `max_r = (len_in/2).min(len_out/2)` clamp was
  replaced with the same neighbour-aware formula, using `prev_rounded = closed || i >= 2`
  and `next_rounded = closed || i < n - 2` (underflow-safe — `flush` returns early when
  `pts.len() < 2` so `n >= 2` — only evaluated for interior corners). Closed subpaths
  keep the L/2 split; open-run corners adjacent
  to an endpoint gain the full retreat. Fillet emission is unchanged.
- **Tests**: two new tessellator tests (`open_polyline_corner_rounds_past_half_edge`,
  `closed_rect_adjacent_fillets_never_overlap`) and two new MCP tests
  (`open_run_corner_rounds_past_half_edge`, `closed_square_splits_edges_fifty_fifty`)
  in a new `round_corners_tests` module.
- **Docs**: reworded the `round_corners` doc comment (tessellator) and the
  `round_corners` tool description (`server.rs`) from "clamped to half the shortest
  adjacent segment" to "clamped so adjacent fillets never overlap". No tool
  schema/arg change, so `docs/mcp-api.md` needs no regeneration.

### Verification
`cargo build --release`, `cargo test -p photonic-render` (23 passed),
`cargo test -p photonic-mcp` (round_corners_tests + all others passed), and
`cargo check --workspace` all succeed. No new clippy warnings (both clamp
predicates use the `<`/`< n - 1` form, not `int_plus_one`).

**Scope of the two halves — read this before assuming end-to-end parity:**
- **MCP half is genuinely wired.** `server.rs:901` dispatch → `handlers::nodes::round_corners`
  → `apply_round_corners` → node update. The neighbour-aware clamp there produces
  observable, user-visible behaviour and is covered by the two new MCP tests.
- **Tessellator half is a latent-bug fix to currently-unreferenced public API.**
  `tessellator::round_corners` (and its `round_subpath` / `emit_line_run` internals
  where this clamp lives) has **no production caller** anywhere in the workspace: an
  exhaustive grep for `round_corners` across all `.rs` shows only the definition plus
  its own two test functions. The render pipeline (`renderer.rs`, `compositor.rs`,
  `headless.rs`) imports only `tessellate_fill` / `tessellate_stroke` /
  `tessellate_stroke_variable`, and there is no `corner_radius` / `border_radius` node
  property in photonic-render or photonic-core that would flow into it. So this side is
  **source-level parity** with the GUI/MCP clamp, unit-tested but **not an observable
  render-behaviour change** (goal-backward Level 3 "wired-in" is not met — it sits at
  L2 "real code", reachable only from tests). The change is correct; the honest claim
  is "consistent at the source level", not "consistent runtime behaviour across the
  render tessellator".

### Remaining work
- **Wire `tessellator::round_corners` into the render path, or retire it.** It is
  currently unreferenced public API. If corner rounding is intended to affect rendering
  (e.g. a future `border-radius` / `corner_radius` node property feeding the fill/stroke
  tessellation), that wiring is explicit outstanding work and should be tracked as such;
  until then the render-side fix is latent (correct-but-dormant).
- The GUI `round_selected_corners` path was already fixed in #165 / PR #178 and is
  intentionally out of scope here.

## Original plan (design scaffold)

Follow-up from **#165** (PR #178). That change fixed the artificial half-edge
clamp only in the **GUI Direct Select** flow (`round_selected_corners` in
`crates/photonic-gui/src/app/geometry.rs`). The reviewers flagged that the same
class of clamp lives in two other whole-path rounders that were explicitly out
of scope for the GUI-only fix. This proposal ports the #165 neighbour-aware
clamp to both so the logic is consistent across the GUI, the render
tessellator, and the MCP surface. Note (see Verification): only the GUI and MCP
paths are wired end-to-end; `tessellator::round_corners` currently has no
in-workspace caller, so its fix is source-level parity, not a runtime change.

## Summary

Both whole-path rounders apply an **unconditional** per-corner half-edge clamp:

- `crates/photonic-render/src/tessellator.rs`, `round_subpath` → `emit_line_run`,
  the `clamped_r` closure (line ~108):
  ```rust
  radius.min(seg_in * 0.5).min(seg_out * 0.5)
  ```
- `crates/photonic-mcp/src/handlers/nodes.rs`, `apply_round_corners` → the
  `flush` closure (lines ~9450-9451):
  ```rust
  let max_r = (len_in / 2.0).min(len_out / 2.0);
  let r = radius.min(max_r);
  ```

Because these round the *whole* path (not a selected subset), one might assume
every neighbour is also rounded, making the half clamp correct. It is not always
correct: a corner's neighbour is **not** filleted when the neighbour is

- a **subpath endpoint** of an open (unclosed) run — the endpoint keeps its full
  vertex, taking none of the shared edge;
- a **concave** corner (tessellator only rounds *convex* corners; concave ones
  stay sharp);
- across a **curve junction** — a `CurveTo`/`QuadTo` breaks the straight-line run
  into separate open runs, so the vertex bordering the curve is an endpoint.

In all three cases the neighbour consumes none of the shared edge, so the corner
could round out to (just short of) the adjacent vertex. The current half clamp
caps it at `L/2` regardless — the exact artificial cap #165 removed. Interior
corners whose neighbours *are* rounded must still split the shared edge 50/50 to
prevent overlapping fillets; that case stays at `L/2`.

## Scope

### In
- `crates/photonic-render/src/tessellator.rs` — make `clamped_r` in
  `emit_line_run` neighbour-aware, mirroring #165:
  - `max_in  = if prev_rounded { seg_in  * 0.5 } else { seg_in  * (1.0 - eps) }`
  - `max_out = if next_rounded { seg_out * 0.5 } else { seg_out * (1.0 - eps) }`
  - `r = radius.min(max_in).min(max_out)`, `eps = 1e-3`.
  - `prev_rounded` / `next_rounded` reuse the run's existing filleting predicate:
    a neighbour `j` is rounded iff it is a genuine filleted corner of this run —
    for a `closed` run, `is_convex(j)`; for an open run, `j` is interior
    (`1 <= j <= n-2`) **and** `is_convex(j)`. Endpoints and concave corners are
    "not rounded" and grant the full-edge retreat.
  - Requires reordering so `signed_area` / `winding` / `is_convex` are defined
    *above* `clamped_r` (closures can't forward-reference later closures). The
    `retreat`/`advance`/emission logic is otherwise unchanged.
- `crates/photonic-mcp/src/handlers/nodes.rs` — make `apply_round_corners`'s
  `flush` clamp neighbour-aware. Corners here are the interior vertices of a run
  (endpoints handled by the `is_endpoint` branch), so for corner `i`:
  - `prev_rounded = closed || i >= 2` (prev is interior, not endpoint 0);
  - `next_rounded = closed || i < n - 2` (next is interior, not endpoint n-1);
  - same `max_in`/`max_out`/`r` formula with `eps = 1e-3`.
  - For a `closed` subpath every corner stays at the `L/2` split (behaviour
    unchanged); only open-run corners adjacent to an endpoint gain the full
    retreat.
- Unit tests:
  - `crates/photonic-render/src/tessellator.rs` `#[cfg(test)] mod tests` (already
    present): an open 3-vertex polyline (single interior corner, both neighbours
    are endpoints) rounds its corner past the half-edge; a closed rounded-rect's
    adjacent corners never overlap (fillet endpoints stay on their side of each
    edge midpoint).
  - MCP: a small test on `apply_round_corners` covering the same two cases
    (open run corner rounds past half; closed square corners split 50/50).

### Out
- The GUI `round_selected_corners` path — already fixed in #165/PR #178.
- No change to `emit_corner_arc`, winding/convexity detection, subpath splitting,
  or the `round_corners` tool's public signature / args.
- No MCP tool schema change (behaviour-only). `docs/mcp-api.md` needs no
  regeneration — the tool description already says the radius is auto-clamped;
  the wording "half the shortest adjacent segment" in the server.rs tool
  description and the tessellator doc comment may be lightly reworded to
  "clamped so adjacent fillets never overlap", but no arg/tool change.

## Approach

1. **Tessellator.** In `emit_line_run`, move the `signed_area`/`winding`/
   `is_convex` block above `clamped_r`. Add a local `is_rounded(j: usize) -> bool`
   capturing `closed`, `n`, and `is_convex`. Rewrite `clamped_r(i)` to look up
   `prev_rounded = is_rounded((i + n - 1) % n)` and
   `next_rounded = is_rounded((i + 1) % n)` and apply the `max_in`/`max_out`
   formula above. `retreat`/`advance` already call `clamped_r`, so they inherit
   the new bound with no further change. The convex-only emission loop is
   untouched.
2. **MCP.** In `apply_round_corners`'s `flush`, replace the `max_r` line with the
   neighbour-aware `prev_rounded`/`next_rounded` computation (guarded against
   `usize` underflow — `flush` returns early when `pts.len() < 2` so `n >= 2` and
   `n - 2` cannot underflow). Fillet-endpoint emission (`line_to(fillet_start)` /
   `quad_to(curr, fillet_end)`) is unchanged.
3. Reword the two doc/description strings that assert a hard half-edge cap so the
   documentation matches the new behaviour.

House rule: `cargo build --release` after the edits; run
`cargo test -p photonic-render` and `cargo test -p photonic-mcp`; then
`cargo check --workspace`.
