//! Video scopes (07 §5): waveform, vectorscope, histogram — compute-shader
//! accumulation into storage buffers with atomic adds, each with a CPU
//! reference (07 §6.3).
//!
//! **Scope signal domain.** Scopes read the graded working texture (03 §3.6
//! readback point: after the clip's `Grade`, before `CaptionOverlay`/fold) — a
//! premultiplied linear-Rec.709 `Rgba16Float` texture. Each pixel is
//! unpremultiplied to straight color and encoded through the **BT.709 OETF**
//! into the video-signal (gamma) domain before any scope math, which is the
//! conventional domain scopes measure (signal level, not scene light) and lets
//! the Rec.709 luma weights and the YCbCr denominators reuse `crate::color`'s
//! BT.709 constants. Luma is Rec.709 (07 §3).

use wgpu::util::DeviceExt;

use crate::color;
use crate::grade::luma709;

/// Luma/channel histogram bin count (07 §5, §6.4 fallback is 128).
pub const HIST_BINS: usize = 256;
/// Waveform vertical resolution (intensity bins per column).
pub const WAVEFORM_BINS: usize = 256;
/// Vectorscope plane resolution (Cb × Cr).
pub const VECTORSCOPE_SIZE: usize = 256;

// ── CPU signal encode ───────────────────────────────────────────────────────

/// Unpremultiply + BT.709 OETF one working pixel into signal-domain R'G'B'.
#[inline]
fn signal(px: &[f32]) -> [f32; 3] {
    let a = px[3].max(1e-6);
    [
        color::bt709_oetf((px[0] / a).clamp(0.0, 1.0)),
        color::bt709_oetf((px[1] / a).clamp(0.0, 1.0)),
        color::bt709_oetf((px[2] / a).clamp(0.0, 1.0)),
    ]
}

#[inline]
fn bin256(v: f32) -> usize {
    (v * 255.0).round().clamp(0.0, 255.0) as usize
}

// ── CPU scope data ──────────────────────────────────────────────────────────

/// 256-bin luma + per-channel histogram (07 §5).
#[derive(Clone, Debug, PartialEq)]
pub struct Histogram {
    pub luma: [u32; HIST_BINS],
    pub red: [u32; HIST_BINS],
    pub green: [u32; HIST_BINS],
    pub blue: [u32; HIST_BINS],
}

impl Default for Histogram {
    fn default() -> Self {
        Histogram {
            luma: [0; HIST_BINS],
            red: [0; HIST_BINS],
            green: [0; HIST_BINS],
            blue: [0; HIST_BINS],
        }
    }
}

/// Per-column intensity histogram (07 §5). `count(x, bin) = data[x*bins + bin]`.
#[derive(Clone, Debug, PartialEq)]
pub struct Waveform {
    pub width: usize,
    pub bins: usize,
    pub data: Vec<u32>,
}

impl Waveform {
    #[inline]
    pub fn count(&self, x: usize, bin: usize) -> u32 {
        self.data[x * self.bins + bin]
    }
}

/// Cb×Cr scatter histogram (07 §5). `count(cb, cr) = data[cr*size + cb]`.
#[derive(Clone, Debug, PartialEq)]
pub struct Vectorscope {
    pub size: usize,
    pub data: Vec<u32>,
}

impl Vectorscope {
    #[inline]
    pub fn count(&self, cb: usize, cr: usize) -> u32 {
        self.data[cr * self.size + cb]
    }
}

/// All three scopes for one frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Scopes {
    pub histogram: Histogram,
    pub waveform: Waveform,
    pub vectorscope: Vectorscope,
}

// ── CPU references (07 §6.3) ────────────────────────────────────────────────

/// 256-bin luma + per-channel histogram over an RGBA `f32` buffer.
pub fn histogram_cpu(pixels: &[f32], _width: u32, _height: u32) -> Histogram {
    let mut h = Histogram::default();
    for px in pixels.chunks_exact(4) {
        let s = signal(px);
        h.luma[bin256(luma709(s))] += 1;
        h.red[bin256(s[0])] += 1;
        h.green[bin256(s[1])] += 1;
        h.blue[bin256(s[2])] += 1;
    }
    h
}

