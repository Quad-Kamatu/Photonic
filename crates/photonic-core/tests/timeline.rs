//! Integration tests for the timeline data model (P2, docs/specs/video-editor).
//!
//! Covers: undo idempotency (`apply → inverse → apply == apply`) for the
//! `TimelineCmd` variants; per-op invariant enforcement; serde round-trip of a
//! fully-populated `TimelineProject`; the v2→v3 migration (v2 files load
//! unchanged); and a proptest that random edit sequences never break the
//! non-overlap invariant (11 §3.1).

use photonic_core::history::Command;
use photonic_core::timeline::commands::AnimTarget;
use photonic_core::timeline::*;
use photonic_core::Document;

// ── Fixture ─────────────────────────────────────────────────────────────────

/// A document with a small but non-trivial timeline: one sequence, a video
/// track with two clips, and an audio track with one audio-enabled clip.
struct Fixture {
    doc: Document,
    seq: SequenceId,
    vtrack: TrackId,
    atrack: TrackId,
    clip_a: ClipId,
    clip_b: ClipId,
    aclip: ClipId,
}

fn fixture() -> Fixture {
    let mut project = TimelineProject::new();
    let mut sequence = Sequence::new("Seq 1", FrameRate::FPS_30, 1920, 1080);

    let mut vtrack = Track::new(TrackKind::Video, "V1");
    let mut ca = Clip::new(
        ClipSource::SolidColor {
            color: photonic_core::Color {
                r: 1.0,
                g: 0.0,
                b: 0.0,
                a: 1.0,
            },
        },
        Tick(0),
        Tick(1000),
    );
    ca.name = "A".into();
    let mut cb = Clip::new(
        ClipSource::SolidColor {
            color: photonic_core::Color {
                r: 0.0,
                g: 1.0,
                b: 0.0,
                a: 1.0,
            },
        },
        Tick(1000),
        Tick(1000),
    );
    cb.name = "B".into();
    let (clip_a, clip_b) = (ca.id, cb.id);
    vtrack.clips.push(ca);
    vtrack.clips.push(cb);

    let mut atrack = Track::new(TrackKind::Audio, "A1");
    let mut ac = Clip::new(ClipSource::Adjustment, Tick(0), Tick(2000));
    ac.audio = Some(ClipAudio::new());
    let aclip = ac.id;
    atrack.clips.push(ac);

    let (vt, at) = (vtrack.id, atrack.id);
    sequence.video_tracks.push(vtrack);
    sequence.audio_tracks.push(atrack);
    let seq = sequence.id;
    project.insert_sequence(sequence);

    let mut doc = Document::new("t", 100.0, 100.0);
    doc.timeline = Some(project);
    Fixture {
        doc,
        seq,
        vtrack: vt,
        atrack: at,
        clip_a,
        clip_b,
        aclip,
    }
}

impl Fixture {
    fn project(&self) -> &TimelineProject {
        self.doc.timeline.as_ref().unwrap()
    }
}

/// Assert the undo contract for a single command: applying it, then its inverse,
/// restores the timeline; and applying it again reproduces the post-apply state
/// (`apply → inverse → apply == apply`).
fn assert_undo_roundtrip(doc: &Document, cmd: &TimelineCmd) {
    let before = doc.timeline.clone();

    let mut d1 = doc.clone();
    Command::Timeline(cmd.clone()).apply(&mut d1);
    let after_apply = d1.timeline.clone();

    // inverse restores the original state.
    let inv = cmd
        .inverse(&d1)
        .expect("every implemented variant has an inverse");
    let mut d2 = d1.clone();
    Command::Timeline(inv).apply(&mut d2);
    assert_eq!(
        d2.timeline,
        before,
        "inverse did not restore original for {}",
        cmd.description()
    );

    // Re-applying forward reproduces the post-apply state.
    let mut d3 = d2.clone();
    Command::Timeline(cmd.clone()).apply(&mut d3);
    assert_eq!(
        d3.timeline,
        after_apply,
        "apply→inverse→apply != apply for {}",
        cmd.description()
    );
}

// ── Undo idempotency across variants ────────────────────────────────────────

