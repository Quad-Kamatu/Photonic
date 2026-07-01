# 184 — Select tool: open-hand cursor on hover, closed-hand only while dragging

## Status: Implemented

**What this PR implements:** Two cursor-icon edits in
`crates/photonic-gui/src/app/tool_handlers.rs`, mirroring the Pan tool's
`Grab`/`Grabbing` pattern:

- The `self.moving` branch now uses `egui::CursorIcon::Grabbing` (closed hand)
  — shown only while actively dragging a move.
- The `on_body` hover branch now uses `egui::CursorIcon::Grab` (open hand) —
  shown on pointer-up hover over a selected object's body.

Because `self.moving` is set only on drag-start and cleared on release/finalize,
the two branches cleanly separate the dragging vs. hovering states with no new
state or predicate. Resize-handle cursors and hit-testing are untouched.

**Remaining work:** None. No MCP tools touched, so `docs/mcp-api.md` is unchanged.

## Summary

With the Select tool, hovering (pointer up) over the body of a selected object
shows a **closed-hand** cursor. The closed hand should appear only while actively
dragging the object to move it; on hover it should be an **open hand**.

Root cause: `crates/photonic-gui/src/app/tool_handlers.rs` uses `CursorIcon::Move`
for *both* the mid-move (`self.moving`) state and the on-body hover state, so the
two are indistinguishable. On common cursor themes `Move` renders as a closed
(grabbing) hand, giving the wrong affordance on hover.

The Pan tool already does the correct thing (`app/mod.rs:4910/4912` and
`5126/5128`): `Grabbing` while dragging, `Grab` on hover. We mirror that here.

## Scope

### In
- `tool_handlers.rs` cursor block: change the `self.moving` branch to
  `CursorIcon::Grabbing` (closed hand while dragging) and the `on_body` hover
  branch to `CursorIcon::Grab` (open hand on hover).

### Out
- Resize-handle cursors (`ResizeNwSe`/`ResizeNeSw`) — already correct, untouched.
- Any change to hit-testing, move-recording, or selection logic.
- Cursor behavior for other tools.

## Approach

Two one-line edits in `crates/photonic-gui/src/app/tool_handlers.rs`:

- Line ~967, the `} else if self.moving {` branch:
  `egui::CursorIcon::Move` → `egui::CursorIcon::Grabbing`
- Line ~1020, inside the `if on_body {` hover branch:
  `egui::CursorIcon::Move` → `egui::CursorIcon::Grab`

`self.moving` is only true once a drag has started (set on drag-start, cleared on
release/finalize — lines 489/544/747/863), so the `Grabbing` branch is exactly
the drag state and the `on_body` branch is the pointer-up hover state. No new
state or predicate is needed.

Verify with `cargo build --release` (house rule) and a manual GUI check: select
an object, hover its body (open hand), press and drag (closed hand), release
(open hand again).
