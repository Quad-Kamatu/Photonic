//! Pure timeline edit ops (01 §10): `fn …(…) -> Result<TimelineCmd, EditError>`.
//!
//! Every op reads the current project to capture the old state and construct the
//! command that performs the edit — it never mutates. The GUI and MCP both call
//! these (CAP-019 parity), then hand the command to the history for apply/undo.
//! Invariants (non-overlap, `duration > 0`, sorted, cycle-freedom, the
//! composition-on-`Adjustment` rejection) are enforced here, before any command
//! exists — an invalid edit returns `Err` and produces no document change.
//!
//! A handful of ops perform two arena mutations atomically (create/paste a
//! composition, set the project graph); those return `Vec<TimelineCmd>` for the
//! caller to wrap in a single `Command::Batch`, mirroring the existing
//! `GroupNodes` batching idiom.

use super::anim::{Interp, Keyframe, PropPath, PropertyTrack};
use super::clip::{Clip, ClipEffect, ClipSource, ClipTransform, LinkGroupId};
use super::commands::{
    AnimTarget, AudioCmd, ClipTiming, FormatOp, FxOwner, TimelineCmd, TrackSettings,
};
use super::grade::Grade;
use super::graph::NodeGraph;
use super::ids::*;
use super::media::MediaBin;
use super::sequence::{Marker, Sequence, SequenceFormat, TimelineProject, Track, TrackKind};
use super::time::Tick;
use std::path::PathBuf;

/// A rejected timeline edit — no command is produced and the document is
/// unchanged.
#[derive(Debug, Clone, PartialEq)]
pub enum EditError {
    NoProject,
    NoSequence(SequenceId),
    NoTrack(TrackId),
    NoClip(ClipId),
    NoAsset(AssetId),
    NoGraph(GraphId),
    NoGradeOp(GradeOpId),
    /// The requested placement/trim would overlap another clip on the track.
    Overlap,
    /// A clip duration would be `<= 0`.
    NonPositiveDuration,
    /// A split point was not strictly inside the clip.
    InvalidSplit,
    /// An index was out of range for the target vector.
    IndexOutOfRange,
    /// A graph edge would create a cycle (01 §8).
    WouldCreateCycle,
    /// A composition was requested on a `ClipSource::Adjustment` clip (07 §6.6).
    CompositionOnAdjustment,
    /// A ducking/sidechain wiring would create a cycle (09 §6.3).
    SidechainCycle,
    /// A nested-sequence insertion would create a sequence cycle (CAP-005).
    SequenceCycle,
}

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for EditError {}

// ── Read helpers ────────────────────────────────────────────────────────────

fn seq(p: &TimelineProject, id: SequenceId) -> Result<&Sequence, EditError> {
    p.sequences.get(&id).ok_or(EditError::NoSequence(id))
}

fn track(s: &Sequence, id: TrackId) -> Result<&Track, EditError> {
    s.track(id).ok_or(EditError::NoTrack(id))
}

fn clip(t: &Track, id: ClipId) -> Result<&Clip, EditError> {
    t.clips
        .iter()
        .find(|c| c.id == id)
        .ok_or(EditError::NoClip(id))
}

/// True if `[start, end)` would overlap any clip on `t` other than `ignore`.
fn overlaps_other(t: &Track, start: Tick, end: Tick, ignore: Option<ClipId>) -> bool {
    t.clips
        .iter()
        .filter(|c| Some(c.id) != ignore)
        .any(|c| c.start < end && start < c.end())
}

// ── Project / media ─────────────────────────────────────────────────────────

/// Create the timeline project (first video-mode action, 01 §2).
pub fn create_project() -> TimelineCmd {
    TimelineCmd::CreateProject {
        project: Box::new(TimelineProject::new()),
    }
}

pub fn add_asset(asset: super::media::MediaAsset) -> TimelineCmd {
    TimelineCmd::AddAsset {
        asset: Box::new(asset),
    }
}

pub fn remove_asset(p: &TimelineProject, asset: AssetId) -> Result<TimelineCmd, EditError> {
    let a = p
        .media
        .assets
        .get(&asset)
        .ok_or(EditError::NoAsset(asset))?;
    Ok(TimelineCmd::RemoveAsset {
        asset: Box::new(a.clone()),
    })
}

pub fn relink_asset(
    p: &TimelineProject,
    asset: AssetId,
    new_path: PathBuf,
) -> Result<TimelineCmd, EditError> {
    let a = p
        .media
        .assets
        .get(&asset)
        .ok_or(EditError::NoAsset(asset))?;
    let old_path = match &a.source {
        super::media::AssetSource::File { path, .. } => path.clone(),
        _ => PathBuf::new(),
    };
    Ok(TimelineCmd::RelinkAsset {
        asset,
        old_path,
        new_path,
    })
}

// ── Sequences / formats / tracks ────────────────────────────────────────────

pub fn add_sequence(s: Sequence) -> TimelineCmd {
    TimelineCmd::AddSequence {
        sequence: Box::new(s),
    }
}

pub fn remove_sequence(p: &TimelineProject, id: SequenceId) -> Result<TimelineCmd, EditError> {
    let s = seq(p, id)?;
    let order_index = p.sequence_order.iter().position(|x| *x == id).unwrap_or(0);
    Ok(TimelineCmd::RemoveSequence {
        sequence: Box::new(s.clone()),
        order_index,
        was_active: p.active_sequence == Some(id),
    })
}

pub fn set_active_sequence(p: &TimelineProject, new: Option<SequenceId>) -> TimelineCmd {
    TimelineCmd::SetActiveSequence {
        old: p.active_sequence,
        new,
    }
}

pub fn set_active_format(
    p: &TimelineProject,
    id: SequenceId,
    new: usize,
) -> Result<TimelineCmd, EditError> {
    let s = seq(p, id)?;
    if new >= s.formats.len() {
        return Err(EditError::IndexOutOfRange);
    }
    Ok(TimelineCmd::SetActiveFormat {
        seq: id,
        old: s.active_format,
        new,
    })
}

pub fn set_sequence_format(id: SequenceId, op: FormatOp) -> TimelineCmd {
    TimelineCmd::SetSequenceFormat { seq: id, op }
}

pub fn add_format(id: SequenceId, format: SequenceFormat) -> TimelineCmd {
    TimelineCmd::SetSequenceFormat {
        seq: id,
        op: FormatOp::Add { format },
    }
}

pub fn add_track(
    p: &TimelineProject,
    id: SequenceId,
    t: Track,
    index: Option<usize>,
) -> Result<TimelineCmd, EditError> {
    let s = seq(p, id)?;
    let kind = t.kind;
    let len = match kind {
        TrackKind::Video => s.video_tracks.len(),
        TrackKind::Audio => s.audio_tracks.len(),
    };
    Ok(TimelineCmd::AddTrack {
        seq: id,
        kind,
        index: index.unwrap_or(len).min(len),
        track: Box::new(t),
    })
}

