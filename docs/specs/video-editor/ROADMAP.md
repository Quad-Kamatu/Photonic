# Video Editor Roadmap

**Status:** Live authoritative backlog  
**Date:** 2026-07-10

## 0. Implementation progress — `feat/video-editor-module`

Landed and pushed on this branch (most recent first). Each row is committed with tests green; adversarially verified where noted.

| Area | Status | Commit | Notes |
|---|---|---|---|
| **03 §4.5.3** grade operand space (A-3) | ✅ done | `a05ec8e` | grade ops run straight-alpha (unpremult→op→repremult); shared `ALPHA_EPS` with GPU shader |
| **03 §4.5.4** live canvas linear (A-1) | ✅ done | `5a62494` | document renders to offscreen sRGB target, blitted into egui; canvas blends in linear like headless. *Follow-up:* CPU compositor isolated-layer path is P7-deferred (§4.3) |
| **26 §8 K-0.1** export wiring | ✅ done | `fca5eda` | `EngineCmd::Export` renders via shadow session + progress/cancel; one path shared engine+MCP; e2e ffprobe test |
| **26 §8 K-0.2** render passthrough effects | ✅ done | `fca5eda`+`5a83c50` | all 6 declared effects render: Luma/Chroma/Mask + Blur/Sharpen/Glow; shared separable Gaussian (`ops::blur` + dual-pass GPU `textureLoad`); CPU/GPU parity |
| **26 §8 K-0.3** GPU Merge blend modes (E-9) | ✅ done | `9b2ec60` | GPU Merge honours all 26 modes; CPU/GPU parity sweep across 6 IR enums |
| **26 §8 K-0.4** Wipe/Push passes | ✅ done | `ca6538b` | real `WipeMix`/`PushMix` IR + CPU kernels + WGSL twins; CPU/GPU parity per direction×t; no P3 cross-dissolve fallback |
| **26 §8 K-0.5** `lut_provider` threading | ✅ done | `ca6538b` | `LutProvider` trait + `compile_with_luts`; session `LutCache` warms on snapshot change; grade `Lut3d` resolves to real tables (or inert identity) |
| **26 §8 K-0.6** audio FX chain + mixer + meter | ✅ done | `367511d`+`bf1d89b` | mixer owns track/master fx, discontinuity policy, declick tail; G-4 meter publish + 31 §3 latency compensation closed |
| **26 §8 K-0.7** export audio mux + loudness | ✅ done | `d9f1826` | offline `Mixer::render_block` mix for export range; mux via existing encoder audio sidecar; two-pass `LoudnessTarget` constant gain with true-peak ceiling |
| **26 §8 K-0.8** `EngineCmd::Probe` | ✅ done | `d9f1826` | probe file → `set_asset_meta` + content hash; invalidate decode source |
| **26 §8 K-0.9** `sync_lock` propagation | ✅ done | `ca6538b` | `expand_sync_lock_ripple` in core; insert/extract/ripple_delete/ripple_trim all expand; GUI + MCP ride the same batch |
| **30** effect manifest (E-3/X-4) | ✅ done | `48fb5da`,`49bd585` | schema + 7 authored manifests + `EffectKind`↔`EffectId` bridge + migration/inert-unknown; MCP `list_effect_kinds`/`set_effect_param` generated with range refusal. *Remaining:* full raster bridge catalogue (K-B16 beyond the 6-kernel slice) |
| **31 §2/§3** DSP reset/latency contracts (E-10) | ✅ done | `1ccbeea` | mandatory `reset(AudioDiscontinuity)` + latency/tail across all units |
| **35** markers, effect scopes, groups | ✅ done | `9b2ec60`,`367511d`,`49bd585` | marker categories/anchors, clip markers, group tree; Track/master/asset effect scopes applied in compile; V4→V5 migration; version bumped |
| **36** error model (taxonomy) | ✅ done | `a05ec8e` | `core::diag` taxonomy + catalogue tests. *Remaining:* wire `EngineStatus.last_error` → `Diagnostic` |
| **37** robustness | ✅ done | `a05ec8e` | capability floor, gpu_state, atomic_write, child reaping, scale targets, CI split |
| **38** sequence semantics | ✅ done | `435a3a6`+`ca6538b` | transition handle-clamp, fade-out-at-gap, nest outer-format, frame-rate conform diagnostics, nest dedup; LoadNotice + transition-out-at-cut validation/migration |
| **39 §2.2** unknown-preserving variants | ✅ done | `a05ec8e` | forward-compat inert round-trip |
| **40 §7** spec verification infra | ✅ done | `a05ec8e` | `tools/spec-extract`, drift + acceptance-index scripts |
| **41** accessibility | ✅ done | `1ccbeea`,`a05ec8e` | curve-editor/node-editor focus fixes, keyboard-gate + contrast lints |
| **42 §10** localization (text metrics) | ✅ done | `a05ec8e` | `core::text_metrics` Unicode cell measurement + caption wiring |
| **28 §9** MCP transport hardening | ✅ done | `a05ec8e` | bearer token, no permissive CORS |
| **29 QA-1** acceptance-story harness | 🟡 scaffold | `a05ec8e` | harness + fixtures scaffolded; `video-p1-contract` gate removed |
| **GUI** export progress + diag surface | ✅ done | `1acd25f` | live progress/cancel dialog; diagnostic badge view-model |
| **G-4** master meter publish | ✅ done | `bf1d89b` | feeder publishes `StereoMeter` → `EngineStatus.master_level`; GUI `master_level()` reads it; MCP `get_audio_meters` when live |
| **31 §3** latency compensation | ✅ done | `bf1d89b` | per-track delay lines equalise paths to max latency; `graph_latency_samples` on status for A/V offset |
| **K-B16** raster bridge | ✅ done (catalogue) | `7a7f38d`+`73e6a66` | **38 bridged ids** with manifests + Linear/Transfer operand spaces; multi-point curves (5 knots + contrast); GPU for surface/lens/smart_sharpen + prior twins. Residual polish only: richer curves UI widgets in inspector, exact bilateral/disc parity goldens |
| **E-1** source-range contract | ✅ done | `dd7ef59` | `graph::source_range` — `FrameRange`, per-op identity default, graph union, soft cap (16); TimeOffset still compile-expanded |
| **E-4** threading capability | ✅ done | `dd7ef59` | `ir::Threading` + `threading_for_op` (Any / PerInstance / Serial; undeclared → Serial) |
| **K-G6** interlaced detection | ✅ done | `dd7ef59`+`85b0cea`+`466fbeb` | `ScanType` + probe + pool badge; **deinterlace IR node auto-inserted** for interlaced assets |
| **Effects browser ↔ MANIFESTS** | ✅ done | `85b0cea` | Effects drawer driven by full `MANIFESTS` catalogue (K-B16 ids), grouped by `EffectCategory`; drag/double-click uses `ClipEffect::from_manifest`; inspector labels resolve via manifest name |
| **G-17** sequence tabs | ✅ done (shell) | `85b0cea` | Timeline header tab strip: open/activate/close tabs, `+` create, context Duplicate/Rename; nested breadcrumb pop; `ops_bridge` create/duplicate helpers |
| **K-F1** render queue | ✅ done | `bf1d89b`+`7bbd978`+`f867e95` | `export::RenderQueue` multi-job FIFO; GUI queue inspector panel + multi-format/marker enqueue |
| **K-F2** marker multi-export | ✅ done | `7bbd978` | export dialog "per ranged marker" checkbox → one job per marker×format via RenderQueue |
| **K-F3** multi-format render | ✅ done | `7bbd978` | format checklist → one job per checked `Sequence.formats` entry via RenderQueue |
| **K-F4** job options | ✅ done | `f867e95` | `RenderJobOptions` (proxies, preview res, encoder speed, raw args) on `ExportJob`; dialog collapsible |
| **K-F5** hardware encoders | ✅ done | `f867e95` | probe NVENC/VAAPI/VideoToolbox/QSV; prefer-HW fail-closed; detection report + raw-args hatch |
| **32 §4** playback policy | ✅ done | `f867e95` | `playback::policy::PlaybackPolicy` constants (prefill/drops/ring) + unit pin |
| **32 §7** scale-invariance guard | ✅ done | `f867e95` | Draft vs downsampled Full tolerance tests (CPU+GPU) on geometry+blur fixture |
| **K-G6** deinterlace node | ✅ done | `466fbeb`+`b88aadd` | `IrOp::Deinterlace` CPU + **GPU WGSL twins**; source-range; auto-insert for interlaced assets |
| **E-2** analysis foundation | ✅ done (substrate) | `466fbeb` | `graph::analysis` — typed `AnalysisResult` (Histogram/Levels), content-hash cache, pull-based pure functions. Consumers (scopes/scene/loudness) wire next |
| **K-E4** extract frame | ✅ done | `466fbeb` | `export::extract_frame` PNG path (export colour convert); GUI `video.extract_frame` / `…_to_bin` (Ctrl+Shift+E) via program-monitor readback |
| **E-1** prefetch ← source-range | ✅ done | `b88aadd` | `lead_from_source_range` / `combined_prefetch_lead`; session cut-ahead lead = max(cut-ahead, graph window) so deinterlace expands decode warm |
| **K-G6** GPU deinterlace | ✅ done | `b88aadd` | WGSL twin of spatial methods (OneField / LinearBlend / YadifSpatial) on `IrOp::Deinterlace` — preview no longer blit-combs |
| **K-E1** vectorscope guides | ✅ done | `b88aadd` | I/Q lines + 75%/100% boxes + labels on scopes panel vectorscope overlay |
| **K-C2** usage count | ✅ done (slice) | `b88aadd` | derived clip-reference count on media pool (`×N` / ON TL badge); tags/ratings still open |

