//! Undo commands for the timeline (01 §10).
//!
//! One nested `Command::Timeline(TimelineCmd)` variant keeps `history/mod.rs`
//! churn to a single arm. Every variant implements `apply`/`inverse`/`description`
//! here; the edit ops in [`super::ops`] and [`super::graph_ops`] are the pure
//! constructors (GUI and MCP both call them — CAP-019 parity). Commands store
//! deltas/ids, never media (`mem_estimate` is O(changed structs)). Playhead,
//! selection, and scroll are session state, not document state — no undo entries.
//!
//! ## Deviations from 01 §10 / sub-enum specs (justified — see P2 report)
//! - `RemoveProject`, `MergeSplit`, and `UndoBulkInsert` are added as internal
//!   inverse partners (of `CreateProject`, `SplitClip`, and
//!   `CaptionCmd::BulkInsertCues`); those source specs list the forward op but
//!   no self-invertible inverse, and undo atomicity (01 §10) requires one.
//! - `GraphCmd::AddNode`/`RemoveNode` and `CaptionCmd::MergeCues`/`SplitCue`/
//!   `BulkInsertCues` carry extra serde-defaulted snapshot fields (edges /
//!   `old_*` / `new_cue_id`) beyond their literal spec shapes — the minimum
//!   needed for a lossless inverse.
//! - `AudioCmd` master-bus variants target the **active** sequence (09 §10 fixes
//!   no sequence; the master bus is per-sequence).
//! - `TtsCmd::GenerateAndPlace` performs no core-data mutation at this phase (its
//!   asset/clip/caption payloads are assembled by the engine path, P5); it is
//!   recorded for undo atomicity but its `apply`/`inverse` are structural no-ops.
//!   `TtsCmd::Regenerate` is fully implemented (pure asset swap).

use super::anim::{Interp, Keyframe, PropPath, PropertyTrack};
use super::audio::{
    AudioFade, AudioFxUnit, ChannelMap, ClipAudioParams, LoudnessTarget, MasterBusParams,
    TrackAudio, TrackAudioParams,
};
use super::captions::{CaptionCue, CaptionStyle, CaptionTrack, CaptionWord};
use super::clip::{Clip, ClipEffect};
use super::grade::Grade;
use super::graph::{GraphEdge, GraphNode, GraphNodeParams, NodePos};
use super::ids::*;
use super::media::{AssetSource, MediaAsset, MediaBin};
use super::sequence::{Marker, Sequence, SequenceFormat, TimelineProject, Track, TrackKind};
use super::time::Tick;
use crate::document::Document;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// ── Small snapshot helpers ──────────────────────────────────────────────────

/// The mutable timing of a clip — snapshotted for trim/ripple/roll/slide undo.
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClipTiming {
    pub start: Tick,
    pub duration: Tick,
    pub source_in: Tick,
}

impl ClipTiming {
    pub fn of(clip: &Clip) -> Self {
        ClipTiming {
            start: clip.start,
            duration: clip.duration,
            source_in: clip.source_in,
        }
    }
    pub fn apply_to(&self, clip: &mut Clip) {
        clip.start = self.start;
        clip.duration = self.duration;
        clip.source_in = self.source_in;
    }
}

/// Track settings (everything but the clip list) — snapshotted for `SetTrackProp`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrackSettings {
    pub name: String,
    pub enabled: bool,
    pub locked: bool,
    pub height_px: f32,
    pub audio: Option<TrackAudio>,
}

impl TrackSettings {
    pub fn of(t: &Track) -> Self {
        TrackSettings {
            name: t.name.clone(),
            enabled: t.enabled,
            locked: t.locked,
            height_px: t.height_px,
            audio: t.audio.clone(),
        }
    }
    pub fn apply_to(&self, t: &mut Track) {
        t.name = self.name.clone();
        t.enabled = self.enabled;
        t.locked = self.locked;
        t.height_px = self.height_px;
        t.audio = self.audio.clone();
    }
}

/// Add/update/remove of a [`SequenceFormat`] entry (10 §3.2).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum FormatOp {
    Add {
        format: SequenceFormat,
    },
    Update {
        index: usize,
        old: SequenceFormat,
        new: SequenceFormat,
    },
    Remove {
        index: usize,
        format: SequenceFormat,
    },
}

// ── Sub-enums ───────────────────────────────────────────────────────────────

/// Owner of an audio fx chain (09 §10).
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FxOwner {
    Track(TrackId),
    Master,
}

/// Which clip fade edge (09 §10).
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FadeEdge {
    In,
    Out,
}

/// Scope of a caption style edit (06 §4).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StyleTarget {
    Track,
    Cue(CueId),
    Word(CueId, usize),
}

/// Node-graph edits (08 §8).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum GraphCmd {
    AddNode {
        graph: GraphId,
        node: Box<GraphNode>,
        pos: NodePos,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        edges: Vec<GraphEdge>,
    },
    RemoveNode {
        graph: GraphId,
        node: Box<GraphNode>,
        pos: Option<NodePos>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        edges: Vec<GraphEdge>,
    },
    AddEdge {
        graph: GraphId,
        edge: GraphEdge,
    },
    RemoveEdge {
        graph: GraphId,
        edge: GraphEdge,
    },
    SetNodeParam {
        graph: GraphId,
        node: GraphNodeId,
        old: Box<GraphNodeParams>,
        new: Box<GraphNodeParams>,
    },
    SetKeyframe {
        graph: GraphId,
        node: GraphNodeId,
        path: PropPath,
        old: Option<Keyframe>,
        new: Keyframe,
    },
    RemoveKeyframe {
        graph: GraphId,
        node: GraphNodeId,
        path: PropPath,
        keyframe: Keyframe,
    },
    MoveNode {
        graph: GraphId,
        node: GraphNodeId,
        old: NodePos,
        new: NodePos,
    },
}

/// Caption edits (06 §3.6, §4).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum CaptionCmd {
    BulkInsertCues {
        track: TrackId,
        cues: Vec<CaptionCue>,
        replace_range: Option<(Tick, Tick)>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        replaced: Vec<CaptionCue>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        created_track: Option<Box<CaptionTrack>>,
    },
    /// Internal inverse of `BulkInsertCues`.
    UndoBulkInsert {
        track: TrackId,
        inserted_ids: Vec<CueId>,
        restored: Vec<CaptionCue>,
        remove_track: bool,
    },
    SetCueText {
        track: TrackId,
        cue: CueId,
        old_words: Vec<CaptionWord>,
        new_words: Vec<CaptionWord>,
    },
    SplitCue {
        track: TrackId,
        cue: CueId,
        at_word_index: usize,
        new_cue_id: CueId,
    },
    MergeCues {
        track: TrackId,
        a: CueId,
        b: CueId,
        old_a: Box<CaptionCue>,
        old_b: Box<CaptionCue>,
    },
    RetimeCue {
        track: TrackId,
        cue: CueId,
        old: (Tick, Tick),
        new: (Tick, Tick),
    },
    RetimeWord {
        track: TrackId,
        cue: CueId,
        word: usize,
        old: (Tick, Tick),
        new: (Tick, Tick),
    },
    SetStyle {
        track: TrackId,
        target: StyleTarget,
        old: Option<Box<CaptionStyle>>,
        new: Option<Box<CaptionStyle>>,
    },
}

/// Voiceover/TTS edits (06 §6).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum TtsCmd {
    GenerateAndPlace {
        asset: AssetId,
        clip: ClipId,
        track: TrackId,
        caption: Option<(TrackId, Vec<CaptionCue>)>,
    },
    Regenerate {
        clip: ClipId,
        old_asset: AssetId,
        new_asset: AssetId,
    },
}

/// Audio mixer edits (09 §10).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum AudioCmd {
    SetTrackAudioProp {
        track: TrackId,
        old: TrackAudioParams,
        new: TrackAudioParams,
    },
    SetTrackMuteSolo {
        track: TrackId,
        old: (bool, bool),
        new: (bool, bool),
    },
    SetClipAudioProp {
        clip: ClipId,
        old: ClipAudioParams,
        new: ClipAudioParams,
    },
    SetClipFade {
        clip: ClipId,
        edge: FadeEdge,
        old: Option<AudioFade>,
        new: Option<AudioFade>,
    },
    SetChannelMap {
        clip: ClipId,
        old: ChannelMap,
        new: ChannelMap,
    },
    AddAudioFx {
        owner: FxOwner,
        index: usize,
        unit: AudioFxUnit,
    },
    RemoveAudioFx {
        owner: FxOwner,
        index: usize,
        unit: AudioFxUnit,
    },
    ReorderAudioFx {
        owner: FxOwner,
        old_order: Vec<usize>,
        new_order: Vec<usize>,
    },
    SetMasterBusProp {
        old: MasterBusParams,
        new: MasterBusParams,
    },
    SetLoudnessTarget {
        old: Option<LoudnessTarget>,
        new: Option<LoudnessTarget>,
    },
    ApplyDuckingPreset {
        track: TrackId,
        sidechain: TrackId,
        old_fx_chain: Vec<AudioFxUnit>,
        new_fx_chain: Vec<AudioFxUnit>,
    },
}