pub fn remove_track(
    p: &TimelineProject,
    id: SequenceId,
    track_id: TrackId,
) -> Result<TimelineCmd, EditError> {
    let s = seq(p, id)?;
    for (kind, v) in [
        (TrackKind::Video, &s.video_tracks),
        (TrackKind::Audio, &s.audio_tracks),
    ] {
        if let Some(index) = v.iter().position(|t| t.id == track_id) {
            return Ok(TimelineCmd::RemoveTrack {
                seq: id,
                kind,
                index,
                track: Box::new(v[index].clone()),
            });
        }
    }
    Err(EditError::NoTrack(track_id))
}

/// Reorder a track within its lane (remove+add batched — the caller wraps the
/// two commands in a single `Command::Batch`).
pub fn reorder_track(
    p: &TimelineProject,
    id: SequenceId,
    track_id: TrackId,
    new_index: usize,
) -> Result<Vec<TimelineCmd>, EditError> {
    let remove = remove_track(p, id, track_id)?;
    let (kind, t) = match &remove {
        TimelineCmd::RemoveTrack { kind, track, .. } => (*kind, (**track).clone()),
        _ => unreachable!(),
    };
    let add = TimelineCmd::AddTrack {
        seq: id,
        kind,
        index: new_index,
        track: Box::new(t),
    };
    Ok(vec![remove, add])
}

pub fn set_track_prop(
    p: &TimelineProject,
    id: SequenceId,
    track_id: TrackId,
    new: TrackSettings,
) -> Result<TimelineCmd, EditError> {
    let s = seq(p, id)?;
    let t = track(s, track_id)?;
    Ok(TimelineCmd::SetTrackProp {
        seq: id,
        track: track_id,
        old: Box::new(TrackSettings::of(t)),
        new: Box::new(new),
    })
}

// ── Clips ───────────────────────────────────────────────────────────────────

pub fn insert_clip(
    p: &TimelineProject,
    id: SequenceId,
    track_id: TrackId,
    c: Clip,
) -> Result<TimelineCmd, EditError> {
    let s = seq(p, id)?;
    let t = track(s, track_id)?;
    if c.duration.0 <= 0 {
        return Err(EditError::NonPositiveDuration);
    }
    if overlaps_other(t, c.start, c.end(), None) {
        return Err(EditError::Overlap);
    }
    if let ClipSource::NestedSequence { sequence } = &c.source {
        if *sequence == id || nests_into(p, *sequence, id) {
            return Err(EditError::SequenceCycle);
        }
    }
    Ok(TimelineCmd::InsertClip {
        seq: id,
        track: track_id,
        clip: Box::new(c),
    })
}

/// Does sequence `outer` (transitively) contain a clip nesting `target`?
fn nests_into(p: &TimelineProject, outer: SequenceId, target: SequenceId) -> bool {
    let Some(s) = p.sequences.get(&outer) else {
        return false;
    };
    for t in s.video_tracks.iter().chain(s.audio_tracks.iter()) {
        for c in &t.clips {
            if let ClipSource::NestedSequence { sequence } = &c.source {
                if *sequence == target || nests_into(p, *sequence, target) {
                    return true;
                }
            }
        }
    }
    false
}

pub fn remove_clip(
    p: &TimelineProject,
    id: SequenceId,
    track_id: TrackId,
    clip_id: ClipId,
) -> Result<TimelineCmd, EditError> {
    let s = seq(p, id)?;
    let t = track(s, track_id)?;
    let c = clip(t, clip_id)?;
    Ok(TimelineCmd::RemoveClip {
        seq: id,
        track: track_id,
        clip: Box::new(c.clone()),
    })
}

/// Move a clip within its track. Signature preserved for existing callers;
/// cross-track moves use [`move_clip_to_track`].
pub fn move_clip(
    p: &TimelineProject,
    id: SequenceId,
    track_id: TrackId,
    clip_id: ClipId,
    new_start: Tick,
) -> Result<TimelineCmd, EditError> {
    move_clip_to_track(p, id, track_id, clip_id, new_start, None)
}

/// Move a clip, optionally to a different track (`new_track = Some(dest)`).
/// The destination must be the same [`TrackKind`] and have room at `new_start`
/// (non-overlap enforced here). Inverse returns the clip to its original
/// track + position (lossless).
pub fn move_clip_to_track(
    p: &TimelineProject,
    id: SequenceId,
    track_id: TrackId,
    clip_id: ClipId,
    new_start: Tick,
    new_track: Option<TrackId>,
) -> Result<TimelineCmd, EditError> {
    if new_start.0 < 0 {
        return Err(EditError::Overlap);
    }
    let s = seq(p, id)?;
    let src = track(s, track_id)?;
    let c = clip(src, clip_id)?;

    // Normalize: a `Some(same)` destination is really a same-track move.
    let dest_id = match new_track {
        Some(t) if t != track_id => Some(t),
        _ => None,
    };

    match dest_id {
        None => {
            if overlaps_other(src, new_start, new_start + c.duration, Some(clip_id)) {
                return Err(EditError::Overlap);
            }
        }
        Some(dest) => {
            let dst = track(s, dest)?;
            if dst.kind != src.kind {
                return Err(EditError::NoTrack(dest));
            }
            // On the destination the clip is new — nothing to ignore.
            if overlaps_other(dst, new_start, new_start + c.duration, None) {
                return Err(EditError::Overlap);
            }
        }
    }

    Ok(TimelineCmd::MoveClip {
        seq: id,
        track: track_id,
        clip: clip_id,
        old_start: c.start,
        new_start,
        new_track: dest_id,
    })
}

pub fn trim_clip(
    p: &TimelineProject,
    id: SequenceId,
    track_id: TrackId,
    clip_id: ClipId,
    new: ClipTiming,
) -> Result<TimelineCmd, EditError> {
    if new.duration.0 <= 0 {
        return Err(EditError::NonPositiveDuration);
    }
    let s = seq(p, id)?;
    let t = track(s, track_id)?;
    let c = clip(t, clip_id)?;
    if overlaps_other(t, new.start, new.start + new.duration, Some(clip_id)) {
        return Err(EditError::Overlap);
    }
    Ok(TimelineCmd::TrimClip {
        seq: id,
        track: track_id,
        clip: clip_id,
        old: ClipTiming::of(c),
        new,
    })
}

pub fn split_clip(
    p: &TimelineProject,
    id: SequenceId,
    track_id: TrackId,
    clip_id: ClipId,
    at: Tick,
) -> Result<TimelineCmd, EditError> {
    let s = seq(p, id)?;
    let t = track(s, track_id)?;
    let c = clip(t, clip_id)?;
    if at <= c.start || at >= c.end() {
        return Err(EditError::InvalidSplit);
    }
    Ok(TimelineCmd::SplitClip {
        seq: id,
        track: track_id,
        clip: clip_id,
        at,
        new_clip_id: ClipId::new(),
    })
}

