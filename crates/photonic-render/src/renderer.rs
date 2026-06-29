use crate::{
    canvas::CanvasView,
    pipeline::{
        coalesce_segments, create_blur_bgl, create_blur_pipeline, create_camera_bind_group_layout,
        create_fill_pipeline, create_fill_pipeline_with_blend, draw_segments,
        separable_blend_state, BlurParams, CameraUniform, DrawSegment, Vertex,
        SEPARABLE_BLEND_MODES,
    },
    tessellator::{tessellate_fill, tessellate_stroke, tessellate_stroke_variable},
};
use glyphon::{
    Attrs, Buffer, Cache, Color as GlyphonColor, Family, FontSystem, Metrics, Resolution, Shaping,
    Style as GlyphonStyle, SwashCache, TextArea, TextAtlas, TextBounds, TextRenderer, Viewport,
    Weight,
};
use image::{ImageBuffer, Rgba};
use photonic_core::{
    document::Document,
    layer::BlendMode,
    node::SceneNodeKind,
    path::PathData,
    style::{FillKind, StrokeAlign},
};
use std::sync::Arc;
use tokio::sync::oneshot;
use tokio::sync::Mutex;
use wgpu::util::DeviceExt;
use winit::window::Window;

// ─── Background colour (deep violet-dark canvas surround) ─────────────────────
// Linear values for sRGB target #0D0D14 (r:13 g:13 b:20).
const BG: wgpu::Color = wgpu::Color {
    r: 0.002,
    g: 0.002,
    b: 0.005,
    a: 1.0,
};
const MSAA_SAMPLES: u32 = 4;

// ─── Frame handle ─────────────────────────────────────────────────────────────

/// Owns the surface frame for one in-flight render.
///
/// After `begin_frame` records the document pass, callers may record
/// additional passes (e.g. egui) into `encoder` before handing the handle
/// back to `finish_frame`.
pub struct FrameHandle {
    pub surface_texture: wgpu::SurfaceTexture,
    pub view: wgpu::TextureView,
    pub encoder: wgpu::CommandEncoder,
}

// ─── Main renderer ───────────────────────────────────────────────────────────

pub struct PhotonicRenderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface_config: wgpu::SurfaceConfiguration,
    surface_format: wgpu::TextureFormat,

    fill_pipeline: wgpu::RenderPipeline,
    /// One fill-pipeline variant per separable blend mode (Multiply/Screen/
    /// Darken/Lighten), built at `MSAA_SAMPLES`. Modes without an entry fall back
    /// to `fill_pipeline` (normal alpha blending).
    blend_pipelines: Vec<(BlendMode, wgpu::RenderPipeline)>,
    camera_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,

    msaa_texture: wgpu::Texture,
    msaa_view: wgpu::TextureView,

    pub view: CanvasView,
    document: Arc<Mutex<Document>>,
    capture_rx: std::sync::mpsc::Receiver<oneshot::Sender<Vec<u8>>>,

    width: u32,
    height: u32,

    /// Last successfully built geometry — returned as-is when the doc lock is contended.
    cached_vertices: Vec<Vertex>,
    cached_indices: Vec<u32>,
    /// Per-blend-mode index ranges for the current frame, in draw order. Read by
    /// `record_document_pass` to issue one draw call per contiguous run.
    draw_segments: Vec<DrawSegment>,
    cached_segments: Vec<DrawSegment>,

    // ── Text rendering (glyphon) ───────────────────────────────────────────────
    font_system: FontSystem,
    swash_cache: SwashCache,
    /// Kept alive so `text_atlas` and `text_viewport` remain valid.
    #[allow(dead_code)]
    text_glyph_cache: Cache,
    text_atlas: TextAtlas,
    text_renderer: TextRenderer,
    text_viewport: Viewport,
    /// Text nodes collected during `build_geometry` for the current frame.
    pending_texts: Vec<TextSnapshot>,
    /// Text-on-path glyph outlines (document space) + RGBA fill colour, built
    /// during `build_geometry` and tessellated into the fill geometry. Rendered
    /// as vector fills because glyphon cannot rotate glyphs along a curve.
    pending_path_text: Vec<(Vec<PathData>, [f32; 4])>,

    // ── Gaussian glow blur ────────────────────────────────────────────────────
    fill_pipeline_1spp: wgpu::RenderPipeline, // sample_count=1 for offscreen silhouette
    blur_pipeline_h: wgpu::RenderPipeline,    // H blur (alpha-blend output)
    blur_pipeline_v: wgpu::RenderPipeline,    // V blur (additive composite to surface)
    blur_bgl: wgpu::BindGroupLayout,
    blur_sampler: wgpu::Sampler,
    glow_tex_a: wgpu::Texture, // silhouette & V-blur source
    glow_tex_a_view: wgpu::TextureView,
    glow_tex_b: wgpu::Texture, // H-blur output
    glow_tex_b_view: wgpu::TextureView,
    /// Gaussian glow jobs built each frame by build_geometry, consumed by render_gaussian_glow_pass.
    pending_gaussian_glows: Vec<GaussianGlowJob>,
}

/// Screen-space snapshot of one text node, ready for glyphon.
struct TextSnapshot {
    content: String,
    font_family: String,
    /// Font size already scaled by canvas zoom (physical pixels).
    font_size: f32,
    /// Line height multiplier (default: 1.2).
    line_height_mul: f32,
    /// Font weight (100–900).
    font_weight: u16,
    /// Font style: 0=Normal, 1=Italic, 2=Oblique.
    font_style: u8,
    /// RGBA 0-255 fill colour.
    color: [u8; 4],
    screen_x: f32,
    screen_y: f32,
}

struct GaussianGlowJob {
    /// Fill geometry coloured with the glow tint (rendered to offscreen silhouette texture).
    verts: Vec<Vertex>,
    idxs: Vec<u32>,
    /// Blur sigma in screen pixels (radius_doc_units × zoom).
    sigma_px: f32,
}

impl PhotonicRenderer {
    pub async fn new(
        window: Arc<Window>,
        document: Arc<Mutex<Document>>,
        capture_rx: std::sync::mpsc::Receiver<oneshot::Sender<Vec<u8>>>,
    ) -> Self {
        let size = window.inner_size();
        let width = size.width.max(1);
        let height = size.height.max(1);

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        let surface = instance
            .create_surface(window)
            .expect("Failed to create wgpu surface");

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .expect("No suitable GPU adapter found");

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("photonic_device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                    memory_hints: Default::default(),
                },
                None,
            )
            .await
            .expect("Failed to create wgpu device");

