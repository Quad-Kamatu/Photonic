# 05 — `render/renderer.rs` Decomposition

**File:** `render/src/renderer.rs` (2,259). **Track:** render (independent).

Mostly a set of **well-isolated render passes** (mechanical to extract) around
one genuinely complex method, `build_geometry` (840 L). `PhotonicRenderer` stays
as the orchestrator; passes move to sibling files. The crate already has sibling
render modules (`compositor.rs`, `pipeline.rs`, `tessellator.rs`, `text_path.rs`),
so the pattern is established.

## Structure
| Cluster | Lines | |
|---|---|---|
| Struct + init (device/queue/surface, `FrameHandle`, job structs) | 55–362 | keep in `renderer.rs` |
| Accessors | 365–423 | keep |
| Frame lifecycle (`update`/`begin_frame`/`finish_frame`) | 426–472 | → `frame_manager.rs` |
| Text pass (glyphon) | 481–593 | → `text_renderer.rs` |
| Gaussian glow pass | 594–731 | → `glow_renderer.rs` |
| `build_geometry` (tessellator loop, 6 node kinds) | 781–1,620 | **Codex** |
| `record_document_pass` / `render_scene` | 1,621–1,737 | → `scene_renderer.rs` |
| Effects layer (blur bg, composite) | 1,738–1,907 | → `effects_renderer.rs` |
| `capture_png` (offscreen export) | 1,979–2,194 | → `capture.rs` |
| `push_camera` + `CameraUniform` | 760–780, utils | → `camera.rs` |

---

## WP-5A — Extract isolated render passes  ·  **Hermes (Qwen 3.7) ×7**  ·  no deps
Each pass is a self-contained method + its job struct, moved to its own file as
an `impl PhotonicRenderer` block. All CI-verifiable via the golden PNG snapshot
(§6 of the master plan — pixel-identical output proves the pass survived).

| Sub-WP | New file | Source | Isolation |
|---|---|---|---|
| 5A-1 | `text_renderer.rs` | 481–593 (+`TextSnapshot`) | high — nothing else depends on it |
| 5A-2 | `glow_renderer.rs` | 594–731 (+`GaussianGlowJob`) | high — orthogonal pass |
| 5A-3 | `effects_renderer.rs` | 1,738–1,907 (+`BlurJob`) | high — ping-pong textures |
| 5A-4 | `scene_renderer.rs` | 1,621–1,737 | med — thin wgpu wrapper |
| 5A-5 | `frame_manager.rs` | 426–472 + resize | med — surface config/MSAA |
| 5A-6 | `capture.rs` | 1,979–2,194 | high — isolated export path |
| 5A-7 | `camera.rs` | 760–780 + `CameraUniform` | high |

Optional Layer-B tidy: define a `RenderPass` trait so passes register uniformly.
Lower value than the MCP/history registries; treat as stretch.

**Watch (R-render):** wgpu texture lifecycle — MSAA/glow/effect texture sizes
must stay in sync or the GPU panics. Codex should, in a tiny pre-pass, centralize
the texture builders (`create_msaa_texture`, `create_glow_textures`, `align256`)
into a `textures.rs` **before** the effects/glow passes are handed to Hermes, so
each pass file doesn't re-derive sizing. (Small Codex scaffold, like MCP 1A.)

---

## WP-5B — Refactor `build_geometry`  ·  **Codex**  ·  the one hard piece
Lines 781–1,620 (~840 L): the main tessellator loop, branching per node kind
(path/group/text/raster + fills/strokes/variable-width/text-on-path). Dense,
node-kind-specific, and the hot path. Codex should split by node kind into
`geometry/{path,text,raster,group}.rs` behind a small dispatch, **carefully
preserving tessellation output** (golden snapshot is the guard). Not mechanical —
the branches share tessellator state and ordering matters.

## Summary
| WP | Tier | Model | Deps |
|---|---|---|---|
| 5A (+texture-builder scaffold) | Hermes (Codex scaffold) | Qwen 3.7 | scaffold first |
| 5B build_geometry | Codex | — | after 5A |

Same self-conflict caveat as the app/panels tracks: 5A sub-WPs all edit
`renderer.rs`, so **serialize them** (or one agent, sequential) rather than
fanning out onto the same file.
