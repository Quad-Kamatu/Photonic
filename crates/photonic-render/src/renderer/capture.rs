use super::*;

impl PhotonicRenderer {
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

        // Draw geometry into the offscreen texture via MSAA (with effects layer).
        let mut enc = self.device.create_command_encoder(&Default::default());
        self.render_scene(
            &mut enc,
            &capture_msaa_view,
            &tex_view,
            w,
            h,
            vertices,
            indices,
        );

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
                let glyph_style = match snap.font_style {
                    1 => GlyphonStyle::Italic,
                    2 => GlyphonStyle::Oblique,
                    _ => GlyphonStyle::Normal,
                };
                let attrs = Attrs::new()
                    .family(Family::Name(&snap.font_family))
                    .weight(Weight(snap.font_weight))
                    .style(glyph_style);
                buf.set_text(
                    &mut self.font_system,
                    &snap.content,
                    attrs,
                    Shaping::Advanced,
                );
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
                    top: snap.screen_y + snap.top_offset,
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