/// Slip a clip's source in/out without moving it on the timeline.
pub fn slip_clip(
    p: &TimelineProject,
    id: SequenceId,
    track_id: TrackId,
    clip_id: ClipId,
    new_source_in: Tick,
) -> Result<TimelineCmd, EditError> {
    let s = seq(p, id)?;
    let t = track(s, track_id)?;
    let c = clip(t, clip_id)?;
    Ok(TimelineCmd::SlipClip {
        seq: id,
        track: track_id,
        clip: clip_id,
        old_source_in: c.source_in,
        new_source_in,
    })
}

/// Ripple-delete a clip: remove it and shift every later clip on the track left
/// by its duration. Returns `[RemoveClip, RippleEdit]` for the caller to batch.
pub fn ripple_delete(
    p: &TimelineProject,
    id: SequenceId,
    track_id: TrackId,
    clip_id: ClipId,
) -> Result<Vec<TimelineCmd>, EditError> {
    let s = seq(p, id)?;
    let t = track(s, track_id)?;
    let c = clip(t, clip_id)?;
    let shift = c.duration;
    let start = c.start;
    let mut changes = Vec::new();
    for other in &t.clips {
        if other.start >= start && other.id != clip_id {
            let old = ClipTiming::of(other);
            let new = ClipTiming {
                start: other.start - shift,
                ..old
            };
            changes.push((other.id, old, new));
        }
    }
    Ok(vec![
        TimelineCmd::RemoveClip {
            seq: id,
            track: track_id,
            clip: Box::new(c.clone()),
        },
        TimelineCmd::RippleEdit {
            seq: id,
            track: track_id,
            changes,
        },
    ])
}

/// Which edge of a clip a trim targets.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ClipEdge {
    Start,
    End,
}

/// Ripple-trim one edge of a clip to `new_boundary` (a timeline tick) and shift
/// every later clip on the same track by the resulting delta, closing the gap
/// (04 §2.4 "Shift + edge" ripple; distinct from [`ripple_delete`]).
///
/// - **End edge**: the clip's out-point moves to `new_boundary` (duration
///   `= new_boundary - start`); later clips shift by `new_boundary - old_end`.
/// - **Start edge**: the clip's in-point moves; the clip keeps its timeline
///   `start`, its `source_in` advances by the delta (`speed`-scaled) and its
///   duration shrinks/grows by the delta; later clips shift by `-delta` so the
///   left gap closes. `new_boundary` is the new in-point position on the timeline.
///
/// Emitted as a single `RippleEdit` command (one undo step) whose changes list
/// carries the trimmed clip plus every shifted clip — invariant-safe by
/// construction and inverted by the existing `RippleEdit` logic.
pub fn ripple_trim(
    p: &TimelineProject,
    id: SequenceId,
    track_id: TrackId,
    clip_id: ClipId,
    edge: ClipEdge,
    new_boundary: Tick,
) -> Result<TimelineCmd, EditError> {
    let s = seq(p, id)?;
    let t = track(s, track_id)?;
    let c = clip(t, clip_id)?;
    let old_start = c.start;
    let old_end = c.end();

    let (trimmed, shift) = match edge {
        ClipEdge::End => {
            if new_boundary <= old_start {
                return Err(EditError::NonPositiveDuration);
            }
            let new_dur = new_boundary - old_start;
            let delta = new_boundary - old_end;
            (
                ClipTiming {
                    start: old_start,
                    duration: new_dur,
                    source_in: c.source_in,
                },
                delta,
            )
        }
        ClipEdge::Start => {
            let delta = new_boundary - old_start;
            let new_dur = c.duration - delta;
            if new_dur.0 <= 0 {
                return Err(EditError::NonPositiveDuration);
            }
            let new_source_in = c.source_in + c.speed.source_delta(delta);
            if new_source_in.0 < 0 {
                // Cannot pull the in-point before the start of the source media.
                return Err(EditError::InvalidSplit);
            }
            (
                ClipTiming {
                    start: old_start,
                    duration: new_dur,
                    source_in: new_source_in,
                },
                // Later clips move opposite the in-point drag to close the gap.
                Tick(-delta.0),
            )
        }
    };

    let mut changes = vec![(clip_id, ClipTiming::of(c), trimmed)];
    for other in &t.clips {
        if other.id != clip_id && other.start >= old_end {
            let old = ClipTiming::of(other);
            changes.push((
                other.id,
                old,
                ClipTiming {
                    start: other.start + shift,
                    ..old
                },
            ));
        }
    }

    Ok(TimelineCmd::RippleEdit {
        seq: id,
        track: track_id,
        changes,
    })
}

/// Roll the shared edit point between two adjacent clips.
pub fn roll_edit(
    p: &TimelineProject,
    id: SequenceId,
    track_id: TrackId,
    left: ClipId,
    right: ClipId,
    delta: Tick,
) -> Result<TimelineCmd, EditError> {
    let s = seq(p, id)?;
    let t = track(s, track_id)?;
    let l = clip(t, left)?;
    let r = clip(t, right)?;
    let l_old = ClipTiming::of(l);
    let r_old = ClipTiming::of(r);
    let l_new = ClipTiming {
        duration: l.duration + delta,
        ..l_old
    };
    let r_new = ClipTiming {
        start: r.start + delta,
        duration: r.duration - delta,
        source_in: r.source_in + delta,
    };
    if l_new.duration.0 <= 0 || r_new.duration.0 <= 0 {
        return Err(EditError::NonPositiveDuration);
    }
    Ok(TimelineCmd::RollEdit {
        seq: id,
        track: track_id,
        changes: vec![(left, l_old, l_new), (right, r_old, r_new)],
    })
}

/// Slide a clip over its neighbors, keeping total span.
pub fn slide_clip(
    p: &TimelineProject,
    id: SequenceId,
    track_id: TrackId,
    clip_id: ClipId,
    delta: Tick,
) -> Result<TimelineCmd, EditError> {
    let s = seq(p, id)?;
    let t = track(s, track_id)?;
    let pos = t
        .clips
        .iter()
        .position(|c| c.id == clip_id)
        .ok_or(EditError::NoClip(clip_id))?;
    let cur = &t.clips[pos];
    let mut changes = vec![(
        clip_id,
        ClipTiming::of(cur),
        ClipTiming {
            start: cur.start + delta,
            ..ClipTiming::of(cur)
        },
    )];
    if pos > 0 {
        let prev = &t.clips[pos - 1];
        let new = ClipTiming {
            duration: prev.duration + delta,
            ..ClipTiming::of(prev)
        };
        if new.duration.0 <= 0 {
            return Err(EditError::NonPositiveDuration);
        }
        changes.push((prev.id, ClipTiming::of(prev), new));
    }
    if pos + 1 < t.clips.len() {
        let next = &t.clips[pos + 1];
        let new = ClipTiming {
            start: next.start + delta,
            duration: next.duration - delta,
            source_in: next.source_in + delta,
        };
        if new.duration.0 <= 0 {
            return Err(EditError::NonPositiveDuration);
        }
        changes.push((next.id, ClipTiming::of(next), new));
    }
    Ok(TimelineCmd::SlideClip {
        seq: id,
        track: track_id,
        changes,
    })
}

