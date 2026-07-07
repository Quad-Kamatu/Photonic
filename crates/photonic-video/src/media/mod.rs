//! Media services (02 §1 module map): probe, ffmpeg location, keyframe/pts
//! indices. Pure I/O over the ffmpeg toolchain; no GPU, no engine threads.
//!
//! Probing and index building **block** (they shell out to `ffprobe`); the
//! engine runs them on worker threads. Nothing in this module spawns threads.

pub mod ffmpeg_locate;
pub mod keyframe_index;
pub mod probe;

pub use ffmpeg_locate::{locate, FfmpegTools, LocateError, FFMPEG_DIR_ENV};
pub use keyframe_index::{cache_dir_for_project, IndexError, KeyframeIndex, PtsIndex};
pub use probe::{content_hash, probe_asset, probe_details, ProbeDetails, ProbeError};
