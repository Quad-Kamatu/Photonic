# 163 — Selection hit-testing: clicking a shape's fill should reliably select it

> Status: **IMPLEMENTED** on branch `pre-deploy/interactive-editing-fixes`.
> Regression / incomplete fix of #3 ("hit-testing uses bounding box instead of shape
> geometry"). PR #74 added geometry-based hit-testing in
> `crates/photonic-gui/src/app/hit_test.rs`, but the interior fill test was unreliable
> for **open-but-filled** and **compound** paths. This PR fixes that.

## What this PR implements

- Added `closed_for_fill(bez: &BezPath) -> BezPath` in `hit_test.rs`: returns a copy
  with every open subpath explicitly closed (inserts `ClosePath` before each `MoveTo`
  following an open subpath and at the end). Idempotent for already-closed subpaths.
- `path_geometry_hit` now tests `closed_for_fill(&canvas_bez).contains(pt)` for the
  interior body test (still gated on `pn.fill.enabled`).
- `direct_select_hit` now tests `closed_for_fill(&canvas_bez).contains(pt)` for its
  fill-agnostic interior test.
- The outline-proximity fallback (`seg.nearest(...).distance_sq <= tol_sq`) is
  **unchanged** in both, so unfilled open/stroke-only paths remain clickable only on
  their edge — preserving the #3 "click through transparent areas" contract.
- The drag-to-select branch in `tool_handlers.rs` funnels through the same `hit_test`,
  so it is fixed transitively (no change needed there).
- Six unit tests added in `hit_test.rs`: open filled quad selects on interior click;
  closed shape still selects (no regression); compound path with unclosed first subpath
  selects inside it; unfilled open path NOT selected on interior body click (only on the
  edge); plus helper-level idempotency and close-insertion tests.

Verification: `cargo build --release` OK, `cargo test -p photonic-gui` 35 passed
(6 new), `cargo check --workspace` OK.

## Remaining work

- Even-odd vs non-zero fill-rule fidelity for self-intersecting compound fills is not
  addressed (kurbo `contains` stays non-zero) — out of scope, as scoped below.
- No MCP tools touched; `docs/mcp-api.md` regeneration not required.

## Summary

Both the Selection tool (`hit_test` → `path_geometry_hit`) and the Direct Select tool
(`direct_select_hit`) test whether a click lands on a shape's body with
`canvas_bez.contains(pt)` (kurbo winding, non-zero rule). The problem: kurbo's winding
iterator (`kurbo-0.11.3` `bezpath.rs`) only emits a closing edge for an **explicit**
`PathEl::ClosePath`. An open subpath contributes **no** closing segment, so `contains`
returns `false` for points that are visually inside the fill.

A fill, however, is *rasterised as if each contour is closed* — so a shape can look
solidly filled while its stored path has no trailing `Z`, and clicking dead-center on
its fill misses. This is exactly the "clicking a shape's fill does not reliably select
it" report.

Primitive shapes (`PathData::rect` / `ellipse` / `star` / `regular_polygon`) call
`bez.close_path()` and the `Z` survives the `to_svg` / `from_svg` round-trip, so those
select fine — hence "reliably" (some shapes work, some don't). The ones that fail:

- **Pen-tool paths with a fill** that were never explicitly closed (fill implicitly
  closes on render; hit-test doesn't).
- **Compound paths** where an intermediate subpath lacks `ClosePath` before the next
  `MoveTo` (winding for that region is computed against an open contour).
- **Imported SVG** paths that carry a fill but open subpaths.

## Scope

### In
- Add a small helper in `hit_test.rs` — `closed_for_fill(bez: &BezPath) -> BezPath` —
  that returns a copy with **every open subpath explicitly closed** (insert
  `ClosePath` before each `MoveTo` that follows an open subpath, and at the end).
  Idempotent for already-closed subpaths (kurbo skips a zero-length closing edge when
  `last == start`).
- Use it for the **interior** containment test in both hit paths:
  - `path_geometry_hit`: gate stays `pn.fill.enabled`, but test
    `closed_for_fill(&canvas_bez).contains(pt)` instead of `canvas_bez.contains(pt)`.
  - `direct_select_hit`: the fill-agnostic interior test becomes
    `closed_for_fill(&canvas_bez).contains(pt)`.
- Leave the **outline-proximity** fallback (`seg.nearest(...).distance_sq <= tol_sq`)
  unchanged — it already handles open, stroke-only, and hairline paths on the edge, and
  must NOT be forced closed (an unfilled open path is clickable only on its stroke).
- Unit tests in `hit_test.rs`: (a) an open filled quad (MoveTo + 3 LineTo, **no**
  close) selects on an interior click; (b) a closed shape still selects (no
  regression); (c) a compound path with an unclosed first subpath selects inside that
  subpath; (d) an unfilled open path is NOT selected on an interior click (only on the
  edge) — preserving the #3 "click through transparent areas" contract.

### Out
- No change to the cheap bounding-box pre-reject (`text_aware_canvas_bounds`), the
  click-vs-drag routing in `tool_handlers.rs`, or non-path node handling (text / image
  / group keep bbox hits). The drag-to-select branch already funnels through the same
  `hit_test`, so it is fixed transitively.
- No change to `photonic-core` path storage or the shape constructors — shapes stay
  stored exactly as authored; we only normalise a throwaway copy at hit-test time.
- Even-odd vs non-zero fill-rule fidelity for self-intersecting compound fills is not
  addressed here (kurbo `contains` stays non-zero); out of scope.

## Approach

Root cause is isolated to two functions in one file. The fix mirrors render semantics
(fills close their contours) inside the hit-test, without mutating stored geometry:

```rust
/// Close every open subpath so winding-based `contains` matches how a fill is
/// actually rasterised. Idempotent for already-closed subpaths.
fn closed_for_fill(bez: &BezPath) -> BezPath {
    use kurbo::PathEl;
    let mut out = BezPath::new();
    let mut open = false;
    for el in bez.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                if open { out.close_path(); }
                out.move_to(p);
                open = true;
            }
            PathEl::ClosePath => { out.close_path(); open = false; }
            other => { out.push(other); }
        }
    }
    if open { out.close_path(); }
    out
}
```

Then in `path_geometry_hit`:

```rust
if pn.fill.enabled && closed_for_fill(&canvas_bez).contains(pt) {
    return Some(true);
}
```

and the equivalent substitution in `direct_select_hit`. Verify with
`cargo build --release`, `cargo test -p photonic-gui`, and `cargo check --workspace`.

## Files

- `crates/photonic-gui/src/app/hit_test.rs` — add `closed_for_fill`, use it in
  `path_geometry_hit` and `direct_select_hit`, add unit tests.
