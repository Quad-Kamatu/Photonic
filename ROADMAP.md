# Photonic MCP Toolbox Roadmap

Expanding the MCP tool surface to better enable Claude to create high-quality vector art.
Derived from analysis of alpha output (squid logo) and identified capability gaps.

> **Status (historical roadmap).** Most items below have shipped — the MCP
> surface is now 280+ tools. For the **current, authoritative** tool list, see
> the auto-generated [`docs/mcp-api.md`](docs/mcp-api.md) (regenerated from
> `server::tool_list()`), not this document. This file is retained for design
> rationale and history.

---

## Priority 1 — Structural Foundations

These three unlock the way a real illustrator works: create rough shapes, cut/combine, organize, refine.

### 1. Z-order Control — `reorder_node`

**Why:** Layering (tentacles behind body, pupils on eyes) must currently be planned perfectly at creation time. This tool lets Claude create first, arrange second.

**Operations:**
- `send_to_back`
- `bring_to_front`
- `move_above { relative_id }`
- `move_below { relative_id }`

**Also needed:** `get_document_state` should include z-order index in its node output.

---

### 2. Group / Ungroup — `group_nodes` / `ungroup_nodes`

**Why:** No concept of groups exists. Claude can't treat 8 tentacles as one unit for scaling or moving. Every professional workflow depends on grouping.

**Parameters:**
- `group_nodes { node_ids, name? }` → returns group node_id
- `ungroup_nodes { group_id }` → returns child node_ids

Groups should participate in transforms, z-order, and `get_document_state` like any other node.

---

### 3. Boolean Operations — `boolean_operation`

**Why:** This is the single biggest quality ceiling. Compound shapes (donut, crescent, cutouts, letter forms) are impossible without it. Eye pupils are currently stacked ellipses — with subtract they become true cutouts.

**Operations:** `union`, `subtract`, `intersect`, `exclude`

**Parameters:**
```json
{
  "operation": "subtract",
  "target_id": "<uuid>",
  "tool_id": "<uuid>",
  "keep_originals": false
}
```

---

## Priority 2 — Significant Quality Gains

### 4. Duplicate with Transform — `duplicate_node`

**Why:** Repeated elements (tentacles, grid dots, petals) currently require N identical `create_path` calls with manually computed coordinates. Reduces both token cost and arithmetic error.

**Parameters:**
```json
{
  "node_id": "<uuid>",
  "count": 8,
  "translate": { "x": 12, "y": 0 },
  "rotate": { "angle_degrees": 45, "origin_x": 0, "origin_y": 0 }
}
```

Returns array of new node IDs.

---

### 5. Bounding Box Query — `get_bounding_box`

**Why:** Claude currently dead-reckons coordinate math. With bounding boxes it can express "place the eye at 30% from the top of the body" and verify placement after screenshot.

**Parameters:** `{ node_ids: [...] }` (single or multiple; multiple returns union AABB)

**Returns:** `{ x, y, width, height, center_x, center_y }`

---

### 6. Align and Distribute — `align_nodes`

**Why:** Lets Claude express spatial relationships ("center these eyes relative to the body") as a single call rather than computing pixel offsets manually.

**Anchors:** `left`, `right`, `top`, `bottom`, `center_x`, `center_y`

**Distribute:** `distribute_horizontal { spacing? }`, `distribute_vertical { spacing? }`

**Reference:** relative to bounding box of selection, or to a specific `reference_id`.

---

### 7. Clipping Masks — `create_clip`

**Why:** Required for texture-inside-shape, photo crops, complex logo treatments. Without clipping the only option is careful manual path construction.

**Parameters:**
```json
{
  "clip_shape_id": "<uuid>",
  "masked_node_id": "<uuid>"
}
```

Also needs `release_clip { node_id }`.

---

### 8. Compound Paths — `combine_paths`

**Why:** Builds a single path with holes — eye sockets, donuts, letter O. Closely related to booleans but operates at the path data level and produces a single editable path node.

**Parameters:**
```json
{
  "node_ids": ["<uuid>", "<uuid>"],
  "fill_rule": "evenodd" | "nonzero"
}
```

---

### 9. Text Tool — `create_text`

**Why:** Logos need text. Even a minimal primitive with a basic font stack unlocks label and wordmark workflows.

**Parameters:**
```json
{
  "content": "PHOTONIC",
  "x": 100, "y": 200,
  "font_family": "sans-serif",
  "font_size": 48,
  "font_weight": "bold",
  "letter_spacing": 2,
  "fill": { "type": "solid", "color": "#1a1a2e" }
}
```

---

## Priority 3 — Polish and Workflow

### 10. Named Color Palette — `set_palette` / `get_palette`

**Why:** Prevents color drift across 30+ shape calls. Claude registers swatches at session start and references them by name throughout.

**Usage:** `set_palette { swatches: { "body_blue": "#5b7fa6", "accent": "#ffffff" } }`

Fill/stroke specs should accept `{ "type": "swatch", "name": "body_blue" }` in addition to hex strings.

---

### 11. Batch Create — `batch_create`

**Why:** Submits multiple create operations in one JSON-RPC call. Reduces round trips for repetitive elements and keeps context usage low.

**Parameters:** `{ operations: [ { tool: "create_shape", args: {...} }, ... ] }`

Returns array of results in matching order.

---

### 12. Offset Path — `offset_path`

**Why:** Expands or contracts a path by a given amount. Used for outlines, halos, inset shapes — without this Claude must manually recalculate every point.

**Parameters:** `{ node_id, amount, join_type: "miter" | "round" | "bevel" }`

---

### 13. Canvas / Artboard Setup — `set_canvas`

**Why:** Claude is currently guessing coordinate space. Knowing canvas dimensions at the start is fundamental.

**Parameters:** `{ width, height, background_color? }`

`get_document_state` should also return canvas dimensions.

---

### 14. Effects — `apply_effect`

**Why:** Even one or two effects push output from "flat icon" to "polished logo."

**Initial effects:**
- `drop_shadow { dx, dy, blur, color, opacity }`
- `gaussian_blur { radius }`
- `inner_glow { color, opacity, blur }`

---

## Instruction Set Improvements

Beyond the toolbox, the tool descriptions and any system prompt Claude receives should include:

| Item | Detail |
|---|---|
| **Canvas dimensions at session start** | Claude should receive `{ width, height }` in the initial handshake or `initialize` response |
| **Coordinate conventions** | Y-axis direction, origin location, units (pixels) |
| **Design workflow template** | Sketch proportions → layer structure → fills → details → screenshot verify |
| **Screenshot cadence guidance** | After every major structural change, not after every shape |
| **Flat design color palette** | A curated 12-color starter palette for consistent, professional results |
| **Node naming conventions** | Semantic names enforced by example: `body`, `left_fin`, `eye_left_pupil` |
| **Tag taxonomy** | `body`, `detail`, `highlight`, `shadow` — enables "select all highlights" queries |

---

## Implementation Order

```
Phase A  →  reorder_node, group/ungroup, get_bounding_box
Phase B  →  boolean_operation, duplicate_node, align_nodes
Phase C  →  text, compound paths, clipping masks
Phase D  →  batch_create, offset_path, set_canvas, named palette
Phase E  →  effects, advanced gradients, mesh gradients
```

Phase A + B together cover the critical path. Everything after is additive quality.
