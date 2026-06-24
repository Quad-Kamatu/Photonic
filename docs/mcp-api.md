# MCP API Reference

Photonic exposes a JSON-RPC 2.0 API over HTTP POST at `http://localhost:7842` (default port).

All requests use the envelope:

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "<tool_name>",
  "params": { ... }
}
```

All successful responses:

```json
{ "jsonrpc": "2.0", "id": 1, "result": { ... } }
```

Error responses carry a structured `error` object with `code` and `message`.

---

## Document queries

### `get_document_state`

Returns the current document structure.

**Parameters**

| Field | Type | Default | Description |
|---|---|---|---|
| `include_path_data` | bool | `false` | Include raw SVG path strings |
| `layer_id` | string? | — | Filter to one layer |
| `summary_only` | bool | `false` | Lightweight response (node count, layer names) |

**Returns** — full document JSON: canvas dimensions, layer order, all nodes with transforms and style.

---

### `get_node`

Retrieve a single node by ID or name.

**Parameters**

| Field | Type | Description |
|---|---|---|
| `node_id` | string? | UUID |
| `name` | string? | Exact name match |

Exactly one of `node_id` or `name` must be provided.

**Returns** — serialized `SceneNode`.

---

## Shape creation

### `create_shape`

Create a primitive shape and add it to the active layer.

**Parameters**

| Field | Type | Required | Description |
|---|---|---|---|
| `shape_type` | string | yes | `"rectangle"`, `"ellipse"`, `"polygon"`, `"star"`, `"line"` |
| `x`, `y` | f64 | yes | Top-left position (rectangle/ellipse) or center (polygon/star) |
| `width`, `height` | f64 | rect/ellipse | Bounding box size |
| `cx`, `cy` | f64 | polygon/star | Center point |
| `radius` | f64 | polygon/star | Outer radius |
| `sides` | u32 | polygon | Number of sides (≥ 3) |
| `points` | u32 | star | Number of points (≥ 2) |
| `inner_radius` | f64 | star | Inner (concave) radius |
| `x1`,`y1`,`x2`,`y2` | f64 | line | Endpoints |
| `fill` | FillArg? | — | See Fill types below |
| `stroke` | StrokeArg? | — | See Stroke below |
| `name` | string? | — | Node name |
| `layer_id` | string? | — | Target layer UUID |
| `tags` | string[]? | — | Semantic labels |

**Returns** `{ node_id: "<uuid>" }`

**Example — filled rectangle**
```json
{
  "method": "create_shape",
  "params": {
    "shape_type": "rectangle",
    "x": 50, "y": 50,
    "width": 200, "height": 120,
    "fill": { "type": "solid", "color": "#3b82f6" },
    "name": "background"
  }
}
```

---

### `create_path`

Create a node from an SVG path string.

**Parameters**

| Field | Type | Required | Description |
|---|---|---|---|
| `path_data` | string | yes | SVG path (e.g. `"M 0 0 L 100 0 L 50 80 Z"`) |
| `fill` | FillArg? | — | |
| `stroke` | StrokeArg? | — | |
| `transform` | TransformArg? | — | |
| `name` | string? | — | |
| `layer_id` | string? | — | |
| `tags` | string[]? | — | |

**Returns** `{ node_id: "<uuid>" }`

---

### `build_shape_from_points`

Create a closed polygon from an array of `[x, y]` points.

**Parameters**

| Field | Type | Required | Description |
|---|---|---|---|
| `points` | [f64, f64][] | yes | Array of `[x, y]` coordinates |
| `closed` | bool | — | Default `true` |
| `fill` | FillArg? | — | |
| `stroke` | StrokeArg? | — | |
| `name` | string? | — | |

**Returns** `{ node_id: "<uuid>" }`

---

## Updating nodes

### `update_node`

Modify any property of an existing node.

**Parameters**

| Field | Type | Description |
|---|---|---|
| `node_id` | string | Target UUID |
| `fill` | FillArg? | Replace fill |
| `stroke` | StrokeArg? | Replace stroke |
| `transform` | TransformArg? | Replace transform |
| `opacity` | f32? | 0.0–1.0 |
| `name` | string? | Rename node |
| `visible` | bool? | |
| `blend_mode` | string? | See blend modes below |
| `tags` | string[]? | Replace tag list |

**Returns** `{ node_id: "<uuid>" }`

---

## Transforms

### `apply_transform`

Apply a transform operation to one or more nodes.

**Parameters**

| Field | Type | Required | Description |
|---|---|---|---|
| `node_ids` | string[] | yes | UUIDs to transform |
| `operation` | string | yes | `"translate"`, `"rotate"`, `"scale"`, `"matrix"`, `"reflect_horizontal"`, `"reflect_vertical"` |
| `x`, `y` | f64 | translate | Pixel offsets |
| `angle` | f64 | rotate | Degrees |
| `origin_x`, `origin_y` | f64 | rotate/scale | Transform origin (default: node center) |
| `scale_x`, `scale_y` | f64 | scale | Scale factors |
| `matrix` | f64[6] | matrix | `[a,b,c,d,e,f]` affine matrix |

**Returns** `{ node_ids: ["<uuid>", ...] }`

---

## Document structure

### `create_layer`

**Parameters**

| Field | Type | Description |
|---|---|---|
| `name` | string | Layer name |
| `position` | u32? | Insert index (default: top) |

**Returns** `{ layer_id: "<uuid>" }`

---

### `group_nodes`

Wrap multiple nodes in a new group node on the same layer.

**Parameters**

| Field | Type | Required | Description |
|---|---|---|---|
| `node_ids` | string[] | yes | Nodes to group (order preserved) |
| `name` | string? | — | Group node name |

**Returns** `{ group_id: "<uuid>" }`

---

### `ungroup_nodes`

Dissolve a group, returning its children to the parent layer.

**Parameters**

| Field | Type | Required |
|---|---|---|
| `group_id` | string | yes |

**Returns** `{ node_ids: ["<uuid>", ...] }`

---

### `reorder_node`

Change z-order of a node within its layer.

**Parameters**

| Field | Type | Required | Description |
|---|---|---|---|
| `node_id` | string | yes | |
| `operation` | string | yes | `"send_to_back"`, `"bring_to_front"`, `"send_backward"`, `"bring_forward"`, `"move_above"`, `"move_below"` |
| `relative_id` | string? | move_above/below | Reference node |

**Returns** `{ node_id: "<uuid>" }`

---

### `delete_nodes`

**Parameters**

| Field | Type | Required |
|---|---|---|
| `node_ids` | string[] | yes |

**Returns** `{ deleted: ["<uuid>", ...] }`

---

## Path operations

### `boolean_operation`

Combine two paths using a set operation.

**Parameters**

| Field | Type | Required | Description |
|---|---|---|---|
| `operation` | string | yes | `"union"`, `"subtract"`, `"intersect"`, `"exclude"` |
| `target_id` | string | yes | Base shape (style is inherited by result) |
| `tool_id` | string | yes | Cutting/combining shape |
| `keep_originals` | bool | — | Default `false` |

**Returns** `{ node_id: "<uuid>" }`

---

## History

### `undo`

Undo the last command.

**Parameters** — none

**Returns** `{ success: true }`

---

### `redo`

Redo the previously undone command.

**Parameters** — none

**Returns** `{ success: true }`

---

### `create_checkpoint`

Save a named snapshot of the current document state.

**Parameters**

| Field | Type | Required |
|---|---|---|
| `name` | string | yes |

**Returns** `{ checkpoint_id: "<uuid>" }`

---

### `list_checkpoints`

**Returns** array of `{ id, name, created_at }` objects.

---

### `restore_checkpoint`

Revert the document to a saved snapshot.

**Parameters**

| Field | Type | Required |
|---|---|---|
| `checkpoint_id` | string | yes |

**Returns** `{ success: true }`

---

## Canvas

### `screenshot`

Capture the current canvas as PNG.

**Parameters**

| Field | Type | Description |
|---|---|---|
| `scale` | f64? | Pixel multiplier (default 1.0) |
| `region` | { x, y, width, height }? | Crop to document-space region |

**Returns** `{ image: "<base64 PNG>", mime_type: "image/png" }`

---

## Type reference

### FillArg

```jsonc
// No fill
{ "type": "none" }

