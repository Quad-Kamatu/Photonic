//! Lightfall — an animated fragment-shader background for the welcome screens.
//!
//! A WGSL port of the "Lightfall" effect: a raymarched curved space down which
//! soft, colored, glowing light-streaks rain, over a dark accent glow. It runs
//! as a real fullscreen GPU pass *behind* the egui welcome UI via an
//! [`egui_wgpu`] paint callback.
//!
//! The render pipeline is built once (it needs the surface format, only known at
//! startup) and stashed in the egui renderer's `callback_resources`; see
//! [`LightfallResources::new`], installed from the app's setup. Each frame the
//! [`Lightfall`] callback updates a small uniform buffer (time, resolution,
//! palette, tuning) and draws a fullscreen triangle.

use egui_wgpu::CallbackResources;

// ─── Uniforms (std140-friendly: everything 16-byte aligned) ─────────────────────

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    /// xy = resolution in pixels.
    resolution: [f32; 4],
    /// Palette colours (rgb in xyz), up to 8.
    colors: [[f32; 4]; 8],
    /// Background glow colour (rgb in xyz).
    bg_color: [f32; 4],
    /// time, speed, glow, density
    p0: [f32; 4],
    /// twinkle, zoom, bg_glow, opacity
    p1: [f32; 4],
    /// streak_width, streak_length, streak_count, color_count
    p2: [f32; 4],
}

fn hex_rgb(hex: &str) -> [f32; 3] {
    let h = hex.trim_start_matches('#');
    let p = |i: usize| {
        u8::from_str_radix(h.get(i..i + 2).unwrap_or("00"), 16).unwrap_or(0) as f32 / 255.0
    };
    [p(0), p(2), p(4)]
}

impl Uniforms {
    /// Photonic-tuned defaults (purple-leaning palette over a deep accent glow).
    fn build(width_px: f32, height_px: f32, time: f32) -> Self {
        const PALETTE: &[&str] = &["6E56CF", "9680F0", "A6C8FF", "FF9FFC"];
        let count = PALETTE.len();
        let mut colors = [[0.0f32; 4]; 8];
        for (i, slot) in colors.iter_mut().enumerate() {
            let c = hex_rgb(PALETTE[i.min(count - 1)]);
            *slot = [c[0], c[1], c[2], 1.0];
        }
        let bg = hex_rgb("2E2A78");
        Self {
            resolution: [width_px.max(1.0), height_px.max(1.0), 1.0, 0.0],
            colors,
            bg_color: [bg[0], bg[1], bg[2], 1.0],
            // time, speed, glow, density
            p0: [time, 0.5, 1.0, 0.6],
            // twinkle, zoom, bg_glow, opacity
            p1: [1.0, 3.0, 0.45, 1.0],
            // streak_width, streak_length, streak_count, color_count
            p2: [1.0, 1.0, 3.0, count as f32],
        }
    }
}

// ─── GPU resources (built once, stored in callback_resources) ───────────────────

pub struct LightfallResources {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniform_buf: wgpu::Buffer,
}

impl LightfallResources {
    /// Build the pipeline for the given surface format. Call once at startup and
    /// insert into `egui_renderer.callback_resources`.
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("lightfall_uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("lightfall_bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("lightfall_bg"),
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buf.as_entire_binding(),
            }],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("lightfall_shader"),
            source: wgpu::ShaderSource::Wgsl(WGSL.into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("lightfall_layout"),
            bind_group_layouts: &[&bgl],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("lightfall_pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });
        Self {
            pipeline,
            bind_group,
            uniform_buf,
        }
    }
}

// ─── The per-frame paint callback ───────────────────────────────────────────────

struct Lightfall {
    uniforms: Uniforms,
}

impl egui_wgpu::CallbackTrait for Lightfall {
    fn prepare(
        &self,
        _device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen_descriptor: &egui_wgpu::ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if let Some(res) = resources.get::<LightfallResources>() {
            queue.write_buffer(&res.uniform_buf, 0, bytemuck::bytes_of(&self.uniforms));
        }
        Vec::new()
    }

    fn paint(
        &self,
        info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        resources: &CallbackResources,
    ) {
        let Some(res) = resources.get::<LightfallResources>() else {
            return;
        };
        let vp = info.viewport_in_pixels();
        render_pass.set_viewport(
            vp.left_px as f32,
            vp.top_px as f32,
            vp.width_px.max(1) as f32,
            vp.height_px.max(1) as f32,
            0.0,
            1.0,
        );
        render_pass.set_pipeline(&res.pipeline);
        render_pass.set_bind_group(0, &res.bind_group, &[]);
        render_pass.draw(0..3, 0..1);
    }
}

/// Paint the Lightfall shader as an animated background filling `rect`. No-op
/// (transparent) if the pipeline wasn't installed (e.g. headless). Drive
/// `time` from `ctx.input(|i| i.time)` and request a repaint each frame.
pub fn paint(ui: &mut egui::Ui, rect: egui::Rect, time: f32) {
    let ppp = ui.ctx().pixels_per_point();
    let cb = Lightfall {
        uniforms: Uniforms::build(rect.width() * ppp, rect.height() * ppp, time),
    };
    ui.painter()
        .add(egui_wgpu::Callback::new_paint_callback(rect, cb));
}

// ─── WGSL (port of the GLSL Lightfall fragment shader) ──────────────────────────