**Not yet started (next bands):** K-A residual; K-C tags/ratings/relink/archive; K-D; E-2 consumers; legal-or-fixture-blocked G/D items.

## 1. Authority and precedence

This file owns live video backlog status, priority, gates, and delivery order. Detailed contracts remain in linked owner docs. Repo-root `ROADMAP.md` remains historical vector/MCP rationale.

Precedence:

1. [SPEC.md](SPEC.md) — product capabilities, constraints, non-goals.
2. [00-overview.md](00-overview.md) through [13-ux-components.md](13-ux-components.md) — normative architecture/design.
3. This roadmap — live status, gates, priority, waves.
4. **Accepted policy and contracts** — [23-legal-open-source-implementation-routes.md](23-legal-open-source-implementation-routes.md), [24-preview-media-load.md](24-preview-media-load.md), [29-qa-spec.md](29-qa-spec.md). These carry an explicit acceptance record; a draft may not override them.
5. **Draft implementation contracts and audits** — [19-editing-velocity-shot-management.md](19-editing-velocity-shot-management.md)–[22](22-dji-advanced-workflows.md), [25](25-performance.md)–[28](28-security-model.md), [30](30-effect-catalogue.md)–[42](42-localization.md). Normative once accepted; advisory until then.
6. [17-nle-parity-round2.md](17-nle-parity-round2.md) and [18-dji-parity.md](18-dji-parity.md) — historical gap rationale only.
7. [DESIGN.md](../../../DESIGN.md) — visual tokens.

