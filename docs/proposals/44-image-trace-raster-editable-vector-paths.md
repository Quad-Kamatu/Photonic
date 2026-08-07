# Image Trace: Raster → Editable Vector Paths with Presets (#44) — Design Proposal

> Status: design scaffold (not an implementation).

## Summary

No image tracing exists. The `image` crate is already present in the workspace (used by `photonic-render` for canvas screenshots and headless output). The `ops/` module in `photonic-core` has boolean and stroke operations but no rasterization-to-vector pipeline. Image Trace requires: (a) an `ImageNode` to trace (M3 dependency), (b) a tracing engine that converts pixel data to `PathData`, and (c) a non-destructive "live trace" object in the document model that remembers the source image and trace settings so the user can re-trace with different parameters. The end state is "Expand" which replaces the live trace with plain `PathNode`s.

## Scope (in / out)

**In:**
- Tracing engine: B/W (threshold-based), grayscale, and color quantization modes.
- Presets: B/W Logo, Line Art, Sketched Art, 3/6/16 Colors, Grayscale, Silhouettes, Technical Drawing — each preset is a named `TraceOptions` value.
- Controls: mode, color palette source, color count (2–256), threshold (B/W), noise suppression (min path area), path fidelity (curve smoothing tolerance), corner threshold, fills vs. strokes output, ignore-white toggle, snap curves to lines.
- `ImageTraceNode` (or `LiveTrace` wrapper): stored in the document alongside the source `ImageNode`; holds `TraceOptions` and a cached result `Vec<PathData>`. Re-traces when options change.
- **Expand**: replaces the `ImageTraceNode` with concrete `PathNode`s via `Command::Batch`.
- MCP tool: `image_trace(node_id, preset, options)` → returns traced node IDs; `expand_trace(node_id)`.
- Progress callback / streaming result for large images (trace can be slow).

