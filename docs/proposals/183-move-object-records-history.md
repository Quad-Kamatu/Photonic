# Moving an object must record one undoable History step (#183)

> Status: **A2 fix implemented; A1 not addressed; end-to-end unproven pending
> live GUI confirmation.** Regression fix of #11 (move → History), sibling of #5
> (resize) and #182 (drag coalescing). This addresses the *A2* root cause
> (canvas response swallowed on release) with a testable release predicate and
> live instrumentation; the *A1* root cause (origins never captured) is
> explicitly NOT fixed here and is deferred — see "Root-cause honesty" below.

## What this PR implements

Diagnosis (static, verification-first): the #11 recorder in
`App::handle_select_tool` (`crates/photonic-gui/src/app/tool_handlers.rs`) is
present and, in isolation, correct. `Command::coalesce` (`history.rs`) returns
`None` for **every** `Command::Batch`, so the release-time move Batch can never
fold into an open #182 coalescing gesture. The move also captures its
`move_drag_origins`/`move_snap_origins` in the *same* first-drag-frame block that
applies the motion, so whenever the object visibly moves those origins are
populated. That leaves **one** way the reported symptom (no Ctrl+Z, no timeline
entry) can occur: `response.drag_stopped_by(Primary)` never fires on the canvas
response, because a competing overlay allocated **later** in the frame — the
artboard drag-handle / name hit-target (`app/mod.rs:4053/4068`) or a full-canvas
modal scrim — captured/shadowed the pointer for that release. When that happens
the recorder block is simply never reached and the move is lost.

Fix (both branches of the plan, belt-and-suspenders):

1. **Reachability (Branch A).** The completed-move recording is extracted into
   `PhotonicApp::finalize_move(...)` and invoked from **two** places: the normal
   `drag_stopped_by(Primary)` path, and a new **fallback** branch that fires when
   a move is still pending (`!move_drag_origins.is_empty()`) but the primary
   button is no longer held and we are not mid-drag. So even if the canvas
   response is swallowed and `drag_stopped_by` never fires, the move is still
   finalized on release. The two paths are idempotent — whichever runs first
   consumes `move_drag_origins`, so the move records exactly once.

2. **Discrete step (Branch B).** `finalize_move` records through
   `CommandHistory::execute_discrete` instead of `execute`, guaranteeing the
   move lands as its own undo entry regardless of any coalescing gesture still
   open on the shared GUI+MCP history — mirroring the non-GUI-edit rationale
   already documented on `execute_discrete`. Applies to both the plain-move
   (`UpdateNode` Batch) and Alt-duplicate (`AddNode` Batch) cases.

3. **Testable fix predicate (GUI).** The release decision for the fallback is
   extracted into a pure function
   `photonic_gui::app::tool_handlers::should_finalize_move_fallback(move_pending,
   primary_down, dragged_by_primary)` and unit-tested in `move_fallback_tests`
   (5 tests): the swallowed-response frame selects finalize; an active drag, a
   paused-but-held button, the `drag_stopped_by` frame, and the no-pending-move
   case (including the A1 shape) all correctly stand down. This exercises the
   **actual #183 fix condition** in CI — the earlier core test could not, since
   it never reaches `finalize_move` or the fallback branch.

4. **Live A2/A1 instrumentation (GUI).** The release path now emits `tracing`
   under target `photonic::move`: a `debug` line when the move is recorded via
   the normal `drag_stopped_by(Primary)` path, a distinct `debug` line when it is
   recovered via the #183 fallback, and a `warn` when a drag stops in move mode
   with **no captured origins** (the A1 signature). Running the GUI with
   `RUST_LOG=photonic::move=debug` therefore *proves which branch recovers the
   reported bug* — confirming the A2 hypothesis or surfacing A1 instead of
   silently no-op'ing.

5. **Core guardrail (reframed).** The `photonic-core` test — renamed
   `history::tests::batch_never_coalesces_into_open_gesture` — asserts, for both
   `execute` and `execute_discrete`, that a move Batch pushed while a coalescing
   gesture is open (primed same-target anchor) records exactly one undo step and
   a single `undo()` restores every node's transform. Per adversarial review its
   doc comment now states plainly that it locks a **pre-existing core invariant
   the GUI fix relies on**, would pass with the #183 GUI fix reverted, and is
   **not** the #183 regression contract (that role is filled by item 3 + manual
   confirmation).

## Root-cause honesty (A2 addressed, A1 deferred)

Diagnosis listed two candidate causes (see below). This change fixes **A2**
(overlay swallows the canvas response so `drag_stopped_by(Primary)` never fires).
It does **not** fix **A1** (origin capture / `self.moving` never runs, so
`move_drag_origins` is empty at release): the fallback is guarded by
`!move_drag_origins.is_empty()`, so if A1 is the true cause the fallback is a
no-op and #183 stays broken. This is now *observable rather than silent* — the
A1 `warn` (item 4) fires in that case. If live logs show origins empty at
release, this fix must be paired with a hit-test / `self.moving` / origin-capture
fix in a follow-up; do not close #183 on this change alone until the logs confirm
the A2 fallback (not A1) is what recovers the reported case.

