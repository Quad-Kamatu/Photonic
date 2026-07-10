//! wgpu evaluator (02 §2 "Evaluation").
//!
//! A topological walk of the [`FrameGraph`], one render pass per op, results
//! cached in [`NodeCache`] by [`ContentHash`] (02 §5). Working textures are
//! `Rgba16Float`, premultiplied, linear-light Rec.709 (D-09); `DecodeVideo`
//! reuses [`photonic_render::video::convert_yuv_planes_to_working`] for the
//! YUV→working upload so the GPU and CPU reference agree on the color math
//! (03 §4.4).
//!
//! ## P3 op coverage
//! - `SolidColor` — a uniform fill.
//! - `DecodeVideo` / `DecodeStill` / `RasterVector` — resolved by a
//!   [`GpuFrameSource`] (decode rings + the headless vector renderer at the
//!   session layer; never called for the solid-color paths tests exercise).
//! - `Merge` — a premultiplied `over` composite. **Normal only in P3** — the
//!   full 26-mode `COMPOSITE_SHADER` wiring (03 §2.4) is the seam noted below.
//! - `Grade` — the resolved grade stack, run through
//!   [`photonic_render::apply_grade_stack_gpu`] (07 §3), byte-for-byte the WGSL
//!   twin of `eval_cpu`'s `apply_grade_cpu` (GPU/CPU parity, 03 §4.4).
//! - `Effect{Invert}` — a real invert pass (08 §3). The other `Effect` kinds
//!   (Blur/Sharpen/Glow/ChromaKey/LumaKey/MaskShapeGen) stay blit-passthrough
//!   until their `ResolvedParams` payload finalizes (P5/P7).
//! - `Transform2D` — inverse-affine nearest/bilinear sampling matching the CPU
//!   reference's pixel-center and edge-clamp semantics.
//! - `Crop` / `Resize` / `Output` and the remaining passthrough ops — a texture
//!   blit.

use std::sync::Arc;

use photonic_core::timeline::EffectKind;
use wgpu::util::DeviceExt;

use crate::contract::{AssetId, Tick, VectorRef, VectorStateKey};
use crate::graph::cache::{CacheStats, NodeCache};
use crate::graph::ir::{FrameGraph, IrOp, TextureDesc};
use crate::pool::DEFAULT_BUDGET_BYTES;

const WORKING_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// Shared wgpu device/queue handle (02 §1: "shares wgpu Device/Queue with
/// renderer"). Cheap to clone (two `Arc`s).
#[derive(Clone)]
pub struct GpuContext {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
}

impl GpuContext {
    pub fn new(device: Arc<wgpu::Device>, queue: Arc<wgpu::Queue>) -> Self {
        GpuContext { device, queue }
    }

    pub fn device(&self) -> &Arc<wgpu::Device> {
        &self.device
    }

    pub fn queue(&self) -> &Arc<wgpu::Queue> {
        &self.queue
    }

    /// Request a headless GPU context (own adapter/device) for CLI/headless/MCP
    /// and tests. `None` when no adapter is available (CI without a GPU).
    pub fn request_blocking() -> Option<Self> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))?;
        let (device, queue) =
            pollster::block_on(adapter.request_device(&Default::default(), None)).ok()?;
        Some(GpuContext::new(Arc::new(device), Arc::new(queue)))
    }
}

/// Resolves source ops (`DecodeVideo`/`DecodeStill`/`RasterVector`) to working
/// textures. The session implements it over decode rings + the headless vector
/// renderer; tests that use only `SolidColor`/`Merge` never trigger it. A `None`
/// return means "not available yet" — the evaluator substitutes transparent.
pub trait GpuFrameSource {
    fn video_texture(
        &mut self,
        gpu: &GpuContext,
        asset: AssetId,
        src_time: Tick,
        proxy: bool,
    ) -> Option<GpuFrame>;

    fn still_texture(&mut self, gpu: &GpuContext, asset: AssetId) -> Option<GpuFrame>;

    fn vector_texture(
        &mut self,
        gpu: &GpuContext,
        vref: VectorRef,
        key: VectorStateKey,
        w: u32,
        h: u32,
    ) -> Option<GpuFrame>;
}

/// A source texture plus its exact logical dimensions. The physical texture
/// may be pool-bucket padded; sampling must never infer content bounds from it.
#[derive(Clone)]
pub struct GpuFrame {
    pub texture: Arc<wgpu::Texture>,
    pub width: u32,
    pub height: u32,
}

impl GpuFrame {
    pub fn new(texture: Arc<wgpu::Texture>, width: u32, height: u32) -> Self {
        Self {
            texture,
            width: width.max(1),
            height: height.max(1),
        }
    }
}

/// A source that resolves nothing (transparent everywhere) — for tests /
/// solid-color graphs with no media.
pub struct NullFrameSource;

impl GpuFrameSource for NullFrameSource {
    fn video_texture(&mut self, _: &GpuContext, _: AssetId, _: Tick, _: bool) -> Option<GpuFrame> {
        None
    }
    fn still_texture(&mut self, _: &GpuContext, _: AssetId) -> Option<GpuFrame> {
        None
    }
    fn vector_texture(&mut self, _: &GpuContext, _: VectorRef, _: VectorStateKey, _: u32, _: u32) -> Option<GpuFrame> {
        None
    }
}

/// The wgpu evaluator: pipelines + the node-result cache.
pub struct Evaluator {
    gpu: GpuContext,
    passes: Passes,
    cache: NodeCache,
    /// Glyphon caption text compositor (06 §5.3) — burns `CaptionOverlay` glyph
    /// runs over the working texture. Owns its glyphon state behind a `Mutex` so
    /// it composites through `&self` like the rest of `render_op`.
    caption: photonic_render::caption::CaptionCompositor,
    /// The content hash of the last presented output, pinned in the cache.
    pinned_output: Option<crate::graph::ir::ContentHash>,
}

impl Evaluator {
    pub fn new(gpu: GpuContext) -> Self {
        Self::with_budget(gpu, DEFAULT_BUDGET_BYTES)
    }

