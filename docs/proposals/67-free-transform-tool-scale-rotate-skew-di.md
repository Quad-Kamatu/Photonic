# Free Transform Tool (#67) — Design Proposal

> Status: design scaffold (not an implementation).

## Summary

Scale and rotate handles already exist in the Select tool (`app.rs:~9438–9627`). Shear is wired through the Transform panel (`PanelAction::ShearNode`, `app.rs:~4137`). This issue unifies those capabilities into a dedicated **Free Transform tool** with on-canvas modifier-driven sub-modes (scale / rotate / skew / free-distort / perspective), a selectable 9-point reference origin, and a single undoable `Command::Batch` per transform gesture.

## Scope

**In**
- New `Tool::FreeTransform` variant with its own cursor + handle set
- Sub-modes activated by keyboard modifier during drag:
  - Default: proportional scale from selected reference point
  - Shift: unconstrained scale
  - Ctrl (side handle): skew along that axis (maps to `transform_ops::shear`)
  - Alt: scale from center regardless of reference point setting
  - (Planned) free-distort: each corner handle moves independently (affine decomposition; result is stored as a general 2×3 matrix)
  - (Planned) perspective distort: homographic warp — requires M3 render support; out of M1 scope
- 9-point reference-point selector rendered in the tool options bar (same visual as Illustrator's proxy widget); stored in `App`
- Transform preview is live (no-commit); single `Command::UpdateNode` (or `Command::Batch` for multi-selection) committed on pointer release
- Rotation handle above the bounding box (same as existing Select rotate handle)

**Out**
- Perspective distort (needs non-affine render path — M3)
- Envelope / warp mesh
- Transform each object separately vs. as a group (first pass: treat selection as a unit)

## Proposed Approach

1. **`Tool` enum** (`crates/photonic-gui/src/tools/mod.rs`): add `FreeTransform` variant with label `"Free Transform"` and a suitable phosphor icon (`ph::FRAME_CORNERS` or similar).

2. **Reference-point state in `App`**: `free_transform_origin: ReferencePoint` (9-variant enum: TL, TC, TR, ML, MC, MR, BL, BC, BR; default MC). Add `free_transform_origin_world: Option<(f64, f64)>` — the canvas-space anchor computed from the selection bbox at drag start.

3. **Handle layout**: reuse the existing 4-corner + 4-midpoint handle math from the Select tool (`ResizeHandle` enum). For Free Transform, mid-side handles activate skew (Ctrl modifier); corner handles activate scale; a separate rotation arc handle sits above the top-center.

4. **Sub-mode dispatch in `handle_free_transform`**: read `ui.input(|i| i.modifiers)` on each `drag` event. Dispatch to:
   - Scale: `Transform::scale_around(sx, sy, anchor_x, anchor_y)` (same as existing resize path)
   - Skew: `transform_ops::shear(&mut node, shear_x, shear_y, cx, cy)` + `Transform::shear_around`
   - Rotate: `Transform::rotate_around(angle, cx, cy)`

5. **Live preview**: mutate `node.transform` in-place during drag (same as existing `self.resizing` path). On `response.drag_released()`, compute `Command::UpdateNode { old: drag_origin_node, new: current_node }` and `history.execute(cmd, doc)`.

6. **Multi-selection**: same `resize_multi_origins: Vec<(NodeId, [f64; 6])>` mechanism already used by the Select tool; adapt it for the full matrix set.

7. **Tool options bar**: add a 9-button grid for `free_transform_origin` in the existing `draw_tools_panel` or a dedicated options strip at the top of the canvas panel.

## Affected Modules

- `crates/photonic-gui/src/tools/mod.rs` — `Tool::FreeTransform` variant + label/icon
- `crates/photonic-gui/src/app.rs` — `App` struct fields (`free_transform_origin`, `free_transform_origin_world`); new `handle_free_transform()` method; routing in the main event loop
- `crates/photonic-core/src/ops/transform_ops.rs` — `shear` already exists; `scale_around` / `rotate_around` on `Transform` already exist in `transform.rs`
- `crates/photonic-core/src/history.rs` — `Command::UpdateNode` / `Command::Batch` (already exist)
- `crates/photonic-gui/src/panels/` — tool options bar for reference-point selector

## Risks & Open Questions

- **Overlap with Select tool handles**: the Select tool already handles resize + rotation. Free Transform must not conflict when both are available. Recommend: Free Transform shows its handles only when `active_tool == Tool::FreeTransform`; Select tool keeps its existing lightweight handles.
- **Free-distort (corner-independent drag)**: each corner moves one vertex of the bounding quad independently. The resulting transform is still affine only if opposite corners move coherently; otherwise you need a perspective matrix. Defer to M3 or limit M1 to affine skew only.
- **Undo granularity**: do not push an `UpdateNode` on every drag frame — capture the node state at drag-start and commit once on release.
- **shear_x/shear_y fields in App** (`app.rs:289-290`): currently used for the panel's Shear button. Free Transform's canvas skew should mirror these values if the panel is visible, or at minimum keep them in sync on commit.

## Acceptance Criteria

- [ ] `Tool::FreeTransform` appears in the toolbox and is selectable
- [ ] Corner handles scale; Shift disables proportional lock; reference point changes the anchor
- [ ] Ctrl on a side handle activates skew along that axis with a live preview
- [ ] Rotation handle rotates around the reference point
- [ ] Entire gesture produces a single undo step
- [ ] Multi-selection transforms as a unit

## Effort Estimate

**M** — most primitives (transform math, handle rendering, undo) exist; new work is the sub-mode dispatch logic, reference-point widget, and Free Transform state management.
