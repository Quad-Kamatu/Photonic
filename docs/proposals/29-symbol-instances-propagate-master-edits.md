# Symbol instances: propagate master edits + nested symbols (#29) — Design Proposal

> Status: design scaffold (not an implementation).

## Summary

The `Symbol` struct (`document.rs` line 125-141) names a `master_node_id`. Instances are
`SceneNode`s with `symbol_ref: Option<Uuid>` (node.rs line 164) plus fill/stroke
overrides (lines 167-170). The problem: `collect_draw_nodes` (document.rs line 802)
pushes the instance node's own `SceneNodeKind` to the draw list — the master is never
consulted. Editing the master therefore has no visible effect on instances. Nested symbols
are unguarded (infinite loops possible). This proposal rewires rendering to resolve
instances by reference and adds cycle protection.

## Scope

**In:**
- Renderer resolves `symbol_ref` by looking up the symbol's `master_node_id` in
  `doc.symbols` and rendering the master node tree (with the instance's transform and
  overrides applied on top).
- Nested symbol support: instance → master (which itself has `symbol_ref`) → its master,
  etc., with a depth/cycle guard.
- Per-instance overrides (current `symbol_fill_override`, `symbol_stroke_override`)
  applied on top of master fill/stroke during render.
- `break_link_to_symbol` (if it exists in MCP handlers/nodes.rs) must still produce a
  correct independent copy of the master after this change.
- Headless capture uses the same resolution path as the live renderer.
- SVG export resolves symbols to flat geometry (or emits `<use>` with `<defs>` — decide
  below).

**Out:**
- Richer override types beyond fill/stroke colour (e.g. per-child overrides, text
  content overrides — separate feature).
- Symbols panel UI revamp.
- Nested symbol editing in-place (edit the nested master from within an outer instance).

## Proposed approach

1. **Resolution helper** (`photonic-core/src/document.rs`):
   ```rust
   pub fn resolve_symbol_node<'a>(
       &'a self, node: &'a SceneNode, depth: u8
   ) -> Option<&'a SceneNode>
   ```
   Returns the master `SceneNode` for a symbol instance, guarding `depth > 8` (returns
   `None`, treated as "not renderable" — logged as a warning). Checks `doc.symbols` for a
   `Symbol` whose `id == node.symbol_ref?`, then returns
   `doc.nodes.get(&symbol.master_node_id)`.

2. **Draw order / rendering** (`photonic-core/src/document.rs`, `collect_draw_nodes`):
   When a node has `symbol_ref.is_some()`, instead of pushing the instance node, call
   `resolve_symbol_node(node, depth)` and push the master (recursively). The instance's
   transform is already on the instance `SceneNode`; the master's own transform is
   composed on top during render.

3. **Renderer** (`photonic-render/src/renderer.rs`): the render loop (line ~746) pulls
   nodes from `nodes_in_draw_order()`; if the draw-order list now contains master nodes
   (from step 2), overrides need to reach the render. Two options:
   - **Option A (simpler)**: produce a `ResolvedSnapshot` that merges the master's
     `PathNode` fill/stroke with the instance's overrides at collection time.
   - **Option B**: pass overrides as a side-channel alongside the node.
   Option A is preferred to avoid touching the render hot-path.

4. **Override application**: in `collect_draw_nodes`, when substituting the master, create
   a shallow clone of the master `SceneNode` and apply `symbol_fill_override` /
   `symbol_stroke_override` from the instance. This is a temporary render clone, not a
   document mutation.

5. **Cycle guard**: before recursing into a master's children (for group masters), check
   that the master's `NodeId` is not already in a per-call visited set.

6. **`break_link_to_symbol`** (MCP handler): deep-clone the master node tree into the
   document as new independent nodes, assign the instance's transform, clear `symbol_ref`.
   Must work correctly after the rendering change (master tree may itself contain
   `symbol_ref` nodes — clone recursively, flattening inner symbols).

7. **SVG export** (`photonic-core/src/export.rs`): emit `<defs><g id="sym_<id>">...</g></defs>`
   for each symbol's master, then `<use href="#sym_<id>" transform="..."/>` for each
   instance. Apply override fill/stroke as inline `style` on the `<use>` element.

## Affected modules (real paths)

- `crates/photonic-core/src/document.rs` — `resolve_symbol_node`, `collect_draw_nodes`
- `crates/photonic-render/src/renderer.rs` — override-merged snapshot collection
- `crates/photonic-core/src/export.rs` — `<defs>` + `<use>` SVG emission
- `crates/photonic-mcp/src/handlers/nodes.rs` — `break_link_to_symbol`, `set_symbol_override`
  (already exists at line 16817)

## Risks & open questions

- **Transform composition**: the instance's `SceneNode.transform` must wrap the master's
  own transform matrix. Verify `Transform::compose` (transform.rs) is correct for this.
- **Group masters**: if the master is a `GroupNode`, all its children must be collected
  and rendered with overrides cascaded — this makes Option A more complex (need to
  traverse the subtree and apply overrides to leaf `PathNode` fills).
- **Master off-canvas**: masters may live on a hidden layer or at coordinate (0,0).
  Instances must ignore the master's layer visibility and position, using only its
  geometry and style.
- **Undo**: editing the master now visually updates all instances; undo of the master edit
  should instantly revert all instances (already correct if instances just reference the
  master at render time).
- Open: should masters be stored off-canvas on a dedicated "Symbols" layer or anywhere in
  the layer stack? Convention is a hidden symbols layer — the issue doesn't specify.

## Acceptance criteria

- [ ] Editing a master node's geometry or fill updates all placed instances live on canvas
      and in headless export.
- [ ] Per-instance fill/stroke overrides (`set_symbol_override`) remain applied after a
      master edit.
- [ ] Nested symbols (instance A references master B which is itself an instance of
      master C) render correctly without infinite loops.
- [ ] `break_link_to_symbol` produces an independent copy identical in appearance to the
      instance, correctly reflecting the current master.
- [ ] SVG export emits `<defs>` + `<use>` elements with correct transforms.

## Effort estimate

**M** — The resolution logic is clear; the main complexity is transform composition for
nested cases and correct override cascading through group masters.
