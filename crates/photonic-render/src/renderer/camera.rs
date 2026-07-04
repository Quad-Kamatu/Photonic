use super::*;

impl PhotonicRenderer {
    // ── Private helpers ───────────────────────────────────────────────────────

    pub(crate) fn push_camera(&self) {
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
}