#[test]
fn create_and_remove_project_roundtrip() {
    let mut doc = Document::new("t", 100.0, 100.0);
    assert!(doc.timeline.is_none());
    let cmd = ops::create_project();
    assert_undo_roundtrip(&doc, &cmd);
    // Applying it actually creates the project.
    Command::Timeline(cmd).apply(&mut doc);
    assert!(doc.timeline.is_some());
}

#[test]
fn clip_edit_ops_roundtrip() {
    let f = fixture();
    let p = f.project();

    let cmds = vec![
        ops::move_clip(p, f.seq, f.vtrack, f.clip_b, Tick(1500)).unwrap(),
        ops::trim_clip(
            p,
            f.seq,
            f.vtrack,
            f.clip_a,
            ClipTiming {
                start: Tick(0),
                duration: Tick(500),
                source_in: Tick(0),
            },
        )
        .unwrap(),
        ops::split_clip(p, f.seq, f.vtrack, f.clip_a, Tick(500)).unwrap(),
        ops::slip_clip(p, f.seq, f.vtrack, f.clip_a, Tick(200)).unwrap(),
        ops::remove_clip(p, f.seq, f.vtrack, f.clip_b).unwrap(),
        ops::roll_edit(p, f.seq, f.vtrack, f.clip_a, f.clip_b, Tick(200)).unwrap(),
        ops::slide_clip(p, f.seq, f.vtrack, f.clip_b, Tick(0)).unwrap(),
    ];
    for c in &cmds {
        assert_undo_roundtrip(&f.doc, c);
    }
}

#[test]
fn sequence_and_track_ops_roundtrip() {
    let f = fixture();
    let p = f.project();
    let cmds = vec![
        ops::add_sequence(Sequence::new("Seq 2", FrameRate::FPS_25, 1080, 1920)),
        ops::remove_sequence(p, f.seq).unwrap(),
        ops::set_active_format(p, f.seq, 0).unwrap(),
        ops::add_format(f.seq, SequenceFormat::new("9:16", 1080, 1920)),
        ops::add_track(p, f.seq, Track::new(TrackKind::Video, "V2"), None).unwrap(),
        ops::remove_track(p, f.seq, f.atrack).unwrap(),
        ops::set_track_prop(p, f.seq, f.vtrack, {
            let mut ts = TrackSettings::of(p.sequences[&f.seq].track(f.vtrack).unwrap());
            ts.enabled = false;
            ts
        })
        .unwrap(),
    ];
    for c in &cmds {
        assert_undo_roundtrip(&f.doc, c);
    }
}

#[test]
fn effect_and_grade_ops_roundtrip() {
    let f = fixture();
    let p = f.project();
    let cmds = vec![
        ops::add_effect(
            p,
            f.seq,
            f.vtrack,
            f.clip_a,
            ClipEffect::new(EffectKind::Blur),
            None,
        )
        .unwrap(),
        ops::set_grade(p, f.seq, f.vtrack, f.clip_a, Some(Grade::new())).unwrap(),
    ];
    for c in &cmds {
        assert_undo_roundtrip(&f.doc, c);
    }

    // Removing an effect that exists.
    let mut d = f.doc.clone();
    Command::Timeline(cmds[0].clone()).apply(&mut d);
    let rem =
        ops::remove_effect(d.timeline.as_ref().unwrap(), f.seq, f.vtrack, f.clip_a, 0).unwrap();
    assert_undo_roundtrip(&d, &rem);
}

#[test]
fn keyframe_ops_roundtrip() {
    let f = fixture();
    let p = f.project();
    let target = AnimTarget::ClipTransform { clip: f.clip_a };
    let path = PropPath::new("transform.opacity");
    let kf = Keyframe::new(Tick(100), PropValue::Float(0.5), Interp::Linear);

    let set = ops::set_keyframe(p, target.clone(), path.clone(), kf);
    assert_undo_roundtrip(&f.doc, &set);

    // After setting, removing and re-interp'ing round-trip too.
    let mut d = f.doc.clone();
    Command::Timeline(set.clone()).apply(&mut d);
    let p2 = d.timeline.as_ref().unwrap();
    let rem = ops::remove_keyframe(p2, target.clone(), path.clone(), Tick(100)).unwrap();
    assert_undo_roundtrip(&d, &rem);
    let interp = ops::set_keyframe_interp(p2, target, path, Tick(100), Interp::Hold).unwrap();
    assert_undo_roundtrip(&d, &interp);
}

