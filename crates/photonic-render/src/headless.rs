//! Headless (off-screen) renderer — no window surface required.
//!
//! Used by the Lua script runner to render a document to a PNG file
//! without opening a visible window.

use crate::{
    canvas::CanvasView,
    pipeline::{
        create_blur_bgl, create_blur_pipeline_with_blend, create_camera_bind_group_layout,
        create_fill_pipeline, BlurBlend, BlurParams, CameraUniform, Vertex,
    },
    tessellator::{tessellate_fill, tessellate_stroke},
};
use image::{ImageBuffer, Rgba};
use photonic_core::{node::SceneNodeKind, style::FillKind, Document};
use wgpu::util::DeviceExt;

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const BG: wgpu::Color = wgpu::Color {
    r: 0.15,
    g: 0.15,
    b: 0.15,
    a: 1.0,
};
const MSAA_SAMPLES: u32 = 4;

// ─── Export options ───────────────────────────────────────────────────────────

/// What to render behind the artwork.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ExportBackground {
    /// White artboard rectangle (matches the in-app canvas appearance).
    Artboard,
    /// Fully transparent — shapes rendered over alpha=0 background.
    Transparent,
}

/// Settings that control how a document is rendered for export.
#[derive(Debug, Clone)]
pub struct ExportOptions {
    pub background: ExportBackground,
    /// When true the output is cropped to the tight bounding box of all
    /// visible artwork rather than the full artboard dimensions.
    pub crop_to_content: bool,
    /// Which square sizes to include in an `.ico` file.
    pub ico_sizes: Vec<u32>,
    /// JPEG quality (1–100). Only used by `render_jpeg_*` methods.
    pub jpeg_quality: u8,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            background: ExportBackground::Artboard,
            crop_to_content: false,
            ico_sizes: vec![16, 32, 48, 256],
            jpeg_quality: 90,
        }
    }
}

pub struct HeadlessRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    camera_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    fill_pipeline: wgpu::RenderPipeline,
    // ── Live-effects blur layer ───────────────────────────────────────────────
    /// 1-sample fill pipeline for rendering effect silhouettes to an offscreen
    /// texture (the blur ping-pong textures are single-sample).
    fill_pipeline_1spp: wgpu::RenderPipeline,
    blur_bgl: wgpu::BindGroupLayout,
    /// Separable blur pass (alpha-composited). Also used with sigma≈0 as a
    /// texture-passthrough compositor.
    blur_pipeline: wgpu::RenderPipeline,
    blur_sampler: wgpu::Sampler,
}

