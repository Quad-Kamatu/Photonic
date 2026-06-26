# Live property constraints (parametric binding between nodes) (#28) — Design Proposal

> Status: design scaffold (not an implementation).

## Summary

Nodes are positioned by literal coordinates; nothing ties one node's property to
another. `Document` already has two related-but-different mechanism seeds:
`DocumentVariable` (text binding, document.rs line 148) and `GrammarRule` (design
validation, document.rs line 331). Neither is a constraint engine. This proposal adds a
`PropertyConstraint` list to `Document` that stores `(node_id, property) = expression`
bindings, evaluates them reactively in topological order after every mutation, and
exposes them via GUI lock indicators and MCP tools.

## Scope

**In:**
- `PropertyConstraint` stored on `Document`: target node+property, expression string
  referencing other nodes/properties (e.g. `"nodes['b'].width * 2"`).
- Simple expression language: arithmetic (`+ - * /`), numeric literals, property
  references (`nodes['<id>'].<prop>`), constants. No full scripting.
- Supported target properties initially: `x`, `y`, `width`, `height`, `opacity`,
  `font_size` (for `TextNode`).
- Reactive evaluation: after any mutation-bearing `Command` in `history.rs`, re-evaluate
  all constraints in dependency order; constrained properties are set as a derived write
  (not user-editable until constraint removed).
- Cycle detection: report a named error, leave affected properties at their last valid
  values, do not crash.
- Constraints persisted in `.photonic` under a new `constraints` field on `Document`.
- MCP tools: `set_constraint`, `list_constraints`, `remove_constraint`.
- GUI: lock icon on constrained property fields in the Properties panel; formula tooltip.

**Out:**
- Full scripting / user-defined functions.
- Geometry constraints (e.g. parallel lines, tangency) — purely numeric for now.
- Constraint-based layout system (separate from the existing flex/grid layer concepts).
- Real-time constraint solving (Cassowary/SMT); this is forward-evaluation only.

## Proposed approach

1. **Model** (`photonic-core/src/document.rs`):
   ```rust
   pub struct PropertyConstraint {
       pub id: Uuid,
       pub target_node_id: NodeId,
       pub target_property: String,  // "x", "y", "width", "height", "opacity", "font_size"
       pub expression: String,       // human-readable; parsed on evaluation
   }
   ```
   Add `pub constraints: Vec<PropertyConstraint>` to `Document`.

2. **Evaluator** (new file `photonic-core/src/ops/constraints.rs`):
   - Parse expression with a minimal recursive-descent parser or use an existing Rust
     expression crate (e.g. `evalexpr` — already in Rust ecosystem, no unsafe).
   - `fn evaluate_constraints(doc: &mut Document) -> Result<(), ConstraintError>`:
     (a) build dependency graph from expressions,
     (b) topological sort (Kahn's algorithm),
     (c) on cycle: collect cycle members, return `ConstraintError::Cycle(Vec<Uuid>)`,
     (d) evaluate in order, apply each result to the target node's property via a
         dedicated setter (not a full `Command` — avoid history re-entrancy).

3. **Mutation hook** (`photonic-core/src/history.rs`):
   After `apply_command`, call `evaluate_constraints` if `doc.constraints` is non-empty.
   Store any constraint errors in a transient `doc.constraint_errors` field (`#[serde(skip)]`).

4. **MCP** (`photonic-mcp/src/handlers/document.rs` or a new `constraints.rs` handler):
   `set_constraint(node_id, property, expression)` — validates target node+property
   exist, adds to `doc.constraints`, triggers evaluation.
   `list_constraints()` — returns current list with evaluated current values.
   `remove_constraint(constraint_id)`.

5. **GUI** (`photonic-gui`): Properties panel shows a lock icon beside constrained
   fields; clicking shows and allows editing the expression string; error state shown
   inline when a cycle is detected.

## Affected modules (real paths)

- `crates/photonic-core/src/document.rs` — `PropertyConstraint` struct, `Document` field
- `crates/photonic-core/src/ops/constraints.rs` — new file: parser + evaluator
- `crates/photonic-core/src/ops/mod.rs` — expose new module
- `crates/photonic-core/src/history.rs` — post-command evaluation hook
- `crates/photonic-mcp/src/handlers/` — new constraint handlers
- `crates/photonic-gui` — Properties panel lock UI

## Risks & open questions

- **Expression parser complexity**: a minimal arithmetic + property-ref grammar is
  manageable; adding a crate dependency (`evalexpr`) may be cleaner but adds a dep.
- **Re-entrancy in history**: `evaluate_constraints` writes node properties without
  creating `Command` records; this means constraint-derived writes are invisible to undo.
  Decide: should undo restore the pre-constraint values (requires recording them), or
  simply re-evaluate after undo (simpler, preferred)?
- **Property writability**: some properties (e.g. `width` on a group) are derived from
  children; can a constraint forcibly set them? Probably not — need an allowlist.
- **Performance**: if many constraints exist, evaluate after every keystroke during path
  editing could be expensive; consider debouncing to animation frame boundary.
- Open: should constraints be evaluated inside the MCP server (server-side) or only in
  the app (client-side)? MCP editing a node should trigger re-evaluation.

## Acceptance criteria

- [ ] Setting `rect_b.width = nodes['rect_a'].width * 2` causes rect_b to update when
      rect_a is resized.
- [ ] A cycle between two constraints is detected and reported with both node/property
      names; the document remains editable.
- [ ] Constraints round-trip through `.photonic` save/load.
- [ ] `set_constraint`, `list_constraints`, `remove_constraint` MCP tools work.
- [ ] Constrained properties show a lock indicator in the Properties panel.

## Effort estimate

**L** — Expression parsing is the non-trivial part; the mutation hook and topological
evaluator are bounded in scope, but the GUI lock UX and undo semantics need careful design.