/// Per-column luma waveform over an RGBA `f32` buffer.
pub fn waveform_cpu(pixels: &[f32], width: u32, height: u32) -> Waveform {
    let w = width.max(1) as usize;
    let mut data = vec![0u32; w * WAVEFORM_BINS];
    for (i, px) in pixels.chunks_exact(4).enumerate() {
        let x = i % w;
        let s = signal(px);
        let bin = bin256(luma709(s));
        data[x * WAVEFORM_BINS + bin] += 1;
    }
    let _ = height;
    Waveform {
        width: w,
        bins: WAVEFORM_BINS,
        data,
    }
}

/// Rec.709 Cb×Cr vectorscope over an RGBA `f32` buffer. The YCbCr denominators
/// reuse `crate::color::{BT709_CB_B, BT709_CR_R}` (07 §5).
pub fn vectorscope_cpu(pixels: &[f32], _width: u32, _height: u32) -> Vectorscope {
    let mut data = vec![0u32; VECTORSCOPE_SIZE * VECTORSCOPE_SIZE];
    for px in pixels.chunks_exact(4) {
        let s = signal(px);
        let (cb_bin, cr_bin) = cbcr_bins(s);
        data[cr_bin * VECTORSCOPE_SIZE + cb_bin] += 1;
    }
    Vectorscope {
        size: VECTORSCOPE_SIZE,
        data,
    }
}

/// Rec.709 (Cb, Cr) bin indices for a signal-domain R'G'B'.
#[inline]
fn cbcr_bins(s: [f32; 3]) -> (usize, usize) {
    let y = luma709(s);
    let cb = (s[2] - y) / color::BT709_CB_B; // 1.8556
    let cr = (s[0] - y) / color::BT709_CR_R; // 1.5748
    let cb_bin = ((cb + 0.5) * 255.0).round().clamp(0.0, 255.0) as usize;
    let cr_bin = ((cr + 0.5) * 255.0).round().clamp(0.0, 255.0) as usize;
    (cb_bin, cr_bin)
}

/// All three CPU scopes for one frame.
pub fn scopes_from_pixels_cpu(pixels: &[f32], width: u32, height: u32) -> Scopes {
    Scopes {
        histogram: histogram_cpu(pixels, width, height),
        waveform: waveform_cpu(pixels, width, height),
        vectorscope: vectorscope_cpu(pixels, width, height),
    }
}

// ── compute shaders (atomic accumulation, 07 §5) ────────────────────────────

/// Shared compute prelude: unpremultiply + BT.709 OETF signal encode, Rec.709
/// luma, and the 256-bin quantizer. Constants match `crate::color` (asserted by
/// `contains_token`).
const COMPUTE_PRELUDE: &str = r#"
@group(0) @binding(0) var t_src: texture_2d<f32>;
@group(0) @binding(1) var<storage, read_write> bins: array<atomic<u32>>;

fn oetf709(e: f32) -> f32 {
    let x = clamp(e, 0.0, 1.0);
    if (x < 0.018) { return 4.5 * x; }
    return 1.099 * pow(x, 0.45) - 0.099;
}
fn signal(px: vec4<f32>) -> vec3<f32> {
    let a = max(px.a, 1e-6);
    let s = px.rgb / a;
    return vec3<f32>(oetf709(s.r), oetf709(s.g), oetf709(s.b));
}
fn luma709(c: vec3<f32>) -> f32 { return 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b; }
fn bin256(v: f32) -> u32 { return u32(clamp(round(v * 255.0), 0.0, 255.0)); }
"#;

/// Histogram compute kernel: luma into `[0,256)`, R/G/B into
/// `[256,512)/[512,768)/[768,1024)` (07 §5).
pub const HISTOGRAM_SHADER: &str = r#"
__PRELUDE__
@compute @workgroup_size(8, 8, 1)
fn cs_hist(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = textureDimensions(t_src);
    if (gid.x >= dims.x || gid.y >= dims.y) { return; }
    let px = textureLoad(t_src, vec2<i32>(i32(gid.x), i32(gid.y)), 0);
    let s = signal(px);
    atomicAdd(&bins[bin256(luma709(s))], 1u);
    atomicAdd(&bins[256u + bin256(s.r)], 1u);
    atomicAdd(&bins[512u + bin256(s.g)], 1u);
    atomicAdd(&bins[768u + bin256(s.b)], 1u);
}
"#;

