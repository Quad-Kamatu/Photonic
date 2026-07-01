# 171 — Color picker: recent colors should only commit on release, not stream while dragging

> **Status: IMPLEMENTED.** Commit-on-release landed in
> `crates/photonic-gui/src/app/mod.rs`:
> - New `App` field `pending_recent_color: Option<Color>` (near
>   `pending_panel_actions`), initialized `None` in the constructor.
> - `PanelAction::UpdateNodeFill` / `UpdateNodeStroke` handlers now stash the
>   chosen color into `pending_recent_color` instead of calling
>   `doc.record_recent_color(...)` per frame (the fill/stroke is still applied
>   live via `history.execute`, so canvas preview is unchanged).
> - A post-`'actions`-loop block records exactly one color into `recent_colors`
>   when `ctx.input(|i| i.pointer.any_released())` — one final swatch per drag.
> - `cargo build --release`, `cargo test -p photonic-gui`, and
>   `cargo check --workspace` all pass. No picker-widget changes were needed.
>
> **Remaining work (deferred, out of scope for #171):** undo-history spam —
> dragging the picker still calls `history.execute(...)` per frame, creating many
> undo steps. Fixing that requires coalescing live edits into a single undo step
> and is tracked separately. Gradient/fluid/mesh color editing does not feed
> `record_recent_color`, so it is unaffected.

## Summary

When recoloring an object via the Fill/Stroke color button, egui's
`color_edit_button_rgba_unmultiplied` popup emits `Response::changed()` on **every
frame** while the user drags inside the saturation/value square or a slider. Each of
those changes is turned into a `PanelAction::UpdateNodeFill` / `UpdateNodeStroke`, and
the handler for those actions calls `doc.record_recent_color(...)` **synchronously**.
The result: the whole drag path is streamed into the **Recent** swatch list, flooding it
with dozens of near-identical intermediate colors instead of the single final color.

Fix: keep applying the color to the object live during the drag (so the canvas preview
stays responsive), but **defer recording into `recent_colors` until the pointer is
released**, committing exactly one final color per interaction.

## Root cause (grounded)

- `crates/photonic-gui/src/app/mod.rs`
  - `PanelAction::UpdateNodeFill` handler (~5750): calls `doc.record_recent_color(*c)`
    every time the fill changes.
  - `PanelAction::UpdateNodeStroke` handler (~5768): calls
    `doc.record_recent_color(stroke.color)` every time the stroke changes.
  - Both live inside `App::draw()` (fn at ~1920), which has `ctx: &egui::Context`,
    `doc: &mut Document`, and `&mut self` in scope — the `'actions` drain loop starts at
    ~5601.
- `crates/photonic-gui/src/panels/mod.rs`
  - `draw_fill_editor` (~6742) / the solid arm (~6875) return `Some(Fill)` whenever the
    color button's `.changed()` fires — which streams during the drag.
  - `draw_stroke_editor` behaves the same, feeding `UpdateNodeStroke` (~1998).
- `crates/photonic-core/src/document.rs`
  - `record_recent_color` (~917): dedups + inserts at front + truncates to 20. Correct
    behavior; it is simply being *called* too often.

## Approach

Decouple **live color application** (must stay per-frame for canvas preview) from
**recent-color recording** (should be once, on release). Introduce a small App-level
"pending recent color" seam:

1. Add a field to the `App` struct (near `pending_panel_actions`, ~494 in
   `app/mod.rs`):
   ```rust
   /// Color chosen via the Fill/Stroke picker this interaction, recorded into
   /// `recent_colors` only once the pointer is released (#171) — avoids streaming
   /// the whole drag path into the Recent swatch list.
   pending_recent_color: Option<Color>,
   ```
   Initialize it to `None` in the `App` constructor(s) (~901).

2. In the `UpdateNodeFill` handler, replace the immediate
   `doc.record_recent_color(*c)` with `self.pending_recent_color = Some(*c);` (still
   apply the fill live, unchanged).

3. In the `UpdateNodeStroke` handler, replace the immediate
   `doc.record_recent_color(stroke.color)` with
   `self.pending_recent_color = Some(stroke.color);` (still apply the stroke live).

4. Immediately **after** the `'actions` drain loop closes (still inside `draw()`), add a
   commit-on-release check:
   ```rust
   // #171: commit the picked color to Recent only once the drag ends, so the
   // intermediate colors dragged through the picker don't flood the list.
   if self.pending_recent_color.is_some()
       && ctx.input(|i| i.pointer.any_released())
   {
       if let Some(c) = self.pending_recent_color.take() {
           doc.record_recent_color(c);
           doc_modified = true;
       }
   }
   ```

This makes **every** recolor path commit-on-release, which is also correct for the
discrete-click paths that reuse `UpdateNodeFill` (Recent swatch click, Color Guide
swatch, Recolor): those fire on a frame where the pointer also releases, so they still
record exactly one color (dedup just moves it to front). No change to the picker widgets
themselves is required.

## Scope

**In**
- `crates/photonic-gui/src/app/mod.rs`: new `pending_recent_color` field + init; two
  one-line handler edits; one post-loop release-commit block.

**Out (deferred / not this issue)**
- Undo-history spam: dragging the picker also calls `history.execute(...)` per frame,
  creating many undo steps. Real but a separate concern (coalescing live edits into one
  undo step) — note it, don't fix it here.
- Gradient-stop / fluid / mesh color editing does not feed `record_recent_color` today,
  so no change needed there.
- Hex-field commits end on Enter/focus-loss rather than a pointer release; they still get
  recorded on the next pointer-up anywhere, which is acceptable and not a regression.

## Verification

- `cargo build --release` succeeds.
- Manual: select an object, open the Fill color button, drag around the picker — Recent
  should gain a single swatch on release, not a trail. Repeat for Stroke. Clicking a
  Recent/Color-Guide swatch still records one color.
