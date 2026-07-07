//! The intent → `timeline/ops.rs` → `CommandHistory` bridge (04 §2.3).
//!
//! This is the SOLE place in the timeline GUI that calls `ops::*` and pushes to
//! history. `interact.rs` decides *what the user intends*; every function here
//! turns that intent into a pure `ops::*` call producing a `TimelineCmd`, then
//! routes it through `CommandHistory`. No GUI code mutates `doc.timeline`
//! directly — the one sanctioned exception is [`set_track_height`] (04 §2.3
//! marks `height_px` a persisted-but-non-undoable UI field).
//!
//! Drag gestures commit ONE command on pointer release (via `execute_discrete`,
//! which never folds), so each gesture is exactly one undo step — the same
//! guarantee the coalesce anchor gives streamed edits (01 §10), reached here by
//! previewing a ghost during the drag and committing from the drag-start
//! snapshot on release. Multi-clip ops (roll/ripple/slide) have no `coalesce`
//! arm, so commit-on-release is the only correct choice for them and is applied
//! uniformly.

use photonic_core::document::Document;
use photonic_core::history::{Command, CommandHistory};
use photonic_core::timeline::{
    ops, ClipTiming, FrameRate, Sequence, SequenceId, Tick, TimelineCmd, Track, TrackId, TrackKind,
    TrackSettings,
};

/// Push one timeline command as a single, non-folding undo step.
fn commit(history: &mut CommandHistory, doc: &mut Document, cmd: TimelineCmd) {
    history.execute_discrete(Command::Timeline(cmd), doc);
}

/// Push several timeline commands as ONE undo step (`Command::Batch`).
fn commit_batch(history: &mut CommandHistory, doc: &mut Document, cmds: Vec<TimelineCmd>) {
    if cmds.is_empty() {
        return;
    }
    let batch = cmds.into_iter().map(Command::Timeline).collect();
    history.execute_discrete(Command::Batch(batch), doc);
}

// ── Project / sequence / track lifecycle ────────────────────────────────────

/// Guarantee a project *and* at least one sequence exist, returning the active
/// sequence id. The first video-mode action creates them lazily and undoably
/// (04 §1.3): `CreateProject` (+ a default 1080p sequence if the project has
/// none) batched into one step.
pub(crate) fn ensure_project_and_sequence(
    doc: &mut Document,
    history: &mut CommandHistory,
    frame_rate: FrameRate,
) -> Option<SequenceId> {
    // Already have an active sequence → nothing to do.
    if let Some(p) = doc.timeline.as_ref() {
        if let Some(active) = p.active_sequence {
            return Some(active);
        }
        // Project exists but no sequence: add one.
        let seq = Sequence::new("Sequence 1", frame_rate, 1920, 1080);
        let id = seq.id;
        commit(history, doc, ops::add_sequence(seq));
        // `AddSequence` makes it active when none was (see commands.rs).
        return doc
            .timeline
            .as_ref()
            .and_then(|p| p.active_sequence)
            .or(Some(id));
    }

    // No project at all: create project + first sequence in one undo step.
    let create = ops::create_project();
    let seq = Sequence::new("Sequence 1", frame_rate, 1920, 1080);
    let id = seq.id;
    commit_batch(history, doc, vec![create, ops::add_sequence(seq)]);
    doc.timeline
        .as_ref()
        .and_then(|p| p.active_sequence)
        .or(Some(id))
}

/// Append a new track of `kind` to the active sequence.
pub(crate) fn add_track(
    doc: &mut Document,
    history: &mut CommandHistory,
    seq: SequenceId,
    kind: TrackKind,
) {
    let Some(p) = doc.timeline.as_ref() else {
        return;
    };
    let n = p
        .sequences
        .get(&seq)
        .map(|s| match kind {
            TrackKind::Video => s.video_tracks.len(),
            TrackKind::Audio => s.audio_tracks.len(),
        })
        .unwrap_or(0);
    let prefix = match kind {
        TrackKind::Video => "V",
        TrackKind::Audio => "A",
    };
    let track = Track::new(kind, format!("{prefix}{}", n + 1));
    if let Ok(cmd) = ops::add_track(p, seq, track, None) {
        commit(history, doc, cmd);
    }
}

pub(crate) fn remove_track(
    doc: &mut Document,
    history: &mut CommandHistory,
    seq: SequenceId,
    track: TrackId,
) {
    let Some(p) = doc.timeline.as_ref() else {
        return;
    };
    if let Ok(cmd) = ops::remove_track(p, seq, track) {
        commit(history, doc, cmd);
    }
}

pub(crate) fn move_track(
    doc: &mut Document,
    history: &mut CommandHistory,
    seq: SequenceId,
    track: TrackId,
    new_index: usize,
) {
    let Some(p) = doc.timeline.as_ref() else {
        return;
    };
    if let Ok(cmds) = ops::reorder_track(p, seq, track, new_index) {
        commit_batch(history, doc, cmds);
    }
}

