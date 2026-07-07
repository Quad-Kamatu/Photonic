# 00 — Overview: Photonic Video Editor Module

**Spec set:** `docs/specs/video-editor/` · **Kernel:** [SPEC.md](SPEC.md) · **Status:** Draft 0.1 · **Date:** 2026-07-07

This doc set is the technical design for the video editor module (Approach A: timeline-first, node-ready frame-graph IR). SPEC.md is the technology-agnostic contract; docs 01–12 are the design layer. Terminology defined in 01/02 is normative for all other docs.

---

## 1. Vision

Photonic gains a video editing mode: Premiere/CapCut-class timeline editing, Resolve-class color and node compositing, CapCut-class captions — with Photonic's unique advantage that vector documents are native, resolution-independent, animatable timeline citizens. One binary, one document model, one undo history, full MCP (agent) parity.

## 2. Acceptance stories (normative)

**AS-1 Social clip.** Import screen recording + music track → cut on timeline → auto-caption via hosted transcription → styled karaoke captions → switch sequence 16:9 → 9:16 with per-clip reframe → add animated Photonic vector title → quick grade → export MP4 (H.264 preset "Social 9:16").

**AS-2 Short film.** Import several 4K clips → generate proxies → multi-track edit with cross-dissolves → one shot opened as per-clip node composition (keyed overlay merge) → full grade pass with scopes (CDL wheels + curves + LUT) → audio mix with keyframed automation, EQ on dialogue track, music ducking → export master (AV1 high-quality) + web H.264.

**AS-3 Motion graphics.** Animate a Photonic vector document with keyframes (transform + path/fill properties) → composite over footage in a node composition → caption + grade → export WebM/ProRes-style with alpha.

Each build phase (§6) ends with a named slice of one story demonstrably working.

## 3. Architecture summary

```
                photonic-app (winit loop, mode dispatch)
                        │
        ┌───────────────┼──────────────────┐
   photonic-gui    photonic-mcp       photonic-video   ← NEW crate (engine)
   (egui panels,   (handlers/video.rs,  (media pool, decode,
    timeline UI,    timeline tools)      frame-graph eval,
    monitor, mode)                       playback clock, audio
        │               │                mixer, export)
        └───────┬───────┴───────┬────────┘
          photonic-render   photonic-core
          (wgpu passes,     (timeline data model:  ← NEW module core/src/timeline/
           video texture     sequences, tracks,
           path, scopes)     clips, keyframes,
                             grades, node graphs)
```

Layering rules (normative):
- `photonic-core::timeline` is pure data + pure functions (interpolation, edit ops). No I/O, no GPU, no threads. Everything serde + undoable.
- `photonic-video` owns all video I/O and the temporal engine. Depends on core + render. Never depends on gui.
- `photonic-gui` renders panels and forwards intents; it never decodes or evaluates graphs itself.
- `photonic-mcp` calls the same core edit ops and `photonic-video` services the GUI uses (CAP-019 parity by construction).

### The frame-graph IR (the load-bearing idea of Approach A)

The timeline never renders directly. For any (sequence, time) the **compiler** (`photonic-video::graph::compile`) produces a `FrameGraph`: a DAG of typed image/audio operations (decode, rasterize-vector, transform, effect, merge, grade, LUT, caption-overlay, output). The evaluator executes it on wgpu with per-node caching.

Per-clip node compositions (D-06) are user-authored subgraphs stored in the data model; the compiler splices them in place of that clip's **source op** — clip-level transform, effects, grade, and reframe still apply on top (the Resolve Fusion→Edit→Color model). The project-level graph is spliced between sequence output and the output node. Fusion pages (Phase 8) therefore add UI + node types, not a new engine.

### Working color space (D-09)