        let caps = surface.get_capabilities(&adapter);
        // Prefer a non-sRGB linear format so egui doesn't double-gamma-correct.
        // Bgra8Unorm / Rgba8Unorm are the formats egui explicitly recommends.
        let surface_format = caps
            .formats
            .iter()
            .find(|f| {
                matches!(
                    f,
                    wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Rgba8Unorm
                )
            })
            .copied()
            .unwrap_or(caps.formats[0]);

        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width,
            height,
            present_mode: wgpu::PresentMode::AutoVsync,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &surface_config);

        // Camera bind group
        let camera_bgl = create_camera_bind_group_layout(&device);
        let initial_cam = CameraUniform::from_viewport(0.0, 0.0, 1.0, width, height);
        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("camera_buf"),
            contents: bytemuck::bytes_of(&initial_cam),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("camera_bg"),
            layout: &camera_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });

        // Log any uncaptured GPU errors (device lost, OOM, etc.) so they appear
        // in the log file even if they don't trigger a Rust panic.
        device.on_uncaptured_error(Box::new(|e| {
            tracing::error!("wgpu uncaptured error: {:?}", e);
        }));

        let fill_pipeline =
            create_fill_pipeline(&device, surface_format, &camera_bgl, MSAA_SAMPLES);
        // One pipeline variant per separable blend mode, sharing the fill shader.
        let blend_pipelines: Vec<(BlendMode, wgpu::RenderPipeline)> = SEPARABLE_BLEND_MODES
            .iter()
            .filter_map(|&mode| {
                separable_blend_state(mode).map(|blend| {
                    (
                        mode,
                        create_fill_pipeline_with_blend(
                            &device,
                            surface_format,
                            &camera_bgl,
                            MSAA_SAMPLES,
                            blend,
                        ),
                    )
                })
            })
            .collect();
        let (msaa_texture, msaa_view) = create_msaa_texture(&device, surface_format, width, height);

        let blur_bgl = create_blur_bgl(&device);
        let fill_pipeline_1spp = create_fill_pipeline(&device, surface_format, &camera_bgl, 1);
        let blur_pipeline_h = create_blur_pipeline(&device, surface_format, &blur_bgl, false);
        let blur_pipeline_v = create_blur_pipeline(&device, surface_format, &blur_bgl, true);
        let blur_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("blur_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let (glow_tex_a, glow_tex_a_view, glow_tex_b, glow_tex_b_view) =
            create_glow_textures(&device, surface_format, width, height);

        // Fit the view to the document artboard
        let mut view = CanvasView::new(width, height);
        {
            let doc = document.blocking_lock();
            view.fit_to_rect(0.0, 0.0, doc.width, doc.height);
        }

        // ── Glyphon text rendering ─────────────────────────────────────────────
        let font_system = FontSystem::new();
        let swash_cache = SwashCache::new();
        let text_glyph_cache = Cache::new(&device);
        let text_viewport = Viewport::new(&device, &text_glyph_cache);
        let mut text_atlas = TextAtlas::new(&device, &queue, &text_glyph_cache, surface_format);
        let text_renderer = TextRenderer::new(
            &mut text_atlas,
            &device,
            wgpu::MultisampleState::default(),
            None,
        );

        Self {
            surface,
            device,
            queue,
            surface_config,
            surface_format,
            fill_pipeline,
            blend_pipelines,
            camera_buffer,
            camera_bind_group,
            msaa_texture,
            msaa_view,
            view,
            document,
            capture_rx,
            width,
            height,
            cached_vertices: Vec::new(),
            cached_indices: Vec::new(),
            draw_segments: Vec::new(),
            cached_segments: Vec::new(),
            font_system,
            swash_cache,
            text_glyph_cache,
            text_atlas,
            text_renderer,
            text_viewport,
            pending_texts: Vec::new(),
            pending_path_text: Vec::new(),
            fill_pipeline_1spp,
            blur_pipeline_h,
            blur_pipeline_v,
            blur_bgl,
            blur_sampler,
            glow_tex_a,
            glow_tex_a_view,
            glow_tex_b,
            glow_tex_b_view,
            pending_gaussian_glows: Vec::new(),
        }
    }

    // ── Public accessors ──────────────────────────────────────────────────────

    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    pub fn surface_format(&self) -> wgpu::TextureFormat {
        self.surface_format
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Measure the rendered dimensions of a text string using glyphon layout.
    /// Returns `(width, height)` in document units (same coordinate space as `font_size`).
    pub fn measure_text(&mut self, content: &str, font_family: &str, font_size: f64) -> (f64, f64) {
        let fs = font_size as f32;
        let line_height = fs * 1.2;
        let mut buf = Buffer::new(&mut self.font_system, Metrics::new(fs, line_height));
        buf.set_size(&mut self.font_system, None, None);
        let attrs = Attrs::new().family(Family::Name(font_family));
        buf.set_text(&mut self.font_system, content, attrs, Shaping::Basic);
        buf.shape_until_scroll(&mut self.font_system, false);
        let width = buf.layout_runs().map(|r| r.line_w).fold(0.0_f32, f32::max);
        let height = buf
            .layout_runs()
            .map(|r| r.line_height)
            .fold(0.0_f32, |a, h| a + h);
        let height = if height == 0.0 { line_height } else { height };
        (width as f64, height as f64)
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.width = width;
        self.height = height;
        self.view.screen_width = width;
        self.view.screen_height = height;
        self.surface_config.width = width;
        self.surface_config.height = height;
        self.surface.configure(&self.device, &self.surface_config);
        let (msaa_texture, msaa_view) =
            create_msaa_texture(&self.device, self.surface_format, width, height);
        self.msaa_texture = msaa_texture;
        self.msaa_view = msaa_view;
        let (glow_tex_a, glow_tex_a_view, glow_tex_b, glow_tex_b_view) =
            create_glow_textures(&self.device, self.surface_format, width, height);
        self.glow_tex_a = glow_tex_a;
        self.glow_tex_a_view = glow_tex_a_view;
        self.glow_tex_b = glow_tex_b;
        self.glow_tex_b_view = glow_tex_b_view;
    }

    // ── Frame management ──────────────────────────────────────────────────────

    /// Push camera uniforms and build vertex/index data for this frame.
    pub fn update(&mut self) -> (Vec<Vertex>, Vec<u32>) {
        self.push_camera();
        self.build_geometry()
    }

    /// Acquire the swapchain frame and record the document render pass into
    /// `FrameHandle::encoder`. Returns `None` when the surface is lost/timeout
    /// (the surface will be reconfigured automatically).
    ///
    /// Callers may append further render passes (e.g. egui) to `handle.encoder`
    /// before calling `finish_frame`.
    pub fn begin_frame(&self, vertices: &[Vertex], indices: &[u32]) -> Option<FrameHandle> {
        let surface_texture = match self.surface.get_current_texture() {
            Ok(t) => t,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                self.surface.configure(&self.device, &self.surface_config);
                return None;
            }
            Err(e) => {
                tracing::warn!("surface error: {:?}", e);
                return None;
            }
        };
        let view = surface_texture.texture.create_view(&Default::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame_encoder"),
            });
        self.record_document_pass(&mut encoder, &self.msaa_view, &view, vertices, indices);
        Some(FrameHandle {
            surface_texture,
            view,
            encoder,
        })
    }

    /// Submit the frame encoder and present the surface texture.
    pub fn finish_frame(&self, handle: FrameHandle) {
        self.queue.submit([handle.encoder.finish()]);
        handle.surface_texture.present();
    }

    /// Render any text nodes collected during `update()` into the frame.
    ///
    /// Call this after `begin_frame` and before the egui pass.
    /// No-op if there are no text nodes this frame.
    pub fn render_text_pass(&mut self, frame: &mut FrameHandle) {
        if self.pending_texts.is_empty() {
            return;
        }

        // Update the glyphon viewport to the current screen resolution.
        self.text_viewport.update(
            &self.queue,
            Resolution {
                width: self.width,
                height: self.height,
            },
        );

        // Build one glyphon Buffer per text node and collect TextAreas.
        let snapshots = &self.pending_texts;
        let mut buffers: Vec<Buffer> = Vec::with_capacity(snapshots.len());
        for snap in snapshots.iter() {
            let font_size = snap.font_size.max(1.0);
            let line_height = font_size * 1.2;
            let mut buf = Buffer::new(&mut self.font_system, Metrics::new(font_size, line_height));
            buf.set_size(&mut self.font_system, None, None);
            let glyph_style = match snap.font_style {
                1 => GlyphonStyle::Italic,
                2 => GlyphonStyle::Oblique,
                _ => GlyphonStyle::Normal,
            };
            let attrs = Attrs::new()
                .family(Family::Name(&snap.font_family))
                .weight(Weight(snap.font_weight))
                .style(glyph_style);
            buf.set_text(&mut self.font_system, &snap.content, attrs, Shaping::Basic);
            buf.shape_until_scroll(&mut self.font_system, false);
            buffers.push(buf);
        }

        let text_areas: Vec<TextArea> = snapshots
            .iter()
            .zip(buffers.iter())
            .map(|(snap, buf)| TextArea {
                buffer: buf,
                left: snap.screen_x,
                top: snap.screen_y,
                scale: 1.0,
                bounds: TextBounds {
                    left: i32::MIN,
                    top: i32::MIN,
                    right: i32::MAX,
                    bottom: i32::MAX,
                },
                default_color: GlyphonColor::rgba(
                    snap.color[0],
                    snap.color[1],
                    snap.color[2],
                    snap.color[3],
                ),
                custom_glyphs: &[],
            })
            .collect();

        if let Err(e) = self.text_renderer.prepare(
            &self.device,
            &self.queue,
            &mut self.font_system,
            &mut self.text_atlas,
            &self.text_viewport,
            text_areas,
            &mut self.swash_cache,
        ) {
            tracing::warn!("glyphon prepare failed: {:?}", e);
            return;
        }

        {
            let mut pass = frame
                .encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("text_pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &frame.view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                })
                .forget_lifetime();

            if let Err(e) =
                self.text_renderer
                    .render(&self.text_atlas, &self.text_viewport, &mut pass)
            {
                tracing::warn!("glyphon render failed: {:?}", e);
            }
        }

        self.text_atlas.trim();
    }

    /// Execute all pending Gaussian glow jobs collected during `update()`.
    ///
    /// Must be called **after** `begin_frame` (so the scene is already rendered on the
    /// surface texture) and **before** `finish_frame`.
    /// Uses an additive blend so the glow brightens the scene without erasing the fill.
    pub fn render_gaussian_glow_pass(&mut self, frame: &mut FrameHandle) {
        if self.pending_gaussian_glows.is_empty() {
            return;
        }

        let jobs: Vec<GaussianGlowJob> = std::mem::take(&mut self.pending_gaussian_glows);

        for job in &jobs {
            if job.verts.is_empty() {
                continue;
            }

            let sigma = job.sigma_px.max(0.5);

            // ── Pass A: render fill silhouette (glow tint colour) → glow_tex_a ──
            {
                let vbuf = self
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("gglow_vbuf"),
                        contents: bytemuck::cast_slice(&job.verts),
                        usage: wgpu::BufferUsages::VERTEX,
                    });
                let ibuf = self
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("gglow_ibuf"),
                        contents: bytemuck::cast_slice(&job.idxs),
                        usage: wgpu::BufferUsages::INDEX,
                    });
                let mut pass = frame
                    .encoder
                    .begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("gglow_shape"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &self.glow_tex_a_view,
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
                pass.set_pipeline(&self.fill_pipeline_1spp);
                pass.set_bind_group(0, &self.camera_bind_group, &[]);
                pass.set_vertex_buffer(0, vbuf.slice(..));
                pass.set_index_buffer(ibuf.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..job.idxs.len() as u32, 0, 0..1);
            }

            // Helper: create blur bind group for a given source texture view.
            let make_blur_bg = |src_view: &wgpu::TextureView, sigma: f32, horizontal: bool| {
                let params = BlurParams {
                    sigma,
                    horizontal: horizontal as u32,
                    _pad: [0.0; 2],
                };
                let params_buf =
                    self.device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("blur_params"),
                            contents: bytemuck::bytes_of(&params),
                            usage: wgpu::BufferUsages::UNIFORM,
                        });
                self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("blur_bg"),
                    layout: &self.blur_bgl,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(src_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&self.blur_sampler),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: params_buf.as_entire_binding(),
                        },
                    ],
                })
            };

            // ── Pass B: horizontal blur  glow_tex_a → glow_tex_b ────────────────
            {
                let bg = make_blur_bg(&self.glow_tex_a_view, sigma, true);
                let mut pass = frame
                    .encoder
                    .begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("gglow_blur_h"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &self.glow_tex_b_view,
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
                pass.set_pipeline(&self.blur_pipeline_h);
                pass.set_bind_group(0, &bg, &[]);
                pass.draw(0..6, 0..1);
            }

            // ── Pass C: vertical blur  glow_tex_b → surface (additive) ──────────
            {
                let bg = make_blur_bg(&self.glow_tex_b_view, sigma, false);
                let mut pass = frame
                    .encoder
                    .begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("gglow_blur_v"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &frame.view,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Load,
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                    });
                pass.set_pipeline(&self.blur_pipeline_h);
                pass.set_bind_group(0, &bg, &[]);
                pass.draw(0..6, 0..1);
            }
        }
    }

    /// Poll the capture channel and service any pending screenshot requests.
    pub fn service_captures(&mut self, vertices: &[Vertex], indices: &[u32]) {
        while let Ok(reply_tx) = self.capture_rx.try_recv() {
            tracing::info!(
                "render: capture_png starting ({}x{})",
                self.width,
                self.height
            );
            let png = self.capture_png(vertices, indices);
            tracing::info!(
                "render: capture_png done ({} bytes) — sending reply",
                png.len()
            );
            let _ = reply_tx.send(png);
            tracing::info!("render: capture reply sent — render loop resuming");
        }
    }

    /// Convenience: full render loop without an egui overlay.
    pub fn render(&mut self) {
        let (verts, idxs) = self.update(); // already &mut self
        if let Some(frame) = self.begin_frame(&verts, &idxs) {
            self.finish_frame(frame);
        }
        self.service_captures(&verts, &idxs);
    }

    // ── Private helpers ───────────────────────────────────────────────────────

    fn push_camera(&self) {
        let cam = CameraUniform::from_viewport(
            self.view.pan_x,
            self.view.pan_y,
            self.view.zoom,
            self.width,
            self.height,
        );
        self.queue
            .write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&cam));
    }

    /// Walk the document and build one combined vertex + index buffer.
    ///
    /// The doc lock is held only long enough to clone the node data needed for
    /// tessellation. All actual tessellation happens after the lock is released,
    /// so MCP handlers and the egui closure are never blocked by tessellation time.
    ///
    /// If the doc lock is currently held by another thread (e.g. an MCP handler
    /// that outlived its session), we return the cached geometry from the previous
    /// frame instead of blocking indefinitely.
    fn build_geometry(&mut self) -> (Vec<Vertex>, Vec<u32>) {
        // ── Read phase: clone what we need, release doc lock immediately ──────
        struct NodeSnapshot {
            matrix: [f64; 6],
            fill_enabled: bool,
            fill_kind: FillKind,
            fill_opacity: f32,
            node_opacity: f32,
            fill_is_none: bool,
            stroke_enabled: bool,
            stroke_color: [f32; 4],
            stroke_width: f32,
            stroke_cap: photonic_core::style::LineCap,
            stroke_join: photonic_core::style::LineJoin,
            stroke_miter: f32,
            stroke_align: StrokeAlign,
            /// Resolved variable-width profile samples (uniform-t along the path).
            /// `None` → render with the uniform `stroke_width`.
            stroke_widths: Option<Vec<f64>>,
            path_data: photonic_core::path::PathData,
            /// (r, g, b, a, opacity, size) + join — None when disabled.
            outer_glow: Option<([f32; 6], photonic_core::style::LineJoin)>,
            inner_glow: Option<([f32; 6], photonic_core::style::LineJoin)>,
            gaussian_glow: Option<([f32; 4], f32)>, // ([r,g,b,a*opacity], radius_doc)
            is_compound: bool,
            arrowhead_start: photonic_core::style::ArrowheadStyle,
            arrowhead_end: photonic_core::style::ArrowheadStyle,
            blend_mode: BlendMode,
        }

        let (artboard_w, artboard_h, artboards, nodes): (f32, f32, Vec<[f32; 4]>, Vec<NodeSnapshot>) = {
            // try_lock — never block; return cached geometry if lock is contended.
            let doc = match self.document.try_lock() {
                Ok(g) => g,
                Err(_) => {
                    tracing::debug!("render: doc lock contended — reusing cached geometry");
                    self.draw_segments = self.cached_segments.clone();
                    return (self.cached_vertices.clone(), self.cached_indices.clone());
                }
            };
            let w = doc.width as f32;
            let h = doc.height as f32;
            // Snapshot each artboard rect (x, y, w, h) in document space.
            let artboards: Vec<[f32; 4]> = doc
                .artboards
                .iter()
                .map(|a| [a.x as f32, a.y as f32, a.width as f32, a.height as f32])
                .collect();

            // Single pass: snapshot text nodes for glyphon and path nodes for
            // tessellation in one traversal, halving scene graph walk cost per frame.
            let zoom = self.view.zoom;
            let pan_x = self.view.pan_x;
            let pan_y = self.view.pan_y;
            self.pending_texts.clear();
            self.pending_path_text.clear();
            let mut nodes: Vec<NodeSnapshot> = Vec::new();
            for node in doc.nodes_in_draw_order() {
                // Symbol instances render from the *current* master so master
                // edits and per-instance overrides take effect live.
                let resolved = doc.resolve_render_node(node);
                let node = resolved.as_ref();
                match &node.kind {
                    SceneNodeKind::Text(text_node) => {
                        // Text-on-path: render glyph outlines along the spine as
                        // vector fills instead of flat glyphon text.
                        if let Some(spine_node) =
                            text_node.path_spine_id.and_then(|id| doc.get_node(&id))
                        {
                            if let SceneNodeKind::Path(spine) = &spine_node.kind {
                                let mut bez = spine.path_data.to_bez_path();
                                bez.apply_affine(kurbo::Affine::new(spine_node.transform.matrix));
                                let spine_doc = PathData::from_bez_path(&bez);

                                let opacity = text_node.fill.opacity * node.opacity;
                                let rgba = match &text_node.fill.kind {
                                    FillKind::Solid(c) => [c.r, c.g, c.b, c.a * opacity],
                                    _ => [0.0, 0.0, 0.0, opacity],
                                };
                                let params = crate::text_path::TextOnPathParams {
                                    content: &text_node.content,
                                    font_family: &text_node.font_family,
                                    font_size: text_node.font_size,
                                    font_weight: text_node.font_weight,
                                    font_style: text_node.font_style,
                                    line_height: text_node.line_height,
                                    letter_spacing: text_node.letter_spacing,
                                    align: text_node.align,
                                    path_offset: text_node.path_offset,
                                };
                                let glyphs = crate::text_path::layout_text_on_path(
                                    &mut self.font_system,
                                    &params,
                                    &spine_doc,
                                );
                                if !glyphs.is_empty() {
                                    self.pending_path_text.push((glyphs, rgba));
                                }
                                continue; // handled — skip flat glyphon text
                            }
                        }
                        let (doc_x, doc_y) = node.transform.apply(0.0, 0.0);
                        let screen_x = (doc_x * zoom + pan_x) as f32;
                        let screen_y = (doc_y * zoom + pan_y) as f32;
                        let opacity = text_node.fill.opacity * node.opacity;
                        let color = match &text_node.fill.kind {
                            FillKind::Solid(c) => [
                                (c.r * 255.0) as u8,
                                (c.g * 255.0) as u8,
                                (c.b * 255.0) as u8,
                                (c.a * opacity * 255.0) as u8,
                            ],
                            _ => [0, 0, 0, (opacity * 255.0) as u8],
                        };
                        let font_style_u8 = match text_node.font_style {
                            photonic_core::node::FontStyle::Normal => 0,
                            photonic_core::node::FontStyle::Italic => 1,
                            photonic_core::node::FontStyle::Oblique => 2,
                        };
                        self.pending_texts.push(TextSnapshot {
                            content: text_node.content.clone(),
                            font_family: text_node.font_family.clone(),
                            font_size: (text_node.font_size * zoom) as f32,
                            line_height_mul: text_node.line_height as f32,
                            font_weight: text_node.font_weight,
                            font_style: font_style_u8,
                            color,
                            screen_x,
                            screen_y,
                        });
                    }
                    SceneNodeKind::Path(path_node) => {
                        let sc = &path_node.stroke;
                        let stroke_alpha = sc.color.a * sc.opacity * node.opacity;
                        nodes.push(NodeSnapshot {
                            matrix: node.transform.matrix,
                            fill_enabled: path_node.fill.enabled,
                            fill_kind: path_node.fill.kind.clone(),
                            fill_opacity: path_node.fill.opacity,
                            node_opacity: node.opacity,
                            fill_is_none: matches!(path_node.fill.kind, FillKind::None),
                            stroke_enabled: sc.enabled && sc.width > 0.0,
                            stroke_color: [sc.color.r, sc.color.g, sc.color.b, stroke_alpha],
                            stroke_width: sc.width as f32,
                            stroke_cap: sc.line_cap,
                            stroke_join: sc.line_join,
                            stroke_miter: sc.miter_limit as f32,
                            stroke_align: sc.align,
                            stroke_widths: sc.width_profile_id.and_then(|id| {
                                doc.width_profiles
                                    .iter()
                                    .find(|p| p.id == id)
                                    .filter(|p| p.widths.len() >= 2)
                                    .map(|p| p.widths.clone())
                            }),
                            path_data: path_node.path_data.clone(),
                            is_compound: path_node.is_compound,
                            arrowhead_start: sc.arrowhead_start,
                            arrowhead_end: sc.arrowhead_end,
                            blend_mode: node.blend_mode,
                            outer_glow: if node.outer_glow.enabled {
                                let c = &node.outer_glow.color;
                                Some((
                                    [
                                        c.r,
                                        c.g,
                                        c.b,
                                        c.a,
                                        node.outer_glow.opacity,
                                        node.outer_glow.size,
                                    ],
                                    node.outer_glow.join,
                                ))
                            } else {
                                None
                            },
                            inner_glow: if node.inner_glow.enabled {
                                let c = &node.inner_glow.color;
                                Some((
                                    [
                                        c.r,
                                        c.g,
                                        c.b,
                                        c.a,
                                        node.inner_glow.opacity,
                                        node.inner_glow.size,
                                    ],
                                    node.inner_glow.join,
                                ))
                            } else {
                                None
                            },
                            gaussian_glow: if node.gaussian_glow.enabled {
                                let c = &node.gaussian_glow.color;
                                let a = c.a * node.gaussian_glow.opacity * node.opacity;
                                Some(([c.r, c.g, c.b, a], node.gaussian_glow.radius))
                            } else {
                                None
                            },
                        });
                    }
                    _ => {} // Group nodes and future kinds: no GPU geometry of their own
                }
            }
            (w, h, artboards, nodes)
        }; // doc lock released here

        // ── Tessellate phase: all CPU work happens with no locks held ─────────
        self.pending_gaussian_glows.clear();
        let mut verts: Vec<Vertex> = Vec::new();
        let mut idxs: Vec<u32> = Vec::new();

        // White artboard rectangles — one per artboard (spatial multi-artboard
        // model). Falls back to the full document bounds when none are present.
        let white = [1.0f32, 1.0, 1.0, 1.0];
        let mut boards = artboards;
        if boards.is_empty() {
            boards.push([0.0, 0.0, artboard_w, artboard_h]);
        }
        for b in &boards {
            let (x0, y0, x1, y1) = (b[0], b[1], b[0] + b[2], b[1] + b[3]);
            let base = verts.len() as u32;
            verts.extend_from_slice(&[
                Vertex {
                    position: [x0, y0],
                    color: white,
                },
                Vertex {
                    position: [x1, y0],
                    color: white,
                },
                Vertex {
                    position: [x1, y1],
                    color: white,
                },
                Vertex {
                    position: [x0, y1],
                    color: white,
                },
            ]);
            idxs.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }

        // Helpers for appending tessellated meshes to the vertex/index buffers.
        // Defined as closures to keep the loop body readable.
        let append_fill = |node: &NodeSnapshot, verts: &mut Vec<Vertex>, idxs: &mut Vec<u32>| {
            if !node.fill_enabled || node.fill_is_none {
                return;
            }
            let [a, b, c, d, e, f] = node.matrix;
            let opacity = node.fill_opacity * node.node_opacity;
            let mesh = tessellate_fill(&node.path_data, node.is_compound);
            if mesh.is_empty() {
                return;
            }
            let base = verts.len() as u32;
            for pos in &mesh.vertices {
                let x = a * pos[0] as f64 + c * pos[1] as f64 + e;
                let y = b * pos[0] as f64 + d * pos[1] as f64 + f;
                let color = node.fill_kind.sample_at(x, y, opacity);
                verts.push(Vertex {
                    position: [x as f32, y as f32],
                    color,
                });
            }
            for &i in &mesh.indices {
                idxs.push(base + i);
            }
        };

        // Render N layered strokes to approximate a Gaussian glow.
        // Drawing order: largest (faintest) first so smaller brighter layers overwrite near the edge.
        const GLOW_STEPS: usize = 10;
        let append_glow = |path_data: &photonic_core::path::PathData,
                           matrix: &[f64; 6],
                           glow: &[f32; 6],
                           join: photonic_core::style::LineJoin,
                           verts: &mut Vec<Vertex>,
                           idxs: &mut Vec<u32>| {
            let [gr, gg, gb, ga, go, gs] = *glow;
            let [a, b, c, d, e, f] = *matrix;
            for i in (0..GLOW_STEPS).rev() {
                // t goes from 1.0 (outermost, widest) down to 1/N (innermost)
                let t = (i + 1) as f32 / GLOW_STEPS as f32;
                let width = 2.0 * gs * t;
                // Gaussian falloff: alpha peaks at edge (small t) and fades outward
                let gaussian = (-4.5 * t * t).exp();
                let step_alpha = (go * gaussian * ga).min(1.0);
                let color = [gr, gg, gb, step_alpha];
                let mesh = tessellate_stroke(
                    path_data,
                    width,
                    photonic_core::style::LineCap::Round,
                    join,
                    4.0,
                );
                if mesh.is_empty() {
                    continue;
                }
                let base = verts.len() as u32;
                for pos in &mesh.vertices {
                    let x = a * pos[0] as f64 + c * pos[1] as f64 + e;
                    let y = b * pos[0] as f64 + d * pos[1] as f64 + f;
                    verts.push(Vertex {
                        position: [x as f32, y as f32],
                        color,
                    });
                }
                for &idx in &mesh.indices {
                    idxs.push(base + idx);
                }
            }
        };

        let append_stroke =
            |node: &NodeSnapshot, width: f32, verts: &mut Vec<Vertex>, idxs: &mut Vec<u32>| {
                if !node.stroke_enabled {
                    return;
                }
                let [a, b, c, d, e, f] = node.matrix;
                let mesh = match &node.stroke_widths {
                    // Variable-width profile: scale samples by the same factor the
                    // caller applies to the uniform width (stroke-align doubling),
                    // then build a filled ribbon outline.
                    Some(widths) if node.stroke_width > 0.0 => {
                        let scale = (width / node.stroke_width) as f64;
                        let scaled: Vec<f64> = widths.iter().map(|w| w * scale).collect();
                        tessellate_stroke_variable(&node.path_data, &scaled)
                    }
                    _ => tessellate_stroke(
                        &node.path_data,
                        width,
                        node.stroke_cap,
                        node.stroke_join,
                        node.stroke_miter,
                    ),
                };
                if mesh.is_empty() {
                    return;
                }
                let base = verts.len() as u32;
                for pos in &mesh.vertices {
                    let x = a * pos[0] as f64 + c * pos[1] as f64 + e;
                    let y = b * pos[0] as f64 + d * pos[1] as f64 + f;
                    verts.push(Vertex {
                        position: [x as f32, y as f32],
                        color: node.stroke_color,
                    });
                }
                for &i in &mesh.indices {
                    idxs.push(base + i);
                }
            };

        // ── Arrowhead helper ──────────────────────────────────────────────────
        // Appends a filled triangular or open-V arrowhead at the given world-space
        // endpoint `(px, py)` oriented toward direction `(dx, dy)` (not normalized).
        let append_arrowhead_triangle =
            |px: f64,
             py: f64,
             dx: f64,
             dy: f64,
             half_w: f64,
             length: f64,
             color: [f32; 4],
             style: photonic_core::style::ArrowheadStyle,
             verts: &mut Vec<Vertex>,
             idxs: &mut Vec<u32>| {
                use photonic_core::style::ArrowheadStyle;
                let len = (dx * dx + dy * dy).sqrt();
                if len < 1e-9 {
                    return;
                }
                let (ux, uy) = (dx / len, dy / len); // unit tangent toward tip
                let (nx, ny) = (-uy, ux); // unit normal

                match style {
                    ArrowheadStyle::FilledArrow => {
                        // Solid filled triangle: tip at (px,py), base behind by `length`.
                        let tip = (px, py);
                        let base = (px - ux * length, py - uy * length);
                        let left = (base.0 + nx * half_w, base.1 + ny * half_w);
                        let right = (base.0 - nx * half_w, base.1 - ny * half_w);
                        let base_idx = verts.len() as u32;
                        verts.push(Vertex {
                            position: [tip.0 as f32, tip.1 as f32],
                            color,
                        });
                        verts.push(Vertex {
                            position: [left.0 as f32, left.1 as f32],
                            color,
                        });
                        verts.push(Vertex {
                            position: [right.0 as f32, right.1 as f32],
                            color,
                        });
                        idxs.extend_from_slice(&[base_idx, base_idx + 1, base_idx + 2]);
                    }
                    ArrowheadStyle::OpenArrow => {
                        // Two thin quads forming an open V. Each "line" is `line_w` wide.
                        let line_w = half_w * 0.25;
                        let tip = (px, py);
                        let left = (
                            px - ux * length + nx * half_w,
                            py - uy * length + ny * half_w,
                        );
                        let right = (
                            px - ux * length - nx * half_w,
                            py - uy * length - ny * half_w,
                        );

                        // Draw each arm as a thin quad (four verts, two tris).
                        let draw_arm =
                            |a: (f64, f64),
                             b: (f64, f64),
                             verts: &mut Vec<Vertex>,
                             idxs: &mut Vec<u32>| {
                                let adx = b.0 - a.0;
                                let ady = b.1 - a.1;
                                let al = (adx * adx + ady * ady).sqrt().max(1e-12);
                                let (anx, any) = (-ady / al, adx / al);
                                let p0 = (a.0 + anx * line_w, a.1 + any * line_w);
                                let p1 = (a.0 - anx * line_w, a.1 - any * line_w);
                                let p2 = (b.0 - anx * line_w, b.1 - any * line_w);
                                let p3 = (b.0 + anx * line_w, b.1 + any * line_w);
                                let bi = verts.len() as u32;
                                for (px2, py2) in [p0, p1, p2, p3] {
                                    verts.push(Vertex {
                                        position: [px2 as f32, py2 as f32],
                                        color,
                                    });
                                }
                                idxs.extend_from_slice(&[bi, bi + 1, bi + 2, bi, bi + 2, bi + 3]);
                            };
                        draw_arm(tip, left, verts, idxs);
                        draw_arm(tip, right, verts, idxs);
                    }
                    ArrowheadStyle::None => {}
                }
            };

        // Extract start and end endpoint + tangent from a BezPath.
        // Returns (start_pt, start_tangent, end_pt, end_tangent) as Option tuples.
        let path_endpoints =
            |bez: &kurbo::BezPath| -> Option<((f64, f64), (f64, f64), (f64, f64), (f64, f64))> {
                use kurbo::PathEl;
                let els: Vec<_> = bez.elements().to_vec();
                if els.is_empty() {
                    return None;
                }
                let mut start_pt = (0.0f64, 0.0f64);
                let mut start_tan = (1.0f64, 0.0f64);
                let mut end_pt = (0.0f64, 0.0f64);
                let mut end_tan = (1.0f64, 0.0f64);
                let mut cur = (0.0f64, 0.0f64);
                let mut found_start = false;
                for el in &els {
                    match el {
                        PathEl::MoveTo(p) => {
                            cur = (p.x, p.y);
                        }
                        PathEl::LineTo(p) => {
                            if !found_start {
                                start_pt = cur;
                                start_tan = (p.x - cur.0, p.y - cur.1);
                                found_start = true;
                            }
                            end_pt = (p.x, p.y);
                            end_tan = (p.x - cur.0, p.y - cur.1);
                            cur = (p.x, p.y);
                        }
                        PathEl::QuadTo(c, p) => {
                            if !found_start {
                                start_pt = cur;
                                start_tan = (c.x - cur.0, c.y - cur.1);
                                found_start = true;
                            }
                            end_pt = (p.x, p.y);
                            end_tan = (p.x - c.x, p.y - c.y);
                            cur = (p.x, p.y);
                        }
                        PathEl::CurveTo(c1, c2, p) => {
                            if !found_start {
                                start_pt = cur;
                                start_tan = (c1.x - cur.0, c1.y - cur.1);
                                found_start = true;
                            }
                            end_pt = (p.x, p.y);
                            end_tan = (p.x - c2.x, p.y - c2.y);
                            cur = (p.x, p.y);
                        }
                        PathEl::ClosePath => {}
                    }
                }
                if !found_start {
                    return None;
                }
                Some((start_pt, start_tan, end_pt, end_tan))
            };

        // Per-node index ranges tagged with their blend mode, coalesced into
        // `draw_segments` after the loop. The artboard rect (already appended
        // above) blends normally.
        let mut raw_segments: Vec<(BlendMode, u32, u32)> =
            vec![(BlendMode::Normal, 0, idxs.len() as u32)];

        for node in &nodes {
            let seg_start = idxs.len() as u32;
            // ── Outer glow: behind fill so fill clips the inward half ─────────
            if let Some((ref og, og_join)) = node.outer_glow {
                append_glow(
                    &node.path_data,
                    &node.matrix,
                    og,
                    og_join,
                    &mut verts,
                    &mut idxs,
                );
            }

            match node.stroke_align {
                // Outside: render doubled-width stroke first, then fill on top.
                // The fill paints over the inner half of the stroke, leaving only
                // the outer half visible.
                StrokeAlign::Outside => {
                    append_stroke(node, node.stroke_width * 2.0, &mut verts, &mut idxs);
                    append_fill(node, &mut verts, &mut idxs);
                    // Inner glow: render after fill, then re-clip with fill
                    if let Some((ref ig, ig_join)) = node.inner_glow {
                        append_glow(
                            &node.path_data,
                            &node.matrix,
                            ig,
                            ig_join,
                            &mut verts,
                            &mut idxs,
                        );
                        append_fill(node, &mut verts, &mut idxs);
                    }
                }
                // Center: fill first, then stroke centred on the path edge.
                StrokeAlign::Center => {
                    append_fill(node, &mut verts, &mut idxs);
                    // Inner glow: rendered over fill, re-clipped before stroke
                    if let Some((ref ig, ig_join)) = node.inner_glow {
                        append_glow(
                            &node.path_data,
                            &node.matrix,
                            ig,
                            ig_join,
                            &mut verts,
                            &mut idxs,
                        );
                        append_fill(node, &mut verts, &mut idxs);
                    }
                    append_stroke(node, node.stroke_width, &mut verts, &mut idxs);
                }
                // Inside: fill, then doubled-width stroke, then fill again.
                // The second fill paints over the outer half of the stroke, leaving
                // only the inner half visible.
                StrokeAlign::Inside => {
                    append_fill(node, &mut verts, &mut idxs);
                    // Inner glow: before the inside stroke
                    if let Some((ref ig, ig_join)) = node.inner_glow {
                        append_glow(
                            &node.path_data,
                            &node.matrix,
                            ig,
                            ig_join,
                            &mut verts,
                            &mut idxs,
                        );
                        append_fill(node, &mut verts, &mut idxs);
                    }
                    append_stroke(node, node.stroke_width * 2.0, &mut verts, &mut idxs);
                    // Re-draw fill to clip the outer half of the stroke.
                    if node.stroke_enabled {
                        append_fill(node, &mut verts, &mut idxs);
                    }
                }
            }

            // ── Arrowheads ────────────────────────────────────────────────────
            if node.stroke_enabled
                && (node.arrowhead_start != photonic_core::style::ArrowheadStyle::None
                    || node.arrowhead_end != photonic_core::style::ArrowheadStyle::None)
            {
                let bez = node.path_data.to_bez_path();
                if let Some((s_pt, s_tan, e_pt, e_tan)) = path_endpoints(&bez) {
                    let [a, b, c, d, e, f] = node.matrix;
                    let transform_pt = |px: f64, py: f64| -> (f64, f64) {
                        (a * px + c * py + e, b * px + d * py + f)
                    };
                    let transform_dir =
                        |dx: f64, dy: f64| -> (f64, f64) { (a * dx + c * dy, b * dx + d * dy) };
                    let w = node.stroke_width as f64;
                    let arrow_len = w * 3.5;
                    let arrow_hw = w * 1.5;
                    let color = node.stroke_color;

                    if node.arrowhead_start != photonic_core::style::ArrowheadStyle::None {
                        let (wx, wy) = transform_pt(s_pt.0, s_pt.1);
                        // Negate tangent so arrow points outward (away from path interior).
                        let (tdx, tdy) = transform_dir(-s_tan.0, -s_tan.1);
                        append_arrowhead_triangle(
                            wx,
                            wy,
                            tdx,
                            tdy,
                            arrow_hw,
                            arrow_len,
                            color,
                            node.arrowhead_start,
                            &mut verts,
                            &mut idxs,
                        );
                    }
                    if node.arrowhead_end != photonic_core::style::ArrowheadStyle::None {
                        let (wx, wy) = transform_pt(e_pt.0, e_pt.1);
                        let (tdx, tdy) = transform_dir(e_tan.0, e_tan.1);
                        append_arrowhead_triangle(
                            wx,
                            wy,
                            tdx,
                            tdy,
                            arrow_hw,
                            arrow_len,
                            color,
                            node.arrowhead_end,
                            &mut verts,
                            &mut idxs,
                        );
                    }
                }
            }

            // ── Gaussian glow job ─────────────────────────────────────────────
            if let Some(([gr, gg, gb, ga], radius_doc)) = node.gaussian_glow {
                let [a, b, c, d, e, f] = node.matrix;
                let mesh = tessellate_fill(&node.path_data, node.is_compound);
                if !mesh.is_empty() {
                    let glow_color = [gr, gg, gb, ga];
                    let mut gverts = Vec::with_capacity(mesh.vertices.len());
                    for pos in &mesh.vertices {
                        let x = (a * pos[0] as f64 + c * pos[1] as f64 + e) as f32;
                        let y = (b * pos[0] as f64 + d * pos[1] as f64 + f) as f32;
                        gverts.push(Vertex {
                            position: [x, y],
                            color: glow_color,
                        });
                    }
                    let sigma_px = (radius_doc as f64 * self.view.zoom) as f32;
                    self.pending_gaussian_glows.push(GaussianGlowJob {
                        verts: gverts,
                        idxs: mesh.indices,
                        sigma_px,
                    });
                }
            }

            // Tag this node's fill/stroke/arrowhead geometry with its blend mode.
            // (Gaussian glow lives in a separate buffer and is unaffected.)
            raw_segments.push((node.blend_mode, seg_start, idxs.len() as u32));
        }

        // ── Text-on-path glyphs ───────────────────────────────────────────────
        // Glyph outlines are already in document coordinates, so they are
        // appended directly (no per-node matrix). Drawn last → on top of shapes,
        // consistent with how flat text sits above the fill layer.
        let text_seg_start = idxs.len() as u32;
        for (glyphs, rgba) in &self.pending_path_text {
            for glyph in glyphs {
                let mesh = tessellate_fill(glyph, false);
                if mesh.is_empty() {
                    continue;
                }
                let base = verts.len() as u32;
                for pos in &mesh.vertices {
                    verts.push(Vertex {
                        position: [pos[0], pos[1]],
                        color: *rgba,
                    });
                }
                for &i in &mesh.indices {
                    idxs.push(base + i);
                }
            }
        }
        // Cover the appended glyph indices with a Normal-blend segment so the
        // separable-blend segment draw pass actually renders them (untagged
        // index ranges are skipped). Glyphs blend Normal, on top of everything.
        if (idxs.len() as u32) > text_seg_start {
            raw_segments.push((BlendMode::Normal, text_seg_start, idxs.len() as u32));
        }

        let segments = coalesce_segments(raw_segments);

        // Update cache for next frame (used when lock is contended).
        self.cached_vertices = verts.clone();
        self.cached_indices = idxs.clone();
        self.draw_segments = segments.clone();
        self.cached_segments = segments;

        (verts, idxs)
    }

    /// Record the document render pass into an existing command encoder.
    ///
    /// `msaa_view` is the 4× multisampled render target; `resolve_view` is the
    /// single-sample destination (surface texture or offscreen capture texture).
    fn record_document_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        msaa_view: &wgpu::TextureView,
        resolve_view: &wgpu::TextureView,
        vertices: &[Vertex],
        indices: &[u32],
    ) {
        if !vertices.is_empty() {
            let vbuf = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("vbuf"),
                    contents: bytemuck::cast_slice(vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                });
            let ibuf = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("ibuf"),
                    contents: bytemuck::cast_slice(indices),
                    usage: wgpu::BufferUsages::INDEX,
                });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("fill_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: msaa_view,
                    resolve_target: Some(resolve_view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(BG),
                        store: wgpu::StoreOp::Discard,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_bind_group(0, &self.camera_bind_group, &[]);
            pass.set_vertex_buffer(0, vbuf.slice(..));
            pass.set_index_buffer(ibuf.slice(..), wgpu::IndexFormat::Uint32);
            draw_segments(
                &mut pass,
                &self.draw_segments,
                &self.blend_pipelines,
                &self.fill_pipeline,
                indices.len() as u32,
            );
        } else {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("clear_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: msaa_view,
                    resolve_target: Some(resolve_view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(BG),
                        store: wgpu::StoreOp::Discard,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        }
    }

    /// Render to an offscreen texture, read back pixels, encode as PNG.
    fn capture_png(&mut self, vertices: &[Vertex], indices: &[u32]) -> Vec<u8> {
        let w = self.width;
        let h = self.height;

        // Offscreen resolve target (single-sample, read back as PNG)
        let tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("capture_tex"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.surface_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let tex_view = tex.create_view(&Default::default());

        // MSAA render target for the capture (resolved into tex_view)
        let (capture_msaa_tex, capture_msaa_view) =
            create_msaa_texture(&self.device, self.surface_format, w, h);

        // Draw geometry into the offscreen texture via MSAA
        let mut enc = self.device.create_command_encoder(&Default::default());
        self.record_document_pass(&mut enc, &capture_msaa_view, &tex_view, vertices, indices);

        // Render text nodes on top (same encoder, loads resolved geometry from tex_view)
        if !self.pending_texts.is_empty() {
            self.text_viewport.update(
                &self.queue,
                Resolution {
                    width: w,
                    height: h,
                },
            );

            let mut buffers: Vec<Buffer> = Vec::with_capacity(self.pending_texts.len());
            for snap in self.pending_texts.iter() {
                let font_size = snap.font_size.max(1.0);
                let line_height = font_size * snap.line_height_mul;
                let mut buf =
                    Buffer::new(&mut self.font_system, Metrics::new(font_size, line_height));
                buf.set_size(&mut self.font_system, None, None);
                let attrs = Attrs::new().family(Family::Name(&snap.font_family));
                buf.set_text(&mut self.font_system, &snap.content, attrs, Shaping::Basic);
                buf.shape_until_scroll(&mut self.font_system, false);
                buffers.push(buf);
            }

            let text_areas: Vec<TextArea> = self
                .pending_texts
                .iter()
                .zip(buffers.iter())
                .map(|(snap, buf)| TextArea {
                    buffer: buf,
                    left: snap.screen_x,
                    top: snap.screen_y,
                    scale: 1.0,
                    bounds: TextBounds {
                        left: i32::MIN,
                        top: i32::MIN,
                        right: i32::MAX,
                        bottom: i32::MAX,
                    },
                    default_color: GlyphonColor::rgba(
                        snap.color[0],
                        snap.color[1],
                        snap.color[2],
                        snap.color[3],
                    ),
                    custom_glyphs: &[],
                })
                .collect();

            if self
                .text_renderer
                .prepare(
                    &self.device,
                    &self.queue,
                    &mut self.font_system,
                    &mut self.text_atlas,
                    &self.text_viewport,
                    text_areas,
                    &mut self.swash_cache,
                )
                .is_ok()
            {
                {
                    let mut pass = enc
                        .begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: Some("capture_text_pass"),
                            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                view: &tex_view,
                                resolve_target: None,
                                ops: wgpu::Operations {
                                    load: wgpu::LoadOp::Load,
                                    store: wgpu::StoreOp::Store,
                                },
                            })],
                            depth_stencil_attachment: None,
                            timestamp_writes: None,
                            occlusion_query_set: None,
                        })
                        .forget_lifetime();
                    if let Err(e) =
                        self.text_renderer
                            .render(&self.text_atlas, &self.text_viewport, &mut pass)
                    {
                        tracing::warn!("glyphon render in capture failed: {:?}", e);
                    }
                }
                self.text_atlas.trim();
            }
        }

        self.queue.submit([enc.finish()]);
        drop(capture_msaa_tex); // keep alive until after submit

        // Copy texture → staging buffer (bytes_per_row must be aligned to 256)
        let bpr = align256(w * 4);
        let staging = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("staging"),
            size: (bpr * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let mut enc2 = self.device.create_command_encoder(&Default::default());
        enc2.copy_texture_to_buffer(
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
        self.queue.submit([enc2.finish()]);

        // Map & read
        let slice = staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        tracing::info!("render: capture_png — poll(Wait) starting");
        self.device.poll(wgpu::Maintain::Wait);
        tracing::info!("render: capture_png — poll(Wait) done");
        if rx.recv().ok().and_then(|r| r.ok()).is_none() {
            tracing::warn!("render: capture_png — map_async failed");
            return vec![];
        }

        let raw = slice.get_mapped_range();

        let is_bgra = matches!(
            self.surface_format,
            wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
        );

        let mut pixels: Vec<u8> = Vec::with_capacity((w * h * 4) as usize);
        for row in 0..h {
            let start = (row * bpr) as usize;
            let end = start + (w * 4) as usize;
            if is_bgra {
                for px in raw[start..end].chunks_exact(4) {
                    pixels.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
                }
            } else {
                pixels.extend_from_slice(&raw[start..end]);
            }
        }
        drop(raw);
        staging.unmap();

        // Encode as PNG
        let img: ImageBuffer<Rgba<u8>, _> =
            ImageBuffer::from_raw(w, h, pixels).unwrap_or_else(|| ImageBuffer::new(w, h));
        let mut png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap_or_default();
        png
    }
}

fn align256(n: u32) -> u32 {
    (n + 255) & !255
}

fn create_msaa_texture(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("msaa_texture"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: MSAA_SAMPLES,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    (texture, view)
}

fn create_glow_textures(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
) -> (
    wgpu::Texture,
    wgpu::TextureView,
    wgpu::Texture,
    wgpu::TextureView,
) {
    let desc = wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    };
    let a = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("glow_a"),
        ..desc
    });
    let b = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("glow_b"),
        ..desc
    });
    let av = a.create_view(&Default::default());
    let bv = b.create_view(&Default::default());
    (a, av, b, bv)
}
