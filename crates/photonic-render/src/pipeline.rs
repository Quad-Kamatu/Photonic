use bytemuck::{Pod, Zeroable};
use photonic_core::layer::BlendMode;

/// The colour encoding of the document fill/blend render target — the single
/// source of truth for "what space does fixed-function blending run in".
///
/// It is an **sRGB** format on purpose: with an sRGB attachment the GPU blend
/// unit decodes each channel to linear, blends, then re-encodes, so separable
/// modes (Multiply/Screen) and partial-alpha src-over composite in linear
/// space. Both the headless export path (`headless::FORMAT`) and the windowed
/// document pass (`PhotonicRenderer::scene_format`) target this encoding, which
/// is what makes on-canvas rendering and exported output pixel-identical
/// (issue #145).
pub const SCENE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// The four separable blend modes implementable with fixed-function GPU blending.
///
/// Backdrop-read modes (Overlay, SoftLight, ColorDodge, ColorBurn) and the
/// non-separable HSL modes (Hue, Saturation, Color, Luminosity) cannot be
/// expressed as a `wgpu::BlendState` and require an offscreen-composite shader
/// pass — they are a documented follow-up (issue #17) and currently fall back to
/// normal alpha blending.
pub const SEPARABLE_BLEND_MODES: [BlendMode; 4] = [
    BlendMode::Multiply,
    BlendMode::Screen,
    BlendMode::Darken,
    BlendMode::Lighten,
];

/// Returns the fixed-function `BlendState` implementing `mode`, or `None` when
/// the mode is not expressible as fixed-function blending (Normal, or a
/// backdrop-read / HSL mode that needs a shader pass).
///
/// The colour-channel mappings are exact for opaque fills (alpha = 1) over an
/// opaque backdrop, which is the common case. Partial-alpha blended fills are
/// approximated; full Porter-Duff alpha compositing is a documented follow-up.
pub fn separable_blend_state(mode: BlendMode) -> Option<wgpu::BlendState> {
    use wgpu::{BlendComponent, BlendFactor, BlendOperation, BlendState};

    // Preserve the backdrop's alpha so a blended shape never punches a hole in
    // the coverage of whatever is underneath it.
    let keep_dst_alpha = BlendComponent {
        src_factor: BlendFactor::Zero,
        dst_factor: BlendFactor::One,
        operation: BlendOperation::Add,
    };
    let color = match mode {
        // Cs * Cb
        BlendMode::Multiply => BlendComponent {
            src_factor: BlendFactor::Dst,
            dst_factor: BlendFactor::Zero,
            operation: BlendOperation::Add,
        },
        // Cs + Cb - Cs*Cb  ==  Cs*(1 - Cb) + Cb
        BlendMode::Screen => BlendComponent {
            src_factor: BlendFactor::OneMinusDst,
            dst_factor: BlendFactor::One,
            operation: BlendOperation::Add,
        },
        // min(Cs, Cb) — factors are ignored for Min/Max operations.
        BlendMode::Darken => BlendComponent {
            src_factor: BlendFactor::One,
            dst_factor: BlendFactor::One,
            operation: BlendOperation::Min,
        },
        // max(Cs, Cb)
        BlendMode::Lighten => BlendComponent {
            src_factor: BlendFactor::One,
            dst_factor: BlendFactor::One,
            operation: BlendOperation::Max,
        },
        _ => return None,
    };
    Some(BlendState {
        color,
        alpha: keep_dst_alpha,
    })
}

/// A contiguous run of indices that share one blend mode, drawn in a single
/// `draw_indexed` call with the matching pipeline.
#[derive(Clone)]
pub(crate) struct DrawSegment {
    pub mode: BlendMode,
    /// Offset into the index buffer.
    pub start: u32,
    /// Number of indices in the run.
    pub count: u32,
}

/// Coalesce per-node `(mode, start, end)` index ranges (in draw order) into
/// contiguous draw runs, merging adjacent runs that share a mode. The common
/// all-`Normal` scene collapses to a single segment.
pub(crate) fn coalesce_segments(raw: Vec<(BlendMode, u32, u32)>) -> Vec<DrawSegment> {
    let mut segments: Vec<DrawSegment> = Vec::new();
    for (mode, start, end) in raw {
        if end == start {
            continue;
        }
        if let Some(last) = segments.last_mut() {
            if last.mode == mode && last.start + last.count == start {
                last.count += end - start;
                continue;
            }
        }
        segments.push(DrawSegment {
            mode,
            start,
            count: end - start,
        });
    }
    segments
}

