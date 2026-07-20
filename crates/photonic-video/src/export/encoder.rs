//! FFmpeg **encode** sidecar (02-engine.md §7, 05-import-export.md §3.5/§3.7).
//!
//! ## Two piped inputs: video (stdin) + audio (second input)
//!
//! 02 §7 asks for rawvideo on stdin plus "audio mixed offline (09), piped as
//! f32le on a second input." A single process has exactly one stdin, so a
//! *second* live/pipe input cannot also be `pipe:0` — ffmpeg has no
//! multi-stream stdin demux.
//!
//! Platform strategy (24-preview-media-load / Windows export path):
//! - **Unix:** audio via a **named FIFO** (`mkfifo` / `libc::mkfifo`). A
//!   background thread opens the FIFO for writing (blocks until ffmpeg opens
//!   its reader) and writes the whole pre-mixed PCM buffer, then the FIFO is
//!   closed and unlinked.
//! - **Windows (and any non-unix):** audio is written to a **temp f32le file**
//!   *before* ffmpeg is spawned, then passed as a second `-i` path. The temp
//!   file is deleted on `finish`/`cancel`/drop. Same ffmpeg arg shape; no
//!   concurrent open race.
//!
//! Audio is mixed offline in full before export starts (09's mixer), so there
//! is no streaming/chunked write API — one buffer, one write, EOF.
//!
//! ## Encoder selection (D-03, §3.4, §3.7)
//!
//! [`EncoderCapabilities::probe`] runs `ffmpeg -encoders` once and records
//! which named encoders exist, so codec choice adapts to whatever ffmpeg the
//! caller pointed at (§3.7's bring-your-own-ffmpeg escape hatch) rather than
//! assuming a fixed build:
//! - **H.264**: `libopenh264` when present (the LGPL build Photonic ships,
//!   D-03), else `libx264` (GPL) — the fallback is exercised **only** in
//!   local/CI test runs against the operator's system ffmpeg, never in the
//!   shipped binary, which always carries `libopenh264`. This workstation's
//!   ffmpeg has no `libopenh264` build, so the fallback path is exactly what
//!   every test here exercises; the `libopenh264` branch's CRF→bitrate
//!   translation ([`crf_to_kbps_heuristic`]) is a best-effort/unverified
//!   implementation flagged for a real check once a libopenh264 build is
//!   available (dev finding, reported alongside this module).
//! - **AV1**: `libsvtav1` when present (§3.5's pick — best unencumbered
//!   speed/quality), else `librav1e`. This workstation has both; the SVT-AV1
//!   path is exercised by tests, the rav1e fallback only by a unit test on
//!   the selection logic (not a real encode).
//! - **VP9 alpha**: `libvpx-vp9` rejects `yuva444p` ("not widely supported")
//!   at this build's default strictness — confirmed empirically — so alpha
//!   VP9 uses `yuva420p` (full-res alpha plane, 4:2:0 chroma) plus
//!   `-auto-alt-ref 0` (the standard recipe for VP9-alpha-in-WebM).
//! - **ProRes 4444**: `prores_ks -profile:v 4`. `prores_ks` only accepts
//!   10/12-bit pixel formats (`yuv444p10le`/`yuva444p10le` etc, no 8-bit);
//!   feeding it our 8-bit `yuva444p` rawvideo and *not* forcing an output
//!   `-pix_fmt` lets ffmpeg's implicit format-negotiation upconvert
//!   automatically (confirmed empirically — no explicit `-pix_fmt`/`-vf
//!   format=` needed on the output side).
//! - **Color tagging**: `-color_primaries/-color_trc/-colorspace bt709
//!   -color_range tv` alone only reliably sets *container*-level tags for
//!   some encoders (confirmed: `libx264` alone left `color_transfer`/
//!   `color_primaries` "unknown" via `ffprobe`); pairing it with
//!   `-vf setparams=colorspace=bt709:color_primaries=bt709:color_trc=bt709:range=tv`
//!   makes the encoder itself write the tags into its bitstream (confirmed:
//!   all four fields then read back correctly). Applied to the YUV-family
//!   containers only (Mp4/Mov/WebM/Mkv) — GIF has no such metadata, and
//!   PNG/APNG's sRGB convention doesn't use it (§6.1's carve-out).

use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;

use photonic_core::timeline::FrameRate;

use super::convert::EncodePlanes;
use super::presets::{
    AudioCodec, AudioEncodeSpec, Container, ExportPreset, QualityMode, VideoCodec,
};
use crate::media::ffmpeg_locate::FfmpegTools;