/// Waveform compute kernel: per-column luma histogram,
/// `bins[x*256 + luma_bin]` (07 §5).
pub const WAVEFORM_SHADER: &str = r#"
__PRELUDE__
@compute @workgroup_size(8, 8, 1)
fn cs_wave(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = textureDimensions(t_src);
    if (gid.x >= dims.x || gid.y >= dims.y) { return; }
    let px = textureLoad(t_src, vec2<i32>(i32(gid.x), i32(gid.y)), 0);
    let s = signal(px);
    atomicAdd(&bins[gid.x * 256u + bin256(luma709(s))], 1u);
}
"#;

/// Vectorscope compute kernel: Rec.709 Cb×Cr scatter,
/// `bins[cr*256 + cb]` (07 §5).
pub const VECTORSCOPE_SHADER: &str = r#"
__PRELUDE__
fn cbcr(s: vec3<f32>) -> vec2<u32> {
    let y = luma709(s);
    let cb = (s.b - y) / 1.8556;
    let cr = (s.r - y) / 1.5748;
    let cbn = u32(clamp(round((cb + 0.5) * 255.0), 0.0, 255.0));
    let crn = u32(clamp(round((cr + 0.5) * 255.0), 0.0, 255.0));
    return vec2<u32>(cbn, crn);
}
@compute @workgroup_size(8, 8, 1)
fn cs_vector(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = textureDimensions(t_src);
    if (gid.x >= dims.x || gid.y >= dims.y) { return; }
    let px = textureLoad(t_src, vec2<i32>(i32(gid.x), i32(gid.y)), 0);
    let s = signal(px);
    let b = cbcr(s);
    atomicAdd(&bins[b.y * 256u + b.x], 1u);
}
"#;

fn expand(src: &str) -> String {
    src.replace("__PRELUDE__", COMPUTE_PRELUDE)
}

// Expanded sources for the pipeline shader-validation test (pipeline.rs).
#[cfg(test)]
pub(crate) fn expanded_histogram_shader() -> String {
    expand(HISTOGRAM_SHADER)
}
#[cfg(test)]
pub(crate) fn expanded_waveform_shader() -> String {
    expand(WAVEFORM_SHADER)
}
#[cfg(test)]
pub(crate) fn expanded_vectorscope_shader() -> String {
    expand(VECTORSCOPE_SHADER)
}

// ── GPU runner (readback-point API) ─────────────────────────────────────────

/// Run one scope compute shader over `tex` and read back `out_len` `u32` bins.
/// `tex` is any `Rgba16Float` texture — the 03 §3.6 readback point (the graded,
/// pre-composite clip texture), or any working texture a caller hands in.
fn run_scope(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    tex: &wgpu::Texture,
    shader_src: &str,
    entry: &str,
    out_len: usize,
) -> Vec<u32> {
    let bins = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("scope_bins"),
        contents: bytemuck::cast_slice(&vec![0u32; out_len]),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
    });
    let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("scope_bgl"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    });
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("scope_shader"),
        source: wgpu::ShaderSource::Wgsl(expand(shader_src).into()),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("scope_layout"),
        bind_group_layouts: &[&bgl],
        push_constant_ranges: &[],
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("scope_pipeline"),
        layout: Some(&layout),
        module: &shader,
        entry_point: entry,
        compilation_options: Default::default(),
        cache: None,
    });
    let view = tex.create_view(&Default::default());
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("scope_bg"),
        layout: &bgl,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: bins.as_entire_binding(),
            },
        ],
    });
    let (w, h) = (tex.width(), tex.height());
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("scope_rb"),
        size: (out_len * 4) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut enc = device.create_command_encoder(&Default::default());
    {
        let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("scope_pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
    }
    enc.copy_buffer_to_buffer(&bins, 0, &staging, 0, (out_len * 4) as u64);
    queue.submit([enc.finish()]);

    let slice = staging.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    device.poll(wgpu::Maintain::Wait);
    rx.recv().unwrap().unwrap();
    let raw = slice.get_mapped_range();
    let out: Vec<u32> = bytemuck::cast_slice(&raw).to_vec();
    drop(raw);
    staging.unmap();
    out
}