/// Issue one `draw_indexed` per blend-mode run, selecting the matching blend
/// pipeline and falling back to `fill_pipeline` for modes without one. Runs are
/// in draw order, so fixed-function blends composite against the correct
/// backdrop. With no segments (e.g. first frame), draws all indices once with
/// `fill_pipeline`, preserving the original single-draw behaviour.
pub(crate) fn draw_segments<'a>(
    pass: &mut wgpu::RenderPass<'a>,
    segments: &[DrawSegment],
    blend_pipelines: &'a [(BlendMode, wgpu::RenderPipeline)],
    fill_pipeline: &'a wgpu::RenderPipeline,
    index_count: u32,
) {
    if segments.is_empty() {
        pass.set_pipeline(fill_pipeline);
        pass.draw_indexed(0..index_count, 0, 0..1);
        return;
    }
    for seg in segments {
        let pipeline = blend_pipelines
            .iter()
            .find(|(m, _)| *m == seg.mode)
            .map(|(_, p)| p)
            .unwrap_or(fill_pipeline);
        pass.set_pipeline(pipeline);
        pass.draw_indexed(seg.start..seg.start + seg.count, 0, 0..1);
    }
}

/// A single vertex: 2D position + RGBA colour.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 2],
    pub color: [f32; 4],
}

impl Vertex {
    pub const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &[
            wgpu::VertexAttribute {
                offset: 0,
                shader_location: 0,
                format: wgpu::VertexFormat::Float32x2,
            },
            wgpu::VertexAttribute {
                offset: 8,
                shader_location: 1,
                format: wgpu::VertexFormat::Float32x4,
            },
        ],
    };
}

/// Camera uniform buffer: column-major 4×4 matrix mapping document coords → NDC.
///
/// For a viewport with `pan_x, pan_y` offset and `zoom` scale on a `width × height` screen:
///   ndc_x = 2 * (doc_x * zoom + pan_x) / width  - 1
///   ndc_y = 1 - 2 * (doc_y * zoom + pan_y) / height
///
/// The matrix (column-major):
///   col 0: [2z/w,    0,    0, 0]
///   col 1: [0,    -2z/h,   0, 0]
///   col 2: [0,       0,    1, 0]
///   col 3: [2px/w-1, 1-2py/h, 0, 1]
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct CameraUniform {
    pub view_proj: [[f32; 4]; 4],
}

impl CameraUniform {
    pub fn from_viewport(pan_x: f64, pan_y: f64, zoom: f64, width: u32, height: u32) -> Self {
        let z = zoom as f32;
        let w = width as f32;
        let h = height as f32;
        let px = pan_x as f32;
        let py = pan_y as f32;
        Self {
            view_proj: [
                [2.0 * z / w, 0.0, 0.0, 0.0],                       // col 0
                [0.0, -2.0 * z / h, 0.0, 0.0],                      // col 1
                [0.0, 0.0, 1.0, 0.0],                               // col 2
                [2.0 * px / w - 1.0, 1.0 - 2.0 * py / h, 0.0, 1.0], // col 3
            ],
        }
    }
}

// ─── WGSL shader ─────────────────────────────────────────────────────────────

const FILL_SHADER: &str = r#"
struct Camera {
    view_proj: mat4x4<f32>,
}

@group(0) @binding(0) var<uniform> camera: Camera;

struct VertexIn {
    @location(0) position: vec2<f32>,
    @location(1) color:    vec4<f32>,
}

struct VertexOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0)       color:    vec4<f32>,
}

@vertex
fn vs_main(v: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.clip_pos = camera.view_proj * vec4<f32>(v.position, 0.0, 1.0);
    out.color    = v.color;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    return in.color;
}
"#;

// ─── Pipeline factory ─────────────────────────────────────────────────────────

pub fn create_camera_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("camera_bgl"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    })
}

