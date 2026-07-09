//! P3 facade/playback integration tests (02 §1/§4; fixtures are ground truth).
//!
//! Skip conventions: GPU tests skip-with-message when no adapter is available
//! (pool.rs convention); decode tests additionally skip when ffmpeg/ffprobe
//! are absent (decode_media.rs convention). Neither ever fails the suite on a
//! headless machine.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use photonic_core::timeline::{
    AssetKind, Clip, ClipSource, FrameRate, MediaAsset, Sequence, Tick, TimelineCmd,
    TimelineProject, Track, TrackKind,
};
use photonic_core::{Color, Command, CommandHistory, Document};

use photonic_video::decode::scheduler::{PtsKind, SourceParams};
use photonic_video::decode::{DecodeSource, PixFmt, SharedRing};
use photonic_video::graph::eval::read_texture_rgba16f;
use photonic_video::media::ffmpeg_locate::locate_for_test;
use photonic_video::media::keyframe_index::KeyframeIndex;
use photonic_video::media::probe::probe_details;
use photonic_video::{
    colorimetry_for_probe, EngineCmd, EngineFrame, EngineSession, GpuContext, VideoEngine,
};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

macro_rules! gpu_or_skip {
    () => {
        match GpuContext::request_blocking() {
            Some(gpu) => gpu,
            None => {
                eprintln!(
                    "no GPU adapter — skipping session test at {}:{}",
                    file!(),
                    line!()
                );
                return;
            }
        }
    };
}

macro_rules! tools_or_skip {
    () => {
        match locate_for_test() {
            Some(t) => t,
            None => {
                eprintln!(
                    "ffmpeg/ffprobe not found — skipping session test at {}:{}",
                    file!(),
                    line!()
                );
                return;
            }
        }
    };
}

