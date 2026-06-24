---
name: photonic-workflow
description: Use when actively drawing in a Photonic session — after a composition brief is written. Covers optimal MCP tool sequence, draw order, screenshot cadence, naming conventions, and error recovery.
---

# Photonic Workflow

## Overview

Encodes the proven tool call sequence for a Photonic drawing session. Eliminates wasted calls, prevents z-order chaos, and ensures the session ends with a clean, well-organized document.

**Core principle:** Get document state first. Draw back-to-front. Group immediately after each component. Screenshot after each component, not after each node.

---

## Session Phases

### Phase 1 — Orient

```
1. get_document_state          → understand what already exists
2. create_layer (×N)           → create all layers from the composition brief (back-to-front order)
3. screenshot                  → confirm blank canvas / starting state
```

Never skip `get_document_state`. Even on a new session, it confirms canvas dimensions and layer state.

---

### Phase 2 — Draw (Repeat Per Component)

For each component in the composition brief, follow this loop:

```
DRAW LOOP (one component at a time)
  1. Draw all nodes for this component, back-to-front within the component
  2. Apply boolean operations for this component if flagged (e.g., head+body union)
  3. Reorder nodes within layer if any z-order corrections needed
  4. Group all nodes for this component → assign group name from brief
  5. screenshot → review, correct if needed before moving to next component
```

Component order follows the layer stack:
```
background shapes → base silhouettes → overlay markings → detail features → highlights
```

---

### Phase 3 — Finalize

```
1. get_document_state           → audit: check all planned groups exist, all layers populated
2. Group any remaining ungrouped loose nodes
3. screenshot                   → final review
```

---

## Tool Sequence Reference

### Starting a Session

```
get_document_state
create_layer (name="background")
create_layer (name="base")
create_layer (name="overlay")
create_layer (name="detail")
create_layer (name="highlight")
screenshot
```

### Creating a Node

Always provide `layer_id` and `name` — never create anonymous nodes.

```
create_shape
  layer_id: <id of target layer>
  name:     "{component}_{part}_{qualifier}"   ← see Naming below
  fill:     <fill object from palette — see fill types below>
  stroke:   {hex or null}
```

**Fill objects — all supported types:**

```jsonc
// Solid
{"type": "solid", "color": "#5BA4D4"}

// Linear gradient — coords: [start_x, start_y, end_x, end_y] in document space
{"type": "gradient", "gradient_type": "linear", "colors": ["#87CEEB", "#FFFFFF"], "coords": [512, 0, 512, 1024]}

// Radial gradient — coords: [center_x, center_y, radius] in document space
{"type": "gradient", "gradient_type": "radial", "colors": ["#FFFFFF", "#5BA4D4"], "coords": [200, 300, 80]}

// Fluid gradient — free-placed control points, blended by inverse distance
{"type": "fluid_gradient", "points": [{"x": 100, "y": 100, "color": "#ff6b6b"}, {"x": 500, "y": 300, "color": "#4ecdc4"}], "power": 2.0}

// Mesh gradient — rows×cols grid, vertices in row-major order (left→right, top→bottom)
{"type": "mesh_gradient", "rows": 2, "cols": 2, "vertices": [
  {"x": 0,   "y": 0,   "color": "#ff0000"},
  {"x": 200, "y": 0,   "color": "#00ff00"},
  {"x": 0,   "y": 200, "color": "#0000ff"},
  {"x": 200, "y": 200, "color": "#ffff00"}
]}

// No fill (stroke-only)
{"type": "none"}
```

**Gradient coordinate rule:** Always use document-space coordinates, not percentages. Compute from the shape's x/y/width/height as planned in the composition brief.

### After Completing a Component

```
group_nodes
  node_ids: [all node ids for this component]
  name:     "{component}_group"
screenshot
```

### Before a Boolean Operation

```
get_node (node_id: <target>)   → confirm it is a path node
get_node (node_id: <tool>)     → confirm it is a path node
boolean_operation
  target_id: <base shape — inherits fill/stroke>
  tool_id:   <cutter shape — consumed>
  operation: union | subtract | intersect | exclude
  keep_originals: false
```

**Important:** `boolean_operation` only works on path nodes (created via `create_path` or `build_shape_from_points`). Primitive shapes (`create_shape`) must be converted or redrawn as paths if you need to boolean them.

