use super::*;

impl PhotonicRenderer {
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
        if let Some(surface) = &self.surface {
            surface.configure(&self.device, &self.surface_config);
        }
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
        // Windowless (offscreen) renderers never present — see `new_offscreen`.
        let surface = self.surface.as_ref()?;
        let surface_texture = match surface.get_current_texture() {
            Ok(t) => t,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                surface.configure(&self.device, &self.surface_config);
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
        self.render_scene(
            &mut encoder,
            &self.msaa_view,
            &view,
            self.width,
            self.height,
            vertices,
            indices,
        );
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
}
