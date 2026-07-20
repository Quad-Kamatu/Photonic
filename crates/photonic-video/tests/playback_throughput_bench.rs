//! Headless playback-throughput benchmark (diagnostic, `#[ignore]`d).
//!
//! Answers "is preview choppy because full-res decode/composite can't hit the
//! frame budget, and does the half-res proxy fix it?" by driving the *real*
//! `VideoEngine` on a real GPU + real ffmpeg sidecar decode, in real time, and
//! reporting frames-dropped + effective presented FPS.
//!
//! Not a pass/fail correctness test — it prints a table. Run with:
//!   cargo test -p photonic-video --release --test playback_throughput_bench -- --ignored --nocapture
//!
//! Skips (never fails) when no GPU adapter or no ffmpeg is present, per the
//! crate's decode/GPU test convention.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use photonic_core::timeline::{
    AssetKind, Clip, ClipSource, FrameRate, MediaAsset, ProxyRef, ProxyStatus, Sequence, Tick,
    TimelineProject, Track, TrackKind,
};
use photonic_core::{CommandHistory, Document};

use photonic_video::media::ffmpeg_locate::locate_for_test;
use photonic_video::media::proxy::generate_proxy;
use photonic_video::{EngineCmd, GpuContext, ProxyMode, VideoEngine};

const PLAY_SECS: u64 = 8;
const POLL: Duration = Duration::from_millis(4);

struct BenchResult {
    label: String,
    dropped: u64,
    /// Real decoded frames delivered to the compositor (ring hits) in the
    /// window — the honest smoothness signal. Skipped frames composite
    /// transparent, so distinct presented ticks would over-count.
    real_frames: u64,
    window_secs: f64,
    src_fps: f64,
}

impl BenchResult {
    fn real_fps(&self) -> f64 {
        self.real_frames as f64 / self.window_secs
    }
    fn smoothness_pct(&self) -> f64 {
        (self.real_fps() / self.src_fps * 100.0).min(100.0)
    }
}

/// ffmpeg-generate a moving-content clip at `w`x`h`, 30fps, `secs` long, H.264
/// with a normal GOP (so seeks require keyframe hunts — the real-world case).
fn gen_clip(ffmpeg: &Path, out: &Path, w: u32, h: u32, secs: u32) {
    let src = format!("testsrc2=size={w}x{h}:rate=30:duration={secs}");
    let status = Command::new(ffmpeg)
        .args(["-y", "-nostdin", "-loglevel", "error", "-f", "lavfi", "-i"])
        .arg(&src)
        .args([
            "-c:v", "libx264", "-preset", "medium", "-pix_fmt", "yuv420p",
            // GOP 60 (2s) with B-frames: realistic long-GOP, expensive seeks.
            "-g", "60", "-bf", "2",
        ])
        .arg(out)
        .status()
        .expect("spawn ffmpeg to generate clip");
    assert!(status.success(), "ffmpeg clip generation failed");
}

fn doc_with_project(project: TimelineProject, w: f64, h: f64) -> Arc<Mutex<Document>> {
    let mut doc = Document::new("bench", w, h);
    doc.timeline = Some(project);
    Arc::new(Mutex::new(doc))
}

