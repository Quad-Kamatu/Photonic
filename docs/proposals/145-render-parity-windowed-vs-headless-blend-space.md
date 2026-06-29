# Render Parity: Windowed vs Headless Blend Space (#145)

> Status: **implemented** (Option 1 — export is the source of truth). The
> windowed document pass now blends in the same sRGB-encoded (linear-blended)
> space the headless exporter already uses, via a shared scene-format constant
> and a swapchain sRGB `view_format`. No extra blit.

## What this PR implements

1. **Shared scene format.** `crates/photonic-render/src/pipeline.rs` defines
   `pub const SCENE_FORMAT: wgpu::TextureFormat =
   wgpu::TextureFormat::Rgba8UnormSrgb` — the single source of truth for the
   document blend space. `headless.rs` replaced its private `FORMAT` with
   `const FORMAT = SCENE_FORMAT`, so the export path is defined in terms of it.

2. **Swapchain sRGB view format.** In `renderer.rs::new`, the renderer derives
   `scene_format = surface_format.add_srgb_suffix()` (e.g. `Bgra8Unorm →
   Bgra8UnormSrgb`) and pushes it into `surface_config.view_formats`. The new
   `scene_format` field is stored on `PhotonicRenderer`. egui / text / glow keep
   drawing into the default **non-sRGB** swapchain view (they expect linear
   bytes); only the document pass uses the sRGB view.

3. **Document pipelines + MSAA on `scene_format`.** `fill_pipeline`, the
   `blend_pipelines` variants, and the document MSAA target
   (`create_msaa_texture`, plus the `resize` rebuild) are built against
   `scene_format`. `fill_pipeline_1spp`, the blur pipelines, the glow textures,
   and the glyphon text atlas stay on `surface_format` (out of scope).

4. **`begin_frame` sRGB resolve.** The document pass resolves its MSAA target
   into an sRGB view of the swapchain texture; the non-sRGB `view` is kept in
   `FrameHandle` for the subsequent text / glow / egui passes.

5. **`capture_png` parity.** The in-app screenshot/clipboard path mirrors the
   swapchain layout: `capture_tex` keeps the non-sRGB base format (for the text
   pass and byte read-back) with the sRGB `view_format` added, the capture MSAA
   is built in `scene_format`, and the document pass resolves into the sRGB
   view. Read-back bytes are therefore sRGB-encoded, matching
   `headless::render_png` exactly.

