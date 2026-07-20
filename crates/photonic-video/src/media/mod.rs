//! Media services (02 §1 module map): probe, ffmpeg location, keyframe/pts
//! indices. Pure I/O over the ffmpeg toolchain; no GPU, no engine threads.
//!
//! Probing and index building **block** (they shell out to `ffprobe`); the
//! engine runs them on worker threads. Nothing in this module spawns threads.

pub mod ffmpeg_locate;
pub mod keyframe_index;
/// Import-ladder L3 poster stills (24-preview-media-load).
pub mod poster;
pub mod probe;
pub mod proxy;
/// Clip thumbnails + waveform loading, sidecar-cached (spec 15, NLE parity 10).
pub mod thumbnails;

pub use ffmpeg_locate::{locate, FfmpegTools, LocateError, FFMPEG_DIR_ENV};
pub use keyframe_index::{
    cache_dir_for_project, keyframe_cache_path, keyframe_index_ready, IndexError, KeyframeIndex,
    PtsIndex,
};
pub use poster::{ensure_poster, poster_cache_dir, poster_cache_path, poster_ready, PosterError};
pub use probe::{content_hash, probe_asset, probe_details, ProbeDetails, ProbeError};
pub use proxy::{
    generate_proxy, proxy_cache_dir, proxy_cache_path, resolve_decode_input, ProxyError,
};
pub use thumbnails::{
    DecodeThumbnailSource, RgbaThumb, ThumbHandle, ThumbnailCache, ThumbnailConfig,
    ThumbnailDecodeSpec, ThumbnailSource, WaveformCache, WaveformSource,
};
