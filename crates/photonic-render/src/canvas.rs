/// Viewport state: pan, zoom, and the canvas-to-screen transform.
#[derive(Debug, Clone)]
pub struct CanvasView {
    pub pan_x: f64,
    pub pan_y: f64,
    pub zoom: f64,
    pub screen_width: u32,
    pub screen_height: u32,
}

impl CanvasView {
    pub fn new(screen_width: u32, screen_height: u32) -> Self {
        Self {
            pan_x: 0.0,
            pan_y: 0.0,
            zoom: 1.0,
            screen_width,
            screen_height,
        }
    }

    /// Convert a canvas-space point to screen-space.
    pub fn canvas_to_screen(&self, x: f64, y: f64) -> (f64, f64) {
        (x * self.zoom + self.pan_x, y * self.zoom + self.pan_y)
    }

    /// Convert a screen-space point to canvas-space.
    pub fn screen_to_canvas(&self, sx: f64, sy: f64) -> (f64, f64) {
        ((sx - self.pan_x) / self.zoom, (sy - self.pan_y) / self.zoom)
    }

    /// Zoom in/out around a pivot point (screen coordinates).
    pub fn zoom_at(&mut self, factor: f64, pivot_sx: f64, pivot_sy: f64) {
        let (cx, cy) = self.screen_to_canvas(pivot_sx, pivot_sy);
        self.zoom *= factor;
        self.zoom = self.zoom.clamp(0.01, 64.0);
        self.pan_x = pivot_sx - cx * self.zoom;
        self.pan_y = pivot_sy - cy * self.zoom;
    }

    pub fn fit_to_rect(&mut self, rect_x: f64, rect_y: f64, rect_w: f64, rect_h: f64) {
        let scale_x = self.screen_width as f64 / rect_w;
        let scale_y = self.screen_height as f64 / rect_h;
        self.zoom = scale_x.min(scale_y) * 0.9;
        self.pan_x = (self.screen_width as f64 - rect_w * self.zoom) / 2.0 - rect_x * self.zoom;
        self.pan_y = (self.screen_height as f64 - rect_h * self.zoom) / 2.0 - rect_y * self.zoom;
    }
}

impl Default for CanvasView {
    fn default() -> Self {
        Self::new(1280, 720)
    }
}
