# Render the 15 Advanced Blend Modes (#17) — Design Proposal

> Status: design scaffold (not an implementation).

## Summary

`BlendMode` (16 variants, `crates/photonic-core/src/layer.rs:49-67`) is stored on both
`Layer` and `SceneNode` but the renderer hardwires `PREMULTIPLIED_ALPHA_BLENDING` for
every draw call (`crates/photonic-render/src/pipeline.rs:303`). Setting any non-Normal
blend mode via MCP or the GUI has no visible effect. This proposal wires the blend mode
into the GPU pipeline.

## Scope

**In:**
- Separable modes (Multiply, Screen, Darken, Lighten, Difference, Exclusion, HardLight)
  via per-mode `wgpu::BlendState` where fixed-function hardware can express them.
- Backdrop-read modes (Overlay, SoftLight, ColorDodge, ColorBurn) and non-separable HSL
  modes (Hue, Saturation, Color, Luminosity) via an offscreen-composite shader pass.
- Layer groups with a non-Normal blend mode composited as an isolated group against the
  accumulated backdrop.
- Parity verification against CSS `mix-blend-mode` / Illustrator reference output.

**Out:**
- Extended Porter-Duff alpha compositing beyond `src-over` (out of scope for this issue).
- Blend modes on text nodes (follow-up).

## Proposed Approach

1. **Extend `NodeSnapshot`** in `renderer.rs:703` to capture `blend_mode: BlendMode`.
2. **Separable modes — fixed-function:** create one `wgpu::RenderPipeline` variant per
   mode that can be expressed as `wgpu::BlendState` (Multiply maps to
   `Dst × Src`; Screen maps to `Src + Dst − Src×Dst`, etc.). Add a
   `create_fill_pipeline_blended(mode)` factory next to `create_fill_pipeline` in
   `pipeline.rs`.
3. **Backdrop-read modes — offscreen composite:** introduce a
   `blend_offscreen_tex: wgpu::Texture` (same size as surface). Before drawing a node
   with a backdrop-read mode, blit the accumulated surface into this texture, then run a
   custom `fs_blend` fragment shader that reads both the source pixel and the
   `blend_offscreen_tex` sample and applies the formula in WGSL.
4. **Non-separable HSL modes:** implement `Hue`, `Saturation`, `Color`, `Luminosity` in
   the same `fs_blend` shader by converting RGB ↔ HSL in WGSL.
5. **Group isolation:** `GroupNode` with non-Normal `blend_mode` must render its children
   to a temp texture first, then composite that texture against the backdrop using the
   group's blend mode. Detect this during the `build_geometry` traversal.
6. **SVG export:** `crates/photonic-core/src/export.rs` — emit `mix-blend-mode` on the
   SVG element's `style` attribute.
7. **Golden-image tests:** add a test fixture in `crates/photonic-render/` rendering two
   overlapping rectangles for each mode and pixel-comparing against a reference PNG.

## Affected Modules

- `crates/photonic-render/src/pipeline.rs` — new pipeline variants / `fs_blend` shader
- `crates/photonic-render/src/renderer.rs` — `NodeSnapshot`, `build_geometry`, draw loop,
  offscreen texture management
- `crates/photonic-core/src/export.rs` — SVG `mix-blend-mode` attribute
- `crates/photonic-core/src/layer.rs` — `BlendMode` (read-only, already complete)

## Risks & Open Questions

- **Performance:** offscreen blit per backdrop-read node is expensive on complex scenes.
  May need to gate on whether any backdrop-read mode is actually present in the frame.
- **wgpu `BlendState` limits:** not all blend equations are representable; fallback path
  must remain correct.
- **Group isolation semantics:** CSS/SVG isolate groups automatically when `mix-blend-mode`
  is non-normal; Illustrator's model differs subtly. Need to define Photonic's rule.
- **MSAA:** the surface uses `MSAA_SAMPLES = 4` (`renderer.rs:34`); the offscreen
  composite texture must resolve before sampling.

## Acceptance Criteria

- [ ] Each of the 16 `BlendMode` variants renders output visually matching CSS
      `mix-blend-mode` within a 2/255 per-channel tolerance.
- [ ] Golden-image tests pass for all modes.
- [ ] On-canvas, headless export (`crates/photonic-render/src/headless.rs`), and SVG
      export all agree.
- [ ] Setting `blend_mode = Normal` is a no-op (no pipeline change, no perf cost).
- [ ] No regression on existing Normal-mode rendering benchmarks.

## Effort Estimate

**L** — separable modes alone are M; backdrop-read + HSL modes + group isolation + tests push to L.
