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
//! - `Merge` — a premultiplied `over` composite honouring all 26 blend modes
//!   (K-0.3a / 03 §2.4), the WGSL twin of `graph::ops::merge_pixel` + the CPU
//!   `blend_rgb` reference. `Normal` keeps the exact fast-path expression so the
//!   golden corpus does not shift; every other mode unpremultiplies, blends, and
//!   re-composites per the W3C model.
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

use photonic_core::layer::BlendMode;
use photonic_core::timeline::EffectKind;
use wgpu::util::DeviceExt;

use crate::contract::{AssetId, Tick, VectorRef, VectorStateKey};
use crate::graph::cache::{CacheStats, NodeCache};
use crate::graph::ir::{FrameGraph, IrOp, TextureDesc, WipeDirection};
use crate::pool::DEFAULT_BUDGET_BYTES;

const WORKING_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// Uniform for the 26-mode merge pass: the blend-mode id and the layer opacity.
/// Same field layout as [`photonic_render::pipeline::CompositeParams`] so the two
/// composite paths stay in lockstep.
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct MergeParams {
    mode: u32,
    opacity: f32,
    _pad: [f32; 2],
}

/// Stable numeric id for a blend mode, fed to the merge shader's `switch`.
///
/// This mirrors `photonic_render::pipeline::blend_mode_index` exactly (declaration
/// order, HSL modes ≥ 12, Photoshop extras 16..=25). The canonical mapping is
/// `pub(crate)` in `photonic-render`; once it is promoted to `pub` and re-exported
/// this local copy should be replaced by that re-export rather than kept forked.
/// The exhaustive `match` (no wildcard) makes any future `BlendMode` variant a
/// compile error here until it is given an id, so the fork cannot silently drift.
fn merge_mode_index(mode: BlendMode) -> u32 {
    match mode {
        BlendMode::Normal => 0,
        BlendMode::Multiply => 1,
        BlendMode::Screen => 2,
        BlendMode::Overlay => 3,
        BlendMode::Darken => 4,
        BlendMode::Lighten => 5,
        BlendMode::ColorDodge => 6,
        BlendMode::ColorBurn => 7,
        BlendMode::HardLight => 8,
        BlendMode::SoftLight => 9,
        BlendMode::Difference => 10,
        BlendMode::Exclusion => 11,
        BlendMode::Hue => 12,
        BlendMode::Saturation => 13,
        BlendMode::Color => 14,
        BlendMode::Luminosity => 15,
        BlendMode::LinearDodge => 16,
        BlendMode::LinearBurn => 17,
        BlendMode::Subtract => 18,
        BlendMode::Divide => 19,
        BlendMode::VividLight => 20,
        BlendMode::LinearLight => 21,
        BlendMode::PinLight => 22,
        BlendMode::HardMix => 23,
        BlendMode::DarkerColor => 24,
        BlendMode::LighterColor => 25,
    }
}

/// The merge fragment shader body (26 blend modes). Working textures are
/// `Rgba16Float`, **premultiplied**, linear — so unlike `COMPOSITE_SHADER` (which
/// samples straight-alpha sRGB) this unpremultiplies before blending and keeps the
/// premultiplied source-over math of `graph::ops::merge_pixel` byte-for-byte:
/// `Normal` uses the exact fast-path expression (golden-corpus invariant); every
/// other mode blends `B(cb, cs)` then `Cs' = (1-αb)·Cs + αb·B`, and composites
/// `out = αs·Cs' + (1-αs)·bot`. The `blend_channel`/HSL helpers are the WGSL twins
/// of `photonic_core::raster::blend` (and identical to `COMPOSITE_SHADER`'s).
const MERGE_FS: &str = r#"
@group(0) @binding(0) var t_top: texture_2d<f32>;
@group(0) @binding(1) var t_bot: texture_2d<f32>;
@group(0) @binding(2) var s: sampler;
struct M { mode: u32, opacity: f32, pad: vec2<f32> }
@group(0) @binding(3) var<uniform> m: M;

fn screen1(cb: f32, cs: f32) -> f32 { return cb + cs - cb * cs; }
fn hard_light1(cb: f32, cs: f32) -> f32 {
    if (cs <= 0.5) { return cb * (2.0 * cs); }
    return screen1(cb, 2.0 * cs - 1.0);
}
fn soft_light1(cb: f32, cs: f32) -> f32 {
    if (cs <= 0.5) { return cb - (1.0 - 2.0 * cs) * cb * (1.0 - cb); }
    var d: f32;
    if (cb <= 0.25) { d = ((16.0 * cb - 12.0) * cb + 4.0) * cb; } else { d = sqrt(cb); }
    return cb + (2.0 * cs - 1.0) * (d - cb);
}
fn color_dodge1(cb: f32, cs: f32) -> f32 {
    if (cb == 0.0) { return 0.0; }
    if (cs >= 1.0) { return 1.0; }
    return min(cb / (1.0 - cs), 1.0);
}
fn color_burn1(cb: f32, cs: f32) -> f32 {
    if (cb >= 1.0) { return 1.0; }
    if (cs <= 0.0) { return 0.0; }
    return 1.0 - min((1.0 - cb) / cs, 1.0);
}
fn vivid_light1(cb: f32, cs: f32) -> f32 {
    if (cs <= 0.5) { return color_burn1(cb, min(2.0 * cs, 1.0)); }
    return color_dodge1(cb, min(2.0 * (cs - 0.5), 1.0));
}
fn blend_channel(mode: u32, cb: f32, cs: f32) -> f32 {
    switch (mode) {
        case 1u:  { return cb * cs; }
        case 2u:  { return screen1(cb, cs); }
        case 3u:  { return hard_light1(cs, cb); }
        case 4u:  { return min(cb, cs); }
        case 5u:  { return max(cb, cs); }
        case 6u:  { return color_dodge1(cb, cs); }
        case 7u:  { return color_burn1(cb, cs); }
        case 8u:  { return hard_light1(cb, cs); }
        case 9u:  { return soft_light1(cb, cs); }
        case 10u: { return abs(cb - cs); }
        case 11u: { return cb + cs - 2.0 * cb * cs; }
        case 16u: { return min(cb + cs, 1.0); }
        case 17u: { return max(cb + cs - 1.0, 0.0); }
        case 18u: { return max(cb - cs, 0.0); }
        case 19u: { if (cs <= 0.0) { return 1.0; } return min(cb / cs, 1.0); }
        case 20u: { return vivid_light1(cb, cs); }
        case 21u: { return clamp(cb + 2.0 * cs - 1.0, 0.0, 1.0); }
        case 22u: {
            if (cs <= 0.5) { return min(cb, 2.0 * cs); }
            return max(cb, 2.0 * cs - 1.0);
        }
        case 23u: {
            if (vivid_light1(cb, cs) < 0.5) { return 0.0; }
            return 1.0;
        }
        default:  { return cs; }
    }
}
fn lum(c: vec3<f32>) -> f32 { return 0.3 * c.r + 0.59 * c.g + 0.11 * c.b; }
fn clip_color(c: vec3<f32>) -> vec3<f32> {
    let l = lum(c);
    let n = min(min(c.r, c.g), c.b);
    let x = max(max(c.r, c.g), c.b);
    var o = c;
    if (n < 0.0) { o = vec3<f32>(l) + (o - vec3<f32>(l)) * (l / max(l - n, 1e-6)); }
    if (x > 1.0) { o = vec3<f32>(l) + (o - vec3<f32>(l)) * ((1.0 - l) / max(x - l, 1e-6)); }
    return o;
}
fn set_lum(c: vec3<f32>, l: f32) -> vec3<f32> {
    let d = l - lum(c);
    return clip_color(c + vec3<f32>(d));
}
fn sat(c: vec3<f32>) -> f32 { return max(max(c.r, c.g), c.b) - min(min(c.r, c.g), c.b); }
fn set_sat(c: vec3<f32>, sval: f32) -> vec3<f32> {
    let cmin = min(min(c.r, c.g), c.b);
    let cmax = max(max(c.r, c.g), c.b);
    let rng = cmax - cmin;
    if (rng <= 0.0) { return vec3<f32>(0.0); }
    return (c - vec3<f32>(cmin)) * (sval / rng);
}
@fragment
fn fs(i: VOut) -> @location(0) vec4<f32> {
    let top = textureSample(t_top, s, i.uv);
    let bot = textureSample(t_bot, s, i.uv);
    // Normal: exact fast-path expression, byte-for-byte the pre-K-0.3a shader so
    // the golden corpus does not shift.
    if (m.mode == 0u) {
        let t = top * m.opacity;
        return t + bot * (1.0 - t.a);
    }
    let a_s = top.a * m.opacity;
    let a_b = bot.a;
    var cs = vec3<f32>(0.0);
    if (top.a > 1e-6) { cs = top.rgb / top.a; }
    var cb = vec3<f32>(0.0);
    if (bot.a > 1e-6) { cb = bot.rgb / bot.a; }
    let mode = m.mode;
    var blended: vec3<f32>;
    if (mode == 12u)      { blended = set_lum(set_sat(cs, sat(cb)), lum(cb)); }
    else if (mode == 13u) { blended = set_lum(set_sat(cb, sat(cs)), lum(cb)); }
    else if (mode == 14u) { blended = set_lum(cs, lum(cb)); }
    else if (mode == 15u) { blended = set_lum(cb, lum(cs)); }
    else if (mode == 24u) { if (lum(cb) <= lum(cs)) { blended = cb; } else { blended = cs; } }
    else if (mode == 25u) { if (lum(cb) >= lum(cs)) { blended = cb; } else { blended = cs; } }
    else {
        blended = vec3<f32>(
            blend_channel(mode, cb.r, cs.r),
            blend_channel(mode, cb.g, cs.g),
            blend_channel(mode, cb.b, cs.b),
        );
    }
    let cs_prime = (1.0 - a_b) * cs + a_b * blended;
    let out_rgb = a_s * cs_prime + (1.0 - a_s) * bot.rgb;
    let out_a = a_s + a_b * (1.0 - a_s);
    return vec4<f32>(out_rgb, out_a);
}
"#;