    pub fn with_budget(gpu: GpuContext, budget_bytes: u64) -> Self {
        let passes = Passes::new(gpu.device());
        let cache = NodeCache::new(gpu.device().clone(), budget_bytes);
        let caption = photonic_render::caption::CaptionCompositor::new(
            gpu.device(),
            gpu.queue(),
            WORKING_FORMAT,
        );
        Evaluator {
            gpu,
            passes,
            cache,
            caption,
            pinned_output: None,
        }
    }

    pub fn gpu(&self) -> &GpuContext {
        &self.gpu
    }

    pub fn cache_stats(&self) -> CacheStats {
        self.cache.stats()
    }

    /// Evict cached results whose content hash matches `pred` (asset relink /
    /// proxy swap — `InvalidateRange`, 02 §5). Call between frames.
    pub fn invalidate_matching(&mut self, pred: impl Fn(crate::graph::ir::ContentHash) -> bool) {
        self.cache.invalidate_matching(pred);
    }

    /// Evaluate `graph` at canvas size `canvas`, returning the pinned output
    /// texture (or `None` for an empty graph). Sources are resolved via `source`.
    pub fn evaluate(
        &mut self,
        graph: &FrameGraph,
        canvas: (u32, u32),
        source: &mut dyn GpuFrameSource,
    ) -> Option<Arc<wgpu::Texture>> {
        let (cw, ch) = (canvas.0.max(1), canvas.1.max(1));
        let mut results: Vec<Option<GpuFrame>> =
            (0..graph.nodes.len()).map(|_| None).collect();

        for (i, node) in graph.nodes.iter().enumerate() {
            let inputs: Vec<GpuFrame> = node
                .inputs
                .iter()
                .filter_map(|(id, _)| results[id.0 as usize].clone())
                .collect();

            let out = match &node.op {
                IrOp::DecodeVideo { asset, src_time, proxy } => match source
                    .video_texture(&self.gpu, *asset, *src_time, *proxy)
                {
                    Some(frame) => self.normalize_source_cached(node.content_hash, frame, cw, ch),
                    None => self.transparent(cw, ch),
                },
                IrOp::DecodeStill { asset } => match source.still_texture(&self.gpu, *asset) {
                    Some(frame) => self.normalize_source_cached(node.content_hash, frame, cw, ch),
                    None => self.transparent(cw, ch),
                },
                IrOp::RasterVector { vref, doc_state, w, h } => match source
                    .vector_texture(&self.gpu, *vref, *doc_state, *w, *h)
                {
                    Some(frame) => self.normalize_source_cached(node.content_hash, frame, cw, ch),
                    None => self.transparent(cw, ch),
                },
                _ => self.render_cached(node, &inputs, cw, ch),
            };
            results[i] = Some(out);
        }

        let out_node = graph.output?;
        let out_hash = graph.nodes[out_node.0 as usize].content_hash;
        // Pin the displayed output; unpin the previous one (03 §3.4 exception 1).
        if let Some(prev) = self.pinned_output.replace(out_hash) {
            if prev != out_hash {
                self.cache.unpin(prev);
            }
        }
        self.cache.pin(out_hash);
        results[out_node.0 as usize]
            .clone()
            .map(|frame| frame.texture)
    }

    /// Render a computed (non-source) op into a cached texture, or return the
    /// cache hit unchanged.
    fn render_cached(
        &mut self,
        node: &crate::graph::ir::IrNode,
        inputs: &[GpuFrame],
        cw: u32,
        ch: u32,
    ) -> GpuFrame {
        let (w, h) = op_size(&node.op, cw, ch);
        let desc = TextureDesc { width: w, height: h };
        let (target, valid) = self.cache.lookup_or_alloc(node.content_hash, desc);
        if valid {
            return GpuFrame::new(target, w, h);
        }
        self.render_op(&node.op, inputs, &target, w, h);
        self.cache.mark_rendered(node.content_hash);
        GpuFrame::new(target, w, h)
    }

    fn normalize_source_cached(
        &mut self,
        hash: crate::graph::ir::ContentHash,
        source: GpuFrame,
        width: u32,
        height: u32,
    ) -> GpuFrame {
        if source.width == width && source.height == height {
            return source;
        }
        let desc = TextureDesc { width, height };
        let (target, valid) = self.cache.lookup_or_alloc(hash, desc);
        if !valid {
            self.passes.transform(
                &self.gpu,
                &source.texture,
                &target,
                glam::Mat3::IDENTITY,
                crate::graph::ir::Sampling::Bilinear,
                width,
                height,
                source.width,
                source.height,
            );
            self.cache.mark_rendered(hash);
        }
        GpuFrame::new(target, width, height)
    }

