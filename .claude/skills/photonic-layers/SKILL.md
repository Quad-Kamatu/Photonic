---
name: photonic-layers
description: Use when auditing, reorganizing, or cleaning up layer structure in a Photonic document — especially after SVG import, when nodes are unnamed, ungrouped, misplaced, or the document structure is unclear.
---

# Photonic Layer Analysis

## Overview

A structured workflow for reading, diagnosing, and fixing the organizational state of a Photonic document. Applies to freshly imported SVGs, documents built incrementally without a plan, or any session where the layer/group structure needs auditing.

**Core principle:** Understand before you touch. Always read the full document state first. Never rename, regroup, or reorder without a complete picture of what exists.

---

## When to Use

- An SVG was imported and nodes have generic names (e.g., `path_1`, `ellipse_3`, `group_7`)
- The user asks to "clean up", "reorganize", "label", or "audit" layers
- Nodes exist but grouping is missing, inconsistent, or broken
- The document was built without a plan and structure has drifted
- You need to understand an existing illustration before making changes

**Do NOT use this skill** when drawing a new illustration from scratch — use `photonic-plan` instead.

---

## Phase 1 — Document Audit

Run these calls in parallel to get a complete picture before touching anything:

```
get_document_state      → full layer/node tree, IDs, types, names, z-order
screenshot              → visual state of the canvas
export_svg              → raw SVG for detailed path/structure inspection (optional, for complex imports)
```

From `get_document_state`, extract:

| What to count | Why |
|---|---|
| Total node count | Baseline before any changes |
| Unnamed nodes (`""`, `"path_1"`, `"group"`) | Candidates for `auto_name_nodes` or manual rename |
| Ungrouped loose nodes | Nodes not inside any group |
| Nodes on wrong layers | Mismatches between node type and layer purpose |
| Empty or single-node groups | Often leftover from import or boolean ops |
| Duplicate names | Two nodes with identical names break selection by name |

---

## Phase 2 — Problem Classification

Classify every identified problem before fixing any of them. Do not interleave diagnosis and repair.

### Problem Types

**Naming problems**
- Generic auto-names: `path_1`, `ellipse`, `group_7`, `rect`
- Empty names: `""`
- Non-descriptive imports: `layer1`, `g23`, `symbol_4`

**Grouping problems**
- Orphaned nodes: visually part of a component but not in its group
- Over-grouping: a group containing only one node
- Wrong-level grouping: a shape grouped with unrelated shapes

**Z-order problems**
- Foreground elements behind background elements
- Detail shapes below base shapes of same component
- Highlight dots behind the surface they highlight

**Layer problems**
- All nodes on one layer (flat structure)
- Nodes on layers with no semantic meaning (`layer1`, `layer2`)
- Mismatched content: a background shape on the `detail` layer

**Geometry problems**
- Sub-pixel coordinates: `x: 102.4732`, `y: 88.1109` (from SVG import rounding)
- Inconsistent sizes: similar shapes with slightly different dimensions
- Off-axis alignment: shapes that should share an edge but are offset by 1–2px

---

## Phase 3 — Analysis Report

Before making any changes, write a short plain-text analysis covering:

```
=== LAYER ANALYSIS REPORT ===

DOCUMENT SUMMARY
  Total nodes:   [N]
  Layers:        [names]
  Named nodes:   [N of total]
  Grouped nodes: [N of total]

PROBLEMS FOUND
  [type] [count] — [brief description]
  e.g.:
  naming   12 — generic names (path_1 through path_12)
  grouping  5 — orphaned nodes not in a group
  z-order   2 — highlight below body on head component
  layer     3 — background shapes on detail layer
  geometry  8 — sub-pixel coordinates (import artifact)

PROPOSED ACTIONS
  1. [action] — [which nodes, why]
  2. ...

=== BEGIN REPAIRS ===
```

Do not start repairs until this report is written and (if unclear) confirmed with the user.

---

## Phase 4 — Repair Sequence

Apply fixes in this order. Each phase must complete before the next.

### Step 1 — Rename

Use `auto_name_nodes` for nodes with obviously auto-generated names, then manually rename anything it gets wrong using `update_node`.

```
auto_name_nodes
  → reviews node geometry and content, assigns descriptive names
  → review its output before proceeding; correct wrong names manually

update_node
  node_id: <id>
  name:    "{component}_{part}_{qualifier}"   ← naming convention from photonic-workflow
```

**Naming convention:**
```
{component}_{part}_{qualifier}
  head_base
  head_eye_iris
  wing_left_stripe_1
  body_silhouette
  tail_feather_3
```

