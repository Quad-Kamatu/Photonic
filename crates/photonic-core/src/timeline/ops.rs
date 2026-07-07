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
use super::clip::{Clip, ClipEffect, ClipSource};
use super::commands::{
    AnimTarget, AudioCmd, ClipTiming, FormatOp, FxOwner, TimelineCmd, TrackSettings,
};
use super::grade::Grade;
use super::graph::NodeGraph;
use super::ids::*;
use super::sequence::{Sequence, SequenceFormat, TimelineProject, Track, TrackKind};
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

pub fn move_clip(
    p: &TimelineProject,
    id: SequenceId,
    track_id: TrackId,
    clip_id: ClipId,
    new_start: Tick,
) -> Result<TimelineCmd, EditError> {
    if new_start.0 < 0 {
        return Err(EditError::Overlap);
    }
    let s = seq(p, id)?;
    let t = track(s, track_id)?;
    let c = clip(t, clip_id)?;
    if overlaps_other(t, new_start, new_start + c.duration, Some(clip_id)) {
        return Err(EditError::Overlap);
    }
    Ok(TimelineCmd::MoveClip {
        seq: id,
        track: track_id,
        clip: clip_id,
        old_start: c.start,
        new_start,
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
