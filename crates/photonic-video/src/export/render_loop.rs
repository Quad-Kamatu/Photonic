//! The export render loop shell (02-engine.md §7): "for frame f in work
//! range → compile graph at tick(f) → eval GPU → readback `Rgba16Float` →
//! convert to encoder pix_fmt → write to encoder sidecar stdin... cancellable
//! between frames... `ExportProgress { frame, total, fps, eta }` events."
//!
//! [`export_frames`] is deliberately **engine-independent**: it takes a
//! `frame_source` closure (`u64` tick index → [`Frame`]) rather than a
//! `VideoEngine`/graph handle, so it compiles and is fully testable (with
//! synthetic gradient/counter frames) before the P3 evaluator exists. The
//! evaluator builder wires a real closure in later — that integration point
//! is intentionally the only thing this module doesn't own.
//!
//! Per-frame retiming (05 §6.2: nearest-source-frame resampling when
//! `FrameRatePolicy::Explicit` differs from the sequence rate) is the
//! evaluator's responsibility, not this loop's — `export_frames` just asks
//! `frame_source` for `total_frames` consecutive *output* ticks (`0..total`)
//! and encodes exactly what comes back.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use photonic_core::timeline::FrameRate;
use photonic_render::color::Colorimetry;

use super::convert;
use super::encoder::{
    plane_kind_for, AudioStreamSpec, EncodeError, EncodeSpec, EncoderCapabilities, EncoderProcess,
    PlaneKind,
};
use super::presets::ExportPreset;
use crate::media::ffmpeg_locate::FfmpegTools;

/// One evaluated frame handed to the render loop: linear, premultiplied,
/// Rec.709 RGBA — the CPU-side `f32` shape of an `Rgba16Float` working-texture
/// readback (03 §4.4 rule 3 keeps CPU reference math in `f32`, not `f16`).
/// `rgba_premult.len()` must equal `width * height * 4`.
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgba_premult: Vec<f32>,
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ExportProgress {
    pub frame: u64,
    pub total: u64,
    pub fps: f32,
    pub eta: Duration,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExportEvent {
    Progress(ExportProgress),
    Done,
    Cancelled,
}

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("encode error: {0}")]
    Encode(#[from] EncodeError),
    #[error("frame {frame} is {got_w}x{got_h} but the export was resolved to {want_w}x{want_h}")]
    FrameSizeMismatch {
        frame: u64,
        got_w: u32,
        got_h: u32,
        want_w: u32,
        want_h: u32,
    },
}

/// Everything resolved from an [`ExportPreset`]'s abstract `ResolutionSpec`/
/// `FrameRatePolicy` (05 §3.2/§3.3) down to concrete numbers, plus the
/// output-color target and destination path. Resolving these from a
/// `Sequence`/`SequenceFormat` (01 §4) is the caller's job — this module only
/// consumes the resolved result.
pub struct ResolvedExport {
    pub width: u32,
    pub height: u32,
    pub frame_rate: FrameRate,
    pub audio: Option<AudioStreamSpec>,
    pub out_path: PathBuf,
    /// Target matrix/range for the YUV conversion (§3.5 step 4) — independent
    /// of the *source's* probed colorimetry (03 §3.5: "re-matrices, does not
    /// just relabel").
    pub colorimetry: Colorimetry,
}