/// Universal clip property change (mirrors `UpdateNode`).
pub fn set_clip_prop(
    p: &TimelineProject,
    id: SequenceId,
    track_id: TrackId,
    new: Clip,
) -> Result<TimelineCmd, EditError> {
    let s = seq(p, id)?;
    let t = track(s, track_id)?;
    let old = clip(t, new.id)?.clone();
    if new.duration.0 <= 0 {
        return Err(EditError::NonPositiveDuration);
    }
    if overlaps_other(t, new.start, new.end(), Some(new.id)) {
        return Err(EditError::Overlap);
    }
    Ok(TimelineCmd::SetClipProp {
        seq: id,
        track: track_id,
        old: Box::new(old),
        new: Box::new(new),
    })
}

// ── Color labels & linking (14 §M-1/M-2, gaps #7/#8's data half) ───────────

/// Set (or clear, with `None`) a clip's organizational color label. Reuses
/// [`set_clip_prop`] — a whole-clip diff — since a label change never
/// affects timing/overlap; `EditError::Overlap`/`NonPositiveDuration` from
/// that path can't actually trigger here.
pub fn set_clip_color_label(
    p: &TimelineProject,
    id: SequenceId,
    track_id: TrackId,
    clip_id: ClipId,
    label: Option<u8>,
) -> Result<TimelineCmd, EditError> {
    let s = seq(p, id)?;
    let t = track(s, track_id)?;
    let mut new = clip(t, clip_id)?.clone();
    new.color_label = label;
    set_clip_prop(p, id, track_id, new)
}

/// Link two clips (e.g. a split A/V pair) into the same link group so a
/// future move can carry them together (14 §M-2; the GUI drag-together
/// wiring is a later story — this just establishes the group). If either
/// clip already belongs to a group, that group is reused for both; otherwise
/// a fresh [`LinkGroupId`] is minted. Returns `[SetClipProp, SetClipProp]`
/// for the caller to wrap in one `Command::Batch` (one undo step).
pub fn link_clips(
    p: &TimelineProject,
    id: SequenceId,
    track_a: TrackId,
    clip_a: ClipId,
    track_b: TrackId,
    clip_b: ClipId,
) -> Result<Vec<TimelineCmd>, EditError> {
    let s = seq(p, id)?;
    let a = clip(track(s, track_a)?, clip_a)?.clone();
    let b = clip(track(s, track_b)?, clip_b)?.clone();
    let group = a
        .link_group
        .or(b.link_group)
        .unwrap_or_else(LinkGroupId::new);

    let mut new_a = a;
    new_a.link_group = Some(group);
    let mut new_b = b;
    new_b.link_group = Some(group);

    Ok(vec![
        set_clip_prop(p, id, track_a, new_a)?,
        set_clip_prop(p, id, track_b, new_b)?,
    ])
}

/// Remove `clip_id` from its link group (a no-op edit — still `Ok` — if it
/// wasn't linked, so callers don't need a special case).
pub fn unlink_clip(
    p: &TimelineProject,
    id: SequenceId,
    track_id: TrackId,
    clip_id: ClipId,
) -> Result<TimelineCmd, EditError> {
    let s = seq(p, id)?;
    let t = track(s, track_id)?;
    let mut new = clip(t, clip_id)?.clone();
    new.link_group = None;
    set_clip_prop(p, id, track_id, new)
}

/// All clip ids across the project sharing `group` (14 §M-2 helper — e.g. to
/// resolve an A/V pair to move/select as a unit). Pure read; empty when the
/// group has no members.
pub fn clips_in_link_group(p: &TimelineProject, group: LinkGroupId) -> Vec<ClipId> {
    p.sequences
        .values()
        .flat_map(|s| s.video_tracks.iter().chain(s.audio_tracks.iter()))
        .flat_map(|t| t.clips.iter())
        .filter(|c| c.link_group == Some(group))
        .map(|c| c.id)
        .collect()
}

// ── Auto-reframe (14 §9/CAP-012) ────────────────────────────────────────────

/// Compute a center-fill ("cover") reframe [`ClipTransform`] for retargeting
/// a clip authored against `content` onto `target`: scale isotropically (so
/// the aspect ratio doesn't distort) by the *larger* of the two per-axis
/// ratios, so `content`'s box fully covers `target`'s box, then leave
/// position/anchor/rotation at their centered defaults — mirrors an "auto
/// reframe: center-fill" preset in reference NLEs. `x`/`y`/`anchor_x`/
/// `anchor_y`/`rotation` are `0.0` and `opacity` is `1.0`
/// ([`ClipTransform::default`]); only `scale_x`/`scale_y` are set.
///
/// **Content-box assumption**: `Clip` doesn't track a source's intrinsic
/// pixel size (asset probing is out of this story's scope). The documented,
/// tested convention: `content` is the sequence's format at index 0 — the
/// sequence's original/native format, which is exactly what a clip's
/// un-reframed (identity-scale) `transform` already fills at authoring time,
/// so "the clip's transform baseline" and "the sequence's base format dims"
/// name the same box. `target` is the format being reframed into
/// (`sequence.formats[format_index]`); the result is stored via
/// [`set_clip_reframe`].
pub fn fit_clip_to_format(content: &SequenceFormat, target: &SequenceFormat) -> ClipTransform {
    let content_w = content.width.max(1) as f64;
    let content_h = content.height.max(1) as f64;
    let target_w = target.width.max(1) as f64;
    let target_h = target.height.max(1) as f64;
    // Cover fit: the larger of the two per-axis ratios so both target
    // dimensions are fully covered (may crop content; never letterboxes).
    let scale = (target_w / content_w).max(target_h / content_h);
    ClipTransform {
        scale_x: scale,
        scale_y: scale,
        ..ClipTransform::default()
    }
}

/// Set (or clear, with `transform = None`) a clip's per-`SequenceFormat`
/// static reframe override (CAP-012, `Clip.reframe[format_index]`). Mirrors
/// the existing GUI (`app/reframe.rs::commit_reframe`) and MCP
/// (`set_clip_prop`'s `reframe` arg) `reframe.insert`/`remove` pattern as a
/// first-class, independently testable op.
pub fn set_clip_reframe(
    p: &TimelineProject,
    id: SequenceId,
    track_id: TrackId,
    clip_id: ClipId,
    format_index: usize,
    transform: Option<ClipTransform>,
) -> Result<TimelineCmd, EditError> {
    let s = seq(p, id)?;
    let t = track(s, track_id)?;
    let mut new = clip(t, clip_id)?.clone();
    match transform {
        Some(xf) => {
            new.reframe.insert(format_index, xf);
        }
        None => {
            new.reframe.remove(&format_index);
        }
    }
    set_clip_prop(p, id, track_id, new)
}

// ── Keyframes (generic over any AnimProps target) ───────────────────────────