#[derive(Debug, thiserror::Error)]
pub enum EncodeError {
    #[error("failed to spawn ffmpeg encoder: {0}")]
    Spawn(#[source] std::io::Error),
    #[error("failed to probe ffmpeg encoder capabilities: {0}")]
    Probe(#[source] std::io::Error),
    #[error("io error: {0}")]
    Io(#[source] std::io::Error),
    #[error("invalid audio sidecar path (non-UTF8 / contains NUL)")]
    InvalidFifoPath,
    #[error("audio writer thread panicked")]
    AudioWriterPanicked,
    #[error("encoder exited with status {status:?}; stderr tail:\n{stderr}")]
    EncoderExited { status: Option<i32>, stderr: String },
}

// ── Encoder capability probing (§3.4/§3.7) ───────────────────────────────────

/// Which named encoders/muxers this ffmpeg build has, from one
/// `ffmpeg -encoders` invocation.
#[derive(Clone, Debug)]
pub struct EncoderCapabilities {
    names: HashSet<String>,
}

impl EncoderCapabilities {
    pub fn probe(tools: &FfmpegTools) -> Result<Self, EncodeError> {
        let output = Command::new(&tools.ffmpeg)
            .args(["-hide_banner", "-encoders"])
            .output()
            .map_err(EncodeError::Probe)?;
        let text = String::from_utf8_lossy(&output.stdout);
        Ok(Self::parse(&text))
    }

    /// Parse `ffmpeg -encoders` output. Each encoder line looks like
    /// `<flags> <name>  <long name>` (flags is a fixed-width 6-char field,
    /// `ffmpeg -encoders`'s documented format) — the name is the first
    /// whitespace-delimited token after the flags column.
    fn parse(text: &str) -> Self {
        let mut names = HashSet::new();
        for line in text.lines() {
            let trimmed = line.trim_start();
            // Encoder lines start with a flags column like "V....D" / "A....D";
            // header/blank lines don't. Skip anything that doesn't match.
            let mut parts = trimmed.splitn(3, char::is_whitespace);
            let flags = match parts.next() {
                Some(f) if f.len() == 6 && f.starts_with(['V', 'A', 'S']) => f,
                _ => continue,
            };
            let _ = flags;
            if let Some(name) = trimmed.split_whitespace().nth(1) {
                names.insert(name.to_string());
            }
        }
        EncoderCapabilities { names }
    }

    pub fn has(&self, encoder_name: &str) -> bool {
        self.names.contains(encoder_name)
    }

    /// H.264: `libopenh264` (shipped LGPL build, D-03) else `libx264` (dev/CI
    /// fallback only — see module docs).
    pub fn h264_encoder(&self) -> &'static str {
        if self.has("libopenh264") {
            "libopenh264"
        } else {
            "libx264"
        }
    }

    /// AV1: `libsvtav1` (§3.5's pick) else `librav1e`.
    pub fn av1_encoder(&self) -> &'static str {
        if self.has("libsvtav1") {
            "libsvtav1"
        } else {
            "librav1e"
        }
    }
}

// ── Plane-shape selection (which convert.rs function to feed) ───────────────

/// Which raw pixel layout a codec expects, driving both the ffmpeg input
/// `-pix_fmt` declaration and which `convert::working_frame_to_*` function
/// `render_loop` must call for a given frame.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PlaneKind {
    Yuv420,
    Yuva420,
    Yuva444,
    Rgba8,
}

impl PlaneKind {
    pub fn ffmpeg_pix_fmt(self) -> &'static str {
        match self {
            PlaneKind::Yuv420 => "yuv420p",
            PlaneKind::Yuva420 => "yuva420p",
            PlaneKind::Yuva444 => "yuva444p",
            PlaneKind::Rgba8 => "rgba",
        }
    }
}

/// §3.4's allow-list, restated as a plane-shape choice: PNG/APNG are RGB, VP9
/// alpha is 4:2:0 (broad real-world decoder compatibility — `yuva444p` is
/// rejected by `libvpx-vp9` at default strictness), ProRes 4444 is 4:4:4.
pub fn plane_kind_for(codec: Option<VideoCodec>, alpha: bool) -> PlaneKind {
    match codec {
        Some(VideoCodec::Png) | Some(VideoCodec::Apng) => PlaneKind::Rgba8,
        Some(VideoCodec::ProResLikeMezzanine) if alpha => PlaneKind::Yuva444,
        Some(VideoCodec::Vp9) if alpha => PlaneKind::Yuva420,
        _ if alpha => PlaneKind::Yuva444, // not reachable via `validate`'s allow-list; safe default
        _ => PlaneKind::Yuv420,
    }
}

// ── ffmpeg arg building (pure — testable without spawning ffmpeg) ───────────

/// Resolved (post-`ResolutionSpec`/`FrameRatePolicy`) target audio format.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct AudioStreamSpec {
    pub sample_rate: u32,
    pub channels: u16,
}

/// Everything [`build_ffmpeg_args`]/[`EncoderProcess::spawn`] need, already
/// resolved from an [`ExportPreset`]'s abstract `ResolutionSpec`/
/// `FrameRatePolicy` down to concrete numbers — that resolution is the
/// caller's (render_loop/the evaluator's) job, not this module's.
pub struct EncodeSpec<'a> {
    pub preset: &'a ExportPreset,
    pub width: u32,
    pub height: u32,
    pub frame_rate: FrameRate,
    pub audio: Option<AudioStreamSpec>,
    pub out_path: PathBuf,
}

