# Close the visual gap between the icon rail and the selected drawer (#168)

> Status: **implemented**. Small, self-contained GUI layout fix.

## What this PR implements

Both fixes landed in `crates/photonic-gui/src/app/mod.rs`:

- **Rail panel** (`SidePanel::left("drawer_rail")`): a custom `.frame(...)` built
  from `Frame::side_top_panel(&ctx.style())` (preserves panel fill) with
  `inner_margin = Margin { left: 5.0, right: 0.0, top: 2.0, bottom: 2.0 }`. The
  8 px right margin is now 0, so the icon buttons hug the drawer edge.
- **Drawer panel** (`SidePanel::left("properties")`): a custom `.frame(...)`, also
  from `Frame::side_top_panel(&ctx.style())`, with
  `inner_margin = Margin { left: 0.0, right: 8.0, top: 2.0, bottom: 2.0 }`. The
  frame is set on the base `let mut panel = egui::SidePanel::left("properties")`
  binding *before* the `fully_open` / mid-tween reassignments, so it carries
  through both the `.resizable(true)` branch and the `.exact_width(...)` tween
  branch. The 8 px left margin is now 0, so drawer content starts flush at the
  rail edge.

Net effect: the previous ~16 px dead band (8 px rail right + 8 px drawer left)
collapses to 0. No changes to drawer width, animation timing, easing, or the
resize handle. `fill` is preserved on both frames via `ctx.style()`.

Verified: `cargo build --release` (workspace) succeeds; `cargo test -p
photonic-gui` passes (0 tests, lib compiles); `cargo check --workspace` clean.
Joseph verifies the GUI visually.

## Remaining work

None for this issue. A faint 1 px panel separator line may remain at the rail
edge — that reads as an intentional divider, not a gap, and is out of scope.

## Summary

The left icon rail (`SidePanel::left("drawer_rail")`) and the drawer it opens
(`SidePanel::left("properties")`) are two adjacent egui side panels. Each uses
the default panel frame, `Frame::side_top_panel`, whose `inner_margin` is
`Margin::symmetric(8.0, 2.0)` (egui 0.29.1). That means the rail contributes an
8 px right inner margin and the drawer an 8 px left inner margin, leaving a
~16 px empty band between the rail buttons and the drawer content. The user
perceives this as a gap. Tighten it so the drawer content sits flush against
the rail.

Both panels are built in `crates/photonic-gui/src/app/mod.rs`:
- Rail: line ~3174, `SidePanel::left("drawer_rail").exact_width(40.0)` — no
  custom frame, 30 px buttons centered inside a 40 px panel with 8 px side
  margins.
- Drawer: line ~3230, `let mut panel = egui::SidePanel::left("properties");`
  then resizable/exact-width branches, no custom frame.

## Scope

**In**
- Give the drawer panel (`"properties"`) a custom frame that removes its left
  inner margin (set left to 0, keep right/top/bottom at their current values,
  e.g. `Margin { left: 0.0, right: 8.0, top: 2.0, bottom: 2.0 }`) so its content
  starts flush at the rail edge.
- Trim the rail panel's right inner margin (e.g. `Margin { left: 5.0, right:
  0.0, top: 2.0, bottom: 2.0 }`) so the rail buttons hug its right edge.
- Apply the frame on the base `panel` binding before the fully-open / mid-tween
  branches so it carries through both `.resizable(true)` and
  `.exact_width(...)` paths (and both frames use `ctx.style()` for correct fill).

**Out**
- No change to drawer width, animation timing, easing, or the resize handle.
- No restyling of the right panel, top bars, or drawer internal padding beyond
  the single left-margin removal.
- No change to `DrawerGroup` model, rail icons, or `has_content` gating.

## Approach

1. In `crates/photonic-gui/src/app/mod.rs`, add `.frame(...)` to the rail
   `SidePanel::left("drawer_rail")` builder (line ~3174) with a reduced right
   margin as above, keeping `fill: style.visuals.panel_fill` via
   `Frame::side_top_panel(&ctx.style())` before overriding `inner_margin`.
2. On the drawer, set the custom frame on the initial
   `let mut panel = egui::SidePanel::left("properties");` (line ~3230) with
   `left: 0.0`, so the subsequent `panel = if fully_open { panel.resizable(...) }
   else { panel.exact_width(...) }` reassignments preserve it.
3. Verify visually: rail buttons and drawer content should read as one
   continuous surface with no dead band. If a faint separator line remains from
   the rail panel edge, it is acceptable (it reads as the divider, not a gap);
   only remove margin whitespace.
4. `cargo build --release` must succeed; then Joseph launches the GUI to confirm.

## Files to touch
- `crates/photonic-gui/src/app/mod.rs` (rail + drawer `SidePanel` builders,
  ~lines 3174 and 3230–3241).