**Out:**
- AI-powered semantic tracing — out of scope.
- Centerline tracing (for line art producing strokes, not fills) — defer to a follow-up.
- GPU-accelerated tracing — out of scope for V1.
- Tracing of raster layers from PSD (depends on #41 ImageNode) — automatically works once M3 + #41 land.

## Proposed Approach

### Tracing Engine

1. **Dependency**: Add `vtracer = "0.6"` (pure Rust, MIT; wraps a potrace-style algorithm with color clustering) to `photonic-core/Cargo.toml`. `vtracer` takes RGBA pixel data and outputs SVG path strings, which can be parsed by `PathData::from_svg`. Alternatively, integrate `potrace-rs` (wraps libpotrace; faster for B/W but adds a C dep). For V1, use `vtracer` (no native dep, adequate quality for the preset list).

2. **`TraceOptions` struct** (`crates/photonic-core/src/ops/image_trace.rs` — new file):

```rust
pub struct TraceOptions {
    pub mode: TraceMode,              // Bw | Grayscale | LimitedColor | FullColor
    pub color_count: u8,              // 2–256 (LimitedColor)
    pub threshold: u8,                // 0–255 (Bw / Grayscale)
    pub noise_suppression: u32,       // min pixels to keep (suppress speckles)
    pub path_fidelity: f64,           // curve smoothing; 0.0 (coarse) – 1.0 (exact)
    pub corner_threshold: f64,        // radians; below = sharp corner
    pub output_fills: bool,
    pub output_strokes: bool,
    pub ignore_white: bool,
    pub snap_curves_to_lines: bool,
}

pub fn named_presets() -> HashMap<&'static str, TraceOptions> { … }
```

3. **`trace_image` function**:

```rust
pub fn trace_image(
    rgba: &[u8], width: u32, height: u32, opts: &TraceOptions
) -> Result<Vec<PathData>, TraceError>
```

- For `TraceMode::Bw`: threshold RGBA → 1-bit bitmap; pass to vtracer's B/W tracing; parse SVG `d` strings back to `PathData::from_svg`.
- For `TraceMode::LimitedColor` / `FullColor`: quantize to `color_count` using a median-cut algorithm (implement in ~100 lines or use the `quantette` crate); for each color layer, threshold and trace; combine into a list of `(PathData, Fill::solid(color))` pairs.
- Apply noise suppression: discard paths whose bounding box area < `noise_suppression` px².
- Apply `path_fidelity`: run `PathData::convert_to_smooth` (existing method) if fidelity is high, or simplify via `ops/simplify.rs` (existing) if low.

4. **`ImageTraceNode` variant** (add to `SceneNodeKind` in `node.rs`, or as a wrapper `GroupNode` with metadata):

```rust
pub struct ImageTraceNode {
    pub source_id: NodeId,        // the ImageNode being traced
    pub options: TraceOptions,
    pub cached_paths: Vec<(PathData, Fill)>,  // cached result; None = not yet traced
}
```

On document open, if `cached_paths` is empty, re-run `trace_image`. On option change, invalidate and re-run (can be async).

5. **Expand**: `Command::Batch` containing `Command::RemoveNode` (the `ImageTraceNode`) + one `Command::AddNode` per `(PathData, Fill)` in `cached_paths`.

6. **Renderer**: Render an `ImageTraceNode` by drawing its `cached_paths` as filled `PathNode`s. If not yet traced, show the source image with a "tracing…" overlay.

7. **MCP**: `image_trace(node_id, preset?, options?)` replaces the source `ImageNode` with an `ImageTraceNode` and returns the new node ID. `expand_trace(node_id)` expands to paths.

### Preset Table (implementation reference)

| Preset            | Mode          | Colors | Threshold | Fidelity | Noise | Ignore white |
|-------------------|---------------|--------|-----------|----------|-------|--------------|
| B/W Logo          | Bw            | 2      | 128       | 0.75     | 25    | true         |
| Line Art          | Bw            | 2      | 200       | 0.9      | 10    | true         |
| Sketched Art      | Grayscale     | 8      | 128       | 0.6      | 5     | false        |
| 3 Colors          | LimitedColor  | 3      | —         | 0.75     | 25    | false        |
| 6 Colors          | LimitedColor  | 6      | —         | 0.75     | 25    | false        |
| 16 Colors         | LimitedColor  | 16     | —         | 0.75     | 10    | false        |
| Grayscale         | Grayscale     | 16     | —         | 0.75     | 10    | false        |
| Silhouettes       | Bw            | 2      | 100       | 0.5      | 50    | true         |
| Technical Drawing | Bw            | 2      | 150       | 0.95     | 5     | true         |

## Affected Modules

- `crates/photonic-core/src/ops/image_trace.rs` — new: `TraceOptions`, `trace_image`, presets
- `crates/photonic-core/src/ops/mod.rs` — add `pub mod image_trace`
- `crates/photonic-core/src/node.rs` — add `ImageTraceNode`, new `SceneNodeKind::ImageTrace` variant
- `crates/photonic-core/src/history.rs` — expand trace in `Command`
- `crates/photonic-core/Cargo.toml` — add `vtracer = "0.6"`
- `crates/photonic-render/src/renderer.rs` — render `ImageTraceNode`
- `crates/photonic-gui/src/panels/mod.rs` — Image Trace panel (preset picker, option sliders, Expand button)
- `crates/photonic-mcp/src/server.rs` + `protocol.rs` — `ImageTraceArgs`, `ExpandTraceArgs` handlers

## Risks & Open Questions

- **`ImageNode` dependency (M3)**: Cannot trace a placed image until `ImageNode` and its pixel data accessor exist. The tracing engine (`trace_image`) can be unit-tested independently with raw pixel data; wiring to `ImageNode` is a separate step.
- **`vtracer` quality**: vtracer produces good B/W results but color quantization is basic. If quality is insufficient for the preset list, swap to a two-step approach: `quantette` for color quantization → `vtracer` per color band. Keep the `trace_image` API stable regardless.
- **Trace performance**: A 2000×2000 px image with 16 colors can take 2–5 seconds. Run `trace_image` in a `tokio::task::spawn_blocking` and show a progress indicator in the GUI. The `ImageTraceNode` stores the cached result so re-renders are instant.
- **New `SceneNodeKind` variant**: Adding `ImageTrace` to the enum is a breaking change for any exhaustive match. All `match` sites in `node.rs`, `export.rs`, `renderer.rs`, `history.rs` must add an arm. Consider using `#[non_exhaustive]` on the enum now to make future additions easier.
- **SVG export of live trace**: Export as expanded paths (same as Expand), not as a reference to the source image. This is destructive but ensures the SVG is self-contained.

## Acceptance Criteria

- [ ] `trace_image` produces non-empty `PathData` results for each of the 9 named presets on a test pixel buffer.
- [ ] A placed `ImageNode` can be converted to an `ImageTraceNode` via the MCP tool; changing options re-traces without re-importing the image.
- [ ] Expand replaces the `ImageTraceNode` with plain `PathNode`s that match the trace rendering; undo restores the `ImageTraceNode`.
- [ ] The GUI Image Trace panel shows the preset list and all option controls.
- [ ] Large images (≥2MP) trace in a background thread without blocking the UI.

## Effort Estimate

**L** — The tracing engine integration is compact, but adding a new `SceneNodeKind` variant touches many match sites across multiple crates. The GUI panel, preset table, MCP wiring, and async plumbing add up. Gated on M3 `ImageNode` for full end-to-end functionality.
