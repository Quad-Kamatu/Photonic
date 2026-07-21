//! FFmpeg decode sidecar: one process, `rawvideo` on stdout (02 §3).
//!
//! We build the args ourselves (no `ffmpeg-sidecar` crate) so the exact seek /
//! pixel-format / verbosity flags are under our control. The process is killed
//! on drop so a dropped [`DecodeSource`](super::scheduler::DecodeSource) never
//! leaks an ffmpeg. stderr is drained on a worker thread (a full stderr pipe
//! would otherwise deadlock ffmpeg) and its tail is kept for error reporting.

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex, PoisonError};

use photonic_core::timeline::Tick;

use super::{DecodeError, PixFmt};
use crate::media::ffmpeg_locate::FfmpegTools;

/// What to decode and how.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SidecarConfig {
    pub input: PathBuf,
    /// Input-level seek origin (a keyframe tick from the index). `-ss` before
    /// `-i` = fast keyframe-accurate seek; decode-forward discards to the target.
    pub seek: Tick,
    pub pix_fmt: PixFmt,
}

/// Lines of stderr kept for diagnostics (bounded ring).
const STDERR_TAIL: usize = 16;

/// A running ffmpeg decode process. Holds the child for kill-on-drop; stdout is
/// handed to the reader separately (so the reader can own it without a
/// self-referential borrow on the `Sidecar`).
pub struct Sidecar {
    child: Child,
    stderr_tail: Arc<Mutex<Vec<String>>>,
}

impl Sidecar {
    /// Spawn ffmpeg and return the sidecar plus its stdout pipe.
    ///
    /// Args: `ffmpeg -hide_banner -nostdin -loglevel error -ss <seek> -i <file>
    /// -f rawvideo -pix_fmt <fmt> pipe:1`. Input `-ss` seeks to the keyframe at
    /// or before `seek`; the caller decode-forwards to the exact target.
    pub fn spawn(
        tools: &FfmpegTools,
        cfg: &SidecarConfig,
    ) -> Result<(Self, ChildStdout), DecodeError> {
        let seek_secs = format!("{:.6}", cfg.seek.as_seconds_f64());
        let mut command = Command::new(&tools.ffmpeg);
        command
            .args(["-hide_banner", "-nostdin", "-loglevel", "error"])
            .arg("-ss")
            .arg(&seek_secs)
            .arg("-i")
            .arg(&cfg.input)
            .args([
                "-an",
                "-f",
                "rawvideo",
                "-pix_fmt",
                cfg.pix_fmt.ffmpeg_name(),
            ])
            .arg("pipe:1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // 37 §2.2: on Linux ask the kernel to SIGKILL this ffmpeg child if the
        // editor process dies, so a hard-killed parent leaves no orphan decoder.
        crate::media::child_registry::arm_parent_death_signal(&mut command);
        let mut child = command.spawn().map_err(DecodeError::Spawn)?;

        // We requested `Stdio::piped()`, so stdout should be present — but if
        // ffmpeg was killed / exited between spawn and here, `take` yields None.
        // Return a typed error instead of panicking, and don't leak the child
        // (std's `Child` drop does not kill the process).
        let stdout = match child.stdout.take() {
            Some(s) => s,
            None => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(DecodeError::Spawn(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "ffmpeg stdout pipe unavailable (process exited during spawn)",
                )));
            }
        };

        // Drain stderr on a worker thread; a full pipe would wedge ffmpeg.
        let stderr_tail = Arc::new(Mutex::new(Vec::<String>::new()));
        if let Some(stderr) = child.stderr.take() {
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

        Ok((Sidecar { child, stderr_tail }, stdout))
    }

    /// The last lines ffmpeg wrote to stderr (for a `DecodeError` message).
    pub fn stderr_tail(&self) -> String {
        // Poison-tolerant: recover the tail even if the drain thread panicked.
        self.stderr_tail
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .join("\n")
    }

    /// Whether the process has exited (non-blocking check).
    pub fn has_exited(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(Some(_)))
    }
}

impl Drop for Sidecar {
    fn drop(&mut self) {
        // Kill-on-drop (02 §3). Ignore errors — the process may already be gone.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