/// `WipeMix` fragment shader (08 §2.0b) — the WGSL twin of `graph::ops::wipe`.
/// Reveal (`1` = incoming, `0` = outgoing) is a `smoothstep` edge at the eased
/// factor's remapped position `edge`, with a `hw` half-width band. The edge
/// position is derived from `@builtin(position)` and the LOGICAL canvas dims
/// (`dims.xy`) — matching the CPU pixel-center sweep coord — while the two layers
/// are sampled at the quad uv (same-pixel, like `MERGE_FS`). Premultiplied linear
/// is closed under lerp, so the blend is a plain `mix`.
const WIPE_FS: &str = r#"
@group(0) @binding(0) var t_in: texture_2d<f32>;
@group(0) @binding(1) var t_out: texture_2d<f32>;
@group(0) @binding(2) var s: sampler;
struct W { params: vec4<f32>, dims: vec4<f32> }
@group(0) @binding(3) var<uniform> u: W;
@fragment fn fs(i: VOut) -> @location(0) vec4<f32> {
  let xn = i.pos.x / u.dims.x;
  let yn = i.pos.y / u.dims.y;
  let dir = i32(u.params.x + 0.5);
  var p = xn;
  if (dir == 1) { p = 1.0 - xn; }
  else if (dir == 2) { p = yn; }
  else if (dir == 3) { p = 1.0 - yn; }
  let edge = u.params.y;
  let hw = u.params.z;
  let reveal = 1.0 - smoothstep(edge - hw, edge + hw, p);
  let inc = textureSample(t_in, s, i.uv);
  let outg = textureSample(t_out, s, i.uv);
  return mix(outg, inc, reveal);
}
"#;