/// Poll `latest_frame` until `pred` holds (engine work — probe, keyframe
/// index, cold seek — happens on its own thread).
fn wait_frame(
    session: &EngineSession,
    timeout: Duration,
    pred: impl Fn(&EngineFrame) -> bool,
) -> Option<Arc<EngineFrame>> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Some(frame) = session.latest_frame() {
            if pred(&frame) {
                return Some(frame);
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    None
}

fn doc_with_project(project: TimelineProject, w: f64, h: f64) -> Arc<Mutex<Document>> {
    let mut doc = Document::new("session-test", w, h);
    doc.timeline = Some(project);
    Arc::new(Mutex::new(doc))
}

// ── 1. Headless session: 2 tracks over counter.mp4 + color_bars.mp4 ──────────

#[test]
fn headless_session_seek_then_step_matches_decode_truth() {
    let gpu = gpu_or_skip!();
    let tools = tools_or_skip!();

    let rate = FrameRate::FPS_30;
    let tpf = rate.ticks_per_frame().0;

    // Tiny 2-track project: counter.mp4 on V1 [0,10s), color_bars.mp4 on V2
    // [4s,8s) — at the probed ticks (frames 75/76 ≈ 2.5s) only the counter is
    // visible, so the sampled pixels are decode truth for counter.mp4.
    let mut project = TimelineProject::new();
    let counter_id = project.media.insert(MediaAsset::from_file(
        AssetKind::Video,
        fixture("counter.mp4"),
    ));
    let bars_id = project.media.insert(MediaAsset::from_file(
        AssetKind::Video,
        fixture("color_bars.mp4"),
    ));

    let mut seq = Sequence::new("seq", rate, 320, 180);
    let seq_id = seq.id;
    let mut v1 = Track::new(TrackKind::Video, "V1");
    v1.clips.push(Clip::new(
        ClipSource::Asset { asset: counter_id },
        Tick(0),
        Tick::from_seconds(10),
    ));
    let mut v2 = Track::new(TrackKind::Video, "V2");
    v2.clips.push(Clip::new(
        ClipSource::Asset { asset: bars_id },
        Tick::from_seconds(4),
        Tick::from_seconds(4),
    ));
    seq.video_tracks.push(v1);
    seq.video_tracks.push(v2);
    project.insert_sequence(seq);
    project.active_sequence = Some(seq_id);

    let doc = doc_with_project(project, 320.0, 180.0);
    let history = Arc::new(Mutex::new(CommandHistory::new(64)));

    let engine = VideoEngine::new(gpu.clone());
    let session = engine.open_session(doc, history);

    // Seek to a MID-frame tick inside frame 75: EngineFrame.time must be the
    // exact frame-start tick (cover-interval rule), not the raw seek target.
    session.send(EngineCmd::Seek(Tick(75 * tpf + tpf / 3)));
    let f75 = wait_frame(&session, Duration::from_secs(30), |f| {
        f.time == Tick(75 * tpf)
    })
    .expect("frame 75 presented with exact frame-start time");
    assert_eq!(f75.sequence, seq_id);
    assert_eq!(f75.time, Tick(75 * tpf), "EngineFrame.time is tick-exact");
    let engine_px_75 = read_texture_rgba16f(&gpu, &f75.texture, 320, 180);

    // Decode truth: the same frame through DecodeSource + the same
    // YUV→working conversion, entirely outside the engine.
    let truth_px_75 = decode_truth_pixels(&gpu, &tools, Tick(75 * tpf));
    assert_frames_match(&engine_px_75, &truth_px_75, 320, "frame 75");

    // Step exactly one frame (CAP-004): presented tick = frame 76's start.
    session.send(EngineCmd::Step(1));
    let f76 = wait_frame(&session, Duration::from_secs(15), |f| {
        f.time == Tick(76 * tpf)
    })
    .expect("step presents exactly frame 76");
    assert_eq!(
        f76.time,
        Tick(76 * tpf),
        "Step lands on the exact next tick"
    );
    let engine_px_76 = read_texture_rgba16f(&gpu, &f76.texture, 320, 180);
    let truth_px_76 = decode_truth_pixels(&gpu, &tools, Tick(76 * tpf));
    assert_frames_match(&engine_px_76, &truth_px_76, 320, "frame 76");

    // The burn-in digits advanced 75 → 76: the top-left region must differ.
    let burn_in_diff: f32 = (0..30)
        .flat_map(|y| (0..80).map(move |x| y * 320 + x))
        .map(|i| {
            (0..3)
                .map(|c| (engine_px_75[i][c] - engine_px_76[i][c]).abs())
                .sum::<f32>()
        })
        .sum();
    assert!(
        burn_in_diff > 1.0,
        "burn-in region differs between frames 75 and 76 (diff {burn_in_diff})"
    );

    // Status reflects the paused, stepped state.
    let status = session.status();
    assert!(!status.playing);
    assert_eq!(status.playhead, Tick(76 * tpf));
    assert_eq!(status.active_sequence, Some(seq_id));

    session.shutdown();
}

fn decode_truth_pixels(
    gpu: &GpuContext,
    tools: &photonic_video::media::ffmpeg_locate::FfmpegTools,
    target: Tick,
) -> Vec<[f32; 4]> {
    let tex = decode_truth_texture(gpu, tools, target);
    read_texture_rgba16f(gpu, &tex, 320, 180)
}

/// Decode `counter.mp4` at `target` outside the engine and upload it with the
/// identical probe-derived colorimetry — the ground-truth pixel path.
fn decode_truth_texture(
    gpu: &GpuContext,
    tools: &photonic_video::media::ffmpeg_locate::FfmpegTools,
    target: Tick,
) -> wgpu::Texture {
    let path = fixture("counter.mp4");
    let details = probe_details(tools, &path).expect("probe counter");
    let colorimetry = colorimetry_for_probe(&details);
    let keyframes = KeyframeIndex::build(tools, &path).expect("keyframe index");
    let params = SourceParams {
        input: path,
        width: 320,
        height: 180,
        pix_fmt: PixFmt::Yuv420p,
        pts_kind: PtsKind::Cfr(FrameRate::FPS_30),
        keyframes,
    };
    let mut src = DecodeSource::new(tools.clone(), params, SharedRing::preview());
    let frame = src.seek(target).expect("truth seek");
    assert_eq!(frame.pts, target, "truth decode is pts-exact");
    photonic_render::video::convert_yuv_planes_to_working(
        gpu.device(),
        gpu.queue(),
        &frame.planes.as_yuv_planes(),
        colorimetry,
    )
}

/// Full-frame compare with an f16-round-trip tolerance: the engine path adds
/// two extra `Rgba16Float` render hops (Transform2D + Output), which must be
/// pixel-exact identity maps over the decode-truth upload.
fn assert_frames_match(engine: &[[f32; 4]], truth: &[[f32; 4]], width: usize, label: &str) {
    assert_eq!(engine.len(), truth.len(), "{label}: same pixel count");
    for (i, (e_px, t_px)) in engine.iter().zip(truth.iter()).enumerate() {
        for c in 0..4 {
            let (e, t) = (e_px[c], t_px[c]);
            assert!(
                (e - t).abs() < 0.02,
                "{label}: pixel ({},{}) channel {c}: engine {e} vs truth {t}",
                i % width,
                i / width
            );
        }
    }
}

// ── 2. Soft-clock playback: monotonic presentation, exact frame ticks ────────

#[test]
fn playback_presents_monotonic_exact_frame_ticks() {
    let gpu = gpu_or_skip!();

    // Solid-color project: no ffmpeg dependency; the audio engine opens
    // lazily and falls back to the soft clock when no device exists.
    let rate = FrameRate::FPS_30;
    let tpf = rate.ticks_per_frame().0;
    let mut project = TimelineProject::new();
    let mut seq = Sequence::new("seq", rate, 16, 16);
    let seq_id = seq.id;
    let mut v1 = Track::new(TrackKind::Video, "V1");
    v1.clips.push(Clip::new(
        ClipSource::SolidColor {
            color: Color {
                r: 0.2,
                g: 0.4,
                b: 0.8,
                a: 1.0,
            },
        },
        Tick(0),
        Tick::from_seconds(10),
    ));
    seq.video_tracks.push(v1);
    project.insert_sequence(seq);
    project.active_sequence = Some(seq_id);

    let doc = doc_with_project(project, 16.0, 16.0);
    let history = Arc::new(Mutex::new(CommandHistory::new(64)));
    let engine = VideoEngine::new(gpu);
    let session = engine.open_session(doc, history);

    session.send(EngineCmd::Seek(Tick(0)));
    wait_frame(&session, Duration::from_secs(10), |f| f.time == Tick(0)).expect("initial frame");

    session.send(EngineCmd::Play);
    let mut times: Vec<Tick> = Vec::new();
    let end = Instant::now() + Duration::from_millis(400);
    while Instant::now() < end {
        if let Some(f) = session.latest_frame() {
            if times.last() != Some(&f.time) {
                times.push(f.time);
            }
        }
        std::thread::sleep(Duration::from_millis(3));
    }
    session.send(EngineCmd::Pause);

    assert!(
        times.len() >= 3,
        "several distinct frames presented over 400ms of playback (got {times:?})"
    );
    assert!(
        times.windows(2).all(|w| w[0] < w[1]),
        "presentation times are strictly monotonic: {times:?}"
    );
    assert!(
        times.iter().all(|t| t.0 % tpf == 0),
        "every presented time is an exact frame-start tick: {times:?}"
    );

    // Pause is asynchronous — poll status until the engine processed it.
    let deadline = Instant::now() + Duration::from_secs(5);
    while session.status().playing {
        assert!(Instant::now() < deadline, "engine paused after Pause");
        std::thread::sleep(Duration::from_millis(5));
    }
    session.shutdown();
}

// ── 3. Snapshot-on-revision: a TimelineCmd edit reaches the next compile ─────

#[test]
fn snapshot_on_revision_edit_is_visible_in_next_compile() {
    let gpu = gpu_or_skip!();

    let rate = FrameRate::FPS_30;
    let mut project = TimelineProject::new();
    let mut seq = Sequence::new("seq", rate, 16, 16);
    let seq_id = seq.id;
    let mut v1 = Track::new(TrackKind::Video, "V1");
    v1.clips.push(Clip::new(
        ClipSource::SolidColor {
            color: Color {
                r: 1.0,
                g: 0.0,
                b: 0.0,
                a: 1.0,
            },
        },
        Tick(0),
        Tick::from_seconds(2),
    ));
    let v2 = Track::new(TrackKind::Video, "V2"); // empty, edited below
    let v2_id = v2.id;
    seq.video_tracks.push(v1);
    seq.video_tracks.push(v2);
    project.insert_sequence(seq);
    project.active_sequence = Some(seq_id);

    let doc = doc_with_project(project, 16.0, 16.0);
    let history = Arc::new(Mutex::new(CommandHistory::new(64)));
    let engine = VideoEngine::new(gpu.clone());
    let session = engine.open_session(Arc::clone(&doc), Arc::clone(&history));

    session.send(EngineCmd::Seek(Tick(0)));
    let red = wait_frame(&session, Duration::from_secs(10), |f| f.time == Tick(0))
        .expect("initial red frame");
    let px = read_texture_rgba16f(&gpu, &red.texture, 16, 16)[8 * 16 + 8];
    assert!(
        px[0] > 0.9 && px[1] < 0.1,
        "V1 red visible before the edit ({px:?})"
    );

    // Edit the document through the real command path: insert a green clip on
    // V2 via a TimelineCmd. `CommandHistory::execute` bumps `revision`, which
    // the engine polls (doc_generation) to re-snapshot.
    {
        let mut doc = doc.lock().unwrap();
        let mut history = history.lock().unwrap();
        let green = Clip::new(
            ClipSource::SolidColor {
                color: Color {
                    r: 0.0,
                    g: 1.0,
                    b: 0.0,
                    a: 1.0,
                },
            },
            Tick(0),
            Tick::from_seconds(2),
        );
        history.execute(
            Command::Timeline(TimelineCmd::InsertClip {
                seq: seq_id,
                track: v2_id,
                clip: Box::new(green),
            }),
            &mut doc,
        );
        assert!(history.revision() > 0, "execute bumped the revision");
    }

    // The engine re-snapshots on the revision bump and re-presents: the next
    // compile at the same tick must see the green top clip.
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut last = px;
    loop {
        assert!(
            Instant::now() < deadline,
            "edit became visible (last pixel {last:?})"
        );
        if let Some(f) = session.latest_frame() {
            last = read_texture_rgba16f(&gpu, &f.texture, 16, 16)[8 * 16 + 8];
            if last[1] > 0.9 && last[0] < 0.1 {
                break; // green won
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    let status = session.status();
    assert!(
        status.doc_revision > 0,
        "status carries the snapshot revision"
    );
    session.shutdown();
}

// ── 4. Export/Probe stubs surface NotImplemented on status ───────────────────

#[test]
fn export_and_probe_are_stubbed_not_implemented() {
    let gpu = gpu_or_skip!();
    let doc = doc_with_project(TimelineProject::new(), 16.0, 16.0);
    let history = Arc::new(Mutex::new(CommandHistory::new(8)));
    let engine = VideoEngine::new(gpu);
    let session = engine.open_session(doc, history);

    session.send(EngineCmd::Probe(photonic_core::timeline::AssetId::new()));
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let status = session.status();
        if let Some(err) = &status.last_error {
            assert!(err.contains("not implemented"), "stub error: {err}");
            break;
        }
        assert!(Instant::now() < deadline, "Probe stub surfaced on status");
        std::thread::sleep(Duration::from_millis(5));
    }
    session.shutdown();
}
