//! FFmpeg-sidecar decode (02 §3, D-03).
//!
//! One persistent `ffmpeg` process per `(asset, quality)` streams headerless
//! `rawvideo` on a pipe; a reader parses it into [`DecodedFrame`]s keyed by an
//! exact presentation [`Tick`]; a [`FrameRing`](ring::FrameRing) buffers them
//! around the playhead; a [`scheduler::DecodeSource`] drives seek/fill.
//!
//! ## Layout
//! - [`sidecar`] — spawn/kill the ffmpeg process (kill-on-drop), args, stderr drain.
//! - [`reader`]  — frame-boundary parser on the pipe → [`DecodedFrame`], PTS derivation.
//! - [`ring`]    — per-source decoded-frame ring, pts-keyed, thread-safe handoff.
//! - [`scheduler`] — minimal seek + sequential-fill driver.
//!
//! The decoded planes match [`photonic_render::video::YuvPlanes`] (the GPU-upload
//! consumer) exactly, so a `DecodedFrame` hands straight to the render crate with
//! zero copy via [`DecodedPlanes::as_yuv_planes`].

pub mod reader;
pub mod ring;
pub mod scheduler;
pub mod sidecar;

use photonic_core::timeline::Tick;
use photonic_render::video::YuvPlanes;

pub use reader::{FrameReader, PtsModel};
pub use ring::{FrameRing, SharedRing};
pub use scheduler::DecodeSource;
pub use sidecar::{Sidecar, SidecarConfig};

/// Decode quality: the ring/cache and process are keyed by this so preview and
/// full-res streams don't collide (02 §3 "one process per (asset, quality)").
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum DecodeQuality {
    /// Preview (proxy allowed): 16 fwd / 4 back ring default.
    Preview,
    /// Full resolution (export / originals).
    Full,
}

/// Output pixel layout requested from ffmpeg. `Yuva444p` is chosen only when the
/// source carries alpha (02 §3); otherwise `Yuv420p` (half the chroma bytes).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum PixFmt {
    Yuv420p,
    Yuva444p,
}

impl PixFmt {
    /// The ffmpeg `-pix_fmt` token.
    pub fn ffmpeg_name(self) -> &'static str {
        match self {
            PixFmt::Yuv420p => "yuv420p",
            PixFmt::Yuva444p => "yuva444p",
        }
    }

    /// Bytes for one full frame at `width`×`height` in this layout. Chroma
    /// dimensions round up (`(w+1)/2`) so odd sizes are handled.
    pub fn frame_bytes(self, width: u32, height: u32) -> usize {
        let (w, h) = (width as usize, height as usize);
        match self {
            PixFmt::Yuv420p => {
                let cw = w.div_ceil(2);
                let ch = h.div_ceil(2);
                w * h + 2 * cw * ch
            }
            PixFmt::Yuva444p => 4 * w * h,
        }
    }

    /// Pick the layout for a source: `Yuva444p` iff it carries alpha.
    pub fn for_alpha(has_alpha: bool) -> PixFmt {
        if has_alpha {
            PixFmt::Yuva444p
        } else {
            PixFmt::Yuv420p
        }
    }
}

/// One decoded frame with its exact presentation tick and owned planes.
#[derive(Clone, Debug, PartialEq)]
pub struct DecodedFrame {
    pub pts: Tick,
    pub planes: DecodedPlanes,
}

/// Owned YUV(+A) plane data. Borrowed as [`YuvPlanes`] for GPU upload with no
/// copy; the enum shape mirrors the render crate's consumer exactly.
#[derive(Clone, Debug, PartialEq)]
pub enum DecodedPlanes {
    Yuv420 {
        width: u32,
        height: u32,
        y: Vec<u8>,
        cb: Vec<u8>,
        cr: Vec<u8>,
    },
    Yuva444 {
        width: u32,
        height: u32,
        y: Vec<u8>,
        cb: Vec<u8>,
        cr: Vec<u8>,
        a: Vec<u8>,
    },
}

impl DecodedPlanes {
    pub fn dims(&self) -> (u32, u32) {
        match *self {
            DecodedPlanes::Yuv420 { width, height, .. }
            | DecodedPlanes::Yuva444 { width, height, .. } => (width, height),
        }
    }

    /// Borrow as the render crate's [`YuvPlanes`] for GPU upload (zero copy).
    pub fn as_yuv_planes(&self) -> YuvPlanes<'_> {
        match self {
            DecodedPlanes::Yuv420 {
                width,
                height,
                y,
                cb,
                cr,
            } => YuvPlanes::Yuv420 {
                width: *width,
                height: *height,
                y,
                cb,
                cr,
            },
            DecodedPlanes::Yuva444 {
                width,
                height,
                y,
                cb,
                cr,
                a,
            } => YuvPlanes::Yuva444 {
                width: *width,
                height: *height,
                y,
                cb,
                cr,
                a,
            },
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("failed to spawn ffmpeg: {0}")]
    Spawn(#[source] std::io::Error),
    #[error("pipe read error: {0}")]
    Io(#[source] std::io::Error),
    #[error("ffmpeg stream ended mid-frame (got {got} of {expected} bytes)")]
    PartialFrame { got: usize, expected: usize },
    #[error("decode process exhausted its restart budget ({max}); last error: {last}")]
    RestartsExhausted { max: u32, last: String },
    #[error("no keyframe index / pts model available for pts-true decode")]
    NoPtsModel,
    #[error("decoder produced no frames for the requested seek")]
    EmptyDecode,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_bytes_yuv420_and_yuva444() {
        // 320×180 4:2:0 = 320*180 + 2*160*90 = 57600 + 28800 = 86400.
        assert_eq!(PixFmt::Yuv420p.frame_bytes(320, 180), 86_400);
        // 4:4:4:A = 4*320*180 = 230400.
        assert_eq!(PixFmt::Yuva444p.frame_bytes(320, 180), 230_400);
    }

    #[test]
    fn frame_bytes_rounds_odd_dims() {
        // 3×3 4:2:0: 9 + 2*(2*2) = 9 + 8 = 17.
        assert_eq!(PixFmt::Yuv420p.frame_bytes(3, 3), 17);
    }

    #[test]
    fn pix_fmt_for_alpha() {
        assert_eq!(PixFmt::for_alpha(true), PixFmt::Yuva444p);
        assert_eq!(PixFmt::for_alpha(false), PixFmt::Yuv420p);
    }

    #[test]
    fn planes_borrow_as_yuv_planes() {
        let p = DecodedPlanes::Yuv420 {
            width: 2,
            height: 2,
            y: vec![1, 2, 3, 4],
            cb: vec![128],
            cr: vec![128],
        };
        match p.as_yuv_planes() {
            YuvPlanes::Yuv420 { width, y, .. } => {
                assert_eq!(width, 2);
                assert_eq!(y, &[1, 2, 3, 4]);
            }
            _ => panic!("wrong variant"),
        }
    }
}