Verification: `cargo build --release` ✓, `cargo test -p photonic-core` ✓ (incl.
the renamed guardrail test), `cargo test -p photonic-gui` ✓ (incl. the 5 new
`move_fallback_tests`), `cargo fmt --all --check` ✓.

## Remaining work

- **Manual GUI confirmation (blocking sign-off).** A true live drag cannot be
  exercised headlessly here. Run the app with `RUST_LOG=photonic::move=debug` and
  confirm: single-node move, multi-select move, and Alt-duplicate move each
  produce **one** Ctrl+Z-undoable History timeline entry — including a move that
  starts on/over an artboard whose drag handle previously shadowed the release —
  AND that the log shows the fallback (or drag_stopped) path firing, not the A1
  `warn`. If the A1 `warn` appears, escalate to the A1 follow-up before closing.
- **A1 follow-up (conditional).** If instrumentation shows origins empty at
  release, add the hit-test / origin-capture fix so the ordinary
  drag-a-selected-object gesture reliably sets `self.moving` and snapshots
  origins. Deferred here pending the live diagnosis.
- Resize (#5) recording keeps its own `drag_stopped_by`-only path (no fallback);
  left intact per scope. If it shows the same overlay-shadowing symptom it can
  reuse the same fallback pattern in a follow-up.

---

## Original plan (design scaffold)

## Summary

Dragging an object with the Select tool must record the completed move as
**one** undoable History step on release: Ctrl+Z reverts it and it shows in the
History timeline. #183 reports this no longer happens. #11's fix lives today in
`App::handle_select_tool` (`crates/photonic-gui/src/app/tool_handlers.rs`), which
snapshots the moved nodes at drag start and emits
`history.execute(Command::Batch([UpdateNode{old,new}, …]))` on
`drag_stopped_by(Primary)`.

Static analysis shows that recording code is still present and, in isolation,
correct — including its interaction with #182 coalescing: a `Command::Batch` is
not a mergeable target for `Command::coalesce`, so the release-time push can
never be folded/absorbed into another gesture's anchor. The regression therefore
is *not* an obviously missing line; it must be pinned by reproduction before a
fix is written. This is a verification-first bug (systematic-debugging).

## Scope

### In
- The Select-tool object-move path only: `handle_select_tool`'s
  `dragged_by`/`drag_stopped_by` branches in
  `crates/photonic-gui/src/app/tool_handlers.rs`.
- Ensuring exactly one coalesced `Command::Batch`/`UpdateNode` step is recorded
  per move-drag on release (per #182's single-step-on-release contract), undoable
  with Ctrl+Z and visible in the timeline.
- The multi-select move case (Batch over all moved nodes) and the plain
  single-node move.

### Out
- Alt-duplicate-drag recording (already its own AddNode Batch branch) — verify
  it still works but don't redesign it.
- Resize recording (#5) — separate `resize_drag_origins` branch; leave intact.
- Artboard drag/resize (`artboard_drag`/`artboard_resize` in `app/mod.rs`),
  raster brush strokes, DirectSelect point edits — different subsystems.
- Any `.photon` history format change or new `Command` variants.

## Approach

1. **Reproduce & instrument (diagnosis first).** Build `--release`, run the GUI,
   drag a selected object, release, press Ctrl+Z, inspect the timeline. Add a
   temporary `eprintln!`/`tracing` probe at the top of the
   `drag_stopped_by(Primary)` block in `handle_select_tool` reporting
   `self.moving`, `self.move_drag_origins.len()`, and whether the
   `cmds`/`history.execute` branch fired. This splits the cause into one of two
   branches:

2. **Branch A — `move_drag_origins` is empty at release** (recording code never
   reached). Root cause is upstream: `self.moving` was never set true, or the
   first-move-frame snapshot in the `dragged_by` branch never ran (e.g. a
   hit-test / selection-bounds regression after the `1e29944` module split, or a
   competing overlay `ui.interact` — artboard drag handle, selection-name handle
   — consuming the canvas `response` so `drag_stopped_by` never fires on it).
   Fix: restore `self.moving`/origin-capture for the ordinary
   drag-a-selected-object gesture so the existing release-time recorder runs.

3. **Branch B — `history.execute` fires but nothing is undoable/visible**
   (recording code reached but ineffective). Likely the timeline reads a
   different structure than the undo stack, or the Batch is emitted at a moment
   the gesture-coalescing window mishandles it. Fix: record the completed move as
   a **discrete** step — `history.execute_discrete(Command::Batch(cmds), doc)` —
   so it is guaranteed to land as its own undo entry regardless of the open
   `coalescing` gesture (mirroring the non-GUI-edit rationale documented on
   `execute_discrete`), and confirm the History timeline panel renders from the
   undo stack.

4. **Regression guard.** Add a `photonic-core` unit test asserting that a
   `Command::Batch` of `UpdateNode`s pushed via `execute`/`execute_discrete`
   while a coalescing gesture is open records exactly one undo step and a single
   `undo()` restores the pre-move transforms — locking the move→History contract
   at the core layer independent of the GUI.

5. **Verify:** `cargo build --release`, `cargo test -p photonic-core`,
   `cargo test -p photonic-gui`, then a manual GUI pass (single move, multi-select
   move, Alt-duplicate move) confirming one undo step each and a timeline entry.
