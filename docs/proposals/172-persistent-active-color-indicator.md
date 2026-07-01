# 172 — Persistent, more-visible active-color indicator (bottom of the icon rail)

## Status — IMPLEMENTED

### What this PR implements

- A persistent, always-visible **active fill-color swatch** pinned to the bottom of the
  left icon rail (`SidePanel::left("drawer_rail")`, `crates/photonic-gui/src/app/mod.rs`).
  It is rendered after the `DrawerGroup::ALL` button loop inside a
  `egui::Layout::bottom_up(egui::Align::Center)` region so it hugs the rail floor
  regardless of how many group buttons are shown.
- The swatch is a fixed 26×26 `color_edit_button_rgba_unmultiplied(&mut self.fill_color)`
  (rail-local `interact_size` override keeps it from spilling past the 40 px rail). On
  `.changed()` it mirrors the new value into `self.prefs.default_fill_color` and calls
  `self.prefs.save()` — the exact inverse of the Tool Defaults handler (mod.rs ~2597),
  so the two controls stay in lockstep and the color persists across sessions.
- Hover tooltip: "Active fill color — click to change".

No new fields, no new preferences, no model/render changes — reuses `App::fill_color`
(mod.rs:336) and `prefs.default_fill_color` (`preferences.rs:35`).

Verification: `cargo build --release` (workspace) passes; `cargo test -p photonic-gui`
passes; `cargo check --workspace` clean (only pre-existing warnings unrelated to #172).

### Remaining work

None for the issue's scope. Explicitly out of scope (unchanged): no stroke-color swatch,
no gradient/fluid/mesh indicator, no swatch history, no right-hand rail (the app has no
right icon rail — see Placement note below).

## Summary

The current **active fill color** (used when drawing pencil/pen/shapes) lives in
`App::fill_color` and its persisted twin `prefs.default_fill_color`. Today the only way
to see or change it is to dig into the **Tool Defaults** flyout of the toolbar
(`crates/photonic-gui/src/app/mod.rs` ~2593–2600), where a `color_edit_button` sits
behind a menu. There is no always-visible readout of "what color will my next stroke
be", which is the gap #172 calls out.

Fix: add a small, always-visible **active-color swatch** pinned to the **bottom of the
left icon rail** (`SidePanel::left("drawer_rail")`), directly editable — clicking it
opens egui's color popup on `App::fill_color` and syncs the change into
`prefs.default_fill_color` (mirroring the existing Tool Defaults handler), then saves
prefs. This gives a persistent, glanceable indicator plus a one-click way to change the
active color.

## Placement note (issue vs. reality)

The issue suggests "the bottom of the icon rail on the **right-hand side**". The app has
**no right icon rail**: the only icon rail is the left `drawer_rail` (a 40 px
`SidePanel::left`, `crates/photonic-gui/src/app/mod.rs:3193`); `SidePanel::right`
("right_panel", ~3386) is a wide Layers/Changelog/AI-chat panel, not an icon rail. The
swatch therefore goes at the **bottom of the existing left icon rail** — the literal
"bottom of the icon rail" — which is the correct home for a 40 px-wide swatch and keeps
the change self-contained. (The always-on statusbar at the bottom, ~3123, is a possible
alternative but is text-oriented and horizontally crowded.)

## Scope

### In

- New helper `draw_rail_active_color(ui, ...)` (or inline block) that renders a swatch at
  the bottom of the `drawer_rail` panel, below the `DrawerGroup::ALL` button loop
  (`crates/photonic-gui/src/app/mod.rs` ~3197–3227), using a bottom-anchored layout so
  it hugs the rail bottom regardless of how many group buttons are shown.
- Swatch is a `color_edit_button_rgba_unmultiplied(&mut self.fill_color)`; on `.changed()`
  push the value into `self.prefs.default_fill_color` and call `self.prefs.save()`
  (same sync direction as the existing Tool Defaults handler at mod.rs:2597–2599, kept
  consistent so the two controls never disagree).
- Hover tooltip ("Active fill color — click to change").

### Out

- No new persisted state or preferences field (reuses `fill_color` /
  `default_fill_color`).
- No stroke-color swatch, no gradient/fluid/mesh indicator, no swatch history — fill
  only, per the issue.
- No relocation to a right-hand rail and no new rail creation.
- No changes to how `fill_color` is applied to drawing tools.

## Approach (grounded)

1. In the `drawer_rail` closure (`crates/photonic-gui/src/app/mod.rs:3197`), after the
   `for group in DrawerGroup::ALL` loop that draws the top buttons, add a
   bottom-anchored region. egui renders top-down, so pin the swatch using
   `ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| { … })` (or
   allocate the remaining vertical space, then draw), so the swatch stays flush to the
   rail bottom.
2. Draw a fixed-size (~26×26) `color_edit_button_rgba_unmultiplied(&mut self.fill_color)`.
   On `.changed()`: `self.prefs.default_fill_color = self.fill_color; self.prefs.save();`
   Add `.on_hover_text(...)`.
3. Keep the existing Tool Defaults control as-is; both now write the same two fields, so
   editing either keeps `fill_color` and `default_fill_color` in lockstep.
4. `cargo build --release` and `cargo test -p photonic-gui` must pass.

## Files to touch

- `crates/photonic-gui/src/app/mod.rs` — the `SidePanel::left("drawer_rail")` block
  (~3193–3227); reuses existing `self.fill_color` (mod.rs:336) and
  `prefs.default_fill_color` (`crates/photonic-gui/src/preferences.rs:35`).

Single file, single crate (`photonic-gui`). No model/render changes.

## Round-1 adversarial fixes (post-implementation)

- **[major] Per-frame synchronous `prefs.save()` on color-popup slider drag** —
  The initial implementation called `self.prefs.save()` on every `resp.changed()`
  frame. Because `color_edit_button_rgba_unmultiplied` drives an RGBA-slider popup
  whose `.changed()` fires on every dragged frame, a single color pick fired dozens
  of full `serde_json::to_string_pretty` + blocking `std::fs::write` cycles per
  second on the UI thread (frame hitches + needless SSD writes) — the same problem
  #171 fixed for recent-colors.
  **Fix:** on change, mirror in-memory only (`prefs.default_fill_color = fill_color`)
  and set a new `fill_swatch_dirty` flag; flush a single `prefs.save()` on the frame
  the picker interaction settles (`ui.input(|i| i.pointer.any_released())`), reusing
  the #171 commit-on-release idiom. The misleading comment that claimed byte-for-byte
  lockstep with the Tool Defaults sibling (mod.rs ~2597, which deliberately does *not*
  save) was corrected to describe the mirror relationship and the deferred write.

No deferrals — the finding is fully addressed in-tree.
