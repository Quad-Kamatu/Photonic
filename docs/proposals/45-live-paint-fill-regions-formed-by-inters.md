# Live Paint: Fill Regions Formed by Intersecting Paths (#45) — Design Proposal

> Status: design scaffold (not an implementation).

## Summary

No Live Paint exists. Live Paint lets a user treat any set of overlapping open or closed paths as a stained-glass partition: each bounded region (face) can receive an independent fill, and each shared edge can receive an independent stroke, without requiring the user to close paths manually or run boolean operations first. The `geo` crate is already a workspace dependency (used by `ops/boolean.rs` for `BooleanOps`), and `geo` provides the polygon decomposition primitives needed to build the face map. The challenge is building and maintaining the planar graph (face + edge adjacency) as paths are edited, and rendering it live on-canvas.

## Scope (in / out)

**In:**
- **Planar map builder**: given a set of `PathData` objects, compute the arrangement of all intersection points, split paths at intersections, and enumerate the bounded faces (closed polygon regions) and their bounding edges.
- **`LivePaintGroup`** document node: stores the constituent `PathData` objects, the computed face map (faces + edges), per-face `Fill`, and per-edge `Stroke`.
- **Gap detection**: configurable gap threshold; edges within the gap distance are treated as connected (useful for hand-drawn paths that do not quite meet).
- **Live Paint Bucket tool** (`photonic-gui`): hover highlights the face under the cursor (computed by point-in-face test); click assigns the active fill to that face.
- **Live Paint Selection tool**: select individual faces or edges; apply fill/stroke from the appearance panel.
- **Expand**: convert the `LivePaintGroup` to a flat set of `PathNode`s — one per face with its assigned `Fill` — plus stroke `PathNode`s for painted edges. Uses `Command::Batch`.
- **Release**: revert to the original constituent paths (stored in the `LivePaintGroup`).
- **MCP tools**: `create_live_paint(node_ids)`, `paint_face(group_id, face_index, fill)`, `paint_edge(group_id, edge_index, stroke)`, `expand_live_paint(group_id)`.
- **Rebuild on edit**: if a constituent path is edited (node update via `Command::UpdateNode`), the face map is invalidated and recomputed.

**Out:**
- Topology editing (adding/removing paths from an existing `LivePaintGroup`) — defer.
- Gradients or pattern fills per face — defer; solid fills only for V1.
- Raster content as region boundaries — out of scope.
- GPU-accelerated arrangement computation — out of scope.

## Proposed Approach

### Planar Map Construction

1. **Intersection finding**: Use `geo::algorithm::line_intersection` and `geo::algorithm::sweep_line` (or a simpler pairwise scan) to find all intersection points between the path segments. For each intersecting segment pair, record the parameter values `t_a` and `t_b` at the intersection. `kurbo::BezPath` with `kurbo::ParamCurve::subdivide_at_t` handles Bézier splitting.

2. **Arrangement graph**: After splitting all paths at intersection points, the result is a planar straight-line graph (PSLG). Build this as:

```rust
pub struct PlanarArrangement {
    pub vertices: Vec<ArrangementVertex>,
    pub half_edges: Vec<HalfEdge>,
    pub faces: Vec<Face>,
}

pub struct ArrangementVertex { pub point: kurbo::Point }
pub struct HalfEdge {
    pub origin: usize,         // vertex index
    pub twin: usize,           // opposite half-edge
    pub next: usize,           // next half-edge in face
    pub face: usize,           // face index
    pub path_segment: PathData, // the Bézier segment this edge came from
}
pub struct Face {
    pub outer_edge: Option<usize>, // half-edge on boundary (None = unbounded face)
    pub fill: Fill,
    pub is_outer: bool,            // true = the unbounded exterior face
}
```

Use the standard half-edge DCEL (Doubly Connected Edge List) construction. The `geo` crate's `BooleanOps` internally builds similar structures; however, for Live Paint we need to retain all original path curves as Bézier segments rather than polygonizing them. Build the DCEL directly from kurbo geometry.

3. **Face enumeration**: Trace each face boundary by following `half_edge.next` chains. Use a winding-number test (`geo::algorithm::winding_order`) to classify outer vs. inner faces.

4. **Point-in-face**: For the Bucket tool hover, test `cursor_point` against each non-outer face using `geo::algorithm::Contains` (after converting the face boundary to a `geo::Polygon` for the point test; approximate curved edges with polylines for the containment test).

5. **Gap detection**: After splitting, scan pairs of endpoints closer than `gap_threshold`; merge coincident vertices to close small gaps. Store `gap_threshold: f64` in `LivePaintGroup`.

### Document Model

6. **New node kind** (`node.rs`):

```rust
pub struct LivePaintGroup {
    pub constituent_ids: Vec<NodeId>,  // original PathNode IDs (kept in document)
    pub gap_threshold: f64,
    pub arrangement: PlanarArrangement,  // computed; can be rebuilt from constituent_ids
}
```