/// Best-effort CRF→bitrate translation for encoders without true CRF-mode
/// rate control (currently only the `libopenh264` branch — **unverified**,
/// see module docs: no `libopenh264` build is available to test against
/// here). Follows the common x264 rule of thumb that perceptual bitrate
/// roughly halves every +6 CRF, anchored at 1080p/CRF23≈4000kbps and scaled
/// by pixel count.
fn crf_to_kbps_heuristic(crf: f32, width: u32, height: u32) -> u32 {
    const BASE_KBPS_1080P_CRF23: f32 = 4000.0;
    let scale = (width as f32 * height as f32) / (1920.0 * 1080.0);
    let factor = 2f32.powf((23.0 - crf) / 6.0);
    (BASE_KBPS_1080P_CRF23 * scale * factor).clamp(200.0, 50_000.0) as u32
}

fn push_video_codec_args(
    args: &mut Vec<String>,
    caps: &EncoderCapabilities,
    codec: VideoCodec,
    quality: QualityMode,
    alpha: bool,
    width: u32,
    height: u32,
) {
    match codec {
        VideoCodec::H264 => {
            let enc = caps.h264_encoder();
            args.extend(["-c:v".into(), enc.into()]);
            let is_libx264 = enc == "libx264";
            match quality {
                QualityMode::Crf(crf) if is_libx264 => {
                    args.extend(["-crf".into(), crf.to_string()]);
                }
                QualityMode::Crf(crf) => {
                    let kbps = crf_to_kbps_heuristic(crf, width, height);
                    args.extend(["-b:v".into(), format!("{kbps}k")]);
                }
                QualityMode::Bitrate {
                    target_kbps,
                    max_kbps,
                } => {
                    args.extend([
                        "-b:v".into(),
                        format!("{target_kbps}k"),
                        "-maxrate".into(),
                        format!("{max_kbps}k"),
                        "-bufsize".into(),
                        format!("{}k", max_kbps.saturating_mul(2)),
                    ]);
                }
                QualityMode::Lossless if is_libx264 => {
                    args.extend(["-crf".into(), "0".into()]);
                }
                QualityMode::Lossless => {
                    args.extend(["-b:v".into(), "0".into()]);
                }
            }
        }
        VideoCodec::Av1 => {
            let enc = caps.av1_encoder();
            args.extend(["-c:v".into(), enc.into()]);
            if enc == "libsvtav1" {
                args.extend(["-preset".into(), "4".into()]); // §3.5: "preset speed 4"
                let crf = match quality {
                    QualityMode::Crf(v) => v,
                    QualityMode::Bitrate { .. } => {
                        // libsvtav1 also accepts -b:v directly; CRF path is what
                        // the catalog uses, Bitrate mode just passes through.
                        if let QualityMode::Bitrate {
                            target_kbps,
                            max_kbps,
                        } = quality
                        {
                            args.extend([
                                "-b:v".into(),
                                format!("{target_kbps}k"),
                                "-maxrate".into(),
                                format!("{max_kbps}k"),
                            ]);
                        }
                        return;
                    }
                    QualityMode::Lossless => 0.0,
                };
                args.extend(["-crf".into(), (crf.round() as i32).clamp(0, 63).to_string()]);
            } else {
                // librav1e fallback: `-qp` (0..255, lower = better) is not the
                // same scale as SVT-AV1's CRF (0..63) — documented approximate
                // mapping (x4), not a claimed perceptual equivalence.
                args.extend(["-speed".into(), "6".into()]);
                let qp = match quality {
                    QualityMode::Crf(v) => (v * 4.0).round() as i32,
                    QualityMode::Bitrate { target_kbps, .. } => {
                        args.extend(["-b:v".into(), format!("{target_kbps}k")]);
                        return;
                    }
                    QualityMode::Lossless => 0,
                };
                args.extend(["-qp".into(), qp.clamp(0, 255).to_string()]);
            }
        }
        VideoCodec::Vp9 => {
            args.extend(["-c:v".into(), "libvpx-vp9".into()]);
            match quality {
                QualityMode::Crf(v) => {
                    args.extend([
                        "-crf".into(),
                        (v.round() as i32).clamp(0, 63).to_string(),
                        // libvpx-vp9 CRF mode requires -b:v 0, else it runs
                        // constrained-quality instead of true constant-quality.
                        "-b:v".into(),
                        "0".into(),
                    ]);
                }
                QualityMode::Bitrate {
                    target_kbps,
                    max_kbps,
                } => {
                    args.extend([
                        "-b:v".into(),
                        format!("{target_kbps}k"),
                        "-maxrate".into(),
                        format!("{max_kbps}k"),
                    ]);
                }
                QualityMode::Lossless => {
                    args.extend(["-lossless".into(), "1".into()]);
                }
            }
            if alpha {
                // Standard VP9-alpha-in-WebM recipe: alt-ref frames don't
                // interact well with the alpha side-channel.
                args.extend(["-auto-alt-ref".into(), "0".into()]);
            }
        }
        VideoCodec::ProResLikeMezzanine => {
            // profile 4 == "4444" (ffmpeg's -profile enum for prores_ks).
            args.extend([
                "-c:v".into(),
                "prores_ks".into(),
                "-profile:v".into(),
                "4".into(),
            ]);
        }
        VideoCodec::Gif => {
            // High-quality paletted GIF: generate a palette from the stream,
            // then dither against it (standard ffmpeg recipe). This replaces
            // the color-tagging `-vf` entirely for GIF outputs (mutually
            // exclusive with `setparams` on the same output stream) — the
            // caller skips the color-tag block for `VideoCodec::Gif`.
            //
            // The graph's first pad is labeled `[0:v]` (not left unlabeled): an
            // unlabeled `filter_complex` input pad is auto-fed from an *unused*
            // input stream, which the caller's explicit `-map 0:v` would have
            // already consumed ("Cannot find an unused video input stream to
            // feed the unlabeled input pad split"). The caller therefore also
            // omits `-map 0:v` for GIF and lets `paletteuse`'s unlabeled output
            // auto-map to the output file.
            args.extend([
                "-filter_complex".into(),
                "[0:v]split[s0][s1];[s0]palettegen[p];[s1][p]paletteuse=dither=bayer".into(),
            ]);
        }
        VideoCodec::Png => {
            args.extend(["-c:v".into(), "png".into()]);
        }
        VideoCodec::Apng => {
            // `-plays 0` = loop forever (APNG's standard "loop" convention).
            args.extend(["-c:v".into(), "apng".into(), "-plays".into(), "0".into()]);
        }
    }
}