Video path: linear-light Rec.709 primaries, premultiplied alpha, `Rgba16Float` textures. Decoded YUV converts to linear at GPU upload; encode converts back at export. Vector/raster assets entering the video graph convert sRGB→linear at the boundary (`03-render-color-pipeline.md` §4 defines exact transfer functions and the reconciliation with the existing renderer's sRGB conventions).

## 4. Locked decisions

D-01…D-10 in [SPEC.md](SPEC.md#decisions). Every doc cites decisions it depends on.

## 5. Document map

| Doc | Contents | Depends on |
|---|---|---|
| 01-data-model.md | Time representation, timeline/track/clip/keyframe/grade/graph/caption/audio types, serialization v3, undo commands, memory strategy | — |
| 02-engine.md | photonic-video crate: frame-graph compile/eval, decode (ffmpeg-sidecar), caches, playback clock, A/V sync, proxies, threading, export loop | 01 |
| 03-render-color-pipeline.md | Renderer prerequisite work (dirty tracking, persistent buffers, COMPOSITE_SHADER), video texture path, color-space design, f16 pipeline | 01, 02 |
| 04-ui-mode-timeline.md | AppMode mechanism, bottom timeline panel, program monitor, mode-adaptive panels, keyboard model | 01, 02 |
| 05-import-export.md | Media pool, probing, import flows, export presets, encoders, aspect-ratio/reframe system, compression options | 01, 02 |
| 06-captions-ai.md | Caption data model usage, provider trait (hosted transcription + TTS default), styling/karaoke, SRT/VTT/ASS interchange | 01, 02 |
| 07-color-grading.md | Grade node stack, CDL/wheels/curves/HSL/LUT operators, scopes (compute-shader histograms), color page UI | 01, 02, 03 |
| 08-fusion-node-flows.md | Node type catalog, per-clip + project graphs, egui-snarl UI, eval semantics, caching | 01, 02, 04 |
| 09-audio-mixer.md | Audio engine (cpal), mixer graph, automation, EQ/compressor, ducking, waveform pyramid, meters | 01, 02 |
| 10-mcp-tools.md | Tool surface for all video domains, schema/dispatch/args wiring, headless renderer access | all |
| 11-testing-phasing.md | Golden-frame corpus, A/V sync tests, perf budgets, per-phase exit criteria | all |
| 12-agent-execution-plan.md | Implementation agent roles, model tiers (Fable/Opus/Sonnet), parallelism waves, file-conflict boundaries | 11 |

## 6. Phases

| Phase | Delivers | Story slice unlocked |
|---|---|---|
| P1 Renderer foundation | Dirty tracking, persistent GPU buffers, COMPOSITE_SHADER wired, f16 video texture path (D-10) | — (prerequisite; vector editing gets faster) |
| P2 Time + timeline core | `core::timeline` data model, v3 format, undo commands, timeline panel UI, cut/trim/split/ripple, mode switch | AS-1: arrange + cut |
| P3 Playback + media | photonic-video engine: decode, frame graph v1 (decode→transform→merge→output), A/V playback, media pool, proxies | AS-1: play; AS-2: proxy edit |
| P4 Import/export + reframe | Export presets, encoder integration, aspect-ratio system, mobile preview | AS-1 complete except captions |
| P5 Captions + AI audio | Provider trait, hosted transcription + TTS, caption track UI, styling | AS-1 complete |
| P6 Keyframes + motion | Keyframe curves UI, animatable vector documents in timeline, transitions catalog (08 §2.0b), starter vector title-template set (D-11), effect params animatable | AS-3 core |
| P7 Color page | Grade operators, wheels/curves/LUT UI, scopes | AS-2 grade pass |
| P8 Fusion + full mixer | Per-clip/global node UI + node catalog, audio EQ/compression/automation/ducking | AS-2, AS-3 complete |

Phase ordering constraints and intra-phase parallelism: `12-agent-execution-plan.md`.

## 7. Top risks

| Risk | Mitigation |
|---|---|
| Renderer rework (P1) destabilizes vector editing | P1 lands behind golden-output comparison against current renderer; existing tests + new snapshot corpus gate the merge |
| Frame-graph IR designed too narrow for fusion phase | 08 authors review 02's IR before P3 code starts (explicit gate in 12) |
| ffmpeg-sidecar seek latency hurts scrub feel | Keyframe index at import + decoded-frame ring cache + proxies (02 §5); measured budget in 11 |
| Full-mixer scope (D-05) blows P8 | Mixer engine core lands in P3 with playback (gain/pan only); EQ/comp/automation are additive DSP nodes later |
| Color-space unification breaks existing canvas==export guarantee | Video path is additive; vector paths keep current behaviour until P7 revisits unification with tests |