impl HeadlessRenderer {
    pub async fn new() -> Self {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None, // no window surface
                force_fallback_adapter: false,
            })
            .await
            .expect("No suitable GPU adapter for headless rendering");

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("headless_device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                    memory_hints: Default::default(),
                },
                None,
            )
            .await
            .expect("Failed to create headless wgpu device");

        let camera_bgl = create_camera_bind_group_layout(&device);
        let initial_cam = CameraUniform::from_viewport(0.0, 0.0, 1.0, 1, 1);
        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("headless_camera_buf"),
            contents: bytemuck::bytes_of(&initial_cam),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("headless_camera_bg"),
            layout: &camera_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });

        let fill_pipeline = create_fill_pipeline(&device, FORMAT, &camera_bgl, MSAA_SAMPLES);
        let fill_pipeline_1spp = create_fill_pipeline(&device, FORMAT, &camera_bgl, 1);
        let blur_bgl = create_blur_bgl(&device);
        let blur_pipeline =
            create_blur_pipeline_with_blend(&device, FORMAT, &blur_bgl, BlurBlend::StraightAlpha);
        let blur_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("headless_blur_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        Self {
            device,
            queue,
            camera_buffer,
            camera_bind_group,
            fill_pipeline,
            fill_pipeline_1spp,
            blur_bgl,
            blur_pipeline,
            blur_sampler,
        }
    }

    /// Render `document` to a PNG and return the bytes.
    ///
    /// Output dimensions match the document artboard (clamped to 1 pixel minimum).
    pub fn render_png(&self, document: &Document) -> Vec<u8> {
        let w = (document.width as u32).max(1);
        let h = (document.height as u32).max(1);
        self.render_png_at_size(document, w, h)
    }

    /// Render `document` to a PNG at an explicit pixel size using default options.
    pub fn render_png_at_size(&self, document: &Document, w: u32, h: u32) -> Vec<u8> {
        self.render_png_with_opts(document, w, h, &ExportOptions::default())
    }

    /// Render `document` to a PNG at an explicit pixel size with full export control.
    pub fn render_png_with_opts(
        &self,
        document: &Document,
        w: u32,
        h: u32,
        opts: &ExportOptions,
    ) -> Vec<u8> {
        let w = w.max(1);
        let h = h.max(1);

        let include_artboard_bg = opts.background == ExportBackground::Artboard;
        let (verts, idxs, blur_jobs) = build_geometry(document, include_artboard_bg);

        // Camera: fit artboard or content bounding box to the output size.
        let mut view = CanvasView::new(w, h);
        if opts.crop_to_content {
            if let Some((cx, cy, cw, ch)) = content_bounds(&verts, include_artboard_bg, document) {
                view.fit_to_rect(cx, cy, cw, ch);
            } else {
                view.fit_to_rect(0.0, 0.0, document.width, document.height);
            }
        } else {
            view.fit_to_rect(0.0, 0.0, document.width, document.height);
        }

        let cam = CameraUniform::from_viewport(view.pan_x, view.pan_y, view.zoom, w, h);
        self.queue
            .write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&cam));

        let clear = match opts.background {
            ExportBackground::Artboard => BG,
            ExportBackground::Transparent => wgpu::Color {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.0,
            },
        };

        // Final readback target: single-sample, COPY_SRC.
        let tex = self.make_color_tex(
            w,
            h,
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let tex_view = tex.create_view(&Default::default());

        // MSAA render target for the sharp document geometry.
        let msaa_tex =
            self.make_color_tex(w, h, MSAA_SAMPLES, wgpu::TextureUsages::RENDER_ATTACHMENT);
        let msaa_view = msaa_tex.create_view(&Default::default());

        let mut enc = self.device.create_command_encoder(&Default::default());

        if blur_jobs.is_empty() {
            // Fast path: render the document straight into the readback target.
            self.record_pass(&mut enc, &msaa_view, &tex_view, &verts, &idxs, clear);
        } else {
            // Layered path: the live-effects blur layer must sit *between* the
            // artboard background and the sharp shapes. So render the shapes
            // (minus the artboard rect) to a transparent offscreen texture, blur
            // the effect silhouettes into a separate layer, then composite
            //   background → effects → shapes
            // into the readback target.
            let doc_tex = self.make_color_tex(
                w,
                h,
                1,
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            );
            let doc_view = doc_tex.create_view(&Default::default());

            // The artboard rect is the first 4 verts / 6 indices when present;
            // skip it here and reproduce it via the composite clear colour.
            let skip = if include_artboard_bg { 6 } else { 0 };
            let transparent = wgpu::Color::TRANSPARENT;
            self.record_pass(
                &mut enc,
                &msaa_view,
                &doc_view,
                &verts,
                &idxs[skip..],
                transparent,
            );

            let (fx_tex, fx_view) =
                self.render_effects_layer(&mut enc, &blur_jobs, view.zoom, w, h);

            // Composite: clear to the artboard/background, then effects, then shapes.
            let comp_clear = if include_artboard_bg {
                wgpu::Color::WHITE
            } else {
                clear
            };
            self.composite_layers(&mut enc, &tex_view, &[&fx_view, &doc_view], comp_clear);
            drop(fx_tex);
            drop(doc_tex);
        }
        drop(msaa_tex); // keep alive until submit
        self.queue.submit([enc.finish()]);

        // Copy texture → staging buffer (row stride must be aligned to 256)
        let bpr = align256(w * 4);
        let staging = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("headless_staging"),
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

        // Map and read back
        let slice = staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device.poll(wgpu::Maintain::Wait);
        if rx.recv().ok().and_then(|r| r.ok()).is_none() {
            return vec![];
        }

        let raw = slice.get_mapped_range();
        let mut pixels = Vec::with_capacity((w * h * 4) as usize);
        for row in 0..h {
            let start = (row * bpr) as usize;
            let end = start + (w * 4) as usize;
            pixels.extend_from_slice(&raw[start..end]);
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

    /// Render `document` to JPEG at an explicit pixel size with full export control.
    ///
    /// JPEG does not support transparency — alpha is composited onto a white
    /// background before encoding.  Quality is taken from `opts.jpeg_quality`
    /// (clamped 1–100).
    pub fn render_jpeg_with_opts(
        &self,
        document: &Document,
        w: u32,
        h: u32,
        opts: &ExportOptions,
    ) -> Vec<u8> {
        // Render to RGBA pixels using the existing PNG pipeline.
        let rgba_bytes = self.render_png_with_opts(document, w, h, opts);

        // Decode the PNG into an image buffer so we can re-encode as JPEG.
        let img = image::load_from_memory_with_format(&rgba_bytes, image::ImageFormat::Png)
            .unwrap_or_else(|_| image::DynamicImage::new_rgba8(w, h));

        // Composite alpha onto white (to_rgb8 composites onto black).
        let rgba = img.to_rgba8();
        let mut rgb = image::RgbImage::new(rgba.width(), rgba.height());
        for (src, dst) in rgba.pixels().zip(rgb.pixels_mut()) {
            let a = src[3] as f32 / 255.0;
            dst[0] = (src[0] as f32 * a + 255.0 * (1.0 - a)) as u8;
            dst[1] = (src[1] as f32 * a + 255.0 * (1.0 - a)) as u8;
            dst[2] = (src[2] as f32 * a + 255.0 * (1.0 - a)) as u8;
        }
        let rgb = image::DynamicImage::ImageRgb8(rgb);

        let quality = opts.jpeg_quality.clamp(1, 100);
        let mut buf = Vec::new();
        let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(
            std::io::Cursor::new(&mut buf),
            quality,
        );
        rgb.write_with_encoder(encoder).unwrap_or_default();
        buf
    }

    /// Render `document` to WebP at an explicit pixel size with full export control.
    ///
    /// WebP supports transparency (lossy or lossless). Quality from `opts.jpeg_quality`
    /// (reused field; 1–100, where 100 = lossless).
    pub fn render_webp_with_opts(
        &self,
        document: &Document,
        w: u32,
        h: u32,
        opts: &ExportOptions,
    ) -> Vec<u8> {
        let rgba_bytes = self.render_png_with_opts(document, w, h, opts);
        let img = image::load_from_memory_with_format(&rgba_bytes, image::ImageFormat::Png)
            .unwrap_or_else(|_| image::DynamicImage::new_rgba8(w, h));

        let mut buf = Vec::new();
        let encoder =
            image::codecs::webp::WebPEncoder::new_lossless(std::io::Cursor::new(&mut buf));
        img.write_with_encoder(encoder).unwrap_or_default();
        buf
    }

    /// Render `document` to GIF at an explicit pixel size.
    pub fn render_gif_with_opts(
        &self,
        document: &Document,
        w: u32,
        h: u32,
        opts: &ExportOptions,
    ) -> Vec<u8> {
        let rgba_bytes = self.render_png_with_opts(document, w, h, opts);
        let img = image::load_from_memory_with_format(&rgba_bytes, image::ImageFormat::Png)
            .unwrap_or_else(|_| image::DynamicImage::new_rgba8(w, h));
        let mut buf = Vec::new();
        let encoder = image::codecs::gif::GifEncoder::new(std::io::Cursor::new(&mut buf));
        img.write_with_encoder(encoder).unwrap_or_default();
        buf
    }

    /// Render `document` to TIFF at an explicit pixel size.
    pub fn render_tiff_with_opts(
        &self,
        document: &Document,
        w: u32,
        h: u32,
        opts: &ExportOptions,
    ) -> Vec<u8> {
        let rgba_bytes = self.render_png_with_opts(document, w, h, opts);
        let img = image::load_from_memory_with_format(&rgba_bytes, image::ImageFormat::Png)
            .unwrap_or_else(|_| image::DynamicImage::new_rgba8(w, h));
        let mut buf = Vec::new();
        img.write_to(
            &mut std::io::Cursor::new(&mut buf),
            image::ImageFormat::Tiff,
        )
        .unwrap_or_default();
        buf
    }

    /// Render `document` as a multi-resolution `.ico` file and return the bytes.
    pub fn render_ico(&self, document: &Document) -> anyhow::Result<Vec<u8>> {
        self.render_ico_with_opts(document, &ExportOptions::default())
    }

    /// Render `document` as a `.ico` file with full export control.
    pub fn render_ico_with_opts(
        &self,
        document: &Document,
        opts: &ExportOptions,
    ) -> anyhow::Result<Vec<u8>> {
        let mut icon_dir = ico::IconDir::new(ico::ResourceType::Icon);

        for &size in &opts.ico_sizes {
            let png = self.render_png_with_opts(document, size, size, opts);
            if png.is_empty() {
                continue;
            }
            let icon_image = ico::IconImage::read_png(std::io::Cursor::new(&png))?;
            icon_dir.add_entry(ico::IconDirEntry::encode(&icon_image)?);
        }

        let mut buf = Vec::new();
        icon_dir.write(std::io::Cursor::new(&mut buf))?;
        Ok(buf)
    }

    fn record_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        msaa_view: &wgpu::TextureView,
        resolve_view: &wgpu::TextureView,
        vertices: &[Vertex],
        indices: &[u32],
        clear: wgpu::Color,
    ) {
        if !vertices.is_empty() {
            let vbuf = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("hl_vbuf"),
                    contents: bytemuck::cast_slice(vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                });
            let ibuf = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("hl_ibuf"),
                    contents: bytemuck::cast_slice(indices),
                    usage: wgpu::BufferUsages::INDEX,
                });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("hl_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: msaa_view,
                    resolve_target: Some(resolve_view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear),
                        store: wgpu::StoreOp::Discard,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.fill_pipeline);
            pass.set_bind_group(0, &self.camera_bind_group, &[]);
            pass.set_vertex_buffer(0, vbuf.slice(..));
            pass.set_index_buffer(ibuf.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..indices.len() as u32, 0, 0..1);
        } else {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("hl_clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: msaa_view,
                    resolve_target: Some(resolve_view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear),
                        store: wgpu::StoreOp::Discard,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        }
    }

    /// Create a colour texture of the given size and sample count.
    fn make_color_tex(
        &self,
        w: u32,
        h: u32,
        sample_count: u32,
        usage: wgpu::TextureUsages,
    ) -> wgpu::Texture {
        self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("headless_fx_tex"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage,
            view_formats: &[],
        })
    }

    /// Bind group for the blur shader: source texture + sampler + params.
    fn blur_bind_group(
        &self,
        src: &wgpu::TextureView,
        sigma: f32,
        horizontal: bool,
    ) -> wgpu::BindGroup {
        let params = BlurParams {
            sigma,
            horizontal: horizontal as u32,
            _pad: [0.0; 2],
        };
        let buf = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("headless_blur_params"),
                contents: bytemuck::bytes_of(&params),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("headless_blur_bg"),
            layout: &self.blur_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(src),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.blur_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: buf.as_entire_binding(),
                },
            ],
        })
    }

    /// Render each blur job (silhouette → H-blur → V-blur) and accumulate them
    /// into a single-sample effects texture (straight-alpha "over"). Returns the
    /// accumulation texture and its view.
    fn render_effects_layer(
        &self,
        enc: &mut wgpu::CommandEncoder,
        jobs: &[BlurJob],
        zoom: f64,
        w: u32,
        h: u32,
    ) -> (wgpu::Texture, wgpu::TextureView) {
        let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        let fx_a = self.make_color_tex(w, h, 1, usage);
        let fx_b = self.make_color_tex(w, h, 1, usage);
        let fx_accum = self.make_color_tex(w, h, 1, usage);
        let (a_view, b_view, accum_view) = (
            fx_a.create_view(&Default::default()),
            fx_b.create_view(&Default::default()),
            fx_accum.create_view(&Default::default()),
        );

        // Clear the accumulator once; jobs composite into it with Load below.
        let mut accum_cleared = false;
        for job in jobs {
            if job.idxs.is_empty() {
                continue;
            }
            let sigma = (job.radius_doc * zoom).max(0.0) as f32;

            // Pass A: silhouette → fx_a (cleared transparent).
            let vbuf = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("fx_vbuf"),
                    contents: bytemuck::cast_slice(&job.verts),
                    usage: wgpu::BufferUsages::VERTEX,
                });
            let ibuf = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("fx_ibuf"),
                    contents: bytemuck::cast_slice(&job.idxs),
                    usage: wgpu::BufferUsages::INDEX,
                });
            {
                let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("fx_silhouette"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &a_view,
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

            // Pass B: horizontal blur fx_a → fx_b.
            {
                let bg = self.blur_bind_group(&a_view, sigma, true);
                let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("fx_blur_h"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &b_view,
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
                pass.set_pipeline(&self.blur_pipeline);
                pass.set_bind_group(0, &bg, &[]);
                pass.draw(0..6, 0..1);
            }

            // Pass C: vertical blur fx_b → fx_accum (accumulate).
            {
                let bg = self.blur_bind_group(&b_view, sigma, false);
                let load = if accum_cleared {
                    wgpu::LoadOp::Load
                } else {
                    accum_cleared = true;
                    wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                };
                let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("fx_blur_v"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &accum_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                pass.set_pipeline(&self.blur_pipeline);
                pass.set_bind_group(0, &bg, &[]);
                pass.draw(0..6, 0..1);
            }
        }

        (fx_accum, accum_view)
    }

    /// Composite `layers` (bottom-first) onto `target` over a cleared background,
    /// using the blur shader at sigma≈0 as a straight-alpha texture passthrough.
    fn composite_layers(
        &self,
        enc: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        layers: &[&wgpu::TextureView],
        clear: wgpu::Color,
    ) {
        for (i, layer) in layers.iter().enumerate() {
            let bg = self.blur_bind_group(layer, 0.0, true);
            let load = if i == 0 {
                wgpu::LoadOp::Clear(clear)
            } else {
                wgpu::LoadOp::Load
            };
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("fx_composite"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.blur_pipeline);
            pass.set_bind_group(0, &bg, &[]);
            pass.draw(0..6, 0..1);
        }
    }
}

// ─── Shared geometry builder ──────────────────────────────────────────────────

/// Compute the axis-aligned bounding box of all shape vertices (content only,
/// excluding the artboard background rect).  Returns `(min_x, min_y, width,
/// height)` in canvas space, or `None` if there are no shape vertices.
fn content_bounds(
    verts: &[Vertex],
    include_artboard_bg: bool,
    doc: &Document,
) -> Option<(f64, f64, f64, f64)> {
    // When the artboard bg was included it occupies the first 4 vertices.
    let skip = if include_artboard_bg { 4 } else { 0 };
    let shape_verts = &verts[skip..];
    if shape_verts.is_empty() {
        return None;
    }
    let mut min_x = f32::MAX;
    let mut min_y = f32::MAX;
    let mut max_x = f32::MIN;
    let mut max_y = f32::MIN;
    for v in shape_verts {
        min_x = min_x.min(v.position[0]);
        min_y = min_y.min(v.position[1]);
        max_x = max_x.max(v.position[0]);
        max_y = max_y.max(v.position[1]);
    }
    let w = (max_x - min_x) as f64;
    let h = (max_y - min_y) as f64;
    if w < 1.0 || h < 1.0 {
        // Degenerate — fall back to artboard
        return Some((0.0, 0.0, doc.width, doc.height));
    }
    Some((min_x as f64, min_y as f64, w, h))
}

/// One blurred effect to render into the offscreen effects layer (composited
/// beneath the sharp document): geometry already transformed into document
/// space, plus the blur radius in document units (scaled by zoom at render time).
struct BlurJob {
    verts: Vec<Vertex>,
    idxs: Vec<u32>,
    radius_doc: f64,
}

/// Tessellate `path`'s fill, transform it by `m` (+ `offset`), flat-color it,
/// and package it as a [`BlurJob`]. Returns `None` for empty geometry.
fn silhouette_job(
    path: &photonic_core::path::PathData,
    m: &[f64; 6],
    offset: (f64, f64),
    color: [f32; 4],
    radius_doc: f64,
) -> Option<BlurJob> {
    let mesh = tessellate_fill(path, false);
    if mesh.is_empty() {
        return None;
    }
    let [a, b, c, d, e, f] = *m;
    let (ox, oy) = offset;
    let mut verts = Vec::with_capacity(mesh.vertices.len());
    for pos in &mesh.vertices {
        let x = a * pos[0] as f64 + c * pos[1] as f64 + e + ox;
        let y = b * pos[0] as f64 + d * pos[1] as f64 + f + oy;
        verts.push(Vertex {
            position: [x as f32, y as f32],
            color,
        });
    }
    Some(BlurJob {
        verts,
        idxs: mesh.indices,
        radius_doc,
    })
}

fn build_geometry(
    doc: &Document,
    include_artboard_bg: bool,
) -> (Vec<Vertex>, Vec<u32>, Vec<BlurJob>) {
    let mut verts: Vec<Vertex> = Vec::new();
    let mut idxs: Vec<u32> = Vec::new();
    let mut blur_jobs: Vec<BlurJob> = Vec::new();

    // Optional white artboard rectangle (always first 4 vertices when present).
    if include_artboard_bg {
        let (w, h) = (doc.width as f32, doc.height as f32);
        let white = [1.0f32, 1.0, 1.0, 1.0];
        let base = verts.len() as u32;
        verts.extend_from_slice(&[
            Vertex {
                position: [0.0, 0.0],
                color: white,
            },
            Vertex {
                position: [w, 0.0],
                color: white,
            },
            Vertex {
                position: [w, h],
                color: white,
            },
            Vertex {
                position: [0.0, h],
                color: white,
            },
        ]);
        idxs.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    for node in doc.nodes_in_draw_order() {
        // Resolve symbol instances to the current master (+ overrides) so
        // headless export matches the live renderer.
        let resolved = doc.resolve_render_node(node);
        let node = resolved.as_ref();
        let SceneNodeKind::Path(path_node) = &node.kind else {
            continue;
        };
        let [a, b, c, d, e, f] = node.transform.matrix;

        // ── Drop shadow → blurred offset silhouette in the effects layer ───────
        if node.drop_shadow.enabled {
            let s = &node.drop_shadow;
            let alpha = (s.color.a * s.opacity * node.opacity).min(1.0);
            if let Some(job) = silhouette_job(
                &path_node.path_data,
                &node.transform.matrix,
                (s.dx as f64, s.dy as f64),
                [s.color.r, s.color.g, s.color.b, alpha],
                s.blur as f64,
            ) {
                blur_jobs.push(job);
            }
        }

        // ── Object blur / feather → blurred fill in the effects layer ──────────
        // For solid fills the sharp fill is suppressed and replaced by a true
        // Gaussian-blurred copy. Gradient/image interior blur is a follow-up.
        let blur_radius = if node.object_blur.enabled {
            node.object_blur.radius
        } else if node.feather.enabled {
            node.feather.radius
        } else {
            0.0
        };
        let mut fill_blurred = false;
        if blur_radius > 0.0 {
            if let FillKind::Solid(col) = &path_node.fill.kind {
                let alpha = col.a * path_node.fill.opacity * node.opacity;
                if let Some(job) = silhouette_job(
                    &path_node.path_data,
                    &node.transform.matrix,
                    (0.0, 0.0),
                    [col.r, col.g, col.b, alpha],
                    blur_radius as f64,
                ) {
                    blur_jobs.push(job);
                    fill_blurred = true;
                }
            }
        }

        // ── Fill (skipped when replaced by a blurred copy) ─────────────────────
        if !fill_blurred
            && path_node.fill.enabled
            && !matches!(&path_node.fill.kind, FillKind::None)
        {
            let opacity = path_node.fill.opacity * node.opacity;
            let mesh = tessellate_fill(&path_node.path_data, false);
            if !mesh.is_empty() {
                let base = verts.len() as u32;
                for pos in &mesh.vertices {
                    let x = a * pos[0] as f64 + c * pos[1] as f64 + e;
                    let y = b * pos[0] as f64 + d * pos[1] as f64 + f;
                    let color = path_node.fill.kind.sample_at(x, y, opacity);
                    verts.push(Vertex {
                        position: [x as f32, y as f32],
                        color,
                    });
                }
                for &i in &mesh.indices {
                    idxs.push(base + i);
                }
            }
        }

        // ── Stroke ───────────────────────────────────────────────────────────
        if path_node.stroke.enabled && path_node.stroke.width > 0.0 {
            let sc = &path_node.stroke;
            let alpha = sc.color.a * sc.opacity * node.opacity;
            let stroke_color = [sc.color.r, sc.color.g, sc.color.b, alpha];

            let mesh = tessellate_stroke(
                &path_node.path_data,
                sc.width as f32,
                sc.line_cap,
                sc.line_join,
                sc.miter_limit as f32,
            );
            if !mesh.is_empty() {
                let base = verts.len() as u32;
                for pos in &mesh.vertices {
                    let x = a * pos[0] as f64 + c * pos[1] as f64 + e;
                    let y = b * pos[0] as f64 + d * pos[1] as f64 + f;
                    verts.push(Vertex {
                        position: [x as f32, y as f32],
                        color: stroke_color,
                    });
                }
                for &i in &mesh.indices {
                    idxs.push(base + i);
                }
            }
        }
    }

    (verts, idxs, blur_jobs)
}

fn align256(n: u32) -> u32 {
    (n + 255) & !255
}

#[cfg(test)]
mod drop_shadow_tests {
    use super::*;
    use photonic_core::{
        color::Color,
        node::{PathNode, SceneNode, SceneNodeKind},
        path::PathData,
        style::Fill,
        Document,
    };

    fn try_renderer() -> Option<HeadlessRenderer> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))?;
        Some(pollster::block_on(HeadlessRenderer::new()))
    }

    fn luma(px: [u8; 4]) -> f32 {
        (0.299 * px[0] as f32 + 0.587 * px[1] as f32 + 0.114 * px[2] as f32) / 255.0
    }

    #[test]
    fn hard_drop_shadow_appears_offset_and_darkens_backdrop() {
        let Some(r) = try_renderer() else {
            eprintln!("no GPU adapter — skipping drop-shadow test");
            return;
        };
        let mut doc = Document::new("ds", 100.0, 100.0);
        // White square at (30,30)-(70,70).
        let mut node = SceneNode::new(
            "sq",
            doc.active_layer_id.unwrap(),
            SceneNodeKind::Path(
                PathNode::new(PathData::rect(30.0, 30.0, 40.0, 40.0))
                    .with_fill(Fill::solid(Color::WHITE)),
            ),
        );
        // Hard black shadow offset down-right by (20,20).
        node.drop_shadow.enabled = true;
        node.drop_shadow.color = Color::new(0.0, 0.0, 0.0, 1.0);
        node.drop_shadow.opacity = 0.5;
        node.drop_shadow.dx = 20.0;
        node.drop_shadow.dy = 20.0;
        node.drop_shadow.blur = 0.0;
        doc.add_node(node, None);

        let png = r.render_png_at_size(&doc, 100, 100);
        let img = image::load_from_memory(&png).expect("png").to_rgba8();
        let at = |x, y| luma(img.get_pixel(x, y).0);

        // (80,80): inside shadow square (50-90) but outside fill (30-70) → darkened.
        let shadow = at(80, 80);
        // (50,50): inside the white fill → stays bright (fill drawn over shadow).
        let fill = at(50, 50);
        // (10,10): untouched white artboard.
        let bg = at(10, 10);

        assert!(bg > 0.9, "artboard should be white, got {bg}");
        assert!(fill > 0.9, "fill should be white, got {fill}");
        assert!(
            shadow < 0.8 && shadow > 0.2,
            "shadow region should be a mid-gray (got {shadow})",
        );
    }

    #[test]
    fn object_blur_softens_the_edge() {
        let Some(r) = try_renderer() else {
            eprintln!("no GPU adapter — skipping object-blur test");
            return;
        };
        let mut doc = Document::new("blur", 100.0, 100.0);
        let mut node = SceneNode::new(
            "sq",
            doc.active_layer_id.unwrap(),
            SceneNodeKind::Path(
                PathNode::new(PathData::rect(30.0, 30.0, 40.0, 40.0))
                    .with_fill(Fill::solid(Color::WHITE)),
            ),
        );
        node.object_blur.enabled = true;
        node.object_blur.radius = 8.0;
        doc.add_node(node, None);

        // Transparent background so the soft halo shows as partial coverage.
        let opts = ExportOptions {
            background: ExportBackground::Transparent,
            ..Default::default()
        };
        let png = r.render_png_with_opts(&doc, 100, 100, &opts);
        let img = image::load_from_memory(&png).expect("png").to_rgba8();
        // Square spans 30–70; just outside the right edge a hard fill would be
        // fully transparent — a soft edge gives partial coverage there.
        let halo = img.get_pixel(72, 50).0[3] as f32 / 255.0; // alpha, ~2px out
        let far = img.get_pixel(95, 50).0[3] as f32 / 255.0;
        let inside = img.get_pixel(50, 50).0[3] as f32 / 255.0;

        assert!(inside > 0.9, "fill interior should be opaque, got {inside}");
        assert!(
            halo > 0.03 && halo < 0.95,
            "edge should be partially covered (soft), got {halo}",
        );
        assert!(far < 0.05, "far outside should stay transparent, got {far}");
    }

    #[test]
    fn soft_drop_shadow_falls_off_gradually() {
        let Some(r) = try_renderer() else {
            eprintln!("no GPU adapter — skipping soft-shadow falloff test");
            return;
        };
        let mut doc = Document::new("soft", 100.0, 100.0);
        let mut node = SceneNode::new(
            "sq",
            doc.active_layer_id.unwrap(),
            SceneNodeKind::Path(
                PathNode::new(PathData::rect(20.0, 20.0, 30.0, 30.0))
                    .with_fill(Fill::solid(Color::WHITE)),
            ),
        );
        node.drop_shadow.enabled = true;
        node.drop_shadow.color = Color::new(0.0, 0.0, 0.0, 1.0);
        node.drop_shadow.opacity = 1.0;
        node.drop_shadow.dx = 0.0;
        node.drop_shadow.dy = 0.0;
        node.drop_shadow.blur = 10.0; // true gaussian
        doc.add_node(node, None);

        let opts = ExportOptions {
            background: ExportBackground::Transparent,
            ..Default::default()
        };
        let png = r.render_png_with_opts(&doc, 100, 100, &opts);
        let img = image::load_from_memory(&png).expect("png").to_rgba8();
        // Shadow alpha just outside the right edge (x=50), increasing distance.
        let a = |x: u32| img.get_pixel(x, 35).0[3] as f32 / 255.0;
        let near = a(53); // 3px out
        let mid = a(60); // 10px out
        let outer = a(66); // 16px out

        // A true Gaussian blur decays monotonically with distance; a hard edge
        // would jump to ~0 immediately.
        assert!(near > mid, "near ({near}) should exceed mid ({mid})");
        assert!(mid > outer, "mid ({mid}) should exceed outer ({outer})");
        assert!(
            near > 0.1,
            "shadow should be visible near the edge, got {near}"
        );
    }
}
