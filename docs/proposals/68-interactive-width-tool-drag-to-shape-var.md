# Interactive Width Tool (#68) — Design Proposal

> Status: implemented (editing MVP). Variable-width *rendering* polish is tracked
> as Remaining work below.

## What this PR implements

A real, on-canvas `Tool::Width` that makes `WidthProfile` data editable directly
on a path's stroke, with full undo. Mapped to the acceptance criteria:

- **`Tool::Width` in the toolbox under "Path Editing"** — added to the `Tool`
  enum (`tools/mod.rs`) with all four exhaustive matches (`label`, `icon`
  = `ph::ARROWS_VERTICAL`, `description`, `is_shape_creator`), the global-search
  catalog, and the "Path Editing" toolbar group.
- **Hovering a stroke shows width handles at existing width points** — the tool
  hit-tests the nearest visible `Path` node (sampled arc-length, like Scissors)
  and draws two diamond handles per profile sample: top = left side, bottom =
  right side. The handle offsets reflect each sample's half-width at the current
  zoom; the connecting bar shows total width.
- **Clicking an empty stroke position inserts a new width point** — the click's
  normalized arc-length `t` is used to insert a sample; the new width (and
  right-side width, if asymmetric) is interpolated from the existing profile.
  The first click on a stroke with *no* profile lazily creates a uniform 2-point
  profile from the current stroke width and attaches it to the stroke
  (`stroke.width_profile_id`).
- **Dragging a handle changes the width; Alt+drag changes one side only** —
  canvas delta-y maps to a half-width change (clamped ≥ 0). Without Alt the drag
  is symmetric (both sides mirror). With Alt only the dragged side changes,
  populating `widths_right` to make the profile asymmetric while keeping
  `widths[i]` as the total width (so the renderer and `average_width()` stay
  correct).
- **Delete key removes the selected width handle** — removes the selected sample
  (and its position / right-side entry), keeping a minimum of two samples.
- **The edited profile can be saved with a name and shows in the panel dropdown**
  — created profiles are real entries in `Document::width_profiles` and appear
  immediately in the Width Profiles panel (now showing point count + average).
  A new inline rename control (`PanelAction::RenameWidthProfile`) renames a
  profile to the text in the panel's name field, also undoable.

### Data model & undo (genuine, not stubbed)

- `WidthProfile` (`photonic-core/src/document.rs`) gained
  `positions: Vec<f64>` (normalized arc-length, `#[serde(default)]`) so samples
  have explicit `t` positions, and `widths_right: Option<Vec<f64>>`
  (`#[serde(default)]`, `None` = symmetric) for asymmetric profiles. Back-compat
  is covered by tests: legacy JSON without these fields deserializes and falls
  back to evenly spaced positions via `effective_positions()`.
- New `Command::SetWidthProfiles { old, new }` (`history.rs`) snapshots the whole
  (small) profile list for self-contained undo/redo; profile creation that also
  attaches an id to a stroke is committed as a `Batch([UpdateNode, SetWidthProfiles])`.

### Files changed / created

- `crates/photonic-core/src/document.rs` — `WidthProfile` fields + `with_positions`,
  `uniform_positions`, `effective_positions`; unit tests.
- `crates/photonic-core/src/history.rs` — `Command::SetWidthProfiles` (description,
  apply, inverse).
- `crates/photonic-gui/src/tools/mod.rs` — `Tool::Width` + four matches.
- `crates/photonic-gui/src/global_search.rs` — search keywords for the new tool.
- `crates/photonic-gui/src/app/mod.rs` — width-tool state fields + `Default`,
  `mod width_tool;`, toolbar group entry, dispatch, `RenameWidthProfile` handler.
- `crates/photonic-gui/src/app/width_tool.rs` — **new** — `handle_width_tool` and
  helpers (hover hit-test, handle positions, drag math, insert/create/delete,
  overlay painting).
- `crates/photonic-gui/src/panels/mod.rs` — `PanelAction::RenameWidthProfile`,
  rename button + point-count display in the Width Profiles panel.

## Remaining work

