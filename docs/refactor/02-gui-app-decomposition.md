# 02 — GUI `app/mod.rs` Decomposition

**File:** `gui/src/app/mod.rs` (14,946). **Track:** independent of MCP.
**The hardest structural target** — but with a large mechanical portion that
peels off first.

`PhotonicApp` has ~85 fields across 15 subsystems and 54 methods, but the mass
is one method: **`draw()` is 11,179 lines (lines 2,305–13,483)** with no helper
extraction. Inside it, a single "drain panel actions" match block is **6,596
lines (lines 6,286–12,881) — 59% of `draw()`**.

## The enabling fact
The `app/` directory **already uses the correct pattern**: 10 sibling files
(`tool_handlers.rs`, `geometry.rs`, `direct_select.rs`, `width_tool.rs`,
`erase_tools.rs`, `command_center.rs`, `hit_test.rs`, `rulers.rs`,
`layer_ops.rs`, `demos.rs`) each add an `impl PhotonicApp` block, retaining full
private-field access. **Every extraction below follows this established
pattern** → the moves are mechanical and low-risk despite the struct's size.

---

## Tier-A (Hermes) — wholesale block extractions

These are self-contained blocks that move into a new `impl PhotonicApp` file
with near-zero refactoring. Sequence them to shrink `draw()` fastest-first.

### WP-3A — Drain panel actions → `ui_actions.rs`  ·  **Hermes (Deepseek V4)**  ·  biggest single win
Lines 6,286–12,881 (**6,596 lines**). A giant `match action { … }` over
`PanelAction` (SelectNode, ReorderNode, BooleanOp, ~100 more). It only mutates
`doc` + `history`, which are already designed to be mutated together, so it
lifts wholesale into one method. **`draw()` drops from 11.2k → ~4.6k in one PR.**
Long-context block move → Deepseek V4.
*Risk:* the doc+history mutation pairs must move together; the match is already
self-contained so this is low. Golden snapshot is the proof.

### WP-3B — Remaining self-contained UI blocks  ·  **Hermes (Qwen 3.7) ×4**
Each a separate file / PR:
- `ui_drawers.rs` (~874 L) — the menu drawer (File 144 / Preferences 155 /
  History 332). Pure UI; writes ~20 prefs fields + reads history.
- `ui_dialogs.rs` (~1,000 L) — export / simplify / merge / find-replace modals.
  These already have `draw_*_modal()` methods; consolidate the inline pieces.
- `canvas_overlays.rs` (~600 L) — raster/preview/outline/grid/guides/smart-
  guides/dimensions/diff overlays. **Read-only on doc/view, pure rendering** —
  lowest risk in the whole file.
- `artboard_ui.rs` (~500 L) — artboard labels/drag/resize/rename/add-remove;
  mutates only the localized `artboard_*` fields.

After 3A + 3B, `draw()` is ~2k lines of orchestration.

---

## Tier-B (Codex) — the interleaving that resists mechanical extraction

### WP-3C — Frame-phase abstractions + render/input separation  ·  **Codex**  ·  gate for 3D
The central canvas area (~lines 4,790–6,286, ~1,500 L) **interleaves rendering
with input handling** in one painter loop — you cannot mechanically cut it
because render code mutates interaction state mid-pass. Codex introduces:
- `FrameState` — groups the per-frame scratch booleans (`doc_modified`,
  `escape`, …) currently loose on the struct.
- `CanvasInput` — encapsulates mouse/keyboard/scroll for the frame.
- `DrawPhase` enum (`RenderOnly` | `InteractionAllowed`) — makes it a *type
  error* for render code to mutate state, so the two concerns can be split.

Then restructure `draw()` into an explicit pipeline:
`initialize_frame_state() → draw_static_ui() → draw_canvas_and_overlays() →
process_user_input() → dispatch_tool_handlers() → process_panel_actions() →
finalize_frame()`.

**Why Codex:** inventing `FrameState`/`CanvasInput`/`DrawPhase` and re-sequencing
control flow is rubric C1+C2. This is the one genuinely hard piece here.

### WP-3D — Extract canvas input & tool dispatch  ·  **Hermes (Qwen 3.7)**  ·  needs 3C
Once 3C has separated the phases:
- `canvas_input.rs` (~500 L) — pan/zoom/WASD/keyboard nav (mutates smooth-view +
  `view.pan/zoom`).
- `tool_dispatch.rs` (~200 L) — the mechanical if-chain routing to the existing
  `tool_handlers.rs` methods.

Mechanical *after* 3C makes the seam clean.

---

## Optional follow-ups (Codex, low priority)
`frame_init.rs` (~300 L pre-draw guards/polling) can peel off after 3C if
desired. Introduce `DialogManager` / `PanelStateManager` to unify modal + drawer
lifecycles — nice-to-have, not required for the size goal.

## Summary

| WP | Tier | Model | ~Lines out of `draw()` | Deps |
|---|---|---|---:|---|
| 3A ui_actions | Hermes | Deepseek V4 | 6,596 | — |
| 3B ×4 drawers/dialogs/overlays/artboard | Hermes | Qwen 3.7 | ~2,974 | — |
| 3C frame-phase abstractions | Codex | — | (restructure) | after 3A/3B |
| 3D canvas_input + tool_dispatch | Hermes | Qwen 3.7 | ~700 | 3C |

End state: `draw()` → ~2k-line orchestrator + ~7 focused `impl PhotonicApp`
submodules, matching the pattern the crate already uses. Note 3A/3B are all one
file (`app/mod.rs`) → **serialize these Hermes PRs** (or have one Hermes agent do
them in sequence) to avoid self-conflict; they don't parallelize the way the MCP
handler split does.