fn existing_kf(
    p: &TimelineProject,
    target: &AnimTarget,
    path: &PropPath,
    at: Tick,
) -> Option<Keyframe> {
    read_tracks(p, target)?
        .iter()
        .find(|l| &l.property == path)
        .and_then(|l| l.keyframes.iter().find(|k| k.at == at).copied())
}

fn read_tracks<'a>(p: &'a TimelineProject, target: &AnimTarget) -> Option<&'a Vec<PropertyTrack>> {
    let find_clip = |cid: ClipId| -> Option<&Clip> {
        p.sequences
            .values()
            .flat_map(|s| s.video_tracks.iter().chain(s.audio_tracks.iter()))
            .flat_map(|t| t.clips.iter())
            .find(|c| c.id == cid)
    };
    let find_track = |tid: TrackId| -> Option<&Track> {
        p.sequences
            .values()
            .flat_map(|s| s.video_tracks.iter().chain(s.audio_tracks.iter()))
            .find(|t| t.id == tid)
    };
    match target {
        AnimTarget::ClipTransform { clip } => find_clip(*clip).map(|c| &c.transform.tracks),
        AnimTarget::ClipEffect { clip, effect_index } => find_clip(*clip)?
            .effects
            .get(*effect_index)
            .map(|e| &e.params.tracks),
        AnimTarget::GradeOp { clip, op } => find_clip(*clip)?
            .grade
            .as_ref()?
            .ops
            .iter()
            .find(|o| o.id == *op)
            .map(|o| &o.params.tracks),
        AnimTarget::ClipAudio { clip } => {
            find_clip(*clip)?.audio.as_ref().map(|a| &a.params.tracks)
        }
        AnimTarget::TrackAudio { track } => {
            find_track(*track)?.audio.as_ref().map(|a| &a.params.tracks)
        }
        AnimTarget::MasterBus { seq } => {
            p.sequences.get(seq).map(|s| &s.audio_master.params.tracks)
        }
        AnimTarget::AudioFx { owner, index } => match owner {
            FxOwner::Track(t) => find_track(*t)?
                .audio
                .as_ref()?
                .fx_chain
                .get(*index)
                .map(|u| &u.params.tracks),
            FxOwner::Master => {
                let sid = p.active_sequence?;
                p.sequences
                    .get(&sid)?
                    .audio_master
                    .fx_chain
                    .get(*index)
                    .map(|u| &u.params.tracks)
            }
        },
    }
}

pub fn set_keyframe(
    p: &TimelineProject,
    target: AnimTarget,
    path: PropPath,
    kf: Keyframe,
) -> TimelineCmd {
    let old = existing_kf(p, &target, &path, kf.at);
    TimelineCmd::SetKeyframe {
        target,
        path,
        old,
        new: kf,
    }
}

pub fn remove_keyframe(
    p: &TimelineProject,
    target: AnimTarget,
    path: PropPath,
    at: Tick,
) -> Result<TimelineCmd, EditError> {
    let kf = existing_kf(p, &target, &path, at).ok_or(EditError::IndexOutOfRange)?;
    Ok(TimelineCmd::RemoveKeyframe {
        target,
        path,
        keyframe: kf,
    })
}

pub fn set_keyframe_interp(
    p: &TimelineProject,
    target: AnimTarget,
    path: PropPath,
    at: Tick,
    new: Interp,
) -> Result<TimelineCmd, EditError> {
    let kf = existing_kf(p, &target, &path, at).ok_or(EditError::IndexOutOfRange)?;
    Ok(TimelineCmd::SetKeyframeInterp {
        target,
        path,
        at,
        old: kf.interp,
        new,
    })
}

// ── Effects ─────────────────────────────────────────────────────────────────

pub fn add_effect(
    p: &TimelineProject,
    id: SequenceId,
    track_id: TrackId,
    clip_id: ClipId,
    effect: ClipEffect,
    index: Option<usize>,
) -> Result<TimelineCmd, EditError> {
    let s = seq(p, id)?;
    let t = track(s, track_id)?;
    let c = clip(t, clip_id)?;
    let idx = index.unwrap_or(c.effects.len()).min(c.effects.len());
    Ok(TimelineCmd::AddEffect {
        seq: id,
        track: track_id,
        clip: clip_id,
        index: idx,
        effect: Box::new(effect),
    })
}

pub fn remove_effect(
    p: &TimelineProject,
    id: SequenceId,
    track_id: TrackId,
    clip_id: ClipId,
    index: usize,
) -> Result<TimelineCmd, EditError> {
    let s = seq(p, id)?;
    let t = track(s, track_id)?;
    let c = clip(t, clip_id)?;
    let effect = c
        .effects
        .get(index)
        .ok_or(EditError::IndexOutOfRange)?
        .clone();
    Ok(TimelineCmd::RemoveEffect {
        seq: id,
        track: track_id,
        clip: clip_id,
        index,
        effect: Box::new(effect),
    })
}

pub fn reorder_effects(
    p: &TimelineProject,
    id: SequenceId,
    track_id: TrackId,
    clip_id: ClipId,
    new_order: Vec<usize>,
) -> Result<TimelineCmd, EditError> {
    let s = seq(p, id)?;
    let t = track(s, track_id)?;
    let c = clip(t, clip_id)?;
    if new_order.len() != c.effects.len() {
        return Err(EditError::IndexOutOfRange);
    }
    let old_order: Vec<usize> = (0..c.effects.len()).collect();
    Ok(TimelineCmd::ReorderEffects {
        seq: id,
        track: track_id,
        clip: clip_id,
        old_order,
        new_order,
    })
}

pub fn set_grade(
    p: &TimelineProject,
    id: SequenceId,
    track_id: TrackId,
    clip_id: ClipId,
    new: Option<Grade>,
) -> Result<TimelineCmd, EditError> {
    let s = seq(p, id)?;
    let t = track(s, track_id)?;
    let c = clip(t, clip_id)?;
    Ok(TimelineCmd::SetGrade {
        seq: id,
        track: track_id,
        clip: clip_id,
        old: c.grade.clone().map(Box::new),
        new: new.map(Box::new),
    })
}

// ── Compositions (08 §4) ────────────────────────────────────────────────────

/// Create a per-clip composition (`ClipIn → Output`) and point the clip at it.
/// Rejected for `ClipSource::Adjustment` (07 §6.6). Returns
/// `[AddGraph, SetClipComposition]` for the caller to batch.
pub fn create_clip_composition(
    p: &TimelineProject,
    id: SequenceId,
    track_id: TrackId,
    clip_id: ClipId,
) -> Result<Vec<TimelineCmd>, EditError> {
    let s = seq(p, id)?;
    let t = track(s, track_id)?;
    let c = clip(t, clip_id)?;
    if matches!(c.source, ClipSource::Adjustment) {
        return Err(EditError::CompositionOnAdjustment);
    }
    let (graph, _clip_in) = NodeGraph::new_clip_composition(format!("{} comp", c.name));
    let new_ref = graph.id;
    Ok(vec![
        TimelineCmd::AddGraph {
            graph: Box::new(graph),
        },
        TimelineCmd::SetClipComposition {
            seq: id,
            track: track_id,
            clip: clip_id,
            old: c.composition,
            new: Some(new_ref),
        },
    ])
}