Add `SceneNodeKind::LivePaint(LivePaintGroup)` to the `SceneNodeKind` enum. The constituent paths remain as regular `PathNode`s in the document but are rendered as "source paths" (hairline, no fill) when the `LivePaintGroup` is active.

7. **Rendering** (`photonic-render/src/renderer.rs`): For a `LivePaintGroup`, iterate faces; tessellate each face boundary polygon using `lyon` (already a workspace dependency) and draw with the face's `Fill`. For painted edges, draw the half-edge path segment with the assigned `Stroke`. For the hover face, render a highlight overlay.

8. **History**: `create_live_paint` → `Command::Batch([AddNode(LivePaintGroup), ...])`. `paint_face` → `Command::UpdateNode` (update the face's `Fill` in the arrangement). Expand → `Command::Batch([RemoveNode(group), AddNode(...) * n_faces])`. Release → `Command::RemoveNode(group)` (constituent paths already present).

### GUI Tools

9. **Live Paint Bucket** (`tools/mod.rs`): On mouse move, compute the face under the cursor and highlight it. On click, emit `Command::UpdateNode` to set `arrangement.faces[face_idx].fill = active_fill`.

10. **Live Paint Selection** (`tools/mod.rs`): Click to select a face or edge (use proximity test for edges). Selected faces/edges show handles in the panel.

## Affected Modules

- `crates/photonic-core/src/ops/live_paint.rs` — new: `PlanarArrangement`, DCEL construction, gap detection, point-in-face
- `crates/photonic-core/src/ops/mod.rs` — add `pub mod live_paint`
- `crates/photonic-core/src/node.rs` — add `LivePaintGroup`, `SceneNodeKind::LivePaint`
- `crates/photonic-core/src/history.rs` — handle `LivePaint` variant in `Command::apply` / `inverse`
- `crates/photonic-render/src/renderer.rs` — render face fills + edge strokes + hover highlight
- `crates/photonic-render/src/tessellator.rs` — tessellate arbitrary polygon face boundaries via `lyon`
- `crates/photonic-gui/src/tools/mod.rs` — `LivePaintBucketTool`, `LivePaintSelectionTool`
- `crates/photonic-gui/src/panels/mod.rs` — face/edge inspector, gap threshold control
- `crates/photonic-mcp/src/server.rs` + `protocol.rs` — new tool handlers

## Risks & Open Questions

- **DCEL from Bézier segments**: Standard DCEL construction algorithms assume straight edges (PSLG). Curved Bézier edges require adaptive parameterization for intersection finding and face boundary walking. Use the polyline approximation (`kurbo::BezPath::flatten`) for topology and retain the original Bézier for rendering — the "topology polyline, render Bézier" dual representation adds implementation complexity.
- **Intersection accuracy**: Bézier–Bézier intersection is iterative and sensitive to near-tangent cases. `kurbo` does not expose a Bézier–Bézier intersection API directly; may need `kurbo::common::solve_cubic` or a subdivision-based approach. Alternatively, polygon-approximate all input curves for arrangement construction and keep originals for display only.
- **Performance on complex arrangements**: N paths with K intersections each produce O(N²) intersection tests. For large artwork (100+ paths), this may be slow. Bound the face map to a reasonable N (e.g., 200 paths) for V1 and document the limit.
- **`SceneNodeKind` exhaustive match**: Adding `LivePaint` variant triggers the same multi-crate touch-up as `ImageTrace` in #44. Mark `SceneNodeKind` `#[non_exhaustive]` before M7.
- **Constituent path visibility**: When a `LivePaintGroup` is active, constituent paths should be visible as thin guides but not independently selectable. The renderer needs to suppress their normal fill/stroke and the selection system must filter them from regular selection.
- **Gap detection edge cases**: Two paths that nearly-but-not-quite meet may produce a tiny sliver face. Gap detection merges vertices but may still create degenerate faces. Add a minimum-area face filter in the DCEL construction.

## Acceptance Criteria

- [ ] Two crossing open lines produce four bounded faces in the planar arrangement; each can receive an independent fill.
- [ ] The Live Paint Bucket tool highlights the hovered face and assigns the active fill on click.
- [ ] Painted edges (half-edge strokes) render independently of face fills.
- [ ] Gap detection merges path endpoints within the configured threshold.
- [ ] Expand produces one filled `PathNode` per non-outer face (with correct fill) + edge stroke nodes; Undo restores the `LivePaintGroup`.
- [ ] Release returns the original constituent `PathNode`s unchanged.
- [ ] MCP `create_live_paint` / `paint_face` / `expand_live_paint` tools are functional.
- [ ] Performance is acceptable (< 1 second arrangement rebuild) for up to 50 intersecting paths.

## Effort Estimate

**XL** — The DCEL planar arrangement from Bézier curves is the hardest algorithmic problem in this issue set. Even with `geo` and `kurbo` as foundations, building a correct half-edge structure from curved segments, rendering it, and integrating two new GUI tools with undo/redo is substantial. The gap detection, face highlight, and expand path add further scope. Nothing here is impossible, but correctness under edge cases (near-tangent intersections, degenerate faces, self-intersecting paths) requires careful implementation and extensive testing.
