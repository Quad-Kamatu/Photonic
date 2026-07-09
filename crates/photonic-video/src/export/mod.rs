//! Export render loop + encoder sidecar + presets (02-engine.md §7,
//! 05-import-export.md §3, 03-render-color-pipeline.md §3.5).
//!
//! - [`presets`] — `ExportPreset` schema, the §3.5 built-in catalog, §3.4
//!   alpha-allow-list validation, app-level preset persistence (§3.6).
//! - [`convert`] — working format (`Rgba16Float` readback) → encoder pix_fmt
//!   (§3.5 steps 2-6), the CPU-side inverse of decode's YUV→working pass.
//! - [`encoder`] — the ffmpeg encode sidecar: process/arg building, codec
//!   capability probing, the video-stdin + audio-FIFO dual-input pipe.
//! - [`render_loop`] — the engine-independent `export_frames` shell (02 §7)
//!   that the P3 evaluator feeds.

pub mod convert;
pub mod encoder;
pub mod presets;
pub mod render_loop;