fn push_audio_codec_args(args: &mut Vec<String>, audio: &AudioEncodeSpec) {
    match audio.codec {
        AudioCodec::Aac => {
            args.extend(["-c:a".into(), "aac".into()]);
            if let Some(kbps) = audio.bitrate_kbps {
                args.extend(["-b:a".into(), format!("{kbps}k")]);
            }
        }
        AudioCodec::Opus => {
            args.extend(["-c:a".into(), "libopus".into()]);
            if let Some(kbps) = audio.bitrate_kbps {
                args.extend(["-b:a".into(), format!("{kbps}k")]);
            }
        }
        AudioCodec::Pcm => {
            args.extend(["-c:a".into(), "pcm_s16le".into()]);
        }
    }
}

/// Whether `container` carries the kind of stream-level color metadata the
/// `setparams`/`-color_*` tagging block applies to (§6.1).
fn container_supports_color_tags(container: Container) -> bool {
    matches!(
        container,
        Container::Mp4 | Container::Mov | Container::WebM | Container::Mkv
    )
}

/// Build the full ffmpeg argument list for one export encode. Pure/testable
/// without spawning a process — [`EncoderProcess::spawn`] is the only caller
/// that actually runs it.
pub fn build_ffmpeg_args(
    caps: &EncoderCapabilities,
    spec: &EncodeSpec,
    video_pix_fmt: &str,
    audio_fifo: Option<&Path>,
) -> Vec<String> {
    let mut args = vec![
        "-hide_banner".to_string(),
        "-nostdin".to_string(),
        "-y".to_string(),
        "-loglevel".to_string(),
        "error".to_string(),
    ];

    let fr = format!("{}/{}", spec.frame_rate.num, spec.frame_rate.den);
    args.extend([
        "-f".into(),
        "rawvideo".into(),
        "-pix_fmt".into(),
        video_pix_fmt.into(),
        "-s".into(),
        format!("{}x{}", spec.width, spec.height),
        "-r".into(),
        fr,
        "-i".into(),
        "pipe:0".into(),
    ]);

    let has_audio = audio_fifo.is_some() && spec.audio.is_some() && spec.preset.audio.is_some();
    if let (Some(fifo), Some(a)) = (audio_fifo, spec.audio.as_ref()) {
        if spec.preset.audio.is_some() {
            args.extend([
                "-f".into(),
                "f32le".into(),
                "-ar".into(),
                a.sample_rate.to_string(),
                "-ac".into(),
                a.channels.to_string(),
                "-i".into(),
                fifo.to_string_lossy().into_owned(),
            ]);
        }
    }

    // GIF drives its video through a `filter_complex` whose `paletteuse`
    // output auto-maps; an explicit `-map 0:v` there would both double-map and
    // starve the filtergraph's `[0:v]` input pad (see the `VideoCodec::Gif`
    // arm). Every other codec maps the raw video stream directly.
    let is_gif = matches!(
        spec.preset.video.as_ref().map(|v| v.codec),
        Some(VideoCodec::Gif)
    );
    if !is_gif {
        args.extend(["-map".into(), "0:v".into()]);
    }
    if has_audio {
        args.extend(["-map".into(), "1:a".into()]);
    }

    match &spec.preset.video {
        Some(v) => push_video_codec_args(
            &mut args,
            caps,
            v.codec,
            v.quality,
            spec.preset.alpha,
            spec.width,
            spec.height,
        ),
        None => args.push("-vn".into()),
    }

    if has_audio {
        if let Some(a) = &spec.preset.audio {
            push_audio_codec_args(&mut args, a);
        }
    } else {
        args.push("-an".into());
    }

    if container_supports_color_tags(spec.preset.container) && !is_gif {
        args.extend([
            "-vf".into(),
            "setparams=colorspace=bt709:color_primaries=bt709:color_trc=bt709:range=tv".into(),
            "-color_primaries".into(),
            "bt709".into(),
            "-color_trc".into(),
            "bt709".into(),
            "-colorspace".into(),
            "bt709".into(),
            "-color_range".into(),
            "tv".into(),
        ]);
    }

    if spec.preset.faststart {
        args.extend(["-movflags".into(), "+faststart".into()]);
    }

    args.push(spec.out_path.to_string_lossy().into_owned());
    args
}