#[test]
fn audio_ops_roundtrip() {
    let f = fixture();
    let p = f.project();

    let set_track = TimelineCmd::AudioEdit(commands::AudioCmd::SetTrackAudioProp {
        track: f.atrack,
        old: TrackAudioParams::default(),
        new: TrackAudioParams {
            volume_db: -6.0,
            pan: 0.3,
        },
    });
    assert_undo_roundtrip(&f.doc, &set_track);

    let duck = ops::apply_ducking_preset(p, f.atrack, f.atrack);
    assert_eq!(
        duck,
        Err(ops::EditError::SidechainCycle),
        "self-duck must be rejected"
    );

    // A second audio track to duck against.
    let mut d = f.doc.clone();
    let a2 = Track::new(TrackKind::Audio, "A2");
    let a2_id = a2.id;
    Command::Timeline(ops::add_track(d.timeline.as_ref().unwrap(), f.seq, a2, None).unwrap())
        .apply(&mut d);
    let duck = ops::apply_ducking_preset(d.timeline.as_ref().unwrap(), f.atrack, a2_id).unwrap();
    assert_undo_roundtrip(&d, &duck);
}

#[test]
fn composition_ops_roundtrip_and_reject_adjustment() {
    let f = fixture();
    let p = f.project();

    // Creating a composition on a normal clip yields [AddGraph, SetClipComposition].
    let batch = ops::create_clip_composition(p, f.seq, f.vtrack, f.clip_a).unwrap();
    assert_eq!(batch.len(), 2);
    let mut d = f.doc.clone();
    for c in &batch {
        assert_undo_roundtrip(&d, c);
        Command::Timeline(c.clone()).apply(&mut d);
    }
    assert!(d.timeline.as_ref().unwrap().graphs.len() == 1);

    // Rejected on an Adjustment-source clip (07 §6.6).
    let adj = ops::create_clip_composition(p, f.seq, f.atrack, f.aclip);
    assert_eq!(adj, Err(ops::EditError::CompositionOnAdjustment));
}

#[test]
fn graph_ops_cycle_refusal() {
    let f = fixture();
    // Build a composition graph in the arena.
    let mut d = f.doc.clone();
    for c in ops::create_clip_composition(f.project(), f.seq, f.vtrack, f.clip_a).unwrap() {
        Command::Timeline(c).apply(&mut d);
    }
    let p = d.timeline.as_ref().unwrap();
    let (gid, g) = p.graphs.iter().next().unwrap();
    // The seed is ClipIn → Output; adding Output → ClipIn would cycle.
    let clip_in = g
        .nodes
        .values()
        .find(|n| matches!(n.op, GraphOp::ClipIn))
        .unwrap()
        .id;
    let out = g.output;
    let err = graph_ops::add_edge(p, *gid, (out, OutPort::PRIMARY), (clip_in, InPort::PRIMARY));
    assert_eq!(err, Err(ops::EditError::WouldCreateCycle));
}

// ── Invariant enforcement ───────────────────────────────────────────────────

#[test]
fn insert_clip_rejects_overlap_and_zero_duration() {
    let f = fixture();
    let p = f.project();
    // Overlaps clip A (0..1000).
    let overlapping = Clip::new(ClipSource::Adjustment, Tick(500), Tick(100));
    assert_eq!(
        ops::insert_clip(p, f.seq, f.vtrack, overlapping),
        Err(ops::EditError::Overlap)
    );
    // Zero duration.
    let zero = Clip::new(ClipSource::Adjustment, Tick(3000), Tick(0));
    assert_eq!(
        ops::insert_clip(p, f.seq, f.vtrack, zero),
        Err(ops::EditError::NonPositiveDuration)
    );
    // A valid placement after B succeeds.
    let ok = Clip::new(ClipSource::Adjustment, Tick(2000), Tick(500));
    assert!(ops::insert_clip(p, f.seq, f.vtrack, ok).is_ok());
}

#[test]
fn move_into_overlap_is_rejected() {
    let f = fixture();
    let p = f.project();
    // Move B onto A.
    assert_eq!(
        ops::move_clip(p, f.seq, f.vtrack, f.clip_b, Tick(500)),
        Err(ops::EditError::Overlap)
    );
}

