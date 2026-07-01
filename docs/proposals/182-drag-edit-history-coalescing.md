# Continuous/drag edits: coalesce one History step per gesture (#182)

> Status: **implemented**. Umbrella fix for per-tick History flooding on drag/continuous edits.
> Also closes #180 (fill/stroke color-picker instance).

## What this PR implements

- **`CommandHistory` gesture coalescing** (`crates/photonic-core/src/history.rs`):
  - Two new fields `coalescing` / `coalesce_started` plus a
    `begin_coalescing()` / `end_coalescing()` pair (`begin` is idempotent;
    `is_coalescing()` added for introspection/tests).
  - `Command::coalesce(last, new) -> Option<Command>` merges only same-target
    value-replace commands — `UpdateNode` (same `new.id`), `SetWidthProfiles`,
    `SetGuides`, `SetArtboards`, `ResizeCanvas` — keeping the anchor's `old`
    (pre-gesture state) and the incoming `new`. Everything else returns `None`.
  - `execute()` folds a mergeable command into `undo_stack.last()` when
    `coalescing && coalesce_started`, applying the command to the doc and
    rescheduling the debounce, instead of pushing a new step. The first push of a
    gesture still goes through the normal path (clears redo, enforces step
    ceiling) and sets `coalesce_started` so only later ticks of the *same*
    gesture fold. `revision` is intentionally not bumped in the merge path —
    matching the existing `execute` path, which never bumped it either.
  - 5 unit tests: 20 streamed `UpdateNode`s → one undo step + single-undo
    restores the pre-gesture state (+redo re-applies); two gestures → two steps;
    interleaved different node ids → no merge; first edit of a gesture never
    folds into a leftover pre-gesture step; and a `Command::coalesce` merge/refuse
    matrix (same-id merge, different-id refuse, width/resize merge, add + mismatch
    refuse).
- **One GUI wiring point** (`crates/photonic-gui/src/app/mod.rs`, `App::draw`):
  `history.begin_coalescing()` when `pointer.any_down()` (before the tool/panel
  handlers), and `history.end_coalescing()` in the existing post-loop
  `any_released()` block that already commits the pending recent color (~line
  11963), so a final same-frame edit still folds in.

Verified: `cargo test -p photonic-core` (305 lib pass, incl. the 5 new), release
build, `cargo test -p photonic-gui`, and `cargo check --workspace` all green.

## Remaining work

- No `.photon` history format change and no new `Command` variants — the undo
  stack still holds ordinary entries, so no MCP/doc-API regeneration was needed.
- Tools that already coalesce (move/resize/width/document-recolor) are untouched:
  they emit a single `execute` per gesture, so nothing merges. A future edit path
  that mutates the doc *without* going through `history.execute` would still need
  its own wiring — out of scope here.

---


## Summary

Some live edits push a `Command` onto the undo stack on **every pointer tick** of
a continuous gesture, so one logical edit becomes dozens of undo steps. The wanted
behavior is **one gesture = one undo step**: preview live while the pointer is down,
commit a single History node on release.

Most drag paths in the codebase already do this via the established
"live-preview + commit-on-release" idiom and are **not** the problem:

- Move / resize / transform: capture `move_drag_origins` / `resize_drag_origins`
  at drag start, emit one `UpdateNode` batch on release (`app/mod.rs`).
- Width tool: previews directly on the doc during drag, emits one
  `Command::SetWidthProfiles` on `drag_stopped()` (`app/width_tool.rs`).
- Document color-swatch recolor: `RecolorPreview` (no history) → `RecolorCommit`
  (one step) (`panels/mod.rs`).