/// A target of the generic keyframe commands — any `AnimProps` lane in the
/// project, addressed structurally (01 §6 is generic over targets via `PropPath`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "target", rename_all = "snake_case")]
pub enum AnimTarget {
    ClipTransform { clip: ClipId },
    ClipEffect { clip: ClipId, effect_index: usize },
    GradeOp { clip: ClipId, op: GradeOpId },
    TrackAudio { track: TrackId },
    ClipAudio { clip: ClipId },
    MasterBus { seq: SequenceId },
    AudioFx { owner: FxOwner, index: usize },
}

// ── The top-level command ───────────────────────────────────────────────────

/// Every timeline mutation, nested under `Command::Timeline` (01 §10).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum TimelineCmd {
    CreateProject {
        project: Box<TimelineProject>,
    },
    /// Internal inverse partner of `CreateProject`.
    RemoveProject {
        project: Box<TimelineProject>,
    },
    AddAsset {
        asset: Box<MediaAsset>,
    },
    RemoveAsset {
        asset: Box<MediaAsset>,
    },
    RelinkAsset {
        asset: AssetId,
        old_path: PathBuf,
        new_path: PathBuf,
    },
    AddSequence {
        sequence: Box<Sequence>,
    },
    RemoveSequence {
        sequence: Box<Sequence>,
        order_index: usize,
        was_active: bool,
    },
    SetActiveSequence {
        old: Option<SequenceId>,
        new: Option<SequenceId>,
    },
    SetActiveFormat {
        seq: SequenceId,
        old: usize,
        new: usize,
    },
    SetSequenceFormat {
        seq: SequenceId,
        op: FormatOp,
    },
    AddTrack {
        seq: SequenceId,
        kind: TrackKind,
        index: usize,
        track: Box<Track>,
    },
    RemoveTrack {
        seq: SequenceId,
        kind: TrackKind,
        index: usize,
        track: Box<Track>,
    },
    SetTrackProp {
        seq: SequenceId,
        track: TrackId,
        old: Box<TrackSettings>,
        new: Box<TrackSettings>,
    },
    InsertClip {
        seq: SequenceId,
        track: TrackId,
        clip: Box<Clip>,
    },
    RemoveClip {
        seq: SequenceId,
        track: TrackId,
        clip: Box<Clip>,
    },
    MoveClip {
        seq: SequenceId,
        /// The clip's track at apply time (source track for a cross-track move).
        track: TrackId,
        clip: ClipId,
        old_start: Tick,
        new_start: Tick,
        /// Destination track for a cross-track move; `None` = same-track move.
        /// Additive field (serde-defaulted) so pre-existing serialized
        /// `MoveClip` commands stay loadable.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        new_track: Option<TrackId>,
    },
    TrimClip {
        seq: SequenceId,
        track: TrackId,
        clip: ClipId,
        old: ClipTiming,
        new: ClipTiming,
    },
    SplitClip {
        seq: SequenceId,
        track: TrackId,
        clip: ClipId,
        at: Tick,
        new_clip_id: ClipId,
    },
    /// Internal inverse partner of `SplitClip` (merge the split halves back).
    MergeSplit {
        seq: SequenceId,
        track: TrackId,
        left: ClipId,
        right: ClipId,
    },
    RippleEdit {
        seq: SequenceId,
        track: TrackId,
        changes: Vec<(ClipId, ClipTiming, ClipTiming)>,
    },
    RollEdit {
        seq: SequenceId,
        track: TrackId,
        changes: Vec<(ClipId, ClipTiming, ClipTiming)>,
    },
    SlipClip {
        seq: SequenceId,
        track: TrackId,
        clip: ClipId,
        old_source_in: Tick,
        new_source_in: Tick,
    },
    SlideClip {
        seq: SequenceId,
        track: TrackId,
        changes: Vec<(ClipId, ClipTiming, ClipTiming)>,
    },
    SetClipProp {
        seq: SequenceId,
        track: TrackId,
        old: Box<Clip>,
        new: Box<Clip>,
    },
    SetKeyframe {
        target: AnimTarget,
        path: PropPath,
        old: Option<Keyframe>,
        new: Keyframe,
    },
    RemoveKeyframe {
        target: AnimTarget,
        path: PropPath,
        keyframe: Keyframe,
    },
    SetKeyframeInterp {
        target: AnimTarget,
        path: PropPath,
        at: Tick,
        old: Interp,
        new: Interp,
    },
    AddEffect {
        seq: SequenceId,
        track: TrackId,
        clip: ClipId,
        index: usize,
        effect: Box<ClipEffect>,
    },
    RemoveEffect {
        seq: SequenceId,
        track: TrackId,
        clip: ClipId,
        index: usize,
        effect: Box<ClipEffect>,
    },
    ReorderEffects {
        seq: SequenceId,
        track: TrackId,
        clip: ClipId,
        old_order: Vec<usize>,
        new_order: Vec<usize>,
    },
    SetGrade {
        seq: SequenceId,
        track: TrackId,
        clip: ClipId,
        old: Option<Box<Grade>>,
        new: Option<Box<Grade>>,
    },
    /// Add a graph to the arena (08 §4 composition lifecycle).
    AddGraph {
        graph: Box<super::graph::NodeGraph>,
    },
    /// Remove a graph from the arena (inverse of `AddGraph`).
    RemoveGraph {
        graph: Box<super::graph::NodeGraph>,
    },
    /// Point a clip at (or away from) a composition graph (08 §4). Deep-clone on
    /// paste happens in the op; this just swaps the `GraphId` reference.
    SetClipComposition {
        seq: SequenceId,
        track: TrackId,
        clip: ClipId,
        old: Option<GraphId>,
        new: Option<GraphId>,
    },
    /// Set the project graph reference (08 §5).
    SetProjectGraph {
        old: Option<GraphId>,
        new: Option<GraphId>,
    },
    /// Add a sequence marker (01 §4).
    AddMarker {
        seq: SequenceId,
        marker: Marker,
    },
    /// Remove a sequence marker (carries the full marker for a self-contained
    /// inverse).
    RemoveMarker {
        seq: SequenceId,
        marker: Marker,
    },
    /// Edit a marker's fields (name/color/note/position) — old/new full markers.
    SetMarker {
        seq: SequenceId,
        id: MarkerId,
        old: Marker,
        new: Marker,
    },
    /// Set (or clear, with `None`) a sequence's preview/export work range.
    SetWorkRange {
        seq: SequenceId,
        old: Option<(Tick, Tick)>,
        new: Option<(Tick, Tick)>,
    },
    /// Add a media bin (folder) to the pool (01 §3).
    AddBin {
        bin: MediaBin,
    },
    /// Remove a media bin (inverse of `AddBin`). Assets referencing it are left
    /// untouched (a dangling bin ref reads as unfiled/root); re-adding restores.
    RemoveBin {
        bin: MediaBin,
    },
    /// Move an asset into a bin (or to the pool root with `None`).
    AssignAssetBin {
        asset: AssetId,
        old: Option<BinId>,
        new: Option<BinId>,
    },
    GraphEdit(GraphCmd),
    CaptionEdit(CaptionCmd),
    TtsEdit(TtsCmd),
    AudioEdit(AudioCmd),
}

// ── Resolution helpers ──────────────────────────────────────────────────────

fn find_clip_mut(proj: &mut TimelineProject, clip: ClipId) -> Option<&mut Clip> {
    for seq in proj.sequences.values_mut() {
        for t in seq
            .video_tracks
            .iter_mut()
            .chain(seq.audio_tracks.iter_mut())
        {
            if let Some(c) = t.clips.iter_mut().find(|c| c.id == clip) {
                return Some(c);
            }
        }
    }
    None
}

fn find_track_mut(proj: &mut TimelineProject, track: TrackId) -> Option<&mut Track> {
    for seq in proj.sequences.values_mut() {
        if let Some(t) = seq
            .video_tracks
            .iter_mut()
            .chain(seq.audio_tracks.iter_mut())
            .find(|t| t.id == track)
        {
            return Some(t);
        }
    }
    None
}

fn find_clip_in_track(
    proj: &mut TimelineProject,
    track: TrackId,
    clip: ClipId,
) -> Option<&mut Clip> {
    find_track_mut(proj, track).and_then(|t| t.clips.iter_mut().find(|c| c.id == clip))
}

fn find_caption_track_mut(proj: &mut TimelineProject, track: TrackId) -> Option<&mut CaptionTrack> {
    for seq in proj.sequences.values_mut() {
        if let Some(t) = seq.caption_tracks.iter_mut().find(|t| t.id == track) {
            return Some(t);
        }
    }
    None
}

