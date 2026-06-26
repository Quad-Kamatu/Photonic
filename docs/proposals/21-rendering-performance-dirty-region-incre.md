# Rendering Performance: Dirty-Region / Incremental Tessellation + Benchmark Suite (#21) — Design Proposal

> Status: design scaffold (not an implementation).

## Summary

`build_geometry` (`renderer.rs:701`) walks every visible node in draw order every frame,
tessellates all path geometry, uploads a fresh vertex + index buffer, and bakes text
snapshots — unconditionally. The only partial optimization is a `try_lock` fallback that
returns `cached_vertices` / `cached_indices` (`renderer.rs:732-734`) when the doc mutex
is contended, but this is a lock-contention safety valve, not an invalidation strategy.
For large documents (1k+ nodes) this will not scale. There is also no benchmark suite.

## Scope

**In:**
- Per-node geometry cache keyed on `NodeId`; invalidate only changed nodes.
- Skip re-tessellation on pure camera pan/zoom (camera changes only the `CameraUniform`,
  not geometry).
- Criterion-based benchmark suite: build/redraw time and memory for synthetic 100 / 1k /
  10k node documents, with/without effects.
- CI integration as a non-gating report.

**Out:**
- Tile-based dirty rectangle culling (GPU-level, future).
- Parallel tessellation across threads (future; requires the lyon tessellator to be `Send`).

## Proposed Approach

1. **Node geometry cache** — add to `PhotonicRenderer`:
   ```rust
   node_geo_cache: HashMap<NodeId, CachedNodeGeo>,
   doc_version: u64,  // or a per-node change counter
   ```
   ```rust
   struct CachedNodeGeo {
       verts: Vec<Vertex>,
       idxs: Vec<u32>,
       node_version: u64,  // matches Document node change counter
   }
   ```
   `Document` needs a cheap "what changed" signal. Options:
   - **Option A (simple):** add `change_counter: u64` to `Document`; bump on any
     mutation; record a `HashSet<NodeId>` of dirty nodes in a `dirty_nodes` field reset
     each frame after geometry rebuild.
   - **Option B (richer):** per-`SceneNode` `version: u64` bumped by each mutation.
   Option A is simpler and sufficient for this milestone.

2. **Incremental `build_geometry`:**
   - On each call, take a snapshot of `dirty_nodes` from the document, then release the
     lock.
   - Re-tessellate only nodes in `dirty_nodes`; update `node_geo_cache`.
   - Assemble the full vertex + index buffer by concatenating cached entries in draw
     order. This concatenation is O(nodes) in pointer chasing but skips all expensive
     lyon tessellation for unchanged nodes.
   - On a pure camera change (detected by comparing `view: CanvasView` fields), skip
     `build_geometry` entirely and only upload the updated `CameraUniform` buffer.

3. **Document mutation tracking** — add to `Document`:
   ```rust
   pub dirty_nodes: HashSet<NodeId>,
   pub change_counter: u64,
   ```
   Every `&mut self` method that touches a node must insert the `NodeId` into
   `dirty_nodes`. `history.rs` commit methods are the natural injection points.

4. **Benchmark suite** — add `crates/photonic-render/benches/render_bench.rs`:
   - Use `criterion` crate.
   - Synthetic document builders: `make_doc(n: usize)` filling n rectangles with
     gradients, `make_doc_effects(n)` adding gaussian glows.
   - Measure: first-frame tessellation, re-tessellate after one-node edit, camera-only
     redraw. Report peak vertex buffer size.
   - CI: add `cargo bench --no-run` to the CI pipeline to ensure benchmarks compile;
     run full bench in a scheduled workflow and store results as an artifact.

5. **Camera-change fast path** — add a `last_view: CanvasView` field to
   `PhotonicRenderer`; if `view == last_view` and `dirty_nodes.is_empty()`, skip
   `build_geometry` and skip the vertex buffer upload.

## Affected Modules

- `crates/photonic-core/src/document.rs` — `dirty_nodes: HashSet<NodeId>`, `change_counter`
- `crates/photonic-core/src/history.rs` — inject dirty-node marking on commit
- `crates/photonic-render/src/renderer.rs` — `node_geo_cache`, incremental `build_geometry`,
  `last_view` camera fast path
- `crates/photonic-render/benches/render_bench.rs` — new Criterion benchmark file
- `.github/workflows/` — CI benchmark job

## Risks & Open Questions

- **Buffer assembly cost:** concatenating cached `Vec<Vertex>` slices in draw order is
  O(total vertex count) per frame even with no dirty nodes; could be avoided by keeping
  a persistent GPU buffer with sub-allocations, but that is a much larger change.
- **Draw order sensitivity:** geometry cache entries store flat vertex/index buffers; if
  draw order changes (layer reorder), indices must be rebased. Need index remapping or
  a per-node index offset scheme.
- **Group nodes:** groups have no geometry themselves; their children's cached entries
  must be invalidated when the group's transform changes.
- **Effect nodes:** `GaussianGlowJob` is rebuilt every frame (`pending_gaussian_glows`).
  Caching glow geometry is a separate concern (depends on blur sigma in screen space,
  which changes with zoom).

## Acceptance Criteria

- [ ] Editing one node does not re-tessellate any other node (verified via a tracing
      counter or unit test).
- [ ] Camera pan/zoom skips `build_geometry` entirely.
- [ ] Criterion benchmarks exist and run: 100 / 1k / 10k node documents measured.
- [ ] Benchmark baseline numbers documented in a file in `crates/photonic-render/benches/`.
- [ ] CI runs benchmarks in a scheduled job and stores the output artifact.

## Effort Estimate

**L** — incremental tessellation requires coordinated changes across `document.rs`,
`history.rs`, and `renderer.rs`; benchmarks are M on their own but testing the dirty
propagation correctly requires care.
