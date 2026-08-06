# EPS / AI and PSD Import (#41) — Design Proposal

> Status: design scaffold (not an implementation).

## Summary

No EPS, AI, or PSD import exists. These three legacy formats collectively represent most designer-side source files in professional workflows. AI files since CS (CS1+) are PDF-compatible and can largely reuse the PDF import pipeline from #40. Pure EPS (pre-CS) requires a limited PostScript interpreter. PSD is a completely separate binary format needing layer-aware parsing. Both tracks depend on M2 (blend mode rendering — `BlendMode` enum already exists in `layer.rs`) and M3 (`ImageNode` for raster layers). The scope here is structural fidelity, not pixel-perfect rendering of every feature.

## Scope (in / out)

**In:**
- **EPS/AI**: Parse PostScript path operators from EPS headers / the PDF-embedded AI stream into `PathData`; map fill/stroke to `Fill`/`Stroke`. AI (CS+) delegates to the PDF import path from #40 with an AI-specific OC/layer-name pass.
- **PSD**: Parse layer tree → Photonic `Layer` objects, layer names, visibility, opacity, `BlendMode`. Raster pixel data per layer → `ImageNode` (M3). Text layers → `TextNode` (basic; no complex PS type engine). Shape layers (path records) → `PathNode`.
- Preserve layer order, nesting (layer groups / section dividers), and names.
- Map PSD blend modes to `BlendMode` (PSD uses the same 16 compositing modes, named differently).
- `ImportError` extension for format-specific failures.

**Out:**
- Smart Objects, adjustment layers, layer effects (PSD) — emit as flattened raster `ImageNode` with a warning annotation.
- Pure PostScript (non-EPS, non-PDF-AI) full interpreter — out of scope.
- PSD export.
- CMYK color handling beyond a best-effort RGB conversion.

## Proposed Approach

### EPS / AI

1. **Detection**: Inspect the first 4 bytes. `%!PS` → EPS. `%PDF` → AI (CS+, hand off to `import_pdf` from #40 with the flag `ai_mode: true`). Pre-CS AI (`%AI`) → legacy PostScript path.

2. **EPS module** `crates/photonic-core/src/import_eps.rs`:
   - `pub fn import_eps(bytes: &[u8]) -> Result<Document, ImportError>`.
   - Tokenize the DSC comments and PS body using a hand-rolled tokenizer (no external crate needed for the subset we care about). Track the current path via `m`, `l`, `c`, `v`, `y`, `h`, `f`, `S`, `b` operators (identical semantics to PDF).
   - Translate EPS coordinate system (bottom-left origin, document units from BoundingBox DSC comment) to Photonic's top-left.
   - Ignore PostScript constructs we do not handle; silently skip unknown operators.

3. **AI (CS+)**: Call `import_pdf` with a post-processing pass: extract `%%Layer` DSC comments from the PDF Optional Content Group names and map them to Photonic `Layer` objects in the right order.

### PSD

4. **Dependency**: Add `psd` crate (`psd = "0.3"`, pure Rust, MIT) to `photonic-core/Cargo.toml`. It decodes the full layer tree, pixel data, and text engine descriptors.

5. **New module** `crates/photonic-core/src/import_psd.rs`:
   - `pub fn import_psd(bytes: &[u8]) -> Result<Document, ImportError>`.
   - `struct PsdContext` — holds the decoded `psd::Psd`, accumulates `Layer` and `SceneNode` objects.
   - Walk `psd.layers()` in document order:
     - **Raster layers**: call `layer.rgba()` → raw pixels → `ImageNode` (M3; if unavailable, record a placeholder).
     - **Type layers**: parse `layer.text()` → `TextNode` with font name, size, color.
     - **Shape layers**: parse the vector path records from layer descriptor → `PathData` → `PathNode`.
     - **Group begin/end markers** (section divider layers): map to `GroupNode` nesting.
   - Map blend modes via `blend_mode_from_psd(key: &[u8; 4]) -> BlendMode` helper using the 4-byte PSD blend key table (e.g., `b"norm"` → `BlendMode::Normal`, `b"mul "` → `BlendMode::Multiply`, etc.) against the existing `BlendMode` enum in `layer.rs`.
   - Set `Layer::opacity` from PSD layer opacity.
   - Document canvas size → `Page` width/height.

6. **Tests**: Round-trip a minimal PSD byte string (hand-constructed or fixture file committed to `tests/fixtures/`). Assert layer count, names, blend modes, and node types.

## Affected Modules

- `crates/photonic-core/src/import_eps.rs` — new (EPS/pre-CS AI path tokenizer)
- `crates/photonic-core/src/import_psd.rs` — new (PSD layer parser)
- `crates/photonic-core/src/import.rs` — expose both; extend `ImportError`
- `crates/photonic-core/src/import_pdf.rs` (#40) — add `ai_mode` parameter for AI layer pass
- `crates/photonic-core/src/lib.rs` — re-export new importers
- `crates/photonic-core/Cargo.toml` — add `psd = "0.3"`
- `crates/photonic-mcp/src/server.rs` — add `import_eps` / `import_psd` tool handlers
- `crates/photonic-mcp/src/protocol.rs` — arg structs

## Risks & Open Questions

- **`psd` crate coverage**: The `psd` crate covers pixel data and basic layer info well, but deep text-engine descriptors (styled runs, ligatures) may be incomplete. Accept degraded fidelity (single-style `TextNode`) on complex type layers rather than crashing.
- **PSD shape layers**: Shape paths are stored in `vmsk` / `vsms` resource blocks as Bézier point lists in a PSD-specific coordinate system (relative to layer bounds, 16.16 fixed-point). Conversion to `PathData` requires careful coordinate transform.
- **`ImageNode` dependency**: If M3 is not done, PSD import can only produce text and shape nodes; raster layers become placeholders. This limits usefulness of the feature until M3 ships.
- **Blend mode on `Layer` vs. node**: Same open question as #39. PSD blends per-layer (and per-node in smart objects); Photonic currently stores `blend_mode` on `Layer`. May need to promote to `SceneNode` before this import is fully faithful.
- **EPS operators**: Real-world EPS files often use Level 2 / Level 3 operators and custom dictionaries. Limit scope to the DSC-conformant path/paint subset and document the limitation clearly.

## Acceptance Criteria

- [ ] An EPS file containing basic filled/stroked paths imports as editable `PathNode` objects on a single `Page`.
- [ ] An AI (CS+) file imports via the PDF path with layer names preserved as Photonic `Layer` objects.
- [ ] A PSD file imports with layer tree structure, blend modes, and opacity values correctly mapped; text layers produce `TextNode`s; shape layers produce `PathNode`s.
- [ ] Unsupported PSD features (adjustment layers, smart objects) produce an `ImportError::Warning` (non-fatal) with an annotation in the document, not a panic.
- [ ] Unit tests cover layer count, names, and blend modes for each format.

## Effort Estimate

**L** — Three distinct format parsers, each with significant quirks. AI (via PDF) is cheapest (reuse #40). EPS PostScript subset is medium. PSD has the deepest state to decode (layer tree, pixel data, descriptors). Coordinate system differences add friction across all three.