#[test]
fn split_outside_clip_is_rejected() {
    let f = fixture();
    let p = f.project();
    assert_eq!(
        ops::split_clip(p, f.seq, f.vtrack, f.clip_a, Tick(0)),
        Err(ops::EditError::InvalidSplit)
    );
    assert_eq!(
        ops::split_clip(p, f.seq, f.vtrack, f.clip_a, Tick(5000)),
        Err(ops::EditError::InvalidSplit)
    );
}

#[test]
fn split_then_validate_holds() {
    let f = fixture();
    let mut d = f.doc.clone();
    let cmd = ops::split_clip(f.project(), f.seq, f.vtrack, f.clip_a, Tick(400)).unwrap();
    Command::Timeline(cmd).apply(&mut d);
    let seq = &d.timeline.as_ref().unwrap().sequences[&f.seq];
    assert!(seq.validate().is_ok());
    // The split produced 3 clips (A-left, A-right, B).
    assert_eq!(seq.track(f.vtrack).unwrap().clips.len(), 3);
}

#[test]
fn nested_sequence_cycle_is_rejected() {
    let f = fixture();
    let p = f.project();
    // A clip that nests the very sequence it lives in.
    let c = Clip::new(
        ClipSource::NestedSequence { sequence: f.seq },
        Tick(2000),
        Tick(100),
    );
    assert_eq!(
        ops::insert_clip(p, f.seq, f.vtrack, c),
        Err(ops::EditError::SequenceCycle)
    );
}

// ── Follow-up gaps: markers, work range, cross-track move, ripple-trim, bins ──

#[test]
fn marker_and_work_range_ops_roundtrip() {
    let f = fixture();
    let p = f.project();

    let add = ops::add_marker(p, f.seq, Marker::new(Tick(500), "cue A")).unwrap();
    assert_undo_roundtrip(&f.doc, &add);
    let set_wr = ops::set_work_range(p, f.seq, Some((Tick(100), Tick(1500)))).unwrap();
    assert_undo_roundtrip(&f.doc, &set_wr);

    // After adding two markers, remove and edit round-trip against that state.
    let mut d = f.doc.clone();
    Command::Timeline(add.clone()).apply(&mut d);
    let m2 = Marker::new(Tick(200), "cue B");
    let m2_id = m2.id;
    Command::Timeline(ops::add_marker(d.timeline.as_ref().unwrap(), f.seq, m2).unwrap())
        .apply(&mut d);

    let rem = ops::remove_marker(d.timeline.as_ref().unwrap(), f.seq, m2_id).unwrap();
    assert_undo_roundtrip(&d, &rem);

    let mut edited = Marker::new(Tick(900), "renamed");
    edited.id = m2_id; // edit the existing marker (id preserved)
    edited.note = "note".into();
    let set = ops::set_marker(d.timeline.as_ref().unwrap(), f.seq, edited).unwrap();
    assert_undo_roundtrip(&d, &set);

    // Clearing the work range (None) also round-trips.
    let mut d2 = f.doc.clone();
    Command::Timeline(set_wr).apply(&mut d2);
    let clear = ops::set_work_range(d2.timeline.as_ref().unwrap(), f.seq, None).unwrap();
    assert_undo_roundtrip(&d2, &clear);
}

#[test]
fn media_bin_ops_roundtrip() {
    let f = fixture();
    // Put an asset in the pool first.
    let mut d = f.doc.clone();
    let asset = MediaAsset::from_file(AssetKind::Video, "/clips/a.mp4");
    let asset_id = asset.id;
    Command::Timeline(ops::add_asset(asset)).apply(&mut d);

    let create = ops::create_bin("Footage", None);
    assert_undo_roundtrip(&d, &create);
    Command::Timeline(create.clone()).apply(&mut d);
    let bin_id = match &create {
        TimelineCmd::AddBin { bin } => bin.id,
        _ => unreachable!(),
    };

    let assign =
        ops::assign_asset_bin(d.timeline.as_ref().unwrap(), asset_id, Some(bin_id)).unwrap();
    assert_undo_roundtrip(&d, &assign);
    Command::Timeline(assign).apply(&mut d);
    assert_eq!(
        d.timeline.as_ref().unwrap().media.assets[&asset_id].bin,
        Some(bin_id)
    );

    let remove = ops::remove_bin(d.timeline.as_ref().unwrap(), bin_id).unwrap();
    assert_undo_roundtrip(&d, &remove);

    // Assigning to a nonexistent bin is rejected.
    assert_eq!(
        ops::assign_asset_bin(d.timeline.as_ref().unwrap(), asset_id, Some(BinId::new())),
        Err(ops::EditError::IndexOutOfRange)
    );
}

