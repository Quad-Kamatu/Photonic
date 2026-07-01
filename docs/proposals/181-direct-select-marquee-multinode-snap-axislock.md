# Direct Select — marquee vertex selection + multi-node drag + snap/axis-lock (#181)

> Status: **implemented**. Small, self-contained GUI-behavior feature in the
> Direct Selection tool. area:gui, priority:p2, type:feature. Follow-up from #164
> (PR #178), which deferred exactly these three interaction-parity pieces.

## What this PR implements

All three parity gaps are wired end-to-end in `photonic-gui`, reusing existing
helpers (no new geometry, no MCP surface touched):

1. **Marquee / rubber-band vertex select.** New `point_marquee_start:
   Option<egui::Pos2>` field on `PhotonicApp` (init `None`; reset in
   `clear_point_edit`, `invalidate_point_edit`, and the Escape block). In
   `handle_direct_select_tool`'s empty-canvas body branch of `drag_started`, a
   press with a `point_edit_node` set now begins a marquee (stores the screen
   press pos) instead of deselecting. A translucent accent rect is drawn each
   frame (mirroring the Move tool's marquee in `tool_handlers.rs`). On
   `drag_stopped`, every anchor of the edit node is mapped to screen
   (`node.transform.apply` → `view.canvas_to_screen`, same mapping as the
   anchor-square overlay) and those inside the rect are selected — Shift/Ctrl
   unions with the current selection, else replaces. A zero-area click-through
   with no enclosed anchors falls back to a plain deselect. The marquee records
   no undo (it changes only the selection).
2. **Multi-node keep-drag.** The anchor-hit branch of `drag_started` collapses
   `point_selected` to a single index **only** when the pressed anchor is not
   already selected; grabbing a member of the current multi-selection keeps the
   whole set, so the existing `DirectDrag::Anchors` + `bez_move_anchors` path
   (rewritten in #164 to translate an anchor set rigidly) moves every selected
   anchor together. (The guard was already present from #164; this PR documents
   the #181 intent inline so the behavior is not accidentally regressed.)
3. **Shape-drag snap/axis-lock.** The `DirectDrag::Shape` arm now applies
   `axis_lock_8(dx, dy)` (`hit_test.rs`) when Shift is held, otherwise grid-snaps
   the translation target via `self.snap` (no-op unless `prefs.snap_to_grid`) —
   matching the Move tool's precedence (axis-lock beats grid snap). Drag-end
   history already records the transform change (#164 `drag_stopped`).

Verified: `cargo build --release`, `cargo check --workspace`, and
`cargo test -p photonic-gui` (51 tests) all pass.

## Remaining work

Deferred exactly as scoped below (no change from plan): cross-object marquee
across multiple paths, object-aware/smart-guide snapping on the shape drag
(#66), and marquee-driven handle / Live-Corner widget box-select.

## Summary

Three Illustrator-parity gaps remain in the Direct Selection tool
(`crates/photonic-gui/src/app/direct_select.rs`) after #164:

1. **Marquee / rubber-band vertex selection.** Dragging a box over empty canvas
   should select every anchor of the edit path that falls inside the box.
   Currently a body press on empty canvas just deselects, and only single-anchor
   click/shift-click seeding exists.
2. **Multi-node Direct Select.** Pressing an anchor that is *already* part of the
   multi-selection should keep the whole selection and drag every selected anchor
   together. Today the anchor-hit branch replaces `point_selected` with a single
   index unless Shift/Ctrl is held, so grabbing one of many selected anchors
   collapses the selection.
3. **Snap-to-grid / axis-lock on shape drag.** The `DirectDrag::Shape` fill-move
   (#164 req 2) writes a raw translation with no grid snap and no Shift-to-
   constrain-axis, unlike the Move tool.

All logic stays in `photonic-gui`. The move-tool already has every helper we
need to reuse: `axis_lock_8` (`app/hit_test.rs:237`), `PhotonicApp::snap`
(`app/mod.rs:12533`), and the marquee overlay pattern (`app/tool_handlers.rs`
~805/947). `bez_move_anchors` was already rewritten in #164 to translate a
*set* of anchors rigidly, so multi-node dragging needs only a selection-keep fix,
not new geometry.

## Current behavior (grounding)

`handle_direct_select_tool` (`direct_select.rs`) drives point-edit state on
`PhotonicApp`: `point_edit_node`, `point_selected: Vec<usize>`,
`point_drag_mode: Option<DirectDrag>`, `point_drag_origin: Option<SceneNode>`.

- **drag_started** (~397): priority handle > corner > anchor > body. The
  anchor-hit branch (~447) *replaces* `point_selected` with `[anchor_idx]` unless
  `add_sel` (Shift/Ctrl). The body branch (~460): fill of the already-selected
  node → `DirectDrag::Shape`; a different shape → select + `select_all_anchors`;
  **empty canvas → clears everything** (~490). This empty-canvas case is where a
  marquee must begin instead.
- **dragged_by** (~502): `DirectDrag::Shape` arm (~590) computes
  `dx = (cursor.x - press.x)/zoom`, `dy = …`, then writes
  `matrix[4] = start_e + dx` / `matrix[5] = start_f + dy` — no snap, no axis-lock.
- **Marquee reference** (`tool_handlers.rs`): `marquee_start: Option<egui::Pos2>`
  is set on empty-space drag, the rect is drawn as a translucent accent
  rectangle each frame (~947), and on `drag_stopped` (~805) nodes whose bounds
  fall in the canvas-space rect are selected. We mirror this for anchors.
- `path_anchor_points(bez)` (`geometry.rs:210`) gives `(element_idx, local Point)`
  for every anchor; `node.transform.apply` + `view.canvas_to_screen` maps each to
  screen space for rect containment — the exact mapping the anchor-square overlay
  already uses (~739).

## Scope

### In
- New `point_marquee_start: Option<egui::Pos2>` field on `PhotonicApp` (init to
  `None`; cleared by `clear_point_edit` / `invalidate_point_edit` / Escape).
- **Marquee vertex select:** in the empty-canvas body branch of `drag_started`,
  if a `point_edit_node` is set, begin a marquee (store screen press pos) instead
  of deselecting. Each frame while dragging with an active marquee, draw the
  translucent rect. On `drag_stopped`, select every anchor of the edit node whose
  screen position lies inside the rect (Shift/Ctrl = additive, else replace); a
  zero-area click-through still falls back to deselect.
- **Multi-node keep-drag:** in the anchor-hit branch, if `!add_sel` and the hit
  anchor is *already* in `point_selected`, keep the existing multi-selection
  (only replace with a single index when pressing an unselected anchor). This
  lets a press-drag on a member of the selection move the whole set via the
  existing `DirectDrag::Anchors` + `bez_move_anchors` path.
- **Shape-drag snap/axis-lock:** in the `DirectDrag::Shape` arm, when Shift is
  held apply `axis_lock_8(dx, dy)` to the canvas delta; otherwise grid-snap the
  resulting translation target with `self.snap(start_e + dx)` /
  `self.snap(start_f + dy)` (no-op when grid snap is off), mirroring the Move
  tool's precedence (axis-lock beats grid snap).

### Out / deferred
- Cross-object marquee that selects anchors across *multiple* paths at once
  (`point_edit_node` is single-node; keep marquee scoped to the current edit
  path). Multi-path point editing is a larger model change.
- Object-aware / smart-guide snapping (`snap::resolve_snap`, #66) on the shape
  drag — grid + axis-lock only here.
- Marquee-driven handle selection or box-select of Live-Corner widgets.

## Approach

**1. State (`app/mod.rs`).** Add `point_marquee_start: Option<egui::Pos2>` near
`marquee_start` / the other `point_*` fields; initialize `None` in the struct
literal (~918 area). Reset it in `clear_point_edit`, `invalidate_point_edit`, and
the Escape block in `handle_direct_select_tool`.

**2. Marquee begin (`direct_select.rs`, empty-canvas branch ~490).** Replace the
unconditional clear with: if `self.point_edit_node.is_some()`, set
`self.point_marquee_start = Some(press_pos)` and leave `point_drag_mode = None`
(marquee is tracked separately from `DirectDrag`); else keep the existing
deselect.

**3. Marquee complete (new block near drag_stopped ~613).** On
`drag_stopped_by(Primary)`, `take()` `point_marquee_start`; compute the
canvas-space rect from press→release, gather `path_anchor_points` of the edit
node, map each to screen via `node.transform.apply` + `view.canvas_to_screen`,
and collect indices inside the rect. `add_sel` unions with the current
selection, else replaces. This block runs before/independent of the
`point_drag_origin` history block (marquee changes no geometry, records no undo).

**4. Marquee overlay (visual section ~694).** If `point_marquee_start` is set,
draw `egui::Rect::from_two_pos(start, hover)` with the same translucent accent
fill + stroke used in `tool_handlers.rs` (~947).

**5. Multi-node keep (anchor-hit branch ~447).**
```rust
} else if let Some(anchor_idx) = anchor_hit {
    if add_sel {
        if !self.point_selected.contains(&anchor_idx) {
            self.point_selected.push(anchor_idx);
        }
    } else if !self.point_selected.contains(&anchor_idx) {
        // Only collapse to a single anchor when grabbing an UNselected one;
        // grabbing a member of the current multi-selection keeps it so the
        // drag moves every selected anchor together (#181).
        self.point_selected = vec![anchor_idx];
    }
    self.point_drag_mode = Some(DirectDrag::Anchors);
    …
}
```

**6. Shape-drag snap/axis-lock (`DirectDrag::Shape` arm ~590).**
```rust
let (dx, dy) = if ui.input(|i| i.modifiers.shift) {
    axis_lock_8(raw_dx, raw_dy)
} else {
    (raw_dx, raw_dy)
};
let (mut te, mut tf) = (start_e + dx, start_f + dy);
if !ui.input(|i| i.modifiers.shift) {
    te = self.snap(te);
    tf = self.snap(tf);
}
node.transform.matrix[4] = te;
node.transform.matrix[5] = tf;
```
(`self.snap` is a no-op unless `prefs.snap_to_grid`.) Drag-end history already
records the transform change (#164 `drag_stopped` compares `transform.matrix`).

## Verification
- `cargo build --release` and `cargo check --workspace` pass; `cargo test -p
  photonic-gui` passes.
- Manual (Joseph, GUI): Direct Select a path → box-drag empty canvas selects the
  enclosed anchors (Shift adds); grab one of several selected anchors and drag →
  all move; fill-drag with Shift locks to 8 directions and with grid snap on
  lands on grid. No MCP tools touched → `docs/mcp-api.md` unaffected.

## Fix round 1 (adversarial review)

**[major] Shape-drag grid snap quantized the raw translation instead of
aligning the shape to the grid.** The `DirectDrag::Shape` arm applied
`self.snap()` to the absolute translation target (`start_e + dx`). Because
freshly-created paths use `Transform::IDENTITY` (`matrix[4]=matrix[5]=0`) with
geometry baked into `path_data` in absolute canvas coordinates, `start_e` is
usually `0`, so `self.snap(0 + dx)` merely quantized the drag *distance* to grid
multiples. A shape at canvas `x=137` would step 137 → 157 → 177… — grid-sized
steps whose edges never land on grid lines. This contradicted the arm's own
"Mirrors the Move tool's precedence" comment; the Move tool snaps a canvas-space
reference point (bbox top-left) via `self.snap(rx + raw_dx) - rx`.

Fix: mirror the Move tool exactly. `DirectDrag::Shape` now carries
`ref_pt: Option<(f64,f64)>` — the node's bbox top-left in canvas space, captured
at press via `selection_canvas_bounds(doc, &[nid], renderer)`. The non-shift
drag branch computes the snapped delta from the reference point
(`self.snap(rx + raw_dx) - rx`, `self.snap(ry + raw_dy) - ry`) and adds it to
`start_e`/`start_f`, so the shape's edge aligns to the grid regardless of the
transform's translation origin. When bounds are unavailable it falls back to the
raw target. Axis-lock (Shift) was already correct and is unchanged.
`crates/photonic-gui/src/app/mod.rs` (`DirectDrag::Shape` variant) and
`crates/photonic-gui/src/app/direct_select.rs` (press capture + drag arm).

Deferred (secondary, as flagged): object-aware snapping (`snap_to_objects`) on
shape drag is still absent here vs. the Move tool. Out of scope for this
grid-alignment fix; can follow in a later pass.

- `cargo build --release` passes; `cargo test -p photonic-gui` passes.