/// GPU histogram at the readback point (07 §5).
pub fn histogram_gpu(device: &wgpu::Device, queue: &wgpu::Queue, tex: &wgpu::Texture) -> Histogram {
    let raw = run_scope(
        device,
        queue,
        tex,
        HISTOGRAM_SHADER,
        "cs_hist",
        HIST_BINS * 4,
    );
    let mut h = Histogram::default();
    h.luma.copy_from_slice(&raw[0..256]);
    h.red.copy_from_slice(&raw[256..512]);
    h.green.copy_from_slice(&raw[512..768]);
    h.blue.copy_from_slice(&raw[768..1024]);
    h
}

/// GPU waveform at the readback point (07 §5).
pub fn waveform_gpu(device: &wgpu::Device, queue: &wgpu::Queue, tex: &wgpu::Texture) -> Waveform {
    let w = tex.width() as usize;
    let data = run_scope(
        device,
        queue,
        tex,
        WAVEFORM_SHADER,
        "cs_wave",
        w * WAVEFORM_BINS,
    );
    Waveform {
        width: w,
        bins: WAVEFORM_BINS,
        data,
    }
}

/// GPU vectorscope at the readback point (07 §5).
pub fn vectorscope_gpu(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    tex: &wgpu::Texture,
) -> Vectorscope {
    let data = run_scope(
        device,
        queue,
        tex,
        VECTORSCOPE_SHADER,
        "cs_vector",
        VECTORSCOPE_SIZE * VECTORSCOPE_SIZE,
    );
    Vectorscope {
        size: VECTORSCOPE_SIZE,
        data,
    }
}