Groups: append `_group` → `head_group`, `wing_left_group`

### Step 2 — Reorder (Z-order)

Fix stacking order before regrouping — ungrouped nodes are easier to reorder.

```
reorder_node
  node_id:   <node>
  operation: send_to_back | bring_to_front | move_above | move_below
  relative_id: <anchor node>   ← required for move_above / move_below
```

Standard z-order within any component (back → front):
```
base silhouette → body markings → overlays → detail features → highlights → specular dots
```

### Step 3 — Reassign Layers

Move nodes to semantically correct layers using `collect_in_new_layer` (creates layer + moves) or `reorder_node` on the layer level.

```
update_layer
  layer_id: <id>
  name:     "background" | "base" | "overlay" | "detail" | "highlight"
```

Standard layer stack (back → front):
```
background — sky, ground, large fills
base       — main silhouettes, large body shapes
overlay    — stripes, markings, patches
detail     — eyes, beak, small anatomical features
highlight  — specular dots, shine marks, accents
```

### Step 4 — Regroup

Group nodes into logical components. Group immediately after each component is complete.

```
group_nodes
  node_ids: [all node ids for this component]
  name:     "{component}_group"
```

**When to ungroup first:**
- If nodes are in a wrong group, use `ungroup_nodes` to dissolve it, then regroup correctly
- Never move nodes between groups directly — always ungroup → move → regroup

```
ungroup_nodes
  group_id: <wrong group id>
→ nodes returned to layer at their former z-position
→ then group_nodes with correct membership
```

### Step 5 — Fix Geometry

For import artifacts (sub-pixel coordinates, near-round sizes):

```
measure_nodes
  node_ids: [<id>]
→ returns exact bounding box (x, y, width, height)

update_node
  node_id:  <id>
  x:        [round to nearest integer]
  y:        [round to nearest integer]
  width:    [round to nearest 2px or design grid]
  height:   [round to nearest 2px or design grid]
```

**Rounding rule:** Round to the nearest whole pixel. For sizes, round to the nearest even number if the shape is symmetric. Do not "snap" if the shape is intentionally positioned off-grid (e.g., a rotated element with a non-round transform).

```
align_nodes
  node_ids:  [list of related shapes]
  alignment: align_left | align_center_h | align_center_v | align_right | align_top | align_bottom
```

Use `align_nodes` when multiple shapes should share an edge or center axis.

---

## Phase 5 — Verify

After all repairs:

```
get_document_state   → confirm node count, names, groups, layers match your plan
screenshot           → visual confirmation; look for missing shapes or z-order regressions
check_style_continuity → flag any fills/strokes that are outliers vs the rest of the document
```

Compare node count before and after. If the count decreased unexpectedly, investigate — a group operation should not lose nodes.

---

## Tool Reference

| Goal | Tool |
|---|---|
| Full layer/node tree | `get_document_state` |
| Single node details | `get_node` |
| Query by type/layer/tag | `find_nodes` |
| Geometry + structure metrics | `inspect_node` |
| Auto-rename by content | `auto_name_nodes` |
| Rename / move / resize a node | `update_node` |
| Change layer name/visibility | `update_layer` |
| Change z-order | `reorder_node` |
| Combine nodes into group | `group_nodes` |
| Dissolve a group | `ungroup_nodes` |
| Move nodes to new layer | `collect_in_new_layer` |
| Move each node to own layer | `release_to_layers` |
| Merge layers together | `merge_layers` |
| Align/distribute nodes | `align_nodes` |
| Get bounding box | `measure_nodes` |
| Resize to exact dimensions | `set_node_size` |
| Find all nodes with same fill | `select_same` |
| Style consistency check | `check_style_continuity` |
| Extract design tokens | `export_design_tokens` |
| Annotate for review | `add_annotation` |
| Visual review | `screenshot` |

---

## Common Mistakes

| Mistake | Fix |
|---|---|
| Renaming while the structure is unknown | Always run `get_document_state` + `screenshot` first |
| Regrouping before fixing z-order | Fix stacking first — reordering inside groups is harder |
| Using `auto_name_nodes` and trusting all results | Review every auto-generated name; it gets generic shapes wrong |
| Ungrouping without noting which nodes belonged together | Write out the group membership before dissolving |
| Rounding all coordinates blindly | Skip rounding for transformed/rotated elements — their coordinates are intentional |
| Merging all nodes into one group | Group per component, not per layer. One group = one visual component |
| Deleting unnamed nodes without inspecting them | Unnamed ≠ empty; always `get_node` before `delete_nodes` |
| Fixing geometry before fixing structure | Structure (names, groups, layers) must be correct first; geometry is last |
