# Direct Select — right-click anchor context menu for point type + curvature handles (#187)

> Status: **implemented.** Self-contained GUI-behaviour work in the Direct
> Selection tool. area:gui, priority:p2, type:enhancement/ux. The heavy lifting
> (`bez_convert_anchors`, `round_selected_corners`, on-canvas bezier-handle
> render/drag) already existed from #188 / #164 / #165; the genuinely missing
> piece was a **right-click context menu on an anchor**, which this PR adds.

## What this PR implements

- **Right-click context menu on a directly-selected anchor**
  (`crates/photonic-gui/src/app/direct_select.rs`). In `handle_direct_select_tool`,
  `response.secondary_clicked()` hit-tests the anchor at `interact_pointer_pos()`
  via the shared `ds_anchor_at` method (see round-1 fix below). On a hit the anchor is selected and
  recorded in a new transient field `point_context_anchor`. `response.context_menu`
  (same API as `panels/mod.rs:1052`) is registered every frame and renders items
  only when a context anchor is set (otherwise it closes so no empty menu lingers):
  - **Corner** → `bez_convert_anchors(&bez, {idx}, false)`.
  - **Smooth / Curved** → `bez_convert_anchors(&bez, {idx}, true)`.
  - **Round corner** submenu → `4 / 8 / 16 / 32 px` → `round_selected_corners`
    (restricted to genuine straight corners via `straight_corners`, matching the
    inspector "Round corners" buttons and the Live-Corners widget).
- **No new `PanelAction` variant.** Each action runs inline through two small
  helper methods (`ds_convert_context_anchor`, `ds_round_context_anchor`) that
  build a `Command::UpdateNode { old, new }`, run `history.execute(...)`, and set
  `*doc_modified = true` — exactly like the inline delete-anchor block already in
  this function. Atomic undo/redo.
- **Curvature handles surface immediately after Smooth.** After the convert the
  anchor is re-selected by matching its unchanged local position with
  `nearest_anchor_screen` (robust to the element renumber that
  `bez_convert_anchors` can cause when it materializes a closed subpath's implicit
  seam). The existing render loop (`direct_select.rs`) then draws its in/out
  handles and `ds_find_handle` makes them draggable on the next frame.
- **New state field** `point_context_anchor: Option<usize>` on `PhotonicApp`
  (`app/mod.rs`), reset in `clear_point_edit`, `invalidate_point_edit`, and the
  Escape handler alongside `point_selected`.
- **Discoverability polish:** inspector **Smooth** button relabelled to
  **Smooth / Curved** with a hover hint noting the right-click path
  (`panels/mod.rs`).

Geometry is unchanged; no MCP tools touched (so `docs/mcp-api.md` is unaffected).
`cargo build --release`, `cargo test -p photonic-gui` (50 + 1 pass), and
`cargo check --workspace` all pass.

## Round-1 adversarial fix — radial wheel shadowed the anchor menu (blocker)

The reviewer found the feature was orphaned end-to-end: the canvas response
handler in `app/mod.rs` runs a **global** right-click radial-wheel block (no
`active_tool` gate) that fires on `response.secondary_clicked()` for every tool,
unconditionally sets `self.radial_wheel = Some(...)`, and the following
`if self.radial_wheel.is_some()` block early-returns on the same frame — *before*
`handle_direct_select_tool` is ever called at its single call site. So the new
`response.context_menu(...)` closure was never registered, and right-clicking an
anchor in Direct Select popped the selection wheel instead of the Corner /
Smooth-Curved / Round menu.

Fix (no behaviour change for any other tool):

- Added a shared hit-test method **`PhotonicApp::ds_anchor_at(cx, cy, doc, view)`**
  in `direct_select.rs` that returns the nearest anchor index of the current
  `point_edit_node` within the 12 px grab radius. It replaces the former private
  `find_anchor` closure (removed), which had the identical body — the tool
  handler's four hit-test call sites now route through this one method, and so
  does the wheel guard, guaranteeing both agree on "is an anchor under the click?".
- In the radial-wheel block in `app/mod.rs`, before opening the wheel, compute
  `ds_anchor_menu = self.active_tool == Tool::DirectSelect && self.ds_anchor_at(cx, cy, doc, view).is_some()`
  and skip opening the wheel (do not set `radial_wheel`, do not early-return) when
  it is true. Control then falls through to `handle_direct_select_tool`, which
  registers the context menu. Right-clicking **empty canvas** (or a non-anchor
  location) in Direct Select still opens the wheel, matching every other tool.

Verified: `cargo build --release` clean; `cargo test -p photonic-gui` 50 + 1
pass. Interactive/headless egui simulation of the click-to-menu path is not part
of the existing test harness (no egui test-context fixtures in this crate), so
that automated interaction check is deferred; the guard reuses the exact,
already-unit-covered `nearest_anchor_screen` hit-test that the tool itself uses.

## Remaining work / deferred

- **Marquee / multi-vertex right-click** — deferred (see "Out" below); the menu
  targets the single right-clicked anchor. If a multi-anchor selection is already
  active, Shift/Ctrl-right-click adds to it, but the actions still operate on the
  single context anchor.