const WGSL: &str = r#"
struct Uniforms {
    resolution : vec4<f32>,
    colors     : array<vec4<f32>, 8>,
    bg_color   : vec4<f32>,
    p0         : vec4<f32>, // time, speed, glow, density
    p1         : vec4<f32>, // twinkle, zoom, bg_glow, opacity
    p2         : vec4<f32>, // streak_width, streak_length, streak_count, color_count
};
@group(0) @binding(0) var<uniform> U : Uniforms;

struct VsOut {
    @builtin(position) pos : vec4<f32>,
    @location(0) uv : vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi : u32) -> VsOut {
    var verts = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 3.0, -1.0),
        vec2<f32>(-1.0,  3.0),
    );
    var out : VsOut;
    let p = verts[vi];
    out.pos = vec4<f32>(p, 0.0, 1.0);
    // 0..1 with y measured from the top (screen-like).
    out.uv = vec2<f32>(p.x * 0.5 + 0.5, 0.5 - p.y * 0.5);
    return out;
}

fn palette(h : f32) -> vec3<f32> {
    var count = i32(U.p2.w);
    if (count < 1) { count = 1; }
    let idx = i32(floor(clamp(h, 0.0, 0.999999) * f32(count)));
    return U.colors[clamp(idx, 0, 7)].xyz;
}

fn tanhv(x : vec3<f32>) -> vec3<f32> {
    let e = exp(-2.0 * x);
    return (vec3<f32>(1.0) - e) / (vec3<f32>(1.0) + e);
}

fn sceneC(frag : vec2<f32>, r : vec2<f32>, zoom : f32) -> vec2<f32> {
    let P = (frag + frag - r) / r.x;
    var z = 0.0;
    var d = 1.0e3;
    var O = vec4<f32>(0.0);
    for (var k = 0; k < 39; k = k + 1) {
        if (d <= 1.0e-4) { break; }
        O = z * normalize(vec4<f32>(P, zoom, 0.0)) - vec4<f32>(0.0, 4.0, 1.0, 0.0) / 4.5;
        d = 1.0 - sqrt(length(O * O));
        z = z + d;
    }
    return vec2<f32>(O.x, atan2(O.z, O.y));
}

@fragment
fn fs_main(in : VsOut) -> @location(0) vec4<f32> {
    let r = U.resolution.xy;
    var C = in.uv * r;

    let time   = U.p0.x;
    let speed  = U.p0.y;
    let glow   = U.p0.z;
    let density = U.p0.w;
    let twinkle = U.p1.x;
    let zoom   = U.p1.y;
    let bg_glow = U.p1.z;
    let opacity = U.p1.w;
    let streak_width  = U.p2.x;
    let streak_length = U.p2.y;
    let streak_count  = i32(U.p2.z);

    let TAU = 6.28318530718;
    let uv0 = (C + C - r) / r.x;
    let T = 0.1 * time * speed + 9.0;
    let angRings = max(1.0, floor(TAU * max(density, 0.05) + 0.5));
    let Y = vec2<f32>(5.0e-3, TAU / angRings);

    let c0  = sceneC(C, r, zoom);
    let cdx = sceneC(C + vec2<f32>(1.0, 0.0), r, zoom);
    let cdy = sceneC(C + vec2<f32>(0.0, 1.0), r, zoom);
    var dCx = cdx - c0;
    var dCy = cdy - c0;
    dCx.y = dCx.y - TAU * floor(dCx.y / TAU + 0.5);
    dCy.y = dCy.y - TAU * floor(dCy.y / TAU + 0.5);
    let fw = abs(dCx) + abs(dCy);
    C = c0;

    let P = vec2<f32>(2.0, 1.0) * uv0 - (r / r.x) * vec2<f32>(0.0, 1.0);
    var O = vec4<f32>(U.bg_color.xyz * 90.0 * bg_glow / (1.0e3 * dot(P, P) + 6.0), 0.0);

    let zr = 5.0e-4 * streak_width;
    let rr = vec2<f32>(max(length(fw), 1.0e-5));
    let tail = 19.0 / max(streak_length, 0.05);

    for (var m = 0; m < 16; m = m + 1) {
        if (m >= streak_count) { break; }
        let jf = f32(m) + 1.0;
        let ic = fract(sin(dot(vec2<f32>(jf, floor(C.x / Y.x + 0.5)), vec2<f32>(7.0, 11.0)) * 73.0));
        var Pp = C - (T + T * ic) * vec2<f32>(0.0, 1.0);
        Pp = Pp - floor(Pp / Y + 0.5) * Y;
        let h = fract(8663.0 * ic);
        let col = palette(h);
        let weight = mix(1.5, 1.0 + sin(T + 7.0 * h + 4.0), twinkle);
        let inner = vec2<f32>(length(max(Pp, vec2<f32>(-1.0, 0.0))), length(Pp) - zr) - vec2<f32>(zr);
        let sm = vec2<f32>(1.0) - smoothstep(-rr, rr, inner);
        O = vec4<f32>(O.xyz + dot(sm, vec2<f32>(exp(tail * Pp.y), 3.0)) * col * weight, O.w);
        C.x = C.x + Y.x / 8.0;
    }

    let colr = sqrt(tanhv(max(O.xyz * glow - vec3<f32>(0.04, 0.08, 0.02), vec3<f32>(0.0))));
    return vec4<f32>(colr, opacity);
}
"#;