// ── Process management ───────────────────────────────────────────────────────

const STDERR_TAIL: usize = 32;

fn unique_audio_sidecar_path(ext: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!(
        "photonic-export-audio-{}-{n}-{nanos}.{ext}",
        std::process::id()
    ))
}

#[cfg(unix)]
fn create_fifo(path: &Path) -> Result<(), EncodeError> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let c_path =
        CString::new(path.as_os_str().as_bytes()).map_err(|_| EncodeError::InvalidFifoPath)?;
    // 0o600: owner read/write only — the FIFO carries transient local PCM
    // data for the lifetime of one export job.
    let ret = unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) };
    if ret != 0 {
        return Err(EncodeError::Io(std::io::Error::last_os_error()));
    }
    Ok(())
}

/// Write interleaved f32le PCM to `path` (Windows / non-unix second input).
#[cfg_attr(unix, allow(dead_code))]
fn write_pcm_file(path: &Path, samples: &[f32]) -> Result<(), EncodeError> {
    let mut bytes = Vec::with_capacity(samples.len() * 4);
    for s in samples {
        bytes.extend_from_slice(&s.to_le_bytes());
    }
    std::fs::write(path, bytes).map_err(EncodeError::Io)
}

/// Stage PCM for a second ffmpeg `-i` without requiring a platform FIFO.
/// Public for headless tests of the Windows/non-unix path on any OS.
pub fn stage_audio_tempfile(samples: &[f32]) -> Result<PathBuf, EncodeError> {
    let p = unique_audio_sidecar_path("f32le");
    write_pcm_file(&p, samples)?;
    Ok(p)
}

/// Second (audio) input path staged for this encode job (FIFO or temp file).
#[derive(Debug)]
struct AudioSidecar {
    path: PathBuf,
}

impl AudioSidecar {
    fn path(&self) -> &Path {
        &self.path
    }

    fn cleanup(self) {
        let _ = std::fs::remove_file(self.path);
    }
}

/// A running ffmpeg encode process: video frames go to stdin; if the preset
/// has an audio track, a second input carries pre-mixed f32le PCM (FIFO on
/// unix, temp file elsewhere — see module docs).
pub struct EncoderProcess {
    child: Child,
    video_stdin: Option<ChildStdin>,
    audio_writer: Option<JoinHandle<Result<(), EncodeError>>>,
    audio_sidecar: Option<AudioSidecar>,
    stderr_tail: Arc<Mutex<Vec<String>>>,
}

