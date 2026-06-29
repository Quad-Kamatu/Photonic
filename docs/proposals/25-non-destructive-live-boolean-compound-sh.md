# Non-destructive (live) boolean / compound shapes (#25) — Design Proposal

> Status: design scaffold (not an implementation).

## Summary

Boolean ops today (`crates/photonic-core/src/ops/boolean.rs`) immediately consume
operands via `geo::BooleanOps` and replace them with a flat `PathData`. A Compound Shape
node stores the ordered operands and per-operand boolean mode as children, recomputes the
merged geometry on every mutation, and lets users edit operands directly (Direct Select).

## Scope

**In:**
- New `SceneNodeKind::CompoundShape(CompoundShapeNode)` variant in
  `crates/photonic-core/src/node.rs`.
- `CompoundShapeNode` holds `Vec<(NodeId, BooleanOp)>` — operands + per-operand mode.
  The existing `BooleanOp` enum in `ops/boolean.rs` is reused without change.
- Reactive geometry: call `boolean_op()` chain whenever an operand node changes and
  cache the result `PathData` on the compound node.
- "Expand" command bakes compound → flat `PathNode`, removes the compound wrapper.
- Round-trip through `.photonic` format (serde, no format_version bump needed now).
- Direct-Select into a compound resolves to the operand's own `SceneNode`.
- SVG export: compound renders as a single `<path>` using the baked geometry.

**Out:**
- UI for reordering operands or changing per-operand mode (separate issue).
- Divide into named regions (remains destructive for now; gap doc §J3 note).
- Appearance-stack integration (depends on a separate Appearance epic).

## Proposed approach

1. **Model** (`photonic-core/src/node.rs`): Add `CompoundShape(CompoundShapeNode)` to
   `SceneNodeKind`. `CompoundShapeNode`:
   ```
   pub struct CompoundShapeNode {
       pub operands: Vec<(NodeId, BooleanOp)>,  // ordered; first operand is base
       pub cached_result: Option<PathData>,      // recomputed on dirty
       pub fill: Fill,
       pub stroke: Stroke,
   }
   ```
2. **Evaluation** (`photonic-core/src/ops/boolean.rs`): Add
   `fn eval_compound(doc: &Document, node: &CompoundShapeNode) -> PathData` that folds
   operands left-to-right through the existing `boolean_op()`.
3. **Mutation hook** (`photonic-core/src/history.rs`): After applying any `Command` that
   moves/edits a node, walk parent compound shapes that reference that node and
   invalidate/recompute `cached_result`.
4. **Renderer** (`photonic-render/src/renderer.rs`): `SceneNodeKind::CompoundShape` arm
   in `collect_draw_nodes` renders `cached_result` exactly like a `PathNode`, reusing
   `tessellate_fill` / `tessellate_stroke`.
5. **MCP** (`photonic-mcp/src/handlers/nodes.rs`): `create_compound_shape`,
   `add_compound_operand`, `set_compound_op`, `expand_compound` tools.
6. **Export** (`photonic-core/src/export.rs`): `export_svg` handles
   `SceneNodeKind::CompoundShape` by emitting the cached `PathData` as a `<path>`.

## Affected modules (real paths)

- `crates/photonic-core/src/node.rs` — new variant + struct
- `crates/photonic-core/src/ops/boolean.rs` — `eval_compound` helper
- `crates/photonic-core/src/document.rs` — `collect_draw_nodes`, mutation hooks
- `crates/photonic-core/src/history.rs` — post-command invalidation
- `crates/photonic-render/src/renderer.rs` — render dispatch arm
- `crates/photonic-core/src/export.rs` — SVG export arm
- `crates/photonic-mcp/src/handlers/nodes.rs` — new MCP tool handlers

## Risks & open questions

- **Cycle detection**: an operand could reference its own parent compound — need a guard
  in `eval_compound`.
- **Performance**: deep stacks of many-operand compounds recalculate on every keystroke;
  consider debouncing invalidation or incremental re-evaluation.
- **Bounding box**: `SceneNode::bounding_box()` (node.rs line ~210) must handle the
  `CompoundShape` arm; falls back to the cached result's bounding box.
- **History granularity**: editing an operand inside a compound creates two dirty records
  (the operand edit + the compound recompute) — confirm undo/redo semantics.
- Open: does Direct Select into a compound promote the operand to the top-level selection
  or keep it in compound context? (affects GUI design, not core.)

## Acceptance criteria

- [ ] Creating a compound from two overlapping paths evaluates and renders the boolean
      result live (not a baked path).
- [ ] Moving or editing an operand immediately updates the compound's rendered outline.
- [ ] "Expand" produces a flat `PathNode` geometrically identical to the compound result.
- [ ] Compound round-trips through save/load (`.photonic`).
- [ ] SVG export of a compound emits a `<path>` matching the expanded geometry.
- [ ] Cycle guard prevents infinite loops when operand references its own compound.

## Effort estimate

**L** — Core model change touches the renderer, history, export, and MCP layers; cache
invalidation wiring is non-trivial, but the geo boolean engine already works.