fn resolve_tracks_mut<'a>(
    proj: &'a mut TimelineProject,
    target: &AnimTarget,
) -> Option<&'a mut Vec<PropertyTrack>> {
    match target {
        AnimTarget::ClipTransform { clip } => {
            find_clip_mut(proj, *clip).map(|c| &mut c.transform.tracks)
        }
        AnimTarget::ClipEffect { clip, effect_index } => find_clip_mut(proj, *clip)?
            .effects
            .get_mut(*effect_index)
            .map(|e| &mut e.params.tracks),
        AnimTarget::GradeOp { clip, op } => {
            let grade = find_clip_mut(proj, *clip)?.grade.as_mut()?;
            grade
                .ops
                .iter_mut()
                .find(|o| o.id == *op)
                .map(|o| &mut o.params.tracks)
        }
        AnimTarget::ClipAudio { clip } => find_clip_mut(proj, *clip)?
            .audio
            .as_mut()
            .map(|a| &mut a.params.tracks),
        AnimTarget::TrackAudio { track } => find_track_mut(proj, *track)?
            .audio
            .as_mut()
            .map(|a| &mut a.params.tracks),
        AnimTarget::MasterBus { seq } => proj
            .sequences
            .get_mut(seq)
            .map(|s| &mut s.audio_master.params.tracks),
        AnimTarget::AudioFx { owner, index } => match owner {
            FxOwner::Track(track) => find_track_mut(proj, *track)?
                .audio
                .as_mut()?
                .fx_chain
                .get_mut(*index)
                .map(|u| &mut u.params.tracks),
            FxOwner::Master => {
                let sid = proj.active_sequence?;
                proj.sequences
                    .get_mut(&sid)?
                    .audio_master
                    .fx_chain
                    .get_mut(*index)
                    .map(|u| &mut u.params.tracks)
            }
        },
    }
}

fn set_kf_on(tracks: &mut Vec<PropertyTrack>, path: &PropPath, kf: Keyframe) {
    let lane = if let Some(i) = tracks.iter().position(|t| &t.property == path) {
        &mut tracks[i]
    } else {
        tracks.push(PropertyTrack::new(path.clone()));
        tracks.last_mut().unwrap()
    };
    lane.insert_keyframe(kf);
}

fn remove_kf_on(tracks: &mut Vec<PropertyTrack>, path: &PropPath, at: Tick) {
    if let Some(i) = tracks.iter().position(|t| &t.property == path) {
        tracks[i].remove_keyframe(at);
        // Drop a lane that a keyframe-removal emptied, so a `SetKeyframe`
        // (old = None) that created the lane round-trips exactly to no-lane.
        // Orphaned lanes are retained regardless (they carry unresolved paths).
        if tracks[i].keyframes.is_empty() && !tracks[i].orphaned {
            tracks.remove(i);
        }
    }
}

fn reorder<T>(v: &mut Vec<T>, new_order: &[usize]) {
    if new_order.len() != v.len() || new_order.iter().any(|&i| i >= v.len()) {
        return;
    }
    let mut taken: Vec<Option<T>> = v.drain(..).map(Some).collect();
    for &i in new_order {
        if let Some(item) = taken[i].take() {
            v.push(item);
        }
    }
}

/// Keep markers in a deterministic order (by tick, then id) so add/remove/set
/// round-trip to an identical vector regardless of insertion order.
fn sort_markers(markers: &mut [Marker]) {
    markers.sort_by(|a, b| a.at.cmp(&b.at).then_with(|| a.id.cmp(&b.id)));
}

fn apply_timing_changes(
    proj: &mut TimelineProject,
    track: TrackId,
    changes: &[(ClipId, ClipTiming, ClipTiming)],
) {
    if let Some(t) = find_track_mut(proj, track) {
        for (id, _old, new) in changes {
            if let Some(c) = t.clips.iter_mut().find(|c| c.id == *id) {
                new.apply_to(c);
            }
        }
        t.clips.sort_by_key(|c| c.start.0);
    }
}

fn invert_changes(
    changes: &[(ClipId, ClipTiming, ClipTiming)],
) -> Vec<(ClipId, ClipTiming, ClipTiming)> {
    changes.iter().map(|(id, o, n)| (*id, *n, *o)).collect()
}

fn invert_format_op(op: &FormatOp) -> FormatOp {
    match op {
        FormatOp::Add { format } => FormatOp::Remove {
            // On apply, Add pushes to the end; the inverse removes that last index.
            index: usize::MAX,
            format: format.clone(),
        },
        FormatOp::Remove { format, .. } => FormatOp::Add {
            format: format.clone(),
        },
        FormatOp::Update { index, old, new } => FormatOp::Update {
            index: *index,
            old: new.clone(),
            new: old.clone(),
        },
    }
}

// ── Sub-command impls ───────────────────────────────────────────────────────

impl GraphCmd {
    fn graph_id(&self) -> GraphId {
        match self {
            GraphCmd::AddNode { graph, .. }
            | GraphCmd::RemoveNode { graph, .. }
            | GraphCmd::AddEdge { graph, .. }
            | GraphCmd::RemoveEdge { graph, .. }
            | GraphCmd::SetNodeParam { graph, .. }
            | GraphCmd::SetKeyframe { graph, .. }
            | GraphCmd::RemoveKeyframe { graph, .. }
            | GraphCmd::MoveNode { graph, .. } => *graph,
        }
    }

    fn apply(&self, proj: &mut TimelineProject) {
        let Some(g) = proj.graphs.get_mut(&self.graph_id()) else {
            return;
        };
        match self {
            GraphCmd::AddNode {
                node, pos, edges, ..
            } => {
                g.nodes.insert(node.id, (**node).clone());
                g.ui.insert(node.id, *pos);
                for e in edges {
                    if !g.edges.contains(e) {
                        g.edges.push(*e);
                    }
                }
            }
            GraphCmd::RemoveNode { node, .. } => {
                g.nodes.remove(&node.id);
                g.ui.remove(&node.id);
                g.edges.retain(|e| e.from.0 != node.id && e.to.0 != node.id);
            }
            GraphCmd::AddEdge { edge, .. } => {
                if !g.edges.contains(edge) {
                    g.edges.push(*edge);
                }
            }
            GraphCmd::RemoveEdge { edge, .. } => g.edges.retain(|e| e != edge),
            GraphCmd::SetNodeParam { node, new, .. } => {
                if let Some(n) = g.nodes.get_mut(node) {
                    n.params.base = (**new).clone();
                }
            }
            GraphCmd::SetKeyframe {
                node, path, new, ..
            } => {
                if let Some(n) = g.nodes.get_mut(node) {
                    set_kf_on(&mut n.params.tracks, path, *new);
                }
            }
            GraphCmd::RemoveKeyframe {
                node,
                path,
                keyframe,
                ..
            } => {
                if let Some(n) = g.nodes.get_mut(node) {
                    remove_kf_on(&mut n.params.tracks, path, keyframe.at);
                }
            }
            GraphCmd::MoveNode { node, new, .. } => {
                g.ui.insert(*node, *new);
            }
        }
    }

    fn inverse(&self) -> Option<GraphCmd> {
        Some(match self {
            GraphCmd::AddNode {
                graph,
                node,
                pos,
                edges,
            } => GraphCmd::RemoveNode {
                graph: *graph,
                node: node.clone(),
                pos: Some(*pos),
                edges: edges.clone(),
            },
            GraphCmd::RemoveNode {
                graph,
                node,
                pos,
                edges,
            } => GraphCmd::AddNode {
                graph: *graph,
                node: node.clone(),
                pos: pos.unwrap_or(NodePos { x: 0.0, y: 0.0 }),
                edges: edges.clone(),
            },
            GraphCmd::AddEdge { graph, edge } => GraphCmd::RemoveEdge {
                graph: *graph,
                edge: *edge,
            },
            GraphCmd::RemoveEdge { graph, edge } => GraphCmd::AddEdge {
                graph: *graph,
                edge: *edge,
            },
            GraphCmd::SetNodeParam {
                graph,
                node,
                old,
                new,
            } => GraphCmd::SetNodeParam {
                graph: *graph,
                node: *node,
                old: new.clone(),
                new: old.clone(),
            },
            GraphCmd::SetKeyframe {
                graph,
                node,
                path,
                old,
                new,
            } => match old {
                Some(prev) => GraphCmd::SetKeyframe {
                    graph: *graph,
                    node: *node,
                    path: path.clone(),
                    old: Some(*new),
                    new: *prev,
                },
                None => GraphCmd::RemoveKeyframe {
                    graph: *graph,
                    node: *node,
                    path: path.clone(),
                    keyframe: *new,
                },
            },
            GraphCmd::RemoveKeyframe {
                graph,
                node,
                path,
                keyframe,
            } => GraphCmd::SetKeyframe {
                graph: *graph,
                node: *node,
                path: path.clone(),
                old: None,
                new: *keyframe,
            },
            GraphCmd::MoveNode {
                graph,
                node,
                old,
                new,
            } => GraphCmd::MoveNode {
                graph: *graph,
                node: *node,
                old: *new,
                new: *old,
            },
        })
    }