/// Change one field of a track's settings via `SetTrackProp` (04 §2.6).
fn set_track_settings(
    doc: &mut Document,
    history: &mut CommandHistory,
    seq: SequenceId,
    track: TrackId,
    edit: impl FnOnce(&mut TrackSettings),
) {
    let Some(p) = doc.timeline.as_ref() else {
        return;
    };
    let Some(t) = p.sequences.get(&seq).and_then(|s| s.track(track)) else {
        return;
    };
    let mut new = TrackSettings::of(t);
    edit(&mut new);
    if let Ok(cmd) = ops::set_track_prop(p, seq, track, new) {
        commit(history, doc, cmd);
    }
}

pub(crate) fn rename_track(
    doc: &mut Document,
    history: &mut CommandHistory,
    seq: SequenceId,
    track: TrackId,
    name: String,
) {
    set_track_settings(doc, history, seq, track, |s| s.name = name);
}

pub(crate) fn toggle_enabled(
    doc: &mut Document,
    history: &mut CommandHistory,
    seq: SequenceId,
    track: TrackId,
) {
    set_track_settings(doc, history, seq, track, |s| s.enabled = !s.enabled);
}

pub(crate) fn toggle_locked(
    doc: &mut Document,
    history: &mut CommandHistory,
    seq: SequenceId,
    track: TrackId,
) {
    set_track_settings(doc, history, seq, track, |s| s.locked = !s.locked);
}

/// The one sanctioned direct mutation (04 §2.3): `height_px` is a persisted-but-
/// non-undoable UI field, so a header height-drag writes it in place rather than
/// producing a command.
pub(crate) fn set_track_height(doc: &mut Document, seq: SequenceId, track: TrackId, height: f32) {
    if let Some(p) = doc.timeline.as_mut() {
        if let Some(s) = p.sequences.get_mut(&seq) {
            if let Some(t) = s.track_mut(track) {
                t.height_px = height.clamp(28.0, 240.0);
            }
        }
    }
}

// ── Clip edits ──────────────────────────────────────────────────────────────

pub(crate) fn move_clip(
    doc: &mut Document,
    history: &mut CommandHistory,
    seq: SequenceId,
    track: TrackId,
    clip: photonic_core::timeline::ClipId,
    new_start: Tick,
) {
    let Some(p) = doc.timeline.as_ref() else {
        return;
    };
    if let Ok(cmd) = ops::move_clip(p, seq, track, clip, new_start) {
        commit(history, doc, cmd);
    }
}

/// Move a clip to a different track (compose remove + insert as one undo step —
/// no single cross-track op exists in the committed core).
pub(crate) fn move_clip_cross_track(
    doc: &mut Document,
    history: &mut CommandHistory,
    seq: SequenceId,
    from_track: TrackId,
    to_track: TrackId,
    clip: photonic_core::timeline::ClipId,
    new_start: Tick,
) {
    let Some(p) = doc.timeline.as_ref() else {
        return;
    };
    let Some(existing) = p
        .sequences
        .get(&seq)
        .and_then(|s| s.track(from_track))
        .and_then(|t| t.clips.iter().find(|c| c.id == clip).cloned())
    else {
        return;
    };
    let mut moved = existing.clone();
    moved.start = if new_start.0 < 0 {
        Tick::ZERO
    } else {
        new_start
    };
    // Reject if it would overlap on the destination track.
    if let Some(dst) = p.sequences.get(&seq).and_then(|s| s.track(to_track)) {
        let end = moved.start + moved.duration;
        if dst
            .clips
            .iter()
            .any(|c| c.start < end && moved.start < c.end())
        {
            return;
        }
    }
    let Ok(remove) = ops::remove_clip(p, seq, from_track, clip) else {
        return;
    };
    let Ok(insert) = ops::insert_clip(p, seq, to_track, moved) else {
        return;
    };
    commit_batch(history, doc, vec![remove, insert]);
}

pub(crate) fn trim(
    doc: &mut Document,
    history: &mut CommandHistory,
    seq: SequenceId,
    track: TrackId,
    clip: photonic_core::timeline::ClipId,
    new: ClipTiming,
) {
    let Some(p) = doc.timeline.as_ref() else {
        return;
    };
    if let Ok(cmd) = ops::trim_clip(p, seq, track, clip, new) {
        commit(history, doc, cmd);
    }
}

