# Fix undo of node/layer deletion silently no-oping (#153)

> Status: **implemented**
> Area: core (undo/redo engine). Type: bug.

## What this PR implements

All in `crates/photonic-core/src/history.rs`, additive to the public `Command`
enum with zero call-site edits:

- **New `Command::RemoveNodeFull { node: SceneNode }`** variant mirroring the
  existing `RemoveLayerFull { layer }`. Its `apply` calls `doc.remove_node(&node.id)`;
  its `inverse` returns `AddNode { node, layer_id: Some(node.layer_id) }`, which
  also fixes the secondary bug where the old bare-`RemoveNode` inverse used
  `layer_id: None` and re-homed the undeleted node to the *active* layer instead
  of its original layer. A `description` arm (`"Remove {name}"`) is included.
- **`Command::hydrate(self, doc: &Document) -> Command`** rewrites bare
  `RemoveNode { node_id }` → `RemoveNodeFull { node }` and
  `RemoveLayer { layer_id }` → `RemoveLayerFull { layer }` while the entity still
  exists, recursing into `Batch`. Absent entities pass through unchanged; all
  other variants pass through.
- **`History::execute` calls `cmd.hydrate(doc)`** immediately before `cmd.apply`,
  so the entry pushed onto the undo stack — and therefore the persisted `.photon`
  history — is always self-contained and invertible. `undo`/`redo`/serialization
  were not otherwise touched.
- **Legacy `RemoveNode` / `RemoveLayer` variants retained** for backward-compatible
  deserialization of older `.photon` files and direct `apply`.
- **Tests added** (all passing, `cargo test -p photonic-core`): node and layer
  delete→undo→redo round-trips; a node delete inside a `Batch`; a test asserting
  the undeleted node returns to its *original* (non-active) layer; and a hydration
  test asserting the pushed undo-stack entry is the `*Full` form (standalone and
  inside a `Batch`).

Verification: `cargo build --release` ✓, `cargo test -p photonic-core` ✓
(295 passed), `cargo check --workspace` ✓.

## Remaining work

Deferred, unchanged from the original scope below:

- **Child nodes of a removed layer are not restored.** `remove_layer` drops the
  layer's nodes; neither the pre-existing `RemoveLayerFull` nor this change
  captures them, so layer-delete undo restores the empty layer only. Full
  layer-with-contents undo is a separate follow-up (carry the layer's nodes in
  the payload).
- **Exact z-order index of an undeleted node is not restored.** `add_node`
  appends to the end of `layer.node_ids`, matching current
  `RemoveLayerFull`/`AddNode` behavior.

## Summary

Undo of a node or layer **deletion** silently no-ops. `Command::RemoveNode` and
`Command::RemoveLayer` compute their inverse by reading the entity **out of the
current document** (`crates/photonic-core/src/history.rs`, `Command::inverse`),
but by the time `inverse()` runs during `undo()` the entity has already been
deleted, so the lookup returns `None`. `undo()` then re-pushes the command and
returns `false` — a silent no-op. A `Command::Batch` containing a `RemoveNode`
is equally un-undoable, because `Batch::inverse` propagates the `None` via
`cmd.inverse(doc)?`, so the whole batch fails to invert.

`RemoveLayerFull { layer }` already does this correctly: it carries the full
`Layer` payload, so its inverse is self-contained and needs no document lookup.
The fix generalizes that pattern to node/layer removals — and, because commands
are now persisted in `.photon`, makes the persisted history self-contained so a
top-of-stack delete survives reopen and stays undoable.

## Root cause

`Command::inverse` (history.rs ~945):

```rust
Command::RemoveNode { node_id } => {
    let node = doc.nodes.get(node_id)?.clone();   // already removed → None
    Some(Command::AddNode { node, layer_id: None })
}
Command::RemoveLayer { layer_id } => {
    let layer = doc.layers.get(layer_id)?.clone(); // already removed → None
    Some(Command::AddLayer { layer })
}
```

`Document::remove_node` / `remove_layer` (`document.rs:979` / `:937`) fully drop
the entry before the inverse is ever computed.