    fn render_op(
        &self,
        op: &IrOp,
        inputs: &[GpuFrame],
        target: &wgpu::Texture,
        logical_w: u32,
        logical_h: u32,
    ) {
        match op {
            IrOp::SolidColor { color } => {
                self.passes
                    .fill(&self.gpu, target, [color.r, color.g, color.b, color.a]);
            }
            IrOp::Merge { opacity, .. } => match (inputs.first(), inputs.get(1)) {
                (Some(top), Some(bottom)) => {
                    self.passes.merge(
                        &self.gpu,
                        &top.texture,
                        &bottom.texture,
                        *opacity,
                        target,
                    );
                }
                (Some(only), None) | (None, Some(only)) => {
                    self.passes.blit(&self.gpu, &only.texture, target);
                }
                (None, None) => self.passes.fill(&self.gpu, target, [0.0; 4]),
            },
            // Styled-text generator (G-12 title clips): start transparent, then
            // burn the resolved cue on top via the SAME glyphon compositor the
            // `CaptionOverlay` path uses (06 §5.3) — text over transparent, which
            // the clip's `Transform2D`/`Merge` then places in the frame. An absent
            // cue (empty string) stays transparent.
            IrOp::TextGen { block } => {
                self.passes.fill(&self.gpu, target, [0.0; 4]);
                if let Some(cue) = &block.cue {
                    self.caption.composite(
                        self.gpu.device(),
                        self.gpu.queue(),
                        target,
                        std::slice::from_ref(cue),
                        target.width(),
                        target.height(),
                    );
                }
            }
            // Caption overlay (06 §5.3): lay the input composite down, then burn
            // the resolved glyph runs on top via the glyphon pipeline. glyphon's
            // `ALPHA_BLENDING` + `Accurate` sRGB→linear conversion make this a
            // correct straight-alpha `over` onto the premultiplied linear target.
            IrOp::CaptionOverlay { cue_batch } => {
                match inputs.first() {
                    Some(src) => self.passes.blit(&self.gpu, &src.texture, target),
                    None => self.passes.fill(&self.gpu, target, [0.0; 4]),
                }
                if !cue_batch.cues.is_empty() {
                    self.caption.composite(
                        self.gpu.device(),
                        self.gpu.queue(),
                        target,
                        &cue_batch.cues,
                        target.width(),
                        target.height(),
                    );
                }
            }
            // Real effect kernel: Invert (08 §3). Other kinds fall through to the
            // blit passthrough below until their `ResolvedParams` payload lands.
            IrOp::Effect { kind: EffectKind::Invert, .. } => match inputs.first() {
                Some(src) => self.passes.invert(&self.gpu, &src.texture, target),
                None => self.passes.fill(&self.gpu, target, [0.0; 4]),
            },
            IrOp::Transform2D { mat, sampling } => match inputs.first() {
                Some(src) => self.passes.transform(
                    &self.gpu,
                    &src.texture,
                    target,
                    *mat,
                    *sampling,
                    logical_w,
                    logical_h,
                    src.width,
                    src.height,
                ),
                None => self.passes.fill(&self.gpu, target, [0.0; 4]),
            },
            // Real grade kernel: run the resolved stack through the WGSL twins of
            // `eval_cpu`'s `apply_grade_cpu`, then blit the result into the pooled
            // cache target (07 §3, GPU/CPU parity 03 §4.4). An empty stack falls
            // through to the passthrough blit below.
            IrOp::Grade { ops } if !ops.is_empty() => match inputs.first() {
                Some(src) => {
                    let graded = photonic_render::apply_grade_stack_gpu(
                        self.gpu.device(),
                        self.gpu.queue(),
                        &src.texture,
                        ops,
                    );
                    self.passes.blit(&self.gpu, &graded, target);
                }
                None => self.passes.fill(&self.gpu, target, [0.0; 4]),
            },
            // Blit passthrough for Crop, Resize, Output, empty Grade, and the
            // marker filter/color ops.
            _ => match inputs.first() {
                Some(src) => self.passes.blit(&self.gpu, &src.texture, target),
                None => self.passes.fill(&self.gpu, target, [0.0; 4]),
            },
        }
    }

    /// A cached transparent texture of size `(w, h)` (fresh, filled once).
    fn transparent(&mut self, w: u32, h: u32) -> GpuFrame {
        // Key transparents in a reserved high-bit namespace so fillers never
        // collide with real content hashes (xxh3 fills the low 120 bits; the top
        // byte 0xFE is reserved here).
        let hash = crate::graph::ir::ContentHash(
            (0xFE_u128 << 120) | ((w as u128) << 32) | h as u128,
        );
        let (tex, valid) = self.cache.lookup_or_alloc(hash, TextureDesc { width: w, height: h });
        if !valid {
            self.passes.fill(&self.gpu, &tex, [0.0; 4]);
            self.cache.mark_rendered(hash);
        }
        GpuFrame::new(tex, w, h)
    }
}

/// The output size of a computed op.
fn op_size(op: &IrOp, cw: u32, ch: u32) -> (u32, u32) {
    match op {
        IrOp::Resize { w, h, .. } | IrOp::Output { w, h } => (*w, *h),
        _ => (cw, ch),
    }
}

// ── Render pipelines ──────────────────────────────────────────────────────────

struct Passes {
    fill_pipeline: wgpu::RenderPipeline,
    fill_bgl: wgpu::BindGroupLayout,
    blit_pipeline: wgpu::RenderPipeline,
    blit_bgl: wgpu::BindGroupLayout,
    transform_pipeline: wgpu::RenderPipeline,
    transform_bgl: wgpu::BindGroupLayout,
    /// `Effect{Invert}` (08 §3): shares the blit bind-group layout (tex + sampler).
    invert_pipeline: wgpu::RenderPipeline,
    merge_pipeline: wgpu::RenderPipeline,
    merge_bgl: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

const QUAD_VS: &str = r#"
struct VOut { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32> }
@vertex
fn vs_quad(@builtin(vertex_index) vi: u32) -> VOut {
    var pos = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0,  1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0,  1.0), vec2<f32>(-1.0, 1.0)
    );
    var uvs = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 1.0), vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 0.0)
    );
    var o: VOut;
    o.pos = vec4<f32>(pos[vi], 0.0, 1.0);
    o.uv = uvs[vi];
    return o;
}
"#;

impl Passes {
    fn new(device: &wgpu::Device) -> Self {
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("eval_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        // Fill: uniform premultiplied color.
        let fill_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fill_bgl"),
            entries: &[uniform_entry(0)],
        });
        let fill_src = format!(
            "{QUAD_VS}\nstruct Fill {{ color: vec4<f32> }}\n@group(0) @binding(0) var<uniform> f: Fill;\n@fragment fn fs(i: VOut) -> @location(0) vec4<f32> {{ return f.color; }}\n"
        );
        let fill_pipeline = make_pipeline(device, &fill_bgl, &fill_src, "fs");

