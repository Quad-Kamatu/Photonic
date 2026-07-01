# Direct Select — whole-object vertex highlight + drag-fill to move (#164)

> Status: **implemented**. Small, self-contained GUI-behavior fix in the Direct
> Selection tool. area:gui, priority:p1, type:bug.

## What this PR implements

All of the "In" scope below shipped in the `photonic-gui` crate:

1. **Show all vertices on selection (req 1).** A new `last_tool: Tool` field on
   `PhotonicApp` edge-detects switching *into* Direct Select at the top of
   `draw` (`app/mod.rs`). On that edge, `seed_direct_select_from_selection`
   (`app/direct_select.rs`) sets `point_edit_node` from `selected_id` (or the
   first `doc.selection` id) when it is a `Path`, and `select_all_anchors` fills
   `point_selected` with every anchor index via `path_anchor_points`, so the
   whole path renders filled with no extra click. A fresh body click on a shape
   in `drag_started` now also selects all its anchors.
2. **Drag the fill to move (req 2).** New `DirectDrag::Shape { start_e, start_f }`
   variant (`app/mod.rs`). In `drag_started`, a body press on the
   *already-selected* edit node captures the node + its `matrix[4]/[5]` and
   starts `Shape` mode; a press on a *different* shape just selects it (+ all
   anchors). The `dragged_by` `Shape` arm translates `matrix[4]/[5]` by the
   canvas delta (`press_origin`→`interact_pointer_pos` / `view.zoom`), mirroring
   the Move tool. `drag_stopped`'s `changed` check now also compares
   `transform.matrix`, so the move records a `Command::UpdateNode` for undo/redo.
3. **`bez_move_anchors` single-pass rewrite (`app/geometry.rs`).** Replaced the
   two-write approach (which corrupted handles when several/all anchors moved
   together — exposed by req 1's all-selected default) with a single membership
   pass: endpoint + in-handle `c2` move iff the element's anchor is selected;
   out-handle `c1` moves iff the previous anchor is selected (skipping
   `ClosePath`); a `QuadTo` control moves iff either endpoint is selected.
   Single-anchor behaviour (incl. the `set_anchor_pos` caller) is unchanged.

Build: `cargo build --release` passes; `cargo check --workspace` passes;
`cargo test -p photonic-gui` passes (no unit tests in that target). No MCP tools
touched, so `docs/mcp-api.md` is unaffected.

## Remaining work

Deferred exactly as scoped under "Out / deferred" — none of these are required
by #164:
- Marquee / rubber-band vertex selection.
- Multi-node Direct Select (seeding handles the single-selected-node case; a
  multi-selection seeds the first path node only).
- Snapping / axis-lock on the shape drag (Move-tool parity beyond translation).

## Summary

The Direct Selection tool should treat a whole object like Illustrator does:

1. **Show all vertices on selection.** Selecting a shape with Direct Select —
   or switching to Direct Select while an object is already selected — should
   display *all* anchor points on the path, rendered as if the whole path were
   selected (filled).
2. **Drag the fill to move the shape.** With a shape selected via Direct Select,
   pressing-and-dragging its fill/interior should move the entire shape.
   Currently a body press only (re)selects the node and the drag does nothing.

All logic lives in `crates/photonic-gui/src/app/direct_select.rs`
(`handle_direct_select_tool`), with a new drag variant in `app/mod.rs` and one
geometry helper fix in `app/geometry.rs`.

## Current behavior (grounding)

`handle_direct_select_tool` (`app/direct_select.rs`) keeps point-edit state on
`PhotonicApp`: `point_edit_node: Option<NodeId>`, `point_selected: Vec<usize>`
(anchor element indices), `point_drag_mode: Option<DirectDrag>`,
`point_drag_origin: Option<SceneNode>`.

- On `drag_started`, priority is handle > corner-widget > anchor > **body**. The
  body branch (`direct_select.rs` ~line 224) just sets
  `self.point_edit_node = hit_shape; self.point_selected.clear();` and sets
  `point_drag_mode = None` — so a fill drag is inert.
- On `clicked` (~line 380), a body hit selects the node but clears
  `point_selected`.
- The overlay already draws *every* anchor square (selected = filled accent,
  others = white), so vertices are drawn once a node is the edit node — but
  nothing seeds `point_edit_node` when you **switch into** Direct Select with a
  node already selected, so no points appear until you click the shape again.
- `DirectDrag` (`app/mod.rs:105`) has `Anchors`, `Handle`, `Corner` — no
  whole-shape move.
- The regular Move tool (`app/tool_handlers.rs` ~line 613) moves a node by
  writing `node.transform.matrix[4]/[5]` (translation e/f) — the pattern to
  reuse for a whole-shape move.
- `bez_move_anchors` (`app/geometry.rs:378`) has a latent bug: when moving a
  *set* of adjacent anchors it skips the outgoing handle `c1` of a segment whose
  next endpoint is also selected (`!sel_set.contains(&j)` guard), and the
  ascending-order writes overwrite each other, so rigidly translating multiple
  anchors corrupts curve handles. This matters once "all anchors selected"
  becomes the default state (requirement 1) and the user drags a vertex.

## Scope

### In
- Seed `point_edit_node` (and select all its anchors) when the Direct Select
  tool becomes active while a single path node is selected.
- Select all anchors when a shape newly becomes the edit node via a body
  click, so the whole path renders filled ("as if selected").
- New `DirectDrag::Shape` variant + drag handling: a body press on the
  **already-selected** edit node starts a whole-shape move that translates the
  node transform; drag-end records it in history.
- Fix `bez_move_anchors` so rigidly moving several/all selected anchors
  translates every handle correctly (single-pass rewrite).

### Out / deferred
- Marquee/rubber-band vertex selection (not requested).
- Multi-node Direct Select (seeding handles the single-selected-node case; a
  multi-selection just seeds the first path node).
- Snapping/axis-lock on the shape drag (Move-tool parity beyond translation).

## Approach

**1. DirectDrag::Shape (`app/mod.rs`).** Add a variant carrying the press
anchor in canvas space so per-frame delta is stable:
```rust
/// Moving the whole shape by dragging its fill — translates the node transform.
Shape { start_e: f64, start_f: f64 },
```
(store the node's original `matrix[4]/[5]`; combine with the canvas delta from
`press_origin` → current pointer, mirroring the Move tool).

**2. Seed on tool entry (`direct_select.rs`, top of handler).** Add a small
"just entered Direct Select" edge detector — a `bool` field on `PhotonicApp`
(e.g. `direct_select_active`) set true here and cleared wherever `active_tool`
leaves DirectSelect (or a `last_tool: Tool` field updated once per frame). On
the entry edge, if `point_edit_node.is_none()` and `self.selected_id` (or the
first `doc.selection` id) is a `SceneNodeKind::Path`, set `point_edit_node` to
it and populate `point_selected` with all anchor indices via a helper
`select_all_anchors` (below). Edge-triggering (not `is_none()` every frame)
avoids re-selecting after the user deliberately clicks empty to deselect.

**3. Select-all-anchors helper.** Small helper that fills `point_selected` from
`path_anchor_points(&bez)` (`geometry.rs:210`) — the element indices of every
`MoveTo/LineTo/CurveTo/QuadTo`:
```rust
self.point_selected = path_anchor_points(&bez).iter().map(|(i, _)| *i).collect();
```
Call it (a) on the tool-entry seed and (b) in the body branches that newly set
`point_edit_node` to a shape (drag_started ~224, clicked ~385) so a fresh
selection shows the whole path filled.

**4. Drag the fill to move (`direct_select.rs` drag_started body branch).**
Replace the inert body branch:
- If `hit_shape == self.point_edit_node` (already the edit node) → start
  `DirectDrag::Shape { start_e, start_f }` and capture
  `point_drag_origin = node.clone()`.
- Else (different or first shape) → set `point_edit_node = hit_shape` and
  `select_all_anchors` (no move this press — matches "select first, then drag").

In the `dragged_by` match, add a `Shape` arm that writes
`node.transform.matrix[4] = start_e + dx; matrix[5] = start_f + dy` where
`(dx, dy)` is the canvas delta (`press_origin`→`interact_pointer_pos`, divided
by `view.zoom`), and sets `*doc_modified = true`.

**5. Record shape moves in history (`direct_select.rs` drag_stopped).** The
drag-end `changed` check currently compares only `path_data`. Extend it so a
transform-only change is also recorded, e.g. also compare
`old_node.transform.matrix != new_node.transform.matrix`, then
`history.execute(Command::UpdateNode { old, new }, doc)` as today.

**6. Fix `bez_move_anchors` (`app/geometry.rs`).** Rewrite as a single pass over
elements, moving each constituent point by membership so rigid multi-anchor
moves are correct and single-anchor behavior is unchanged:
- endpoint `p` and in-handle `c2` of element `i` move iff `i ∈ sel`;
- out-handle `c1` of element `i` moves iff the previous endpoint anchor
  (`i-1`, when not a `ClosePath`) `∈ sel`.
This reproduces today's single-anchor result (out-handle lives on the *next*
element) while fixing the overwrite/guard bug for the all-selected case that
requirement 1 makes routine.