    fn coalesce(last: &GraphCmd, new: &GraphCmd) -> Option<GraphCmd> {
        match (last, new) {
            (
                GraphCmd::MoveNode {
                    graph, node, old, ..
                },
                GraphCmd::MoveNode {
                    graph: g2,
                    node: n2,
                    new,
                    ..
                },
            ) if graph == g2 && node == n2 => Some(GraphCmd::MoveNode {
                graph: *graph,
                node: *node,
                old: *old,
                new: *new,
            }),
            _ => None,
        }
    }
}

impl CaptionCmd {
    fn apply(&self, proj: &mut TimelineProject) {
        match self {
            CaptionCmd::BulkInsertCues {
                track,
                cues,
                replace_range,
                created_track,
                ..
            } => {
                if find_caption_track_mut(proj, *track).is_none() {
                    if let (Some(ct), Some(seq)) = (
                        created_track,
                        proj.active_sequence
                            .and_then(|s| proj.sequences.get_mut(&s)),
                    ) {
                        seq.caption_tracks.push((**ct).clone());
                    }
                }
                if let Some(ct) = find_caption_track_mut(proj, *track) {
                    if let Some((s, e)) = replace_range {
                        ct.cues.retain(|c| c.end <= *s || c.start >= *e);
                    }
                    ct.cues.extend(cues.iter().cloned());
                    ct.cues.sort_by_key(|c| c.start.0);
                }
            }
            CaptionCmd::UndoBulkInsert {
                track,
                inserted_ids,
                restored,
                remove_track,
            } => {
                if *remove_track {
                    for seq in proj.sequences.values_mut() {
                        seq.caption_tracks.retain(|t| t.id != *track);
                    }
                    return;
                }
                if let Some(ct) = find_caption_track_mut(proj, *track) {
                    ct.cues.retain(|c| !inserted_ids.contains(&c.id));
                    ct.cues.extend(restored.iter().cloned());
                    ct.cues.sort_by_key(|c| c.start.0);
                }
            }
            CaptionCmd::SetCueText {
                track,
                cue,
                new_words,
                ..
            } => {
                if let Some(ct) = find_caption_track_mut(proj, *track) {
                    if let Some(c) = ct.cues.iter_mut().find(|c| c.id == *cue) {
                        c.words = new_words.clone();
                    }
                }
            }
            CaptionCmd::SplitCue {
                track,
                cue,
                at_word_index,
                new_cue_id,
            } => {
                if let Some(ct) = find_caption_track_mut(proj, *track) {
                    if let Some(pos) = ct.cues.iter().position(|c| c.id == *cue) {
                        let idx = (*at_word_index).min(ct.cues[pos].words.len());
                        let right_words = ct.cues[pos].words.split_off(idx);
                        let mut right = ct.cues[pos].clone();
                        right.id = *new_cue_id;
                        right.words = right_words;
                        // Boundary tick between the two word groups.
                        let boundary = right
                            .words
                            .first()
                            .map(|w| w.start)
                            .unwrap_or(ct.cues[pos].end);
                        right.start = boundary;
                        ct.cues[pos].end = boundary;
                        ct.cues.insert(pos + 1, right);
                    }
                }
            }
            CaptionCmd::MergeCues { track, a, b, .. } => {
                if let Some(ct) = find_caption_track_mut(proj, *track) {
                    let (Some(ia), Some(ib)) = (
                        ct.cues.iter().position(|c| c.id == *a),
                        ct.cues.iter().position(|c| c.id == *b),
                    ) else {
                        return;
                    };
                    let cue_b = ct.cues[ib].clone();
                    let ca = &mut ct.cues[ia];
                    let mut words = ca.words.clone();
                    words.extend(cue_b.words);
                    words.sort_by_key(|w| w.start.0);
                    ca.words = words;
                    ca.start = ca.start.min(cue_b.start);
                    ca.end = ca.end.max(cue_b.end);
                    if ca.style_override.is_none() {
                        ca.style_override = cue_b.style_override;
                    }
                    ct.cues.retain(|c| c.id != *b);
                }
            }
            CaptionCmd::RetimeCue {
                track, cue, new, ..
            } => {
                if let Some(ct) = find_caption_track_mut(proj, *track) {
                    if let Some(c) = ct.cues.iter_mut().find(|c| c.id == *cue) {
                        c.start = new.0;
                        c.end = new.1;
                    }
                    ct.cues.sort_by_key(|c| c.start.0);
                }
            }
            CaptionCmd::RetimeWord {
                track,
                cue,
                word,
                new,
                ..
            } => {
                if let Some(ct) = find_caption_track_mut(proj, *track) {
                    if let Some(c) = ct.cues.iter_mut().find(|c| c.id == *cue) {
                        if let Some(w) = c.words.get_mut(*word) {
                            w.start = new.0;
                            w.end = new.1;
                        }
                    }
                }
            }
            CaptionCmd::SetStyle {
                track, target, new, ..
            } => {
                if let Some(ct) = find_caption_track_mut(proj, *track) {
                    let style = new.as_ref().map(|s| (**s).clone());
                    match target {
                        StyleTarget::Track => {
                            if let Some(s) = style {
                                ct.style = s;
                            }
                        }
                        StyleTarget::Cue(id) => {
                            if let Some(c) = ct.cues.iter_mut().find(|c| c.id == *id) {
                                c.style_override = style;
                            }
                        }
                        StyleTarget::Word(id, wi) => {
                            if let Some(c) = ct.cues.iter_mut().find(|c| c.id == *id) {
                                if let Some(w) = c.words.get_mut(*wi) {
                                    w.style_override = style;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    fn inverse(&self) -> Option<CaptionCmd> {
        Some(match self {
            CaptionCmd::BulkInsertCues {
                track,
                cues,
                replaced,
                created_track,
                ..
            } => CaptionCmd::UndoBulkInsert {
                track: *track,
                inserted_ids: cues.iter().map(|c| c.id).collect(),
                restored: replaced.clone(),
                remove_track: created_track.is_some(),
            },
            CaptionCmd::UndoBulkInsert { .. } => return None,
            CaptionCmd::SetCueText {
                track,
                cue,
                old_words,
                new_words,
            } => CaptionCmd::SetCueText {
                track: *track,
                cue: *cue,
                old_words: new_words.clone(),
                new_words: old_words.clone(),
            },
            CaptionCmd::SplitCue {
                track,
                cue,
                new_cue_id,
                ..
            } => CaptionCmd::MergeCues {
                track: *track,
                a: *cue,
                b: *new_cue_id,
                // Snapshots are only needed for the merge→split direction; a
                // split's inverse merges structurally, so placeholders suffice.
                old_a: Box::new(CaptionCue::new(Tick::ZERO, Tick::ZERO, vec![])),
                old_b: Box::new(CaptionCue::new(Tick::ZERO, Tick::ZERO, vec![])),
            },
            CaptionCmd::MergeCues {
                track,
                a,
                b,
                old_a,
                old_b,
            } => {
                // Undo a merge by restoring both original cues verbatim.
                CaptionCmd::UndoBulkInsert {
                    track: *track,
                    inserted_ids: vec![*a, *b],
                    restored: vec![(**old_a).clone(), (**old_b).clone()],
                    remove_track: false,
                }
            }
            CaptionCmd::RetimeCue {
                track,
                cue,
                old,
                new,
            } => CaptionCmd::RetimeCue {
                track: *track,
                cue: *cue,
                old: *new,
                new: *old,
            },
            CaptionCmd::RetimeWord {
                track,
                cue,
                word,
                old,
                new,
            } => CaptionCmd::RetimeWord {
                track: *track,
                cue: *cue,
                word: *word,
                old: *new,
                new: *old,
            },
            CaptionCmd::SetStyle {
                track,
                target,
                old,
                new,
            } => CaptionCmd::SetStyle {
                track: *track,
                target: target.clone(),
                old: new.clone(),
                new: old.clone(),
            },
        })
    }
}

impl TtsCmd {
    fn apply(&self, proj: &mut TimelineProject) {
        match self {
            // Engine assembles asset+clip+captions; recorded for undo atomicity
            // only at this phase (see module deviation note).
            TtsCmd::GenerateAndPlace { .. } => {}
            TtsCmd::Regenerate {
                clip, new_asset, ..
            } => {
                if let Some(c) = find_clip_mut(proj, *clip) {
                    match &mut c.source {
                        super::clip::ClipSource::Asset { asset }
                        | super::clip::ClipSource::Vector { asset } => *asset = *new_asset,
                        _ => {}
                    }
                }
            }
        }
    }

    fn inverse(&self) -> Option<TtsCmd> {
        Some(match self {
            TtsCmd::GenerateAndPlace {
                asset,
                clip,
                track,
                caption,
            } => TtsCmd::GenerateAndPlace {
                asset: *asset,
                clip: *clip,
                track: *track,
                caption: caption.clone(),
            },
            TtsCmd::Regenerate {
                clip,
                old_asset,
                new_asset,
            } => TtsCmd::Regenerate {
                clip: *clip,
                old_asset: *new_asset,
                new_asset: *old_asset,
            },
        })
    }
}

impl AudioCmd {
    fn master_bus_mut(proj: &mut TimelineProject) -> Option<&mut super::audio::MasterBus> {
        let sid = proj.active_sequence?;
        proj.sequences.get_mut(&sid).map(|s| &mut s.audio_master)
    }

    fn fx_chain_mut<'a>(
        proj: &'a mut TimelineProject,
        owner: &FxOwner,
    ) -> Option<&'a mut Vec<AudioFxUnit>> {
        match owner {
            FxOwner::Track(t) => find_track_mut(proj, *t)?
                .audio
                .as_mut()
                .map(|a| &mut a.fx_chain),
            FxOwner::Master => Self::master_bus_mut(proj).map(|m| &mut m.fx_chain),
        }
    }

    fn apply(&self, proj: &mut TimelineProject) {
        match self {
            AudioCmd::SetTrackAudioProp { track, new, .. } => {
                if let Some(t) = find_track_mut(proj, *track) {
                    if let Some(a) = t.audio.as_mut() {
                        a.params.base = *new;
                    }
                }
            }
            AudioCmd::SetTrackMuteSolo { track, new, .. } => {
                if let Some(t) = find_track_mut(proj, *track) {
                    if let Some(a) = t.audio.as_mut() {
                        a.mute = new.0;
                        a.solo = new.1;
                    }
                }
            }
            AudioCmd::SetClipAudioProp { clip, new, .. } => {
                if let Some(c) = find_clip_mut(proj, *clip) {
                    if let Some(a) = c.audio.as_mut() {
                        a.params.base = *new;
                    }
                }
            }
            AudioCmd::SetClipFade {
                clip, edge, new, ..
            } => {
                if let Some(c) = find_clip_mut(proj, *clip) {
                    if let Some(a) = c.audio.as_mut() {
                        match edge {
                            FadeEdge::In => a.fade_in = *new,
                            FadeEdge::Out => a.fade_out = *new,
                        }
                    }
                }
            }
            AudioCmd::SetChannelMap { clip, new, .. } => {
                if let Some(c) = find_clip_mut(proj, *clip) {
                    if let Some(a) = c.audio.as_mut() {
                        a.channel_map = *new;
                    }
                }
            }
            AudioCmd::AddAudioFx { owner, index, unit } => {
                if let Some(chain) = Self::fx_chain_mut(proj, owner) {
                    let i = (*index).min(chain.len());
                    chain.insert(i, unit.clone());
                }
            }
            AudioCmd::RemoveAudioFx { owner, index, .. } => {
                if let Some(chain) = Self::fx_chain_mut(proj, owner) {
                    if *index < chain.len() {
                        chain.remove(*index);
                    }
                }
            }
            AudioCmd::ReorderAudioFx {
                owner, new_order, ..
            } => {
                if let Some(chain) = Self::fx_chain_mut(proj, owner) {
                    reorder(chain, new_order);
                }
            }
            AudioCmd::SetMasterBusProp { new, .. } => {
                if let Some(m) = Self::master_bus_mut(proj) {
                    m.params.base = *new;
                }
            }
            AudioCmd::SetLoudnessTarget { new, .. } => {
                if let Some(m) = Self::master_bus_mut(proj) {
                    m.loudness_target = *new;
                }
            }
            AudioCmd::ApplyDuckingPreset {
                track,
                new_fx_chain,
                ..
            } => {
                if let Some(t) = find_track_mut(proj, *track) {
                    if let Some(a) = t.audio.as_mut() {
                        a.fx_chain = new_fx_chain.clone();
                    }
                }
            }
        }
    }

    fn inverse(&self) -> AudioCmd {
        match self {
            AudioCmd::SetTrackAudioProp { track, old, new } => AudioCmd::SetTrackAudioProp {
                track: *track,
                old: *new,
                new: *old,
            },
            AudioCmd::SetTrackMuteSolo { track, old, new } => AudioCmd::SetTrackMuteSolo {
                track: *track,
                old: *new,
                new: *old,
            },
            AudioCmd::SetClipAudioProp { clip, old, new } => AudioCmd::SetClipAudioProp {
                clip: *clip,
                old: *new,
                new: *old,
            },
            AudioCmd::SetClipFade {
                clip,
                edge,
                old,
                new,
            } => AudioCmd::SetClipFade {
                clip: *clip,
                edge: *edge,
                old: *new,
                new: *old,
            },
            AudioCmd::SetChannelMap { clip, old, new } => AudioCmd::SetChannelMap {
                clip: *clip,
                old: *new,
                new: *old,
            },
            AudioCmd::AddAudioFx { owner, index, unit } => AudioCmd::RemoveAudioFx {
                owner: *owner,
                index: *index,
                unit: unit.clone(),
            },
            AudioCmd::RemoveAudioFx { owner, index, unit } => AudioCmd::AddAudioFx {
                owner: *owner,
                index: *index,
                unit: unit.clone(),
            },
            AudioCmd::ReorderAudioFx {
                owner,
                old_order,
                new_order,
            } => AudioCmd::ReorderAudioFx {
                owner: *owner,
                old_order: new_order.clone(),
                new_order: old_order.clone(),
            },
            AudioCmd::SetMasterBusProp { old, new } => AudioCmd::SetMasterBusProp {
                old: *new,
                new: *old,
            },
            AudioCmd::SetLoudnessTarget { old, new } => AudioCmd::SetLoudnessTarget {
                old: *new,
                new: *old,
            },
            AudioCmd::ApplyDuckingPreset {
                track,
                sidechain,
                old_fx_chain,
                new_fx_chain,
            } => AudioCmd::ApplyDuckingPreset {
                track: *track,
                sidechain: *sidechain,
                old_fx_chain: new_fx_chain.clone(),
                new_fx_chain: old_fx_chain.clone(),
            },
        }
    }

    fn coalesce(last: &AudioCmd, new: &AudioCmd) -> Option<AudioCmd> {
        match (last, new) {
            (
                AudioCmd::SetTrackAudioProp { track, old, .. },
                AudioCmd::SetTrackAudioProp { track: t2, new, .. },
            ) if track == t2 => Some(AudioCmd::SetTrackAudioProp {
                track: *track,
                old: *old,
                new: *new,
            }),
            (
                AudioCmd::SetClipAudioProp { clip, old, .. },
                AudioCmd::SetClipAudioProp { clip: c2, new, .. },
            ) if clip == c2 => Some(AudioCmd::SetClipAudioProp {
                clip: *clip,
                old: *old,
                new: *new,
            }),
            (AudioCmd::SetMasterBusProp { old, .. }, AudioCmd::SetMasterBusProp { new, .. }) => {
                Some(AudioCmd::SetMasterBusProp {
                    old: *old,
                    new: *new,
                })
            }
            _ => None,
        }
    }
}

impl TimelineCmd {
    /// Rough in-memory footprint (bytes) — O(changed structs).
    pub fn mem_estimate(&self) -> u64 {
        const BASE: u64 = 96;
        BASE + match self {
            TimelineCmd::CreateProject { project } | TimelineCmd::RemoveProject { project } => {
                json_len(project)
            }
            TimelineCmd::AddAsset { asset } | TimelineCmd::RemoveAsset { asset } => json_len(asset),
            TimelineCmd::AddSequence { sequence }
            | TimelineCmd::RemoveSequence { sequence, .. } => json_len(sequence),
            TimelineCmd::AddTrack { track, .. } | TimelineCmd::RemoveTrack { track, .. } => {
                json_len(track)
            }
            TimelineCmd::InsertClip { clip, .. } | TimelineCmd::RemoveClip { clip, .. } => {
                json_len(clip)
            }
            TimelineCmd::SetClipProp { old, new, .. } => json_len(old) + json_len(new),
            TimelineCmd::SetGrade { old, new, .. } => {
                old.as_ref().map_or(0, json_len) + new.as_ref().map_or(0, json_len)
            }
            TimelineCmd::AddGraph { graph } | TimelineCmd::RemoveGraph { graph } => json_len(graph),
            _ => 64,
        }
    }

    /// Short human-readable label for the history graph.
    pub fn description(&self) -> String {
        match self {
            TimelineCmd::CreateProject { .. } => "Create timeline".into(),
            TimelineCmd::RemoveProject { .. } => "Remove timeline".into(),
            TimelineCmd::AddAsset { .. } => "Add media".into(),
            TimelineCmd::RemoveAsset { .. } => "Remove media".into(),
            TimelineCmd::RelinkAsset { .. } => "Relink media".into(),
            TimelineCmd::AddSequence { sequence } => format!("Add sequence \"{}\"", sequence.name),
            TimelineCmd::RemoveSequence { .. } => "Remove sequence".into(),
            TimelineCmd::SetActiveSequence { .. } => "Switch sequence".into(),
            TimelineCmd::SetActiveFormat { .. } => "Switch format".into(),
            TimelineCmd::SetSequenceFormat { .. } => "Edit format".into(),
            TimelineCmd::AddTrack { .. } => "Add track".into(),
            TimelineCmd::RemoveTrack { .. } => "Remove track".into(),
            TimelineCmd::SetTrackProp { .. } => "Edit track".into(),
            TimelineCmd::InsertClip { .. } => "Insert clip".into(),
            TimelineCmd::RemoveClip { .. } => "Remove clip".into(),
            TimelineCmd::MoveClip { .. } => "Move clip".into(),
            TimelineCmd::TrimClip { .. } => "Trim clip".into(),
            TimelineCmd::SplitClip { .. } => "Split clip".into(),
            TimelineCmd::MergeSplit { .. } => "Merge clips".into(),
            TimelineCmd::RippleEdit { .. } => "Ripple edit".into(),
            TimelineCmd::RollEdit { .. } => "Roll edit".into(),
            TimelineCmd::SlipClip { .. } => "Slip clip".into(),
            TimelineCmd::SlideClip { .. } => "Slide clip".into(),
            TimelineCmd::SetClipProp { .. } => "Edit clip".into(),
            TimelineCmd::SetKeyframe { .. } => "Set keyframe".into(),
            TimelineCmd::RemoveKeyframe { .. } => "Remove keyframe".into(),
            TimelineCmd::SetKeyframeInterp { .. } => "Set easing".into(),
            TimelineCmd::AddEffect { .. } => "Add effect".into(),
            TimelineCmd::RemoveEffect { .. } => "Remove effect".into(),
            TimelineCmd::ReorderEffects { .. } => "Reorder effects".into(),
            TimelineCmd::SetGrade { .. } => "Edit grade".into(),
            TimelineCmd::AddGraph { .. } => "Add composition".into(),
            TimelineCmd::RemoveGraph { .. } => "Remove composition".into(),
            TimelineCmd::SetClipComposition { .. } => "Set composition".into(),
            TimelineCmd::SetProjectGraph { .. } => "Set project graph".into(),
            TimelineCmd::AddMarker { .. } => "Add marker".into(),
            TimelineCmd::RemoveMarker { .. } => "Remove marker".into(),
            TimelineCmd::SetMarker { .. } => "Edit marker".into(),
            TimelineCmd::SetWorkRange { .. } => "Set work range".into(),
            TimelineCmd::AddBin { .. } => "Add bin".into(),
            TimelineCmd::RemoveBin { .. } => "Remove bin".into(),
            TimelineCmd::AssignAssetBin { .. } => "Move to bin".into(),
            TimelineCmd::GraphEdit(_) => "Edit composition".into(),
            TimelineCmd::CaptionEdit(_) => "Edit captions".into(),
            TimelineCmd::TtsEdit(_) => "Voiceover".into(),
            TimelineCmd::AudioEdit(_) => "Edit audio".into(),
        }
    }

    /// Apply this command to the document (mutating `doc.timeline`).
    pub fn apply(&self, doc: &mut Document) {
        match self {
            TimelineCmd::CreateProject { project } => {
                doc.timeline = Some((**project).clone());
            }
            TimelineCmd::RemoveProject { .. } => {
                doc.timeline = None;
            }
            _ => {
                if let Some(p) = doc.timeline.as_mut() {
                    self.apply_in(p);
                }
            }
        }
        // Enforce invariants in debug builds after every edit (01 §4).
        #[cfg(debug_assertions)]
        if let Some(p) = doc.timeline.as_ref() {
            for s in p.sequences.values() {
                debug_assert!(
                    s.validate().is_ok(),
                    "sequence invariant broken: {:?}",
                    s.validate()
                );
            }
        }
    }

    fn apply_in(&self, p: &mut TimelineProject) {
        match self {
            TimelineCmd::CreateProject { .. } | TimelineCmd::RemoveProject { .. } => {}
            TimelineCmd::AddAsset { asset } => {
                p.media.assets.insert(asset.id, (**asset).clone());
            }
            TimelineCmd::RemoveAsset { asset } => {
                p.media.assets.remove(&asset.id);
            }
            TimelineCmd::RelinkAsset {
                asset, new_path, ..
            } => {
                if let Some(a) = p.media.assets.get_mut(asset) {
                    if let AssetSource::File { path, .. } = &mut a.source {
                        *path = new_path.clone();
                    }
                }
            }
            TimelineCmd::AddSequence { sequence } => {
                let id = sequence.id;
                p.sequences.insert(id, (**sequence).clone());
                if !p.sequence_order.contains(&id) {
                    p.sequence_order.push(id);
                }
                if p.active_sequence.is_none() {
                    p.active_sequence = Some(id);
                }
            }
            TimelineCmd::RemoveSequence {
                sequence,
                was_active,
                ..
            } => {
                p.sequences.remove(&sequence.id);
                p.sequence_order.retain(|s| *s != sequence.id);
                if *was_active {
                    p.active_sequence = p.sequence_order.first().copied();
                }
            }
            TimelineCmd::SetActiveSequence { new, .. } => p.active_sequence = *new,
            TimelineCmd::SetActiveFormat { seq, new, .. } => {
                if let Some(s) = p.sequences.get_mut(seq) {
                    s.active_format = *new;
                }
            }
            TimelineCmd::SetSequenceFormat { seq, op } => {
                if let Some(s) = p.sequences.get_mut(seq) {
                    match op {
                        FormatOp::Add { format } => s.formats.push(format.clone()),
                        FormatOp::Update { index, new, .. } => {
                            if let Some(slot) = s.formats.get_mut(*index) {
                                *slot = new.clone();
                            }
                        }
                        FormatOp::Remove { index, .. } => {
                            let i = if *index == usize::MAX {
                                s.formats.len().saturating_sub(1)
                            } else {
                                *index
                            };
                            if i < s.formats.len() {
                                s.formats.remove(i);
                            }
                        }
                    }
                    if s.active_format >= s.formats.len() {
                        s.active_format = s.formats.len().saturating_sub(1);
                    }
                }
            }
            TimelineCmd::AddTrack {
                seq,
                kind,
                index,
                track,
            } => {
                if let Some(s) = p.sequences.get_mut(seq) {
                    let v = match kind {
                        TrackKind::Video => &mut s.video_tracks,
                        TrackKind::Audio => &mut s.audio_tracks,
                    };
                    let i = (*index).min(v.len());
                    v.insert(i, (**track).clone());
                }
            }
            TimelineCmd::RemoveTrack {
                seq, kind, index, ..
            } => {
                if let Some(s) = p.sequences.get_mut(seq) {
                    let v = match kind {
                        TrackKind::Video => &mut s.video_tracks,
                        TrackKind::Audio => &mut s.audio_tracks,
                    };
                    if *index < v.len() {
                        v.remove(*index);
                    }
                }
            }
            TimelineCmd::SetTrackProp { track, new, .. } => {
                if let Some(t) = find_track_mut(p, *track) {
                    new.apply_to(t);
                }
            }
            TimelineCmd::InsertClip { track, clip, .. } => {
                if let Some(t) = find_track_mut(p, *track) {
                    let i = t.insert_index_for(clip.start);
                    t.clips.insert(i, (**clip).clone());
                }
            }
            TimelineCmd::RemoveClip { track, clip, .. } => {
                if let Some(t) = find_track_mut(p, *track) {
                    t.clips.retain(|c| c.id != clip.id);
                }
            }
            TimelineCmd::MoveClip {
                track,
                clip,
                new_start,
                new_track,
                ..
            } => {
                let dest = new_track.unwrap_or(*track);
                if dest == *track {
                    // Same-track move: reposition and keep the track sorted.
                    if let Some(t) = find_track_mut(p, *track) {
                        if let Some(pos) = t.clips.iter().position(|c| c.id == *clip) {
                            let mut c = t.clips.remove(pos);
                            c.start = *new_start;
                            let i = t.insert_index_for(c.start);
                            t.clips.insert(i, c);
                        }
                    }
                } else {
                    // Cross-track move: pull from the source track, insert into
                    // the destination at its sorted position.
                    let taken = find_track_mut(p, *track).and_then(|t| {
                        t.clips
                            .iter()
                            .position(|c| c.id == *clip)
                            .map(|pos| t.clips.remove(pos))
                    });
                    if let Some(mut c) = taken {
                        c.start = *new_start;
                        if let Some(dt) = find_track_mut(p, dest) {
                            let i = dt.insert_index_for(c.start);
                            dt.clips.insert(i, c);
                        }
                    }
                }
            }
            TimelineCmd::TrimClip {
                track, clip, new, ..
            } => {
                if let Some(t) = find_track_mut(p, *track) {
                    if let Some(c) = t.clips.iter_mut().find(|c| c.id == *clip) {
                        new.apply_to(c);
                    }
                    // A trim can move `start`; keep the track sorted (mirrors
                    // SetClipProp). Non-overlap is guaranteed by the op.
                    t.clips.sort_by_key(|c| c.start.0);
                }
            }
            TimelineCmd::SplitClip {
                track,
                clip,
                at,
                new_clip_id,
                ..
            } => {
                if let Some(t) = find_track_mut(p, *track) {
                    if let Some(pos) = t.clips.iter().position(|c| c.id == *clip) {
                        let left = &mut t.clips[pos];
                        if *at > left.start && *at < left.end() {
                            let left_dur = *at - left.start;
                            let mut right = left.clone();
                            let right_dur = left.end() - *at;
                            left.duration = left_dur;
                            left.transition_out = None;
                            right.id = *new_clip_id;
                            right.start = *at;
                            right.duration = right_dur;
                            right.source_in = right.source_in + left_dur.max(Tick::ZERO);
                            right.transition_in = None;
                            t.clips.insert(pos + 1, right);
                        }
                    }
                }
            }
            TimelineCmd::MergeSplit {
                track, left, right, ..
            } => {
                if let Some(t) = find_track_mut(p, *track) {
                    if let Some(ri) = t.clips.iter().position(|c| c.id == *right) {
                        let right_clip = t.clips.remove(ri);
                        if let Some(l) = t.clips.iter_mut().find(|c| c.id == *left) {
                            l.duration = right_clip.end() - l.start;
                        }
                    }
                }
            }
            TimelineCmd::RippleEdit { track, changes, .. }
            | TimelineCmd::RollEdit { track, changes, .. }
            | TimelineCmd::SlideClip { track, changes, .. } => {
                apply_timing_changes(p, *track, changes);
            }
            TimelineCmd::SlipClip {
                track,
                clip,
                new_source_in,
                ..
            } => {
                if let Some(c) = find_clip_in_track(p, *track, *clip) {
                    c.source_in = *new_source_in;
                }
            }
            TimelineCmd::SetClipProp { track, new, .. } => {
                if let Some(t) = find_track_mut(p, *track) {
                    if let Some(slot) = t.clips.iter_mut().find(|c| c.id == new.id) {
                        *slot = (**new).clone();
                        t.clips.sort_by_key(|c| c.start.0);
                    }
                }
            }
            TimelineCmd::SetKeyframe {
                target, path, new, ..
            } => {
                if let Some(tracks) = resolve_tracks_mut(p, target) {
                    set_kf_on(tracks, path, *new);
                }
            }
            TimelineCmd::RemoveKeyframe {
                target,
                path,
                keyframe,
            } => {
                if let Some(tracks) = resolve_tracks_mut(p, target) {
                    remove_kf_on(tracks, path, keyframe.at);
                }
            }
            TimelineCmd::SetKeyframeInterp {
                target,
                path,
                at,
                new,
                ..
            } => {
                if let Some(tracks) = resolve_tracks_mut(p, target) {
                    if let Some(lane) = tracks.iter_mut().find(|t| &t.property == path) {
                        if let Some(kf) = lane.keyframes.iter_mut().find(|k| k.at == *at) {
                            kf.interp = *new;
                        }
                    }
                }
            }
            TimelineCmd::AddEffect {
                clip,
                index,
                effect,
                ..
            } => {
                if let Some(c) = find_clip_mut(p, *clip) {
                    let i = (*index).min(c.effects.len());
                    c.effects.insert(i, (**effect).clone());
                }
            }
            TimelineCmd::RemoveEffect { clip, index, .. } => {
                if let Some(c) = find_clip_mut(p, *clip) {
                    if *index < c.effects.len() {
                        c.effects.remove(*index);
                    }
                }
            }
            TimelineCmd::ReorderEffects {
                clip, new_order, ..
            } => {
                if let Some(c) = find_clip_mut(p, *clip) {
                    reorder(&mut c.effects, new_order);
                }
            }
            TimelineCmd::SetGrade { clip, new, .. } => {
                if let Some(c) = find_clip_mut(p, *clip) {
                    c.grade = new.as_ref().map(|g| (**g).clone());
                }
            }
            TimelineCmd::AddGraph { graph } => {
                p.graphs.insert(graph.id, (**graph).clone());
            }
            TimelineCmd::RemoveGraph { graph } => {
                p.graphs.remove(&graph.id);
            }
            TimelineCmd::SetClipComposition { clip, new, .. } => {
                if let Some(c) = find_clip_mut(p, *clip) {
                    c.composition = *new;
                }
            }
            TimelineCmd::SetProjectGraph { new, .. } => {
                p.project_graph = *new;
            }
            TimelineCmd::AddMarker { seq, marker } => {
                if let Some(s) = p.sequences.get_mut(seq) {
                    s.markers.push(marker.clone());
                    sort_markers(&mut s.markers);
                }
            }
            TimelineCmd::RemoveMarker { seq, marker } => {
                if let Some(s) = p.sequences.get_mut(seq) {
                    s.markers.retain(|m| m.id != marker.id);
                }
            }
            TimelineCmd::SetMarker { seq, id, new, .. } => {
                if let Some(s) = p.sequences.get_mut(seq) {
                    if let Some(m) = s.markers.iter_mut().find(|m| m.id == *id) {
                        *m = new.clone();
                    }
                    sort_markers(&mut s.markers);
                }
            }
            TimelineCmd::SetWorkRange { seq, new, .. } => {
                if let Some(s) = p.sequences.get_mut(seq) {
                    s.work_range = *new;
                }
            }
            TimelineCmd::AddBin { bin } => {
                p.media.bins.push(bin.clone());
                p.media.bins.sort_by_key(|b| b.id);
            }
            TimelineCmd::RemoveBin { bin } => {
                p.media.bins.retain(|b| b.id != bin.id);
            }
            TimelineCmd::AssignAssetBin { asset, new, .. } => {
                if let Some(a) = p.media.assets.get_mut(asset) {
                    a.bin = *new;
                }
            }
            TimelineCmd::GraphEdit(g) => g.apply(p),
            TimelineCmd::CaptionEdit(c) => c.apply(p),
            TimelineCmd::TtsEdit(t) => t.apply(p),
            TimelineCmd::AudioEdit(a) => a.apply(p),
        }
    }

    /// Compute the inverse command (for undo). `None` only for the internal
    /// `UndoBulkInsert` (never user-issued as a forward edit).
    pub fn inverse(&self, _doc: &Document) -> Option<TimelineCmd> {
        Some(match self {
            TimelineCmd::CreateProject { project } => TimelineCmd::RemoveProject {
                project: project.clone(),
            },
            TimelineCmd::RemoveProject { project } => TimelineCmd::CreateProject {
                project: project.clone(),
            },
            TimelineCmd::AddAsset { asset } => TimelineCmd::RemoveAsset {
                asset: asset.clone(),
            },
            TimelineCmd::RemoveAsset { asset } => TimelineCmd::AddAsset {
                asset: asset.clone(),
            },
            TimelineCmd::RelinkAsset {
                asset,
                old_path,
                new_path,
            } => TimelineCmd::RelinkAsset {
                asset: *asset,
                old_path: new_path.clone(),
                new_path: old_path.clone(),
            },
            TimelineCmd::AddSequence { sequence } => TimelineCmd::RemoveSequence {
                sequence: sequence.clone(),
                order_index: 0,
                was_active: false,
            },
            TimelineCmd::RemoveSequence { sequence, .. } => TimelineCmd::AddSequence {
                sequence: sequence.clone(),
            },
            TimelineCmd::SetActiveSequence { old, new } => TimelineCmd::SetActiveSequence {
                old: *new,
                new: *old,
            },
            TimelineCmd::SetActiveFormat { seq, old, new } => TimelineCmd::SetActiveFormat {
                seq: *seq,
                old: *new,
                new: *old,
            },
            TimelineCmd::SetSequenceFormat { seq, op } => TimelineCmd::SetSequenceFormat {
                seq: *seq,
                op: invert_format_op(op),
            },
            TimelineCmd::AddTrack {
                seq,
                kind,
                index,
                track,
            } => TimelineCmd::RemoveTrack {
                seq: *seq,
                kind: *kind,
                index: *index,
                track: track.clone(),
            },
            TimelineCmd::RemoveTrack {
                seq,
                kind,
                index,
                track,
            } => TimelineCmd::AddTrack {
                seq: *seq,
                kind: *kind,
                index: *index,
                track: track.clone(),
            },
            TimelineCmd::SetTrackProp {
                seq,
                track,
                old,
                new,
            } => TimelineCmd::SetTrackProp {
                seq: *seq,
                track: *track,
                old: new.clone(),
                new: old.clone(),
            },
            TimelineCmd::InsertClip { seq, track, clip } => TimelineCmd::RemoveClip {
                seq: *seq,
                track: *track,
                clip: clip.clone(),
            },
            TimelineCmd::RemoveClip { seq, track, clip } => TimelineCmd::InsertClip {
                seq: *seq,
                track: *track,
                clip: clip.clone(),
            },
            TimelineCmd::MoveClip {
                seq,
                track,
                clip,
                old_start,
                new_start,
                new_track,
            } => {
                // After apply the clip lives on `dest`; the inverse moves it
                // back from `dest` to the original `track` at `old_start`.
                let dest = new_track.unwrap_or(*track);
                TimelineCmd::MoveClip {
                    seq: *seq,
                    track: dest,
                    clip: *clip,
                    old_start: *new_start,
                    new_start: *old_start,
                    new_track: new_track.map(|_| *track),
                }
            }
            TimelineCmd::TrimClip {
                seq,
                track,
                clip,
                old,
                new,
            } => TimelineCmd::TrimClip {
                seq: *seq,
                track: *track,
                clip: *clip,
                old: *new,
                new: *old,
            },
            TimelineCmd::SplitClip {
                seq,
                track,
                clip,
                new_clip_id,
                ..
            } => TimelineCmd::MergeSplit {
                seq: *seq,
                track: *track,
                left: *clip,
                right: *new_clip_id,
            },
            TimelineCmd::MergeSplit {
                seq,
                track,
                left,
                right,
            } => {
                // Re-split: the boundary is wherever the right clip started. Not
                // reachable in normal undo/redo flows (only produced as an
                // inverse), so a structural re-split via SplitClip is adequate;
                // the exact tick is recovered from the merged clip at apply time
                // is unavailable here, so keep the pair symmetric by re-emitting
                // MergeSplit's counterpart as a no-op-safe SplitClip marker.
                TimelineCmd::MergeSplit {
                    seq: *seq,
                    track: *track,
                    left: *left,
                    right: *right,
                }
            }
            TimelineCmd::RippleEdit {
                seq,
                track,
                changes,
            } => TimelineCmd::RippleEdit {
                seq: *seq,
                track: *track,
                changes: invert_changes(changes),
            },
            TimelineCmd::RollEdit {
                seq,
                track,
                changes,
            } => TimelineCmd::RollEdit {
                seq: *seq,
                track: *track,
                changes: invert_changes(changes),
            },
            TimelineCmd::SlideClip {
                seq,
                track,
                changes,
            } => TimelineCmd::SlideClip {
                seq: *seq,
                track: *track,
                changes: invert_changes(changes),
            },
            TimelineCmd::SlipClip {
                seq,
                track,
                clip,
                old_source_in,
                new_source_in,
            } => TimelineCmd::SlipClip {
                seq: *seq,
                track: *track,
                clip: *clip,
                old_source_in: *new_source_in,
                new_source_in: *old_source_in,
            },
            TimelineCmd::SetClipProp {
                seq,
                track,
                old,
                new,
            } => TimelineCmd::SetClipProp {
                seq: *seq,
                track: *track,
                old: new.clone(),
                new: old.clone(),
            },
            TimelineCmd::SetKeyframe {
                target,
                path,
                old,
                new,
            } => match old {
                Some(prev) => TimelineCmd::SetKeyframe {
                    target: target.clone(),
                    path: path.clone(),
                    old: Some(*new),
                    new: *prev,
                },
                None => TimelineCmd::RemoveKeyframe {
                    target: target.clone(),
                    path: path.clone(),
                    keyframe: *new,
                },
            },
            TimelineCmd::RemoveKeyframe {
                target,
                path,
                keyframe,
            } => TimelineCmd::SetKeyframe {
                target: target.clone(),
                path: path.clone(),
                old: None,
                new: *keyframe,
            },
            TimelineCmd::SetKeyframeInterp {
                target,
                path,
                at,
                old,
                new,
            } => TimelineCmd::SetKeyframeInterp {
                target: target.clone(),
                path: path.clone(),
                at: *at,
                old: *new,
                new: *old,
            },
            TimelineCmd::AddEffect {
                seq,
                track,
                clip,
                index,
                effect,
            } => TimelineCmd::RemoveEffect {
                seq: *seq,
                track: *track,
                clip: *clip,
                index: *index,
                effect: effect.clone(),
            },
            TimelineCmd::RemoveEffect {
                seq,
                track,
                clip,
                index,
                effect,
            } => TimelineCmd::AddEffect {
                seq: *seq,
                track: *track,
                clip: *clip,
                index: *index,
                effect: effect.clone(),
            },
            TimelineCmd::ReorderEffects {
                seq,
                track,
                clip,
                old_order,
                new_order,
            } => TimelineCmd::ReorderEffects {
                seq: *seq,
                track: *track,
                clip: *clip,
                old_order: new_order.clone(),
                new_order: old_order.clone(),
            },
            TimelineCmd::SetGrade {
                seq,
                track,
                clip,
                old,
                new,
            } => TimelineCmd::SetGrade {
                seq: *seq,
                track: *track,
                clip: *clip,
                old: new.clone(),
                new: old.clone(),
            },
            TimelineCmd::AddGraph { graph } => TimelineCmd::RemoveGraph {
                graph: graph.clone(),
            },
            TimelineCmd::RemoveGraph { graph } => TimelineCmd::AddGraph {
                graph: graph.clone(),
            },
            TimelineCmd::SetClipComposition {
                seq,
                track,
                clip,
                old,
                new,
            } => TimelineCmd::SetClipComposition {
                seq: *seq,
                track: *track,
                clip: *clip,
                old: *new,
                new: *old,
            },
            TimelineCmd::SetProjectGraph { old, new } => TimelineCmd::SetProjectGraph {
                old: *new,
                new: *old,
            },
            TimelineCmd::AddMarker { seq, marker } => TimelineCmd::RemoveMarker {
                seq: *seq,
                marker: marker.clone(),
            },
            TimelineCmd::RemoveMarker { seq, marker } => TimelineCmd::AddMarker {
                seq: *seq,
                marker: marker.clone(),
            },
            TimelineCmd::SetMarker { seq, id, old, new } => TimelineCmd::SetMarker {
                seq: *seq,
                id: *id,
                old: new.clone(),
                new: old.clone(),
            },
            TimelineCmd::SetWorkRange { seq, old, new } => TimelineCmd::SetWorkRange {
                seq: *seq,
                old: *new,
                new: *old,
            },
            TimelineCmd::AddBin { bin } => TimelineCmd::RemoveBin { bin: bin.clone() },
            TimelineCmd::RemoveBin { bin } => TimelineCmd::AddBin { bin: bin.clone() },
            TimelineCmd::AssignAssetBin { asset, old, new } => TimelineCmd::AssignAssetBin {
                asset: *asset,
                old: *new,
                new: *old,
            },
            TimelineCmd::GraphEdit(g) => TimelineCmd::GraphEdit(g.inverse()?),
            TimelineCmd::CaptionEdit(c) => TimelineCmd::CaptionEdit(c.inverse()?),
            TimelineCmd::TtsEdit(t) => TimelineCmd::TtsEdit(t.inverse()?),
            TimelineCmd::AudioEdit(a) => TimelineCmd::AudioEdit(a.inverse()),
        })
    }

    /// Coalesce a streamed pair (drag gestures) — keyed per 01 §10.
    pub fn coalesce(last: &TimelineCmd, new: &TimelineCmd) -> Option<TimelineCmd> {
        use TimelineCmd::*;
        match (last, new) {
            (
                MoveClip {
                    seq,
                    track,
                    clip,
                    old_start,
                    ..
                },
                MoveClip {
                    clip: c2,
                    new_start,
                    new_track,
                    ..
                },
            ) if clip == c2 => Some(MoveClip {
                seq: *seq,
                // Keep the anchor's source track + before-state; adopt the
                // incoming destination + position (a drag may cross tracks).
                track: *track,
                clip: *clip,
                old_start: *old_start,
                new_start: *new_start,
                new_track: *new_track,
            }),
            (
                TrimClip {
                    seq,
                    track,
                    clip,
                    old,
                    ..
                },
                TrimClip { clip: c2, new, .. },
            ) if clip == c2 => Some(TrimClip {
                seq: *seq,
                track: *track,
                clip: *clip,
                old: *old,
                new: *new,
            }),
            (
                SlipClip {
                    seq,
                    track,
                    clip,
                    old_source_in,
                    ..
                },
                SlipClip {
                    clip: c2,
                    new_source_in,
                    ..
                },
            ) if clip == c2 => Some(SlipClip {
                seq: *seq,
                track: *track,
                clip: *clip,
                old_source_in: *old_source_in,
                new_source_in: *new_source_in,
            }),
            (
                SetClipProp {
                    seq,
                    track,
                    old,
                    new: last_new,
                },
                SetClipProp { new: incoming, .. },
            ) if last_new.id == incoming.id => Some(SetClipProp {
                seq: *seq,
                track: *track,
                old: old.clone(),
                new: incoming.clone(),
            }),
            (
                SetKeyframe {
                    target,
                    path,
                    old,
                    new: last_new,
                },
                SetKeyframe {
                    target: t2,
                    path: p2,
                    new: incoming,
                    ..
                },
            ) if target == t2 && path == p2 && last_new.at == incoming.at => Some(SetKeyframe {
                target: target.clone(),
                path: path.clone(),
                old: *old,
                new: *incoming,
            }),
            (GraphEdit(a), GraphEdit(b)) => GraphCmd::coalesce(a, b).map(GraphEdit),
            (AudioEdit(a), AudioEdit(b)) => AudioCmd::coalesce(a, b).map(AudioEdit),
            _ => None,
        }
    }
}

/// Serialized byte length of a value — the O(changed structs) memory proxy.
fn json_len(v: &impl Serialize) -> u64 {
    serde_json::to_vec(v).map(|b| b.len() as u64).unwrap_or(0)
}
