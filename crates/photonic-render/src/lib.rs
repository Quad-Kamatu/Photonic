pub mod canvas;
pub mod color;
pub mod compositor;
pub mod headless;
mod headless_text;
pub mod pipeline;
pub mod renderer;
pub mod tessellator;
pub mod text_outline;
pub mod text_path;
pub mod video;

pub use canvas::CanvasView;
pub use color::{Colorimetry, Matrix, Range};
pub use headless::{
    document_needs_cpu_compositor, ExportBackground, ExportOptions, HeadlessRenderer,
};
pub use renderer::PhotonicRenderer;
pub use text_outline::{layout_text_flat, outline_document_text, resolve_document_font, ResolvedFace};
