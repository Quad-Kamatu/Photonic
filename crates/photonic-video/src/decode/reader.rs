//! Frame-boundary parser over the `rawvideo` pipe → [`DecodedFrame`] (02 §3/§4).
//!
//! `rawvideo` is headerless: frames are back-to-back plane bytes with no
//! timestamps, so the reader (a) slices the stream into fixed
//! [`PixFmt::frame_bytes`]-sized frames and (b) derives each frame's exact
//! presentation [`Tick`] itself.
//!
//! ## PTS derivation — why two models (02 §4 "pts-true")
//!
//! The pipe carries no PTS, so we reconstruct it. For a **constant-frame-rate**
//! source this is exact arithmetic at zero extra cost: the first output frame is
//! the seek keyframe (input `-ss` lands exactly on it), so
//! `pts(i) = (start_frame + i) × ticks_per_frame`. This keeps the cold-seek
//! budget (02 §8) — no second probe pass.
//!
//! For a **variable-frame-rate** source that arithmetic is wrong (the per-frame
//! interval changes — e.g. `vfr_sample.mp4` steps 24→30 fps mid-file). There is
//! no way to recover true PTS from the headerless pipe, so decode falls back to
//! a ground-truth [`PtsIndex`](crate::media::PtsIndex) built once via ffprobe and
//! cached in the sidecar dir; the reader maps output ordinal → absolute source
//! frame → `table[frame]`. This is the pts-true path 02 §4 requires; it costs one
//! extra probe pass, paid only for genuinely variable sources.

use std::io::Read;
use std::sync::Arc;

use photonic_core::timeline::{FrameRate, Tick};

use super::{DecodeError, DecodedFrame, DecodedPlanes, PixFmt};
use crate::media::keyframe_index::PtsIndex;

/// How the reader assigns a presentation tick to output frame `i`.
#[derive(Clone, Debug)]
pub enum PtsModel {
    /// Constant-frame-rate: `pts(i) = (start_frame + i) × ticks_per_frame`.
    /// `start_frame` is the seek keyframe's source frame ordinal.
    Cfr { rate: FrameRate, start_frame: i64 },
    /// Variable-frame-rate: look up the absolute source frame in the pts table.
    /// `pts(i) = table[start_frame + i]`.
    Table {
        start_frame: i64,
        table: Arc<PtsIndex>,
    },
}

impl PtsModel {
    /// Presentation tick for the `i`-th output frame since the seek origin.
    fn pts(&self, i: i64) -> Option<Tick> {
        match self {
            PtsModel::Cfr { rate, start_frame } => {
                let frame = start_frame + i;
                Some(Tick(frame * rate.ticks_per_frame().0))
            }
            PtsModel::Table { start_frame, table } => table.pts_at(start_frame + i),
        }
    }
}

/// Splits the rawvideo pipe into presentation-timed frames.
pub struct FrameReader<R: Read> {
    reader: R,
    width: u32,
    height: u32,
    pix_fmt: PixFmt,
    frame_bytes: usize,
    /// Output ordinal since the seek origin (0-based).
    ordinal: i64,
    pts_model: PtsModel,
}

impl<R: Read> FrameReader<R> {
    pub fn new(reader: R, width: u32, height: u32, pix_fmt: PixFmt, pts_model: PtsModel) -> Self {
        FrameReader {
            reader,
            width,
            height,
            pix_fmt,
            frame_bytes: pix_fmt.frame_bytes(width, height),
            ordinal: 0,
            pts_model,
        }
    }

    /// Read the next frame. `Ok(None)` on a clean end-of-stream (frame boundary);
    /// `Err(PartialFrame)` if the stream ends mid-frame (a crash mid-decode).
    pub fn next_frame(&mut self) -> Result<Option<DecodedFrame>, DecodeError> {
        let mut buf = vec![0u8; self.frame_bytes];
        let mut filled = 0;
        while filled < self.frame_bytes {
            match self.reader.read(&mut buf[filled..]) {
                Ok(0) => {
                    if filled == 0 {
                        return Ok(None); // clean EOF at a frame boundary
                    }
                    return Err(DecodeError::PartialFrame {
                        got: filled,
                        expected: self.frame_bytes,
                    });
                }
                Ok(n) => filled += n,
                Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(DecodeError::Io(e)),
            }
        }

        let pts = self
            .pts_model
            .pts(self.ordinal)
            .ok_or(DecodeError::NoPtsModel)?;
        self.ordinal += 1;

        let planes = self.split_planes(buf);
        Ok(Some(DecodedFrame { pts, planes }))
    }

