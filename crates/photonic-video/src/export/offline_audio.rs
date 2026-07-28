//! Offline sequence-audio mix for export (K-0.7 / 09 §7 / 31 §6).
//!
//! Renders the same wall-clock-free [`Mixer::render_block`] path used by
//! interactive playback (PA-10), then optionally applies a constant
//! [`LoudnessTarget`] gain (two-pass: measure integrated LUFS + true peak,
//! then scale — never a time-varying compressor).

use std::collections::{HashMap, HashSet};

use photonic_core::timeline::{
    AssetSource, Clip, ClipAudio, ClipId, ClipSource, SequenceId, Tick, TimelineProject,
    TICKS_PER_SECOND,
};

use super::presets::LoudnessTarget;
use super::render_loop::ExportError;
use crate::audio::dsp::loudness::{integrated_lufs, true_peak_dbtp};
use crate::audio::mixer::{ClipVoice, Mixer, PcmSource, TrackVoice};
use crate::audio::{BLOCK_FRAMES, CHANNELS};
use crate::media::ffmpeg_locate::FfmpegTools;
use crate::playback::FfmpegPcmSource;

/// Default export mix rate when a sequence does not declare one (48 kHz).
pub const DEFAULT_EXPORT_SAMPLE_RATE: u32 = 48_000;

/// Render interleaved stereo f32le PCM for `[start, end)` on `sequence`, at
/// the sequence's `audio_sample_rate` (falling back to
/// [`DEFAULT_EXPORT_SAMPLE_RATE`]). When `tools` is `None`, every block is
/// silence (no decode sidecars) — still a valid length-correct buffer so a
/// video-only machine can mux silent audio if the preset asks for it.
///
/// When `loudness` is `Some`, a constant gain is computed and applied so the
/// integrated LUFS hits the target without breaching `true_peak_dbtp` (31 §6.2).
pub fn render_export_audio(
    project: &TimelineProject,
    sequence: SequenceId,
    start: Tick,
    end: Tick,
    tools: Option<&FfmpegTools>,
    loudness: Option<&LoudnessTarget>,
) -> Result<Vec<f32>, ExportError> {
    if end <= start {
        return Ok(Vec::new());
    }
    let seq = project.sequences.get(&sequence).ok_or_else(|| {
        ExportError::Resolve(format!("sequence {sequence} not found for audio export"))
    })?;
    let sample_rate = if project.settings.audio_sample_rate > 0 {
        project.settings.audio_sample_rate
    } else {
        DEFAULT_EXPORT_SAMPLE_RATE
    };

    let total_frames = ticks_to_frames(end - start, sample_rate);
    if total_frames == 0 {
        return Ok(Vec::new());
    }

    let block_ticks =
        Tick(((BLOCK_FRAMES as i128 * TICKS_PER_SECOND as i128) / sample_rate as i128) as i64)
            .max(Tick(1));

    let mut out_pcm = Vec::with_capacity(total_frames * CHANNELS);
    let mut block = vec![0f32; BLOCK_FRAMES * CHANNELS];
    let mut mixer = Mixer::new(sample_rate);
    let default_clip_audio = ClipAudio::new();
    let mut pcm: HashMap<ClipId, FfmpegPcmSource> = HashMap::new();
    let mut t = start;
    let mut frames_left = total_frames;

    while frames_left > 0 {
        let frames_this = frames_left.min(BLOCK_FRAMES);
        // Which clips sound at t? Audio tracks only (matches interactive feeder).
        let active: Vec<(&photonic_core::timeline::Track, &Clip)> = seq
            .audio_tracks
            .iter()
            .filter(|track| track.enabled && track.audio.is_some())
            .flat_map(|track| {
                track
                    .clips
                    .iter()
                    .filter(|clip| {
                        clip.enabled
                            && clip.start <= t
                            && t < clip.end()
                            && matches!(clip.source, ClipSource::Asset { .. })
                    })
                    .map(move |clip| (track, clip))
            })
            .collect();

        if let Some(tools) = tools {
            for (_, clip) in &active {
                if pcm.contains_key(&clip.id) {
                    continue;
                }
                let ClipSource::Asset { asset } = clip.source else {
                    continue;
                };
                let Some(AssetSource::File { path, .. }) =
                    project.media.assets.get(&asset).map(|a| &a.source)
                else {
                    continue;
                };
                let src_pos = clip.source_in + (t - clip.start);
                if let Ok(source) = FfmpegPcmSource::spawn(tools, path, src_pos, sample_rate) {
                    pcm.insert(clip.id, source);
                }
            }
        }
        let active_ids: HashSet<ClipId> = active.iter().map(|(_, c)| c.id).collect();
        pcm.retain(|id, _| active_ids.contains(id));

        let mut refs: HashMap<ClipId, &mut FfmpegPcmSource> =
            pcm.iter_mut().map(|(id, src)| (*id, src)).collect();
        let mut voices: Vec<TrackVoice<'_>> = Vec::new();
        for track in seq
            .audio_tracks
            .iter()
            .filter(|track| track.enabled && track.audio.is_some())
        {
            let track_audio = track.audio.as_ref().expect("filtered to Some");
            let mut clips: Vec<ClipVoice<'_>> = Vec::new();
            for (_, clip) in active.iter().filter(|(tr, _)| tr.id == track.id) {
                if let Some(source) = refs.remove(&clip.id) {
                    clips.push(ClipVoice {
                        audio: clip.audio.as_ref().unwrap_or(&default_clip_audio),
                        elapsed: t - clip.start,
                        remaining: clip.end() - t,
                        source: source as &mut dyn PcmSource,
                    });
                }
            }
            voices.push(TrackVoice {
                id: track.id,
                audio: track_audio,
                clips,
            });
        }

        block.fill(0.0);
        mixer.render_block(t, &mut voices, &seq.audio_master, &mut block);
        out_pcm.extend_from_slice(&block[..frames_this * CHANNELS]);
        frames_left -= frames_this;
        t = t + block_ticks;
    }

    if let Some(target) = loudness {
        apply_loudness_gain(&mut out_pcm, sample_rate, target);
    }
    Ok(out_pcm)
}