#[test]
fn cross_track_move_roundtrip_and_invariants() {
    let f = fixture();
    // Add a second video track to move onto.
    let mut d = f.doc.clone();
    let v2 = Track::new(TrackKind::Video, "V2");
    let v2_id = v2.id;
    Command::Timeline(ops::add_track(d.timeline.as_ref().unwrap(), f.seq, v2, None).unwrap())
        .apply(&mut d);

    // Move clip B from V1 to V2 (lossless inverse returns it).
    let mv = ops::move_clip_to_track(
        d.timeline.as_ref().unwrap(),
        f.seq,
        f.vtrack,
        f.clip_b,
        Tick(0),
        Some(v2_id),
    )
    .unwrap();
    assert_undo_roundtrip(&d, &mv);

    // Apply it: B leaves V1, lands on V2.
    let mut d2 = d.clone();
    Command::Timeline(mv).apply(&mut d2);
    let seq = &d2.timeline.as_ref().unwrap().sequences[&f.seq];
    assert!(seq
        .track(f.vtrack)
        .unwrap()
        .clips
        .iter()
        .all(|c| c.id != f.clip_b));
    assert!(seq
        .track(v2_id)
        .unwrap()
        .clips
        .iter()
        .any(|c| c.id == f.clip_b));
    assert!(seq.validate().is_ok());

    // Moving onto an occupied region of the destination is rejected.
    let occupied = ops::move_clip_to_track(
        d.timeline.as_ref().unwrap(),
        f.seq,
        f.vtrack,
        f.clip_b,
        Tick(0), // clip A occupies 0..1000 on V1; move onto V1 start collides via A
        None,
    );
    assert_eq!(occupied, Err(ops::EditError::Overlap));

    // Cross-kind moves (video → audio) are rejected.
    let wrong_kind = ops::move_clip_to_track(
        d.timeline.as_ref().unwrap(),
        f.seq,
        f.vtrack,
        f.clip_a,
        Tick(3000),
        Some(f.atrack),
    );
    assert_eq!(wrong_kind, Err(ops::EditError::NoTrack(f.atrack)));

    // The plain 5-arg move_clip still works (same-track) and round-trips.
    let same = ops::move_clip(
        d.timeline.as_ref().unwrap(),
        f.seq,
        f.vtrack,
        f.clip_b,
        Tick(2200),
    )
    .unwrap();
    assert_undo_roundtrip(&d, &same);
}

#[test]
fn ripple_trim_end_and_start() {
    let f = fixture();
    let p = f.project();
    // V1: A[0..1000], B[1000..2000]. Ripple-trim A's END to 600 → A[0..600],
    // and B shifts left by -400 to [600..1600].
    let end = ops::ripple_trim(p, f.seq, f.vtrack, f.clip_a, ClipEdge::End, Tick(600)).unwrap();
    assert_undo_roundtrip(&f.doc, &end);
    let mut d = f.doc.clone();
    Command::Timeline(end).apply(&mut d);
    let seq = &d.timeline.as_ref().unwrap().sequences[&f.seq];
    let clips = &seq.track(f.vtrack).unwrap().clips;
    assert_eq!(clips[0].duration, Tick(600));
    assert_eq!(clips[1].start, Tick(600));
    assert_eq!(clips[1].end(), Tick(1600));
    assert!(seq.validate().is_ok());

    // Ripple-trim B's START later to 1200 → B head trimmed by 200, B keeps its
    // timeline start, later clips (none) unaffected; invariant holds.
    let start = ops::ripple_trim(
        f.project(),
        f.seq,
        f.vtrack,
        f.clip_b,
        ClipEdge::Start,
        Tick(1200),
    )
    .unwrap();
    assert_undo_roundtrip(&f.doc, &start);
    let mut d2 = f.doc.clone();
    Command::Timeline(start).apply(&mut d2);
    let seq2 = &d2.timeline.as_ref().unwrap().sequences[&f.seq];
    assert!(seq2.validate().is_ok());

    // Trimming the end before the start is rejected.
    assert_eq!(
        ops::ripple_trim(
            f.project(),
            f.seq,
            f.vtrack,
            f.clip_a,
            ClipEdge::End,
            Tick(0)
        ),
        Err(ops::EditError::NonPositiveDuration)
    );
}