- Recent-colors list: deferred to release (#171).

The remaining streaming offender is the **fill/stroke node color picker**
(`PanelAction::UpdateNodeFill` / `UpdateNodeStroke` in `app/mod.rs`), which calls
`history.execute(Command::UpdateNode { .. })` on every drag frame inside the RGBA
picker popup — this is exactly the #180 instance. Rather than refactor that one
site to the idiom, this proposal adds a **general gesture-coalescing mechanism to
`CommandHistory`** so any value-replace edit streamed through `execute` during a
pointer gesture collapses to a single undo step — fixing #180 and acting as a
safety net for any future slider/handle that streams `execute`.

## Scope

### In
- **`CommandHistory` gesture coalescing** (`crates/photonic-core/src/history.rs`):
  a `begin_coalescing()` / `end_coalescing()` pair plus a merge path inside
  `execute` that folds a mergeable command into the current gesture's anchor entry
  instead of pushing a new one. Unit tests for the merge/no-merge cases.
- **One GUI wiring point** (`crates/photonic-gui/src/app/mod.rs`, `App::draw`):
  open coalescing while the pointer is down (before the tool/panel handlers),
  close it on pointer release (in the existing post-loop `any_released()` block
  that already commits the recent color, ~line 11949).

### Out (deferred)
- No changes to the tools that already coalesce (move/resize/width/recolor) — they
  emit a single `execute` per gesture, so nothing merges and behavior is unchanged.
- No new `Command` variants and **no `.photon` history format change** — the undo
  stack still holds ordinary `UpdateNode` / `SetWidthProfiles` / … entries.
- No per-site refactor of individual sliders/panels; the general mechanism covers
  them. If a future path mutates the doc *without* going through `history.execute`,
  wiring it is separate follow-up work.

## Approach

### Core: coalescing in `CommandHistory`

Add two fields:

```rust
/// A pointer gesture is open; mergeable same-target edits fold into one step.
coalescing: bool,
/// Set once the first mergeable command of the current gesture is pushed, so
/// the anchor entry (undo_stack.last()) is only merged into within one gesture.
coalesce_started: bool,
```

Public API:

```rust
pub fn begin_coalescing(&mut self) {           // idempotent per gesture
    if !self.coalescing { self.coalescing = true; self.coalesce_started = false; }
}
pub fn end_coalescing(&mut self) {
    self.coalescing = false; self.coalesce_started = false;
}
```

In `execute`, before the normal `undo_stack.push(cmd)`, when
`self.coalescing && self.coalesce_started` and the incoming command merges with the
current top of the undo stack, fold instead of push:

```rust
if self.coalescing && self.coalesce_started {
    if let Some(merged) = Command::coalesce(self.undo_stack.last(), &cmd) {
        cmd.apply(doc);
        reevaluate_constraints(doc);
        *self.undo_stack.last_mut().unwrap() = merged; // keep original `old`, take new `new`
        self.revision += 1;
        self.gui_debounce.schedule(desc);
        return;
    }
}
// … existing push path …
self.coalesce_started = true; // after a normal push while coalescing
```

`Command::coalesce(last, new) -> Option<Command>` merges only same-target
value-replace commands, keeping `last`'s before-state and `new`'s after-state:

- `UpdateNode { old: last.old, new: new.new }` when both are `UpdateNode` with the
  same `new.id`.
- `SetWidthProfiles`, `SetGuides`, `SetArtboards`, `ResizeCanvas`: keep `old` from
  `last`, `new` from the incoming.
- Anything else → `None` (push a fresh step; different targets never merge).

Merging into `undo_stack.last()` is safe: `enforce_steps` trims from the front
(oldest), never the newest anchor. Redo was already cleared on the gesture's first
push, so the merge path doesn't touch it. Coalescable commands need no `hydrate`
(that only rewrites deletes).

### GUI wiring (single point in `App::draw`)

```rust
// before the tool/panel action handlers run this frame:
if ctx.input(|i| i.pointer.any_down()) { history.begin_coalescing(); }
// … existing canvas-tool + PanelAction handling (all history.execute calls) …
// in the existing post-loop `any_released()` block (~11949):
if ctx.input(|i| i.pointer.any_released()) { history.end_coalescing(); }
```

`begin` is idempotent, so it stays open across every frame of the gesture; `end`
runs only on the release frame, *after* the handlers, so a final same-frame edit
still folds into the one step. Between gestures the flag is false and edits push
normally. Discrete clicks (down+up in one frame) push exactly one entry — the first
push sets `coalesce_started` but nothing else merges before release.

Net effect: dragging through the fill/stroke RGBA picker (#180) records **one**
`UpdateNode` undo step spanning the whole gesture; already-coalesced tools are
untouched.

## Verification

- `cargo build --release` and `cargo test -p photonic-core` (new merge tests:
  streamed `UpdateNode`s during one gesture → `undo_depth() == 1` and a single
  `undo` restores the pre-gesture state; two gestures → two steps; different node
  ids → two steps; non-coalescing edits unchanged).
- Manual GUI check: select a shape, drag the Fill color slider — Edit History gains
  one step; Ctrl+Z restores the original color in one press.

## Fix round 1 (adversarial review)

**[major] Coalescing collapsed concurrent MCP/REPL/script edits.** The GUI and the
MCP server share one `Arc<Mutex<CommandHistory>>` (photonic-app `main.rs`: created
at ~:145, handed to the MCP background thread at ~:189 and to the GUI at ~:206). The
gesture flags (`coalescing` / `coalesce_started`) and the merge branch live on that
shared object, but they are armed *only* by GUI pointer state
(`begin_coalescing()` on `pointer.any_down()`, `end_coalescing()` on
`pointer.any_released()`). While a GUI pointer was merely held down (dragging a
swatch, panning, holding a scrollbar, an in-progress marquee), any external edit
executed on the same lock between GUI frames would fold into the open gesture's
anchor — so multiple independent AI/script `UpdateNode`s (and friends) collapsed
into one non-granular undo step, and if the anchor was the user's own drag the AI
edit was absorbed into it (one Ctrl+Z reverting both). This contradicted the scope
claim that MCP/doc-API behavior was untouched. Photonic is an explicit AI+human
collaborative editor, so GUI-pointer-down + concurrent MCP editing is a realistic
path, not an exotic one.

**Fix.** Coalescing is now scoped to GUI-originated edits via a discrete entry
point. `CommandHistory::execute_discrete(cmd, doc)`
(`crates/photonic-core/src/history.rs`) snapshots the gesture flags, forces
coalescing off for the push (so the command always lands as its own undo step),
then restores the gesture-open flag while leaving `coalesce_started` false — the
pushed command is now `undo_stack.last()`, so an in-progress GUI gesture re-anchors
on its next tick instead of folding a later pointer tick into the external command.
Every non-GUI edit source switched from `execute` to `execute_discrete`:

- all 211 `history.execute` sites in `crates/photonic-mcp/src/handlers/**`;
- the Lua/REPL bindings in `crates/photonic-app/src/script.rs` and `repl.rs`.

GUI edit sites in `crates/photonic-gui/**` are unchanged and still use `execute`,
preserving the #180 one-drag-one-step behavior.

Regression test `execute_discrete_does_not_fold_into_open_gesture`
(`crates/photonic-core/src/history.rs`): opens a gesture, pushes a coalesced GUI
anchor, then a simulated external `execute_discrete` on the same node — asserts it
lands as a SEPARATE undo step, that the gesture stays open, that a subsequent GUI
tick re-anchors (does not merge into the external step), and that one `undo` peels
off only the external edit.

**Still deferred.** `Command::Batch` remains coalesce-exempt (`coalesce()` returns
`None` for it), unchanged by this fix. No `.photon` history format change and no new
`Command` variants — `execute_discrete` reuses the ordinary push path.