/// Convert a tick duration to a sample-frame count at `sample_rate` (exact
/// integer arithmetic for rates that divide `TICKS_PER_SECOND` cleanly).
fn ticks_to_frames(duration: Tick, sample_rate: u32) -> usize {
    let d = duration.0.max(0) as i128;
    let n = (d * sample_rate as i128) / TICKS_PER_SECOND as i128;
    n.max(0) as usize
}

/// Two-pass offline loudness (31 §6.2): measure integrated LUFS, compute a
/// constant gain to hit `target.integrated_lufs`, then reduce the gain if it
/// would breach `true_peak_dbtp`. Silence (non-finite LUFS) is left untouched.
fn apply_loudness_gain(pcm: &mut [f32], sample_rate: u32, target: &LoudnessTarget) {
    if pcm.is_empty() {
        return;
    }
    let measured = integrated_lufs(pcm, sample_rate);
    if !measured.is_finite() {
        return;
    }
    let mut gain_db = f64::from(target.integrated_lufs) - measured;
    let peak = true_peak_dbtp(pcm, sample_rate);
    let peak_after = peak as f64 + gain_db;
    if peak_after > f64::from(target.true_peak_dbtp) {
        // Pull the gain down so true peak lands on the ceiling; report via
        // tracing (callers can surface EngineStatus diagnostics later).
        let reduced = f64::from(target.true_peak_dbtp) - peak as f64;
        tracing::info!(
            target: "photonic_video::export",
            measured_lufs = measured,
            wanted_lufs = target.integrated_lufs,
            true_peak_dbtp = peak,
            ceiling_dbtp = target.true_peak_dbtp,
            gain_db_before = gain_db,
            gain_db_after = reduced,
            "loudness gain reduced to honour true-peak ceiling"
        );
        gain_db = reduced;
    }
    let g = 10f64.powf(gain_db / 20.0) as f32;
    if (g - 1.0).abs() < 1e-6 {
        return;
    }
    for s in pcm.iter_mut() {
        *s *= g;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photonic_core::timeline::{FrameRate, MasterBus, Sequence, Track, TrackAudio, TrackKind};

    fn empty_audio_seq() -> (TimelineProject, SequenceId) {
        let mut project = TimelineProject::new();
        project.settings.audio_sample_rate = 48_000;
        let mut seq = Sequence::new("seq", FrameRate::FPS_30, 640, 360);
        let mut t = Track::new(TrackKind::Audio, "A1");
        t.audio = Some(TrackAudio::new());
        seq.audio_tracks.push(t);
        seq.audio_master = MasterBus::default();
        let id = seq.id;
        project.insert_sequence(seq);
        (project, id)
    }

    #[test]
    fn render_export_audio_silence_has_correct_length() {
        let (project, sid) = empty_audio_seq();
        // 1 second at 48 kHz → 48000 frames × 2 channels.
        let start = Tick(0);
        let end = Tick(TICKS_PER_SECOND as i64);
        let pcm = render_export_audio(&project, sid, start, end, None, None).unwrap();
        assert_eq!(pcm.len(), 48_000 * CHANNELS);
        assert!(pcm.iter().all(|&s| s == 0.0));
    }

    #[test]
    fn apply_loudness_on_silence_is_noop() {
        let mut pcm = vec![0.0f32; 48_000 * 2];
        let target = LoudnessTarget {
            integrated_lufs: -14.0,
            true_peak_dbtp: -1.0,
        };
        apply_loudness_gain(&mut pcm, 48_000, &target);
        assert!(pcm.iter().all(|&s| s == 0.0));
    }

    #[test]
    fn apply_loudness_raises_a_quiet_tone_toward_target() {
        // 1 kHz stereo sine at low amplitude for 1 s @ 48 kHz.
        let fs = 48_000u32;
        let frames = fs as usize;
        let mut pcm = vec![0.0f32; frames * 2];
        let amp = 0.05f32;
        for f in 0..frames {
            let s = (2.0 * std::f64::consts::PI * 997.0 * f as f64 / fs as f64).sin() as f32 * amp;
            pcm[f * 2] = s;
            pcm[f * 2 + 1] = s;
        }
        let before = integrated_lufs(&pcm, fs);
        assert!(before.is_finite() && before < -20.0, "before={before}");
        let target = LoudnessTarget {
            integrated_lufs: -23.0,
            true_peak_dbtp: -1.0,
        };
        apply_loudness_gain(&mut pcm, fs, &target);
        let after = integrated_lufs(&pcm, fs);
        // Should land near −23 LUFS (within ~1.5 LU of measurement noise).
        assert!(
            (after - (-23.0)).abs() < 1.5,
            "after={after}, before={before}"
        );
    }

    #[test]
    fn ticks_to_frames_is_exact_for_one_second_at_48k() {
        assert_eq!(
            ticks_to_frames(Tick(TICKS_PER_SECOND as i64), 48_000),
            48_000
        );
    }
}