Secondary defect: even when the lookup *did* succeed, the inverse produced
`AddNode { node, layer_id: None }`, which `Document::add_node` re-homes to the
**active** layer rather than the node's original layer. The self-contained
inverse fixes this by passing `layer_id: Some(node.layer_id)`.

## Scope

### In
- New self-contained `Command::RemoveNodeFull { node: SceneNode }` variant,
  mirroring the existing `RemoveLayerFull { layer }`.
- A `hydrate(&Document)` normalization step invoked once at the single choke
  point `History::execute` (and recursively inside `Batch`) that rewrites bare
  `RemoveNode { node_id }` → `RemoveNodeFull { node }` and
  `RemoveLayer { layer_id }` → `RemoveLayerFull { layer }` **before** `apply`,
  while the entity still exists. The stack (and therefore the persisted
  `.photon` history) then only ever holds the self-contained forms.
- `apply` / `inverse` / `description` arms for `RemoveNodeFull`. Inverse:
  `AddNode { node, layer_id: Some(node.layer_id) }` (restores original layer).
- Keep the legacy `RemoveNode` / `RemoveLayer` variants intact for backward-
  compatible deserialization of older `.photon` files and direct `apply`.
- Tests: delete→undo→redo round-trips for a node and a layer, both standalone
  and wrapped in a `Batch`, plus a hydration test asserting the pushed stack
  entry is the `*Full` form.

### Out
- Restoring the **child nodes** of a removed layer. `remove_layer` also drops
  the layer's nodes; neither the existing `RemoveLayerFull` nor this change
  captures them, so layer-delete undo restores the empty layer only. Full
  layer-with-contents undo is a separate follow-up (would carry the layer's
  nodes in the payload).
- Restoring exact **z-order index** of an undeleted node; `add_node` appends to
  the end of `layer.node_ids`, matching current `RemoveLayerFull`/`AddNode`
  behavior. Out of scope here.
- Any change to the ~40 call sites that build `Command::RemoveNode { node_id }`
  (eraser `erase_tools.rs`, `tool_handlers.rs`, pathfinder/boolean ops in
  `app/mod.rs`, MCP `handlers/nodes.rs`, `repl.rs`, `script.rs`). The
  choke-point `hydrate` means none of them need touching.

## Approach

Fix at the single choke point rather than at every call site.

1. **Add the variant** to `enum Command` (history.rs ~623):
   `RemoveNodeFull { node: SceneNode }`.

2. **`apply`**: `RemoveNodeFull { node } => { doc.remove_node(&node.id); }`.

3. **`inverse`**:
   `RemoveNodeFull { node } => Some(Command::AddNode { node: node.clone(), layer_id: Some(node.layer_id) })`.

4. **`description`**: `format!("Remove {}", node.name)` (mirrors `RemoveLayerFull`).

5. **Add `fn hydrate(self, doc: &Document) -> Command`** (or `&mut self`) that:
   - `RemoveNode { node_id }` → if `doc.nodes.get(&node_id)` is `Some(n)`,
     return `RemoveNodeFull { node: n.clone() }`; else return unchanged.
   - `RemoveLayer { layer_id }` → if present, return `RemoveLayerFull { layer }`;
     else unchanged.
   - `Batch(cmds)` → map `hydrate` over each element (recursive).
   - everything else → unchanged.

6. **Call it in `execute`** (history.rs ~1445): `let cmd = cmd.hydrate(doc);`
   immediately before `cmd.apply(doc)`, so the pushed command is self-contained.
   `undo` / `redo` / serialization are unchanged and now always see invertible
   commands.

Because `undo` re-pushes `cmd` onto `redo_stack` and `redo` re-applies it, and
both stacks are populated only through `execute` (plus snapshot restore), the
hydrated form flows correctly through the entire undo/redo/persist cycle.

## Blast radius

Isolated to `crates/photonic-core/src/history.rs`. No call-site edits; the
public `Command` enum only gains a variant (additive). House rule:
`cargo build --release` after the edit, then run the new + existing history
tests (`cargo test -p photonic-core history`).