/// Run one export job to completion (or until `cancel` is set). Spawns the
/// encoder, pulls `total_frames` frames from `frame_source` in order
/// (`0..total_frames`), converts each to the target pix_fmt, writes it to the
/// encoder, and reports progress/completion via `on_event`. Checks `cancel`
/// **between** frames (02 §7's "cancellable between frames"), never mid-write.
// Each parameter is a distinct, independently-meaningful input (tools/preset/
// resolved describe the job, total_frames/frame_source drive it, audio_samples
// is optional, cancel/on_event are the two callback channels) — a params
// struct would just relocate the count, and this fn is a public cross-crate
// entry point (called from photonic-mcp), so bundling isn't a local, clean win.
#[allow(clippy::too_many_arguments)]
pub fn export_frames(
    tools: &FfmpegTools,
    preset: &ExportPreset,
    resolved: &ResolvedExport,
    total_frames: u64,
    mut frame_source: impl FnMut(u64) -> Frame,
    audio_samples: Option<Vec<f32>>,
    cancel: &AtomicBool,
    mut on_event: impl FnMut(ExportEvent),
) -> Result<(), ExportError> {
    let caps = EncoderCapabilities::probe(tools)?;
    let spec = EncodeSpec {
        preset,
        width: resolved.width,
        height: resolved.height,
        frame_rate: resolved.frame_rate,
        audio: resolved.audio,
        out_path: resolved.out_path.clone(),
    };
    let mut proc = EncoderProcess::spawn(tools, &caps, &spec, audio_samples)?;

    let plane_kind = plane_kind_for(preset.video.as_ref().map(|v| v.codec), preset.alpha);
    let alpha_444 = plane_kind == PlaneKind::Yuva444;

    let start = Instant::now();
    for i in 0..total_frames {
        if cancel.load(Ordering::Relaxed) {
            proc.cancel();
            on_event(ExportEvent::Cancelled);
            return Ok(());
        }

        let frame = frame_source(i);
        if frame.width != resolved.width || frame.height != resolved.height {
            proc.cancel();
            return Err(ExportError::FrameSizeMismatch {
                frame: i,
                got_w: frame.width,
                got_h: frame.height,
                want_w: resolved.width,
                want_h: resolved.height,
            });
        }

        let planes = match plane_kind {
            PlaneKind::Rgba8 => {
                convert::working_frame_to_rgba8(&frame.rgba_premult, frame.width, frame.height)
            }
            _ => convert::working_frame_to_yuv_planes(
                &frame.rgba_premult,
                frame.width,
                frame.height,
                resolved.colorimetry,
                preset.alpha,
                alpha_444,
            ),
        };

        if let Err(e) = proc.write_video_frame(&planes) {
            // The encoder already exited (bad args, codec rejected the
            // stream, disk full, ...) — nothing left to cancel, just surface
            // the error; `EncoderProcess::drop` reaps the child.
            return Err(e.into());
        }

        let done = i + 1;
        let elapsed = start.elapsed().as_secs_f32().max(1e-6);
        let fps = done as f32 / elapsed;
        let remaining = total_frames.saturating_sub(done);
        let eta = if fps > 0.0 {
            Duration::from_secs_f32(remaining as f32 / fps)
        } else {
            Duration::ZERO
        };
        on_event(ExportEvent::Progress(ExportProgress {
            frame: done,
            total: total_frames,
            fps,
            eta,
        }));
    }

    proc.finish()?;
    on_event(ExportEvent::Done);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::presets::{
        AudioCodec, AudioEncodeSpec, Container, FrameRatePolicy, QualityMode, ResolutionSpec,
        VideoCodec, VideoEncodeSpec,
    };
    use crate::media::ffmpeg_locate::locate_for_test;

    macro_rules! tools_or_skip {
        () => {
            match locate_for_test() {
                Some(t) => t,
                None => {
                    eprintln!(
                        "ffmpeg/ffprobe not found — skipping export render_loop test at {}:{}",
                        file!(),
                        line!()
                    );
                    return;
                }
            }
        };
    }

    /// A moving horizontal gradient, premultiplied straight (alpha 1.0) —
    /// deterministic per-frame content so distinct frames are distinguishable.
    fn synthetic_gradient_frame(i: u64, w: u32, h: u32) -> Frame {
        let mut rgba = vec![0.0f32; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let idx = ((y * w + x) * 4) as usize;
                let t = ((x as u64 + i * 3) % w as u64) as f32 / w.max(1) as f32;
                rgba[idx] = t; // R ramps + shifts per frame
                rgba[idx + 1] = 1.0 - t;
                rgba[idx + 2] = 0.5;
                rgba[idx + 3] = 1.0;
            }
        }
        Frame {
            width: w,
            height: h,
            rgba_premult: rgba,
        }
    }

    fn tiny_preset() -> ExportPreset {
        ExportPreset {
            name: "render-loop-test".into(),
            container: Container::Mp4,
            video: Some(VideoEncodeSpec {
                codec: VideoCodec::H264,
                quality: QualityMode::Crf(28.0),
            }),
            audio: Some(AudioEncodeSpec {
                codec: AudioCodec::Aac,
                bitrate_kbps: Some(96),
            }),
            resolution: ResolutionSpec::SourceFormat,
            frame_rate: FrameRatePolicy::MatchSequence,
            alpha: false,
            faststart: false,
            loudness_target: None,
        }
    }

    fn tmp_out(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "photonic-render-loop-test-{}-{name}",
            std::process::id()
        ))
    }

    #[test]
    fn export_frames_emits_monotonic_progress_then_done_and_writes_output() {
        let tools = tools_or_skip!();
        let preset = tiny_preset();
        let out_path = tmp_out("progress.mp4");
        let _ = std::fs::remove_file(&out_path);
        let resolved = ResolvedExport {
            width: 16,
            height: 16,
            frame_rate: FrameRate::new(10, 1),
            audio: Some(AudioStreamSpec {
                sample_rate: 48_000,
                channels: 2,
            }),
            out_path: out_path.clone(),
            colorimetry: Colorimetry::BT709_LIMITED,
        };
        let audio = vec![0.0f32; 48_000 / 10 * 2 * 5]; // 5 frames' worth of silence
        let cancel = AtomicBool::new(false);
        let mut events = Vec::new();

        export_frames(
            &tools,
            &preset,
            &resolved,
            5,
            |i| synthetic_gradient_frame(i, 16, 16),
            Some(audio),
            &cancel,
            |e| events.push(e),
        )
        .expect("export succeeds");

        let progress: Vec<u64> = events
            .iter()
            .filter_map(|e| match e {
                ExportEvent::Progress(p) => Some(p.frame),
                _ => None,
            })
            .collect();
        assert_eq!(
            progress,
            vec![1, 2, 3, 4, 5],
            "progress frames are monotonic 1..=total"
        );
        assert_eq!(events.last(), Some(&ExportEvent::Done));
        assert!(out_path.exists(), "output file was written");
        assert!(std::fs::metadata(&out_path).unwrap().len() > 0);

        let _ = std::fs::remove_file(&out_path);
    }

    #[test]
    fn export_frames_stops_and_reports_cancelled_when_flag_is_set() {
        let tools = tools_or_skip!();
        let preset = tiny_preset();
        let out_path = tmp_out("cancel.mp4");
        let _ = std::fs::remove_file(&out_path);
        let resolved = ResolvedExport {
            width: 16,
            height: 16,
            frame_rate: FrameRate::new(10, 1),
            audio: None,
            out_path: out_path.clone(),
            colorimetry: Colorimetry::BT709_LIMITED,
        };
        let mut preset = preset;
        preset.audio = None;
        let cancel = AtomicBool::new(true); // cancel before the first frame
        let mut events = Vec::new();

        export_frames(
            &tools,
            &preset,
            &resolved,
            100,
            |i| synthetic_gradient_frame(i, 16, 16),
            None,
            &cancel,
            |e| events.push(e),
        )
        .expect("cancel is not an error");

        assert_eq!(events, vec![ExportEvent::Cancelled]);
        let _ = std::fs::remove_file(&out_path);
    }

    #[test]
    fn export_frames_rejects_a_frame_size_mismatch() {
        let tools = tools_or_skip!();
        let mut preset = tiny_preset();
        preset.audio = None;
        let out_path = tmp_out("mismatch.mp4");
        let _ = std::fs::remove_file(&out_path);
        let resolved = ResolvedExport {
            width: 16,
            height: 16,
            frame_rate: FrameRate::new(10, 1),
            audio: None,
            out_path: out_path.clone(),
            colorimetry: Colorimetry::BT709_LIMITED,
        };
        let cancel = AtomicBool::new(false);

        let result = export_frames(
            &tools,
            &preset,
            &resolved,
            3,
            |i| synthetic_gradient_frame(i, 8, 8), // wrong size on purpose
            None,
            &cancel,
            |_| {},
        );

        assert!(matches!(
            result,
            Err(ExportError::FrameSizeMismatch { frame: 0, .. })
        ));
        let _ = std::fs::remove_file(&out_path);
    }
}
