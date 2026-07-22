//! The single end-to-end export path (02 §7, 10 §6): resolve an
//! [`ExportJob`]'s abstract preset (`ResolutionSpec`/`FrameRatePolicy`) against
//! a frozen [`TimelineProject`] snapshot, drive a **dedicated** headless
//! [`EngineSession`] over that snapshot, and feed one full-quality readback
//! frame per output tick into [`render_loop::export_frames`].
//!
//! Both callers — the GUI (`EngineCmd::Export`, via
//! [`crate::session`]) and the MCP `export_sequence` tool — funnel through
//! [`run_export_job`], so there is exactly ONE render/encode path. SS-3
//! determinism requires this: export always evaluates at full quality with
//! [`ProxyMode::ForceOriginal`], and wall-clock never influences output — the
//! only clock in the loop is a per-frame 30 s deadline that *poisons* the run
//! (surfaces [`ExportError::RenderTimeout`]) rather than substituting a frame.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use photonic_core::history::CommandHistory;
use photonic_core::timeline::{FrameRate, SequenceId, Tick, TimelineProject};
use photonic_core::Document;
use photonic_render::color::Colorimetry;

use super::encoder::AudioStreamSpec;
use super::offline_audio::{self, DEFAULT_EXPORT_SAMPLE_RATE};
use super::presets::{self, ExportPreset, FrameRatePolicy, ResolutionSpec};
use super::render_loop::{self, ExportError, ExportEvent, Frame, ResolvedExport};
use crate::graph::eval::{read_texture_rgba16f, GpuContext};
use crate::media::ffmpeg_locate::FfmpegTools;
use crate::session::{EngineCmd, ExportJob, ProxyMode, VideoEngine};

/// Everything an [`ExportJob`]'s abstract preset resolves to against a concrete
/// sequence (05 §3.2/§3.3): concrete output/format sizes, the output frame
/// rate, the exact work range, the frame count, and the (unmodified) preset —
/// audio is kept when the preset requests it (K-0.7).
#[derive(Clone, Debug)]
pub struct ResolvedExportJob {
    pub format_index: usize,
    /// The sequence-format size the engine evaluates at (`fw`, `fh`).
    pub format_size: (u32, u32),
    /// The encoded output size (`ow`, `oh`); `<= format_size` (no upscaling).
    pub out_size: (u32, u32),
    pub seq_rate: FrameRate,
    pub out_rate: FrameRate,
    pub start: Tick,
    pub end: Tick,
    pub total_frames: u64,
    /// The preset after resolution overrides. Audio is preserved (K-0.7).
    pub preset: ExportPreset,
    pub out_path: PathBuf,
}

/// Resolve an [`ExportJob`] against its `project` snapshot into concrete
/// numbers (02 §7 / 05 §3.2). Pure and cheap — callers use it up front to
/// surface validation errors synchronously (bad sequence/format, empty range,
/// upscaling refusal, invalid preset) and to report the frame count, then hand
/// the *same* job to [`run_export_job`], which re-resolves identically.
pub fn resolve_export_job(
    project: &TimelineProject,
    job: &ExportJob,
) -> Result<ResolvedExportJob, ExportError> {
    let seq = project.sequences.get(&job.sequence).ok_or_else(|| {
        ExportError::Resolve(format!("sequence {} not found", job.sequence))
    })?;
    let seq_rate = seq.frame_rate;
    if seq.formats.is_empty() {
        return Err(ExportError::Resolve(format!(
            "sequence {} has no formats",
            job.sequence
        )));
    }
    let format_index = job.format_index.min(seq.formats.len().saturating_sub(1));
    let (fw, fh) = (
        seq.formats[format_index].width.max(1),
        seq.formats[format_index].height.max(1),
    );

    let content_end = seq
        .video_tracks
        .iter()
        .chain(seq.audio_tracks.iter())
        .flat_map(|t| t.clips.iter())
        .map(|c| c.end())
        .max()
        .unwrap_or(Tick(0));
    let (start, end) = job
        .range
        .unwrap_or_else(|| seq.work_range.unwrap_or((Tick(0), content_end)));
    if end <= start {
        return Err(ExportError::Resolve(
            "export range is empty — sequence has no content and no explicit range was given"
                .into(),
        ));
    }

    let out_rate = match job.preset.frame_rate {
        FrameRatePolicy::Explicit(fr) => fr,
        FrameRatePolicy::MatchSequence => seq_rate,
    };
    let (out_w, out_h) = match job.preset.resolution {
        ResolutionSpec::SourceFormat => (fw, fh),
        ResolutionSpec::Explicit { w, h } => (w.max(1), h.max(1)),
        ResolutionSpec::Scale(s) => (
            ((fw as f32 * s).round() as u32).max(1),
            ((fh as f32 * s).round() as u32).max(1),
        ),
    };
    if out_w > fw || out_h > fh {
        return Err(ExportError::Resolve(format!(
            "export upscaling ({out_w}x{out_h} from a {fw}x{fh} format) is not supported \
             in P3 — the engine evaluates at format size and downscales only"
        )));
    }

    // Keep the preset's audio spec (K-0.7): offline mix + mux lands in
    // `run_export_job`. Validation still rejects alpha-incompatible containers etc.
    let preset = job.preset.clone();
    presets::validate(&preset)
        .map_err(|e| ExportError::Resolve(format!("preset invalid after overrides: {e}")))?;

    let tpf_out = out_rate.ticks_per_frame().0.max(1);
    let total_frames = ((end.0 - start.0 + tpf_out - 1) / tpf_out).max(1) as u64;

    Ok(ResolvedExportJob {
        format_index,
        format_size: (fw, fh),
        out_size: (out_w, out_h),
        seq_rate,
        out_rate,
        start,
        end,
        total_frames,
        preset,
        out_path: job.output.clone(),
    })
}

