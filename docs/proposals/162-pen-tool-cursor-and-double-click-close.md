# 162 — Pen tool: cursor doesn't change; double-click doesn't close the shape

> **Status: implemented.** All three in-scope items shipped in
> `crates/photonic-gui/src/app/tool_handlers.rs` (single file, no `photonic-core`
> changes). `cargo build --release`, `cargo test -p photonic-gui`, and
> `cargo check --workspace` all pass.

## What this PR implements

- **State-reflecting cursor** — `handle_pen_tool` now sets a cursor each frame while
  the Pen is active and hovering the canvas: `CursorIcon::Crosshair` for normal
  point-placing, `CursorIcon::PointingHand` when the pointer is within the close
  radius of the first anchor (with ≥3 points placed), signalling "click to close".
- **Double-click closes the path** — `build_pen_path` gained a `close: bool` param
  that appends `bez.close_path()` when `close && pen_points.len() >= 3`. The
  double-click finalise branch passes `true`, so the emitted node is a closed,
  fillable region instead of an open polyline. 2-point paths stay open (a closed
  2-point path is degenerate).
- **Click-to-close (bonus)** — clicking the first anchor (same 8px screen hit test as
  the cursor state, ≥3 points) finalises and closes the path, Illustrator-style.
- **Refactor** — the shared finalise logic (`make_node` + `doc.add_node`) is factored
  into `finalize_pen_node`, and the close hit test into `pen_over_first_anchor`, so
  the double-click and click-to-close paths don't duplicate code.

## Remaining work

- Bezier curve handles / click-drag tangents remain out of scope — the Pen is still
  polyline-only, as before. No `PathData`/`BezPath` changes in `photonic-core`.
- No custom cursor bitmaps; egui has no pen glyph, so existing `CursorIcon` variants
  are reused.

## Summary

Two UX bugs in the Pen tool (`Tool::Pen`), both isolated to
`crates/photonic-gui/src/app/tool_handlers.rs`:

1. **Cursor never changes.** `handle_pen_tool` draws a live preview but never calls
   `ctx.set_cursor_icon(...)`, so the pointer stays the default arrow while the Pen
   is active — unlike Direct Select / Width / Erase tools, which set a cursor each
   frame. The active tool is not visually obvious, and the "close-path" state is
   invisible.
2. **Double-click leaves the shape open.** The double-click branch finalises the
   path via `build_pen_path()`, but that helper emits an open polyline
   (`move_to` + `line_to…`, no `close_path()`). The user expects double-click to
   close the current path; instead they get an un-closed shape.

## Scope

### In
- Set a state-reflecting cursor while the Pen is active and hovering the canvas:
  `CursorIcon::Crosshair` for normal point-placing, and a distinct icon
  (`CursorIcon::PointingHand`) when the pointer is within the close radius of the
  first anchor (signals "click to close") — egui has no dedicated pen glyph, so we
  reuse existing `CursorIcon` variants.
- Make double-click close the path: give `build_pen_path` a `close: bool` param (or
  add `bez.close_path()` in the finalise branch) so a `>= 3`-point path is emitted
  closed. 2-point paths stay open (a closed 2-point path is degenerate).
- Bonus (cheap, standard Pen behaviour, still one file): allow **clicking the first
  anchor** to close+finalise the path, matching Illustrator. Reuses the same close
  radius / hit test as the cursor state.

### Out
- Bezier curve handles / click-drag to shape tangents (the Pen currently only places
  straight-line anchors; `build_pen_path` is polyline-only). Not touched.
- Any change to `PathData`/`BezPath` in `photonic-core`.
- New custom cursor bitmaps.

## Approach

All edits in `crates/photonic-gui/src/app/tool_handlers.rs`, `handle_pen_tool`
(~L912–1005) and `build_pen_path` (~L1008–1019).

1. **Close-hit helper.** Compute the screen-space distance from the hover pos to the
   first anchor (`pen_points[0]` via `view.canvas_to_screen`). Treat within ~8px as
   "over first anchor". Used both for the cursor state and the click-to-close path.

2. **Cursor.** Near the top of `handle_pen_tool`, gate on `response.hovered()` and
   call `ui.ctx().set_cursor_icon(...)`: `PointingHand` when hovering the first
   anchor with `pen_points.len() >= 3` (close available), else `Crosshair`. Matches
   the per-frame pattern already used in `direct_select.rs` and `width_tool.rs`.

3. **Close on finalise.** Change `build_pen_path(&self, close: bool)` to append
   `bez.close_path()` when `close && pen_points.len() >= 3`. Update the single
   caller (the double-click branch, L929) to pass `true`. `BezPath::close_path()`
   already exists and is used across `photonic-core/src/ops/*`.

4. **Click-to-close.** In the single-click branch (L952), before pushing a new
   point, if the click lands on the first anchor and `pen_points.len() >= 3`,
   finalise via `build_pen_path(true)` + `make_node` + `doc.add_node` (same as the
   double-click branch) and clear `pen_points` instead of adding a point. Factor the
   finalise block into a small local closure/helper to avoid duplicating the
   `make_node`/`add_node` logic.

## Verification
- `cargo build --release` succeeds (house rule).
- Manual: activate Pen, confirm crosshair cursor; place 3+ points; hover first
  anchor → PointingHand; double-click (or click first anchor) → node is a **closed**
  path (fill renders as a closed region). Escape still cancels.
