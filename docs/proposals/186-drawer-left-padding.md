# 186 — Left drawer content needs small left padding (follow-up to #168)

## Status: Implemented
This PR changes the drawer frame's `inner_margin.left` from `0.0` to `6.0` at
`crates/photonic-gui/src/app/mod.rs` (the `drawer_frame` block, ~L3370-3382),
set on the base `Frame` binding so it applies to both the resizable and
exact-width tween panel branches. `right/top/bottom` (8/2/2) are unchanged, and
the adjacent comment now explains the #186 correction. The icon rail's own
frame/margins (`mod.rs:3199`, `mod.rs:3260`) were left untouched so the rail
buttons keep hugging the edge — the gutter is added only inside the drawer.
Verified with `cargo build --release`. Joseph verifies the GUI visually.

### Remaining work
None. This is a single-value styling fix fully covered by the change above.

## Summary
When an icon-rail drawer opens on the left (`SidePanel::left("properties")`), its
content sits flush against the icon rail with no breathing room. #168 (commit
`0bd3330`) zeroed the drawer frame's `inner_margin.left` to close a ~16 px dead
band, but that overshot — the content now literally touches the rail. Restore a
modest left pad (~6 px), well short of the old 16 px gap, so the drawer content
breathes without reopening a dead band between rail and drawer.

## Scope

### In
- `crates/photonic-gui/src/app/mod.rs:3372-3377` — the `drawer_frame`
  `inner_margin.left`, change `0.0` → `6.0`.
- Refresh the adjacent comment (currently references "close the #168 gap") to
  note the #186 correction.

### Out
- The icon rail's own frame/margins (`mod.rs:3199`, `mod.rs:3260`) stay untouched —
  the rail buttons must keep hugging the edge; padding is added only inside the
  drawer.
- No change to drawer width, tween/resize logic, or panel structure.
- Other `inner_margin` sites elsewhere in the file are unrelated.

## Approach
Single-value edit inside the `drawer_frame` block. `inner_margin.left` is set on
the base `Frame` binding, so it flows through both the resizable and exact-width
tween panel branches — one change covers all open states. `right/top/bottom`
(8/2/2) are unchanged. Pick `6.0` (within the issue's suggested 6–8 px band) for a
subtle gutter. Then `cargo build --release` per house rule; Joseph verifies the
GUI visually.