// ── Serde round-trip ────────────────────────────────────────────────────────

#[test]
fn fully_populated_project_round_trips() {
    let f = fixture();
    let mut d = f.doc.clone();
    // Enrich: add an effect, a grade, a keyframe, a composition, captions, markers.
    let steps: Vec<TimelineCmd> = {
        let p = d.timeline.as_ref().unwrap();
        let mut v = vec![
            ops::add_effect(
                p,
                f.seq,
                f.vtrack,
                f.clip_a,
                ClipEffect::new(EffectKind::Glow),
                None,
            )
            .unwrap(),
            ops::set_grade(
                p,
                f.seq,
                f.vtrack,
                f.clip_a,
                Some(Grade {
                    ops: vec![GradeOp::new(
                        GradeOpKind::Exposure,
                        GradeOpParams::Exposure { stops: 1.0 },
                    )],
                    bypass: false,
                }),
            )
            .unwrap(),
            ops::set_keyframe(
                p,
                AnimTarget::ClipTransform { clip: f.clip_b },
                PropPath::new("transform.x"),
                Keyframe::new(
                    Tick(0),
                    PropValue::Float(0.0),
                    Interp::Bezier {
                        out_handle: [0.42, 0.0],
                        in_handle: [0.58, 1.0],
                    },
                ),
            ),
        ];
        v.extend(ops::create_clip_composition(p, f.seq, f.vtrack, f.clip_b).unwrap());
        v
    };
    for c in steps {
        Command::Timeline(c).apply(&mut d);
    }
    // Add a caption track + marker directly on the sequence.
    {
        let p = d.timeline.as_mut().unwrap();
        let s = p.sequences.get_mut(&f.seq).unwrap();
        let mut ct = CaptionTrack::new("Captions");
        ct.cues.push(CaptionCue::new(
            Tick(0),
            Tick(500),
            vec![
                CaptionWord::new("hello", Tick(0), Tick(250)),
                CaptionWord::new("world", Tick(250), Tick(500)),
            ],
        ));
        s.caption_tracks.push(ct);
        s.markers.push(Marker::new(Tick(100), "cue"));
        s.work_range = Some((Tick(0), Tick(1500)));
    }

    let project = d.timeline.as_ref().unwrap();
    let json = serde_json::to_string(project).unwrap();
    let back: TimelineProject = serde_json::from_str(&json).unwrap();
    assert_eq!(project, &back, "TimelineProject serde round-trip mismatch");

    // And the whole Document round-trips (with the timeline present).
    let dj = serde_json::to_string(&d).unwrap();
    let dback: Document = serde_json::from_str(&dj).unwrap();
    assert_eq!(dback.timeline.as_ref(), Some(&back));
}

// ── Migration: v2 files load unchanged ──────────────────────────────────────

#[test]
fn v2_document_loads_as_v3_without_timeline() {
    use photonic_core::migration::{detect_version, run_migrations};

    // A v2 document is the current serialization minus `timeline`, with
    // format_version pinned to 2.
    let mut value = serde_json::to_value(Document::new("legacy", 640.0, 480.0)).unwrap();
    let obj = value.as_object_mut().unwrap();
    obj.insert("format_version".into(), serde_json::json!(2));
    obj.remove("timeline");
    assert_eq!(detect_version(&value), 2);

    // Migrate forward to current (3) and deserialize.
    let out = run_migrations(&mut value, 3).unwrap();
    assert_eq!(out, 3, "v2 must migrate to v3");
    let doc: Document = serde_json::from_value(value).unwrap();
    assert_eq!(doc.format_version, 3);
    assert!(doc.timeline.is_none(), "v2 file must load with no timeline");
    assert_eq!(doc.name, "legacy");
}

