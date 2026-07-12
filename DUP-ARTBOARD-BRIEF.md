# Fix duplicate_artboard: capture text nodes + carry gradients on translate

You are in MAIN Photonic on `main`. Two bugs in the recently-added
`duplicate_artboard` / `move_artboard` (and shared with `duplicate_nodes` /
`apply_transform`). Handlers likely in `crates/photonic-mcp/src/handlers/` +
`photonic-core`.

## Bug 1 — text nodes are NOT captured by artboard membership
`duplicate_artboard` clones an artboard + "all content inside it," but TEXT nodes are
excluded. A duplicated Card Front comes out with bg/grid/accent-rule/logo/slashes
(reports "6 root nodes") but NO name/role/email/url text. Geometry capture works; text
is missed.
Root cause (likely): the membership test uses each node's bounding box, and a text
node's bbox is computed from glyph layout — reported as empty/zero or anchored at the
origin instead of at the node's transform position — so the "inside/overlaps the
artboard rect" check fails for text.
Fix: make membership text-aware. Include a node when its transform position (plus a
glyph-measured extent) lies within / overlaps the artboard rect. A text node placed
inside the artboard MUST be captured and offset like every other node.

## Bug 2 — gradients don't move with the node on translate/duplicate
Gradient fills use `userSpaceOnUse` coordinates (absolute document `x0,y0→x1,y1`). When
a node is offset (`duplicate_artboard`, `move_artboard`, `duplicate_nodes`,
`apply_transform` translate), the GEOMETRY moves but the gradient coords stay fixed — so
the copy's shape slides while its gradient sweep stays anchored at the original spot,
making the duplicate look wrong/solid.
Fix: whenever an operation translates a node, translate its fill AND stroke gradient's
`userSpaceOnUse` coords by the SAME `(dx, dy)`. Apply consistently across
`duplicate_artboard`, `move_artboard`, `duplicate_nodes`, and `apply_transform`
translate. For a full transform (matrix/scale/rotate), transform the gradient coords by
the same matrix. Radial gradients: transform center + radius/coords likewise.

## Reproduce / verify (add tests)
- Doc with an artboard at (0,0) containing a text node + a `userSpaceOnUse`
  gradient-filled path. `duplicate_artboard` offset (0,700):
  - assert the copy CONTAINS a text node at the offset position (text duplicated),
  - assert the copy's gradient coords are offset by (0,700) — the gradient renders on
    the copy identically to the original.
- `move_artboard` (dx,dy): text moves AND gradient moves with it.
- Regression: geometry capture still works, undo stays a single entry per op, existing
  tests pass. `cargo build --workspace` + `cargo test --workspace` green.
Commit to `main`. Report root causes, fixes, and verify results.
