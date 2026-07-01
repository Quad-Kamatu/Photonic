# 191 — Ctrl+Z does not undo edit steps (delete not recorded)

## Status: implemented

**What this PR implements**

- `command_center.rs` `edit.delete` (Delete key + command palette) now builds a
  `Vec<Command::RemoveNode>` from the selection and runs it as one
  `Command::Batch` via `history.execute`, instead of looping `doc.remove_node`.
  The whole multi-select delete is a single undoable step.
- `tool_handlers.rs` Select-tool `Delete`/`Backspace` handler does the same,
  using the in-scope `history` (empty-selection is a no-op, existing `return;`
  preserved).
- `photonic-core` regression tests in `history.rs` drive the exact GUI code path
  (`Command::Batch` of bare `RemoveNode`) for both a single-node and a multi-node
  selection, asserting delete removes the nodes and a single `undo` restores them
  into their original layer: `gui_delete_single_node_batch_undo_restores_node`,
  `gui_delete_multi_node_batch_undo_restores_all`.

`execute` already hydrates each bare `RemoveNode` into the self-contained
`RemoveNodeFull` (history.rs:929), so undo re-adds the node with its original
layer membership.

**Verification:** `cargo build --release` ✓, `cargo test -p photonic-core` ✓
(300 passed), `cargo check --workspace` ✓.

**Remaining work (deferred — see "Out" below):** the rest of the systematic
history sweep (drag-move #183, point-type convert #188, stroke/edit coalescing
#182), the focus-guard investigation, and a full GUI/egui interaction smoke
harness. These are tracked under EPIC #190 and are out of scope for this fix.

## Summary

Pressing **Ctrl+Z** appears to do nothing after many edits. The shortcut and the
`history.undo(doc)` plumbing are correctly wired (`command_center.rs:38`,
`tool_handlers.rs:202`); the real problem is that several edits never push a
history step, so `undo()` returns `false` with nothing to revert.

This proposal fixes the single most impactful, fully self-contained instance:
**delete is not recorded**. Both delete entry points mutate the document
directly and never touch `CommandHistory`, so a deleted object can never be
brought back with Ctrl+Z:

- `command_center.rs:63-72` (`edit.delete`, bound to `Delete` and palette) loops
  `doc.remove_node(&nid)` directly.
- `tool_handlers.rs:31-41` (Select tool `Delete`/`Backspace` key) does the same.

The correct, already-proven pattern lives in the same GUI:
`PanelAction::DeleteSelected` at `mod.rs:7659` runs
`history.execute(Command::Batch(cmds), doc)` where `cmds` is a `Vec<Command::RemoveNode>`.
`execute` hydrates each `RemoveNode` into the self-contained `RemoveNodeFull`
(history.rs:929), so undo re-adds the node. We route the two buggy paths through
the same call.

## Scope

### In
- Rewrite `edit.delete` (`command_center.rs`) to build a `Vec<Command::RemoveNode>`
  from the selection and run it as one `Command::Batch` via `history.execute`, so
  the whole multi-select delete is a single undoable step.
- Rewrite the Select-tool `Delete`/`Backspace` handler (`tool_handlers.rs`) the
  same way (`history` is already a parameter of `handle_select_tool`).
- Add `photonic-core` regression tests in `history.rs`: build a doc with a node,
  `execute(Command::Batch(vec![RemoveNode{..}]))`, assert the node is gone, then
  `undo` and assert the node (and its layer membership) is restored. Cover the
  multi-node batch case too (delete two → single undo restores both).

### Out (deferred to EPIC #190)
- Drag-move not recorded (#183), point-type convert no-op (#188), stroke/edit
  coalescing (#182), and the rest of the systematic-history sweep. These each need
  their own tool-specific command capture and are tracked separately.
- Focus-guard change. `viewport_kb(ctx) = !ctx.wants_keyboard_input()`
  (`mod.rs:12402`) is working as designed — it correctly yields the keyboard to
  live text fields. No concrete off-canvas-field leak was found; investigating a
  hypothetical one is deferred until reproduced.
- GUI/egui end-to-end smoke test (the "each tool" harness in the issue). Core-level
  undo tests are the tractable, deterministic slice; a full interaction harness is
  infrastructure-scale.

## Approach

1. `command_center.rs` `edit.delete` arm:
   ```rust
   let ids: Vec<NodeId> = doc.selection.ids().copied().collect();
   if !ids.is_empty() {
       let cmds = ids.iter().map(|&id| Command::RemoveNode { node_id: id }).collect();
       history.execute(Command::Batch(cmds), doc);
       doc.selection.clear();
       self.selected_id = None;
       modified = true;
   }
   ```
2. `tool_handlers.rs` Delete/Backspace block: same substitution (uses the
   in-scope `history`), keeping the existing `return;`.
3. Add tests to the `tests` module in `history.rs` following the existing
   `undo_removes_node` style, but driving through `Command::Batch`.
4. `cargo build --release` + `cargo test -p photonic-core` must pass.

## Files to touch
- `crates/photonic-gui/src/app/command_center.rs`
- `crates/photonic-gui/src/app/tool_handlers.rs`
- `crates/photonic-core/src/history.rs` (tests)