/// Ripple-trim (Shift+edge, 04 §2.4): trim the clip and shift every downstream
/// clip on the same track by the same delta, as one undo step. The core has no
/// dedicated ripple-trim op, so this composes `trim_clip` with a
/// locally-derived `RippleEdit` (a command, not a direct mutation).
pub(crate) fn ripple_trim(
    doc: &mut Document,
    history: &mut CommandHistory,
    seq: SequenceId,
    track: TrackId,
    clip: photonic_core::timeline::ClipId,
    new: ClipTiming,
) {
    let Some(p) = doc.timeline.as_ref() else {
        return;
    };
    let Ok(trim_cmd) = ops::trim_clip(p, seq, track, clip, new) else {
        return;
    };
    // Delta by which the clip's end moved: downstream clips shift by the same.
    let Some(t) = p.sequences.get(&seq).and_then(|s| s.track(track)) else {
        return;
    };
    let Some(c) = t.clips.iter().find(|c| c.id == clip) else {
        return;
    };
    let old_end = c.end();
    let new_end = new.start + new.duration;
    let delta = new_end - old_end;
    let mut changes = Vec::new();
    if delta.0 != 0 {
        for other in &t.clips {
            if other.id != clip && other.start >= old_end {
                let old = ClipTiming::of(other);
                let shifted = ClipTiming {
                    start: other.start + delta,
                    ..old
                };
                if shifted.start.0 >= 0 {
                    changes.push((other.id, old, shifted));
                }
            }
        }
    }
    if changes.is_empty() {
        commit(history, doc, trim_cmd);
    } else {
        commit_batch(
            history,
            doc,
            vec![
                trim_cmd,
                TimelineCmd::RippleEdit {
                    seq,
                    track,
                    changes,
                },
            ],
        );
    }
}

pub(crate) fn roll(
    doc: &mut Document,
    history: &mut CommandHistory,
    seq: SequenceId,
    track: TrackId,
    left: photonic_core::timeline::ClipId,
    right: photonic_core::timeline::ClipId,
    delta: Tick,
) {
    let Some(p) = doc.timeline.as_ref() else {
        return;
    };
    if let Ok(cmd) = ops::roll_edit(p, seq, track, left, right, delta) {
        commit(history, doc, cmd);
    }
}

pub(crate) fn slip(
    doc: &mut Document,
    history: &mut CommandHistory,
    seq: SequenceId,
    track: TrackId,
    clip: photonic_core::timeline::ClipId,
    new_source_in: Tick,
) {
    let Some(p) = doc.timeline.as_ref() else {
        return;
    };
    if let Ok(cmd) = ops::slip_clip(p, seq, track, clip, new_source_in) {
        commit(history, doc, cmd);
    }
}

pub(crate) fn slide(
    doc: &mut Document,
    history: &mut CommandHistory,
    seq: SequenceId,
    track: TrackId,
    clip: photonic_core::timeline::ClipId,
    delta: Tick,
) {
    let Some(p) = doc.timeline.as_ref() else {
        return;
    };
    if let Ok(cmd) = ops::slide_clip(p, seq, track, clip, delta) {
        commit(history, doc, cmd);
    }
}

pub(crate) fn split(
    doc: &mut Document,
    history: &mut CommandHistory,
    seq: SequenceId,
    track: TrackId,
    clip: photonic_core::timeline::ClipId,
    at: Tick,
) {
    let Some(p) = doc.timeline.as_ref() else {
        return;
    };
    if let Ok(cmd) = ops::split_clip(p, seq, track, clip, at) {
        commit(history, doc, cmd);
    }
}

pub(crate) fn remove_clip(
    doc: &mut Document,
    history: &mut CommandHistory,
    seq: SequenceId,
    track: TrackId,
    clip: photonic_core::timeline::ClipId,
) {
    let Some(p) = doc.timeline.as_ref() else {
        return;
    };
    if let Ok(cmd) = ops::remove_clip(p, seq, track, clip) {
        commit(history, doc, cmd);
    }
}

pub(crate) fn ripple_delete(
    doc: &mut Document,
    history: &mut CommandHistory,
    seq: SequenceId,
    track: TrackId,
    clip: photonic_core::timeline::ClipId,
) {
    let Some(p) = doc.timeline.as_ref() else {
        return;
    };
    if let Ok(cmds) = ops::ripple_delete(p, seq, track, clip) {
        commit_batch(history, doc, cmds);
    }
}

/// Enable/disable a clip via the universal `SetClipProp` op (04 §2.6).
pub(crate) fn set_clip_enabled(
    doc: &mut Document,
    history: &mut CommandHistory,
    seq: SequenceId,
    track: TrackId,
    clip: photonic_core::timeline::ClipId,
    enabled: bool,
) {
    let Some(p) = doc.timeline.as_ref() else {
        return;
    };
    let Some(existing) = p
        .sequences
        .get(&seq)
        .and_then(|s| s.track(track))
        .and_then(|t| t.clips.iter().find(|c| c.id == clip).cloned())
    else {
        return;
    };
    let mut new = existing;
    new.enabled = enabled;
    if let Ok(cmd) = ops::set_clip_prop(p, seq, track, new) {
        commit(history, doc, cmd);
    }
}