// Solid color
{ "type": "solid", "color": "#rrggbb" }

// Linear gradient
{
  "type": "linear_gradient",
  "stops": [
    { "offset": 0.0, "color": "#ff0000" },
    { "offset": 1.0, "color": "#0000ff" }
  ],
  "start": [0, 0],   // [x, y] in node-local space
  "end": [100, 0]
}

// Radial gradient
{
  "type": "radial_gradient",
  "stops": [ ... ],
  "center": [50, 50],
  "focal": [50, 50],
  "radius": 50
}

// Fluid gradient (IDW interpolation)
{
  "type": "fluid_gradient",
  "points": [
    { "x": 10, "y": 10, "color": "#ff0000" },
    { "x": 90, "y": 90, "color": "#0000ff" }
  ],
  "power": 2.0
}

// Mesh gradient (rows × cols bilinear grid)
{
  "type": "mesh_gradient",
  "rows": 2, "cols": 2,
  "vertices": [
    { "x": 0,   "y": 0,   "color": "#ff0000" },
    { "x": 100, "y": 0,   "color": "#00ff00" },
    { "x": 0,   "y": 100, "color": "#0000ff" },
    { "x": 100, "y": 100, "color": "#ffff00" }
  ]
}
```

FillArg also accepts optional `"opacity": 0.0–1.0`.

---

### StrokeArg

```jsonc
{
  "color": "#000000",
  "width": 2.0,
  "line_cap": "round",       // "butt" | "round" | "square"
  "line_join": "miter",      // "miter" | "round" | "bevel"
  "dash_array": [4, 2],      // optional
  "miter_limit": 4.0,        // optional
  "enabled": true
}
```

---

### TransformArg

Pass as a 6-element array `[a, b, c, d, e, f]` representing the 2-D affine matrix:

```
| a  c  e |
| b  d  f |
| 0  0  1 |
```

Identity: `[1, 0, 0, 1, 0, 0]`

---

### Blend modes

`"normal"`, `"multiply"`, `"screen"`, `"overlay"`, `"darken"`, `"lighten"`, `"color_dodge"`, `"color_burn"`, `"hard_light"`, `"soft_light"`, `"difference"`, `"exclusion"`, `"hue"`, `"saturation"`, `"color"`, `"luminosity"`

---

## Error codes

| Code | Meaning |
|---|---|
| `-32700` | Parse error |
| `-32600` | Invalid request |
| `-32601` | Method not found |
| `-32602` | Invalid params |
| `-32603` | Internal error |
