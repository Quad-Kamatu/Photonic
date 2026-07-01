# 192 — Global shortcuts fire regardless of active tool

> **Status: Implemented.** The tool-independent shortcuts are hoisted out of
> `handle_select_tool` into `PhotonicApp::handle_global_shortcuts`, dispatched
> unconditionally each frame before per-tool handling. Verified with
> `cargo build --release`, `cargo test -p photonic-gui`, and
> `cargo check --workspace` (all pass). See "What this PR implements" below.

## What this PR implements
- New `PhotonicApp::handle_global_shortcuts(&mut self, ctx: &egui::Context, doc, history) -> bool`
  in `crates/photonic-gui/src/app/tool_handlers.rs`. It contains every
  tool-independent shortcut, rewritten from `ui.ctx()` / `ui.input(...)` to
  `ctx` / `ctx.input(...)`:
  - Undo / redo (`edit.undo`, `edit.redo`)
  - Copy (Ctrl+C), paste (Ctrl+V, +10px) and paste-in-place (Ctrl+Shift+V)
  - Duplicate (`edit.duplicate`), select-all (`selection.select_all`),
    deselect (`selection.deselect`)
  - Flip H / V (`object.flip_horizontal`, `object.flip_vertical`)
  - Group (Ctrl+G, 2+ selected) and ungroup (Ctrl+Shift+G on a group)
  - Z-order Ctrl+] / Ctrl+[ (Shift for bring-to-front / send-to-back)
  - View-preview toggles (`view.outline_mode`, `view.pixel_preview`,
    `view.overprint_preview`) and guide toggle (`view.toggle_guides`)
- The whole handler is guarded by `viewport_kb(ctx)` so a focused text widget
  still swallows the keys — typing is unaffected.
- Called unconditionally in the frame loop at
  `crates/photonic-gui/src/app/mod.rs` immediately after the command-palette
  dispatch (before per-tool dispatch), with its `bool` return folded into the
  existing `doc_modified` flag.
- `handle_select_tool` keeps only genuinely Select-specific behavior:
  Delete/Backspace of the live selection (still short-circuits with `return`),
  the Escape-exits-isolation branch, double-click group isolation, and
  drag-to-move/resize. All hoisted lines were removed from it.

## Remaining work
None for this issue — the fix is complete and end-to-end wired. No MCP tools
were touched, so `docs/mcp-api.md` needs no regeneration.

## Summary
The tool-independent keyboard shortcuts (undo, redo, copy, paste, paste-in-place,
duplicate, select-all, deselect, flip H/V, group, ungroup, z-order, view-preview
toggles, guide toggle) are trapped inside `PhotonicApp::handle_select_tool`, which is
only invoked when `active_tool == Tool::Select`. With any other tool active (Scissors,
Pen, Knife, Eraser, MagicWand, Lasso, Pencil, Smooth, Width, Text, Direct Select…),
none of those shortcuts fire — Ctrl+Z appears dead. This is the confirmed mechanism
behind #191.

## Root cause
- `crates/photonic-gui/src/app/mod.rs:4983` — `handle_select_tool` is called only under
  `if self.active_tool == Tool::Select`, and every other tool branch `return`s before
  reaching it.
- `crates/photonic-gui/src/app/tool_handlers.rs:85-316` — the entire `viewport_kb` guarded
  shortcut block lives inside that handler. `binding_pressed("edit.undo")` and
  `dispatch_command` appear nowhere else in the frame loop (only the command palette
  offers an alternate route).

## Scope

### In
- Extract the tool-independent shortcuts from `handle_select_tool` into a new
  `PhotonicApp::handle_global_shortcuts(&mut self, ctx, doc, history) -> bool` (returns
  whether the doc was modified).
- Call it unconditionally in the frame loop next to the command palette dispatch
  (`crates/photonic-gui/src/app/mod.rs:1990`), before per-tool dispatch, folding its
  return into `doc_modified`.
- Guard the new handler with the same `viewport_kb(ctx)` check so typing in text fields
  is unaffected.

### Out
- No new commands, keymap entries, or preferences.
- Delete/Backspace and any Select-specific nudge stay in `handle_select_tool` (they act
  on the Select tool's live selection UI). Group/ungroup/z-order operate on
  `doc.selection` and are safe to run globally, so they move to the global handler.
- Double-click isolation, drag-to-move/resize logic (`tool_handlers.rs:318+`) stay in
  `handle_select_tool` — genuinely Select-specific.

## Approach
1. Add `handle_global_shortcuts` in `tool_handlers.rs`. It takes `ctx: &egui::Context`
   instead of `&egui::Ui` (all current call sites use `ui.ctx()` for `binding_pressed`
   and `ui.input(...)`, both trivially rewritten to `ctx` / `ctx.input(...)`). Move into it:
   - Ctrl+C copy, Ctrl+V / Ctrl+Shift+V paste (tool_handlers.rs:222-271)
   - flip H/V (275-284), undo/redo (287-296), select-all/deselect/duplicate (301-315)
   - Ctrl+G group (189-198), Ctrl+Shift+G ungroup + Ctrl+]/[ z-order (129-186)
   - view.outline_mode / pixel_preview / overprint_preview / toggle_guides (203-220)

   These are the same helpers already in scope on `self`: `binding_pressed`,
   `dispatch_command`, `flip_selection(doc, history, bool)`,
   `do_group_selected(doc, history, &mut modified)`, `gui_clipboard`, `doc.selection`.
   Return a `bool doc_modified` accumulated across the block.
2. In `handle_select_tool`, keep only the Delete/Backspace-of-selection branch and the
   `!isolated`/double-click/drag interaction that follows; remove the hoisted lines.
3. In `mod.rs` update loop, after (or before) the command-palette block at ~1990 add:
   `if self.handle_global_shortcuts(ctx, doc, history) { doc_modified = true; }`.
   Place it before tool dispatch so a shortcut applies the same frame.
4. `cargo build --release`, then smoke-test: activate Scissors, edit, Ctrl+Z undoes;
   Ctrl+C/Ctrl+V works under a non-Select tool; typing in a text field still ignores
   the shortcuts (viewport_kb guard).

## Related
#191 (Ctrl+Z non-functional), #190 (EPIC history), #69 (customizable shortcuts).