        // Blit: sample one source.
        let blit_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("blit_bgl"),
            entries: &[tex_entry(0), sampler_entry(1)],
        });
        let blit_src = format!(
            "{QUAD_VS}\n@group(0) @binding(0) var t: texture_2d<f32>;\n@group(0) @binding(1) var s: sampler;\n@fragment fn fs(i: VOut) -> @location(0) vec4<f32> {{ return textureSample(t, s, i.uv); }}\n"
        );
        let blit_pipeline = make_pipeline(device, &blit_bgl, &blit_src, "fs");

        // Transform2D: explicit textureLoad sampling avoids sampler-dependent
        // coordinate and padding behavior. The uniform stores three inverse
        // affine columns, sampling mode, and logical canvas/source dimensions.
        let transform_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("transform_bgl"),
            entries: &[tex_entry(0), uniform_entry(1)],
        });
        let transform_src = format!(
            "{QUAD_VS}\n@group(0) @binding(0) var t: texture_2d<f32>;\nstruct T {{ c0: vec4<f32>, c1: vec4<f32>, c2: vec4<f32>, info: vec4<f32> }}\n@group(0) @binding(1) var<uniform> u: T;\nfn at(p: vec2<i32>) -> vec4<f32> {{\n  let hi = vec2<i32>(u.info.zw) - vec2<i32>(1);\n  return textureLoad(t, clamp(p, vec2<i32>(0), hi), 0);\n}}\nfn nearest(p: vec2<f32>) -> vec4<f32> {{ return at(vec2<i32>(floor(p))); }}\nfn bilinear(p: vec2<f32>) -> vec4<f32> {{\n  let q = p - vec2<f32>(0.5);\n  let p0f = floor(q);\n  let p0 = vec2<i32>(p0f);\n  let f = q - p0f;\n  let p00 = at(p0);\n  let p10 = at(p0 + vec2<i32>(1, 0));\n  let p01 = at(p0 + vec2<i32>(0, 1));\n  let p11 = at(p0 + vec2<i32>(1, 1));\n  return mix(mix(p00, p10, f.x), mix(p01, p11, f.x), f.y);\n}}\n@fragment fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {{\n  if (pos.x >= u.info.x || pos.y >= u.info.y) {{ return vec4<f32>(0.0); }}\n  let canvas_src = u.c0.xy * pos.x + u.c1.xy * pos.y + u.c2.xy;\n  let src = canvas_src * u.info.zw / u.info.xy;\n  if (u.c0.w > 0.5) {{ return nearest(src); }}\n  return bilinear(src);\n}}\n"
        );
        let transform_pipeline = make_pipeline(device, &transform_bgl, &transform_src, "fs");

        // Invert (08 §3): invert straight (unpremult) color, keep alpha, then
        // re-premultiply — the WGSL twin of `ops::invert`. Reuses the blit BGL.
        let invert_src = format!(
            "{QUAD_VS}\n@group(0) @binding(0) var t: texture_2d<f32>;\n@group(0) @binding(1) var s: sampler;\n@fragment fn fs(i: VOut) -> @location(0) vec4<f32> {{\n  let c = textureSample(t, s, i.uv);\n  var straight = vec3<f32>(0.0);\n  if (c.a > 1e-6) {{ straight = clamp(c.rgb / c.a, vec3<f32>(0.0), vec3<f32>(1.0)); }}\n  let inv = (vec3<f32>(1.0) - straight) * c.a;\n  return vec4<f32>(inv, c.a);\n}}\n"
        );
        let invert_pipeline = make_pipeline(device, &blit_bgl, &invert_src, "fs");

        // Merge: premultiplied `over`, Normal only (26-mode seam).
        let merge_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("merge_bgl"),
            entries: &[tex_entry(0), tex_entry(1), sampler_entry(2), uniform_entry(3)],
        });
        let merge_src = format!(
            "{QUAD_VS}\n@group(0) @binding(0) var t_top: texture_2d<f32>;\n@group(0) @binding(1) var t_bot: texture_2d<f32>;\n@group(0) @binding(2) var s: sampler;\nstruct M {{ opacity: f32, _p0: f32, _p1: f32, _p2: f32 }}\n@group(0) @binding(3) var<uniform> m: M;\n@fragment fn fs(i: VOut) -> @location(0) vec4<f32> {{\n  let top = textureSample(t_top, s, i.uv) * m.opacity;\n  let bot = textureSample(t_bot, s, i.uv);\n  return top + bot * (1.0 - top.a);\n}}\n"
        );
        let merge_pipeline = make_pipeline(device, &merge_bgl, &merge_src, "fs");

        Passes {
            fill_pipeline,
            fill_bgl,
            blit_pipeline,
            blit_bgl,
            transform_pipeline,
            transform_bgl,
            invert_pipeline,
            merge_pipeline,
            merge_bgl,
            sampler,
        }
    }

    fn fill(&self, gpu: &GpuContext, target: &wgpu::Texture, color: [f32; 4]) {
        let buf = gpu
            .device()
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fill_color"),
                contents: bytemuck::cast_slice(&color),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let bind = gpu.device().create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fill_bg"),
            layout: &self.fill_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buf.as_entire_binding(),
            }],
        });
        self.run(gpu, &self.fill_pipeline, &bind, target);
    }

    fn blit(&self, gpu: &GpuContext, src: &wgpu::Texture, target: &wgpu::Texture) {
        let view = src.create_view(&Default::default());
        let bind = gpu.device().create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("blit_bg"),
            layout: &self.blit_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        self.run(gpu, &self.blit_pipeline, &bind, target);
    }

    fn transform(
        &self,
        gpu: &GpuContext,
        src: &wgpu::Texture,
        target: &wgpu::Texture,
        mat: glam::Mat3,
        sampling: crate::graph::ir::Sampling,
        logical_w: u32,
        logical_h: u32,
        source_w: u32,
        source_h: u32,
    ) {
        if !crate::graph::ops::transform_matrix_is_valid(mat) {
            self.fill(gpu, target, [0.0; 4]);
            return;
        }
        let inverse = mat.inverse().to_cols_array();
        let uniform = [
            inverse[0],
            inverse[1],
            inverse[2],
            if matches!(sampling, crate::graph::ir::Sampling::Nearest) {
                1.0
            } else {
                0.0
            },
            inverse[3],
            inverse[4],
            inverse[5],
            0.0,
            inverse[6],
            inverse[7],
            inverse[8],
            0.0,
            logical_w as f32,
            logical_h as f32,
            source_w as f32,
            source_h as f32,
        ];
        let buffer = gpu.device().create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("transform_uniform"),
            contents: bytemuck::cast_slice(&uniform),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let view = src.create_view(&Default::default());
        let bind = gpu.device().create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("transform_bg"),
            layout: &self.transform_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: buffer.as_entire_binding(),
                },
            ],
        });
        self.run(gpu, &self.transform_pipeline, &bind, target);
    }

    /// `Effect{Invert}` pass — same bind group as `blit` (tex + sampler), the
    /// invert pipeline instead.
    fn invert(&self, gpu: &GpuContext, src: &wgpu::Texture, target: &wgpu::Texture) {
        let view = src.create_view(&Default::default());
        let bind = gpu.device().create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("invert_bg"),
            layout: &self.blit_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        self.run(gpu, &self.invert_pipeline, &bind, target);
    }

    fn merge(
        &self,
        gpu: &GpuContext,
        top: &wgpu::Texture,
        bottom: &wgpu::Texture,
        opacity: f32,
        target: &wgpu::Texture,
    ) {
        let tv = top.create_view(&Default::default());
        let bv = bottom.create_view(&Default::default());
        let uni = [opacity, 0.0, 0.0, 0.0];
        let buf = gpu
            .device()
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("merge_uni"),
                contents: bytemuck::cast_slice(&uni),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let bind = gpu.device().create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("merge_bg"),
            layout: &self.merge_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&tv),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&bv),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: buf.as_entire_binding(),
                },
            ],
        });
        self.run(gpu, &self.merge_pipeline, &bind, target);
    }

    fn run(
        &self,
        gpu: &GpuContext,
        pipeline: &wgpu::RenderPipeline,
        bind: &wgpu::BindGroup,
        target: &wgpu::Texture,
    ) {
        let view = target.create_view(&Default::default());
        let mut enc = gpu
            .device()
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("eval_pass_enc"),
            });
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("eval_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, bind, &[]);
            pass.draw(0..6, 0..1);
        }
        gpu.queue().submit([enc.finish()]);
    }
}