- **Variable-width rendering fidelity (M2).** The tessellator already modulates
  stroke width from a profile's `widths` *assuming uniform sample spacing*; it
  does **not yet honor non-uniform `positions` or asymmetric `widths_right`**.
  Profiles created by this tool with non-uniform positions or per-side widths
  will render using uniform spacing / total width as an approximation. Wiring
  `positions` and `widths_right` through `tessellator::sample_width_profile`
  (and the renderer's profile lookup) is the rendering follow-up.
- **Perpendicular handle orientation.** Handles are offset vertically in screen
  space rather than along the local path normal. A tangent-based offset would
  match Illustrator's tool more closely on steep/curved segments.
- **Live tablet pressure** and **width profiles on text-on-path** remain out of
  scope.

---

## Summary

`WidthProfile` (`photonic-core/src/document.rs:256`) and its storage in `Document::width_profiles` (`document.rs:498`) are already modelled. The current UI lets users select a profile from a panel dropdown but offers no way to interactively drag a point on a stroke to shape its width. This issue adds a `Tool::Width` that makes `WidthProfile` data editable directly on the canvas.

## Scope

**In**
- `Tool::Width` variant: hover a `SceneNodeKind::Path` stroke to show ghosted width handles at existing `WidthProfile::widths` sample positions
- Click an empty position on the stroke to insert a new width point; drag to set its width (symmetric by default)
- Alt+drag: set asymmetric per-side widths (stored as two values per point)
- Delete key on a selected width handle removes that point
- Save the result as a named `WidthProfile` via a small panel field
- Live canvas preview of variable-width stroke during drag (depends on M2 variable-width rendering)

**Out**
- M2 variable-width rendering itself (this issue is the editing tool; rendering is tracked separately)
- Pressure-sensitive width from a drawing tablet
- Width profiles applied to text on a path

## Proposed Approach

1. **`WidthProfile` data model review** (`document.rs:256-281`): current fields are `pub name: String` and `pub widths: Vec<f64>`. For asymmetric per-side widths, the model needs either a `Vec<(f64, f64)>` or a separate `widths_left: Vec<f64>` / `widths_right: Vec<f64>`. Propose adding `pub widths_right: Option<Vec<f64>>` (None = symmetric). This is a `photonic-core` data change and requires a `Command` to update it.

2. **New `Command::UpdateWidthProfiles`** (`history.rs`): stores `old: Vec<WidthProfile>` and `new: Vec<WidthProfile>` so the whole profile list is snapshotted (profiles are small). Alternative: reuse `Command::Batch` wrapping a document-level property update — decide during implementation.

3. **`Tool::Width` variant** (`tools/mod.rs`): add to the `Tool` enum with label `"Width"` and icon `ph::WAVE_SINE` (distinct from Smooth). Added to the "Path Editing" tool group in `app.rs:1217`.

4. **Width tool state in `App`**:
   - `width_tool_hovered_node: Option<NodeId>` — the path the cursor is over
   - `width_tool_hovered_t: f64` — normalized arc-length position [0, 1] on the hovered path
   - `width_tool_selected_point: Option<usize>` — index into `widths` of the active handle
   - `width_tool_drag_origin: Option<f64>` — canvas y at drag start for width calculation

5. **Hit testing**: on hover, find the nearest `SceneNodeKind::Path` node within a screen-space tolerance (reuse the sample-point approach from the Scissors tool at `app.rs:2260`). Map the nearest sample point to a normalized `t` position.

6. **Handle rendering**: for the hovered/selected node, render diamond handles at each `widths` sample position. The handle's half-height on screen reflects the current width value at that t. Use the `Painter` in the canvas paint pass.

7. **Drag behavior**: on `primary_clicked` at a handle, set `width_tool_selected_point`. On `dragged`, delta-y in canvas space adjusts the width at that index (clamped to ≥ 0). Without Alt: symmetric. With Alt: only the side corresponding to the drag direction.

8. **Commit**: on `drag_released`, push `Command::UpdateWidthProfiles` (or equivalent) to `CommandHistory`. The node's `stroke` style should carry a reference to the profile by index/name — ensure `StrokeStyle` has a `width_profile: Option<String>` field (add to `photonic-core/src/style.rs` if absent).

## Affected Modules

- `crates/photonic-core/src/document.rs` — `WidthProfile`: add `widths_right: Option<Vec<f64>>`; `Document::width_profiles` (already exists at line 498)
- `crates/photonic-core/src/style.rs` — `Stroke` or `StrokeStyle`: add `width_profile: Option<String>` to link a node's stroke to a named profile
- `crates/photonic-core/src/history.rs` — new `Command` variant for profile updates (or reuse Batch)
- `crates/photonic-gui/src/tools/mod.rs` — `Tool::Width` variant
- `crates/photonic-gui/src/app.rs` — width tool state fields, hover/drag dispatch, handle rendering
- `crates/photonic-render/` — M2 variable-width rendering (prerequisite; tracked separately)

## Risks & Open Questions

- **M2 dependency**: without variable-width rendering, the width handles can be edited but the stroke preview will appear uniform. The tool can ship with a "flat" preview and the rendering catch-up later, but the AC below marks real rendering as required.
- **`widths: Vec<f64>` length**: what is the implicit parameterization? Equal arc-length segments? Explicit t values? The current struct stores only widths without t positions — this makes interpolation ambiguous. Recommend adding `pub positions: Vec<f64>` (normalized arc-length [0,1]) alongside `widths` before implementing the tool.
- **Profile naming and assignment**: the Width Profile must be assigned to a specific path's stroke. Confirm `StrokeStyle` (or `Stroke`) in `photonic-core/src/style.rs` has a linkage field; add one if missing.

## Acceptance Criteria

- [x] `Tool::Width` appears in the toolbox under "Path Editing"
- [x] Hovering a stroke shows width handles at existing width points
- [x] Clicking an empty stroke position inserts a new width point
- [x] Dragging a handle changes the width; Alt+drag changes one side only
- [x] Delete key removes the selected width handle
- [x] The edited profile can be saved with a name and shows in the panel dropdown
- [ ] The stroke renders with true variable width (requires M2 rendering — see Remaining work)

## Effort Estimate

**L** — depends on M2 rendering; the data-model changes (position parameterization, asymmetric widths) and the `Command` infrastructure add scope beyond pure UI work.