6. **Adapter fallback.** If `add_srgb_suffix()` does not yield a distinct sRGB
   format, the renderer logs a warning and keeps `scene_format ==
   surface_format` (today's non-sRGB document pass) — never a hard failure.

7. **Regression guard.** Parity is now structural (identical pipeline blend
   states into identical-encoding targets on both paths). Tests added in
   `pipeline.rs`: `scene_format_is_srgb_for_linear_blending` (the blend-space
   invariant) and `windowed_scene_format_derivation_is_srgb` (the windowed
   derivation yields a distinct sRGB format for every accepted surface format).
   The existing #17 headless golden test
   (`headless::blend_tests::separable_blend_modes_match_reference`) continues to
   guard value-level parity for the now-shared `SCENE_FORMAT`.

## Remaining work

- **Live on-screen swapchain parity test.** A fully automated windowed-vs-export
  pixel comparison needs a real surface and is not feasible in headless CI; the
  shared-const invariant + capture-path + headless golden test are the practical
  guard. (Unchanged from the original risk assessment.)
- **Text / glow / egui in linear space.** These overlay after the document pass
  and still composite in non-sRGB space — out of scope (the acceptance criteria
  target separable blend modes and semi-transparent fills).
- **Non-separable shader-composite blend modes (#17 step 3-4).** Not yet
  implemented; when added they inherit the unified `SCENE_FORMAT` target for
  free.

---

## Original design notes

## Summary

The windowed renderer and the headless export renderer composite the document into
render targets with **different colour encodings**, so GPU fixed-function blending runs in
different spaces and the same document is not pixel-identical on-canvas vs. in exported
output.

- **Windowed** (`crates/photonic-render/src/renderer.rs:186-198`): the document pass and
  its MSAA target use the **non-sRGB linear** surface format (`Bgra8Unorm`/`Rgba8Unorm`)
  chosen so egui doesn't double-gamma-correct. With a non-sRGB attachment the blend unit
  operates on the **gamma-encoded bytes** directly.
- **Headless** (`crates/photonic-render/src/headless.rs:21`): `FORMAT =
  Rgba8UnormSrgb`. The blend unit decodes to **linear**, blends, then re-encodes.

Separable modes (Multiply/Screen) and src-over over partial alpha are non-linear, so the
two paths diverge. The #17 golden tests assert the **linear** (headless) result, so they
guard headless but not the window. We make the window match headless (export is the source
of truth — issue's preferred Option 1).

## Scope

**In:**
- Drive the windowed **document fill/blend pass** through a render target whose *view
  format* is the same sRGB format headless already uses, so both paths blend in linear
  space with identical pipeline blend states.
- Keep egui / text / glow drawing on the **non-sRGB** view of the same swapchain texture
  (egui still wants a linear-byte target — unchanged).
- Apply the same sRGB target to the in-app `capture_png` path so screenshot/clipboard
  output also matches export.
- A regression guard tying both paths to a single shared scene-format constant.

**Out:**
- Moving text / glow / egui compositing into linear space (they overlay after the
  document pass; not part of the acceptance criteria, which target separable blend modes
  and semi-transparent fills).
- The non-separable shader-composite blend modes from #17 step 3-4 (not yet implemented;
  when added they inherit the unified target for free).
- Any change to the document model, file format, or colour-management/ICC profiles.

## Proposed Approach

The cheapest correct realisation of Option 1 avoids an extra blit by exploiting wgpu
**swapchain `view_formats`**: a single swapchain texture can expose both a non-sRGB view
(for egui) and an sRGB view (for the document resolve target).

1. **Shared scene format.** Add `pub const SCENE_FORMAT: wgpu::TextureFormat =
   wgpu::TextureFormat::Rgba8UnormSrgb;` in `pipeline.rs` and have `headless.rs` replace
   its private `FORMAT` with it. Single source of truth for "blend space".

2. **Surface config** (`renderer.rs:200-210`): set
   `view_formats: vec![surface_format.add_srgb_suffix()]` (guarded — see Risks). Store the
   sRGB variant as `scene_format` on `Renderer`.

3. **Document MSAA target** (`create_msaa_texture`, `renderer.rs:1719`): create it in
   `scene_format` (sRGB) with the MSAA used only by the document pass. The glow path keeps
   `fill_pipeline_1spp` + glow textures on `surface_format` (untouched — out of scope).

4. **Fill/blend pipelines** (`renderer.rs:235-260`): build `fill_pipeline` and the
   `blend_pipelines` variants against `scene_format` instead of `surface_format`
   (`fill_pipeline_1spp`, blur, text, glow pipelines stay on `surface_format`).

5. **begin_frame** (`renderer.rs:408-431`): create an **sRGB resolve view** of the
   swapchain texture
   (`create_view(&TextureViewDescriptor{ format: Some(scene_format), .. })`) and pass it as
   the document pass's `resolve_view`; keep the existing non-sRGB `view` in `FrameHandle`
   for the subsequent text/glow/egui passes. MSAA(sRGB) → swapchain-sRGB-view resolve
   writes correctly-encoded bytes that egui then reads/overlays as linear bytes.

6. **capture_png** (`renderer.rs:1520-1547`): create `capture_tex` and capture MSAA in
   `scene_format`; read-back bytes are then sRGB-encoded, matching `headless.render_png`
   exactly. (The trailing capture text pass keeps its own view as today.)

7. **Regression guard.** Because both paths now run identical pipeline blend states into
   identical `SCENE_FORMAT` targets, parity is structural. Add a test asserting the
   invariant (`renderer`'s scene format == `headless` `SCENE_FORMAT`) and lean on the
   existing #17 headless golden test (`headless.rs:990`) plus the capture path (which now
   exercises the windowed pipelines) for value-level parity.

## Affected Modules

- `crates/photonic-render/src/pipeline.rs` — new `SCENE_FORMAT` const.
- `crates/photonic-render/src/renderer.rs` — surface `view_formats`, `scene_format` field,
  MSAA + fill/blend pipeline formats, `begin_frame` sRGB resolve view, `capture_png`.
- `crates/photonic-render/src/headless.rs` — consume `SCENE_FORMAT`.

## Risks & Open Questions

- **Adapter support for the sRGB `view_format`.** `Bgra8Unorm↔Bgra8UnormSrgb` view
  reinterpretation is broadly supported, but guard it: only push the sRGB variant into
  `surface_config.view_formats` (and use the sRGB resolve view) when present; otherwise
  log a warning and fall back to the current non-sRGB document pass. The fallback keeps
  today's behaviour, never a hard failure.
- **MSAA resolve semantics** are identical to headless by construction (same MSAA format,
  same resolve), so no new divergence is introduced there.
- A fully windowed (on-screen swapchain) automated parity test needs a real surface and is
  not feasible in headless CI; the shared-const invariant + capture-path + headless golden
  test together are the practical guard.
