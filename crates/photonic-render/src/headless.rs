//! Headless (off-screen) renderer — no window surface required.
//!
//! Used by the Lua script runner to render a document to a PNG file
//! without opening a visible window.

use crate::{
    canvas::CanvasView,
    pipeline::{create_camera_bind_group_layout, create_fill_pipeline, CameraUniform, Vertex},
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

        Self {
            device,
            queue,
            camera_buffer,
            camera_bind_group,
            fill_pipeline,
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
        let (verts, idxs) = build_geometry(document, include_artboard_bg);

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

        // Resolve target: single-sample, read back as PNG
        let tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("headless_tex"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let tex_view = tex.create_view(&Default::default());

        // MSAA render target (4×), resolved into tex_view
        let msaa_tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("headless_msaa_tex"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: MSAA_SAMPLES,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let msaa_view = msaa_tex.create_view(&Default::default());

        let mut enc = self.device.create_command_encoder(&Default::default());
        self.record_pass(&mut enc, &msaa_view, &tex_view, &verts, &idxs, clear);
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

fn build_geometry(doc: &Document, include_artboard_bg: bool) -> (Vec<Vertex>, Vec<u32>) {
    let mut verts: Vec<Vertex> = Vec::new();
    let mut idxs: Vec<u32> = Vec::new();

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

        // ── Drop shadow ──────────────────────────────────────────────────────
        // Offset, soft-edged silhouette beneath the node (matches the windowed
        // renderer so export agrees with the on-canvas result).
        if node.drop_shadow.enabled {
            let s = &node.drop_shadow;
            let (sr, sg, sb) = (s.color.r, s.color.g, s.color.b);
            let opacity = s.opacity * node.opacity;
            let (ox, oy) = (s.dx as f64, s.dy as f64);
            let mesh = tessellate_fill(&path_node.path_data, false);
            if !mesh.is_empty() {
                let base = verts.len() as u32;
                let fill_alpha = (s.color.a * opacity).min(1.0);
                for pos in &mesh.vertices {
                    let x = a * pos[0] as f64 + c * pos[1] as f64 + e + ox;
                    let y = b * pos[0] as f64 + d * pos[1] as f64 + f + oy;
                    verts.push(Vertex {
                        position: [x as f32, y as f32],
                        color: [sr, sg, sb, fill_alpha],
                    });
                }
                for &i in &mesh.indices {
                    idxs.push(base + i);
                }
            }
            // Soft edge via gaussian-falloff stroke expansion (mirrors append_glow).
            if s.blur > 0.0 {
                const STEPS: usize = 10;
                for i in (0..STEPS).rev() {
                    let t = (i + 1) as f32 / STEPS as f32;
                    let width = 2.0 * s.blur * t;
                    let step_alpha = (opacity * (-4.5 * t * t).exp() * s.color.a).min(1.0);
                    let smesh = tessellate_stroke(
                        &path_node.path_data,
                        width,
                        photonic_core::style::LineCap::Round,
                        photonic_core::style::LineJoin::Round,
                        4.0,
                    );
                    if smesh.is_empty() {
                        continue;
                    }
                    let base = verts.len() as u32;
                    for pos in &smesh.vertices {
                        let x = a * pos[0] as f64 + c * pos[1] as f64 + e + ox;
                        let y = b * pos[0] as f64 + d * pos[1] as f64 + f + oy;
                        verts.push(Vertex {
                            position: [x as f32, y as f32],
                            color: [sr, sg, sb, step_alpha],
                        });
                    }
                    for &idx in &smesh.indices {
                        idxs.push(base + idx);
                    }
                }
            }
        }

        // ── Object blur / feather: soft fill-colored edge (solid fills) ───────
        {
            let radius = if node.object_blur.enabled {
                node.object_blur.radius
            } else if node.feather.enabled {
                node.feather.radius
            } else {
                0.0
            };
            if let (FillKind::Solid(col), true) = (&path_node.fill.kind, radius > 0.0) {
                let alpha = col.a * path_node.fill.opacity * node.opacity;
                const STEPS: usize = 10;
                for i in (0..STEPS).rev() {
                    let t = (i + 1) as f32 / STEPS as f32;
                    let width = 2.0 * radius * t;
                    let step_alpha = (alpha * (-4.5 * t * t).exp()).min(1.0);
                    let smesh = tessellate_stroke(
                        &path_node.path_data,
                        width,
                        photonic_core::style::LineCap::Round,
                        photonic_core::style::LineJoin::Round,
                        4.0,
                    );
                    if smesh.is_empty() {
                        continue;
                    }
                    let base = verts.len() as u32;
                    for pos in &smesh.vertices {
                        let x = a * pos[0] as f64 + c * pos[1] as f64 + e;
                        let y = b * pos[0] as f64 + d * pos[1] as f64 + f;
                        verts.push(Vertex {
                            position: [x as f32, y as f32],
                            color: [col.r, col.g, col.b, step_alpha],
                        });
                    }
                    for &idx in &smesh.indices {
                        idxs.push(base + idx);
                    }
                }
            }
        }

        // ── Fill ─────────────────────────────────────────────────────────────
        if path_node.fill.enabled && !matches!(&path_node.fill.kind, FillKind::None) {
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

    (verts, idxs)
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
}