/// Play the given project in real time for `PLAY_SECS`, sampling dropped-frame
/// count and the set of distinct frame ticks actually presented.
fn play_and_measure(
    gpu: &GpuContext,
    clip: &Path,
    proxy: Option<&Path>,
    w: u32,
    h: u32,
    label: &str,
) -> BenchResult {
    let rate = FrameRate::FPS_30;

    let mut project = TimelineProject::new();
    let mut asset = MediaAsset::from_file(AssetKind::Video, clip.to_path_buf());
    if let Some(p) = proxy {
        asset.proxy = Some(ProxyRef {
            path: p.to_path_buf(),
            status: ProxyStatus::Ready,
        });
    }
    let asset_id = project.media.insert(asset);

    let mut seq = Sequence::new("seq", rate, w, h);
    let seq_id = seq.id;
    let mut v1 = Track::new(TrackKind::Video, "V1");
    v1.clips.push(Clip::new(
        ClipSource::Asset { asset: asset_id },
        Tick(0),
        Tick::from_seconds(PLAY_SECS as i64),
    ));
    seq.video_tracks.push(v1);
    project.insert_sequence(seq);
    project.active_sequence = Some(seq_id);

    let doc = doc_with_project(project, w as f64, h as f64);
    let history = Arc::new(Mutex::new(CommandHistory::new(64)));
    let engine = VideoEngine::new(gpu.clone());
    let session = engine.open_session(doc, history);

    session.send(EngineCmd::SetProxyMode(if proxy.is_some() {
        ProxyMode::ForceProxy
    } else {
        ProxyMode::ForceOriginal
    }));
    session.send(EngineCmd::Seek(Tick(0)));
    // Let the first frame decode so the ring is warm before we time.
    std::thread::sleep(Duration::from_millis(400));

    let dropped_before = session.status().dropped;
    let (hits_before, _) = photonic_video::session::decode_stats();
    let mut presented: BTreeSet<i64> = BTreeSet::new();
    let start = Instant::now();
    session.send(EngineCmd::Play);
    let end = start + Duration::from_secs(PLAY_SECS);
    while Instant::now() < end {
        if let Some(f) = session.latest_frame() {
            presented.insert(f.time.0);
        }
        std::thread::sleep(POLL);
    }
    let window_secs = start.elapsed().as_secs_f64();
    session.send(EngineCmd::Pause);
    let dropped = session.status().dropped.saturating_sub(dropped_before);
    let (hits, misses) = photonic_video::session::decode_stats();
    let (wseeks, wpumped) = photonic_video::decode::worker::worker_stats();
    let real_frames = hits.saturating_sub(hits_before);
    println!(
        "  [{label}] engine: real_frames={real_frames} skips_total={misses} | worker: seeks={wseeks} pumped={wpumped} | distinct_ticks={}",
        presented.len()
    );
    session.shutdown();

    BenchResult {
        label: label.to_string(),
        dropped,
        real_frames,
        window_secs,
        src_fps: 30.0,
    }
}

fn run_resolution(gpu: &GpuContext, ffmpeg: &Path, dir: &Path, w: u32, h: u32) -> Vec<BenchResult> {
    let clip = dir.join(format!("bench_{w}x{h}.mp4"));
    gen_clip(ffmpeg, &clip, w, h, PLAY_SECS as u32 + 2);

    let proxy = dir.join(format!("bench_{w}x{h}.proxy.mp4"));
    let tools = locate_for_test().expect("ffmpeg tools");
    generate_proxy(&tools, &clip, &proxy, &|| false).expect("generate proxy");

    vec![
        play_and_measure(gpu, &clip, None, w, h, &format!("{w}x{h} FULL")),
        play_and_measure(gpu, &clip, Some(&proxy), w, h, &format!("{w}x{h} PROXY")),
    ]
}

#[test]
#[ignore = "diagnostic benchmark; run explicitly with --ignored --nocapture"]
fn playback_throughput_full_vs_proxy() {
    let Some(gpu) = GpuContext::request_blocking() else {
        eprintln!("no GPU adapter — skipping playback throughput bench");
        return;
    };
    let Some(tools) = locate_for_test() else {
        eprintln!("ffmpeg/ffprobe not found — skipping playback throughput bench");
        return;
    };
    let ffmpeg = tools.ffmpeg.clone();

    let dir: PathBuf = std::env::temp_dir().join("photonic-playback-bench");
    std::fs::create_dir_all(&dir).unwrap();

    let mut results = Vec::new();
    for (w, h) in [(1920u32, 1080u32), (3840, 2160)] {
        results.extend(run_resolution(&gpu, &ffmpeg, &dir, w, h));
    }

    println!("\n=== Playback throughput: {PLAY_SECS}s real-time, source 30fps ===");
    println!(
        "{:<16} {:>10} {:>12} {:>12}",
        "case", "dropped", "real fps", "smoothness"
    );
    for r in &results {
        println!(
            "{:<16} {:>10} {:>12.1} {:>11.0}%",
            r.label,
            r.dropped,
            r.real_fps(),
            r.smoothness_pct()
        );
    }
    println!(
        "\n(real fps = real decoded frames delivered to the compositor per second;\n \
         30.0 = perfectly smooth. dropped = cover-interval late-drops.)"
    );
}
