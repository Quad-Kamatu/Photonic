//! End-to-end **UI code paths** for the video editor residual workstream.
//!
//! These are not unit re-implementations: they call the same functions the GUI
//! uses when the user imports media, sets source marks, builds proxies, and
//! drives the single-monitor engine bridge (`panels/media_pool`,
//! `app/source_marks`, `app/engine`, `app/timeline/interact`).
//!
//! Runs headlessly when no display is present; GPU/ffmpeg cases skip cleanly
//! (same convention as photonic-video integration tests).

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use photonic_core::history::{Command, CommandHistory};
use photonic_core::timeline::{
    ops, AssetKind, ClipSource, FrameRate, MediaAsset, Sequence, Tick, TimelineProject, Track,
    TrackKind,
};
use photonic_core::Document;
use photonic_gui::app::source_marks::{io_targets_source_marks, SourceMarksSession};
use photonic_gui::app::timeline::interact;
use photonic_gui::panels::media_pool::{guess_asset_kind, MediaPoolUi};
use photonic_video::{
    coalesce_commands, EngineCmd, PreviewQuality, PreviewTarget, ProxyMode, VideoEngine,
};

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../photonic-video/tests/fixtures")
        .canonicalize()
        .unwrap_or_else(|_| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../photonic-video/tests/fixtures")
        })
}

fn counter_mp4() -> PathBuf {
    fixtures_dir().join("counter.mp4")
}

fn beep_wav() -> PathBuf {
    fixtures_dir().join("beep_flash.wav")
}

fn tools_or_skip() -> Option<photonic_video::media::FfmpegTools> {
    photonic_video::media::locate().ok()
}

// ── 1. Media pool import ladder as the UI runs it ───────────────────────────

/// Drop multi-select into the pool: L0 stubs on the calling thread (same as
/// `spawn_import` return), then worker fills meta — mirrors media_pool drawer.
#[test]
fn ui_import_ladder_l0_then_meta_drain() {
    let paths = vec![counter_mp4(), beep_wav(), PathBuf::from("/nope/notes.txt")];
    // Some fixtures may be missing in thin checkouts.
    let paths: Vec<PathBuf> = paths
        .into_iter()
        .filter(|p| p.is_file() || p.extension().map(|e| e == "txt").unwrap_or(false))
        .collect();
    let media_paths: Vec<_> = paths
        .iter()
        .filter(|p| guess_asset_kind(p).is_some() && p.is_file())
        .cloned()
        .collect();
    if media_paths.is_empty() {
        eprintln!("skip ui_import: no fixtures");
        return;
    }

    let mut pool_ui = MediaPoolUi::default();
    let t0 = Instant::now();
    let stubs = pool_ui.spawn_import(media_paths.clone(), None, None);
    let l0_ms = t0.elapsed().as_millis();
    assert_eq!(stubs.len(), media_paths.len());
    assert!(stubs
        .iter()
        .all(|a| a.probe.is_none() && a.content_hash.is_none()));
    assert!(l0_ms < 100, "UI L0 must stay cheap ({l0_ms} ms)");

    // Apply L0 AddAsset like PhotonicApp does after spawn_import.
    let mut doc = Document::new("ui", 1920.0, 1080.0);
    let mut history = CommandHistory::new(64);
    let mut project = TimelineProject::new();
    let seq = Sequence::new("Main", FrameRate::FPS_30, 320, 180);
    let seq_id = seq.id;
    project.sequences.insert(seq_id, seq);
    project.active_sequence = Some(seq_id);
    project.sequence_order.push(seq_id);
    for asset in &stubs {
        project.media.insert(asset.clone());
    }
    doc.timeline = Some(project);

    // Poll meta like draw() does — up to 30s for fixture ladder.
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut got = 0usize;
    while Instant::now() < deadline {
        let updates = pool_ui.drain_meta();
        for m in updates {
            if m.waveform_only {
                continue;
            }
            got += 1;
            if let Some(project) = doc.timeline.as_ref() {
                if let Ok(cmd) = ops::set_asset_meta(project, m.asset, m.probe, m.content_hash) {
                    history.execute_discrete(Command::Timeline(cmd), &mut doc);
                }
            }
        }
        // One meta message per asset for L1–L4; waveform_only is extra.
        if got >= stubs.len() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        got >= stubs.len(),
        "UI meta drain expected ≥{} L1–L4 messages, got {got}",
        stubs.len()
    );
    let project = doc.timeline.as_ref().unwrap();
    for s in &stubs {
        let a = project.media.assets.get(&s.id).expect("asset");
        // After worker, video should have hash (when tools present).
        if tools_or_skip().is_some() && a.kind == AssetKind::Video {
            assert!(
                a.content_hash.is_some() || a.probe.is_some(),
                "video asset should receive meta under ffmpeg"
            );
        }
    }
}