impl EncoderProcess {
    /// Spawn the encoder. `audio_samples`, when present, is the **whole**
    /// pre-mixed interleaved `f32` PCM track (09's offline mix) at
    /// `spec.audio`'s sample rate/channel count.
    pub fn spawn(
        tools: &FfmpegTools,
        caps: &EncoderCapabilities,
        spec: &EncodeSpec,
        audio_samples: Option<Vec<f32>>,
    ) -> Result<Self, EncodeError> {
        let plane_kind = plane_kind_for(
            spec.preset.video.as_ref().map(|v| v.codec),
            spec.preset.alpha,
        );
        let video_pix_fmt = plane_kind.ffmpeg_pix_fmt();

        let wants_audio =
            spec.preset.audio.is_some() && spec.audio.is_some() && audio_samples.is_some();

        let (audio_sidecar, audio_writer) = if wants_audio {
            let samples = audio_samples.expect("checked wants_audio");
            #[cfg(unix)]
            {
                let p = unique_audio_sidecar_path("fifo");
                create_fifo(&p)?;
                let path_for_writer = p.clone();
                let writer = std::thread::spawn(move || -> Result<(), EncodeError> {
                    let mut f = std::fs::OpenOptions::new()
                        .write(true)
                        .open(&path_for_writer)
                        .map_err(EncodeError::Io)?;
                    let mut bytes = Vec::with_capacity(samples.len() * 4);
                    for s in &samples {
                        bytes.extend_from_slice(&s.to_le_bytes());
                    }
                    f.write_all(&bytes).map_err(EncodeError::Io)?;
                    Ok(())
                });
                (Some(AudioSidecar { path: p }), Some(writer))
            }
            #[cfg(not(unix))]
            {
                // Pre-write so ffmpeg can open a regular file as second -i.
                let p = unique_audio_sidecar_path("f32le");
                write_pcm_file(&p, &samples)?;
                (Some(AudioSidecar { path: p }), None)
            }
        } else {
            (None, None)
        };

        let audio_path = audio_sidecar.as_ref().map(|s| s.path().to_path_buf());
        let args = build_ffmpeg_args(caps, spec, video_pix_fmt, audio_path.as_deref());

        if let Some(parent) = spec.out_path.parent() {
            std::fs::create_dir_all(parent).map_err(EncodeError::Io)?;
        }

        let mut child = match Command::new(&tools.ffmpeg)
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                if let Some(s) = audio_sidecar {
                    s.cleanup();
                }
                return Err(EncodeError::Spawn(e));
            }
        };

        // We requested `Stdio::piped()`, so stdin should be present — but if
        // ffmpeg was killed / exited between spawn and here, `take` yields None.
        // Return a typed error instead of panicking, and clean up.
        let video_stdin = match child.stdin.take() {
            Some(s) => s,
            None => {
                let _ = child.kill();
                let _ = child.wait();
                if let Some(s) = audio_sidecar {
                    s.cleanup();
                }
                return Err(EncodeError::Io(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "ffmpeg stdin pipe unavailable (process exited during spawn)",
                )));
            }
        };

        let stderr_tail = Arc::new(Mutex::new(Vec::<String>::new()));
        if let Some(stderr) = child.stderr.take() {
            use std::io::{BufRead, BufReader};
            let tail = Arc::clone(&stderr_tail);
            std::thread::spawn(move || {
                let reader = BufReader::new(stderr);
                for line in reader.lines().map_while(Result::ok) {
                    // Poison-tolerant: a panic elsewhere holding this lock must
                    // not wedge stderr draining (a full pipe would deadlock
                    // ffmpeg). The tail is advisory diagnostics, not invariants.
                    let mut t = tail.lock().unwrap_or_else(PoisonError::into_inner);
                    if t.len() == STDERR_TAIL {
                        t.remove(0);
                    }
                    t.push(line);
                }
            });
        }

        Ok(EncoderProcess {
            child,
            video_stdin: Some(video_stdin),
            audio_writer,
            audio_sidecar,
            stderr_tail,
        })
    }

    /// Write one frame's worth of already-converted plane bytes to the
    /// encoder's rawvideo stdin.
    pub fn write_video_frame(&mut self, planes: &EncodePlanes) -> Result<(), EncodeError> {
        let bytes = planes.to_bytes();
        // `video_stdin` is `Some` for the whole life of a live `EncoderProcess`
        // (only `finish`/`cancel`, which consume `self`, take it). Guard against
        // a None anyway so a misuse surfaces as a typed error, not a panic.
        let stdin = self.video_stdin.as_mut().ok_or_else(|| {
            EncodeError::Io(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "video stdin already closed (finish/cancel called)",
            ))
        })?;
        stdin.write_all(&bytes).map_err(EncodeError::Io)
    }

    /// Close stdin (signals video EOF), wait for the audio writer and the
    /// process, and surface a non-zero exit as an error with the stderr tail.
    pub fn finish(mut self) -> Result<(), EncodeError> {
        drop(self.video_stdin.take());
        if let Some(h) = self.audio_writer.take() {
            match h.join() {
                Ok(inner) => inner?,
                Err(_) => return Err(EncodeError::AudioWriterPanicked),
            }
        }
        let status = self.child.wait().map_err(EncodeError::Io)?;
        if let Some(sidecar) = self.audio_sidecar.take() {
            sidecar.cleanup();
        }
        if !status.success() {
            return Err(EncodeError::EncoderExited {
                status: status.code(),
                // Poison-tolerant: recover the tail even if the drain thread
                // panicked, so the exit error still carries diagnostics.
                stderr: self
                    .stderr_tail
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .join("\n"),
            });
        }
        Ok(())
    }

    /// Cancel mid-export (02 §7: "cancellable between frames"): kill the
    /// process immediately, no attempt to produce a valid output file.
    pub fn cancel(mut self) {
        drop(self.video_stdin.take());
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(sidecar) = self.audio_sidecar.take() {
            sidecar.cleanup();
        }
    }
}