/// Detach a clip's composition (revert). The graph stays in the arena (08 §4).
pub fn detach_clip_composition(
    p: &TimelineProject,
    id: SequenceId,
    track_id: TrackId,
    clip_id: ClipId,
) -> Result<TimelineCmd, EditError> {
    let s = seq(p, id)?;
    let t = track(s, track_id)?;
    let c = clip(t, clip_id)?;
    Ok(TimelineCmd::SetClipComposition {
        seq: id,
        track: track_id,
        clip: clip_id,
        old: c.composition,
        new: None,
    })
}

/// Paste a composition, DEEP-CLONING the source graph under a fresh id so the
/// two clips never alias (08 §4). Returns `[AddGraph(clone), SetClipComposition]`.
pub fn paste_clip_composition(
    p: &TimelineProject,
    source_graph: GraphId,
    id: SequenceId,
    track_id: TrackId,
    clip_id: ClipId,
) -> Result<Vec<TimelineCmd>, EditError> {
    let src = p
        .graphs
        .get(&source_graph)
        .ok_or(EditError::NoGraph(source_graph))?;
    let s = seq(p, id)?;
    let t = track(s, track_id)?;
    let c = clip(t, clip_id)?;
    if matches!(c.source, ClipSource::Adjustment) {
        return Err(EditError::CompositionOnAdjustment);
    }
    let clone = src.deep_clone_fresh_ids();
    let new_ref = clone.id;
    Ok(vec![
        TimelineCmd::AddGraph {
            graph: Box::new(clone),
        },
        TimelineCmd::SetClipComposition {
            seq: id,
            track: track_id,
            clip: clip_id,
            old: c.composition,
            new: Some(new_ref),
        },
    ])
}

/// Set the project graph, allocating a fresh empty graph if `graph` is `None`.
pub fn set_project_graph(p: &TimelineProject, graph: Option<NodeGraph>) -> Vec<TimelineCmd> {
    let g = graph.unwrap_or_else(|| NodeGraph::new_project_graph("Project Graph"));
    let new_ref = g.id;
    vec![
        TimelineCmd::AddGraph { graph: Box::new(g) },
        TimelineCmd::SetProjectGraph {
            old: p.project_graph,
            new: Some(new_ref),
        },
    ]
}

// ── Audio ───────────────────────────────────────────────────────────────────

/// Apply the one-click ducking preset (AS-2, 09 §6.3): ensure a sidechained
/// `Compressor` exists on `music_track`, keyed off `voiceover_track`. Rejects a
/// sidechain cycle (09 §6.3).
pub fn apply_ducking_preset(
    p: &TimelineProject,
    music_track: TrackId,
    voiceover_track: TrackId,
) -> Result<TimelineCmd, EditError> {
    if music_track == voiceover_track || sidechain_reaches(p, voiceover_track, music_track) {
        return Err(EditError::SidechainCycle);
    }
    let t = track(seq_of_track(p, music_track)?, music_track)?;
    let old_fx_chain = t
        .audio
        .as_ref()
        .map(|a| a.fx_chain.clone())
        .unwrap_or_default();
    let mut new_fx_chain = old_fx_chain.clone();
    let mut comp = super::audio::AudioFxUnit::new(super::audio::AudioFxKind::Compressor);
    comp.sidechain = Some(voiceover_track);
    if let Some(slot) = new_fx_chain
        .iter_mut()
        .find(|u| u.kind == super::audio::AudioFxKind::Compressor)
    {
        *slot = comp;
    } else {
        new_fx_chain.push(comp);
    }
    Ok(TimelineCmd::AudioEdit(AudioCmd::ApplyDuckingPreset {
        track: music_track,
        sidechain: voiceover_track,
        old_fx_chain,
        new_fx_chain,
    }))
}

/// Does `from`'s compressor-sidechain graph (transitively) reach `to`?
fn sidechain_reaches(p: &TimelineProject, from: TrackId, to: TrackId) -> bool {
    let Some(t) = find_track_ro(p, from) else {
        return false;
    };
    let Some(a) = &t.audio else {
        return false;
    };
    for u in &a.fx_chain {
        if let Some(sc) = u.sidechain {
            if sc == to || sidechain_reaches(p, sc, to) {
                return true;
            }
        }
    }
    false
}

fn find_track_ro(p: &TimelineProject, id: TrackId) -> Option<&Track> {
    p.sequences
        .values()
        .flat_map(|s| s.video_tracks.iter().chain(s.audio_tracks.iter()))
        .find(|t| t.id == id)
}

fn seq_of_track(p: &TimelineProject, id: TrackId) -> Result<&Sequence, EditError> {
    p.sequences
        .values()
        .find(|s| s.track(id).is_some())
        .ok_or(EditError::NoTrack(id))
}

// ── Markers & work range ────────────────────────────────────────────────────

/// Add a marker to a sequence. The caller supplies the fully-built `Marker`
/// (name/color/note/position); `Marker::new` fills a fresh `MarkerId`.
pub fn add_marker(
    p: &TimelineProject,
    id: SequenceId,
    marker: Marker,
) -> Result<TimelineCmd, EditError> {
    seq(p, id)?;
    Ok(TimelineCmd::AddMarker { seq: id, marker })
}

pub fn remove_marker(
    p: &TimelineProject,
    id: SequenceId,
    marker_id: MarkerId,
) -> Result<TimelineCmd, EditError> {
    let s = seq(p, id)?;
    let marker = s
        .markers
        .iter()
        .find(|m| m.id == marker_id)
        .ok_or(EditError::IndexOutOfRange)?
        .clone();
    Ok(TimelineCmd::RemoveMarker { seq: id, marker })
}

/// Edit a marker's fields (name/color/note/position). `new.id` identifies the
/// marker; the op captures the old state for a self-contained inverse.
pub fn set_marker(
    p: &TimelineProject,
    id: SequenceId,
    new: Marker,
) -> Result<TimelineCmd, EditError> {
    let s = seq(p, id)?;
    let old = s
        .markers
        .iter()
        .find(|m| m.id == new.id)
        .ok_or(EditError::IndexOutOfRange)?
        .clone();
    Ok(TimelineCmd::SetMarker {
        seq: id,
        id: new.id,
        old,
        new,
    })
}

/// Set (or clear, with `None`) a sequence's preview/export work range.
pub fn set_work_range(
    p: &TimelineProject,
    id: SequenceId,
    new: Option<(Tick, Tick)>,
) -> Result<TimelineCmd, EditError> {
    let s = seq(p, id)?;
    Ok(TimelineCmd::SetWorkRange {
        seq: id,
        old: s.work_range,
        new,
    })
}

// ── Media bins ──────────────────────────────────────────────────────────────

/// Create a media bin (folder), optionally nested under `parent`.
pub fn create_bin(name: impl Into<String>, parent: Option<BinId>) -> TimelineCmd {
    TimelineCmd::AddBin {
        bin: MediaBin::new(name, parent),
    }
}