- **Rounded-point as a distinct persisted anchor kind** (live, non-destructive
  Illustrator "rounded point") — out of scope; "Round corner" reuses the existing
  destructive fillet.
- **Whole-path / MCP `round_corners` clamp bug** — tracked separately in #179.
- **Hover tooltip on anchors advertising right-click** — the anchor squares are
  painter-drawn (not egui widgets), so there is no `Response` to attach a hover
  tooltip to; discoverability is instead delivered via the pointing-hand cursor
  over anchors plus the relabelled inspector button. Deferred as a nicety.

## Summary

There is no discoverable, in-context way to round/curve a single corner anchor.
The inspector already exposes **Point type: Corner / Smooth**
(`panels/mod.rs:1370-1394`, `PanelAction::ConvertAnchorType`) and a **Round
corners** radius control (`:1398-1408`, `PanelAction::RoundCorners`), and the
canvas already renders + drags bezier curvature handles for selected anchors
(`direct_select.rs:519-538` render, `ds_find_handle` / `bez_set_handle` drag).
But these only appear deep inside path-edit mode, so users report "there appears
to be no way to round a corner."

The fix: right-click a directly-selected anchor → context menu offering
**Corner / Smooth (curved) / Round corner…**, reusing the existing geometry
helpers, and keep the just-converted anchor selected so its curvature handles
are shown and draggable immediately.

## Scope

### In

1. **Right-click context menu on an anchor** in `direct_select.rs`
   (`handle_direct_select_tool`). On `response.secondary_clicked()`, hit-test the
   anchor at `interact_pointer_pos()` via the existing `find_anchor` closure; if
   an anchor is hit, select it (set `point_selected = vec![idx]`) and record it
   as the context target. Attach `response.context_menu(|ui| { … })` (same API
   already used at `panels/mod.rs:1052`) that shows, only when a context anchor
   is set:
   - **Corner** → apply `bez_convert_anchors(&bez, {idx}, false)`.
   - **Smooth / Curved** → apply `bez_convert_anchors(&bez, {idx}, true)`.
   - **Round corner** submenu → `4 / 8 / 16 / 32` → apply `gui_round_corners`
     (matching the inspector radii and the Live-Corners widget).
   Each action builds a `Command::UpdateNode { old, new }` and runs it through
   `history.execute(...)` + sets `*doc_modified = true`, exactly like the inline
   delete-anchor block already in this function (`direct_select.rs:169-196`) — no
   new `PanelAction` variant needed.

2. **Surface curvature handles immediately after Smooth.** In the context-menu
   Smooth path, keep the converted anchor in `point_selected` (do **not** clear
   it) so the existing render loop (`:519-538`) draws its in/out handles and
   `ds_find_handle` makes them draggable on the next frame. (Note: the shared
   `PanelAction::ConvertAnchorType` handler at `app/mod.rs:11296` clears the
   selection because a whole-selection convert can restructure indices; a
   single-anchor convert keeps the same index for the moved anchor, so re-select
   that one index after the command — re-derive it if the convert is known to
   preserve position, else re-select via `find_anchor` at the anchor's point.)

3. **Discoverability polish (small):** relabel the inspector **Smooth** button to
   **Smooth / Curved** (`panels/mod.rs:1384`) to match the menu + user mental
   model, and add a hover hint on anchors that right-click offers point-type
   options.

### Out (deferred)

- **Marquee / multi-vertex right-click** (apply to a whole vertex selection) —
  #181's territory; this PR targets the single right-clicked anchor. If a
  multi-anchor selection is already active when the menu opens, the actions may
  operate on the full `point_selected` set (cheap, reuses the same helpers) but
  that is optional and not required for close-out.
- **Whole-path / MCP `round_corners` clamp bug** — tracked separately in #179.
- **Rounded-point as a distinct persisted anchor kind** (Illustrator "rounded
  point" that stays live) — out of scope; "Round corner…" here reuses the
  existing destructive `gui_round_corners` fillet.
- **QuadTo-adjacent smoothing** — unchanged/verbatim, same as #188.

## Approach notes

- `handle_direct_select_tool` already owns `doc`, `history`, `doc_modified` and
  applies commands inline, so the context menu can execute geometry commands
  directly there — no plumbing to the `PanelAction` dispatch loop.
- Track the right-clicked anchor with a transient local (or a new
  `point_context_anchor: Option<usize>` field on `PhotonicApp` alongside
  `point_selected` at `app/mod.rs:487-495`) captured on `secondary_clicked()` and
  read inside the `context_menu` closure; clear it when the menu closes.
- Geometry is unchanged — this PR only wires UI to `bez_convert_anchors`
  (`geometry.rs:871`) and `gui_round_corners` (`geometry.rs:1313`), both already
  unit-tested.
- No MCP tools touched → `docs/mcp-api.md` unaffected. House rule:
  `cargo build --release` must pass; run `cargo test -p photonic-gui`.

## Related

- #188 (closed) — inspector Smooth synthesizes handles; explicitly deferred this menu.
- #164 / #165 (closed) — whole-object highlight; multi-select corner rounding + cap.
- #179 / #181 (open) — round_corners clamp; Direct Select marquee.
- #63 (EPIC) — interactive editing parity.