// ── Proptest: random edits never break non-overlap ──────────────────────────

use proptest::prelude::*;

#[derive(Debug, Clone)]
enum RandomEdit {
    Insert { start: i64, dur: i64 },
    Move { idx: usize, start: i64 },
    Trim { idx: usize, start: i64, dur: i64 },
    Remove { idx: usize },
    Split { idx: usize, at: i64 },
    RippleTrimEnd { idx: usize, boundary: i64 },
    RippleTrimStart { idx: usize, boundary: i64 },
}

fn edit_strategy() -> impl Strategy<Value = RandomEdit> {
    prop_oneof![
        (0i64..5000, 1i64..2000).prop_map(|(start, dur)| RandomEdit::Insert { start, dur }),
        (0usize..8, 0i64..6000).prop_map(|(idx, start)| RandomEdit::Move { idx, start }),
        (0usize..8, 0i64..5000, 1i64..2000).prop_map(|(idx, start, dur)| RandomEdit::Trim {
            idx,
            start,
            dur
        }),
        (0usize..8).prop_map(|idx| RandomEdit::Remove { idx }),
        (0usize..8, 0i64..6000).prop_map(|(idx, at)| RandomEdit::Split { idx, at }),
        (0usize..8, 0i64..6000)
            .prop_map(|(idx, boundary)| RandomEdit::RippleTrimEnd { idx, boundary }),
        (0usize..8, 0i64..6000)
            .prop_map(|(idx, boundary)| RandomEdit::RippleTrimStart { idx, boundary }),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// A random sequence of edit ops — each applied only if the op accepts it —
    /// must leave every sequence satisfying `validate()` (sorted, non-overlapping,
    /// duration > 0). Ops that would violate invariants return `Err` and are
    /// skipped, so the invariant is an unconditional postcondition (11 §3.1).
    #[test]
    fn random_edits_preserve_non_overlap(edits in prop::collection::vec(edit_strategy(), 0..40)) {
        let f = fixture();
        let mut doc = f.doc.clone();

        for edit in edits {
            let p = doc.timeline.as_ref().unwrap();
            let clips = &p.sequences[&f.seq].track(f.vtrack).unwrap().clips;
            let nth_id = |i: usize| clips.get(i).map(|c| c.id);

            let cmd: Option<TimelineCmd> = match edit {
                RandomEdit::Insert { start, dur } => {
                    ops::insert_clip(
                        p, f.seq, f.vtrack,
                        Clip::new(ClipSource::Adjustment, Tick(start), Tick(dur)),
                    ).ok()
                }
                RandomEdit::Move { idx, start } => {
                    nth_id(idx).and_then(|id| ops::move_clip(p, f.seq, f.vtrack, id, Tick(start)).ok())
                }
                RandomEdit::Trim { idx, start, dur } => {
                    nth_id(idx).and_then(|id| ops::trim_clip(
                        p, f.seq, f.vtrack, id,
                        ClipTiming { start: Tick(start), duration: Tick(dur), source_in: Tick(0) },
                    ).ok())
                }
                RandomEdit::Remove { idx } => {
                    nth_id(idx).and_then(|id| ops::remove_clip(p, f.seq, f.vtrack, id).ok())
                }
                RandomEdit::Split { idx, at } => {
                    nth_id(idx).and_then(|id| ops::split_clip(p, f.seq, f.vtrack, id, Tick(at)).ok())
                }
                RandomEdit::RippleTrimEnd { idx, boundary } => {
                    nth_id(idx).and_then(|id| ops::ripple_trim(
                        p, f.seq, f.vtrack, id, ClipEdge::End, Tick(boundary),
                    ).ok())
                }
                RandomEdit::RippleTrimStart { idx, boundary } => {
                    nth_id(idx).and_then(|id| ops::ripple_trim(
                        p, f.seq, f.vtrack, id, ClipEdge::Start, Tick(boundary),
                    ).ok())
                }
            };

            if let Some(cmd) = cmd {
                Command::Timeline(cmd).apply(&mut doc);
                // The invariant must hold after every accepted edit.
                let seq = &doc.timeline.as_ref().unwrap().sequences[&f.seq];
                prop_assert!(seq.validate().is_ok(), "invariant broken: {:?}", seq.validate());
            }
        }
    }
}