pub fn create_fill_pipeline(
    device: &wgpu::Device,
    surface_format: wgpu::TextureFormat,
    camera_bgl: &wgpu::BindGroupLayout,
    sample_count: u32,
) -> wgpu::RenderPipeline {
    create_fill_pipeline_with_blend(
        device,
        surface_format,
        camera_bgl,
        sample_count,
        wgpu::BlendState::ALPHA_BLENDING,
    )
}

/// Like [`create_fill_pipeline`] but with a caller-supplied `blend` state, used
/// to build one pipeline variant per separable blend mode.
pub fn create_fill_pipeline_with_blend(
    device: &wgpu::Device,
    surface_format: wgpu::TextureFormat,
    camera_bgl: &wgpu::BindGroupLayout,
    sample_count: u32,
    blend: wgpu::BlendState,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("fill_shader"),
        source: wgpu::ShaderSource::Wgsl(FILL_SHADER.into()),
    });

    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("fill_layout"),
        bind_group_layouts: &[camera_bgl],
        push_constant_ranges: &[],
    });

    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("fill_pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: "vs_main",
            buffers: &[Vertex::LAYOUT],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: "fs_main",
            targets: &[Some(wgpu::ColorTargetState {
                format: surface_format,
                blend: Some(blend),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: None, // 2D — no culling
            polygon_mode: wgpu::PolygonMode::Fill,
            unclipped_depth: false,
            conservative: false,
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState {
            count: sample_count,
            mask: !0,
            alpha_to_coverage_enabled: false,
        },
        multiview: None,
        cache: None,
    })
}

// ─── Gaussian blur pipeline ────────────────────────────────────────────────────

pub const BLUR_SHADER: &str = r#"
struct BlurParams {
    sigma:      f32,
    horizontal: u32,
    _pad:       vec2<f32>,
}

@group(0) @binding(0) var t_in:  texture_2d<f32>;
@group(0) @binding(1) var s_in:  sampler;
@group(0) @binding(2) var<uniform> params: BlurParams;

struct VOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0)       uv:       vec2<f32>,
}

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
    var out: VOut;
    out.clip_pos = vec4<f32>(pos[vi], 0.0, 1.0);
    out.uv       = uvs[vi];
    return out;
}

@fragment
fn fs_blur(in: VOut) -> @location(0) vec4<f32> {
    let dim   = vec2<f32>(textureDimensions(t_in));
    let sigma = params.sigma;
    if sigma < 0.5 {
        return textureSample(t_in, s_in, in.uv);
    }
    let radius = min(i32(ceil(sigma * 3.0)), 128);
    var acc      = vec4<f32>(0.0);
    var w_total  = 0.0;
    let step_px  = select(vec2<f32>(0.0, 1.0 / dim.y),
                          vec2<f32>(1.0 / dim.x, 0.0),
                          params.horizontal != 0u);
    for (var i = -radius; i <= radius; i++) {
        let fi = f32(i);
        let w  = exp(-fi * fi / (2.0 * sigma * sigma));
        acc     += textureSample(t_in, s_in, in.uv + step_px * fi) * w;
        w_total += w;
    }
    return acc / w_total;
}
"#;

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct BlurParams {
    pub sigma: f32,
    pub horizontal: u32,
    pub _pad: [f32; 2],
}

pub fn create_blur_bgl(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("blur_bgl"),
        entries: &[
            // texture
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            // sampler
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            // uniform BlurParams
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    })
}

/// How a blur/composite pass blends its output into the target.
#[derive(Copy, Clone, Debug)]
pub enum BlurBlend {
    /// Premultiplied-alpha "over" — the glow halo default.
    Premultiplied,
    /// Additive (`src*srcAlpha + dst`) — brighten without erasing.
    Additive,
    /// Straight-alpha "over" — correct for compositing straight-color layers
    /// (the live-effects layer, whose textures hold non-premultiplied color).
    StraightAlpha,
}

pub fn create_blur_pipeline(
    device: &wgpu::Device,
    output_format: wgpu::TextureFormat,
    blur_bgl: &wgpu::BindGroupLayout,
    additive: bool,
) -> wgpu::RenderPipeline {
    create_blur_pipeline_with_blend(
        device,
        output_format,
        blur_bgl,
        if additive {
            BlurBlend::Additive
        } else {
            BlurBlend::Premultiplied
        },
    )
}