impl Drop for EncoderProcess {
    fn drop(&mut self) {
        // Best-effort cleanup if neither `finish` nor `cancel` ran (e.g. a
        // panic unwind) — kill-on-drop, mirroring decode/sidecar.rs.
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(sidecar) = self.audio_sidecar.take() {
            sidecar.cleanup();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::presets::{
        AudioEncodeSpec, Container, ExportPreset, FrameRatePolicy, LoudnessTarget, QualityMode,
        ResolutionSpec, VideoEncodeSpec,
    };

    fn caps_with(names: &[&str]) -> EncoderCapabilities {
        EncoderCapabilities {
            names: names.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn base_preset() -> ExportPreset {
        ExportPreset {
            name: "test".into(),
            container: Container::Mp4,
            video: Some(VideoEncodeSpec {
                codec: VideoCodec::H264,
                quality: QualityMode::Crf(20.0),
            }),
            audio: Some(AudioEncodeSpec {
                codec: AudioCodec::Aac,
                bitrate_kbps: Some(128),
            }),
            resolution: ResolutionSpec::SourceFormat,
            frame_rate: FrameRatePolicy::MatchSequence,
            alpha: false,
            faststart: true,
            loudness_target: None::<LoudnessTarget>,
        }
    }

    fn spec(preset: &ExportPreset) -> EncodeSpec<'_> {
        EncodeSpec {
            preset,
            width: 32,
            height: 32,
            frame_rate: FrameRate::new(10, 1),
            audio: Some(AudioStreamSpec {
                sample_rate: 48_000,
                channels: 2,
            }),
            out_path: PathBuf::from("/tmp/out.mp4"),
        }
    }

    // ── capability parsing ───────────────────────────────────────────────────

    #[test]
    fn parse_encoders_output_extracts_names() {
        let sample = " V....D libx264              libx264 H.264 / AVC\n\
                        A....D aac                   AAC (Advanced Audio Coding)\n\
                        \n\
                        Encoders:\n";
        let caps = EncoderCapabilities::parse(sample);
        assert!(caps.has("libx264"));
        assert!(caps.has("aac"));
        assert!(!caps.has("libopenh264"));
    }

    #[test]
    fn h264_encoder_prefers_openh264_falls_back_to_libx264() {
        assert_eq!(
            caps_with(&["libopenh264", "libx264"]).h264_encoder(),
            "libopenh264"
        );
        assert_eq!(caps_with(&["libx264"]).h264_encoder(), "libx264");
    }

    #[test]
    fn av1_encoder_prefers_svtav1_falls_back_to_rav1e() {
        assert_eq!(
            caps_with(&["libsvtav1", "librav1e"]).av1_encoder(),
            "libsvtav1"
        );
        assert_eq!(caps_with(&["librav1e"]).av1_encoder(), "librav1e");
    }

    // ── plane-shape selection ────────────────────────────────────────────────

    #[test]
    fn plane_kind_routes_png_apng_to_rgba8() {
        assert_eq!(
            plane_kind_for(Some(VideoCodec::Png), true),
            PlaneKind::Rgba8
        );
        assert_eq!(
            plane_kind_for(Some(VideoCodec::Apng), true),
            PlaneKind::Rgba8
        );
    }

    #[test]
    fn plane_kind_routes_vp9_alpha_to_yuva420_prores_alpha_to_yuva444() {
        assert_eq!(
            plane_kind_for(Some(VideoCodec::Vp9), true),
            PlaneKind::Yuva420
        );
        assert_eq!(
            plane_kind_for(Some(VideoCodec::ProResLikeMezzanine), true),
            PlaneKind::Yuva444
        );
    }

    #[test]
    fn plane_kind_no_alpha_is_yuv420_for_any_yuv_codec() {
        assert_eq!(
            plane_kind_for(Some(VideoCodec::H264), false),
            PlaneKind::Yuv420
        );
        assert_eq!(
            plane_kind_for(Some(VideoCodec::Av1), false),
            PlaneKind::Yuv420
        );
    }

    // ── crf heuristic ────────────────────────────────────────────────────────

    #[test]
    fn crf_to_kbps_heuristic_is_monotonically_decreasing_in_crf() {
        let lo = crf_to_kbps_heuristic(15.0, 1920, 1080);
        let mid = crf_to_kbps_heuristic(23.0, 1920, 1080);
        let hi = crf_to_kbps_heuristic(35.0, 1920, 1080);
        assert!(lo > mid && mid > hi, "lo={lo} mid={mid} hi={hi}");
    }

    #[test]
    fn crf_to_kbps_heuristic_scales_with_pixel_count() {
        let hd = crf_to_kbps_heuristic(23.0, 1920, 1080);
        let sd = crf_to_kbps_heuristic(23.0, 960, 540);
        assert!(hd > sd);
    }

    // ── arg building (pure, no ffmpeg spawn) ─────────────────────────────────

    #[test]
    fn build_args_h264_uses_libx264_crf_when_no_openh264() {
        let preset = base_preset();
        let caps = caps_with(&["libx264", "aac"]);
        let s = spec(&preset);
        let args = build_ffmpeg_args(&caps, &s, "yuv420p", Some(Path::new("/tmp/a.fifo")));
        assert!(args.windows(2).any(|w| w == ["-c:v", "libx264"]));
        assert!(args.windows(2).any(|w| w == ["-crf", "20"]));
        assert!(args.iter().any(|a| a == "pipe:0"));
        assert!(args.windows(2).any(|w| w == ["-map", "0:v"]));
        assert!(args.windows(2).any(|w| w == ["-map", "1:a"]));
        assert!(args.windows(2).any(|w| w == ["-c:a", "aac"]));
        assert!(args.iter().any(|a| a.contains("setparams")));
        assert!(args.windows(2).any(|w| w == ["-movflags", "+faststart"]));
    }

    #[test]
    fn build_args_no_audio_when_preset_has_none() {
        let mut preset = base_preset();
        preset.audio = None;
        let s = spec(&preset);
        let caps = caps_with(&["libx264"]);
        let args = build_ffmpeg_args(&caps, &s, "yuv420p", None);
        assert!(args.iter().any(|a| a == "-an"));
        assert!(!args.iter().any(|a| a == "-c:a"));
        assert!(!args.windows(2).any(|w| w == ["-map", "1:a"]));
    }

    #[test]
    fn build_args_no_video_sets_vn() {
        let mut preset = base_preset();
        preset.video = None;
        let s = spec(&preset);
        let caps = caps_with(&["aac"]);
        let args = build_ffmpeg_args(&caps, &s, "yuv420p", Some(Path::new("/tmp/a.fifo")));
        assert!(args.iter().any(|a| a == "-vn"));
    }

    #[test]
    fn build_args_vp9_alpha_uses_auto_alt_ref_0_and_b_v_0() {
        let mut preset = base_preset();
        preset.container = Container::WebM;
        preset.alpha = true;
        preset.video = Some(VideoEncodeSpec {
            codec: VideoCodec::Vp9,
            quality: QualityMode::Crf(24.0),
        });
        let s = spec(&preset);
        let caps = caps_with(&["libvpx-vp9", "libopus"]);
        let args = build_ffmpeg_args(&caps, &s, "yuva420p", Some(Path::new("/tmp/a.fifo")));
        assert!(args.windows(2).any(|w| w == ["-c:v", "libvpx-vp9"]));
        assert!(args.windows(2).any(|w| w == ["-auto-alt-ref", "0"]));
        assert!(args.windows(2).any(|w| w == ["-b:v", "0"]));
    }

    #[test]
    fn build_args_gif_uses_palette_filter_not_setparams() {
        let mut preset = base_preset();
        preset.container = Container::Gif;
        preset.audio = None;
        preset.video = Some(VideoEncodeSpec {
            codec: VideoCodec::Gif,
            quality: QualityMode::Lossless,
        });
        let s = spec(&preset);
        let caps = caps_with(&[]);
        let args = build_ffmpeg_args(&caps, &s, "yuv420p", None);
        assert!(args.iter().any(|a| a.contains("palettegen")));
        assert!(
            !args.iter().any(|a| a.contains("setparams")),
            "gif skips color tagging"
        );
    }

    #[test]
    fn build_args_prores_sets_profile_4_no_forced_output_pix_fmt() {
        let mut preset = base_preset();
        preset.container = Container::Mov;
        preset.alpha = true;
        preset.video = Some(VideoEncodeSpec {
            codec: VideoCodec::ProResLikeMezzanine,
            quality: QualityMode::Lossless,
        });
        preset.audio = Some(AudioEncodeSpec {
            codec: AudioCodec::Pcm,
            bitrate_kbps: None,
        });
        let s = spec(&preset);
        let caps = caps_with(&[]);
        let args = build_ffmpeg_args(&caps, &s, "yuva444p", Some(Path::new("/tmp/a.fifo")));
        assert!(args.windows(2).any(|w| w == ["-c:v", "prores_ks"]));
        assert!(args.windows(2).any(|w| w == ["-profile:v", "4"]));
        assert!(args.windows(2).any(|w| w == ["-c:a", "pcm_s16le"]));
        // No output -pix_fmt override after the input declaration — only one
        // "-pix_fmt" occurrence total (the rawvideo input side).
        assert_eq!(args.iter().filter(|a| a.as_str() == "-pix_fmt").count(), 1);
    }

    #[test]
    fn build_args_faststart_only_added_when_preset_requests_it() {
        let mut preset = base_preset();
        preset.faststart = false;
        let s = spec(&preset);
        let caps = caps_with(&["libx264"]);
        let args = build_ffmpeg_args(&caps, &s, "yuv420p", None);
        assert!(!args.iter().any(|a| a == "+faststart"));
    }

    #[test]
    fn build_args_color_tags_skipped_for_gif_and_image_sequence() {
        let mut preset = base_preset();
        preset.container = Container::ImageSequence;
        preset.audio = None;
        preset.video = Some(VideoEncodeSpec {
            codec: VideoCodec::Png,
            quality: QualityMode::Lossless,
        });
        let s = spec(&preset);
        let caps = caps_with(&["png"]);
        let args = build_ffmpeg_args(&caps, &s, "rgba", None);
        assert!(!args.iter().any(|a| a.contains("bt709")));
    }
}