    /// The presentation tick the *next* frame will carry (without reading it).
    pub fn next_pts(&self) -> Option<Tick> {
        self.pts_model.pts(self.ordinal)
    }

    fn split_planes(&self, buf: Vec<u8>) -> DecodedPlanes {
        let (w, h) = (self.width as usize, self.height as usize);
        match self.pix_fmt {
            PixFmt::Yuv420p => {
                let cw = w.div_ceil(2);
                let ch = h.div_ceil(2);
                let y_len = w * h;
                let c_len = cw * ch;
                let y = buf[..y_len].to_vec();
                let cb = buf[y_len..y_len + c_len].to_vec();
                let cr = buf[y_len + c_len..y_len + 2 * c_len].to_vec();
                DecodedPlanes::Yuv420 {
                    width: self.width,
                    height: self.height,
                    y,
                    cb,
                    cr,
                }
            }
            PixFmt::Yuva444p => {
                let p = w * h;
                let y = buf[..p].to_vec();
                let cb = buf[p..2 * p].to_vec();
                let cr = buf[2 * p..3 * p].to_vec();
                let a = buf[3 * p..4 * p].to_vec();
                DecodedPlanes::Yuva444 {
                    width: self.width,
                    height: self.height,
                    y,
                    cb,
                    cr,
                    a,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::keyframe_index::PtsIndex;

    /// A CFR reader over an in-memory two-frame yuv420p 2×2 stream derives the
    /// right ticks: frame N (start_frame=60, 30fps) then N+1.
    #[test]
    fn cfr_pts_and_plane_split() {
        let fb = PixFmt::Yuv420p.frame_bytes(2, 2); // 2*2 + 2*1 = 6
        assert_eq!(fb, 6);
        let mut bytes = Vec::new();
        // frame 0 planes: Y=[1,2,3,4] Cb=[100] Cr=[200]
        bytes.extend_from_slice(&[1, 2, 3, 4, 100, 200]);
        // frame 1 planes: Y=[5,6,7,8] Cb=[101] Cr=[201]
        bytes.extend_from_slice(&[5, 6, 7, 8, 101, 201]);

        let rate = FrameRate::FPS_30;
        let tpf = rate.ticks_per_frame().0;
        let mut r = FrameReader::new(
            std::io::Cursor::new(bytes),
            2,
            2,
            PixFmt::Yuv420p,
            PtsModel::Cfr {
                rate,
                start_frame: 60,
            },
        );

        let f0 = r.next_frame().unwrap().unwrap();
        assert_eq!(f0.pts, Tick(60 * tpf));
        match &f0.planes {
            DecodedPlanes::Yuv420 { y, cb, cr, .. } => {
                assert_eq!(y, &[1, 2, 3, 4]);
                assert_eq!(cb, &[100]);
                assert_eq!(cr, &[200]);
            }
            // Test invariant: a Yuv420p reader only ever yields Yuv420 planes.
            other => panic!("expected Yuv420 planes, got {other:?}"),
        }

        let f1 = r.next_frame().unwrap().unwrap();
        assert_eq!(f1.pts, Tick(61 * tpf));

        assert!(r.next_frame().unwrap().is_none(), "clean EOF");
    }

    #[test]
    fn table_pts_model_uses_source_frame() {
        let table = Arc::new(PtsIndex {
            pts: vec![Tick(0), Tick(10), Tick(20), Tick(30), Tick(45)],
        });
        // Seek origin at source frame 2 → first output frame = table[2] = 20.
        let fb = PixFmt::Yuv420p.frame_bytes(2, 2);
        let bytes = vec![0u8; fb * 2];
        let mut r = FrameReader::new(
            std::io::Cursor::new(bytes),
            2,
            2,
            PixFmt::Yuv420p,
            PtsModel::Table {
                start_frame: 2,
                table,
            },
        );
        assert_eq!(r.next_frame().unwrap().unwrap().pts, Tick(20));
        assert_eq!(r.next_frame().unwrap().unwrap().pts, Tick(30));
    }

    #[test]
    fn partial_frame_is_an_error() {
        // 5 bytes for a 6-byte frame → crash mid-frame.
        let mut r = FrameReader::new(
            std::io::Cursor::new(vec![1, 2, 3, 4, 5]),
            2,
            2,
            PixFmt::Yuv420p,
            PtsModel::Cfr {
                rate: FrameRate::FPS_30,
                start_frame: 0,
            },
        );
        assert!(matches!(
            r.next_frame(),
            Err(DecodeError::PartialFrame {
                got: 5,
                expected: 6
            })
        ));
    }
}