### Z-Order Corrections

Draw back-to-front to minimize this. When corrections are needed:

```
reorder_node
  node_id:   <node to move>
  operation: send_to_back | bring_to_front | send_backward | bring_forward | move_above | move_below
  relative_id: <other node>   ← required for move_above / move_below
```

### Applying Transforms

```
apply_transform
  node_ids:  [<id>, ...]
  operation: translate | rotate | scale | reflect_horizontal | reflect_vertical | matrix
```

**Per-operation parameters:**

```jsonc
// Translate — move by delta
{ "operation": "translate", "node_ids": ["id1"], "translate": { "x": 50, "y": -20 } }

// Rotate — degrees clockwise; cx/cy are pivot point in document space (default: shape center)
{ "operation": "rotate", "node_ids": ["id1"], "rotate": { "angle": 45, "cx": 512, "cy": 512 } }

// Scale — sx/sy are scale factors; cx/cy are pivot (default: shape center)
{ "operation": "scale", "node_ids": ["id1"], "scale": { "sx": 1.5, "sy": 0.8, "cx": 200, "cy": 300 } }

// Reflect — no extra params needed; flips in place
{ "operation": "reflect_horizontal", "node_ids": ["id1"] }
{ "operation": "reflect_vertical",   "node_ids": ["id1"] }

// Matrix — 6-element affine [a, b, c, d, e, f] (SVG matrix order)
{ "operation": "matrix", "node_ids": ["id1"], "matrix": [1, 0, 0, 1, 50, -20] }
```

`apply_transform` accepts multiple `node_ids` — fan/rotate a group of shapes in one call.

---

### Deleting Nodes

```
delete_nodes
  node_ids: ["id1", "id2"]
```

Use when: removing placeholder shapes, cleaning up boolean inputs if `keep_originals: true` was set, or discarding a component you want to redo. Prefer `undo` for accidental creates — `delete_nodes` does not preserve undo history of the deleted node's edits.

---

### Ungrouping

```
ungroup_nodes
  group_id: "<group node id>"
```

Dissolves the group and returns children to the layer at the group's former z-position. Use when you need to reorder or individually edit nodes that were prematurely grouped.

---

### Error Recovery

```
undo (steps: 1)               → undo last operation
undo (steps: N)               → undo N operations
get_document_state            → reorient after confusion
```

If node count in `get_document_state` doesn't match expectations: audit before continuing.

---

## Naming Convention

Pattern: `{component}_{part}_{qualifier}`

| Component | Part | Qualifier | Full Name |
|---|---|---|---|
| head | base | — | `head_base` |
| head | eye | sclera | `head_eye_sclera` |
| head | eye | pupil | `head_eye_pupil` |
| wing | left | base | `wing_left_base` |
| wing | left | stripe | `wing_left_stripe_1` |
| wing | left | dot | `wing_left_dot_3` |
| body | silhouette | — | `body_silhouette` |
| tail | feather | — | `tail_feather_2` |
| leg | left | — | `leg_left` |
| leg | left | toe | `leg_left_toe_1` |

Groups: append `_group` → `head_group`, `wing_left_group`, `tail_group`

---

## Screenshot Cadence

| When | Why |
|---|---|
| Session start | Confirm starting state |
| After each completed component | Review before moving on — corrections are cheaper here |
| After any boolean operation | Verify the result looks correct |
| After z-order corrections | Confirm layering |
| Final review | End-of-session sign-off |

Do NOT screenshot after every single node — this creates noise and wastes tool calls. One screenshot per completed component is the target cadence.

---

## Common Mistakes

| Mistake | Fix |
|---|---|
| Skipping `get_document_state` at session start | Always orient first, even on a fresh canvas |
| Creating nodes without `layer_id` | Every node must be assigned to a layer |
| Creating nodes without `name` | Every node must have a descriptive name |
| Grouping all nodes at the end of the session | Group after each component, not at the end |
| Screenshotting after every single node | One screenshot per completed component |
| Running `boolean_operation` on primitive shapes | Verify nodes are paths first via `get_node` |
| Losing track of node IDs after many operations | Call `get_document_state` to reorient |
| Drawing front-to-back | Always draw back-to-front; use `reorder_node` for exceptions |
| Forgetting rotate pivot defaults to shape center | Pass explicit `cx`/`cy` when rotating around a shared pivot |