/// `PushMix` fragment shader (08 §2.0b) — the WGSL twin of `graph::ops::push`.
/// Both layers translate along the direction axis by `t`; each output pixel picks
/// the incoming or outgoing layer and bilinear-samples it at the translated
/// pixel-center via `textureLoad` clamped to the source's LOGICAL dims (`dims.zw`)
/// — byte-for-byte the transform pass's edge-clamp/pixel-center semantics.
const PUSH_FS: &str = r#"
@group(0) @binding(0) var t_in: texture_2d<f32>;
@group(0) @binding(1) var t_out: texture_2d<f32>;
struct P { params: vec4<f32>, dims: vec4<f32> }
@group(0) @binding(2) var<uniform> u: P;
fn at_in(p: vec2<i32>) -> vec4<f32> {
  let hi = vec2<i32>(u.dims.zw) - vec2<i32>(1);
  return textureLoad(t_in, clamp(p, vec2<i32>(0), hi), 0);
}
fn at_out(p: vec2<i32>) -> vec4<f32> {
  let hi = vec2<i32>(u.dims.zw) - vec2<i32>(1);
  return textureLoad(t_out, clamp(p, vec2<i32>(0), hi), 0);
}
fn bilin_in(pf: vec2<f32>) -> vec4<f32> {
  let q = pf - vec2<f32>(0.5);
  let p0f = floor(q); let p0 = vec2<i32>(p0f); let f = q - p0f;
  return mix(mix(at_in(p0), at_in(p0 + vec2<i32>(1, 0)), f.x),
             mix(at_in(p0 + vec2<i32>(0, 1)), at_in(p0 + vec2<i32>(1, 1)), f.x), f.y);
}
fn bilin_out(pf: vec2<f32>) -> vec4<f32> {
  let q = pf - vec2<f32>(0.5);
  let p0f = floor(q); let p0 = vec2<i32>(p0f); let f = q - p0f;
  return mix(mix(at_out(p0), at_out(p0 + vec2<i32>(1, 0)), f.x),
             mix(at_out(p0 + vec2<i32>(0, 1)), at_out(p0 + vec2<i32>(1, 1)), f.x), f.y);
}
@fragment fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
  if (pos.x >= u.dims.x || pos.y >= u.dims.y) { return vec4<f32>(0.0); }
  let dir = i32(u.params.x + 0.5);
  let t = u.params.y;
  let horizontal = (dir == 0 || dir == 1);
  let forward = (dir == 0 || dir == 2);
  var uc: f32; var npx: f32;
  if (horizontal) { uc = pos.x / u.dims.x; npx = u.dims.x; }
  else { uc = pos.y / u.dims.y; npx = u.dims.y; }
  var uu = uc;
  if (!forward) { uu = 1.0 - uc; }
  var use_in = false; var src_u = 0.0;
  if (uu >= t) { use_in = false; src_u = uu - t; }
  else { use_in = true; src_u = uu - t + 1.0; }
  var along = src_u;
  if (!forward) { along = 1.0 - src_u; }
  var pf: vec2<f32>;
  if (horizontal) { pf = vec2<f32>(along * npx, pos.y); }
  else { pf = vec2<f32>(pos.x, along * npx); }
  if (use_in) { return bilin_in(pf); }
  return bilin_out(pf);
}
"#;

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
    fn vector_texture(
        &mut self,
        _: &GpuContext,
        _: VectorRef,
        _: VectorStateKey,
        _: u32,
        _: u32,
    ) -> Option<GpuFrame> {
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
        let mut results: Vec<Option<GpuFrame>> = (0..graph.nodes.len()).map(|_| None).collect();

        for (i, node) in graph.nodes.iter().enumerate() {
            let inputs: Vec<GpuFrame> = node
                .inputs
                .iter()
                .filter_map(|(id, _)| results[id.0 as usize].clone())
                .collect();

            let out = match &node.op {
                IrOp::DecodeVideo {
                    asset,
                    src_time,
                    proxy,
                } => match source.video_texture(&self.gpu, *asset, *src_time, *proxy) {
                    Some(frame) => self.normalize_source_cached(node.content_hash, frame, cw, ch),
                    None => self.transparent(cw, ch),
                },
                IrOp::DecodeStill { asset } => match source.still_texture(&self.gpu, *asset) {
                    Some(frame) => self.normalize_source_cached(node.content_hash, frame, cw, ch),
                    None => self.transparent(cw, ch),
                },
                IrOp::RasterVector {
                    vref,
                    doc_state,
                    w,
                    h,
                } => match source.vector_texture(&self.gpu, *vref, *doc_state, *w, *h) {
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
        let desc = TextureDesc {
            width: w,
            height: h,
        };
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
            IrOp::Merge { mode, opacity } => match (inputs.first(), inputs.get(1)) {
                (Some(top), Some(bottom)) => {
                    self.passes.merge(
                        &self.gpu,
                        &top.texture,
                        &bottom.texture,
                        *mode,
                        *opacity,
                        target,
                    );
                }
                (Some(only), None) | (None, Some(only)) => {
                    self.passes.blit(&self.gpu, &only.texture, target);
                }
                (None, None) => self.passes.fill(&self.gpu, target, [0.0; 4]),
            },
            // Directional transitions (08 §2.0b): inputs [incoming, outgoing], the
            // WGSL twins of `ops::wipe` / `ops::push`.
            IrOp::WipeMix {
                direction,
                softness,
                t,
            } => match (inputs.first(), inputs.get(1)) {
                (Some(incoming), Some(outgoing)) => self.passes.wipe(
                    &self.gpu,
                    &incoming.texture,
                    &outgoing.texture,
                    *direction,
                    *softness,
                    *t,
                    target,
                    logical_w,
                    logical_h,
                ),
                (Some(only), None) | (None, Some(only)) => {
                    self.passes.blit(&self.gpu, &only.texture, target)
                }
                (None, None) => self.passes.fill(&self.gpu, target, [0.0; 4]),
            },
            IrOp::PushMix { direction, t } => match (inputs.first(), inputs.get(1)) {
                (Some(incoming), Some(outgoing)) => self.passes.push(
                    &self.gpu,
                    &incoming.texture,
                    &outgoing.texture,
                    *direction,
                    *t,
                    target,
                    logical_w,
                    logical_h,
                    incoming.width,
                    incoming.height,
                ),
                (Some(only), None) | (None, Some(only)) => {
                    self.passes.blit(&self.gpu, &only.texture, target)
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
            // Real effect kernels for all seven v1 kinds (08 §3 / K-0.2), the
            // WGSL twins of the `ops::*` kernels the CPU reference runs.
            // Unknown/forward-compat kinds fall through to the blit passthrough.
            IrOp::Effect {
                kind: EffectKind::Invert,
                ..
            } => match inputs.first() {
                Some(src) => self.passes.invert(&self.gpu, &src.texture, target),
                None => self.passes.fill(&self.gpu, target, [0.0; 4]),
            },
            IrOp::Effect {
                kind: EffectKind::Blur,
                params,
            } => match inputs.first() {
                Some(src) => self.passes.gaussian_blur(
                    &self.gpu,
                    &src.texture,
                    target,
                    params.f32_or("params.radius", 0.0),
                    logical_w,
                    logical_h,
                ),
                None => self.passes.fill(&self.gpu, target, [0.0; 4]),
            },
            IrOp::Effect {
                kind: EffectKind::Sharpen,
                params,
            } => match inputs.first() {
                Some(src) => self.passes.sharpen(
                    &self.gpu,
                    &src.texture,
                    target,
                    params.f32_or("params.amount", 0.0),
                    params.f32_or("params.radius", 0.0),
                    logical_w,
                    logical_h,
                ),
                None => self.passes.fill(&self.gpu, target, [0.0; 4]),
            },
            IrOp::Effect {
                kind: EffectKind::Glow,
                params,
            } => match inputs.first() {
                Some(src) => {
                    let tint = params.color_or("params.tint", photonic_core::Color::WHITE);
                    self.passes.glow(
                        &self.gpu,
                        &src.texture,
                        target,
                        params.f32_or("params.radius", 0.0),
                        params.f32_or("params.threshold", 0.0),
                        params.f32_or("params.intensity", 0.0),
                        [
                            crate::graph::ops::srgb_to_linear(tint.r),
                            crate::graph::ops::srgb_to_linear(tint.g),
                            crate::graph::ops::srgb_to_linear(tint.b),
                        ],
                        logical_w,
                        logical_h,
                    );
                }
                None => self.passes.fill(&self.gpu, target, [0.0; 4]),
            },
            IrOp::Effect {
                kind: EffectKind::LumaKey,
                params,
            } => match inputs.first() {
                Some(src) => self.passes.luma_key(
                    &self.gpu,
                    &src.texture,
                    target,
                    params.f32_or("params.threshold", 0.0),
                    params.f32_or("params.softness", 0.0),
                    params.bool_or("params.invert", false),
                ),
                None => self.passes.fill(&self.gpu, target, [0.0; 4]),
            },
            IrOp::Effect {
                kind: EffectKind::ChromaKey,
                params,
            } => match inputs.first() {
                Some(src) => {
                    let key = params.color_or("params.key_color", photonic_core::Color::BLACK);
                    self.passes.chroma_key(
                        &self.gpu,
                        &src.texture,
                        target,
                        [
                            crate::graph::ops::srgb_to_linear(key.r),
                            crate::graph::ops::srgb_to_linear(key.g),
                            crate::graph::ops::srgb_to_linear(key.b),
                        ],
                        params.f32_or("params.tolerance", 0.0),
                        params.f32_or("params.edge_softness", 0.0),
                        params.f32_or("params.spill_suppress", 0.0),
                    );
                }
                None => self.passes.fill(&self.gpu, target, [0.0; 4]),
            },
            // K-B16: bridged raster ids lower as Unknown(tag). GPU reuses the
            // existing separable blur/sharpen passes for the two that map cleanly;
            // other bridge ids still blit (CPU is the oracle until WGSL ports).
            IrOp::Effect {
                kind: EffectKind::Unknown(tag),
                params,
            } => match (tag.as_str(), inputs.first()) {
                ("blur.box", Some(src)) => self.passes.gaussian_blur(
                    &self.gpu,
                    &src.texture,
                    target,
                    params.f32_or("params.radius", 1.0),
                    logical_w,
                    logical_h,
                ),
                ("sharpen.unsharp_raster", Some(src)) => self.passes.sharpen(
                    &self.gpu,
                    &src.texture,
                    target,
                    params.f32_or("params.amount", 1.0),
                    params.f32_or("params.radius", 1.0),
                    logical_w,
                    logical_h,
                ),
                ("filter.high_pass", Some(src)) => {
                    // Approximate high-pass as source − blur (linear premult).
                    let blurred = self.passes.temp_texture(&self.gpu, target);
                    self.passes.gaussian_blur(
                        &self.gpu,
                        &src.texture,
                        &blurred,
                        params.f32_or("params.radius", 2.0),
                        logical_w,
                        logical_h,
                    );
                    self.gpu.device().poll(wgpu::Maintain::Wait);
                    self.passes.high_pass_combine(
                        &self.gpu,
                        &src.texture,
                        &blurred,
                        target,
                    );
                }
                (_, Some(src)) => self.passes.blit(&self.gpu, &src.texture, target),
                (_, None) => self.passes.fill(&self.gpu, target, [0.0; 4]),
            },
            IrOp::Effect {
                kind: EffectKind::MaskShapeGen,
                params,
            } => self.passes.mask_shape(
                &self.gpu,
                target,
                [
                    params.f32_or("params.center_x", 0.5),
                    params.f32_or("params.center_y", 0.5),
                ],
                [
                    params.f32_or("params.size_x", 0.5),
                    params.f32_or("params.size_y", 0.5),
                ],
                params.f32_or("params.rotation", 0.0),
                params.f32_or("params.feather", 0.0),
                logical_w,
                logical_h,
            ),
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
        let hash =
            crate::graph::ir::ContentHash((0xFE_u128 << 120) | ((w as u128) << 32) | h as u128);
        let (tex, valid) = self.cache.lookup_or_alloc(
            hash,
            TextureDesc {
                width: w,
                height: h,
            },
        );
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
    /// `Effect{LumaKey}`/`Effect{ChromaKey}` (08 §3): a filter BGL (tex + sampler
    /// + a params uniform), the WGSL twins of `ops::luma_key`/`ops::chroma_key`.
    filter_bgl: wgpu::BindGroupLayout,
    luma_key_pipeline: wgpu::RenderPipeline,
    chroma_key_pipeline: wgpu::RenderPipeline,
    /// Separable Gaussian blur (K-0.2): tex + sampler + `BlurParams` uniform.
    /// Shared by `Effect{Blur}` and the blur half of Sharpen/Glow. Algorithm
    /// matches `photonic_render::pipeline::BLUR_SHADER` / `ops::blur`.
    blur_bgl: wgpu::BindGroupLayout,
    blur_pipeline: wgpu::RenderPipeline,
    /// Unsharp combine (K-0.2): two textures (src, blurred) + amount uniform.
    sharpen_bgl: wgpu::BindGroupLayout,
    sharpen_pipeline: wgpu::RenderPipeline,
    /// High-pass combine (K-B16): `src - blurred` on RGB (reuses sharpen BGL).
    high_pass_pipeline: wgpu::RenderPipeline,
    /// Glow extract (K-0.2): keep pixels whose straight luma ≥ threshold.
    glow_extract_pipeline: wgpu::RenderPipeline,
    /// Glow composite (K-0.2): screen-add tinted glow over source.
    glow_comp_bgl: wgpu::BindGroupLayout,
    glow_comp_pipeline: wgpu::RenderPipeline,
    /// `Effect{MaskShapeGen}` (08 §3): a 0-input generator, uniform-only BGL.
    mask_bgl: wgpu::BindGroupLayout,
    mask_shape_pipeline: wgpu::RenderPipeline,
    merge_pipeline: wgpu::RenderPipeline,
    merge_bgl: wgpu::BindGroupLayout,
    /// `WipeMix` (08 §2.0b): binary directional smoothstep wipe. Reuses the merge
    /// BGL (two textures + sampler + a params uniform). WGSL twin of `ops::wipe`.
    wipe_pipeline: wgpu::RenderPipeline,
    /// `PushMix` (08 §2.0b): binary directional slide. Its own BGL (two textures +
    /// a params uniform; textureLoad, no sampler). WGSL twin of `ops::push`.
    push_pipeline: wgpu::RenderPipeline,
    push_bgl: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

/// Stable numeric id for a wipe/push direction, fed to the wipe/push shaders (and
/// matching the `ops::sweep_coord` / `ops::push` axis+orientation branch order).
fn wipe_direction_index(dir: WipeDirection) -> f32 {
    match dir {
        WipeDirection::LeftToRight => 0.0,
        WipeDirection::RightToLeft => 1.0,
        WipeDirection::TopToBottom => 2.0,
        WipeDirection::BottomToTop => 3.0,
    }
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

        // Filter BGL (tex + sampler + params uniform) — shared by LumaKey/ChromaKey.
        let filter_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("filter_bgl"),
            entries: &[tex_entry(0), sampler_entry(1), uniform_entry(2)],
        });

        // LumaKey (08 §3): scale α by a luma-driven keep factor. `u = [threshold,
        // hi, invert, pad]` (hi = threshold + max(softness, eps), floored in Rust so
        // the GPU and CPU smoothstep bands match). WGSL twin of `ops::luma_key`.
        let luma_key_src = format!(
            "{QUAD_VS}\n@group(0) @binding(0) var t: texture_2d<f32>;\n@group(0) @binding(1) var s: sampler;\nstruct LK {{ p: vec4<f32> }}\n@group(0) @binding(2) var<uniform> u: LK;\n@fragment fn fs(i: VOut) -> @location(0) vec4<f32> {{\n  let c = textureSample(t, s, i.uv);\n  var straight = vec3<f32>(0.0);\n  if (c.a > 1e-6) {{ straight = c.rgb / c.a; }}\n  let luma = 0.2126 * straight.r + 0.7152 * straight.g + 0.0722 * straight.b;\n  var keep = smoothstep(u.p.x, u.p.y, luma);\n  if (u.p.z > 0.5) {{ keep = 1.0 - keep; }}\n  return c * keep;\n}}\n"
        );
        let luma_key_pipeline = make_pipeline(device, &filter_bgl, &luma_key_src, "fs");

        // ChromaKey (08 §3): key out colours near `key`, feather the matte edge,
        // suppress spill on the dominant key channel. `key = [r,g,b,tolerance]`,
        // `aux = [hi, spill, dom, pad]`. WGSL twin of `ops::chroma_key`.
        let chroma_key_src = format!(
            "{QUAD_VS}\n@group(0) @binding(0) var t: texture_2d<f32>;\n@group(0) @binding(1) var s: sampler;\nstruct CK {{ key: vec4<f32>, aux: vec4<f32> }}\n@group(0) @binding(2) var<uniform> u: CK;\n@fragment fn fs(i: VOut) -> @location(0) vec4<f32> {{\n  let c = textureSample(t, s, i.uv);\n  var straight = vec3<f32>(0.0);\n  if (c.a > 1e-6) {{ straight = c.rgb / c.a; }}\n  let dist = length(straight - u.key.xyz);\n  let keep = smoothstep(u.key.w, u.aux.x, dist);\n  var col = straight;\n  let spill = u.aux.y;\n  let dom = i32(u.aux.z + 0.5);\n  if (dom == 0) {{ let m = 0.5 * (col.g + col.b); if (col.r > m) {{ col.r = col.r + (m - col.r) * spill; }} }}\n  else if (dom == 1) {{ let m = 0.5 * (col.r + col.b); if (col.g > m) {{ col.g = col.g + (m - col.g) * spill; }} }}\n  else {{ let m = 0.5 * (col.r + col.g); if (col.b > m) {{ col.b = col.b + (m - col.b) * spill; }} }}\n  let a = c.a * keep;\n  return vec4<f32>(col * a, a);\n}}\n"
        );
        let chroma_key_pipeline = make_pipeline(device, &filter_bgl, &chroma_key_src, "fs");

        // Separable Gaussian blur (K-0.2) — WGSL twin of `ops::blur`. Uses
        // `textureLoad` + LOGICAL dims (not physical pool-bucket size) so the
        // kernel steps match the CPU's clamp-to-edge pixel sampling exactly.
        // Uniform: `info = [sigma, horizontal, logical_w, logical_h]`.
        let blur_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("blur_bgl"),
            entries: &[tex_entry(0), uniform_entry(1)],
        });
        let blur_src = format!(
            "{QUAD_VS}\n@group(0) @binding(0) var t: texture_2d<f32>;\nstruct BlurP {{ info: vec4<f32> }}\n@group(0) @binding(1) var<uniform> p: BlurP;\nfn at(px: i32, py: i32) -> vec4<f32> {{\n  let hi = vec2<i32>(i32(p.info.z), i32(p.info.w)) - vec2<i32>(1);\n  return textureLoad(t, clamp(vec2<i32>(px, py), vec2<i32>(0), hi), 0);\n}}\n@fragment fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {{\n  let lw = p.info.z; let lh = p.info.w;\n  if (pos.x >= lw || pos.y >= lh) {{ return vec4<f32>(0.0); }}\n  let sigma = p.info.x;\n  let x = i32(pos.x); let y = i32(pos.y);\n  if (sigma < 0.5) {{ return at(x, y); }}\n  let radius = min(i32(ceil(sigma * 3.0)), 128);\n  var acc = vec4<f32>(0.0);\n  var w_total = 0.0;\n  let horiz = p.info.y > 0.5;\n  for (var k = -radius; k <= radius; k++) {{\n    let fi = f32(k);\n    let w = exp(-fi * fi / (2.0 * sigma * sigma));\n    var sx = x; var sy = y;\n    if (horiz) {{ sx = x + k; }} else {{ sy = y + k; }}\n    acc += at(sx, sy) * w;\n    w_total += w;\n  }}\n  return acc / w_total;\n}}\n"
        );
        let blur_pipeline = make_pipeline(device, &blur_bgl, &blur_src, "fs");

        // Unsharp combine: `src + amount * (src - blurred)` (K-0.2).
        let sharpen_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sharpen_bgl"),
            entries: &[tex_entry(0), tex_entry(1), sampler_entry(2), uniform_entry(3)],
        });
        let sharpen_src = format!(
            "{QUAD_VS}\n@group(0) @binding(0) var t_src: texture_2d<f32>;\n@group(0) @binding(1) var t_blur: texture_2d<f32>;\n@group(0) @binding(2) var s: sampler;\nstruct S {{ amount: vec4<f32> }}\n@group(0) @binding(3) var<uniform> u: S;\n@fragment fn fs(i: VOut) -> @location(0) vec4<f32> {{\n  let src = textureSample(t_src, s, i.uv);\n  let bl = textureSample(t_blur, s, i.uv);\n  return src + u.amount.x * (src - bl);\n}}\n"
        );
        let sharpen_pipeline = make_pipeline(device, &sharpen_bgl, &sharpen_src, "fs");

        // High-pass: src − blurred on RGB, keep src alpha (K-B16).
        let high_pass_src = format!(
            "{QUAD_VS}\n@group(0) @binding(0) var t_src: texture_2d<f32>;\n@group(0) @binding(1) var t_blur: texture_2d<f32>;\n@group(0) @binding(2) var s: sampler;\nstruct S {{ amount: vec4<f32> }}\n@group(0) @binding(3) var<uniform> u: S;\n@fragment fn fs(i: VOut) -> @location(0) vec4<f32> {{\n  let src = textureSample(t_src, s, i.uv);\n  let bl = textureSample(t_blur, s, i.uv);\n  return vec4<f32>(src.rgb - bl.rgb, src.a);\n}}\n"
        );
        let high_pass_pipeline = make_pipeline(device, &sharpen_bgl, &high_pass_src, "fs");

        // Glow extract: zero out pixels whose straight luma is below threshold.
        // Reuses filter_bgl (tex + sampler + params).
        let glow_extract_src = format!(
            "{QUAD_VS}\n@group(0) @binding(0) var t: texture_2d<f32>;\n@group(0) @binding(1) var s: sampler;\nstruct G {{ p: vec4<f32> }}\n@group(0) @binding(2) var<uniform> u: G;\n@fragment fn fs(i: VOut) -> @location(0) vec4<f32> {{\n  let c = textureSample(t, s, i.uv);\n  var straight = vec3<f32>(0.0);\n  if (c.a > 1e-6) {{ straight = c.rgb / c.a; }}\n  let luma = 0.2126 * straight.r + 0.7152 * straight.g + 0.0722 * straight.b;\n  let keep = select(0.0, 1.0, luma >= u.p.x);\n  return c * keep;\n}}\n"
        );
        let glow_extract_pipeline = make_pipeline(device, &filter_bgl, &glow_extract_src, "fs");

        // Glow composite: screen-add tinted glow over source.
        let glow_comp_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("glow_comp_bgl"),
            entries: &[tex_entry(0), tex_entry(1), sampler_entry(2), uniform_entry(3)],
        });
        let glow_comp_src = format!(
            "{QUAD_VS}\n@group(0) @binding(0) var t_src: texture_2d<f32>;\n@group(0) @binding(1) var t_glow: texture_2d<f32>;\n@group(0) @binding(2) var s: sampler;\nstruct GC {{ tint: vec4<f32>, intensity: vec4<f32> }}\n@group(0) @binding(3) var<uniform> u: GC;\n@fragment fn fs(i: VOut) -> @location(0) vec4<f32> {{\n  let src = textureSample(t_src, s, i.uv);\n  let g = textureSample(t_glow, s, i.uv);\n  let gr = g.r * u.tint.x * u.intensity.x;\n  let gg = g.g * u.tint.y * u.intensity.x;\n  let gb = g.b * u.tint.z * u.intensity.x;\n  let ga = clamp(g.a * u.intensity.x, 0.0, 1.0);\n  return vec4<f32>(\n    src.r + gr * max(1.0 - src.r, 0.0),\n    src.g + gg * max(1.0 - src.g, 0.0),\n    src.b + gb * max(1.0 - src.b, 0.0),\n    src.a + ga * (1.0 - src.a)\n  );\n}}\n"
        );
        let glow_comp_pipeline = make_pipeline(device, &glow_comp_bgl, &glow_comp_src, "fs");

        // MaskShapeGen (08 §3): a uniform-only ellipse-matte generator (no input).
        // `center = [cx, cy, sx, sy]`, `aux = [cos(-rot), sin(-rot), inner, pad]`
        // (inner = 1 − max(feather, eps)). WGSL twin of `ops::mask_shape`; uv is
        // the top-left-origin canvas coordinate (matches the CPU pixel-center uv).
        let mask_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("mask_bgl"),
            entries: &[uniform_entry(0)],
        });
        // Working textures are pool-bucketed (dims rounded up to 64), so the
        // logical canvas occupies the top-left region. A generator must derive its
        // uv from `@builtin(position)` and the LOGICAL dims (`dims.xy`) — exactly
        // as the transform pass does — not from the quad's bucket-spanning uv, or
        // the ellipse would be placed against the 64px bucket, not the canvas.
        let mask_shape_src = format!(
            "{QUAD_VS}\nstruct MS {{ center: vec4<f32>, aux: vec4<f32>, dims: vec4<f32> }}\n@group(0) @binding(0) var<uniform> u: MS;\n@fragment fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {{\n  let uv = vec2<f32>(pos.x / u.dims.x, pos.y / u.dims.y);\n  let d = uv - u.center.xy;\n  let rd = vec2<f32>(d.x * u.aux.x - d.y * u.aux.y, d.x * u.aux.y + d.y * u.aux.x);\n  var ex = 1e18; var ey = 1e18;\n  if (abs(u.center.z) > 1e-6) {{ ex = rd.x / u.center.z; }}\n  if (abs(u.center.w) > 1e-6) {{ ey = rd.y / u.center.w; }}\n  let r = sqrt(ex * ex + ey * ey);\n  let a = 1.0 - smoothstep(u.aux.z, 1.0, r);\n  return vec4<f32>(a, a, a, a);\n}}\n"
        );
        let mask_shape_pipeline = make_pipeline(device, &mask_bgl, &mask_shape_src, "fs");

        // Merge: premultiplied `over`, all 26 blend modes (K-0.3a, 03 §2.4).
        let merge_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("merge_bgl"),
            entries: &[
                tex_entry(0),
                tex_entry(1),
                sampler_entry(2),
                uniform_entry(3),
            ],
        });
        let merge_src = format!("{QUAD_VS}\n{MERGE_FS}");
        let merge_pipeline = make_pipeline(device, &merge_bgl, &merge_src, "fs");

        // WipeMix (08 §2.0b): binary directional wipe, reusing the merge BGL
        // (two textures + sampler + a params uniform). WGSL twin of `ops::wipe`.
        let wipe_src = format!("{QUAD_VS}\n{WIPE_FS}");
        let wipe_pipeline = make_pipeline(device, &merge_bgl, &wipe_src, "fs");

        // PushMix (08 §2.0b): binary directional slide via textureLoad (no
        // sampler) — its own BGL (two textures + a params uniform). WGSL twin of
        // `ops::push`.
        let push_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("push_bgl"),
            entries: &[tex_entry(0), tex_entry(1), uniform_entry(2)],
        });
        let push_src = format!("{QUAD_VS}\n{PUSH_FS}");
        let push_pipeline = make_pipeline(device, &push_bgl, &push_src, "fs");

        Passes {
            fill_pipeline,
            fill_bgl,
            blit_pipeline,
            blit_bgl,
            transform_pipeline,
            transform_bgl,
            invert_pipeline,
            filter_bgl,
            luma_key_pipeline,
            chroma_key_pipeline,
            blur_bgl,
            blur_pipeline,
            sharpen_bgl,
            sharpen_pipeline,
            high_pass_pipeline,
            glow_extract_pipeline,
            glow_comp_bgl,
            glow_comp_pipeline,
            mask_bgl,
            mask_shape_pipeline,
            merge_pipeline,
            merge_bgl,
            wipe_pipeline,
            push_pipeline,
            push_bgl,
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
        let buffer = gpu
            .device()
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
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

    /// Scratch texture matching `like`'s size for multi-pass effects (blur H/V).
    fn temp_like(gpu: &GpuContext, like: &wgpu::Texture) -> wgpu::Texture {
        Self::alloc_temp(gpu, like)
    }

    fn alloc_temp(gpu: &GpuContext, like: &wgpu::Texture) -> wgpu::Texture {
        gpu.device().create_texture(&wgpu::TextureDescriptor {
            label: Some("eval_temp"),
            size: like.size(),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: WORKING_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    }

    fn temp_texture(&self, gpu: &GpuContext, like: &wgpu::Texture) -> wgpu::Texture {
        Self::alloc_temp(gpu, like)
    }

    /// High-pass combine: `src - blurred` on RGB, keep src alpha (K-B16).
    fn high_pass_combine(
        &self,
        gpu: &GpuContext,
        src: &wgpu::Texture,
        blurred: &wgpu::Texture,
        target: &wgpu::Texture,
    ) {
        let sv = src.create_view(&Default::default());
        let bv = blurred.create_view(&Default::default());
        let bind = gpu.device().create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("high_pass_bg"),
            layout: &self.sharpen_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&sv),
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
                    resource: gpu
                        .device()
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("high_pass_u"),
                            // amount = -1 ⇒ src + (-1)*(src-blur) wait — we want src-blur.
                            // sharpen is src + amount*(src-blur). amount=1 ⇒ 2src-blur.
                            // So use a dedicated small shader via amount=-0 and...
                            // Use amount=1 on (src, 2*blur-src)? Simpler: pass amount
                            // as a special: we'll use amount = -1 with inverted meaning
                            // in a one-off uniform: store 1.0 and use high_pass_pipeline.
                            contents: bytemuck::cast_slice(&[1.0f32, 0.0, 0.0, 0.0]),
                            usage: wgpu::BufferUsages::UNIFORM,
                        })
                        .as_entire_binding(),
                },
            ],
        });
        // Reuse sharpen BGL layout but high_pass_pipeline: out = src - blur.
        self.run(gpu, &self.high_pass_pipeline, &bind, target);
    }

    /// One axis of the separable Gaussian (`horizontal` = H pass).
    fn blur_axis(
        &self,
        gpu: &GpuContext,
        src: &wgpu::Texture,
        target: &wgpu::Texture,
        sigma: f32,
        horizontal: bool,
        logical_w: u32,
        logical_h: u32,
    ) {
        let uniform = [
            sigma,
            if horizontal { 1.0 } else { 0.0 },
            logical_w as f32,
            logical_h as f32,
        ];
        let view = src.create_view(&Default::default());
        let buf = gpu
            .device()
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("blur_params"),
                contents: bytemuck::cast_slice(&uniform),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let bind = gpu.device().create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("blur_bg"),
            layout: &self.blur_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: buf.as_entire_binding(),
                },
            ],
        });
        self.run(gpu, &self.blur_pipeline, &bind, target);
    }

    /// `Effect{Blur}` — dual-pass separable Gaussian (H then V). `sigma < 0.5`
    /// is a blit (matches `ops::blur` / the shader early-out). Each axis is a
    /// separate submit; we `poll(Wait)` between them so the V pass samples a
    /// finished H target (no cross-pass race on llvmpipe/soft adapters).
    fn gaussian_blur(
        &self,
        gpu: &GpuContext,
        src: &wgpu::Texture,
        target: &wgpu::Texture,
        sigma: f32,
        logical_w: u32,
        logical_h: u32,
    ) {
        let sigma = if sigma.is_finite() { sigma.max(0.0) } else { 0.0 };
        if sigma < 0.5 {
            self.blit(gpu, src, target);
            return;
        }
        let tmp = Self::temp_like(gpu, target);
        self.blur_axis(gpu, src, &tmp, sigma, true, logical_w, logical_h);
        gpu.device().poll(wgpu::Maintain::Wait);
        self.blur_axis(gpu, &tmp, target, sigma, false, logical_w, logical_h);
    }

    /// `Effect{Sharpen}` — unsharp mask via blur intermediate + combine pass.
    #[allow(clippy::too_many_arguments)]
    fn sharpen(
        &self,
        gpu: &GpuContext,
        src: &wgpu::Texture,
        target: &wgpu::Texture,
        amount: f32,
        radius: f32,
        logical_w: u32,
        logical_h: u32,
    ) {
        let amount = if amount.is_finite() { amount } else { 0.0 };
        if amount == 0.0 {
            self.blit(gpu, src, target);
            return;
        }
        let blurred = Self::temp_like(gpu, target);
        self.gaussian_blur(gpu, src, &blurred, radius, logical_w, logical_h);
        gpu.device().poll(wgpu::Maintain::Wait);
        let sv = src.create_view(&Default::default());
        let bv = blurred.create_view(&Default::default());
        let uniform = [amount, 0.0, 0.0, 0.0];
        let buf = gpu
            .device()
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("sharpen_uniform"),
                contents: bytemuck::cast_slice(&uniform),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let bind = gpu.device().create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sharpen_bg"),
            layout: &self.sharpen_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&sv),
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
        self.run(gpu, &self.sharpen_pipeline, &bind, target);
    }

    /// `Effect{Glow}` — extract brights → blur → tinted screen-add over source.
    #[allow(clippy::too_many_arguments)]
    fn glow(
        &self,
        gpu: &GpuContext,
        src: &wgpu::Texture,
        target: &wgpu::Texture,
        radius: f32,
        threshold: f32,
        intensity: f32,
        tint_linear: [f32; 3],
        logical_w: u32,
        logical_h: u32,
    ) {
        let intensity = if intensity.is_finite() {
            intensity.max(0.0)
        } else {
            0.0
        };
        if intensity == 0.0 {
            self.blit(gpu, src, target);
            return;
        }
        let threshold = if threshold.is_finite() {
            threshold.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let extract = Self::temp_like(gpu, target);
        let uniform = [threshold, 0.0, 0.0, 0.0];
        self.run_filter(gpu, &self.glow_extract_pipeline, src, &extract, &uniform);
        gpu.device().poll(wgpu::Maintain::Wait);
        let blurred = Self::temp_like(gpu, target);
        self.gaussian_blur(gpu, &extract, &blurred, radius, logical_w, logical_h);
        gpu.device().poll(wgpu::Maintain::Wait);
        let sv = src.create_view(&Default::default());
        let gv = blurred.create_view(&Default::default());
        let u = [
            tint_linear[0],
            tint_linear[1],
            tint_linear[2],
            0.0,
            intensity,
            0.0,
            0.0,
            0.0,
        ];
        let buf = gpu
            .device()
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("glow_comp_uniform"),
                contents: bytemuck::cast_slice(&u),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let bind = gpu.device().create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("glow_comp_bg"),
            layout: &self.glow_comp_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&sv),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&gv),
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
        self.run(gpu, &self.glow_comp_pipeline, &bind, target);
    }

    /// `Effect{LumaKey}` pass — tex + sampler + a `[threshold, hi, invert, pad]`
    /// uniform. `hi` is pre-floored in Rust so the shader's smoothstep band and
    /// `ops::luma_key`'s agree exactly (parity).
    fn luma_key(
        &self,
        gpu: &GpuContext,
        src: &wgpu::Texture,
        target: &wgpu::Texture,
        threshold: f32,
        softness: f32,
        invert: bool,
    ) {
        let hi = threshold + softness.max(crate::graph::ops::KEY_BAND_EPS);
        let uniform = [threshold, hi, if invert { 1.0 } else { 0.0 }, 0.0];
        self.run_filter(gpu, &self.luma_key_pipeline, src, target, &uniform);
    }

    /// `Effect{ChromaKey}` pass — tex + sampler + a `[key.rgb, tolerance | hi,
    /// spill, dom, pad]` uniform. `key` is the sRGB→linear key colour, `dom` the
    /// dominant key channel; both are computed in Rust so the GPU matches
    /// `ops::chroma_key` exactly.
    fn chroma_key(
        &self,
        gpu: &GpuContext,
        src: &wgpu::Texture,
        target: &wgpu::Texture,
        key_linear: [f32; 3],
        tolerance: f32,
        edge_softness: f32,
        spill_suppress: f32,
    ) {
        let hi = tolerance + edge_softness.max(crate::graph::ops::KEY_BAND_EPS);
        let dom = if key_linear[0] >= key_linear[1] && key_linear[0] >= key_linear[2] {
            0.0
        } else if key_linear[1] >= key_linear[2] {
            1.0
        } else {
            2.0
        };
        let uniform = [
            key_linear[0],
            key_linear[1],
            key_linear[2],
            tolerance,
            hi,
            spill_suppress,
            dom,
            0.0,
        ];
        self.run_filter(gpu, &self.chroma_key_pipeline, src, target, &uniform);
    }

    /// Shared driver for the tex+sampler+uniform filter passes (LumaKey/ChromaKey).
    fn run_filter(
        &self,
        gpu: &GpuContext,
        pipeline: &wgpu::RenderPipeline,
        src: &wgpu::Texture,
        target: &wgpu::Texture,
        uniform: &[f32],
    ) {
        let view = src.create_view(&Default::default());
        let buf = gpu
            .device()
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("filter_uniform"),
                contents: bytemuck::cast_slice(uniform),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let bind = gpu.device().create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("filter_bg"),
            layout: &self.filter_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: buf.as_entire_binding(),
                },
            ],
        });
        self.run(gpu, pipeline, &bind, target);
    }

    /// `Effect{MaskShapeGen}` pass — a uniform-only ellipse-matte generator (no
    /// input texture), the WGSL twin of `ops::mask_shape`.
    #[allow(clippy::too_many_arguments)]
    fn mask_shape(
        &self,
        gpu: &GpuContext,
        target: &wgpu::Texture,
        center: [f32; 2],
        size: [f32; 2],
        rotation: f32,
        feather: f32,
        logical_w: u32,
        logical_h: u32,
    ) {
        let (cos_r, sin_r) = ((-rotation).cos(), (-rotation).sin());
        let inner = 1.0 - feather.clamp(0.0, 1.0).max(crate::graph::ops::KEY_BAND_EPS);
        let uniform = [
            center[0],
            center[1],
            size[0],
            size[1],
            cos_r,
            sin_r,
            inner,
            0.0,
            logical_w as f32,
            logical_h as f32,
            0.0,
            0.0,
        ];
        let buf = gpu
            .device()
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("mask_uniform"),
                contents: bytemuck::cast_slice(&uniform),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let bind = gpu.device().create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mask_bg"),
            layout: &self.mask_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buf.as_entire_binding(),
            }],
        });
        self.run(gpu, &self.mask_shape_pipeline, &bind, target);
    }

    fn merge(
        &self,
        gpu: &GpuContext,
        top: &wgpu::Texture,
        bottom: &wgpu::Texture,
        mode: BlendMode,
        opacity: f32,
        target: &wgpu::Texture,
    ) {
        let tv = top.create_view(&Default::default());
        let bv = bottom.create_view(&Default::default());
        // Clamp opacity to match the CPU reference `graph::ops::merge` exactly.
        let uni = MergeParams {
            mode: merge_mode_index(mode),
            opacity: opacity.clamp(0.0, 1.0),
            _pad: [0.0; 2],
        };
        let buf = gpu
            .device()
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("merge_uni"),
                contents: bytemuck::bytes_of(&uni),
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

    /// `WipeMix` pass (08 §2.0b): the WGSL twin of `ops::wipe`. `edge`/`hw` are
    /// computed in Rust identically to the CPU kernel so the smoothstep bands
    /// agree; the sweep coord is derived in-shader from the logical dims.
    #[allow(clippy::too_many_arguments)]
    fn wipe(
        &self,
        gpu: &GpuContext,
        incoming: &wgpu::Texture,
        outgoing: &wgpu::Texture,
        dir: WipeDirection,
        softness: f32,
        t: f32,
        target: &wgpu::Texture,
        logical_w: u32,
        logical_h: u32,
    ) {
        let s = softness.max(0.0);
        let edge = -s + t * (1.0 + 2.0 * s);
        let hw = s.max(crate::graph::ops::KEY_BAND_EPS);
        let uniform = [
            wipe_direction_index(dir),
            edge,
            hw,
            0.0,
            logical_w as f32,
            logical_h as f32,
            0.0,
            0.0,
        ];
        let iv = incoming.create_view(&Default::default());
        let ov = outgoing.create_view(&Default::default());
        let buf = gpu
            .device()
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("wipe_uniform"),
                contents: bytemuck::cast_slice(&uniform),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let bind = gpu.device().create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("wipe_bg"),
            layout: &self.merge_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&iv),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&ov),
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
        self.run(gpu, &self.wipe_pipeline, &bind, target);
    }

    /// `PushMix` pass (08 §2.0b): the WGSL twin of `ops::push`. `dims.zw` is the
    /// source's logical size (edge-clamp bound); `dims.xy` the logical canvas.
    #[allow(clippy::too_many_arguments)]
    fn push(
        &self,
        gpu: &GpuContext,
        incoming: &wgpu::Texture,
        outgoing: &wgpu::Texture,
        dir: WipeDirection,
        t: f32,
        target: &wgpu::Texture,
        logical_w: u32,
        logical_h: u32,
        source_w: u32,
        source_h: u32,
    ) {
        let uniform = [
            wipe_direction_index(dir),
            t,
            0.0,
            0.0,
            logical_w as f32,
            logical_h as f32,
            source_w as f32,
            source_h as f32,
        ];
        let iv = incoming.create_view(&Default::default());
        let ov = outgoing.create_view(&Default::default());
        let buf = gpu
            .device()
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("push_uniform"),
                contents: bytemuck::cast_slice(&uniform),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let bind = gpu.device().create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("push_bg"),
            layout: &self.push_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&iv),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&ov),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: buf.as_entire_binding(),
                },
            ],
        });
        self.run(gpu, &self.push_pipeline, &bind, target);
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
pub fn read_texture_rgba16f(
    gpu: &GpuContext,
    tex: &wgpu::Texture,
    w: u32,
    h: u32,
) -> Vec<[f32; 4]> {
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
        fn raster_vector(&mut self, _: VectorRef, _: VectorStateKey, _: u32, _: u32) -> Image {
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
            let expected =
                crate::graph::eval_cpu::evaluate(&graph, (canvas_w, canvas_h), &mut cpu_source);
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
                color: Color {
                    r: 0.5,
                    g: 0.25,
                    b: 0.75,
                    a: 1.0,
                },
            },
            crate::contract::Tick(0),
            crate::contract::Tick::from_seconds(2),
        ));
        seq.video_tracks.push(t);
        project.insert_sequence(seq);

        let compiled = compile(
            &project,
            seq_id,
            0,
            crate::contract::Tick(0),
            Quality::FULL,
            None,
        );
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
        compile(
            &project,
            seq_id,
            0,
            crate::contract::Tick(0),
            Quality::FULL,
            None,
        )
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

    /// Every `BlendMode`, so a new variant added without a shader case fails here.
    const ALL_BLEND_MODES: [BlendMode; 26] = [
        BlendMode::Normal,
        BlendMode::Multiply,
        BlendMode::Screen,
        BlendMode::Overlay,
        BlendMode::Darken,
        BlendMode::Lighten,
        BlendMode::ColorDodge,
        BlendMode::ColorBurn,
        BlendMode::HardLight,
        BlendMode::SoftLight,
        BlendMode::Difference,
        BlendMode::Exclusion,
        BlendMode::Hue,
        BlendMode::Saturation,
        BlendMode::Color,
        BlendMode::Luminosity,
        BlendMode::LinearDodge,
        BlendMode::LinearBurn,
        BlendMode::Subtract,
        BlendMode::Divide,
        BlendMode::VividLight,
        BlendMode::LinearLight,
        BlendMode::PinLight,
        BlendMode::HardMix,
        BlendMode::DarkerColor,
        BlendMode::LighterColor,
    ];

    /// A two-`SolidColor` → `Merge` graph: `top` over `bottom` under `mode` at
    /// full opacity. Colours are premultiplied linear (the working space), so the
    /// CPU and GPU evaluators start from identical pixels.
    fn merge_graph(
        top: crate::graph::ir::LinearColor,
        bottom: crate::graph::ir::LinearColor,
        mode: BlendMode,
    ) -> crate::graph::compile::CompiledFrame {
        let graph = FrameGraph {
            nodes: vec![
                IrNode {
                    op: IrOp::SolidColor { color: top },
                    inputs: vec![],
                    content_hash: ContentHash(700),
                },
                IrNode {
                    op: IrOp::SolidColor { color: bottom },
                    inputs: vec![],
                    content_hash: ContentHash(701),
                },
                IrNode {
                    op: IrOp::Merge { mode, opacity: 1.0 },
                    inputs: vec![
                        (IrNodeId(0), OutPort::default()),
                        (IrNodeId(1), OutPort::default()),
                    ],
                    content_hash: ContentHash(702),
                },
            ],
            output: Some(IrNodeId(2)),
        };
        crate::graph::compile::CompiledFrame {
            graph,
            diagnostics: vec![],
        }
    }

    /// K-0.3a / E-9: the GPU Merge pass must agree with the CPU `blend_rgb`
    /// reference for every one of the 26 blend modes, over both an opaque and a
    /// transparent backdrop (a semi-transparent top exercises the `Cs'` mix and
    /// the source-over composite). Closes the CPU/GPU divergence the Normal-only
    /// shader left. Self-skips without a GPU adapter; run `--test-threads=1`.
    #[test]
    fn merge_gpu_matches_cpu_for_every_blend_mode() {
        use crate::graph::ir::LinearColor;
        // Semi-transparent top (straight 0.7,0.35,0.55 @ α0.5, premultiplied) over
        // an opaque and a transparent backdrop. Colours are chosen to sit clear of
        // every mode's discontinuities — in particular `HardMix`'s 0.5 threshold,
        // where a sub-ULP CPU/GPU difference would flip the output 0↔1 (an inherent
        // discontinuity of the mode, not a divergence in the composite math).
        let top = LinearColor {
            r: 0.35,
            g: 0.175,
            b: 0.275,
            a: 0.5,
        };
        let opaque = LinearColor {
            r: 0.25,
            g: 0.6,
            b: 0.4,
            a: 1.0,
        };
        let transparent = LinearColor {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.0,
        };
        for mode in ALL_BLEND_MODES {
            for bottom in [opaque, transparent] {
                assert_graph_gpu_matches_cpu(&merge_graph(top, bottom, mode), 1e-3);
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
            Color {
                r: 0.5,
                g: 0.4,
                b: 0.6,
                a: 1.0,
            },
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
            compiled
                .graph
                .nodes
                .iter()
                .any(|n| matches!(n.op, IrOp::Grade { .. })),
            "a real Grade node is present"
        );
        assert_graph_gpu_matches_cpu(&compiled, 1e-3);
    }

    /// GPU/CPU parity for `Effect{Invert}` (08 §3).
    #[test]
    fn inverted_solid_gpu_matches_cpu_reference() {
        use photonic_core::timeline::{ClipEffect, EffectKind};
        let compiled = solid_project_graph(
            Color {
                r: 0.7,
                g: 0.2,
                b: 0.9,
                a: 1.0,
            },
            |clip| clip.effects.push(ClipEffect::new(EffectKind::Invert)),
        );
        assert_graph_gpu_matches_cpu(&compiled, 1e-3);
    }

    /// Build a clip effect of `kind` with the given `(path, PropValue)` params set
    /// on its base, for the per-effect parity tests (K-0.2 Step B).
    fn effect_with(
        kind: photonic_core::timeline::EffectKind,
        params: &[(&str, photonic_core::timeline::PropValue)],
    ) -> photonic_core::timeline::ClipEffect {
        let mut eff = photonic_core::timeline::ClipEffect::new(kind);
        for (path, value) in params {
            eff.params.base.set(*path, value.clone());
        }
        eff
    }

    /// GPU/CPU parity for `Effect{LumaKey}` (08 §3): the shader's α-keep smoothstep
    /// must match `ops::luma_key`. Threshold/softness sit so the solid's luma lands
    /// mid-band (a fractional keep, exercising the interpolation, not a 0/1 step).
    #[test]
    fn luma_key_solid_gpu_matches_cpu_reference() {
        use photonic_core::timeline::{EffectKind, PropValue};
        let compiled = solid_project_graph(
            Color {
                r: 0.5,
                g: 0.5,
                b: 0.5,
                a: 1.0,
            },
            |clip| {
                clip.effects.push(effect_with(
                    EffectKind::LumaKey,
                    &[
                        ("params.threshold", PropValue::Float(0.15)),
                        ("params.softness", PropValue::Float(0.2)),
                        ("params.invert", PropValue::Bool(false)),
                    ],
                ))
            },
        );
        assert_graph_gpu_matches_cpu(&compiled, 1e-3);
    }

    /// GPU/CPU parity for `Effect{ChromaKey}` (08 §3): keep-smoothstep + dominant-
    /// channel spill suppression. The solid is greenish and the key is green, so
    /// the colour distance lands in the feather band and the spill branch fires.
    #[test]
    fn chroma_key_solid_gpu_matches_cpu_reference() {
        use photonic_core::timeline::{EffectKind, PropValue};
        let compiled = solid_project_graph(
            Color {
                r: 0.2,
                g: 0.7,
                b: 0.2,
                a: 1.0,
            },
            |clip| {
                clip.effects.push(effect_with(
                    EffectKind::ChromaKey,
                    &[
                        (
                            "params.key_color",
                            PropValue::Color(Color {
                                r: 0.0,
                                g: 1.0,
                                b: 0.0,
                                a: 1.0,
                            }),
                        ),
                        ("params.tolerance", PropValue::Float(0.3)),
                        ("params.edge_softness", PropValue::Float(0.4)),
                        ("params.spill_suppress", PropValue::Float(0.5)),
                    ],
                ))
            },
        );
        assert_graph_gpu_matches_cpu(&compiled, 1e-3);
    }

    /// GPU/CPU parity for `Effect{MaskShapeGen}` (08 §3): a 0-input ellipse-matte
    /// generator. A centered feathered ellipse over the 8×8 canvas exercises the
    /// interior (α=1), exterior (α=0), and the feathered edge band.
    #[test]
    fn mask_shape_solid_gpu_matches_cpu_reference() {
        use photonic_core::timeline::{EffectKind, PropValue};
        let compiled = solid_project_graph(
            Color {
                r: 0.4,
                g: 0.3,
                b: 0.6,
                a: 1.0,
            },
            |clip| {
                clip.effects.push(effect_with(
                    EffectKind::MaskShapeGen,
                    &[
                        ("params.center_x", PropValue::Float(0.5)),
                        ("params.center_y", PropValue::Float(0.5)),
                        ("params.size_x", PropValue::Float(0.5)),
                        ("params.size_y", PropValue::Float(0.5)),
                        ("params.rotation", PropValue::Float(0.0)),
                        ("params.feather", PropValue::Float(0.2)),
                    ],
                ))
            },
        );
        assert_graph_gpu_matches_cpu(&compiled, 1e-3);
    }

    /// GPU/CPU parity for `Effect{Blur}` (K-0.2): a uniform solid is a fixed
    /// point of the Gaussian, locking the dual-pass path. Tolerance is looser
    /// than 1e-3 because the multi-tap f16 accumulation drifts ~1.5e-3 on α≈1.
    #[test]
    fn blur_solid_gpu_matches_cpu_reference() {
        use photonic_core::timeline::{EffectKind, PropValue};
        let compiled = solid_project_graph(
            Color {
                r: 0.6,
                g: 0.3,
                b: 0.1,
                a: 1.0,
            },
            |clip| {
                clip.effects.push(effect_with(
                    EffectKind::Blur,
                    &[("params.radius", PropValue::Float(1.5))],
                ))
            },
        );
        assert_graph_gpu_matches_cpu(&compiled, 1e-3);
    }

    /// GPU/CPU parity for `Effect{Sharpen}` (K-0.2) on a uniform solid.
    #[test]
    fn sharpen_solid_gpu_matches_cpu_reference() {
        use photonic_core::timeline::{EffectKind, PropValue};
        let compiled = solid_project_graph(
            Color {
                r: 0.4,
                g: 0.5,
                b: 0.6,
                a: 1.0,
            },
            |clip| {
                clip.effects.push(effect_with(
                    EffectKind::Sharpen,
                    &[
                        ("params.amount", PropValue::Float(1.0)),
                        ("params.radius", PropValue::Float(1.0)),
                    ],
                ))
            },
        );
        assert_graph_gpu_matches_cpu(&compiled, 1e-3);
    }

    /// GPU/CPU parity for `Effect{Glow}` (K-0.2): threshold 0 keeps every pixel
    /// in the extract, so a bright solid blooms under intensity.
    #[test]
    fn glow_solid_gpu_matches_cpu_reference() {
        use photonic_core::timeline::{EffectKind, PropValue};
        let compiled = solid_project_graph(
            Color {
                r: 0.9,
                g: 0.85,
                b: 0.7,
                a: 1.0,
            },
            |clip| {
                clip.effects.push(effect_with(
                    EffectKind::Glow,
                    &[
                        ("params.radius", PropValue::Float(1.0)),
                        ("params.threshold", PropValue::Float(0.0)),
                        ("params.intensity", PropValue::Float(0.5)),
                        (
                            "params.tint",
                            PropValue::Color(Color {
                                r: 1.0,
                                g: 1.0,
                                b: 1.0,
                                a: 1.0,
                            }),
                        ),
                    ],
                ))
            },
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
                color: Color {
                    r: 0.0,
                    g: 0.0,
                    b: 0.6,
                    a: 1.0,
                },
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
            active_color: Color {
                r: 1.0,
                g: 1.0,
                b: 0.0,
                a: 1.0,
            }, // yellow
            inactive_color: Color {
                r: 0.5,
                g: 0.5,
                b: 0.5,
                a: 1.0,
            }, // grey
        };
        let (project, seq_id) = captioned_project(Some(highlight));
        let mut eval = Evaluator::new(gpu.clone());
        let render = |eval: &mut Evaluator, t: i64| {
            let c = compile(
                &project,
                seq_id,
                0,
                crate::contract::Tick(t),
                Quality::FULL,
                None,
            );
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
            fill: Color {
                r: 1.0,
                g: 0.0,
                b: 0.0,
                a: 1.0,
            },
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

        let compiled = compile(
            &project,
            seq_id,
            0,
            crate::contract::Tick(100),
            Quality::FULL,
            None,
        );
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

    // ── K-0.5 resolved LUT grade / K-0.4 directional transitions parity ───────

    /// A `SolidColor → Grade{Lut3d} → Output` graph carrying a resolved 2³ LUT
    /// that maps `c → 0.5·c` (a linear map, so trilinear reconstruction is exact
    /// on both the GPU sampler and the CPU reference).
    fn lut_grade_graph(color: crate::graph::ir::LinearColor) -> crate::graph::compile::CompiledFrame {
        use photonic_render::grade::{ResolvedGradeOp, ResolvedGradePayload, ResolvedLut3d};
        let mut table = photonic_render::Lut3d::identity(2);
        for sample in &mut table.data {
            for v in sample {
                *v *= 0.5;
            }
        }
        let op = ResolvedGradeOp {
            payload: ResolvedGradePayload::Lut3d(ResolvedLut3d {
                table: Arc::new(table),
                intensity: 1.0,
                tetrahedral: false,
            }),
            mask: None,
        };
        let graph = FrameGraph {
            nodes: vec![
                IrNode {
                    op: IrOp::SolidColor { color },
                    inputs: vec![],
                    content_hash: ContentHash(820),
                },
                IrNode {
                    op: IrOp::Grade { ops: vec![op] },
                    inputs: vec![(IrNodeId(0), OutPort::default())],
                    content_hash: ContentHash(821),
                },
            ],
            output: Some(IrNodeId(1)),
        };
        crate::graph::compile::CompiledFrame {
            graph,
            diagnostics: vec![],
        }
    }

    /// K-0.5: a resolved `Lut3d` grade evaluates identically on the GPU (3D-LUT
    /// sampler) and the CPU reference (03 §4.4).
    #[test]
    fn resolved_lut_grade_gpu_matches_cpu() {
        let compiled = lut_grade_graph(crate::graph::ir::LinearColor {
            r: 0.6,
            g: 0.4,
            b: 0.8,
            a: 1.0,
        });
        assert!(
            compiled
                .graph
                .nodes
                .iter()
                .any(|n| matches!(&n.op, IrOp::Grade { ops } if !ops.is_empty())),
            "a resolved LUT Grade node is present"
        );
        assert_graph_gpu_matches_cpu(&compiled, 1e-3);
    }

    /// A two-`SolidColor` → binary-transition (`WipeMix`/`PushMix`) → `Output`
    /// graph; inputs are `[incoming, outgoing]`.
    fn transition_graph(
        op: IrOp,
        incoming: crate::graph::ir::LinearColor,
        outgoing: crate::graph::ir::LinearColor,
    ) -> crate::graph::compile::CompiledFrame {
        let graph = FrameGraph {
            nodes: vec![
                IrNode {
                    op: IrOp::SolidColor { color: incoming },
                    inputs: vec![],
                    content_hash: ContentHash(830),
                },
                IrNode {
                    op: IrOp::SolidColor { color: outgoing },
                    inputs: vec![],
                    content_hash: ContentHash(831),
                },
                IrNode {
                    op,
                    inputs: vec![
                        (IrNodeId(0), OutPort::default()),
                        (IrNodeId(1), OutPort::default()),
                    ],
                    content_hash: ContentHash(832),
                },
            ],
            output: Some(IrNodeId(2)),
        };
        crate::graph::compile::CompiledFrame {
            graph,
            diagnostics: vec![],
        }
    }

    const PARITY_DIRS: [WipeDirection; 4] = [
        WipeDirection::LeftToRight,
        WipeDirection::RightToLeft,
        WipeDirection::TopToBottom,
        WipeDirection::BottomToTop,
    ];

    /// K-0.4: the `WipeMix` GPU pass must match `ops::wipe` for every direction at
    /// t = 0.25/0.5/0.75 (a softened edge exercises the smoothstep band).
    #[test]
    fn wipe_gpu_matches_cpu_for_every_direction_and_t() {
        let inc = crate::graph::ir::LinearColor {
            r: 0.8,
            g: 0.2,
            b: 0.3,
            a: 1.0,
        };
        let outg = crate::graph::ir::LinearColor {
            r: 0.1,
            g: 0.5,
            b: 0.9,
            a: 1.0,
        };
        for dir in PARITY_DIRS {
            for t in [0.25f32, 0.5, 0.75] {
                let op = IrOp::WipeMix {
                    direction: dir,
                    softness: 0.1,
                    t,
                };
                assert_graph_gpu_matches_cpu(&transition_graph(op, inc, outg), 1e-3);
            }
        }
    }

    /// K-0.4: the `PushMix` GPU pass must match `ops::push` for every direction at
    /// t = 0.25/0.5/0.75.
    #[test]
    fn push_gpu_matches_cpu_for_every_direction_and_t() {
        let inc = crate::graph::ir::LinearColor {
            r: 0.8,
            g: 0.2,
            b: 0.3,
            a: 1.0,
        };
        let outg = crate::graph::ir::LinearColor {
            r: 0.1,
            g: 0.5,
            b: 0.9,
            a: 1.0,
        };
        for dir in PARITY_DIRS {
            for t in [0.25f32, 0.5, 0.75] {
                let op = IrOp::PushMix {
                    direction: dir,
                    t,
                };
                assert_graph_gpu_matches_cpu(&transition_graph(op, inc, outg), 1e-3);
            }
        }
    }
}