/// Run one export job to completion (02 §7, 10 §6). Opens a **dedicated**
/// [`EngineSession`] over a frozen clone of `project` (isolated from any
/// interactive session, so concurrent playback/render can't disturb the
/// seek-then-wait loop), forces full-quality [`ProxyMode::ForceOriginal`], and
/// feeds [`render_loop::export_frames`] one readback frame per output tick.
///
/// Each output tick is snapped to the nearest sequence-grid frame (05 §6.2),
/// seeked, and awaited for a fresh matching frame with a 30 s deadline; a miss
/// poisons the run (cancels between frames, returns
/// [`ExportError::RenderTimeout`]) rather than substituting content. Progress /
/// completion is reported via `on_event`; `cancel` stops the loop between
/// frames.
pub fn run_export_job(
    gpu: GpuContext,
    project: Arc<TimelineProject>,
    job: &ExportJob,
    tools: &FfmpegTools,
    cancel: &AtomicBool,
    on_event: impl FnMut(ExportEvent),
) -> Result<(), ExportError> {
    let r = resolve_export_job(&project, job)?;

    // Frozen shadow project with the export format forced active.
    let mut frozen = (*project).clone();
    if let Some(s) = frozen.sequences.get_mut(&job.sequence) {
        s.active_format = r.format_index;
    }

    let engine = VideoEngine::new(gpu.clone());
    let shadow_doc = {
        let mut d = Document::new("export-shadow", 1.0, 1.0);
        d.timeline = Some(frozen);
        Arc::new(Mutex::new(d))
    };
    let shadow_history = Arc::new(Mutex::new(CommandHistory::new(1)));
    let session = engine.open_session(shadow_doc, shadow_history);
    let seq_id: SequenceId = job.sequence;
    session.send(EngineCmd::SetActiveSequence(seq_id));
    // Export is always full quality (02 §7: identical headless path).
    session.send(EngineCmd::SetProxyMode(ProxyMode::ForceOriginal));

    // Offline mix for the export range (K-0.7). Silence when tools are missing
    // still yields a length-correct PCM buffer so containers with an audio slot
    // get a valid silent track rather than a broken mux.
    let (audio_spec, audio_samples) = if r.preset.audio.is_some() {
        let sample_rate = if project.settings.audio_sample_rate > 0 {
            project.settings.audio_sample_rate
        } else {
            DEFAULT_EXPORT_SAMPLE_RATE
        };
        let pcm = offline_audio::render_export_audio(
            &project,
            job.sequence,
            r.start,
            r.end,
            Some(tools),
            r.preset.loudness_target.as_ref(),
        )?;
        (
            Some(AudioStreamSpec {
                sample_rate,
                channels: 2,
            }),
            Some(pcm),
        )
    } else {
        (None, None)
    };

    let resolved = ResolvedExport {
        width: r.out_size.0,
        height: r.out_size.1,
        frame_rate: r.out_rate,
        audio: audio_spec,
        out_path: r.out_path.clone(),
        colorimetry: Colorimetry::BT709_LIMITED,
    };
    let (fw, fh) = r.format_size;
    let (ow, oh) = r.out_size;
    let start = r.start;
    let seq_rate = r.seq_rate;
    let tpf_out = r.out_rate.ticks_per_frame().0.max(1);

    let mut prev = session.latest_frame();
    let mut frame_fail: Option<String> = None;
    let frame_source = |i: u64| -> Frame {
        // Output tick → nearest sequence frame (05 §6.2 retiming: the engine
        // presents exact sequence-grid ticks, so snap the output-grid tick).
        let t = Tick(start.0 + i as i64 * tpf_out);
        let snapped = seq_rate.frame_start(seq_rate.frame_at(t));
        session.send(EngineCmd::Seek(snapped));
        let deadline = Instant::now() + Duration::from_secs(30);
        let frame = loop {
            if let Some(f) = session.latest_frame() {
                let fresh = prev
                    .as_ref()
                    .map(|q| !Arc::ptr_eq(q, &f))
                    .unwrap_or(true);
                if fresh && f.time == snapped && f.sequence == seq_id {
                    break Some(f);
                }
            }
            if Instant::now() >= deadline {
                break None;
            }
            std::thread::sleep(Duration::from_millis(2));
        };
        match frame {
            Some(f) => {
                prev = Some(Arc::clone(&f));
                // Logical region only (bucket-padded texture, see render loop).
                let px = read_texture_rgba16f(&gpu, &f.texture, fw, fh);
                let px = if (ow, oh) == (fw, fh) {
                    px
                } else {
                    box_downscale(&px, fw, fh, ow, oh)
                };
                Frame {
                    width: ow,
                    height: oh,
                    rgba_premult: px.iter().flat_map(|q| q.iter().copied()).collect(),
                }
            }
            None => {
                // Poison the run: flag the failure, cancel between frames
                // (export_frames checks the flag before the next frame), and
                // hand back a black filler so the closure contract holds.
                frame_fail = Some(format!(
                    "frame {i} (tick {}) was not produced within 30s",
                    snapped.0
                ));
                cancel.store(true, Ordering::Relaxed);
                Frame {
                    width: ow,
                    height: oh,
                    rgba_premult: vec![0.0; (ow * oh * 4) as usize],
                }
            }
        }
    };

    let result = render_loop::export_frames(
        tools,
        &r.preset,
        &resolved,
        r.total_frames,
        frame_source,
        audio_samples,
        cancel,
        on_event,
    );
    session.shutdown();

    match (result, frame_fail) {
        // A frame timeout poisoned the run — surface it even though
        // `export_frames` returned Ok(Cancelled) off the poisoned flag.
        (_, Some(msg)) => Err(ExportError::RenderTimeout(msg)),
        (Err(e), None) => Err(e),
        (Ok(()), None) => Ok(()),
    }
}