/// All three GPU scopes for the readback-point texture (07 §5).
pub fn scopes_from_texture_gpu(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    tex: &wgpu::Texture,
) -> Scopes {
    Scopes {
        histogram: histogram_gpu(device, queue, tex),
        waveform: waveform_gpu(device, queue, tex),
        vectorscope: vectorscope_gpu(device, queue, tex),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::WORKING_FORMAT;

    // ── CPU references (07 §6.3) ────────────────────────────────────────────

    /// A gray ramp whose *signal-domain* luma is exactly `i/(N-1)`: pixel i is
    /// the linear value that BT.709-encodes back to `i/(N-1)`.
    fn signal_ramp(n: usize) -> Vec<f32> {
        let mut v = Vec::with_capacity(n * 4);
        for i in 0..n {
            let sig = i as f32 / (n - 1) as f32;
            let lin = color::bt709_eotf(sig);
            v.extend_from_slice(&[lin, lin, lin, 1.0]);
        }
        v
    }

    #[test]
    fn waveform_of_gradient_is_diagonal() {
        // 07 §6.3: a monotone ramp produces a diagonal waveform — column x lights
        // exactly bin round(x/(N-1) * 255).
        let n = 64;
        let px = signal_ramp(n);
        let wf = waveform_cpu(&px, n as u32, 1);
        for x in 0..n {
            let expect = ((x as f32 / (n - 1) as f32) * 255.0).round() as usize;
            assert_eq!(wf.count(x, expect), 1, "col {x} bin {expect}");
            let total: u32 = (0..WAVEFORM_BINS).map(|b| wf.count(x, b)).sum();
            assert_eq!(total, 1, "one sample per column");
        }
    }

    #[test]
    fn histogram_counts_are_exact() {
        // Four black + four white opaque pixels → 4 in bin 0, 4 in bin 255.
        let mut px = Vec::new();
        for _ in 0..4 {
            px.extend_from_slice(&[0.0, 0.0, 0.0, 1.0]);
        }
        for _ in 0..4 {
            px.extend_from_slice(&[1.0, 1.0, 1.0, 1.0]);
        }
        let h = histogram_cpu(&px, 8, 1);
        assert_eq!(h.luma[0], 4);
        assert_eq!(h.luma[255], 4);
        let total: u32 = h.luma.iter().sum();
        assert_eq!(total, 8);
    }

    #[test]
    fn vectorscope_primaries_cluster_at_known_points() {
        // 07 §6.3: pure red signal → Cr max (255), Cb ≈ 98; the neutral gray at
        // the centre (128,128).
        let red_lin = [
            color::bt709_eotf(1.0),
            color::bt709_eotf(0.0),
            color::bt709_eotf(0.0),
            1.0,
        ];
        let (cb, cr) = cbcr_bins([1.0, 0.0, 0.0]);
        assert_eq!(cr, 255, "red at Cr max");
        assert_eq!(cb, 98, "red Cb bin");
        let vs = vectorscope_cpu(&red_lin, 1, 1);
        assert_eq!(vs.count(cb, cr), 1);

        // Neutral gray sits at plane centre.
        let gray = [color::bt709_eotf(0.5); 3];
        let (gcb, gcr) = cbcr_bins(gray);
        assert_eq!((gcb, gcr), (128, 128), "gray centred");
    }

    // ── GPU parity (07 §6.3), adapter-skip ──────────────────────────────────

    fn try_device() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))?;
        pollster::block_on(adapter.request_device(&Default::default(), None)).ok()
    }

    fn f32_to_f16_bits(v: f32) -> u16 {
        let b = v.to_bits();
        let sign = ((b >> 16) & 0x8000) as u16;
        let e = ((b >> 23) & 0xff) as i32 - 112;
        let m = b & 0x7fffff;
        if e <= 0 {
            sign
        } else if e >= 0x1f {
            sign | 0x7c00
        } else {
            sign | ((e as u16) << 10) | ((m >> 13) as u16)
        }
    }

    fn upload(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        px: &[f32],
        w: u32,
        h: u32,
    ) -> wgpu::Texture {
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("scope_in"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: WORKING_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let mut data = Vec::new();
        for c in px {
            data.extend_from_slice(&f32_to_f16_bits(*c).to_le_bytes());
        }
        queue.write_texture(
            tex.as_image_copy(),
            &data,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(w * 8),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        tex
    }

    /// L1 difference between two bin arrays — each boundary-crossing pixel
    /// contributes 2 (one bin −1, its neighbour +1).
    fn l1(a: &[u32], b: &[u32]) -> u64 {
        a.iter().zip(b).map(|(x, y)| x.abs_diff(*y) as u64).sum()
    }

    #[test]
    fn gpu_histogram_matches_cpu() {
        let Some((device, queue)) = try_device() else {
            eprintln!("no GPU adapter — skipping scope histogram parity");
            return;
        };
        let (w, h) = (16u32, 16u32);
        let mut px = Vec::new();
        for i in 0..(w * h) {
            let g = (i % 32) as f32 / 31.0;
            px.extend_from_slice(&[
                color::bt709_eotf(g),
                color::bt709_eotf(g),
                color::bt709_eotf(g),
                1.0,
            ]);
        }
        let tex = upload(&device, &queue, &px, w, h);
        let gpu = histogram_gpu(&device, &queue, &tex);
        let cpu = histogram_cpu(&px, w, h);
        assert_eq!(gpu.luma.iter().sum::<u32>(), w * h);
        assert!(l1(&gpu.luma, &cpu.luma) <= 4, "luma drift");
        assert!(l1(&gpu.red, &cpu.red) <= 4, "red drift");
    }

    #[test]
    fn gpu_waveform_matches_cpu() {
        let Some((device, queue)) = try_device() else {
            eprintln!("no GPU adapter — skipping scope waveform parity");
            return;
        };
        let n = 32u32;
        let px = signal_ramp(n as usize);
        let tex = upload(&device, &queue, &px, n, 1);
        let gpu = waveform_gpu(&device, &queue, &tex);
        let cpu = waveform_cpu(&px, n, 1);
        assert!(l1(&gpu.data, &cpu.data) <= 4, "waveform drift");
    }

    #[test]
    fn gpu_vectorscope_matches_cpu() {
        let Some((device, queue)) = try_device() else {
            eprintln!("no GPU adapter — skipping scope vectorscope parity");
            return;
        };
        // A block of pure red + a block of gray.
        let mut px = Vec::new();
        for _ in 0..64 {
            px.extend_from_slice(&[color::bt709_eotf(1.0), 0.0, 0.0, 1.0]);
        }
        for _ in 0..64 {
            let g = color::bt709_eotf(0.5);
            px.extend_from_slice(&[g, g, g, 1.0]);
        }
        let tex = upload(&device, &queue, &px, 128, 1);
        let gpu = vectorscope_gpu(&device, &queue, &tex);
        let cpu = vectorscope_cpu(&px, 128, 1);
        assert_eq!(gpu.data.iter().sum::<u32>(), 128);
        assert!(l1(&gpu.data, &cpu.data) <= 8, "vectorscope drift");
    }
}
