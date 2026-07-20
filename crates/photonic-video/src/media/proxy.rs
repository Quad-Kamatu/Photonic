//! Proxy generation + decode-input selection (02 §6, CAP-014).
//!
//! A **proxy** is a lightweight stand-in for a heavy source: half-resolution,
//! **all-intra** H.264 (openh264-compatible baseline) in MP4, transcoded by the
//! ffmpeg sidecar. All-intra (`-g 1`, every frame an IDR keyframe) makes
//! scrubbing instant; the baseline profile keeps the stream openh264-decodable;
//! half-res quarters the pixel throughput on the preview path.
//!
//! Proxies are stored in the sidecar cache dir keyed by the source's content
//! hash (`<cache_dir>/<hash>.proxy.mp4`), mirroring the keyframe/pts index
//! naming — so they survive project moves and are rebuildable at any time. They
//! are **never required for correctness** (CAP-014): [`resolve_decode_input`]
//! silently falls back to the original whenever the proxy is absent, pending, or
//! failed, and export always uses originals.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use photonic_core::timeline::{ProxyRef, ProxyStatus};

use super::cache_dir_for_project;
use super::ffmpeg_locate::FfmpegTools;

#[derive(Debug, thiserror::Error)]
pub enum ProxyError {
    #[error("failed to spawn ffmpeg: {0}")]
    Spawn(#[source] std::io::Error),
    #[error("ffmpeg exited with {status}: {stderr}")]
    Exit { status: String, stderr: String },
    #[error("proxy cache I/O error: {0}")]
    Io(#[source] std::io::Error),
    #[error("proxy generation cancelled")]
    Cancelled,
}

// ── Cache location ───────────────────────────────────────────────────────────

/// The directory generated proxies live in.
///
/// With a saved project this is the sidecar cache dir (`<project>.photon.cache/`,
/// 01 §9) so proxies sit beside the keyframe/pts indices and survive project
/// moves. For an unsaved project (no path) they go to a shared, rebuildable
/// OS-temp cache — proxies are never required for correctness (CAP-014).
pub fn proxy_cache_dir(project_path: Option<&Path>) -> PathBuf {
    match project_path {
        Some(p) => cache_dir_for_project(p),
        None => std::env::temp_dir().join("photonic-proxy-cache"),
    }
}

/// `<cache_dir>/<content_hash>.proxy.mp4` — keyed by content hash so the proxy
/// survives project moves and is rebuildable (02 §6, 01 §9), mirroring the
/// keyframe-index cache naming.
pub fn proxy_cache_path(cache_dir: &Path, content_hash: &str) -> PathBuf {
    cache_dir.join(format!("{content_hash}.proxy.mp4"))
}

// ── Decode-input selection ───────────────────────────────────────────────────

/// Choose the decode input for an asset given whether proxy media was requested.
///
/// Returns the proxy file only when `use_proxy` is set and a [`ProxyStatus::Ready`]
/// proxy is present on disk; otherwise the original. Proxies are never required
/// for correctness (CAP-014): a missing, pending, or failed proxy silently falls
/// back to the original, so `ForceProxy` is always safe even before a proxy has
/// been generated.
pub fn resolve_decode_input(original: &Path, proxy: Option<&ProxyRef>, use_proxy: bool) -> PathBuf {
    if use_proxy {
        if let Some(p) = proxy {
            if p.status == ProxyStatus::Ready && p.path.is_file() {
                return p.path.clone();
            }
        }
    }
    original.to_path_buf()
}

// ── Generation ───────────────────────────────────────────────────────────────

/// ffmpeg encode args for the proxy profile (02 §6). `scale=trunc(iw/4)*2:…`
/// halves each dimension rounding down to an even size (yuv420p requires even
/// w/h); `-g 1` forces all-intra; `baseline` keeps it openh264-decodable. Audio
/// is dropped (`-an`) — the decode path is video-only.
const PROXY_ENCODE_ARGS: &[&str] = &[
    "-an",
    "-vf",
    "scale=trunc(iw/4)*2:trunc(ih/4)*2",
    "-c:v",
    "libx264",
    "-profile:v",
    "baseline",
    "-preset",
    "veryfast",
    "-crf",
    "23",
    "-g",
    "1",
    "-pix_fmt",
    "yuv420p",
    "-movflags",
    "+faststart",
    // Force the muxer explicitly: the output is written to a `.part` staging
    // file whose extension ffmpeg cannot map to a container on its own.
    "-f",
    "mp4",
];

/// The `.part` staging path a proxy is written to before an atomic rename, so a
/// crash or cancel never leaves a truncated file that looks Ready by path.
fn staging_path(output: &Path) -> PathBuf {
    let mut s = output.as_os_str().to_os_string();
    s.push(".part");
    PathBuf::from(s)
}

/// Transcode `input` → a half-res all-intra H.264/MP4 proxy at `output`.
///
/// Writes to a `.part` sibling and atomically renames on success. `cancel` is
/// polled between waits; when it returns `true` the ffmpeg process is killed,
/// the partial file removed, and [`ProxyError::Cancelled`] returned. The parent
/// directory of `output` is created if needed.
pub fn generate_proxy(
    tools: &FfmpegTools,
    input: &Path,
    output: &Path,
    cancel: &dyn Fn() -> bool,
) -> Result<(), ProxyError> {
    if let Some(dir) = output.parent() {
        std::fs::create_dir_all(dir).map_err(ProxyError::Io)?;
    }
    let tmp = staging_path(output);

    let mut command = Command::new(&tools.ffmpeg);
    command
        .arg("-y")
        .args(["-nostdin", "-loglevel", "error"])
        .arg("-i")
        .arg(input)
        .args(PROXY_ENCODE_ARGS)
        .arg(&tmp)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    lower_background_priority(&mut command);
    let mut child = command.spawn().map_err(ProxyError::Spawn)?;

    let status = loop {
        if cancel() {
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_file(&tmp);
            return Err(ProxyError::Cancelled);
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                return Err(ProxyError::Io(e));
            }
        }
    };

    if status.success() {
        std::fs::rename(&tmp, output).map_err(ProxyError::Io)?;
        Ok(())
    } else {
        let stderr = stderr_tail(&mut child);
        let _ = std::fs::remove_file(&tmp);
        Err(ProxyError::Exit {
            status: status.to_string(),
            stderr,
        })
    }
}

/// Make proxy transcodes cooperative with interactive playback on Unix. This
/// runs in the child immediately before `exec`, so it cannot lower the UI
/// process itself. Failure is deliberately non-fatal: containers and hardened
/// desktops may forbid changing priority, but proxy generation must still work.
#[cfg(unix)]
fn lower_background_priority(command: &mut Command) {
    use std::os::unix::process::CommandExt;

    // SAFETY: `pre_exec` runs in the newly-forked child. The closure performs
    // only the async-signal-safe `setpriority` syscall and deliberately ignores
    // a permission failure before returning to exec ffmpeg.
    unsafe {
        command.pre_exec(|| {
            let _ = libc::setpriority(libc::PRIO_PROCESS, 0, 10);
            Ok(())
        });
    }
}

#[cfg(not(unix))]
fn lower_background_priority(_command: &mut Command) {}

/// Read the last ~500 chars ffmpeg wrote to stderr (for a `ProxyError` message).
/// `-loglevel error` keeps this near-empty on success, so reading it after exit
/// (rather than draining concurrently) does not risk a pipe-full wedge here.
fn stderr_tail(child: &mut std::process::Child) -> String {
    use std::io::Read;
    let mut buf = String::new();
    if let Some(mut s) = child.stderr.take() {
        let _ = s.read_to_string(&mut buf);
    }
    let tail: String = buf.chars().rev().take(500).collect();
    tail.chars().rev().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::ffmpeg_locate::locate_for_test;

    fn unique_tmp_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "photonic-proxy-test-{tag}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn proxy_cache_path_names_by_hash() {
        let p = proxy_cache_path(Path::new("/cache"), "deadbeef");
        assert_eq!(p, PathBuf::from("/cache/deadbeef.proxy.mp4"));
    }