/// Remove a media bin. Assets/child bins referencing it are left untouched (a
/// dangling ref reads as unfiled); re-adding restores the bin verbatim.
pub fn remove_bin(p: &TimelineProject, bin_id: BinId) -> Result<TimelineCmd, EditError> {
    let bin = p
        .media
        .bins
        .iter()
        .find(|b| b.id == bin_id)
        .ok_or(EditError::IndexOutOfRange)?
        .clone();
    Ok(TimelineCmd::RemoveBin { bin })
}

/// Move an asset into `new_bin` (or to the pool root with `None`).
pub fn assign_asset_bin(
    p: &TimelineProject,
    asset: AssetId,
    new_bin: Option<BinId>,
) -> Result<TimelineCmd, EditError> {
    let a = p
        .media
        .assets
        .get(&asset)
        .ok_or(EditError::NoAsset(asset))?;
    // A target bin, if given, must exist.
    if let Some(b) = new_bin {
        if !p.media.bins.iter().any(|bin| bin.id == b) {
            return Err(EditError::IndexOutOfRange);
        }
    }
    Ok(TimelineCmd::AssignAssetBin {
        asset,
        old: a.bin,
        new: new_bin,
    })
}

#[cfg(test)]
mod tests {
    use super::super::time::FrameRate;
    use super::*;
    use crate::document::Document;
    use crate::history::Command;

    /// One sequence, one video track with a single clip. Returns the
    /// document plus the ids needed to address that clip.
    fn fixture() -> (Document, SequenceId, TrackId, ClipId) {
        let mut project = TimelineProject::new();
        let mut sequence = Sequence::new("Seq", FrameRate::FPS_30, 1920, 1080);
        let mut vtrack = Track::new(TrackKind::Video, "V1");
        let c = Clip::new(ClipSource::Adjustment, Tick(0), Tick(100));
        let clip_id = c.id;
        vtrack.clips.push(c);
        let track_id = vtrack.id;
        sequence.video_tracks.push(vtrack);
        let seq_id = sequence.id;
        project.insert_sequence(sequence);

        let mut doc = Document::new("t", 100.0, 100.0);
        doc.timeline = Some(project);
        (doc, seq_id, track_id, clip_id)
    }

    /// Adds a second (audio) clip to the fixture's sequence — for the
    /// linking tests, which need two distinct clips.
    fn add_audio_clip(doc: &mut Document, seq_id: SequenceId) -> (TrackId, ClipId) {
        let mut atrack = Track::new(TrackKind::Audio, "A1");
        let c = Clip::new(ClipSource::Adjustment, Tick(0), Tick(100));
        let clip_id = c.id;
        atrack.clips.push(c);
        let track_id = atrack.id;
        doc.timeline
            .as_mut()
            .unwrap()
            .sequences
            .get_mut(&seq_id)
            .unwrap()
            .audio_tracks
            .push(atrack);
        (track_id, clip_id)
    }

    /// Mirrors `tests/timeline.rs::assert_undo_roundtrip` (kept file-local —
    /// this story's territory is `ops.rs`, not the shared integration test
    /// file): `apply → inverse → apply` reproduces the post-apply state, and
    /// `inverse` alone restores the pre-apply state.
    fn assert_undo_roundtrip(doc: &Document, cmd: &TimelineCmd) {
        let before = doc.timeline.clone();

        let mut d1 = doc.clone();
        Command::Timeline(cmd.clone()).apply(&mut d1);
        let after_apply = d1.timeline.clone();

        let inv = cmd
            .inverse(&d1)
            .expect("SetClipProp-based ops always invert");
        let mut d2 = d1.clone();
        Command::Timeline(inv).apply(&mut d2);
        assert_eq!(
            d2.timeline, before,
            "inverse did not restore the original state"
        );

        let mut d3 = d2.clone();
        Command::Timeline(cmd.clone()).apply(&mut d3);
        assert_eq!(
            d3.timeline, after_apply,
            "apply -> inverse -> apply != apply"
        );
    }

    fn find_clip(doc: &Document, seq_id: SequenceId, track_id: TrackId, clip_id: ClipId) -> &Clip {
        doc.timeline
            .as_ref()
            .unwrap()
            .sequences
            .get(&seq_id)
            .unwrap()
            .track(track_id)
            .unwrap()
            .clips
            .iter()
            .find(|c| c.id == clip_id)
            .unwrap()
    }

    // ── Color label ──────────────────────────────────────────────────────

    #[test]
    fn set_clip_color_label_is_undo_idempotent() {
        let (doc, seq_id, track_id, clip_id) = fixture();
        let project = doc.timeline.as_ref().unwrap();
        let cmd = set_clip_color_label(project, seq_id, track_id, clip_id, Some(2)).unwrap();
        assert_undo_roundtrip(&doc, &cmd);

        let mut applied = doc.clone();
        Command::Timeline(cmd).apply(&mut applied);
        assert_eq!(
            find_clip(&applied, seq_id, track_id, clip_id).color_label,
            Some(2)
        );
    }

    #[test]
    fn set_clip_color_label_clear_is_undo_idempotent() {
        let (doc, seq_id, track_id, clip_id) = fixture();
        let project = doc.timeline.as_ref().unwrap();
        let set_cmd = set_clip_color_label(project, seq_id, track_id, clip_id, Some(5)).unwrap();
        let mut labeled = doc.clone();
        Command::Timeline(set_cmd).apply(&mut labeled);

        let project = labeled.timeline.as_ref().unwrap();
        let clear_cmd = set_clip_color_label(project, seq_id, track_id, clip_id, None).unwrap();
        assert_undo_roundtrip(&labeled, &clear_cmd);

        let mut cleared = labeled.clone();
        Command::Timeline(clear_cmd).apply(&mut cleared);
        assert_eq!(
            find_clip(&cleared, seq_id, track_id, clip_id).color_label,
            None
        );
    }

    // ── Linking ──────────────────────────────────────────────────────────

    #[test]
    fn link_clips_groups_both_and_is_undo_idempotent() {
        let (mut doc, seq_id, vtrack, vclip) = fixture();
        let (atrack, aclip) = add_audio_clip(&mut doc, seq_id);

        let project = doc.timeline.as_ref().unwrap();
        let cmds = link_clips(project, seq_id, vtrack, vclip, atrack, aclip).unwrap();
        assert_eq!(cmds.len(), 2);

        let before = doc.timeline.clone();
        let mut d1 = doc.clone();
        for c in &cmds {
            Command::Timeline(c.clone()).apply(&mut d1);
        }
        let group_v = find_clip(&d1, seq_id, vtrack, vclip).link_group;
        let group_a = find_clip(&d1, seq_id, atrack, aclip).link_group;
        assert!(group_v.is_some());
        assert_eq!(group_v, group_a, "both clips must share one link group");

        assert_eq!(
            clips_in_link_group(d1.timeline.as_ref().unwrap(), group_v.unwrap())
                .into_iter()
                .collect::<std::collections::HashSet<_>>(),
            [vclip, aclip].into_iter().collect()
        );

        // apply -> inverse (both, any order — each targets a distinct clip)
        // -> apply reproduces the linked state; inverse alone restores the
        // pre-link state.
        let invs: Vec<TimelineCmd> = cmds.iter().map(|c| c.inverse(&d1).unwrap()).collect();
        let mut d2 = d1.clone();
        for inv in &invs {
            Command::Timeline(inv.clone()).apply(&mut d2);
        }
        assert_eq!(d2.timeline, before);

        let mut d3 = d2.clone();
        for c in &cmds {
            Command::Timeline(c.clone()).apply(&mut d3);
        }
        assert_eq!(d3.timeline, d1.timeline);
    }

