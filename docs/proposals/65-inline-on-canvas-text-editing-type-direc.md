# Inline On-Canvas Text Editing (#65) — Design Proposal

> Status: design scaffold (not an implementation).

## Summary

Text objects (`TextNode` in `photonic-core`) are currently created and modified exclusively through the properties panel. This issue adds double-click-to-edit inline mode: a blinking caret appears on canvas, keyboard input modifies `TextNode::content`, click-drag selects character ranges, and range-level formatting is supported.

## Scope

**In**
- Double-click a `SceneNodeKind::Text` node to enter inline-edit mode
- Blinking text caret rendered on canvas in screen-space
- Keyboard input (printable chars, Backspace, Delete, arrow navigation, Home/End)
- Click to position caret; click-drag to select a character range
- Ctrl+A select all; Ctrl+C/X/V copy/cut/paste of plain text
- IME composition support (pass-through via egui's IME events)
- Apply character formatting (bold, italic, font-size, color) to the active selection range
- Press Escape or click outside to commit + push one `Command::UpdateNode` to `CommandHistory`

**Out**
- Multi-style runs (rich `Vec<Run>` model) — `TextNode::content` is still a flat `String`; per-range formatting is deferred to the M4 text layout engine
- Area-type or path-spine editing modes (they still use the panel)
- Paragraph-level reflow while typing (no wrapping box in M1)
- Bi-directional text / complex script shaping

## Proposed Approach

1. **Inline-edit state in `App`** (`crates/photonic-gui/src/app.rs`): add fields `text_edit_node: Option<NodeId>` and `text_cursor: TextCursor` (byte offset + optional selection range). These shadow the node while editing.

2. **Entry point** (`app.rs` ~line 2690 Text tool block): on `pointer_button_double_click` for `Tool::Text` when hit-test returns a `SceneNodeKind::Text` node, set `text_edit_node` and `text_cursor` to the character nearest the click.

3. **Input handling**: inside the same `Tool::Text` branch, when `text_edit_node.is_some()`, consume `egui::Event::Text` / `egui::Event::Key` events (gated behind `!ctx.wants_keyboard_input()` override — the editor IS the text focus). IME: egui already surfaces `Event::CompositionStart/Update/End`; buffer the composition string separately.

4. **Caret/selection rendering**: in the canvas paint pass, if `text_edit_node` is set, use `PhotonicRenderer` (or direct egui `Painter`) to draw a 1px caret line at the glyph x-advance position and a semi-transparent rect over the selection range. Glyph metrics come from the existing `photonic-render` layout path.

5. **Commit**: on Escape / click-outside, build `Command::UpdateNode { old, new }` where `new.kind` carries the edited `TextNode::content` and push to `CommandHistory`.

6. **`Tool` enum** (`crates/photonic-gui/src/tools/mod.rs`): no new variant needed — reuse `Tool::Text`. `Tool::Select` double-click on a text node should also enter edit mode (same code path).

## Affected Modules

- `crates/photonic-gui/src/app.rs` — inline-edit state fields, input dispatch, caret rendering, commit logic
- `crates/photonic-gui/src/tools/mod.rs` — no new variant; possibly a `Tool::Text` sub-mode enum
- `crates/photonic-core/src/node.rs` — `TextNode` (read only; `content: String` is the mutable target)
- `crates/photonic-core/src/history.rs` — `Command::UpdateNode` (already exists; used for commit)
- `crates/photonic-render/` — glyph-metrics query for caret positioning

## Risks & Open Questions

- **Glyph metrics**: `photonic-render` must expose a way to map a byte offset → screen-space x position. If it only rasterises, caret positioning will be approximate until the M4 layout engine lands.
- **IME on Linux**: egui's Wayland backend IME support is partial; need to test with fcitx5/ibus before declaring done.
- **Flat `String` vs future rich runs**: committing edits as a single `UpdateNode` is fine now; migrating to a `Vec<Run>` model later is a breaking `TextNode` schema change — design the edit state with that in mind (keep a cursor as a byte index, not a run-local index).
- **Selection rendering without glyph layout**: may need a best-effort pixel-width estimate until M4.

## Acceptance Criteria

- [ ] Double-clicking a text node (with either Select or Text tool active) places a caret and enters edit mode
- [ ] Typing, Backspace/Delete, and arrow keys work correctly
- [ ] Click-drag produces a visible selection range; Ctrl+C/X/V copy/cut/paste of the selection
- [ ] Escape or out-of-bounds click commits the edit as a single undoable `UpdateNode` command
- [ ] Character formatting applied to a selection (bold, italic, font-size) reflects in `TextNode`
- [ ] IME composition does not corrupt content

## Effort Estimate

**L** — caret rendering, IME, byte-offset ↔ glyph-metric bridge, and the selection/formatting sub-system each carry non-trivial complexity.
