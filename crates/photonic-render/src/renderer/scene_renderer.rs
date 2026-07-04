use super::*;

impl PhotonicRenderer {
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
        clear: wgpu::Color,
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
                        load: wgpu::LoadOp::Clear(clear),
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

    /// Render the document to `target_view`, inserting the live-effects blur
    /// layer (drop shadow / object blur / feather) between the artboard
    /// background and the sharp shapes when any effect is active. With no
    /// effects this is the original single-pass document render.
    pub(crate) fn render_scene(
        &self,
        enc: &mut wgpu::CommandEncoder,
        msaa_view: &wgpu::TextureView,
        target_view: &wgpu::TextureView,
        w: u32,
        h: u32,
        vertices: &[Vertex],
        indices: &[u32],
    ) {
        if self.pending_blur_jobs.is_empty() {
            self.record_document_pass(enc, msaa_view, target_view, vertices, indices, BG);
            return;
        }

        // The artboard rect is the first 4 verts / 6 indices built by
        // build_geometry; render the rest (shapes) to a transparent offscreen
        // texture so the effects layer can sit beneath them.
        let skip = 6.min(indices.len());
        let doc_tex = self.make_fx_tex(w, h);
        let doc_view = doc_tex.create_view(&Default::default());
        self.record_document_pass(
            enc,
            msaa_view,
            &doc_view,
            vertices,
            &indices[skip..],
            wgpu::Color::TRANSPARENT,
        );

        let (fx_tex, fx_view) = self.render_effects_layer(enc, w, h);

        // Composite onto the target: background → artboard → effects → shapes.
        self.composite_effects(
            enc,
            target_view,
            vertices,
            &indices[..skip],
            &fx_view,
            &doc_view,
        );
        drop(fx_tex);
        drop(doc_tex);
    }
}