/// Like [`create_blur_pipeline`] but with an explicit blend mode.
pub fn create_blur_pipeline_with_blend(
    device: &wgpu::Device,
    output_format: wgpu::TextureFormat,
    blur_bgl: &wgpu::BindGroupLayout,
    blend_mode: BlurBlend,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("blur_shader"),
        source: wgpu::ShaderSource::Wgsl(BLUR_SHADER.into()),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("blur_layout"),
        bind_group_layouts: &[blur_bgl],
        push_constant_ranges: &[],
    });
    let blend = Some(match blend_mode {
        BlurBlend::Additive => wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::SrcAlpha,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        },
        BlurBlend::Premultiplied => wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING,
        BlurBlend::StraightAlpha => wgpu::BlendState::ALPHA_BLENDING,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("blur_pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: "vs_quad",
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: "fs_blur",
            targets: &[Some(wgpu::ColorTargetState {
                format: output_format,
                blend,
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(), // sample_count = 1
        multiview: None,
        cache: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use wgpu::{BlendFactor, BlendOperation};

    #[test]
    fn scene_format_is_srgb_for_linear_blending() {
        // The document blend target must be sRGB-encoded so fixed-function
        // blending decodes to linear, blends, and re-encodes (issue #145).
        assert!(
            SCENE_FORMAT.is_srgb(),
            "SCENE_FORMAT must be an sRGB format so blending runs in linear space",
        );
    }

    #[test]
    fn windowed_scene_format_derivation_is_srgb() {
        // The windowed renderer derives its scene format from the swapchain
        // surface format via `add_srgb_suffix()`. For every non-sRGB surface
        // format we accept, that derivation must yield a distinct sRGB format
        // (otherwise the document pass would silently blend in non-sRGB space).
        for surface in [
            wgpu::TextureFormat::Bgra8Unorm,
            wgpu::TextureFormat::Rgba8Unorm,
        ] {
            let scene = surface.add_srgb_suffix();
            assert!(
                scene.is_srgb(),
                "{surface:?}.add_srgb_suffix() must be sRGB"
            );
            assert_ne!(
                scene, surface,
                "{surface:?} must map to a distinct sRGB view format",
            );
        }
    }

    #[test]
    fn separable_modes_have_blend_states() {
        for mode in SEPARABLE_BLEND_MODES {
            assert!(
                separable_blend_state(mode).is_some(),
                "{mode:?} should map to a fixed-function blend state",
            );
        }
    }

    #[test]
    fn non_separable_modes_fall_back() {
        // Normal and shader-only modes have no fixed-function blend state.
        for mode in [
            BlendMode::Normal,
            BlendMode::Overlay,
            BlendMode::ColorDodge,
            BlendMode::Hue,
            BlendMode::Luminosity,
        ] {
            assert!(separable_blend_state(mode).is_none(), "{mode:?}");
        }
    }

    #[test]
    fn multiply_is_src_times_dst() {
        let bs = separable_blend_state(BlendMode::Multiply).unwrap();
        assert_eq!(bs.color.src_factor, BlendFactor::Dst);
        assert_eq!(bs.color.dst_factor, BlendFactor::Zero);
        assert_eq!(bs.color.operation, BlendOperation::Add);
    }

    #[test]
    fn darken_and_lighten_use_min_max() {
        assert_eq!(
            separable_blend_state(BlendMode::Darken)
                .unwrap()
                .color
                .operation,
            BlendOperation::Min,
        );
        assert_eq!(
            separable_blend_state(BlendMode::Lighten)
                .unwrap()
                .color
                .operation,
            BlendOperation::Max,
        );
    }

    #[test]
    fn alpha_channel_preserves_backdrop() {
        // Every separable mode must keep the backdrop's alpha (src*0 + dst*1).
        for mode in SEPARABLE_BLEND_MODES {
            let bs = separable_blend_state(mode).unwrap();
            assert_eq!(bs.alpha.src_factor, BlendFactor::Zero, "{mode:?}");
            assert_eq!(bs.alpha.dst_factor, BlendFactor::One, "{mode:?}");
        }
    }
}