    #[test]
    fn proxy_cache_dir_uses_sidecar_dir_when_project_known() {
        let d = proxy_cache_dir(Some(Path::new("/proj/movie.photon")));
        assert_eq!(d, PathBuf::from("/proj/movie.photon.cache"));
        // Unsaved project ⇒ a rebuildable temp cache (never required for
        // correctness).
        assert!(proxy_cache_dir(None).ends_with("photonic-proxy-cache"));
    }

    #[test]
    fn resolve_decode_input_falls_back_unless_ready_on_disk() {
        let dir = unique_tmp_dir("resolve");
        let original = dir.join("orig.mp4");
        let proxy_file = dir.join("p.proxy.mp4");
        std::fs::write(&original, b"o").unwrap();
        std::fs::write(&proxy_file, b"p").unwrap();

        let ready = ProxyRef {
            path: proxy_file.clone(),
            status: ProxyStatus::Ready,
        };
        let pending = ProxyRef {
            path: proxy_file.clone(),
            status: ProxyStatus::Pending,
        };
        let missing = ProxyRef {
            path: dir.join("nope.proxy.mp4"),
            status: ProxyStatus::Ready,
        };

        // Ready + present + requested ⇒ proxy.
        assert_eq!(
            resolve_decode_input(&original, Some(&ready), true),
            proxy_file
        );
        // Not requested ⇒ original even when a Ready proxy exists.
        assert_eq!(
            resolve_decode_input(&original, Some(&ready), false),
            original
        );
        // Pending / missing-file / absent ⇒ original (silent fallback).
        assert_eq!(
            resolve_decode_input(&original, Some(&pending), true),
            original
        );
        assert_eq!(
            resolve_decode_input(&original, Some(&missing), true),
            original
        );
        assert_eq!(resolve_decode_input(&original, None, true), original);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Engine integration (02 §6 / CAP-014): generate a real proxy for a
    /// synthetic fixture, then assert `ForceProxy` selects the proxy input and
    /// `ForceOriginal` the original — asserted via the decode source's chosen
    /// path (`DecodeSource::input_path`), not pixels.
    #[test]
    fn generate_proxy_then_decode_source_selects_by_mode() {
        use crate::decode::scheduler::{PtsKind, SourceParams};
        use crate::decode::{DecodeSource, PixFmt, SharedRing};
        use crate::media::keyframe_index::KeyframeIndex;
        use crate::media::probe::probe_details;
        use photonic_core::timeline::FrameRate;

        let Some(tools) = locate_for_test() else {
            eprintln!("skip: ffmpeg/ffprobe not found (set PHOTONIC_FFMPEG_DIR)");
            return;
        };
        let dir = unique_tmp_dir("gen");

        // Synthetic 320×240 fixture (no display needed — lavfi testsrc).
        let input = dir.join("input.mp4");
        let made = Command::new(&tools.ffmpeg)
            .args([
                "-y",
                "-nostdin",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc=size=320x240:rate=10:duration=1",
                "-pix_fmt",
                "yuv420p",
            ])
            .arg(&input)
            .status();
        if !matches!(made, Ok(s) if s.success()) || !input.is_file() {
            eprintln!("skip: could not synthesize a fixture with this ffmpeg build");
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }

        let hash = crate::media::probe::content_hash(&input).unwrap();
        let out = proxy_cache_path(&dir, &hash);

        generate_proxy(&tools, &input, &out, &|| false).expect("proxy generation");
        assert!(out.is_file(), "proxy file was not written");
        assert!(std::fs::metadata(&out).unwrap().len() > 0);

        // Real transcode: half-res, all-intra H.264 in MP4.
        let details = probe_details(&tools, &out).expect("probe proxy");
        let v = details.probe.video.expect("proxy has a video stream");
        assert_eq!((v.width, v.height), (160, 120), "proxy should be half-res");

        // Selection: ForceProxy ⇒ proxy, ForceOriginal ⇒ original.
        let pref = ProxyRef {
            path: out.clone(),
            status: ProxyStatus::Ready,
        };
        let sel_proxy = resolve_decode_input(&input, Some(&pref), true);
        let sel_orig = resolve_decode_input(&input, Some(&pref), false);
        assert_eq!(sel_proxy, out);
        assert_eq!(sel_orig, input);

        // …and the decode source built for each honors that chosen path.
        let mk = |p: &Path| {
            DecodeSource::new(
                tools.clone(),
                SourceParams {
                    input: p.to_path_buf(),
                    width: 160,
                    height: 120,
                    pix_fmt: PixFmt::Yuv420p,
                    pts_kind: PtsKind::Cfr(FrameRate::FPS_30),
                    keyframes: KeyframeIndex::default(),
                },
                SharedRing::preview(),
            )
        };
        assert_eq!(mk(&sel_proxy).input_path(), out.as_path());
        assert_eq!(mk(&sel_orig).input_path(), input.as_path());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
