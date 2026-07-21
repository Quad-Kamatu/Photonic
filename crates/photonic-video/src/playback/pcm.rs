//! PCM audio source over an ffmpeg `-f f32le` sidecar pipe.
//!
//! **Seam note (02 §3):** the audio twin of `decode::sidecar`'s rawvideo pipe.
//! `decode/` ships only the video pipe in P3, so the playback layer owns this
//! minimal PCM sidecar for the mixer's [`PcmSource`] seam (09 §4). When
//! `decode/` grows a parallel PCM pipe per 02 §3 ("+ a parallel PCM pipe for
//! audio assets (`-f f32le`)"), this type moves there unchanged — the mixer
//! only ever sees the trait.
//!
//! Policy conformance (09 §4, `PcmSource` trait doc): output is always stereo
//! (`-ac 2` downmixes/duplicates) at the mixer's configured sample rate
//! (`-ar`), so the mixer never resamples or remaps beyond `ChannelMap`.

use std::io::Read;
use std::path::Path;
use std::process::{Child, ChildStdout, Command, Stdio};

use photonic_core::timeline::Tick;

use crate::audio::mixer::PcmSource;
use crate::media::ffmpeg_locate::FfmpegTools;

/// Interleaved-stereo f32 PCM decoded by a persistent ffmpeg process.
pub struct FfmpegPcmSource {
    child: Child,
    stdout: ChildStdout,
    sample_rate: u32,
    /// Byte staging buffer reused across reads (no per-read allocation).
    buf: Vec<u8>,
    /// End-of-stream reached; subsequent reads return 0 frames.
    finished: bool,
}

impl FfmpegPcmSource {
    /// Spawn a decoder for `input`'s first audio stream, seeked to `start`,
    /// emitting interleaved stereo f32 at `sample_rate`.
    pub fn spawn(
        tools: &FfmpegTools,
        input: &Path,
        start: Tick,
        sample_rate: u32,
    ) -> std::io::Result<FfmpegPcmSource> {
        let mut command = Command::new(&tools.ffmpeg);
        command
            .arg("-v")
            .arg("error")
            .arg("-accurate_seek")
            .arg("-ss")
            .arg(format!("{:.6}", start.as_seconds_f64().max(0.0)))
            .arg("-i")
            .arg(input)
            // Audio only.
            .arg("-vn")
            .arg("-sn")
            .arg("-dn")
            .arg("-ac")
            .arg("2")
            .arg("-ar")
            .arg(sample_rate.to_string())
            .arg("-f")
            .arg("f32le")
            .arg("pipe:1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        // 37 §2.2: SIGKILL this PCM reader if the editor process dies (Linux).
        crate::media::child_registry::arm_parent_death_signal(&mut command);
        let mut child = command.spawn()?;
        let stdout = child.stdout.take().expect("piped stdout");
        Ok(FfmpegPcmSource {
            child,
            stdout,
            sample_rate,
            buf: Vec::new(),
            finished: false,
        })
    }
}

impl PcmSource for FfmpegPcmSource {
    fn channels(&self) -> u16 {
        2 // -ac 2 (see module doc)
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn read(&mut self, out: &mut [f32], frames: usize) -> usize {
        if self.finished || frames == 0 {
            return 0;
        }
        let want_bytes = frames * 2 * 4; // stereo f32
        if self.buf.len() < want_bytes {
            self.buf.resize(want_bytes, 0);
        }
        let mut got = 0usize;
        while got < want_bytes {
            match self.stdout.read(&mut self.buf[got..want_bytes]) {
                Ok(0) => {
                    self.finished = true;
                    break; // EOF: emit the whole frames we have
                }
                Ok(n) => got += n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => {
                    self.finished = true;
                    break;
                }
            }
        }
        let whole_frames = got / 8;
        for (sample, bytes) in out[..whole_frames * 2]
            .iter_mut()
            .zip(self.buf[..whole_frames * 8].chunks_exact(4))
        {
            *sample = f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        }
        whole_frames
    }
}

impl Drop for FfmpegPcmSource {
    fn drop(&mut self) {
        // Kill-on-drop, mirroring decode::sidecar: never leave an ffmpeg
        // writing into a dead pipe.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::ffmpeg_locate::locate_for_test;
    use std::path::PathBuf;

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name)
    }

    #[test]
    fn decodes_beep_wav_as_stereo_f32() {
        let Some(tools) = locate_for_test() else {
            eprintln!("ffmpeg not found — skipping FfmpegPcmSource test");
            return;
        };
        let mut src =
            FfmpegPcmSource::spawn(&tools, &fixture("beep_flash.wav"), Tick::ZERO, 48_000)
                .expect("spawn pcm sidecar");
        assert_eq!(src.channels(), 2);
        assert_eq!(src.sample_rate(), 48_000);

        // The fixture has a 5ms 1kHz beep at t=1.0s and silence off-beep
        // (fixtures/README.md). Read up to just past 1.0s and check energy.
        let frames = 48_128; // slightly past 1.0s
        let mut pcm = vec![0f32; frames * 2];
        let mut done = 0usize;
        while done < frames {
            let n = src.read(&mut pcm[done * 2..], (frames - done).min(4096));
            if n == 0 {
                break;
            }
            done += n;
        }
        assert!(done >= 48_048, "read through the beep instant (got {done})");
        let rms = |range: std::ops::Range<usize>| {
            let s: f64 = pcm[range.start * 2..range.end * 2]
                .iter()
                .map(|v| (*v as f64) * (*v as f64))
                .sum();
            (s / ((range.len() * 2) as f64)).sqrt()
        };
        let off = rms(24_000..24_480); // t=0.5s: silence
        let on = rms(48_000..48_048); // t=1.0s..+1ms: beep
        assert!(off < 1e-3, "off-beep window ~silent (rms {off})");
        assert!(on > 0.05, "beep window has energy (rms {on})");
    }
}
