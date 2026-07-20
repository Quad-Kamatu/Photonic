use super::*;

/// Clamp a requested surface size to the device's maximum 2D texture dimension.
///
/// wgpu panics when a surface — or any texture derived from it (our MSAA /
/// glow / effect targets) — exceeds `max_texture_dimension_2d`, and some
/// compositors momentarily report absurd sizes during monitor hotplug or
/// fractional-scale changes. Clamping (rather than panicking or dropping the
/// frame) keeps the canvas alive at the device's largest supported size.
pub(crate) fn clamp_surface_dims(width: u32, height: u32, max_dim: u32) -> (u32, u32) {
    (width.min(max_dim), height.min(max_dim))
}

impl PhotonicRenderer {
    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        // Clamp before doing anything else: an oversized width/height would
        // panic `surface.configure` and every `create_*_texture` call below.
        // Clamping the stored dimensions too keeps the surface, the derived
        // textures, and the camera/view all in agreement.
        let (width, height) =
            clamp_surface_dims(width, height, self.device.limits().max_texture_dimension_2d);
        // No-op when the size is unchanged. Compositors (notably KWin on
        // multi-monitor / fractional scaling) can emit a stream of `Resized`
        // events carrying the *same* dimensions; reconfiguring the surface +
        // reallocating MSAA/glow textures on each one causes a relayout storm
        // (the timeline appears to resize endlessly) and needless surface
        // churn. Bailing here breaks that loop and only does real work on a
        // genuine size change. Compared post-clamp so a stream of oversized
        // events that all clamp to the same size also collapses to one resize.
        if (width, height) == (self.width, self.height) {
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
        let (verts, idxs) = self.build_geometry();
        // Size the persistent document buffers to this frame's geometry before
        // the render pass (which only has `&self`) uploads into them (03 §2.3).
        self.ensure_doc_buffers(
            std::mem::size_of_val(verts.as_slice()) as u64,
            std::mem::size_of_val(idxs.as_slice()) as u64,
        );
        (verts, idxs)
    }

    /// Acquire the swapchain frame and record the document render pass into
    /// `FrameHandle::encoder`. Returns `None` on a transient timeout and an
    /// error only when the surface stays invalid after a reconfigure + retry.
    ///
    /// Callers may append further render passes (e.g. egui) to `handle.encoder`
    /// before calling `finish_frame`.
    pub fn begin_frame(
        &self,
        vertices: &[Vertex],
        indices: &[u32],
    ) -> anyhow::Result<Option<FrameHandle>> {
        // Windowless (offscreen) renderers never present — see `new_offscreen`.
        let Some(surface) = self.surface.as_ref() else {
            return Ok(None);
        };
        let surface_texture = match surface.get_current_texture() {
            Ok(t) => t,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                // The compositor invalidated the swapchain (a resize, DPI/scale
                // change, or the window moving to another monitor). This is
                // routine and recoverable: reconfigure with the current config
                // — `surface_config` is kept current by `resize`, and its
                // `present_mode` was validated against this adapter's caps at
                // construction (see `choose_surface_config`; caps are stable for
                // the life of an adapter+surface) — then retry acquisition once.
                surface.configure(&self.device, &self.surface_config);
                match surface.get_current_texture() {
                    Ok(t) => t,
                    // Still not ready after reconfigure — treat a timeout as a
                    // skipped frame rather than a hard error.
                    Err(wgpu::SurfaceError::Timeout) => return Ok(None),
                    Err(e) => {
                        return Err(anyhow::anyhow!(
                            "window surface still invalid after reconfigure: {e:?}"
                        ));
                    }
                }
            }
            Err(wgpu::SurfaceError::Timeout) => return Ok(None),
            Err(e) => {
                return Err(anyhow::anyhow!("failed to acquire window surface: {e:?}"));
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
        Ok(Some(FrameHandle {
            surface_texture,
            view,
            encoder,
        }))
    }

    /// Submit the frame encoder and present the surface texture.
    pub fn finish_frame(&self, handle: FrameHandle) {
        self.queue.submit([handle.encoder.finish()]);
        handle.surface_texture.present();
    }
}

#[cfg(test)]
mod tests {
    use super::clamp_surface_dims;

    #[test]
    fn clamps_each_axis_independently_to_the_device_limit() {
        let max = 8192;
        // Within bounds is untouched.
        assert_eq!(clamp_surface_dims(1920, 1080, max), (1920, 1080));
        // Exactly at the limit is kept.
        assert_eq!(clamp_surface_dims(max, max, max), (max, max));
        // Each axis clamps on its own; a bogus oversized value can't panic
        // surface.configure or the derived MSAA/glow texture allocations.
        assert_eq!(clamp_surface_dims(100_000, 1080, max), (max, 1080));
        assert_eq!(clamp_surface_dims(1920, 100_000, max), (1920, max));
        assert_eq!(clamp_surface_dims(u32::MAX, u32::MAX, max), (max, max));
    }
}
