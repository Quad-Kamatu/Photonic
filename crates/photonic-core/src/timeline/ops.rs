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
use super::clip::{
    Clip, ClipEffect, ClipSource, ClipTransform, LinkGroupId, MulticamAngle, MulticamGroup,
    TextClipContent,
};
use super::commands::{
    AnimTarget, AudioCmd, ClipTiming, FormatOp, FxOwner, TimelineCmd, TrackSettings,
};
use super::grade::Grade;
use super::graph::NodeGraph;
use super::ids::*;
use super::media::MediaBin;
use super::sequence::{Marker, Sequence, SequenceFormat, TimelineProject, Track, TrackKind};
use super::time::{FrameRate, Tick};
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

/// Set or clear an asset's proxy attachment while preserving a lossless undo
/// snapshot. Used by background proxy generation when a job moves through
/// Pending → Ready/Failed.
pub fn set_asset_proxy(
    p: &TimelineProject,
    asset: AssetId,
    new: Option<super::media::ProxyRef>,
) -> Result<TimelineCmd, EditError> {
    let old = p
        .media
        .assets
        .get(&asset)
        .ok_or(EditError::NoAsset(asset))?
        .proxy
        .clone();
    Ok(TimelineCmd::SetAssetProxy { asset, old, new })
}

/// Update probe + content hash after L0 pool registration (24-preview-media-load
/// L1/L2). Row already exists via [`add_asset`]; this fills metadata without
/// removing/re-adding the asset id.
pub fn set_asset_meta(
    p: &TimelineProject,
    asset: AssetId,
    new_probe: Option<super::media::MediaProbe>,
    new_hash: Option<String>,
) -> Result<TimelineCmd, EditError> {
    let a = p
        .media
        .assets
        .get(&asset)
        .ok_or(EditError::NoAsset(asset))?;
    Ok(TimelineCmd::SetAssetMeta {
        asset,
        old_probe: a.probe.clone(),
        new_probe,
        old_hash: a.content_hash.clone(),
        new_hash,
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

/// Create a new (empty) sequence and return the command that adds it (17
/// §G-17). A thin convenience over [`add_sequence`] for the sequence-tab UI:
/// builds a [`Sequence`] with one `width`×`height` format and appends it,
/// activating it if the project had none. Undoable via `AddSequence`'s inverse.
pub fn create_sequence(
    name: impl Into<String>,
    frame_rate: FrameRate,
    width: u32,
    height: u32,
) -> TimelineCmd {
    add_sequence(Sequence::new(name, frame_rate, width, height))
}

/// Duplicate a sequence (17 §G-17). Deep-clones it with fresh structural ids
/// (see [`Sequence::duplicate_with_fresh_ids`]) under a `"<name> copy"` name,
/// then returns the `AddSequence` command that inserts the copy. Undoable.
pub fn duplicate_sequence(p: &TimelineProject, id: SequenceId) -> Result<TimelineCmd, EditError> {
    let s = seq(p, id)?;
    let mut dup = s.duplicate_with_fresh_ids();
    dup.name = format!("{} copy", s.name);
    Ok(add_sequence(dup))
}

/// Rename a sequence (17 §G-17 tab rename). Undoable via the `RenameSequence`
/// command (old/new names swapped on inverse).
pub fn rename_sequence(
    p: &TimelineProject,
    id: SequenceId,
    new_name: impl Into<String>,
) -> Result<TimelineCmd, EditError> {
    let s = seq(p, id)?;
    Ok(TimelineCmd::RenameSequence {
        seq: id,
        old: s.name.clone(),
        new: new_name.into(),
    })
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
    let len = s.tracks_for(kind).len();
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

/// Toggle a track's sync-lock (14 §M-9). Data + toggle only — the
/// ripple-propagation across sync-locked tracks is a later GUI concern. Reuses
/// [`set_track_prop`] (a whole-[`TrackSettings`] diff) so undo/redo rides the
/// existing `SetTrackProp` path.
pub fn toggle_sync_lock(
    p: &TimelineProject,
    id: SequenceId,
    track_id: TrackId,
) -> Result<TimelineCmd, EditError> {
    let s = seq(p, id)?;
    let t = track(s, track_id)?;
    let mut new = TrackSettings::of(t);
    new.sync_lock = !new.sync_lock;
    set_track_prop(p, id, track_id, new)
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

/// Create and insert a `ClipSource::Adjustment` clip spanning
/// `[start, start+duration)` on `track_id` (G-7 data half): an adjustment layer
/// whose effect stack / grade applies to the composite of every lower track
/// beneath its span. The clip carries no media — the create/insert is the model
/// op; the engine composites it (a separate lane). Undoable via the existing
/// [`insert_clip`] path (duration `> 0`, non-overlap and track existence
/// validated there).
pub fn add_adjustment_clip(
    p: &TimelineProject,
    id: SequenceId,
    track_id: TrackId,
    start: Tick,
    duration: Tick,
) -> Result<TimelineCmd, EditError> {
    insert_clip(
        p,
        id,
        track_id,
        Clip::new(ClipSource::Adjustment, start, duration),
    )
}

/// Create and insert a `ClipSource::Text` title/graphics clip spanning
/// `[start, start+duration)` on `track_id` (G-12 data half): styled text on a
/// video track, rendered by the engine's text path (no render here). The clip's
/// `name` defaults to the text so the timeline shows a friendly label. Undoable
/// via the existing [`insert_clip`] path.
pub fn add_text_clip(
    p: &TimelineProject,
    id: SequenceId,
    track_id: TrackId,
    start: Tick,
    duration: Tick,
    content: TextClipContent,
) -> Result<TimelineCmd, EditError> {
    let mut clip = Clip::new(ClipSource::Text { content }, start, duration);
    if let ClipSource::Text { content } = &clip.source {
        clip.name = content.text.clone();
    }
    insert_clip(p, id, track_id, clip)
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

/// **Replace With Clip / Replace Edit** (G-5, Premiere): swap a clip's SOURCE
/// (and, optionally, its `source_in`) in place — keeping the clip's timeline
/// `start`, `duration`, `speed`, transform, effect stack, grade, transitions,
/// audio, reframe, color label and link group untouched. The shot changes;
/// everything the editor built around the slot stays. Undoable via the existing
/// [`set_clip_prop`] whole-clip diff (one undo step).
///
/// Rejects a nested-sequence source that would cycle (mirrors [`insert_clip`])
/// and a replacement into `Adjustment` on a clip that still carries a
/// composition (07 §6.6). Duration is unchanged, so a shorter new source is
/// held to the slot (Premiere trims-to-slot; the source is sampled from
/// `new_source_in` for the slot's length by the engine).
pub fn replace_clip_source(
    p: &TimelineProject,
    id: SequenceId,
    track_id: TrackId,
    clip_id: ClipId,
    new_source: ClipSource,
    new_source_in: Option<Tick>,
) -> Result<TimelineCmd, EditError> {
    let s = seq(p, id)?;
    let t = track(s, track_id)?;
    let mut new = clip(t, clip_id)?.clone();

    if let ClipSource::NestedSequence { sequence } = &new_source {
        if *sequence == id || nests_into(p, *sequence, id) {
            return Err(EditError::SequenceCycle);
        }
    }
    if matches!(new_source, ClipSource::Adjustment) && new.composition.is_some() {
        return Err(EditError::CompositionOnAdjustment);
    }

    new.source = new_source;
    if let Some(si) = new_source_in {
        new.source_in = si;
    }
    set_clip_prop(p, id, track_id, new)
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

// ── Multicam (17 §G-20) ─────────────────────────────────────────────────────

/// Consolidate several camera angles into one multicam clip (17 §G-20): attach
/// a [`MulticamGroup`] to `primary_clip` built from itself (angle 0) plus each
/// clip in `angle_clips` (its `source`/`source_in`/`name` become angles 1..),
/// and remove those folded clips. The primary's `source`/`source_in` are
/// unchanged (they already equal angle 0). Any `angle_clips` entry that names
/// the primary itself is skipped (a clip can't be its own extra angle).
/// Returns a batch (`SetClipProp` for the primary + one `RemoveClip` per folded
/// clip) for the caller to wrap in one undo step.
pub fn create_multicam_group(
    p: &TimelineProject,
    id: SequenceId,
    primary_track: TrackId,
    primary_clip: ClipId,
    angle_clips: &[(TrackId, ClipId)],
) -> Result<Vec<TimelineCmd>, EditError> {
    let s = seq(p, id)?;
    let primary = clip(track(s, primary_track)?, primary_clip)?.clone();

    let mut angles = vec![MulticamAngle::new(
        primary.name.clone(),
        primary.source.clone(),
        primary.source_in,
    )];
    let mut removes = Vec::new();
    for (tk, cid) in angle_clips {
        if *cid == primary_clip {
            continue; // the primary is already angle 0
        }
        let c = clip(track(s, *tk)?, *cid)?;
        angles.push(MulticamAngle::new(
            c.name.clone(),
            c.source.clone(),
            c.source_in,
        ));
        removes.push(TimelineCmd::RemoveClip {
            seq: id,
            track: *tk,
            clip: Box::new(c.clone()),
        });
    }

    let mut new_primary = primary;
    new_primary.multicam = Some(MulticamGroup { angles, active: 0 });

    let mut cmds = vec![set_clip_prop(p, id, primary_track, new_primary)?];
    cmds.extend(removes);
    Ok(cmds)
}

/// Set the live angle of a multicam clip (17 §G-20). Clamps `angle` to a valid
/// index and mirrors the chosen angle's `source`/`source_in` onto the clip so a
/// multicam-unaware consumer still shows the live camera. Returns a
/// `SetClipProp`. Undoable. Errors ([`EditError::IndexOutOfRange`]) if the clip
/// carries no (non-empty) multicam group.
pub fn set_multicam_active_angle(
    p: &TimelineProject,
    id: SequenceId,
    track_id: TrackId,
    clip_id: ClipId,
    angle: usize,
) -> Result<TimelineCmd, EditError> {
    let s = seq(p, id)?;
    let t = track(s, track_id)?;
    let mut new = clip(t, clip_id)?.clone();
    let group = new.multicam.as_ref().ok_or(EditError::IndexOutOfRange)?;
    if group.angles.is_empty() {
        return Err(EditError::IndexOutOfRange);
    }
    let a = angle.min(group.angles.len() - 1);
    let chosen = group.angles[a].clone();
    new.multicam.as_mut().unwrap().active = a;
    new.source = chosen.source;
    new.source_in = chosen.source_in;
    set_clip_prop(p, id, track_id, new)
}

// ── Nested sequences (17 §G-16) & open/breadcrumb (17 §G-17) ─────────────────

/// Wrap the `clip_ids` selection on one track into a new nested sequence (17
/// §G-16): build a fresh sequence holding a copy of the selected clips (rebased
/// so the earliest starts at 0), then replace them on the outer track with one
/// [`ClipSource::NestedSequence`] clip spanning their bounding box. Returns the
/// new sequence's id plus a batch (`AddSequence`, one `RemoveClip` per selected
/// clip, then `InsertClip` for the nested clip) for the caller to wrap in one
/// undo step; the ordering keeps every intermediate apply-state invariant-valid.
///
/// Rejects an empty selection ([`EditError::IndexOutOfRange`]), a requested clip
/// absent from the track ([`EditError::NoClip`]), or a non-selected clip lying
/// inside the selection's span (which the single replacement clip would overlap
/// — [`EditError::Overlap`]). Internal gaps *between* selected clips are fine
/// (they become empty space in the nested sequence). No cycle can arise: the
/// nested sequence is brand-new and nothing references it yet.
pub fn create_nested_sequence(
    p: &TimelineProject,
    id: SequenceId,
    track_id: TrackId,
    clip_ids: &[ClipId],
    name: impl Into<String>,
) -> Result<(SequenceId, Vec<TimelineCmd>), EditError> {
    let s = seq(p, id)?;
    let t = track(s, track_id)?;
    if clip_ids.is_empty() {
        return Err(EditError::IndexOutOfRange);
    }
    // Any requested id that isn't on this track is an error.
    if let Some(missing) = clip_ids
        .iter()
        .find(|cid| !t.clips.iter().any(|c| c.id == **cid))
    {
        return Err(EditError::NoClip(*missing));
    }
    let sel_ids: std::collections::HashSet<ClipId> = clip_ids.iter().copied().collect();
    let selected: Vec<&Clip> = t.clips.iter().filter(|c| sel_ids.contains(&c.id)).collect();

    let min_start = selected.iter().map(|c| c.start).min().unwrap();
    let max_end = selected.iter().map(|c| c.end()).max().unwrap();
    // A non-selected clip inside the span would collide with the replacement.
    if t.clips
        .iter()
        .any(|c| !sel_ids.contains(&c.id) && c.start < max_end && min_start < c.end())
    {
        return Err(EditError::Overlap);
    }

    // Build the nested sequence: the selection rebased so `min_start → 0`, on a
    // fresh single track sized to the outer sequence's active format.
    let fmt = s.format();
    let mut inner = Sequence::new(name, s.frame_rate, fmt.width, fmt.height);
    let mut inner_track = Track::new(t.kind, t.name.clone());
    for c in &selected {
        let mut nc = (*c).clone();
        nc.id = ClipId::new();
        nc.start = c.start - min_start;
        inner_track.clips.push(nc);
    }
    inner.tracks_for_mut(t.kind).push(inner_track);
    let inner_id = inner.id;

    // Replace the selection with one NestedSequence clip over its bounding box.
    let mut nested_clip = Clip::new(
        ClipSource::NestedSequence { sequence: inner_id },
        min_start,
        max_end - min_start,
    );
    nested_clip.name = inner.name.clone();

    let mut cmds = vec![add_sequence(inner)];
    for c in &selected {
        cmds.push(TimelineCmd::RemoveClip {
            seq: id,
            track: track_id,
            clip: Box::new((*c).clone()),
        });
    }
    cmds.push(TimelineCmd::InsertClip {
        seq: id,
        track: track_id,
        clip: Box::new(nested_clip),
    });
    Ok((inner_id, cmds))
}

/// The nested sequence a clip opens into (17 §G-16/G-17 double-click target),
/// or `None` for a non-nested clip. Pure read.
pub fn nested_target(c: &Clip) -> Option<SequenceId> {
    match &c.source {
        ClipSource::NestedSequence { sequence } => Some(*sequence),
        _ => None,
    }
}

/// The breadcrumb ancestry of `target`: the chain `[root, …, target]` where
/// each entry nests the next via a [`ClipSource::NestedSequence`] clip (17
/// §G-17). `target` is always last; a sequence no other sequence nests is its
/// own root (a single-element chain). Nesting is acyclic (cycle-checked at edit
/// time) so the walk terminates; a `seen` guard defends against a malformed
/// cycle regardless. Pure read; the GUI renders it as a clickable trail.
pub fn sequence_ancestry(p: &TimelineProject, target: SequenceId) -> Vec<SequenceId> {
    let mut chain = vec![target];
    let mut current = target;
    let mut seen = std::collections::HashSet::new();
    seen.insert(current);
    while let Some(parent) = nesting_parent(p, current) {
        if !seen.insert(parent) {
            break;
        }
        chain.push(parent);
        current = parent;
    }
    chain.reverse();
    chain
}

/// A sequence that directly nests `child` via a `NestedSequence` clip, if any.
fn nesting_parent(p: &TimelineProject, child: SequenceId) -> Option<SequenceId> {
    p.sequences.values().find_map(|s| {
        let nests = s
            .video_tracks
            .iter()
            .chain(s.audio_tracks.iter())
            .flat_map(|t| t.clips.iter())
            .any(|c| {
                matches!(&c.source, ClipSource::NestedSequence { sequence } if *sequence == child)
            });
        nests.then_some(s.id)
    })
}

// ── 3/4-point editing: insert / overwrite / lift / extract (16 §2, gap L-1) ──
//
// The four reference-NLE edit ops. Each is a pure fn returning a batch of the
// existing timeline primitives (`SplitClip`/`RemoveClip`/`RippleEdit`/
// `TrimClip`/`InsertClip`) for the caller to wrap in ONE `Command::Batch` (one
// undo step). The command order is chosen so every intermediate apply-state
// stays invariant-valid (the debug-time per-command `validate()` in
// `TimelineCmd::apply`): removes/shrinks precede any insert into freshly-opened
// space, and multi-clip shifts ride a single atomic `RippleEdit`.

/// New timing for trimming a clip's OUT-point back to `boundary` (shrink; the
/// clip keeps its `start`/`source_in`).
fn trim_end_to(c: &Clip, boundary: Tick) -> ClipTiming {
    ClipTiming {
        start: c.start,
        duration: boundary - c.start,
        source_in: c.source_in,
    }
}

/// New timing for trimming a clip's IN-point forward to `boundary` (shrink; the
/// timeline `start` moves to `boundary` and `source_in` advances by the
/// speed-scaled delta).
fn trim_start_to(c: &Clip, boundary: Tick) -> ClipTiming {
    let delta = boundary - c.start;
    ClipTiming {
        start: boundary,
        duration: c.duration - delta,
        source_in: c.source_in + c.speed.source_delta(delta),
    }
}

/// Reject a source clip whose nested sequence would cycle (mirrors
/// [`insert_clip`]); a no-op for every other source kind.
fn reject_nested_cycle(
    p: &TimelineProject,
    id: SequenceId,
    source: &Clip,
) -> Result<(), EditError> {
    if let ClipSource::NestedSequence { sequence } = &source.source {
        if *sequence == id || nests_into(p, *sequence, id) {
            return Err(EditError::SequenceCycle);
        }
    }
    Ok(())
}

/// Shared core of lift/overwrite/extract: clear the content in `[rs, re)` on
/// `t`, then shift every clip that survives at/after `re` by `delta`
/// (`Tick::ZERO` = leave the gap, for lift/overwrite; `-(re-rs)` = close it, for
/// extract). Emits removes first, then one atomic `RippleEdit` carrying every
/// timing change (trims + shifts) with each clip's *original* timing as `old`,
/// then an `InsertClip` for the tail of a clip that spanned the whole range —
/// an order in which every intermediate state is invariant-valid.
fn clear_and_shift(
    t: &Track,
    id: SequenceId,
    track_id: TrackId,
    rs: Tick,
    re: Tick,
    delta: Tick,
) -> Vec<TimelineCmd> {
    let mut removes: Vec<TimelineCmd> = Vec::new();
    let mut changes: Vec<(ClipId, ClipTiming, ClipTiming)> = Vec::new();
    let mut tail: Option<Clip> = None;

    for c in &t.clips {
        if c.end() <= rs {
            continue; // entirely left of the range — untouched
        }
        if c.start >= re {
            // Entirely right of the range — shift wholesale by `delta`.
            if delta.0 != 0 {
                let old = ClipTiming::of(c);
                changes.push((
                    c.id,
                    old,
                    ClipTiming {
                        start: c.start + delta,
                        ..old
                    },
                ));
            }
            continue;
        }
        // `c` intersects the range.
        match (c.start < rs, c.end() > re) {
            (false, false) => {
                // Fully inside the range — remove.
                removes.push(TimelineCmd::RemoveClip {
                    seq: id,
                    track: track_id,
                    clip: Box::new(c.clone()),
                });
            }
            (true, false) => {
                // Left overhang — trim the OUT-point to `rs` (stays put).
                changes.push((c.id, ClipTiming::of(c), trim_end_to(c, rs)));
            }
            (false, true) => {
                // Right overhang — trim the IN-point to `re`, then shift by delta.
                let post = trim_start_to(c, re);
                changes.push((
                    c.id,
                    ClipTiming::of(c),
                    ClipTiming {
                        start: post.start + delta,
                        ..post
                    },
                ));
            }
            (true, true) => {
                // Spans the whole range — head stays trimmed to `[start, rs)`;
                // the tail becomes a fresh clip at `re + delta`.
                changes.push((c.id, ClipTiming::of(c), trim_end_to(c, rs)));
                let mut nt = c.clone();
                nt.id = ClipId::new();
                nt.transition_in = None;
                nt.transition_out = None;
                let post = trim_start_to(c, re);
                ClipTiming {
                    start: post.start + delta,
                    ..post
                }
                .apply_to(&mut nt);
                tail = Some(nt);
            }
        }
    }

    let mut cmds = removes;
    if !changes.is_empty() {
        cmds.push(TimelineCmd::RippleEdit {
            seq: id,
            track: track_id,
            changes,
        });
    }
    if let Some(nt) = tail {
        cmds.push(TimelineCmd::InsertClip {
            seq: id,
            track: track_id,
            clip: Box::new(nt),
        });
    }
    cmds
}

/// **Insert edit** (3-point, Premiere `,`): open a gap of `source`'s duration at
/// `at` on `target_track` — splitting any clip straddling `at` and rippling all
/// clips at/after `at` on that track RIGHT — then drop `source` into the gap.
/// The track's content grows by the source duration. Returns a batch
/// (`SplitClip?`, `RippleEdit?`, `InsertClip`) for the caller to wrap in one
/// undo step.
pub fn insert_edit(
    p: &TimelineProject,
    id: SequenceId,
    track_id: TrackId,
    at: Tick,
    source: Clip,
) -> Result<Vec<TimelineCmd>, EditError> {
    let s = seq(p, id)?;
    let t = track(s, track_id)?;
    let shift = source.duration;
    if shift.0 <= 0 {
        return Err(EditError::NonPositiveDuration);
    }
    if at.0 < 0 {
        return Err(EditError::Overlap);
    }
    reject_nested_cycle(p, id, &source)?;

    let mut cmds = Vec::new();
    let mut changes: Vec<(ClipId, ClipTiming, ClipTiming)> = Vec::new();

    // 1. Split a clip straddling `at`; its right half must ripple too.
    if let Some(c) = t.clips.iter().find(|c| c.start < at && at < c.end()) {
        let new_clip_id = ClipId::new();
        cmds.push(TimelineCmd::SplitClip {
            seq: id,
            track: track_id,
            clip: c.id,
            at,
            new_clip_id,
        });
        // Post-split timing of the right half (mirrors `SplitClip::apply`, which
        // advances `source_in` by the left-half duration without speed scaling).
        let right = ClipTiming {
            start: at,
            duration: c.end() - at,
            source_in: c.source_in + (at - c.start).max(Tick::ZERO),
        };
        changes.push((
            new_clip_id,
            right,
            ClipTiming {
                start: at + shift,
                ..right
            },
        ));
    }

    // 2. Ripple every clip at/after `at` right by the source duration.
    for other in &t.clips {
        if other.start >= at {
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
    if !changes.is_empty() {
        cmds.push(TimelineCmd::RippleEdit {
            seq: id,
            track: track_id,
            changes,
        });
    }

    // 3. Drop the source clip into the opened gap at `at`.
    let mut placed = source;
    placed.start = at;
    cmds.push(TimelineCmd::InsertClip {
        seq: id,
        track: track_id,
        clip: Box::new(placed),
    });
    Ok(cmds)
}

/// **Overwrite edit** (Premiere `.`): drop `source` at `at` on `target_track`,
/// replacing whatever it covers — trimming partially-covered clips, removing
/// fully-covered ones, splitting a clip that spans the region — with NO ripple.
/// Timeline duration is unchanged unless `source` extends past the old end.
pub fn overwrite_edit(
    p: &TimelineProject,
    id: SequenceId,
    track_id: TrackId,
    at: Tick,
    source: Clip,
) -> Result<Vec<TimelineCmd>, EditError> {
    let s = seq(p, id)?;
    let t = track(s, track_id)?;
    let dur = source.duration;
    if dur.0 <= 0 {
        return Err(EditError::NonPositiveDuration);
    }
    if at.0 < 0 {
        return Err(EditError::Overlap);
    }
    reject_nested_cycle(p, id, &source)?;

    // Clear `[at, at+dur)` leaving the gap (no ripple), then fill it.
    let mut cmds = clear_and_shift(t, id, track_id, at, at + dur, Tick::ZERO);
    let mut placed = source;
    placed.start = at;
    cmds.push(TimelineCmd::InsertClip {
        seq: id,
        track: track_id,
        clip: Box::new(placed),
    });
    Ok(cmds)
}

/// **Lift edit** (Premiere `;`): remove the content in `range` on `track`,
/// leaving a gap (no ripple). Timeline duration is unchanged.
pub fn lift_edit(
    p: &TimelineProject,
    id: SequenceId,
    track_id: TrackId,
    range: (Tick, Tick),
) -> Result<Vec<TimelineCmd>, EditError> {
    let (rs, re) = range;
    if re <= rs {
        return Err(EditError::NonPositiveDuration);
    }
    let s = seq(p, id)?;
    let t = track(s, track_id)?;
    Ok(clear_and_shift(t, id, track_id, rs, re, Tick::ZERO))
}

/// **Extract edit** (Premiere `'`): remove the content in `range` on `track`
/// AND ripple everything after it LEFT to close the gap (generalizes
/// [`ripple_delete`]). The track's content shrinks by the range width.
pub fn extract_edit(
    p: &TimelineProject,
    id: SequenceId,
    track_id: TrackId,
    range: (Tick, Tick),
) -> Result<Vec<TimelineCmd>, EditError> {
    let (rs, re) = range;
    if re <= rs {
        return Err(EditError::NonPositiveDuration);
    }
    let s = seq(p, id)?;
    let t = track(s, track_id)?;
    // Right-of-range content closes the gap by shifting left by its width.
    Ok(clear_and_shift(t, id, track_id, rs, re, Tick(rs.0 - re.0)))
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

    // ── Replace edit + adjustment/text create (G-5 / G-7 / G-12) ──────────

    #[test]
    fn replace_clip_source_keeps_timing_effects_grade_and_is_undo_idempotent() {
        use super::super::clip::{Ratio, SpeedKey, SpeedMap};
        use super::super::effect_kind::EffectKind;

        let (doc, seq_id, track_id, clip_id) = fixture();

        // Seed the slot with an effect, a grade, and a keyframed speed ramp —
        // exactly the "everything the editor built" that Replace must preserve.
        let mut seeded = find_clip(&doc, seq_id, track_id, clip_id).clone();
        seeded.effects.push(ClipEffect::new(EffectKind::Blur));
        seeded.grade = Some(Grade::default());
        seeded.speed = SpeedMap::Keyframed {
            keys: vec![SpeedKey::new(Tick(0), Ratio::new(1, 2))],
        };
        let install =
            set_clip_prop(doc.timeline.as_ref().unwrap(), seq_id, track_id, seeded).unwrap();
        let mut seeded_doc = doc.clone();
        Command::Timeline(install).apply(&mut seeded_doc);

        let new_source = ClipSource::SolidColor {
            color: crate::Color {
                r: 0.2,
                g: 0.4,
                b: 0.6,
                a: 1.0,
            },
        };
        let project = seeded_doc.timeline.as_ref().unwrap();
        let cmd = replace_clip_source(
            project,
            seq_id,
            track_id,
            clip_id,
            new_source.clone(),
            Some(Tick(500)),
        )
        .unwrap();
        assert_undo_roundtrip(&seeded_doc, &cmd);

        let mut replaced = seeded_doc.clone();
        Command::Timeline(cmd).apply(&mut replaced);
        let c = find_clip(&replaced, seq_id, track_id, clip_id);
        assert_eq!(c.source, new_source, "source swapped");
        assert_eq!(c.source_in, Tick(500), "source_in updated");
        assert_eq!(c.start, Tick(0), "start preserved");
        assert_eq!(c.duration, Tick(100), "duration preserved");
        assert_eq!(c.effects.len(), 1, "effect stack preserved");
        assert!(c.grade.is_some(), "grade preserved");
        assert!(
            matches!(c.speed, SpeedMap::Keyframed { .. }),
            "speed ramp preserved"
        );
    }

    #[test]
    fn replace_clip_source_none_keeps_source_in_and_is_undo_idempotent() {
        let (doc, seq_id, track_id, clip_id) = fixture();
        let new_source = ClipSource::SolidColor {
            color: crate::Color {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 1.0,
            },
        };
        let project = doc.timeline.as_ref().unwrap();
        let cmd =
            replace_clip_source(project, seq_id, track_id, clip_id, new_source, None).unwrap();
        assert_undo_roundtrip(&doc, &cmd);
    }

    #[test]
    fn add_adjustment_clip_inserts_and_is_undo_idempotent() {
        let (doc, seq_id, track_id) = track_fixture(&[]);
        let project = doc.timeline.as_ref().unwrap();
        let cmd = add_adjustment_clip(project, seq_id, track_id, Tick(200), Tick(150)).unwrap();
        assert_undo_roundtrip(&doc, &cmd);

        let mut applied = doc.clone();
        Command::Timeline(cmd).apply(&mut applied);
        let clips = &the_track(&applied, seq_id, track_id).clips;
        assert_eq!(clips.len(), 1);
        assert!(matches!(clips[0].source, ClipSource::Adjustment));
        assert_eq!((clips[0].start.0, clips[0].duration.0), (200, 150));
    }

    #[test]
    fn add_adjustment_clip_rejects_overlap() {
        let (doc, seq_id, track_id) = track_fixture(&[(0, 100)]);
        let project = doc.timeline.as_ref().unwrap();
        assert_eq!(
            add_adjustment_clip(project, seq_id, track_id, Tick(50), Tick(100)),
            Err(EditError::Overlap)
        );
    }

    #[test]
    fn add_text_clip_inserts_titled_and_is_undo_idempotent() {
        use super::super::clip::TextClipContent;
        let (doc, seq_id, track_id) = track_fixture(&[]);
        let project = doc.timeline.as_ref().unwrap();
        let cmd = add_text_clip(
            project,
            seq_id,
            track_id,
            Tick(0),
            Tick(90),
            TextClipContent::new("Hello"),
        )
        .unwrap();
        assert_undo_roundtrip(&doc, &cmd);

        let mut applied = doc.clone();
        Command::Timeline(cmd).apply(&mut applied);
        let clips = &the_track(&applied, seq_id, track_id).clips;
        assert_eq!(clips.len(), 1);
        assert_eq!(clips[0].name, "Hello", "name defaults to the text");
        assert!(
            matches!(&clips[0].source, ClipSource::Text { content } if content.text == "Hello")
        );
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

    // ── Sync lock (M-9) ──────────────────────────────────────────────────

    #[test]
    fn toggle_sync_lock_flips_and_is_undo_idempotent() {
        let (doc, seq_id, track_id, _) = fixture();
        let project = doc.timeline.as_ref().unwrap();
        assert!(
            !project.sequences[&seq_id]
                .track(track_id)
                .unwrap()
                .sync_lock
        );

        let cmd = toggle_sync_lock(project, seq_id, track_id).unwrap();
        assert_undo_roundtrip(&doc, &cmd);

        let mut on = doc.clone();
        Command::Timeline(cmd).apply(&mut on);
        assert!(
            on.timeline.as_ref().unwrap().sequences[&seq_id]
                .track(track_id)
                .unwrap()
                .sync_lock,
            "toggle must set sync_lock"
        );

        // A second toggle flips it back off.
        let project = on.timeline.as_ref().unwrap();
        let cmd2 = toggle_sync_lock(project, seq_id, track_id).unwrap();
        let mut off = on.clone();
        Command::Timeline(cmd2).apply(&mut off);
        assert!(
            !off.timeline.as_ref().unwrap().sequences[&seq_id]
                .track(track_id)
                .unwrap()
                .sync_lock
        );
    }

    // ── 3/4-point editing (insert / overwrite / lift / extract) ──────────

    /// A document with one video track carrying `spans` (`(start, dur)` pairs,
    /// assumed sorted + non-overlapping). Returns the ids to address the track.
    fn track_fixture(spans: &[(i64, i64)]) -> (Document, SequenceId, TrackId) {
        let mut project = TimelineProject::new();
        let mut sequence = Sequence::new("Seq", FrameRate::FPS_30, 1920, 1080);
        let mut vtrack = Track::new(TrackKind::Video, "V1");
        for (start, dur) in spans {
            vtrack
                .clips
                .push(Clip::new(ClipSource::Adjustment, Tick(*start), Tick(*dur)));
        }
        let track_id = vtrack.id;
        sequence.video_tracks.push(vtrack);
        let seq_id = sequence.id;
        project.insert_sequence(sequence);
        let mut doc = Document::new("t", 100.0, 100.0);
        doc.timeline = Some(project);
        (doc, seq_id, track_id)
    }

    fn the_track(doc: &Document, seq_id: SequenceId, track_id: TrackId) -> &Track {
        doc.timeline.as_ref().unwrap().sequences[&seq_id]
            .track(track_id)
            .unwrap()
    }

    /// `(start, duration)` of every clip on the track, in stored order.
    fn spans_of(doc: &Document, seq_id: SequenceId, track_id: TrackId) -> Vec<(i64, i64)> {
        the_track(doc, seq_id, track_id)
            .clips
            .iter()
            .map(|c| (c.start.0, c.duration.0))
            .collect()
    }

    /// Max clip end on the track (its content length); 0 when empty.
    fn track_end(doc: &Document, seq_id: SequenceId, track_id: TrackId) -> i64 {
        the_track(doc, seq_id, track_id)
            .clips
            .iter()
            .map(|c| c.end().0)
            .max()
            .unwrap_or(0)
    }

    fn as_batch(cmds: &[TimelineCmd]) -> Command {
        Command::Batch(cmds.iter().cloned().map(Command::Timeline).collect())
    }

    fn apply_batch(doc: &Document, cmds: &[TimelineCmd]) -> Document {
        let mut d = doc.clone();
        as_batch(cmds).apply(&mut d);
        d
    }

    /// A batch (the shape the four edit ops return) round-trips: `apply →
    /// inverse` restores the pre-state, and `apply → inverse → apply`
    /// reproduces the post-apply state (undo idempotency).
    fn assert_batch_undo_roundtrip(doc: &Document, cmds: &[TimelineCmd]) {
        let before = doc.timeline.clone();
        let batch = as_batch(cmds);

        let mut d1 = doc.clone();
        batch.apply(&mut d1);
        let after_apply = d1.timeline.clone();

        let inv = batch.inverse(&d1).expect("edit batches always invert");
        let mut d2 = d1.clone();
        inv.apply(&mut d2);
        assert_eq!(d2.timeline, before, "inverse did not restore the pre-state");

        let mut d3 = d2.clone();
        batch.apply(&mut d3);
        assert_eq!(
            d3.timeline, after_apply,
            "apply -> inverse -> apply != apply"
        );
    }

    fn validate_ok(doc: &Document, seq_id: SequenceId) {
        let s = &doc.timeline.as_ref().unwrap().sequences[&seq_id];
        assert!(s.validate().is_ok(), "invariant broken: {:?}", s.validate());
    }

    fn adj_clip(dur: i64) -> Clip {
        Clip::new(ClipSource::Adjustment, Tick::ZERO, Tick(dur))
    }

    // ── Insert ───────────────────────────────────────────────────────────

    #[test]
    fn insert_edit_grows_track_by_source_duration() {
        // `at` straddles the middle clip; the whole track grows by the source.
        let (doc, seq_id, track_id) = track_fixture(&[(0, 100), (100, 100), (200, 100)]);
        let before_end = track_end(&doc, seq_id, track_id);
        let p = doc.timeline.as_ref().unwrap();
        let cmds = insert_edit(p, seq_id, track_id, Tick(150), adj_clip(50)).unwrap();
        let out = apply_batch(&doc, &cmds);

        validate_ok(&out, seq_id);
        assert_eq!(track_end(&out, seq_id, track_id), before_end + 50);
        // The split boundary produced a source clip in the opened gap [150,200).
        assert_eq!(
            spans_of(&out, seq_id, track_id),
            vec![(0, 100), (100, 50), (150, 50), (200, 50), (250, 100)]
        );
        assert_batch_undo_roundtrip(&doc, &cmds);
    }

    #[test]
    fn insert_edit_at_clip_boundary_ripples_without_splitting() {
        let (doc, seq_id, track_id) = track_fixture(&[(0, 100), (100, 100)]);
        let p = doc.timeline.as_ref().unwrap();
        let cmds = insert_edit(p, seq_id, track_id, Tick(100), adj_clip(40)).unwrap();
        // No SplitClip — `at` is a cut point, not inside a clip.
        assert!(!cmds
            .iter()
            .any(|c| matches!(c, TimelineCmd::SplitClip { .. })));
        let out = apply_batch(&doc, &cmds);
        validate_ok(&out, seq_id);
        assert_eq!(
            spans_of(&out, seq_id, track_id),
            vec![(0, 100), (100, 40), (140, 100)]
        );
        assert_batch_undo_roundtrip(&doc, &cmds);
    }

    #[test]
    fn insert_edit_beyond_content_leaves_a_lead_gap() {
        let (doc, seq_id, track_id) = track_fixture(&[(0, 100)]);
        let p = doc.timeline.as_ref().unwrap();
        let cmds = insert_edit(p, seq_id, track_id, Tick(300), adj_clip(50)).unwrap();
        let out = apply_batch(&doc, &cmds);
        validate_ok(&out, seq_id);
        assert_eq!(track_end(&out, seq_id, track_id), 350);
        assert_batch_undo_roundtrip(&doc, &cmds);
    }

    // ── Overwrite ────────────────────────────────────────────────────────

    #[test]
    fn overwrite_edit_keeps_duration_and_punches_a_hole() {
        // Source lands inside one long clip → head + source + tail, same end.
        let (doc, seq_id, track_id) = track_fixture(&[(0, 300)]);
        let before_end = track_end(&doc, seq_id, track_id);
        let p = doc.timeline.as_ref().unwrap();
        let cmds = overwrite_edit(p, seq_id, track_id, Tick(100), adj_clip(50)).unwrap();
        let out = apply_batch(&doc, &cmds);

        validate_ok(&out, seq_id);
        assert_eq!(track_end(&out, seq_id, track_id), before_end);
        assert_eq!(
            spans_of(&out, seq_id, track_id),
            vec![(0, 100), (100, 50), (150, 150)]
        );
        assert_batch_undo_roundtrip(&doc, &cmds);
    }

    #[test]
    fn overwrite_edit_removes_fully_covered_and_trims_neighbours() {
        let (doc, seq_id, track_id) =
            track_fixture(&[(0, 100), (100, 100), (200, 100), (300, 100)]);
        let p = doc.timeline.as_ref().unwrap();
        // [80, 320) covers clip #2 fully, trims the tail of #1 and head of #4.
        let cmds = overwrite_edit(p, seq_id, track_id, Tick(80), adj_clip(240)).unwrap();
        let out = apply_batch(&doc, &cmds);
        validate_ok(&out, seq_id);
        assert_eq!(
            spans_of(&out, seq_id, track_id),
            vec![(0, 80), (80, 240), (320, 80)]
        );
        assert_batch_undo_roundtrip(&doc, &cmds);
    }

    #[test]
    fn overwrite_edit_past_end_extends() {
        let (doc, seq_id, track_id) = track_fixture(&[(0, 100)]);
        let p = doc.timeline.as_ref().unwrap();
        let cmds = overwrite_edit(p, seq_id, track_id, Tick(50), adj_clip(200)).unwrap();
        let out = apply_batch(&doc, &cmds);
        validate_ok(&out, seq_id);
        assert_eq!(spans_of(&out, seq_id, track_id), vec![(0, 50), (50, 200)]);
        assert_batch_undo_roundtrip(&doc, &cmds);
    }

    // ── Lift ─────────────────────────────────────────────────────────────

    #[test]
    fn lift_edit_keeps_duration_and_leaves_a_gap() {
        let (doc, seq_id, track_id) = track_fixture(&[(0, 100), (100, 100), (200, 100)]);
        let before_end = track_end(&doc, seq_id, track_id);
        let p = doc.timeline.as_ref().unwrap();
        let cmds = lift_edit(p, seq_id, track_id, (Tick(100), Tick(200))).unwrap();
        let out = apply_batch(&doc, &cmds);

        validate_ok(&out, seq_id);
        assert_eq!(track_end(&out, seq_id, track_id), before_end);
        // The middle clip is gone; the [100,200) gap remains open.
        assert_eq!(spans_of(&out, seq_id, track_id), vec![(0, 100), (200, 100)]);
        assert_batch_undo_roundtrip(&doc, &cmds);
    }

    #[test]
    fn lift_edit_splits_a_spanning_clip_into_head_and_tail() {
        let (doc, seq_id, track_id) = track_fixture(&[(0, 300)]);
        let p = doc.timeline.as_ref().unwrap();
        let cmds = lift_edit(p, seq_id, track_id, (Tick(100), Tick(200))).unwrap();
        let out = apply_batch(&doc, &cmds);
        validate_ok(&out, seq_id);
        assert_eq!(spans_of(&out, seq_id, track_id), vec![(0, 100), (200, 100)]);
        // Nothing intersects the lifted range.
        for c in &the_track(&out, seq_id, track_id).clips {
            assert!(c.end().0 <= 100 || c.start.0 >= 200);
        }
        assert_batch_undo_roundtrip(&doc, &cmds);
    }

    // ── Extract ──────────────────────────────────────────────────────────

    #[test]
    fn extract_edit_shrinks_track_by_range_width() {
        let (doc, seq_id, track_id) = track_fixture(&[(0, 100), (100, 100), (200, 100)]);
        let before_end = track_end(&doc, seq_id, track_id);
        let p = doc.timeline.as_ref().unwrap();
        let cmds = extract_edit(p, seq_id, track_id, (Tick(100), Tick(200))).unwrap();
        let out = apply_batch(&doc, &cmds);

        validate_ok(&out, seq_id);
        assert_eq!(track_end(&out, seq_id, track_id), before_end - 100);
        // The gap closed: the third clip slid left into the second's slot.
        assert_eq!(spans_of(&out, seq_id, track_id), vec![(0, 100), (100, 100)]);
        assert_batch_undo_roundtrip(&doc, &cmds);
    }

    #[test]
    fn extract_edit_spanning_clip_closes_the_gap() {
        // One clip spans the whole range and there is trailing content to slide.
        let (doc, seq_id, track_id) = track_fixture(&[(0, 300), (300, 100)]);
        let before_end = track_end(&doc, seq_id, track_id);
        let p = doc.timeline.as_ref().unwrap();
        let cmds = extract_edit(p, seq_id, track_id, (Tick(100), Tick(200))).unwrap();
        let out = apply_batch(&doc, &cmds);
        validate_ok(&out, seq_id);
        assert_eq!(track_end(&out, seq_id, track_id), before_end - 100);
        assert_eq!(
            spans_of(&out, seq_id, track_id),
            vec![(0, 100), (100, 100), (200, 100)]
        );
        assert_batch_undo_roundtrip(&doc, &cmds);
    }

    #[test]
    fn extract_edit_matches_ripple_delete_for_a_whole_clip_range() {
        // Extract over exactly one clip's span equals ripple_delete of it.
        let (doc, seq_id, track_id) = track_fixture(&[(0, 100), (100, 100), (200, 100)]);
        let p = doc.timeline.as_ref().unwrap();
        let extract = extract_edit(p, seq_id, track_id, (Tick(100), Tick(200))).unwrap();
        let mid = the_track(&doc, seq_id, track_id).clips[1].id;
        let ripple = ripple_delete(p, seq_id, track_id, mid).unwrap();

        let a = apply_batch(&doc, &extract);
        let b = apply_batch(&doc, &ripple);
        assert_eq!(
            spans_of(&a, seq_id, track_id),
            spans_of(&b, seq_id, track_id)
        );
    }

    // ── Invariant proptests (sorted / non-overlap / undo) ────────────────

    use proptest::prelude::*;

    /// A random sorted, non-overlapping span set, built by walking a cursor
    /// forward through random (gap, duration) pairs.
    fn arb_spans() -> impl Strategy<Value = Vec<(i64, i64)>> {
        prop::collection::vec((0i64..200, 1i64..200), 0..8).prop_map(|pairs| {
            let mut cursor = 0i64;
            let mut spans = Vec::new();
            for (gap, dur) in pairs {
                cursor += gap;
                spans.push((cursor, dur));
                cursor += dur;
            }
            spans
        })
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(200))]

        #[test]
        fn insert_edit_preserves_invariants_and_end(
            spans in arb_spans(),
            at in 0i64..2500,
            dur in 1i64..300,
        ) {
            let (doc, seq_id, track_id) = track_fixture(&spans);
            let before_end = track_end(&doc, seq_id, track_id);
            let p = doc.timeline.as_ref().unwrap();
            let cmds = insert_edit(p, seq_id, track_id, Tick(at), adj_clip(dur)).unwrap();
            let out = apply_batch(&doc, &cmds);
            let s = &out.timeline.as_ref().unwrap().sequences[&seq_id];
            prop_assert!(s.validate().is_ok(), "invariant broken: {:?}", s.validate());
            prop_assert_eq!(track_end(&out, seq_id, track_id), before_end.max(at) + dur);
            assert_batch_undo_roundtrip(&doc, &cmds);
        }

        #[test]
        fn overwrite_edit_preserves_invariants_and_end(
            spans in arb_spans(),
            at in 0i64..2500,
            dur in 1i64..300,
        ) {
            let (doc, seq_id, track_id) = track_fixture(&spans);
            let before_end = track_end(&doc, seq_id, track_id);
            let p = doc.timeline.as_ref().unwrap();
            let cmds = overwrite_edit(p, seq_id, track_id, Tick(at), adj_clip(dur)).unwrap();
            let out = apply_batch(&doc, &cmds);
            let s = &out.timeline.as_ref().unwrap().sequences[&seq_id];
            prop_assert!(s.validate().is_ok(), "invariant broken: {:?}", s.validate());
            prop_assert_eq!(track_end(&out, seq_id, track_id), before_end.max(at + dur));
            assert_batch_undo_roundtrip(&doc, &cmds);
        }

        #[test]
        fn lift_edit_preserves_invariants_and_clears_range(
            spans in arb_spans(),
            rs in 0i64..2500,
            width in 1i64..400,
        ) {
            let re = rs + width;
            let (doc, seq_id, track_id) = track_fixture(&spans);
            let before_end = track_end(&doc, seq_id, track_id);
            let p = doc.timeline.as_ref().unwrap();
            let cmds = lift_edit(p, seq_id, track_id, (Tick(rs), Tick(re))).unwrap();
            let out = apply_batch(&doc, &cmds);
            let s = &out.timeline.as_ref().unwrap().sequences[&seq_id];
            prop_assert!(s.validate().is_ok(), "invariant broken: {:?}", s.validate());
            // Lift never shifts content, so the track can only shorten, never grow.
            prop_assert!(track_end(&out, seq_id, track_id) <= before_end);
            // Nothing intersects the lifted (still-open) range.
            for c in &the_track(&out, seq_id, track_id).clips {
                prop_assert!(c.end().0 <= rs || c.start.0 >= re);
            }
            assert_batch_undo_roundtrip(&doc, &cmds);
        }

        #[test]
        fn extract_edit_preserves_invariants_and_shrinks(
            spans in arb_spans(),
            rs in 0i64..2500,
            width in 1i64..400,
        ) {
            let re = rs + width;
            let (doc, seq_id, track_id) = track_fixture(&spans);
            let before_end = track_end(&doc, seq_id, track_id);
            let p = doc.timeline.as_ref().unwrap();
            let cmds = extract_edit(p, seq_id, track_id, (Tick(rs), Tick(re))).unwrap();
            let out = apply_batch(&doc, &cmds);
            let s = &out.timeline.as_ref().unwrap().sequences[&seq_id];
            prop_assert!(s.validate().is_ok(), "invariant broken: {:?}", s.validate());
            // With content strictly past the range, the gap closes by its width.
            if re < before_end {
                prop_assert_eq!(track_end(&out, seq_id, track_id), before_end - width);
            }
            assert_batch_undo_roundtrip(&doc, &cmds);
        }
    }

    // ── Sequence management (17 §G-17) ────────────────────────────────────

    #[test]
    fn rename_sequence_is_undo_idempotent() {
        let (doc, seq_id, _t, _c) = fixture();
        let p = doc.timeline.as_ref().unwrap();
        let cmd = rename_sequence(p, seq_id, "Act 1").unwrap();
        assert_undo_roundtrip(&doc, &cmd);

        let mut applied = doc.clone();
        Command::Timeline(cmd).apply(&mut applied);
        assert_eq!(
            applied.timeline.as_ref().unwrap().sequences[&seq_id].name,
            "Act 1"
        );
    }

    #[test]
    fn duplicate_sequence_copies_content_with_fresh_ids_and_is_undo_idempotent() {
        let (doc, seq_id, track_id, _c) = fixture();
        let orig_clip = the_track(&doc, seq_id, track_id).clips[0].id;
        let p = doc.timeline.as_ref().unwrap();
        let cmd = duplicate_sequence(p, seq_id).unwrap();
        assert_undo_roundtrip(&doc, &cmd);

        let mut applied = doc.clone();
        Command::Timeline(cmd).apply(&mut applied);
        let proj = applied.timeline.as_ref().unwrap();
        assert_eq!(proj.sequences.len(), 2);
        let dup = proj.sequences.values().find(|s| s.id != seq_id).unwrap();
        assert_eq!(dup.name, "Seq copy");
        // Same clip timing, fresh clip id.
        assert_eq!(
            dup.video_tracks[0]
                .clips
                .iter()
                .map(|c| (c.start.0, c.duration.0))
                .collect::<Vec<_>>(),
            spans_of(&applied, seq_id, track_id)
        );
        assert_ne!(dup.video_tracks[0].clips[0].id, orig_clip);
    }

    // ── Nested sequences (17 §G-16) ───────────────────────────────────────

    #[test]
    fn create_nested_sequence_wraps_selection_and_is_undo_idempotent() {
        let (doc, seq_id, track_id) = track_fixture(&[(0, 100), (100, 100), (200, 100)]);
        // Nest the first two clips → span [0,200).
        let ids: Vec<ClipId> = the_track(&doc, seq_id, track_id).clips[..2]
            .iter()
            .map(|c| c.id)
            .collect();
        let p = doc.timeline.as_ref().unwrap();
        let (inner_id, cmds) = create_nested_sequence(p, seq_id, track_id, &ids, "Nested").unwrap();
        let out = apply_batch(&doc, &cmds);
        validate_ok(&out, seq_id);

        // Outer: one NestedSequence clip [0,200) + the untouched [200,300).
        assert_eq!(spans_of(&out, seq_id, track_id), vec![(0, 200), (200, 100)]);
        let outer = the_track(&out, seq_id, track_id);
        assert!(matches!(
            outer.clips[0].source,
            ClipSource::NestedSequence { sequence } if sequence == inner_id
        ));
        // Inner: the two clips rebased to start at 0.
        let inner = &out.timeline.as_ref().unwrap().sequences[&inner_id];
        assert_eq!(
            inner.video_tracks[0]
                .clips
                .iter()
                .map(|c| (c.start.0, c.duration.0))
                .collect::<Vec<_>>(),
            vec![(0, 100), (100, 100)]
        );
        assert_batch_undo_roundtrip(&doc, &cmds);
    }

    #[test]
    fn create_nested_sequence_rejects_interior_nonselected_clip() {
        let (doc, seq_id, track_id) = track_fixture(&[(0, 100), (100, 100), (200, 100)]);
        // Select the outer two, leaving [100,200) inside the span.
        let clips = &the_track(&doc, seq_id, track_id).clips;
        let ids = vec![clips[0].id, clips[2].id];
        let p = doc.timeline.as_ref().unwrap();
        assert_eq!(
            create_nested_sequence(p, seq_id, track_id, &ids, "N").unwrap_err(),
            EditError::Overlap
        );
    }

    #[test]
    fn create_nested_sequence_rejects_empty_selection() {
        let (doc, seq_id, track_id) = track_fixture(&[(0, 100)]);
        let p = doc.timeline.as_ref().unwrap();
        assert_eq!(
            create_nested_sequence(p, seq_id, track_id, &[], "N").unwrap_err(),
            EditError::IndexOutOfRange
        );
    }

    #[test]
    fn nested_target_and_ancestry() {
        let (doc, seq_id, track_id) = track_fixture(&[(0, 100), (100, 100)]);
        let ids: Vec<ClipId> = the_track(&doc, seq_id, track_id)
            .clips
            .iter()
            .map(|c| c.id)
            .collect();
        let p = doc.timeline.as_ref().unwrap();
        let (inner_id, cmds) = create_nested_sequence(p, seq_id, track_id, &ids, "Nested").unwrap();
        let out = apply_batch(&doc, &cmds);

        let nested_clip = &the_track(&out, seq_id, track_id).clips[0];
        assert_eq!(nested_target(nested_clip), Some(inner_id));
        let p2 = out.timeline.as_ref().unwrap();
        assert_eq!(sequence_ancestry(p2, inner_id), vec![seq_id, inner_id]);
        assert_eq!(sequence_ancestry(p2, seq_id), vec![seq_id]);
    }

    // ── Multicam (17 §G-20) ───────────────────────────────────────────────

    #[test]
    fn create_multicam_group_folds_angles_and_is_undo_idempotent() {
        let (mut doc, seq_id, v_track, primary) = fixture();
        let (a_track, angle_clip) = add_audio_clip(&mut doc, seq_id);
        let p = doc.timeline.as_ref().unwrap();
        let cmds =
            create_multicam_group(p, seq_id, v_track, primary, &[(a_track, angle_clip)]).unwrap();
        let out = apply_batch(&doc, &cmds);
        validate_ok(&out, seq_id);

        // Primary carries a two-angle group; the folded clip is gone.
        let pc = find_clip(&out, seq_id, v_track, primary);
        let group = pc.multicam.as_ref().unwrap();
        assert_eq!(group.angles.len(), 2);
        assert_eq!(group.active, 0);
        assert!(the_track(&out, seq_id, a_track).clips.is_empty());

        assert_batch_undo_roundtrip(&doc, &cmds);
    }

    #[test]
    fn set_multicam_active_angle_mirrors_source_and_is_undo_idempotent() {
        let (mut doc, seq_id, v_track, primary) = fixture();
        let (a_track, angle_clip) = add_audio_clip(&mut doc, seq_id);
        // Give the angle clip a distinct source so the mirror is observable.
        let asset = AssetId::new();
        {
            let proj = doc.timeline.as_mut().unwrap();
            let c = proj.sequences.get_mut(&seq_id).unwrap().audio_tracks[0]
                .clips
                .iter_mut()
                .find(|c| c.id == angle_clip)
                .unwrap();
            c.source = ClipSource::Asset { asset };
        }
        let p = doc.timeline.as_ref().unwrap();
        let cmds =
            create_multicam_group(p, seq_id, v_track, primary, &[(a_track, angle_clip)]).unwrap();
        let grouped = apply_batch(&doc, &cmds);

        // Cut to angle 1: the clip's source mirrors that angle.
        let p2 = grouped.timeline.as_ref().unwrap();
        let cut = set_multicam_active_angle(p2, seq_id, v_track, primary, 1).unwrap();
        assert_undo_roundtrip(&grouped, &cut);

        let mut applied = grouped.clone();
        Command::Timeline(cut).apply(&mut applied);
        let pc = find_clip(&applied, seq_id, v_track, primary);
        assert_eq!(pc.multicam.as_ref().unwrap().active, 1);
        assert!(matches!(pc.source, ClipSource::Asset { asset: a } if a == asset));
    }

    #[test]
    fn set_multicam_active_angle_errors_without_group() {
        let (doc, seq_id, v_track, primary) = fixture();
        let p = doc.timeline.as_ref().unwrap();
        assert_eq!(
            set_multicam_active_angle(p, seq_id, v_track, primary, 0).unwrap_err(),
            EditError::IndexOutOfRange
        );
    }

    #[test]
    fn set_asset_meta_fills_probe_and_hash_undoably() {
        use super::super::media::{AssetKind, MediaAsset, MediaProbe};
        use super::super::time::Tick;
        let mut doc = Document::new("t", 100.0, 100.0);
        let mut project = TimelineProject::new();
        let asset = MediaAsset::from_file(AssetKind::Video, "/tmp/clip.mp4");
        let id = asset.id;
        project.media.insert(asset);
        doc.timeline = Some(project);

        let probe = MediaProbe {
            duration: Tick(1_000_000),
            video: None,
            audio: None,
            container: "mp4".into(),
            codec: "h264".into(),
        };
        let p = doc.timeline.as_ref().unwrap();
        let cmd = set_asset_meta(
            p,
            id,
            Some(probe.clone()),
            Some("hash-abc".into()),
        )
        .unwrap();
        assert_undo_roundtrip(&doc, &cmd);
        Command::Timeline(cmd).apply(&mut doc);
        let a = doc.timeline.as_ref().unwrap().media.assets.get(&id).unwrap();
        assert_eq!(a.probe.as_ref().map(|p| p.codec.as_str()), Some("h264"));
        assert_eq!(a.content_hash.as_deref(), Some("hash-abc"));
    }
}