    #[test]
    fn link_clips_reuses_an_existing_group() {
        let (mut doc, seq_id, vtrack, vclip) = fixture();
        let (atrack, aclip) = add_audio_clip(&mut doc, seq_id);
        let (btrack, bclip) = add_audio_clip(&mut doc, seq_id);

        let project = doc.timeline.as_ref().unwrap();
        let cmds = link_clips(project, seq_id, vtrack, vclip, atrack, aclip).unwrap();
        for c in &cmds {
            Command::Timeline(c.clone()).apply(&mut doc);
        }
        let group = find_clip(&doc, seq_id, vtrack, vclip).link_group.unwrap();

        // Linking a third clip against the already-linked video clip must
        // join the *same* group rather than minting a new one.
        let project = doc.timeline.as_ref().unwrap();
        let more_cmds = link_clips(project, seq_id, vtrack, vclip, btrack, bclip).unwrap();
        for c in &more_cmds {
            Command::Timeline(c.clone()).apply(&mut doc);
        }
        assert_eq!(
            find_clip(&doc, seq_id, btrack, bclip).link_group,
            Some(group)
        );
    }

    #[test]
    fn unlink_clip_is_undo_idempotent() {
        let (doc, seq_id, track_id, clip_id) = fixture();
        let project = doc.timeline.as_ref().unwrap();
        let mut c = find_clip(&doc, seq_id, track_id, clip_id).clone();
        c.link_group = Some(LinkGroupId::new());
        let seed_cmd = set_clip_prop(project, seq_id, track_id, c).unwrap();
        let mut linked = doc.clone();
        Command::Timeline(seed_cmd).apply(&mut linked);
        assert!(find_clip(&linked, seq_id, track_id, clip_id)
            .link_group
            .is_some());

        let project = linked.timeline.as_ref().unwrap();
        let cmd = unlink_clip(project, seq_id, track_id, clip_id).unwrap();
        assert_undo_roundtrip(&linked, &cmd);

        let mut unlinked = linked.clone();
        Command::Timeline(cmd).apply(&mut unlinked);
        assert_eq!(
            find_clip(&unlinked, seq_id, track_id, clip_id).link_group,
            None
        );
    }

    #[test]
    fn unlink_clip_on_an_unlinked_clip_is_a_harmless_noop() {
        let (doc, seq_id, track_id, clip_id) = fixture();
        let project = doc.timeline.as_ref().unwrap();
        let cmd = unlink_clip(project, seq_id, track_id, clip_id).unwrap();
        assert_undo_roundtrip(&doc, &cmd);
    }

    #[test]
    fn clips_in_link_group_is_empty_for_an_unused_group() {
        let (doc, ..) = fixture();
        let project = doc.timeline.as_ref().unwrap();
        assert!(clips_in_link_group(project, LinkGroupId::new()).is_empty());
    }

    // ── Auto-reframe ─────────────────────────────────────────────────────

    #[test]
    fn fit_clip_to_format_16x9_to_9x16_covers_and_centers() {
        let content = SequenceFormat::new("16:9", 1920, 1080);
        let target = SequenceFormat::new("9:16", 1080, 1920);
        let xf = fit_clip_to_format(&content, &target);

        // Cover scale = max(1080/1920, 1920/1080) = 1920/1080 (the taller
        // axis needs more scale-up) — and it's > 1 (content must grow to
        // cover a narrower, taller frame).
        let expected = 1920.0 / 1080.0;
        assert!((xf.scale_x - expected).abs() < 1e-9);
        assert_eq!(xf.scale_x, xf.scale_y, "scale must be isotropic");
        assert!(xf.scale_x > 1.0);

        // Centered: position/anchor/rotation untouched from the default.
        assert_eq!(xf.x, 0.0);
        assert_eq!(xf.y, 0.0);
        assert_eq!(xf.anchor_x, 0.0);
        assert_eq!(xf.anchor_y, 0.0);
        assert_eq!(xf.rotation, 0.0);
        assert_eq!(xf.opacity, 1.0);
    }

    #[test]
    fn fit_clip_to_format_identity_for_matching_format() {
        let f = SequenceFormat::new("16:9", 1920, 1080);
        let xf = fit_clip_to_format(&f, &f);
        assert_eq!(xf.scale_x, 1.0);
        assert_eq!(xf.scale_y, 1.0);
    }

    #[test]
    fn fit_clip_to_format_9x16_to_16x9_also_covers() {
        // The reverse retarget: same numeric ratio picks the axis that now
        // needs the scale-up (still isotropic, still > 1).
        let content = SequenceFormat::new("9:16", 1080, 1920);
        let target = SequenceFormat::new("16:9", 1920, 1080);
        let xf = fit_clip_to_format(&content, &target);
        let expected = 1920.0 / 1080.0;
        assert!((xf.scale_x - expected).abs() < 1e-9);
        assert!(xf.scale_x > 1.0);
    }

    #[test]
    fn set_clip_reframe_set_and_clear_are_undo_idempotent() {
        let (doc, seq_id, track_id, clip_id) = fixture();
        let content = SequenceFormat::new("16:9", 1920, 1080);
        let target = SequenceFormat::new("9:16", 1080, 1920);
        let xf = fit_clip_to_format(&content, &target);

        let project = doc.timeline.as_ref().unwrap();
        let set_cmd = set_clip_reframe(project, seq_id, track_id, clip_id, 1, Some(xf)).unwrap();
        assert_undo_roundtrip(&doc, &set_cmd);

        let mut reframed = doc.clone();
        Command::Timeline(set_cmd).apply(&mut reframed);
        assert_eq!(
            find_clip(&reframed, seq_id, track_id, clip_id)
                .reframe
                .get(&1),
            Some(&xf)
        );

        let project = reframed.timeline.as_ref().unwrap();
        let clear_cmd = set_clip_reframe(project, seq_id, track_id, clip_id, 1, None).unwrap();
        assert_undo_roundtrip(&reframed, &clear_cmd);

        let mut cleared = reframed.clone();
        Command::Timeline(clear_cmd).apply(&mut cleared);
        assert!(!find_clip(&cleared, seq_id, track_id, clip_id)
            .reframe
            .contains_key(&1));
    }
}