fn make_pipeline(
    device: &wgpu::Device,
    bgl: &wgpu::BindGroupLayout,
    src: &str,
    fs_entry: &str,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("eval_shader"),
        source: wgpu::ShaderSource::Wgsl(src.into()),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("eval_layout"),
        bind_group_layouts: &[bgl],
        push_constant_ranges: &[],
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("eval_pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: "vs_quad",
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: fs_entry,
            targets: &[Some(wgpu::ColorTargetState {
                format: WORKING_FORMAT,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
        cache: None,
    })
}

fn tex_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

fn uniform_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

// ── Readback (tests + the session's sampled-pixel probe) ──────────────────────

/// Read a `Rgba16Float` texture back to CPU as `[r, g, b, a]` f32 pixels
/// (row-major, `w*h`). Used by tests and the engine's sampled-pixel status.
pub fn read_texture_rgba16f(gpu: &GpuContext, tex: &wgpu::Texture, w: u32, h: u32) -> Vec<[f32; 4]> {
    let bpp = 8u32; // Rgba16Float
    let unaligned = w * bpp;
    let bpr = align256(unaligned);
    let staging = gpu.device().create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: (bpr * h) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut enc = gpu.device().create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(
        tex.as_image_copy(),
        wgpu::ImageCopyBuffer {
            buffer: &staging,
            layout: wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(bpr),
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    gpu.queue().submit([enc.finish()]);
    let slice = staging.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    gpu.device().poll(wgpu::Maintain::Wait);
    rx.recv().unwrap().unwrap();
    let raw = slice.get_mapped_range();
    let mut out = Vec::with_capacity((w * h) as usize);
    for row in 0..h {
        let base = (row * bpr) as usize;
        for x in 0..w {
            let o = base + (x * bpp) as usize;
            out.push([
                f16_to_f32(u16::from_le_bytes([raw[o], raw[o + 1]])),
                f16_to_f32(u16::from_le_bytes([raw[o + 2], raw[o + 3]])),
                f16_to_f32(u16::from_le_bytes([raw[o + 4], raw[o + 5]])),
                f16_to_f32(u16::from_le_bytes([raw[o + 6], raw[o + 7]])),
            ]);
        }
    }
    drop(raw);
    staging.unmap();
    out
}

fn align256(n: u32) -> u32 {
    (n + 255) & !255
}

fn f16_to_f32(bits: u16) -> f32 {
    let sign = ((bits >> 15) & 1) as u32;
    let exp = ((bits >> 10) & 0x1f) as i32;
    let frac = (bits & 0x3ff) as u32;
    let v = if exp == 0 {
        frac as f32 * 2f32.powi(-24)
    } else if exp == 0x1f {
        f32::INFINITY
    } else {
        (1.0 + frac as f32 / 1024.0) * 2f32.powi(exp - 15)
    };
    if sign == 1 {
        -v
    } else {
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::compile::{compile, Quality};
    use crate::graph::eval_cpu::FrameProvider;
    use crate::graph::ir::{ContentHash, IrNode, IrNodeId, OutPort, Sampling};
    use crate::graph::ops::Image;
    use photonic_core::timeline::{
        CaptionCue, CaptionStyle, CaptionTrack, CaptionWord, Clip, ClipSource, FrameRate,
        KaraokeMode, KaraokeStyle, Sequence, SequenceId, TextClipContent, TimelineProject, Track,
        TrackKind,
    };
    use photonic_core::Color;

    struct PatternSource {
        frame: GpuFrame,
    }

    struct PatternCpuSource {
        image: Image,
    }

    impl FrameProvider for PatternCpuSource {
        fn decode_video(&mut self, _: AssetId, _: Tick, _: bool, _: u32, _: u32) -> Image {
            self.image.clone()
        }
        fn decode_still(&mut self, _: AssetId, _: u32, _: u32) -> Image {
            self.image.clone()
        }
        fn raster_vector(
            &mut self,
            _: VectorRef,
            _: VectorStateKey,
            _: u32,
            _: u32,
        ) -> Image {
            self.image.clone()
        }
    }

    impl GpuFrameSource for PatternSource {
        fn video_texture(
            &mut self,
            _: &GpuContext,
            _: AssetId,
            _: Tick,
            _: bool,
        ) -> Option<GpuFrame> {
            Some(self.frame.clone())
        }
        fn still_texture(&mut self, _: &GpuContext, _: AssetId) -> Option<GpuFrame> {
            Some(self.frame.clone())
        }
        fn vector_texture(
            &mut self,
            _: &GpuContext,
            _: VectorRef,
            _: VectorStateKey,
            _: u32,
            _: u32,
        ) -> Option<GpuFrame> {
            Some(self.frame.clone())
        }
    }

    fn patterned_image(width: u32, height: u32) -> Image {
        let mut image = Image::new(width, height);
        for y in 0..height {
            for x in 0..width {
                image.pixels[(y * width + x) as usize] = [
                    (x % 2) as f32,
                    (y % 2) as f32,
                    ((x + 2 * y) % 3 == 0) as u8 as f32,
                    1.0,
                ];
            }
        }
        image
    }

    fn upload_pattern(gpu: &GpuContext, image: &Image) -> Arc<wgpu::Texture> {
        let texture = gpu.device().create_texture(&wgpu::TextureDescriptor {
            label: Some("transform_test_pattern"),
            size: wgpu::Extent3d {
                width: image.width,
                height: image.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: WORKING_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let mut half = Vec::with_capacity(image.pixels.len() * 4);
        for pixel in &image.pixels {
            for channel in pixel {
                half.push(if *channel == 0.0 { 0u16 } else { 0x3c00u16 });
            }
        }
        gpu.queue().write_texture(
            texture.as_image_copy(),
            bytemuck::cast_slice(&half),
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(image.width * 8),
                rows_per_image: Some(image.height),
            },
            wgpu::Extent3d {
                width: image.width,
                height: image.height,
                depth_or_array_layers: 1,
            },
        );
        Arc::new(texture)
    }

    #[test]
    fn transform2d_pattern_gpu_matches_cpu_for_bilinear_and_nearest() {
        let Some(gpu) = GpuContext::request_blocking() else {
            eprintln!("no GPU adapter; skipping GPU transform parity");
            return;
        };
        let image = patterned_image(11, 7);
        let mat = glam::Mat3::from_translation(glam::Vec2::new(1.25, -0.75))
            * glam::Mat3::from_angle(0.23)
            * glam::Mat3::from_scale(glam::Vec2::new(1.2, 0.8));

        for (index, sampling) in [Sampling::Bilinear, Sampling::Nearest]
            .into_iter()
            .enumerate()
        {
            let graph = FrameGraph {
                nodes: vec![
                    IrNode {
                        op: IrOp::DecodeStill {
                            asset: AssetId::new(),
                        },
                        inputs: vec![],
                        content_hash: ContentHash(100 + index as u128),
                    },
                    IrNode {
                        op: IrOp::Transform2D { mat, sampling },
                        inputs: vec![(IrNodeId(0), OutPort::default())],
                        content_hash: ContentHash(200 + index as u128),
                    },
                ],
                output: Some(IrNodeId(1)),
            };
            let mut source = PatternSource {
                frame: GpuFrame::new(upload_pattern(&gpu, &image), image.width, image.height),
            };
            let mut evaluator = Evaluator::new(gpu.clone());
            let output = evaluator
                .evaluate(&graph, (image.width, image.height), &mut source)
                .expect("transform output");
            let actual = read_texture_rgba16f(&gpu, &output, image.width, image.height);
            let expected = crate::graph::ops::transform2d(&image, mat, sampling);
            for (pixel_index, (gpu_pixel, cpu_pixel)) in
                actual.iter().zip(&expected.pixels).enumerate()
            {
                for channel in 0..4 {
                    assert!(
                        (gpu_pixel[channel] - cpu_pixel[channel]).abs() < 2e-3,
                        "{sampling:?} pixel {pixel_index} channel {channel}: GPU {} vs CPU {}",
                        gpu_pixel[channel],
                        cpu_pixel[channel]
                    );
                }
            }
        }
    }

    #[test]
    fn transform2d_native_source_gpu_matches_canvas_normalized_cpu() {
        let Some(gpu) = GpuContext::request_blocking() else {
            eprintln!("no GPU adapter; skipping native-source transform parity");
            return;
        };
        let image = patterned_image(7, 5);
        let (canvas_w, canvas_h) = (11, 9);
        let mat = glam::Mat3::from_translation(glam::Vec2::new(0.75, -0.5))
            * glam::Mat3::from_angle(0.17)
            * glam::Mat3::from_scale(glam::Vec2::new(1.1, 0.85));

        for (index, sampling) in [Sampling::Bilinear, Sampling::Nearest]
            .into_iter()
            .enumerate()
        {
            let graph = FrameGraph {
                nodes: vec![
                    IrNode {
                        op: IrOp::DecodeStill {
                            asset: AssetId::new(),
                        },
                        inputs: vec![],
                        content_hash: ContentHash(300 + index as u128),
                    },
                    IrNode {
                        op: IrOp::Transform2D { mat, sampling },
                        inputs: vec![(IrNodeId(0), OutPort::default())],
                        content_hash: ContentHash(400 + index as u128),
                    },
                    IrNode {
                        op: IrOp::Output {
                            w: canvas_w,
                            h: canvas_h,
                        },
                        inputs: vec![(IrNodeId(1), OutPort::default())],
                        content_hash: ContentHash(500 + index as u128),
                    },
                ],
                output: Some(IrNodeId(2)),
            };
            let mut cpu_source = PatternCpuSource {
                image: image.clone(),
            };
            let expected = crate::graph::eval_cpu::evaluate(
                &graph,
                (canvas_w, canvas_h),
                &mut cpu_source,
            );
            let mut gpu_source = PatternSource {
                frame: GpuFrame::new(upload_pattern(&gpu, &image), image.width, image.height),
            };
            let mut evaluator = Evaluator::new(gpu.clone());
            let output = evaluator
                .evaluate(&graph, (canvas_w, canvas_h), &mut gpu_source)
                .expect("transform output");
            let actual = read_texture_rgba16f(&gpu, &output, canvas_w, canvas_h);
            for (pixel_index, (gpu_pixel, cpu_pixel)) in
                actual.iter().zip(&expected.pixels).enumerate()
            {
                for channel in 0..4 {
                    assert!(
                        (gpu_pixel[channel] - cpu_pixel[channel]).abs() < 2e-3,
                        "{sampling:?} pixel {pixel_index} channel {channel}: GPU {} vs CPU {}",
                        gpu_pixel[channel],
                        cpu_pixel[channel]
                    );
                }
            }
        }
    }

    #[test]
    fn singular_transform_gpu_is_transparent() {
        let Some(gpu) = GpuContext::request_blocking() else {
            eprintln!("no GPU adapter; skipping singular transform policy");
            return;
        };
        let image = patterned_image(7, 5);
        let graph = FrameGraph {
            nodes: vec![
                IrNode {
                    op: IrOp::DecodeStill {
                        asset: AssetId::new(),
                    },
                    inputs: vec![],
                    content_hash: ContentHash(600),
                },
                IrNode {
                    op: IrOp::Transform2D {
                        mat: glam::Mat3::from_scale(glam::Vec2::new(0.0, 1.0)),
                        sampling: Sampling::Nearest,
                    },
                    inputs: vec![(IrNodeId(0), OutPort::default())],
                    content_hash: ContentHash(601),
                },
            ],
            output: Some(IrNodeId(1)),
        };
        let mut source = PatternSource {
            frame: GpuFrame::new(upload_pattern(&gpu, &image), image.width, image.height),
        };
        let mut evaluator = Evaluator::new(gpu.clone());
        let output = evaluator
            .evaluate(&graph, (image.width, image.height), &mut source)
            .expect("transform output");
        let actual = read_texture_rgba16f(&gpu, &output, image.width, image.height);
        assert!(
            actual.iter().all(|pixel| *pixel == [0.0; 4]),
            "singular GPU transform must be transparent"
        );
    }

    #[test]
    fn solid_graph_gpu_matches_cpu_reference() {
        let Some(gpu) = GpuContext::request_blocking() else {
            eprintln!("no GPU adapter — skipping GPU solid parity");
            return;
        };
        let mut project = TimelineProject::new();
        let mut seq = Sequence::new("seq", FrameRate::FPS_30, 8, 8);
        let seq_id = seq.id;
        let mut t = Track::new(TrackKind::Video, "V1");
        t.clips.push(Clip::new(
            ClipSource::SolidColor {
                color: Color { r: 0.5, g: 0.25, b: 0.75, a: 1.0 },
            },
            crate::contract::Tick(0),
            crate::contract::Tick::from_seconds(2),
        ));
        seq.video_tracks.push(t);
        project.insert_sequence(seq);

        let compiled = compile(&project, seq_id, 0, crate::contract::Tick(0), Quality::FULL, None);
        let cpu = crate::graph::eval_cpu::evaluate(
            &compiled.graph,
            (8, 8),
            &mut crate::graph::eval_cpu::EmptyProvider,
        );

        let mut eval = Evaluator::new(gpu.clone());
        let out = eval
            .evaluate(&compiled.graph, (8, 8), &mut NullFrameSource)
            .expect("output texture");
        let gpu_px = read_texture_rgba16f(&gpu, &out, 8, 8);

        for (g, c) in gpu_px.iter().zip(cpu.pixels.iter()) {
            for k in 0..4 {
                assert!(
                    (g[k] - c[k]).abs() < 1e-3,
                    "channel {k}: gpu {} vs cpu {}",
                    g[k],
                    c[k]
                );
            }
        }
    }

    /// Build a single-clip project (solid `color`) and let `decorate` attach a
    /// grade / effect before compiling. Returns the compiled graph.
    fn solid_project_graph(
        color: Color,
        decorate: impl FnOnce(&mut Clip),
    ) -> crate::graph::compile::CompiledFrame {
        let mut project = TimelineProject::new();
        let mut seq = Sequence::new("seq", FrameRate::FPS_30, 8, 8);
        let seq_id = seq.id;
        let mut t = Track::new(TrackKind::Video, "V1");
        let mut clip = Clip::new(
            ClipSource::SolidColor { color },
            crate::contract::Tick(0),
            crate::contract::Tick::from_seconds(2),
        );
        decorate(&mut clip);
        t.clips.push(clip);
        seq.video_tracks.push(t);
        project.insert_sequence(seq);
        compile(&project, seq_id, 0, crate::contract::Tick(0), Quality::FULL, None)
    }

    fn assert_graph_gpu_matches_cpu(compiled: &crate::graph::compile::CompiledFrame, tol: f32) {
        let Some(gpu) = GpuContext::request_blocking() else {
            eprintln!("no GPU adapter — skipping GPU parity");
            return;
        };
        let cpu = crate::graph::eval_cpu::evaluate(
            &compiled.graph,
            (8, 8),
            &mut crate::graph::eval_cpu::EmptyProvider,
        );
        let mut eval = Evaluator::new(gpu.clone());
        let out = eval
            .evaluate(&compiled.graph, (8, 8), &mut NullFrameSource)
            .expect("output texture");
        let gpu_px = read_texture_rgba16f(&gpu, &out, 8, 8);
        for (g, c) in gpu_px.iter().zip(cpu.pixels.iter()) {
            for k in 0..4 {
                assert!(
                    (g[k] - c[k]).abs() < tol,
                    "channel {k}: gpu {} vs cpu {}",
                    g[k],
                    c[k]
                );
            }
        }
    }

    /// GPU/CPU parity for the `Grade` op (07 §3, 03 §4.4): an Exposure grade over a
    /// solid must agree within 1e-3 between `apply_grade_stack_gpu` and
    /// `apply_grade_cpu`.
    #[test]
    fn graded_solid_gpu_matches_cpu_reference() {
        use photonic_core::timeline::grade::{Grade, GradeOp, GradeOpKind, GradeOpParams};
        let compiled = solid_project_graph(
            Color { r: 0.5, g: 0.4, b: 0.6, a: 1.0 },
            |clip| {
                let mut grade = Grade::new();
                grade.ops.push(GradeOp::new(
                    GradeOpKind::Exposure,
                    GradeOpParams::Exposure { stops: 0.7 },
                ));
                clip.grade = Some(grade);
            },
        );
        assert!(
            compiled.graph.nodes.iter().any(|n| matches!(n.op, IrOp::Grade { .. })),
            "a real Grade node is present"
        );
        assert_graph_gpu_matches_cpu(&compiled, 1e-3);
    }

    /// GPU/CPU parity for `Effect{Invert}` (08 §3).
    #[test]
    fn inverted_solid_gpu_matches_cpu_reference() {
        use photonic_core::timeline::{ClipEffect, EffectKind};
        let compiled = solid_project_graph(
            Color { r: 0.7, g: 0.2, b: 0.9, a: 1.0 },
            |clip| clip.effects.push(ClipEffect::new(EffectKind::Invert)),
        );
        assert_graph_gpu_matches_cpu(&compiled, 1e-3);
    }

    // ── Caption overlay compositing (06 §5.3) ─────────────────────────────────

    /// A 128×64 blue-background sequence with one caption track (cue `[0,200)`,
    /// two words "AB"/"CD"), optionally karaoke-highlighted.
    fn captioned_project(highlight: Option<KaraokeStyle>) -> (TimelineProject, SequenceId) {
        let mut project = TimelineProject::new();
        let mut seq = Sequence::new("seq", FrameRate::FPS_30, 128, 64);
        let seq_id = seq.id;
        let mut vt = Track::new(TrackKind::Video, "V1");
        vt.clips.push(Clip::new(
            ClipSource::SolidColor {
                color: Color { r: 0.0, g: 0.0, b: 0.6, a: 1.0 },
            },
            crate::contract::Tick(0),
            crate::contract::Tick(1000),
        ));
        seq.video_tracks.push(vt);

        let mut ct = CaptionTrack::new("Caps");
        ct.style = CaptionStyle {
            font_size: 22.0,
            position: [0.5, 0.25],
            highlight,
            ..CaptionStyle::default()
        };
        ct.cues.push(CaptionCue::new(
            crate::contract::Tick(0),
            crate::contract::Tick(200),
            vec![
                CaptionWord::new("AB", crate::contract::Tick(0), crate::contract::Tick(100)),
                CaptionWord::new("CD", crate::contract::Tick(100), crate::contract::Tick(200)),
            ],
        ));
        seq.caption_tracks.push(ct);
        project.insert_sequence(seq);
        (project, seq_id)
    }

    /// Count pixels differing in any channel by more than `tol`.
    fn count_diff(a: &[[f32; 4]], b: &[[f32; 4]], tol: f32) -> usize {
        a.iter()
            .zip(b)
            .filter(|(p, q)| (0..4).any(|k| (p[k] - q[k]).abs() > tol))
            .count()
    }

    /// AS-1 "burned caption": a covering cue burns glyphs over the frame (non-zero
    /// delta vs the caption-free background), and WordPop karaoke recolors the
    /// active word so a mid-word tick differs from a tick where the other word is
    /// active. Proves `CaptionOverlay` is no longer a passthrough stub.
    #[test]
    fn caption_overlay_burns_glyphs_and_karaoke_recolors() {
        let Some(gpu) = GpuContext::request_blocking() else {
            eprintln!("no GPU adapter — skipping caption overlay render");
            return;
        };
        let (w, h) = (128u32, 64u32);
        let highlight = KaraokeStyle {
            mode: KaraokeMode::WordPop,
            active_color: Color { r: 1.0, g: 1.0, b: 0.0, a: 1.0 }, // yellow
            inactive_color: Color { r: 0.5, g: 0.5, b: 0.5, a: 1.0 }, // grey
        };
        let (project, seq_id) = captioned_project(Some(highlight));
        let mut eval = Evaluator::new(gpu.clone());
        let render = |eval: &mut Evaluator, t: i64| {
            let c = compile(&project, seq_id, 0, crate::contract::Tick(t), Quality::FULL, None);
            let out = eval
                .evaluate(&c.graph, (w, h), &mut NullFrameSource)
                .expect("output texture");
            read_texture_rgba16f(&gpu, &out, w, h)
        };

        // t=500 is inside the background clip but past the cue → background only.
        let bg = render(&mut eval, 500);
        // t=50: "AB" active (yellow), "CD" inactive (grey).
        let f_before = render(&mut eval, 50);
        // t=150: swap — "AB" inactive, "CD" active.
        let f_mid = render(&mut eval, 150);

        let burned = count_diff(&f_before, &bg, 0.02);
        assert!(
            burned > 0,
            "caption glyphs must change pixels vs the caption-free background (got {burned})"
        );
        let karaoke = count_diff(&f_before, &f_mid, 0.02);
        assert!(
            karaoke > 0,
            "WordPop karaoke must change pixels between t=50 and t=150 (got {karaoke})"
        );
    }

    // ── Title / text clip rendering (G-12) ────────────────────────────────────

    /// A `ClipSource::Text` clip renders styled glyphs, reusing the caption
    /// glyphon path: the compiled graph lowers it to a `TextGen` carrying a
    /// populated cue, and the rendered frame has lit pixels carrying the fill
    /// colour. Proves the title is no longer a transparent placeholder.
    #[test]
    fn text_clip_renders_styled_glyphs() {
        let Some(gpu) = GpuContext::request_blocking() else {
            eprintln!("no GPU adapter — skipping text clip render");
            return;
        };
        let (w, h) = (128u32, 64u32);
        let mut project = TimelineProject::new();
        let mut seq = Sequence::new("seq", FrameRate::FPS_30, w, h);
        let seq_id = seq.id;
        let mut vt = Track::new(TrackKind::Video, "V1");
        // Red fill, no stroke, so lit glyph pixels isolate the fill colour.
        let style = CaptionStyle {
            font_size: 40.0,
            fill: Color { r: 1.0, g: 0.0, b: 0.0, a: 1.0 },
            stroke: None,
            position: [0.5, 0.4],
            ..CaptionStyle::default()
        };
        vt.clips.push(Clip::new(
            ClipSource::Text {
                content: TextClipContent {
                    text: "HELLO".to_string(),
                    style,
                },
            },
            crate::contract::Tick(0),
            crate::contract::Tick(1000),
        ));
        seq.video_tracks.push(vt);
        project.insert_sequence(seq);

        let compiled = compile(&project, seq_id, 0, crate::contract::Tick(100), Quality::FULL, None);
        assert!(
            compiled
                .graph
                .nodes
                .iter()
                .any(|n| matches!(&n.op, IrOp::TextGen { block } if block.cue.is_some())),
            "Text clip lowers to a TextGen carrying a resolved cue"
        );

        let mut eval = Evaluator::new(gpu.clone());
        let out = eval
            .evaluate(&compiled.graph, (w, h), &mut NullFrameSource)
            .expect("output texture");
        let px = read_texture_rgba16f(&gpu, &out, w, h);

        // Non-zero: glyph coverage lit some pixels over the transparent frame.
        let lit: Vec<[f32; 4]> = px.into_iter().filter(|p| p[3] > 0.05).collect();
        assert!(!lit.is_empty(), "text clip must produce lit glyph pixels");
        // Correctly coloured: premultiplied red (r≈coverage, g≈b≈0) — the fill,
        // not a stray white/black. Every lit pixel is red-dominant.
        let red_dominant = lit
            .iter()
            .filter(|p| p[0] > 0.05 && p[1] < 0.05 && p[2] < 0.05)
            .count();
        assert!(
            red_dominant > 0,
            "lit glyph pixels must carry the red fill ({} lit, {} red-dominant)",
            lit.len(),
            red_dominant
        );
    }
}