// ── 2. Source marks + Insert as the transport/command palette do ────────────

#[test]
fn ui_source_marks_insert_overwrite_path() {
    let path = counter_mp4();
    if !path.is_file() {
        eprintln!("skip marks insert: no fixture");
        return;
    }

    let mut doc = Document::new("ui", 1920.0, 1080.0);
    let mut history = CommandHistory::new(64);
    let mut project = TimelineProject::new();
    let mut seq = Sequence::new("Main", FrameRate::FPS_30, 320, 180);
    let seq_id = seq.id;
    let track = Track::new(TrackKind::Video, "V1");
    let track_id = track.id;
    seq.video_tracks.push(track);
    project.sequences.insert(seq_id, seq);
    project.active_sequence = Some(seq_id);
    project.sequence_order.push(seq_id);

    let asset = MediaAsset::from_file(AssetKind::Video, &path);
    let asset_id = asset.id;
    project.media.insert(asset.clone());
    doc.timeline = Some(project);

    // Pool click arms + peeks (SOURCE) — same session fields as PhotonicApp.
    let mut marks = SourceMarksSession::default();
    marks.arm(asset_id, Tick::ZERO);
    assert!(io_targets_source_marks(true, false, marks.armed_asset));
    // User hits I / O on transport while SOURCE badge shows.
    marks.set_in(Tick(100_000));
    marks.set_out(Tick(800_000));
    let default_dur = Tick(5_000_000);
    let pending = marks
        .pending_source(&asset, default_dur)
        .expect("marks → pending like arm_pending_source");
    assert_eq!(pending.src_in, Tick(100_000));
    assert_eq!(pending.src_out, Tick(800_000));

    // Insert at playhead — real UI insert-edit path (`,`).
    let playhead = Tick(0);
    let clip = pending.to_clip(playhead);
    interact::do_insert_edit(&mut doc, &mut history, seq_id, track_id, playhead, clip);

    let project = doc.timeline.as_ref().unwrap();
    let seq = project.sequences.get(&seq_id).unwrap();
    let t = seq.video_tracks.iter().find(|t| t.id == track_id).unwrap();
    assert_eq!(t.clips.len(), 1);
    assert_eq!(t.clips[0].source_in, Tick(100_000));
    assert_eq!(t.clips[0].duration, Tick(700_000));
    match &t.clips[0].source {
        ClipSource::Asset { asset: a } => assert_eq!(*a, asset_id),
        other => panic!("expected Asset clip, got {other:?}"),
    }

    // SEQUENCE focus: I/O must not target source marks (work range path).
    assert!(!io_targets_source_marks(false, false, marks.armed_asset));
    // Play wins: even SOURCE badge, playing sequence steals I/O.
    assert!(!io_targets_source_marks(true, true, marks.armed_asset));
}

// ── 3. Single-monitor engine bridge (Draft / proxy / peek / play-wins) ──────

#[test]
fn ui_engine_bridge_preview_target_and_proxy_mode() {
    let Some(gpu) = photonic_video::GpuContext::request_blocking() else {
        eprintln!("skip ui_engine_bridge: no GPU (try xvfb + llvmpipe)");
        return;
    };
    let path = counter_mp4();
    if !path.is_file() {
        eprintln!("skip ui_engine_bridge: no fixture");
        return;
    }

    let engine = VideoEngine::new(gpu);
    let doc = Arc::new(Mutex::new(Document::new("ui", 320.0, 180.0)));
    let history = Arc::new(Mutex::new(CommandHistory::new(16)));
    {
        let mut d = doc.lock().unwrap();
        let mut project = TimelineProject::new();
        let mut seq = Sequence::new("Main", FrameRate::FPS_30, 320, 180);
        let seq_id = seq.id;
        let asset = MediaAsset::from_file(AssetKind::Video, &path);
        let asset_id = asset.id;
        project.media.insert(asset);
        let mut track = Track::new(TrackKind::Video, "V1");
        track.clips.push(photonic_core::timeline::Clip::new(
            ClipSource::Asset { asset: asset_id },
            Tick::ZERO,
            Tick(1_000_000),
        ));
        seq.video_tracks.push(track);
        project.sequences.insert(seq_id, seq);
        project.active_sequence = Some(seq_id);
        project.sequence_order.push(seq_id);
        d.timeline = Some(project);
    }

    let session = engine.open_session(Arc::clone(&doc), Arc::clone(&history));
    let asset_id = {
        let d = doc.lock().unwrap();
        *d.timeline
            .as_ref()
            .unwrap()
            .media
            .assets
            .keys()
            .next()
            .unwrap()
    };
    let seq_id = {
        let d = doc.lock().unwrap();
        d.timeline.as_ref().unwrap().active_sequence.unwrap()
    };
    // Ensure paused before peek (play-wins would ignore asset target).
    session.send(EngineCmd::Pause);
    session.send(EngineCmd::SetActiveSequence(seq_id));
    session.send(EngineCmd::SetPreviewQuality(PreviewQuality::Draft));
    session.send(EngineCmd::SetProxyMode(ProxyMode::Auto));
    // Pool click → SOURCE peek (same EngineCmds as EngineBridge::peek_asset).
    session.send(EngineCmd::SetPreviewTarget(PreviewTarget::Asset {
        asset: asset_id,
        source_time: Tick::ZERO,
    }));
    // Poll until engine thread applies (status is arc-swap published).
    let mut saw_asset = false;
    for _ in 0..50 {
        std::thread::sleep(Duration::from_millis(40));
        let st = session.status();
        if matches!(st.preview_target, PreviewTarget::Asset { asset, .. } if asset == asset_id) {
            saw_asset = true;
            assert_eq!(st.preview_quality, PreviewQuality::Draft);
            break;
        }
    }
    assert!(
        saw_asset,
        "engine status never showed Asset peek after SetPreviewTarget"
    );

    // Play wins: Play forces sequence target (24 §3.2) — engine Play path.
    session.send(EngineCmd::Play);
    let mut saw_seq_or_play = false;
    for _ in 0..50 {
        std::thread::sleep(Duration::from_millis(40));
        let st = session.status();
        if matches!(st.preview_target, PreviewTarget::Sequence { .. }) || st.playing {
            saw_seq_or_play = true;
            break;
        }
    }
    assert!(
        saw_seq_or_play,
        "after Play, expect sequence target or playing"
    );
    session.send(EngineCmd::Pause);

    // Coalesce as GUI scrub does.
    let batch = vec![
        EngineCmd::ScrubSeek(Tick(10)),
        EngineCmd::ScrubSeek(Tick(20)),
        EngineCmd::Seek(Tick(30)),
    ];
    let out = coalesce_commands(batch);
    assert!(matches!(out.last(), Some(EngineCmd::Seek(Tick(30)))));

    session.shutdown();
}

