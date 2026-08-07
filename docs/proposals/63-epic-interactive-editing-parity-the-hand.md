# [EPIC] Interactive editing parity — the hands-on editing experience (#63) — Design Proposal

> Status: design scaffold (not an implementation). This is an epic; individual child issues carry implementation details.

## Summary

The MCP/AI surface (260+ tools, `crates/photonic-mcp/src/server.rs`) is mature, but
direct-manipulation editing — the kind a designer uses with mouse and keyboard — lags
behind the class of Illustrator, Affinity Designer, and Inkscape. The GUI already has a
solid base: 22 tool variants in `crates/photonic-gui/src/tools/mod.rs` (Select through
Smooth), a `CommandHistory` with `Command::UpdateNode`/`AddNode`/`GroupNodes`/`Batch` in
`crates/photonic-core/src/history.rs`, grid snapping (`snap_to_grid` field in `app.rs`),
and ruler guides. This epic tracks the interaction gaps that prevent a non-trivial
illustration from being completed without MCP assistance.

## Scope

**In (this milestone M1):**
Issues already filed: #2, #3, #4, #5, #7, #8, #9, #10, #11, #12, #13, #14, #1.
New work scoped under this epic:
- `Tool::Gradient` — on-canvas gradient editor (separate issue #64).
- Inline on-canvas text editing (double-click a `SceneNodeKind::Text` node to edit in place).
- Smart guides / snap-to-object (extends `snap_to_grid` in `app.rs`; geometry-aware, not just grid).
- Free Transform tool (skew, distort, perspective on `PathNode`).
- Interactive Width tool (variable stroke width on `PathNode::stroke`).
- Customizable keyboard shortcuts + command palette.
- Rulers: drag-to-create guides + live measurement overlay.
- Knife, Eraser, Path-Eraser tools.

**Out:**
- MCP tool additions (handled separately in photonic-mcp).
- Rendering quality changes (photonic-render scope).
- Export pipeline.

## Proposed approach (epic-level)

This section describes architectural patterns that apply across child issues.

### History discipline
Every interactive mutation must go through `CommandHistory::execute()` in
`crates/photonic-core/src/history.rs`. The most common oversight (issues #5, #11) is
that drag-based operations bypass `Command::UpdateNode` and mutate the document directly.
The pattern to enforce: on drag-start, snapshot the node's `before` state; on drag-end,
push `Command::UpdateNode { id, before, after }`. Intermediate frames mutate `after` in
place for visual feedback, but only one undo entry is created.

### Tool dispatch architecture
New tools are variants in `crates/photonic-gui/src/tools/mod.rs` (`pub enum Tool`).
Each tool handles:
- `on_pointer_down`, `on_pointer_move`, `on_pointer_up` events dispatched from `app.rs`
- Rendering overlay (handles, guides) via `egui::Painter` in the viewport
- Keyboard modifier state (Shift, Alt, Ctrl) for constrain/clone/snap behaviors

For complex tools (Gradient, Free Transform), introduce per-tool state structs (e.g.
`GradientToolState`, `FreeTransformState`) held in `app.rs` alongside the current
`Tool` enum value.

### Smart guides / snap-to-object
The current `snap_to_grid` in `app.rs` only snaps to the document grid. Smart guides need:
- A pre-pass each frame computing bounding boxes of all `SceneNode`s via `node.bounding_box()` in `crates/photonic-core/src/node.rs`.
- Candidate snap points: center-x, center-y, left, right, top, bottom of each node.
- Visual rendering: a temporary `egui::Painter::line_segment` when snapping is active.
- Threshold in screen pixels (configurable in `AppPreferences`).

### Command palette
A floating `egui::Window` triggered by `Cmd+Shift+P` / `Ctrl+Shift+P` that fuzzy-searches
over all menu actions and tool names. Actions are a flat `Vec<PaletteAction>` built at
startup. Selecting one dispatches via the same code path as the menu.

### Keyboard shortcut customization
Store a `HashMap<egui::Key, Action>` in `AppPreferences` (serialized to `preferences.json`).
Default bindings are constants in `app.rs`; the preference map overrides. A "Keyboard
Shortcuts" dialog iterates the map and lets users rebind.

## Affected modules (epic-level)

- `crates/photonic-gui/src/tools/mod.rs` — new `Tool` variants (Gradient, FreeTTransform, Width, Knife, Eraser)
- `crates/photonic-gui/src/app.rs` — tool dispatch, per-tool state structs, smart-guide pre-pass, palette window, shortcut map
- `crates/photonic-core/src/history.rs` — confirm `Command::UpdateNode` supports all mutation types; possibly add `Command::SetStrokeWidth`
- `crates/photonic-core/src/node.rs` — `SceneNode::bounding_box()` used by snap system
- `crates/photonic-gui/src/preferences.rs` — `AppPreferences`: snap threshold, shortcut map, smart-guide toggle
- `crates/photonic-gui/src/panels/mod.rs` — tool options panels for new tools

## Risks & open questions

- **Scope creep**: The epic is intentionally broad. Each child issue should be scoped to a single interaction; resist bundling multiple behaviors in one PR.
- **History correctness**: The most common regression is a mutation that creates duplicate undo entries or loses undo entirely. All new interactive tools need explicit history tests.
- **Wayland eyedropper** (#8): system color sampling on Wayland requires a portal (`org.freedesktop.portal.Screenshot`); this is a platform-specific async call outside the egui event loop.
- Open Q: What is the "done" state for this milestone — a designer benchmark task (e.g. redraw a reference logo in Photonic without MCP), or a feature checklist?

## Acceptance criteria

- [ ] A designer can complete a non-trivial illustration (multi-shape, grouped, with gradients and text) end-to-end using only mouse + keyboard, no MCP.
- [ ] All new interactive operations are undoable via `Ctrl+Z`.
- [ ] Smart guides appear and snap when an object aligns with another object's edge or center.
- [ ] Command palette (`Ctrl+Shift+P`) lists and executes all menu actions.
- [ ] Keyboard shortcuts are customizable and persist.

## Effort estimate

**XL** — This is a milestone-level epic spanning 15+ child issues. Individual items range from S (aspect-ratio lock, #4) to L (inline text editing, Free Transform).
