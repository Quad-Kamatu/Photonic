use bytemuck::{Pod, Zeroable};

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
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
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

pub fn create_blur_pipeline(
    device: &wgpu::Device,
    output_format: wgpu::TextureFormat,
    blur_bgl: &wgpu::BindGroupLayout,
    additive: bool,
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
    let blend = if additive {
        Some(wgpu::BlendState {
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
        })
    } else {
        Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING)
    };
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