**Reserved document numbers:** `28` — **filled**: [28-security-model.md](28-security-model.md). `29` — **filled**: [29-qa-spec.md](29-qa-spec.md), renumbered from `14-qa-spec.md` on 2026-07-20 per [27 H-1](27-spec-audit.md#7-doc-set-hygiene). Neither number is free.

**Owner docs for the K/E/X inventory** (30–35) hold the implementation contracts; [26](26-kdenlive-mlt-parity.md) remains the inventory and ranking, and this roadmap remains live status.

Status semantics:

| Status | Meaning |
|---|---|
| `done` | User outcome and required surfaces exist; protected from regression. |
| `partial` | Useful code exists; required surface, parity, fixtures, or acceptance remains. |
| `open` | Implementation not started beyond scaffolding. |
| `product-blocked` | Conflicts with current SPEC non-goal; no implementation authorization. |
| `legal-or-fixture-blocked` | Design is valid, but release/auto-apply waits on rights, provenance, representative fixtures, or frozen thresholds. |

## 2. NLE inventory

| ID | Status | Live residual | Owner |
|---|---|---|---|
| G-1 | partial | Core planner consolidation; close-all/simplify MCP; acceptance | [19 §4](19-editing-velocity-shot-management.md#4-g-1--add-edit-close-gap-and-simplify-sequence) |
| G-2 | partial | Linked-A/V policy; core/MCP closure | [19 §5](19-editing-velocity-shot-management.md#5-g-2--keyboard-trims) |
| G-3 | partial | Source Monitor consumption; overlap priority | [19 §6](19-editing-velocity-shot-management.md#6-g-3--match-frame-and-reveal-in-project) |
| G-4 | done | Live master meter from mixer feeder via EngineStatus | [19 §7](19-editing-velocity-shot-management.md#7-g-4--program-monitor-master-meter) |
| G-5 | partial | Alt-drop, probe/EOF acceptance | [19 §8](19-editing-velocity-shot-management.md#8-g-5--replace-with-clip--replace-edit) |
| G-6 | done | Protected source-patch/target routing | [19 §2](19-editing-velocity-shot-management.md#2-current-implementation-status) |
| G-7 | partial | GUI create command/menu; paint clarity; goldens | [19 §9](19-editing-velocity-shot-management.md#9-g-7--adjustment-layer-clips) |
| G-8 | partial | Accessibility/extreme-range/per-tab acceptance | [19 §10](19-editing-velocity-shot-management.md#10-g-8--timeline-navigator) |
| G-9 | partial | Shared-state/a11y regression closure | [19 §11](19-editing-velocity-shot-management.md#11-g-9--effect-controls-unification) |
| G-10 | partial | Single-surface marks/peek/Match Frame/Insert from marks shipped + unit tests; dual-pane + source-audio clock still deferred | [20 §4](20-pro-workflows.md#4-g-10--source-monitor-and-true-source-marks), [24](24-preview-media-load.md) |
| G-11 | partial | Rubber-band UI; audio mapping; goldens | [20 §5](20-pro-workflows.md#5-g-11--speed-and-time-remap-ramps) |
| G-12 | partial | Pin-To, protected time, vector templates | [20 §6](20-pro-workflows.md#6-g-12--title-text-and-responsive-graphics-clips) |
| G-13 | open | Modal tool palette and cursors | [19 §12](19-editing-velocity-shot-management.md#12-g-13--modal-timeline-tool-palette-and-cursor-hints) |
| G-14 | partial | Select-forward and display options | [19 §13](19-editing-velocity-shot-management.md#13-g-14--track-select-forward-and-display-menu) |
| G-15 | partial | G-15A attach + detach (GUI/MCP); G-15B toggle; G-15C on-import L7 + policy checkbox; **batch attach-by-name / external camera proxies (`.lrv`/`.lrf`, see 26 K-C3)**; full ingest modal/thresholds still open | [19 §14](19-editing-velocity-shot-management.md#14-g-15--proxy-workflow-polish), [24](24-preview-media-load.md) |
| G-16 | partial | Nest/open/breadcrumb GUI; MCP | [20 §7](20-pro-workflows.md#7-g-16--nested-sequence-ui) |
| G-17 | partial | Tab strip in timeline header (activate/close/create/duplicate/rename); multi-doc tab persistence still open | [20 §8](20-pro-workflows.md#8-g-17--sequence-tabs-and-multiple-open-sequences) |
| G-18 | open | Transcript projection and ripple edits | [20 §9](20-pro-workflows.md#9-g-18--text-based-transcript-editing) |
| G-19 | open | Dedicated two-up Trim Mode | [20 §10](20-pro-workflows.md#10-g-19--dedicated-trim-mode) |
| G-20 | legal-or-fixture-blocked | S4 accepted; synthetic/owned sync corpus and decoder budget | [20 §11](20-pro-workflows.md#11-g-20--multicam) |
| G-21 | partial | Continuous MCP trail for landed NLE verbs | [19 §15](19-editing-velocity-shot-management.md#15-g-21--mcp-parity-for-new-editing-operations) |

Round-one status: **13 of 20 shipped** after G-6. [14](14-nle-parity.md) is superseded; [15](15-thumbnails-waveforms.md) and [16](16-insert-overwrite-editing.md) are delivered.

## 3. DJI inventory

| ID | Status | Live residual/gate | Owner |
|---|---|---|---|
| D-1 | legal-or-fixture-blocked | Photonic transform accuracy/provenance/naming; optional vendor LUT license | [21 §4](21-dji-core-workflows.md#4-d-1--dji-log-and-hlg-normalization) |
| D-2 | legal-or-fixture-blocked | Rights-cleared/Photonic-authored looks; D-1 first | [21 §5](21-dji-core-workflows.md#5-d-2--device-scoped-creative-look-picker) |
| D-3 | legal-or-fixture-blocked | S1 accepted; per-asset rights-cleared content | [21 §6](21-dji-core-workflows.md#6-d-3--starter-music-and-ambient-sfx-library) |
| D-4 | legal-or-fixture-blocked | Beat analyzer, provenance, snap; licensed music fixture/tolerance | [21 §7](21-dji-core-workflows.md#7-d-4--beat-detection-beat-markers-and-beat-snap) |
| D-5 | done | Manual leveling + deterministic auto-crop v1; auto estimate deferred | [21 §8](21-dji-core-workflows.md#8-d-5--completed-horizon-leveling-context) |
| D-6 | partial | Image-sequence ingest/decode/deflicker. **Owns the image-sequence/stop-motion clip outright** — 26 K-C4 is generators only | [21 §9](21-dji-core-workflows.md#9-d-6--hyperlapse-and-timelapse-assembly) |
| D-7 | legal-or-fixture-blocked | DJI dialect fixtures, parser, telemetry binding/HUD | [21 §10](21-dji-core-workflows.md#10-d-7--dji-telemetry-srt-and-text-hud) |
| D-8 | legal-or-fixture-blocked | S5 accepted; standalone CPU/GPU kernels and safety preflight implemented/approved; native still delivery, effect integration, and owned panorama corpus remain | [21 §11](21-dji-core-workflows.md#11-d-8--dji-panorama-reframe-and-little-planet) |
| D-9 | partial | Continuous DJI MCP trail; privacy/doc parity | [21 §12](21-dji-core-workflows.md#12-d-9--mcp-parity-for-dji-core-verbs) |
| D-10 | open | Requires D-7; offline map-tile provider/cache license | [22 §4](22-dji-advanced-workflows.md#4-d-10--full-telemetry-dashboard) |
| D-11 | open | Requires D-4; template schema/location/legal manifests | [22 §5](22-dji-advanced-workflows.md#5-d-11--beat-conformed-edit-templates) |
| D-12 | legal-or-fixture-blocked | S2 accepted; parser audit and gyro/lens fixtures | [22 §6](22-dji-advanced-workflows.md#6-d-12--gyro-metadata-stabilization) |
| D-13 | legal-or-fixture-blocked | S3 accepted; color vectors, encoder matrix, measured budgets | [22 §7](22-dji-advanced-workflows.md#7-d-13--hdrhlg-10-bit-color-pipeline) |
| D-14 | legal-or-fixture-blocked | S5 accepted; capture fixtures; depends D-8 | [22 §8](22-dji-advanced-workflows.md#8-d-14--panorama-stitcher) |
| D-15 | legal-or-fixture-blocked | Labeled boundary/quality corpus and frozen thresholds. **Also owns the generic split-at-detected-cuts workflow** (26 K-B13); consider gating that half on a lighter corpus than the highlight reel needs | [22 §9](22-dji-advanced-workflows.md#9-d-15--shot-detection-and-deterministic-highlight-reel) |

## 3a. Kdenlive/MLT inventory (K/E/X)

Owner: [26-kdenlive-mlt-parity.md](26-kdenlive-mlt-parity.md). Round-3 parity pass against Kdenlive 26.04 and the MLT framework, written clean-room under [23](23-legal-open-source-implementation-routes.md). `K-*` = Kdenlive-derived feature gaps, `E-*` = MLT-derived engine lessons, `X-*` = interop/format. Statuses are per band-group; where a group mixes states the row says so, and [26 §19](26-kdenlive-mlt-parity.md#19-priority-and-dependencies) is authoritative for per-item placement.

| ID | Status | Live residual / gate | Owner |
|---|---|---|---|
| K-0 | ✅ done | **9/9 seams closed** (K-0.1–0.9). See [§0](#0-implementation-progress--feat-video-editor-module) | [26 §8](26-kdenlive-mlt-parity.md#8-k-0--foundations) |
| K-A | open | Preview rendering ([33](33-timeline-preview-render.md)), marker depth ([35 §1](35-model-decisions.md#1-markers)), spacer, snaps, groups, **timecode as a first-class concept**, duration dialog, grab-item, split-audio, subclips, track compositing, fixed playhead | [33](33-timeline-preview-render.md) (K-A1), [26 §9](26-kdenlive-mlt-parity.md#9-k-a--timeline) |
| K-B | partial | Track/master/asset stacks etc. still open. **K-B16 catalogue bridge done** (38 ids, util, multi-point curves, GPU twins) | [30](30-effect-catalogue.md), [26 §10](26-kdenlive-mlt-parity.md#10-k-b--effects-and-compositing) |
| K-B10 | **product-blocked** | Motion tracking conflicts with the SPEC non-goal on object tracking; needs an S-series amendment before authorization. **Distinct from D-12**, whose S2 carve-out explicitly excludes it | [26 §K-B10](26-kdenlive-mlt-parity.md#k-b10--motion-tracking) |
| K-C | partial | Substrate + **usage-count badge**. Open: clip-jobs, tags/ratings, generators, archive/cache pane, relink, import triage, still-cache keying. K-C3 → G-15; K-C4 image-seq → D-6 | [26 §11](26-kdenlive-mlt-parity.md#11-k-c--media-and-bin) |
| K-D | partial | `AudioStreamInfo`/`ChannelMap` probed; mixer and DSP written but unbound. Open: per-stream/per-channel handling, stems export, **boundary declick (K-D5)** | [31](31-audio-architecture.md), [26 §12](26-kdenlive-mlt-parity.md#12-k-d--audio) |
| K-D1 | legal-or-fixture-blocked | Dual-system-sound align of an arbitrary two-clip selection. Reuses G-20's engine but sits **outside** S4's multicam carve-out, so it needs its own tracking | [26 §K-D1](26-kdenlive-mlt-parity.md#k-d1--align-by-sound-and-by-timecode) |
| K-D2 | **product-blocked** | Timeline audio recording conflicts with the SPEC non-goals "Audio recording (import + TTS only in v1)" and "Live capture / streaming input". Needs an **S13** amendment | [26 §K-D2](26-kdenlive-mlt-parity.md#k-d2--timeline-audio-recording--product-blocked) |
| K-E | partial | Scopes + extract-frame + **I/Q/75% vectorscope guides**. Open: YUV/YPbPr switch, audio spectrum, per-clip tap, grids | [26 §13](26-kdenlive-mlt-parity.md#13-k-e--monitor-and-scopes) |
| K-F | partial | **K-F1–F5 done** for the export/render band (queue + inspector, multi-format/marker, job options, HW preflight). Remaining polish: sleep-inhibit, add-to-bin, burn-in overlay, 2-pass, K-F7 one-eval-many-outputs | [26 §14](26-kdenlive-mlt-parity.md#14-k-f--render-and-export) |
| K-G | partial | Project profiles, notes, layouts, templates, undo-history surface open. **K-G6 detection + deinterlace node landed**; profiles/notes/templates still open | [26 §15](26-kdenlive-mlt-parity.md#15-k-g--project) |
| K-H | partial | Continuous MCP trail for landed K-* verbs; sweeps the pre-existing multicam / nested-sequence / duplicate-sequence tool gaps and `get_audio_meters`. `partial` by construction, as G-21/D-9 are | [26 §16](26-kdenlive-mlt-parity.md#16-k-h--mcp-trail) |
| E-1 | partial | source-range contract + **prefetch driven by graph union**; TimeOffset still compile-expanded | [32 §1](32-engine-contracts.md#1-source-range--the-one-mechanism-for-temporal-access) |
| E-2 | partial | Analysis substrate (`AnalysisResult`, cache, histogram/levels); consumers still open | [32 §2](32-engine-contracts.md#2-analysis-nodes), [31 §5](31-audio-architecture.md#5-pull-based-analysis) |
| E-3 | partial | Manifest table + migration live (38 catalogue ids); kernel binding still video-side by id | [30 §2](30-effect-catalogue.md#2-the-manifest) |
| E-4 | done | `threading_for_op` declared for every current `IrOp` | [32 §3](32-engine-contracts.md#3-threading-capability) |
| E-5 | partial | Policy constants land in `playback::policy`; soak coverage still open | [32 §4](32-engine-contracts.md#4-playback-policy) |
| E-6 | partial | Draft/Full scale-invariance CI guard landed (`tests/scale_invariance.rs`); broaden to more geometry ops as catalogue grows | [32 §7](32-engine-contracts.md#7-scale-invariance) |
| E-7 | partial | Byte-budgeted decode window; stated playback/scrub/export seek policy | [32 §5](32-engine-contracts.md#5-seek-policy-and-decode-budgets) |
| E-8 | protected | Ten properties Photonic already holds that MLT's own docs identify as structural weaknesses. **Five are not in 26 §5's PA-list** — single working format in the graph interior, normalization as an explicit compile pass, cut-as-cheap-view, locale-independent serialization, deterministic ordered params — so §9 must protect them explicitly | [26 §E-8](26-kdenlive-mlt-parity.md#e-8--protected-properties-that-are-already-right) |
| E-10 | done | `reset(AudioDiscontinuity)` + latency/tail contracts landed with K-0.6; mixer equalises paths | [31 §2](31-audio-architecture.md#2-contract-1--discontinuity), [31 §3](31-audio-architecture.md#3-contract-2--latency) |
| E-9 | partial | K-0.3 GPU Merge honours 26 modes + `cpu_gpu_parity` sweep; keep expanding as catalogue grows | [32 §8](32-engine-contracts.md#8-cpugpu-equivalence) |
| X-1 | open | MLT XML / `.kdenlive` read-only import with an explicit unsupported-feature report | [34 §3](34-interchange.md#3-x-1--mlt-xml-and-kdenlive) |
| X-2 | open | OpenTimelineIO import/export; Photonic-authored JSON reader/writer preferred over a dependency | [34 §4](34-interchange.md#4-x-2--opentimelineio) |
| X-3 | open | EDL standalone; AAF/FCPXML via X-2 | [34 §5](34-interchange.md#5-x-3--edl) |
| X-4 | partial | Versioned manifests + migration framework live; drift CI still thin | [30 §2.6](30-effect-catalogue.md#26-versioning-and-migration) |

**Ownership resolved so nothing is built twice.** **K-0.6 ⊃ G-4** — *not* an equivalence: G-4 is only "publish a real mixer output-meter snapshot through engine status", has no blockers, and ships first; K-0.6 additionally binds the FX chain and mixer panel (P8). · **K-C3 → G-15** (its "batch attach-by-name" residual). · **K-C4's image-sequence half → D-6.** · **K-B13 → D-15**, which should widen its residual to cover the generic split-at-cuts workflow. · **K-A11 → G-20.** · **K-B10 ≠ D-12** — S2 explicitly excludes object tracking. · **E-1 precedes G-11.**

## 3b. Spec-set audit (A/SD/O/U/M)

Owner: [27-spec-audit.md](27-spec-audit.md). Cross-cutting defects in the spec set itself — contradictions, spec-vs-code drift, unowned capabilities, under-specified contracts, and never-covered areas. Every finding is assigned to an existing owner doc; 27 is a tracker, not an authority.

| Group | Count | Highest sev | Nature | Resolution owner |
|---|---|---|---|---|
| `A-*` contradictions | 13 (2 P0, rest P1/P2) | **P0** | A-1 (live canvas composites in gamma, headless in linear) and A-3 (grade on premultiplied alpha) are **wrong-pixels defects the headless, all-opaque golden corpus structurally cannot see**. The remainder are doc-vs-doc or doc-vs-code drift | SPEC, 01, 02, 03, 05, 07, 10 |
| `SD-*` spec-vs-code drift | 17 (1 already fixed) | P1 | Docs describe a pre-implementation world. SD-3 (format version), SD-7 (export ownership) and SD-11 (drop-frame) actively mislead implementers | 00, 01, 02, 03, 06, 08, 09, 10, 11, 12, 13 |
| `O-*` unowned capabilities | 4 | P1 | CAP-005, CAP-018 and CAP-020 have no owning design doc. CAP-003 was **downgraded to P2** after re-verification — 04 §2.4 and `29-qa-spec.md` do define it | 01, 02, 04 |
| `U-*` under-specified | 8 | P1 | Heading-plus-a-sentence where a contract is needed. U-1 (transition timing) contradicts a validated invariant | 01, 02, 04, 05 |
| `MC-*` never covered | 10 (2 downgraded) | **P0** | Security, error taxonomy, GPU device loss, undo bounds, scale limits, perf gating, a11y, i18n, diagnostics, shortcut conflicts | new doc + 01, 02, 03, 04, 06, 11, 13, 25, product |

**Three P0 findings gate shipping, and all three now have owning specs:** A-1 and A-3 colour correctness → **[03 §4.5](03-render-color-pipeline.md#45-operand-spaces-for-blending-and-grading-normative)** (normative operand spaces, plus the fixtures that would actually observe the defects) · MC-1 security → **[28](28-security-model.md)**.

The P1 band is likewise owned: MC-2/U-2 error taxonomy → [36](36-error-model.md) · MC-3/U-5/MC-5/MC-6 robustness → [37](37-robustness.md) · U-1/O-2/U-7 sequence semantics → [38](38-sequence-semantics.md) · O-3/O-4/A-4 document lifecycle → [39](39-document-lifecycle.md).

**An adversarial re-verification on 2026-07-20 refuted seven of 27's original findings**, including both of its original P0s. They are rewritten in place with notes on what was wrong — see [27 §1](27-spec-audit.md#1-why-this-document-exists). Treat 27's code citations as needing re-derivation before use; its document citations held up.

**A new owner doc is required for M-1** — no existing doc has a plausible home for a security model. Proposed `28-security-model.md`.

## 3c. Later audits and cross-cutting contracts

| Group | Owner | Note |
|---|---|---|
| `QA-*` — audit of 12, 13, 29 | banners in [12](12-agent-execution-plan.md), [13](13-ux-components.md), [29](29-qa-spec.md) | 20 findings. **QA-1 is P0**: the CAP-019 acceptance-story harness that SS-2/SS-3 and §10 item 10 depend on **does not exist**. Two code actions recommended: remove the `video-p1-contract` feature gate (8 tests excluded from CI), and either implement or delete doc 12's `video` kill-switch |
| Spec drift + acceptance tracking | [40](40-spec-verification.md) | **10–11** of 27's 17 drift findings are structurally checkable; the repo already proves the pattern with `docs/mcp-api.md`. [40 §2.1](40-spec-verification.md#21-what-this-tool-would-not-have-caught) records what it would **not** have caught, and [§3.6](40-spec-verification.md#36-the-complement-lints-and---all-features) gives the two cheaper mechanisms that catch those |
| Accessibility (MC-7) | [41](41-accessibility.md) | Two **verified code defects**: curve-editor arrow-nudge dies when the widget gains focus; node-editor keyboard shortcuts require the mouse to hover. DESIGN.md's palette fails WCAG AA on 7 token pairs |
| Localization (MC-8) | [42](42-localization.md) | — |

## 4. Corrected priority bands

| Band | Order | Exit condition |
|---|---|---|
| A — unblock editing spine | G-10 (single-surface marks/peek per 24); residual G-1–G-4; D-1 validation route; preview/load budgets in 24 | Source marks + fast Draft preview unambiguous; live meter/core parity; D-1 transform fixture gate |
| B — shot management | G-5, G-7, G-13–G-15, residual G-9 | Discoverable GUI + shared core/MCP paths |
| C — pro/core DJI depth | G-11, G-12, G-16+G-17, D-4, D-7, D-6, D-2 | Per-item fixtures and prerequisites green |
| D — gated differentiators | D-10, D-11, G-18, G-19, D-15; G-20/D-3/D-8/D-12/D-13/D-14 after item evidence gates | Legal/fixture/content evidence and mini-spec acceptance green |
| Trail | G-21, D-9, K-H | Tool/schema/docs/tests land with each verb, never as late epics |

K/E/X band placement is a strict mapping onto [26 §19.1](26-kdenlive-mlt-parity.md#19-priority-and-dependencies)'s bands, which own the per-item detail:

| Roadmap band | 26 band |
|---|---|
| **A** | K-Band 0, plus E-6 and K-E2 pulled forward (both are correctness, not polish) |
| **B** | K-Band 1 remainder, then K-Band 2 (the E-* primitives) |
| **C** | K-Band 3, then K-Band 4 |
| **D** | K-Band 5 and the X-* interop set |
| — | 26's **Blocked** row (K-B10, K-D2, K-D1) is **not banded**; it is not backlog until its gate clears |

K-C3 does not appear here: it is tracked under **G-15**, not as a K/E/X row.

## 5. Dependency graph

```mermaid
flowchart TD
    G6[G-6 done] --> G10[G-10 Source Monitor]
    G3[G-3 Match Frame] --> G10
    G10 --> G5[G-5 Replace]
    G10 --> G19[G-19 Trim Mode]
    G16[G-16 Nest] <--> G17[G-17 Tabs]
    G21[G-21 MCP trail] -. follows .-> G1[G-* landed verbs]
    D1[D-1 Normalize] --> D2[D-2 Looks]
    D4[D-4 Beats] --> D11[D-11 Templates]
    D7[D-7 Telemetry] --> D10[D-10 Dashboard]
    D8[D-8 Reframe] --> D14[D-14 Stitcher]
    D9[D-9 MCP trail] -. follows .-> D1
    K0[K-0 foundations] --> KB[K-B effects]
    K0 --> KF[K-F render]
    E3[E-3 effect manifest] --> KB
    E2[E-2 analysis node] --> D4
    E2 --> D15[D-15 shot detect]
    E2 --> KD1[K-D1 align]
    E1[E-1 source range] --> G11[G-11 time remap]
    KA2[K-A2 marker categories] --> KF2[K-F2 multi-export]
    G4[G-4 master meter] --> K06[K-0.6 audio binding]
    X2[X-2 OTIO] --> X3[X-3 AAF/FCPXML]
```

Accepted 2026-07-12: S1–S5. Residual hard gates: item-specific legal/fixture evidence; D-1 vendor bytes→S6. D-4 and D-11 never run in parallel; D-7 and D-10 never run in parallel.

## 6. Conflict-free delivery waves

Owner-doc prefixes are stable scheduling IDs. Same-prefix rows sharing files serialize inside that owner plan.

| Wave | Work |
|---|---|
| `19-W0`–`19-W2` | Core velocity planners, live meter, G-13/G-14, proxy identity/UI, NLE MCP/QA |
| `20-W0`–`20-W3` | Source preview, responsive titles, transcript, G-10/G-11, nesting/tabs, G-19, MCP/QA |
| `20-WG` | G-20 after item-specific sync corpus and decoder-budget evidence |
| `21-C0`–`21-C2` | Registry/file-set foundation; D-1 normalization then D-2 looks |
| `21-C3`–`21-C7` | D-3 after rights evidence; D-4 beats; D-6 sequences; D-7 telemetry; D-8 authorized Slice 0 then fixture-gated expansion |
| `21-C8` | D-5 shared pure auto-crop + MCP `level_horizon`; prove GUI/MCP equality |
| `22-A0`–`22-A2` | D-7 prerequisite/D-10 dashboard; D-4 prerequisite/D-11 templates |
| `22-A3`–`22-A6` | D-12/D-13/D-14 after item-specific legal/fixture gates; D-15 after fixtures |
| `22-A7` | D-11/D-15 integration after both independent contracts and fixtures are green |
| `26-K0` | K-0 seam closure, sequenced with the owning P4–P8 phase work |
| `26-K1`–`26-K2` | Band-1 correctness/quick wins; then the E-* engine primitives, each with one shipped consumer |
| `26-K3`–`26-K4` | Workflow depth, then render/delivery after K-0.1/K-0.7 |
| `26-K5` | Mini-spec-gated larger items and the X-* interop set; one accepted mini-spec per item before code |

D-9 ships with each applicable `21-C*` feature wave. It is a continuous parity trail, never a late standalone epic.

## 7. Legal, content, and product gates

[23-legal-open-source-implementation-routes.md](23-legal-open-source-implementation-routes.md) is the accepted implementation policy for permissive dependencies, Photonic-owned alternatives, S1–S5 scope, clean-room controls, rights manifests, and item stop/go checks. Product/legal/engineering acceptance recorded 2026-07-12; empirical fixture/dependency/release evidence remains required.

### D-1 transform routes

Preferred route: Photonic-authored analytical math from published colorimetry, or clean-room calibration from independently captured chart footage and published facts. Native transform and optional Photonic-authored sampled `.cube` share one identity and equivalence fixtures. Vendor LUT values must never be sampled, reconstructed, or used as calibration oracle.

Optional route: user-installed or redistribution-licensed vendor LUT, with declared input/output signal domain. No vendor bytes ship without permission. Photonic transforms require accuracy, held-out, CPU/GPU, provenance, trademark, and compatibility-naming review; never label them “official DJI LUTs.”

Other gates:

- D-2: Photonic-authored/commissioned looks or licensed vendor assets.
- D-3: SPEC amendment plus per-asset rights manifest.
- D-4/D-7/D-12/D-14/D-15: representative legally usable fixtures and frozen tolerances.
- D-10: offline tile provider/cache license; render cannot require network.
- D-11: template location/format and bundled asset manifests.
- Telemetry/GPS/transcripts stay local; no content logging or upload.

### K/E/X gates

- **K-B10** and **K-D2**: each needs its own accepted S-series SPEC amendment before any code (object tracking and audio recording are current non-goals). No amendment, no authorization.
- **K-D1**: same sync corpus and frozen confidence thresholds as G-20; it is the same engine.
- **K-B7** luma-map wipes and **K-C4** generators: any *bundled* image or audio byte needs an `AssetRightsManifest` per [23 §7.2](23-legal-open-source-implementation-routes.md#72-manifest). Prefer runtime synthesis from the cited standards, which avoids the gate.
- **K-G4** project templates: template storage location and bundled-asset manifests must be settled first — the same gate S11 places on D-11.
- **K-F5** hardware encoders and **K-A1** preview-chunk codecs: codec/patent/distribution record per [23 §10.3](23-legal-open-source-implementation-routes.md#103-patent-and-distribution-gate); availability is preflighted and fails closed, never inferred at runtime.
- **X-1** MLT XML import: implement from the published DTD only; fixtures must be Photonic-authored, never scraped from a GPL project's test suite.
- **X-2** OTIO and **K-E1** audio spectrum: if a dependency is chosen over a Photonic-authored implementation, it needs a [23 §3.3](23-legal-open-source-implementation-routes.md#33-required-evidence-record) evidence record first. `rustfft` remains an `ADOPT` candidate, not an approval.

## 8. Architecture decisions and defaults

| ID | Decision/default | State |
|---|---|---|
| S1 | Narrow offline starter-audio carve-out from [§23 S1](23-legal-open-source-implementation-routes.md#s1--d-3-starter-audio). | Accepted 2026-07-12 |
| S2 | Gyro-metadata-only stabilization carve-out from [§23 S2](23-legal-open-source-implementation-routes.md#s2--d-12-stabilization). | Accepted 2026-07-12 |
| S3 | Explicit per-sequence HDR working state from [§23 S3](23-legal-open-source-implementation-routes.md#s3--d-13-hdr-and-10-bit). | Accepted 2026-07-12 |
| S4 | Local-file multicam carve-out from [§23 S4](23-legal-open-source-implementation-routes.md#s4--g-20-multicam). | Accepted 2026-07-12 |
| S5 | Still-panorama stitch/reframe carve-out from [§23 S5](23-legal-open-source-implementation-routes.md#s5--d-8d-14-still-panoramas). | Accepted 2026-07-12 |
| S6 | Prefer validated Photonic analytical/clean-room transforms; vendor LUT optional and license-gated. | Resolved default |
| S7 | G-10 source marks are session-only/non-undoable in v1. | Resolved |
| S8 | G-5 preserves slot: final video frame holds; audio is silent after EOF. | Resolved |
| S9 | G-12 uses additive Responsive Position + Protected Time schema from 20; freeze before implementation. | Contract drafted |
| S10 | D-10 requires an offline-capable map-tile provider/cache license. | Open gate |
| S11 | D-11 template storage/location must be chosen before bundled templates. | Open gate |
| S12 | Status audit resolved by this roadmap; code signals remain `partial` until full acceptance. | Resolved |
| S13 | Narrow carve-out for local voiceover recording to a timeline track (K-D2). | **Drafted** in [23 §4.7](23-legal-open-source-implementation-routes.md#47-proposed-amendments-s13s14--drafted-not-accepted); recommendation **accept**; awaiting product decision |
| S14 | Narrow carve-out for planar region tracking as an analysis node (K-B10), distinct from D-12's gyro-only stabilization. | **Drafted** in [23 §4.7](23-legal-open-source-implementation-routes.md#47-proposed-amendments-s13s14--drafted-not-accepted); recommendation **defer** until E-2 ships with a simpler consumer; patent review mandatory |

## 9. Protected surfaces

Do not regress:

- G-6 source-patch boxes, explicit targets, lock/kind validation, deterministic fallback.
- Track locks/hatch, Solo, linked A/V, labels, FX badges. (Sync-lock: see the note below.)
- Trim/ripple/roll/slip/slide; Delete/ripple-delete; copy/cut/paste.
- Insert/Overwrite/Lift/Extract; razor split; markers.
- Thumbnails/waveforms; monitor scrub; playback resolution; Fit/100%; shortcut rebinding.
- D-5 manual horizon correction + deterministic centered auto-crop.
- Existing vector editing, file compatibility, undo, offline operation, ffmpeg sidecar-only rule.

Additionally, the **Photonic-ahead register** ([26 §5](26-kdenlive-mlt-parity.md#5-photonic-ahead-register-pa---do-not-port-backwards), **A-1 – A-9 and A-12 only**): content-hashed frame graph with per-node caching · colour-managed linear working space · GPU-first single-backend evaluation · roll and slide trim · sidechain ducking · per-sequence formats · half-open ranges · flicks `Tick` with exact rational frame rates · typed model and errors · dead-branch elimination. A reference NLE's *limitation* is not a requirement — do not port backwards.

Plus the five [26 E-8](26-kdenlive-mlt-parity.md#e-8--protected-properties-that-are-already-right) properties not covered by that list: a **single working format** in the graph interior · **normalization as an explicit, testable compile pass** · **cut as a cheap view** over an `Arc` source · **locale-independent serialization** · **deterministic ordered effect params** (required for SS-3 and stable undo diffs).

**A-10 and A-11 are deliberately excluded from this list.** They are design intent, not currently-held properties: A-10 (one deterministic audio path across playback and export) is untrue while the FX chain is inert and export strips audio; A-11 (full MCP parity) is untrue while multicam, nested-sequence and duplicate-sequence tools are missing and `get_audio_meters` never succeeds. They become protectable at K-0.6/K-0.7 and at K-H closure respectively. Protecting an aspiration hides the gap.

**`sync_lock` is also removed from the protected list** until K-0.9 lands: `toggle_sync_lock` flips a bit with no consumer, so there is currently no behaviour to regress. [17](17-nle-parity-round2.md)'s "M-9 ✅ DONE" refers to the control, not the propagation.

## 10. Definition of done

Item becomes `done` only when:

1. Core op/engine service exists with unit tests.
2. GUI route exists, or an explicit approved GUI exception is recorded.
3. MCP tool/schema/generated docs land for automatable capability.
4. One user verb produces one undo unit; undo/redo identity passes.
5. Additive serde/migration round-trip passes when model changes.
6. New pixel/audio path has IR/eval/golden/sync coverage under [11-testing-phasing.md](11-testing-phasing.md).
7. **Hard gates green; trend metrics reviewed and not regressed beyond threshold.** Per [37 §4.2](37-robustness.md#42-recommendation-two-tiers-and-be-honest-about-which-is-which), the two are different: *hard gates* are deterministic and machine-independent (graph-compile budget, SS-3 A/V drift, export determinism, cache invariants, scale-invariance and CPU/GPU parity) and **block in PR CI**; *trend metrics* are hardware-dependent (eval ms/frame, seek latency, decode throughput, export wall time) and alert on a >20% regression against a rolling median rather than failing the build. The previous wording — "budgets remain green" — was unachievable against [11 §4](11-testing-phasing.md), which makes benches advisory, and gating on CI-runner noise trains people to ignore CI.
8. Offline, privacy, licensing, content, and product gates pass.
9. Protected surfaces regressions are absent.
10. Goal-backward L1–L4 verification proves stated user outcome, including GUI/MCP parity.
