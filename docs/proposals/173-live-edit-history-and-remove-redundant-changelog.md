# Edit History Live Update + Remove Redundant Right-Side Change Log (#173)

> Status: **implemented**. Two related History-panel bugs in `photonic-gui`.

## What this PR implements

1. **Live Edit History.** `App::draw` now recomputes
   `self.history_entries = history.history_entries(20);` each frame, right
   before the drawer `PropPanelCtx` is built
   (`crates/photonic-gui/src/app/mod.rs`, just above the `let mut ctx =
   panels::PropPanelCtx {` at ~line 3349). The list now reflects the current
   undo stack immediately after every edit with no manual refresh. The
   now-redundant `⟳` refresh button was removed from `draw_edit_history`
   (`crates/photonic-gui/src/panels/mod.rs`); the `PanelAction::RefreshHistory`
   variant + its handler are kept (harmless) to minimize blast radius.

2. **Removed the redundant right-side Change Log.** The middle "Change log"
   block in the right `SidePanel` (the `history.list_checkpoints()` fetch, the
   `draw_changelog_panel` call, the `changelog_h` local, and the surrounding
   separator) was deleted from `app/mod.rs`. The right panel now holds Layers
   (top) + AI chat (bottom). The unused `draw_changelog_panel` fn was deleted
   from `panels/mod.rs` and the now-unused `CheckpointInfo` import trimmed. The
   `RestoreCheckpoint` / `DiffWithCheckpoint` variants + handlers lost their
   only UI producer, so both variants are annotated `#[allow(dead_code)]` (with
   a doc note) to keep the build warning-clean while preserving the handlers.

**Verification:** `cargo build --release`, `cargo check --workspace`, and
`cargo test -p photonic-gui` all pass. No new warnings introduced (the 5
`photonic-gui` warnings are pre-existing `id_source`/`id_salt` deprecations).

## Remaining work

- Relocating checkpoint **restore/diff** to a menu, or fully retiring the
  checkpoint UI + its `RestoreCheckpoint`/`DiffWithCheckpoint` handlers. Kept
  behind `#[allow(dead_code)]` for now; out of scope for this pass.

## Summary

The **Edit History** list (left drawer, `History` group) is backed by a cached
`Vec` (`App.history_entries`) that is only repopulated when the user clicks the
`⟳` refresh button (`PanelAction::RefreshHistory`). As a result it goes stale
immediately after any edit and looks empty / out of date until manually
refreshed. It should reflect the current undo stack live.

Separately, the **right side panel** renders a second "Change Log" panel
(`draw_changelog_panel`, driven by `history.list_checkpoints()`) that the user
considers redundant with the left-side Edit History. Per the issue it should be
removed.

## Scope

### In
- Make the left-drawer Edit History update live (no manual refresh needed).
- Remove the right-side "Change Log" panel render from the right `SidePanel`.
- Keep the build warning-clean after removing the panel call.

### Out
- Redesigning the Edit History UI (jump-to-step, per-step colors) — unchanged.
- Reworking the checkpoint subsystem itself (`CommandHistory` checkpoints,
  restore/diff). We only remove the redundant *UI surface*; the underlying
  checkpoint API stays.
- The `.photon` history persistence format.

## Approach

### 1. Live Edit History

`App::draw` (`crates/photonic-gui/src/app/mod.rs`) already receives
`history: &mut CommandHistory`, and the drawer `PropPanelCtx` is built with
`history_entries: &self.history_entries` (line ~3397) and
`history_total: history.undo_depth()` (line ~3398).

`history.history_entries(20)` (`crates/photonic-core/src/history.rs:1907`) is
cheap — it takes at most 20 items off the undo stack and maps
`description()`. Refresh the cache every frame right before the drawer ctx is
constructed:

```rust
// Keep Edit History live — recompute from the current undo stack each frame.
self.history_entries = history.history_entries(20);
```

This preserves the existing `&self.history_entries` borrow shape (the mutable
assignment completes before the immutable borrow in the ctx), so the
`PanelAction::RefreshHistory` handler (line ~9283) and `history_total` continue
to work. The now-redundant `⟳` refresh button in `draw_edit_history`
(`crates/photonic-gui/src/panels/mod.rs:6107-6113`) is removed; the
`RefreshHistory` variant + handler are kept (harmless) to minimize blast radius.

### 2. Remove the right-side Change Log

In the right `SidePanel` (`app/mod.rs:3430-3466`), delete the "Change log
(middle)" block:
- the `let checkpoints = history.list_checkpoints();` fetch,
- the `panels::draw_changelog_panel(ui, &checkpoints, changelog_h)` call and its
  action push,
- the surrounding `ui.separator();` and the `changelog_h` local (line ~3437).

The right panel then holds Layers (top) + AI chat (bottom). Delete the now-unused
`draw_changelog_panel` fn (`panels/mod.rs:7761`) to avoid a dead-code warning.

The `PanelAction::RestoreCheckpoint` / `DiffWithCheckpoint` variants and their
handlers (`app/mod.rs:5792`, `:7755`) lose their only UI producer. To keep the
build warning-clean without deleting real functionality, annotate those two
variants with `#[allow(dead_code)]` (never-constructed enum variants otherwise
warn). This is documented as a follow-up: either relocate checkpoint
restore/diff to a menu or fully retire the checkpoint UI. That relocation is
**out of scope** for this pass.

## Verification

`cargo build --release` must succeed with no new warnings. Manual GUI check:
make an edit → open the History drawer group → the Edit History list reflects
the new step without pressing refresh; confirm the right panel no longer shows a
"Change Log" section.

## Files to touch
- `crates/photonic-gui/src/app/mod.rs` — live refresh; remove right-side change
  log block; `#[allow(dead_code)]` on the two checkpoint action variants.
- `crates/photonic-gui/src/panels/mod.rs` — remove `⟳` button; delete
  `draw_changelog_panel`.