// ── 4. Proxy attach path used by media pool context menu ────────────────────

#[test]
fn ui_attach_proxy_then_resolve_decode() {
    let Some(tools) = tools_or_skip() else {
        eprintln!("skip ui_attach_proxy: no ffmpeg");
        return;
    };
    let path = counter_mp4();
    if !path.is_file() {
        return;
    }
    let cache = std::env::temp_dir().join(format!("photonic-ui-proxy-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&cache);
    std::fs::create_dir_all(&cache).unwrap();
    let hash = photonic_video::media::content_hash(&path).unwrap();
    let proxy_out = photonic_video::media::proxy_cache_path(&cache, &hash);
    photonic_video::media::generate_proxy(&tools, &path, &proxy_out, &|| false)
        .expect("generate stand-in proxy");

    // validate_attach is what MediaAttachProxy panel action calls after rfd.
    let report = photonic_video::media::validate_attach(&tools, &path, &proxy_out, true)
        .expect("attach validation");
    assert!(proxy_out.is_file());
    assert_eq!(
        report.proxy.origin,
        photonic_core::timeline::ProxyOrigin::Attached
    );

    // Draft Auto path selects proxy file when Ready.
    let decoded = photonic_video::media::resolve_decode_input(&path, Some(&report.proxy), true);
    assert_eq!(decoded, proxy_out);
    // Export / ForceOriginal never uses proxy.
    assert_eq!(
        photonic_video::media::resolve_decode_input(&path, Some(&report.proxy), false),
        path
    );
    let _ = std::fs::remove_dir_all(&cache);
}

// ── 5. Cut-ahead as play approaches a cut (pure scan the engine uses) ───────

#[test]
fn ui_cut_ahead_before_edit_boundary() {
    use photonic_video::playback::prefetch::{cut_ahead_targets, CUT_AHEAD_LEAD_FRAMES};

    let mut seq = Sequence::new("Main", FrameRate::FPS_30, 320, 180);
    let a1 = photonic_core::timeline::AssetId::new();
    let a2 = photonic_core::timeline::AssetId::new();
    let mut track = Track::new(TrackKind::Video, "V1");
    track.clips.push(photonic_core::timeline::Clip::new(
        ClipSource::Asset { asset: a1 },
        Tick::ZERO,
        Tick(1_000_000),
    ));
    let mut c2 = photonic_core::timeline::Clip::new(
        ClipSource::Asset { asset: a2 },
        Tick(1_000_000),
        Tick(1_000_000),
    );
    c2.source_in = Tick(10);
    track.clips.push(c2);
    seq.video_tracks.push(track);

    let lead = Tick(seq.frame_rate.ticks_per_frame().0 * CUT_AHEAD_LEAD_FRAMES);
    let targets = cut_ahead_targets(&seq, Tick::ZERO, lead.max(Tick(1_000_000)));
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].asset, a2);
}
