# On-canvas gradient editor (drag stops, angle, and handles directly on the object) (#64) — Design Proposal

> Status: design scaffold (not an implementation).

## Summary

Gradient editing currently lives entirely in the Properties panel (`draw_fill_editor()` in
`crates/photonic-gui/src/panels/mod.rs:5669`). Users can set stop colors and positions
through sliders, but there is no way to drag the gradient axis or reposition stops directly
on the artwork. This proposal adds `Tool::Gradient` to `crates/photonic-gui/src/tools/mod.rs`
and the associated on-canvas overlay and interaction logic.

## Scope

**In:**
- A `Tool::Gradient` tool variant that activates when a node with a gradient fill is selected.
- On-canvas handles for linear and radial gradient axes drawn via `egui::Painter` using `view.canvas_to_screen()` (the coordinate transform already used throughout `app.rs`).
- Drag the start/end handles of a linear gradient to reposition `Gradient::coords[0..=3]` in `crates/photonic-core/src/style.rs`.
- Drag the center + radius handle of a radial gradient (`coords[0,1]` and `coords[4]`).
- On-canvas stop markers along the gradient line: click to select, drag to slide `GradientStop::position`, click the color chip to open the existing color picker.
- Add / delete stops via double-click / Delete key.
- Live update during drag (mutate `after` in a transient state; commit one `Command::UpdateNode` on release).
- Fluid gradient (`FluidGradient`): drag `FluidGradientPoint` control points on canvas.

**Out:**
- Mesh gradient on-canvas editing (separate, complex enough to be its own issue).
- Changing gradient kind (linear → radial) on canvas (use the Properties panel).
- Per-stop easing / midpoints for v1 (add midpoint handles in v2).

## Proposed approach

### 1. Add `Tool::Gradient` variant

In `crates/photonic-gui/src/tools/mod.rs`:
```rust
pub enum Tool {
    // … existing variants …
    Gradient,
}
```
With icon `ph::GRADIENT` (phosphor icons already imported) and label `"Gradient"`.

### 2. Tool state struct

In `crates/photonic-gui/src/app.rs`, add:
```rust
struct GradientToolState {
    dragging: Option<GradientHandle>,
    node_id: NodeId,
    fill_before: Fill,   // snapshot for undo
}

enum GradientHandle {
    LinearStart,
    LinearEnd,
    RadialCenter,
    RadialRadius,
    Stop(usize),
    FluidPoint(usize),
}
```

### 3. Hit testing handles

On each frame when `Tool::Gradient` is active and a single node is selected:
- Retrieve the node's `Fill` from `Document::get_node()` in `crates/photonic-core/src/document.rs`.
- Convert gradient `coords` from document space to screen space via `view.canvas_to_screen(x, y)`.
- Render: endpoint handles as filled circles (8 px radius); stop markers as diamonds along the gradient line segment.
- On pointer-down, check pointer distance against each handle to identify which is hit (pick closest within 10 px threshold).

### 4. Drag interaction

Pointer move while dragging:
- Convert screen delta back to document space via the inverse of `view.canvas_to_screen()`.
- Mutate the in-progress `Fill` copy stored in `GradientToolState`.
- Write the mutated fill directly to the document's node for live preview (bypassing history).

On pointer-up:
- Push `Command::UpdateNode { id, before: fill_before_as_node, after: current_node }` to `CommandHistory` in `crates/photonic-core/src/history.rs`.
- This gives a single, undoable step per drag gesture.

### 5. Stop add/delete

- Double-click on the gradient line: compute `t` at click position along the axis, call `Gradient::stops.insert()` with interpolated color.
- Select a stop handle + `Delete` key: remove from `stops` vec.
- Each mutation wraps in `Command::UpdateNode`.

### 6. Color picker integration

Clicking a stop handle in the `Gradient` tool should open the same color picker egui widget used in `draw_fill_editor()` (`panels/mod.rs:5919`, `FillColorSlot::GradientStop(i)`). The cleanest way is to push a `PanelAction::OpenColorPicker { slot: FillColorSlot::GradientStop(i) }` and let the existing panel handle it.

### 7. Fluid gradient on-canvas

For `FillKind::FluidGradient(fg)`, render `FluidGradientPoint` instances (which have `.x`, `.y` fields in world space) as draggable handles. Drag → update `fg.points[i].x/y`. No special hit-test on a line; every point is an independent handle.

## Affected modules

- `crates/photonic-gui/src/tools/mod.rs` — add `Tool::Gradient` variant + label/icon
- `crates/photonic-gui/src/app.rs` — `GradientToolState`, `GradientHandle`; pointer event dispatch; handle render in the viewport painter block; coordinate conversion using `view.canvas_to_screen()`
- `crates/photonic-core/src/history.rs` — `Command::UpdateNode` (already exists; no changes needed)
- `crates/photonic-core/src/style.rs` — read/write `Gradient::coords`, `GradientStop::position`, `FluidGradientPoint::x/y` (data already there)
- `crates/photonic-gui/src/panels/mod.rs` — keep `draw_fill_editor()` as the companion panel; add `PanelAction::OpenColorPicker` variant if not already present

## Risks & open questions

- **Coordinate inversion**: `view.canvas_to_screen()` is used throughout `app.rs` but its inverse is not yet a named method. Need `view.screen_to_canvas(sx, sy) -> (f64, f64)` — a two-line addition to `crates/photonic-gui/src/viewport.rs` (or inlined at the call site).
- **Handle overlap**: When stops are close together, their handles overlap. Use a minimum pixel separation and a selection ring to distinguish.
- **Fluid gradient UX**: Dragging overlapping fluid control points is ambiguous. A "click to select, then drag" model (like bezier node editing) is safer than immediate drag-from-hover.
- Open Q: Should `Tool::Gradient` activate automatically when the selected node has a gradient fill, or only when the user clicks the Gradient tool button?
- Open Q: Where does the gradient axis angle readout live — tooltip, status bar, or Properties panel? The Properties panel already shows coordinates.

## Acceptance criteria

- [ ] With a node selected and `Tool::Gradient` active, on-canvas handles appear over the gradient axis.
- [ ] Dragging the start/end handle of a linear gradient repositions the axis in real time with live preview.
- [ ] Dragging a stop marker slides it along the gradient axis.
- [ ] Double-click on the gradient line adds a stop; Delete removes the selected stop.
- [ ] Clicking a stop handle opens the color picker and the stop color updates live.
- [ ] All axis-drag and stop-drag operations are undoable with a single Ctrl+Z.
- [ ] Fluid gradient control points are draggable on canvas.

## Effort estimate

**M** — The data model is already in place (`Gradient`, `GradientStop`, `FluidGradient` in `style.rs`). The work is purely UI: hit-testing, painter overlay, coordinate math, and undo integration.