/// Area-average box downscale of a linear-premultiplied RGBA buffer from
/// `w`×`h` to `ow`×`oh` (`ow <= w`, `oh <= h`). The export path only ever
/// downscales (upscaling is refused in [`resolve_export_job`]).
fn box_downscale(src: &[[f32; 4]], w: u32, h: u32, ow: u32, oh: u32) -> Vec<[f32; 4]> {
    let mut out = Vec::with_capacity((ow * oh) as usize);
    for oy in 0..oh {
        let y0 = (oy as u64 * h as u64 / oh as u64) as u32;
        let y1 = (((oy as u64 + 1) * h as u64).div_ceil(oh as u64) as u32).clamp(y0 + 1, h);
        for ox in 0..ow {
            let x0 = (ox as u64 * w as u64 / ow as u64) as u32;
            let x1 = (((ox as u64 + 1) * w as u64).div_ceil(ow as u64) as u32).clamp(x0 + 1, w);
            let mut acc = [0f64; 4];
            for y in y0..y1 {
                for x in x0..x1 {
                    let p = src[(y * w + x) as usize];
                    for (a, c) in acc.iter_mut().zip(p.iter()) {
                        *a += *c as f64;
                    }
                }
            }
            let n = ((y1 - y0) as f64) * ((x1 - x0) as f64);
            out.push([
                (acc[0] / n) as f32,
                (acc[1] / n) as f32,
                (acc[2] / n) as f32,
                (acc[3] / n) as f32,
            ]);
        }
    }
    out
}