## Verification
- `cargo build --release` (house rule).
- GUI (`target/release/photonic`): select a shape with Select, press `A`/switch
  to Direct Select → all vertices appear filled. Drag the fill → whole shape
  moves; undo restores it. Drag a single vertex → path edits without handle
  corruption. Escape still exits point-edit. (Joseph launches/verifies the GUI.)

## Fix round 1 (adversarial review follow-up)

Two reviewer findings addressed in the working tree (not committed):

**[major] Stale `point_edit_node` defeats the re-seed on command-palette /
global-search entry.** `seed_direct_select_from_selection` early-returns while
`point_edit_node.is_some()`, which is only safe if the field is reliably cleared
on every tool switch. Two of the five switch sites (command_center.rs `_ =>
tool_for_command` fallthrough, and mod.rs `apply_search` `A::Tool`) set
`active_tool` without clearing point-edit state, so re-entering Direct Select via
the palette/global-search showed the *previous* object's vertices. Fixed by
extracting the copy-pasted clear block into `PhotonicApp::clear_point_edit()`
(direct_select.rs) and calling it at all five sites — the three toolbar/hotbar
paths (mod.rs) now call the helper, and the two previously-missed palette/search
paths now clear before switching.

**[major] Plain click on a shape body didn't fill anchors.** In egui a clean
press+release fires `clicked()` (not `drag_started()`), so a genuine click ran
only the `clicked` handler, which did `point_selected.clear()` — anchors rendered
white, half-implementing requirement 1. Fixed by mirroring the drag_started body
branch: when a click newly makes a shape the edit node, fetch its bez and call
`select_all_anchors` so every anchor fills. The already-edit-node case still
clears (a body click re-collapses the vertex selection), consistent with prior UX.

Verified: `cargo build --release` clean; `cargo test --release -p photonic-gui`
passes (no dedicated direct-select tests exist yet — deferred).

### Deferrals (unchanged from original plan)
- Multi-node / non-Path selections are still not seeded — only a single Path node
  fills on tool entry (see "Out/deferred").
- No automated regression test for the tool-entry seed or click-fill behavior;
  interaction is GUI-driven and validated manually by Joseph.
