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
    ops, AssetId, AssetKind, Clip, ClipSource, ClipTiming, FrameRate, Marker, MarkerId,
    MediaAsset, Sequence, SequenceFormat, SequenceId, Tick, TimelineCmd, Track, TrackId, TrackKind,
    TrackSettings, TICKS_PER_SECOND,
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

// ── Aspect ratio / sequence format ──────────────────────────────────────────

/// Switch the sequence to the aspect/frame named `name` (`w`×`h`): activate the
/// existing format if one already matches, otherwise add it and activate it —
/// one undoable step either way. This is the one-click "make this 9:16 / 1:1 /
/// 16:9" control behind the monitor's format bar (CAP-012).
pub(crate) fn switch_to_aspect(
    history: &mut CommandHistory,
    doc: &mut Document,
    seq_id: SequenceId,
    name: &str,
    w: u32,
    h: u32,
) {
    let Some(project) = doc.timeline.as_ref() else {
        return;
    };
    let Some(seq) = project.sequences.get(&seq_id) else {
        return;
    };
    // Match an existing format by dimensions (name is cosmetic; the aspect is
    // what matters), so repeated clicks just re-activate rather than pile up.
    if let Some(idx) = seq.formats.iter().position(|f| f.width == w && f.height == h) {
        if seq.active_format != idx {
            if let Ok(cmd) = ops::set_active_format(project, seq_id, idx) {
                commit(history, doc, cmd);
            }
        }
        return;
    }
    let new_idx = seq.formats.len();
    let add = ops::add_format(seq_id, SequenceFormat::new(name, w, h));
    // Add then activate as one undo step.
    let activate = TimelineCmd::SetActiveFormat {
        seq: seq_id,
        old: seq.active_format,
        new: new_idx,
    };
    commit_batch(history, doc, vec![add, activate]);
}

/// The built-in quick aspect presets shown on the monitor's format bar
/// (name, width, height). 1080-tall/wide family per 04 §4.1 / CAP-012.
pub(crate) const ASPECT_PRESETS: &[(&str, u32, u32)] = &[
    ("16:9", 1920, 1080),
    ("9:16", 1080, 1920),
    ("1:1", 1080, 1080),
    ("4:5", 1080, 1350),
    ("4:3", 1440, 1080),
    ("21:9", 2560, 1080),
];

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

/// Move a clip to a different track via `ops::move_clip_to_track` — a single
/// lossless `MoveClip` command (its inverse restores the clip to its original
/// track + position), rather than composing remove+insert.
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
    if let Ok(cmd) = ops::move_clip_to_track(p, seq, from_track, clip, new_start, Some(to_track)) {
        commit(history, doc, cmd);
    }
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

// ── Media pool → timeline (05 §2) ────────────────────────────────────────────

/// Which track kind can host a clip of this asset kind. `None` = not
/// insertable as a clip (LUTs attach to grades, not tracks).
pub(crate) fn track_kind_for_asset(kind: AssetKind) -> Option<TrackKind> {
    match kind {
        AssetKind::Audio => Some(TrackKind::Audio),
        AssetKind::Video | AssetKind::Image | AssetKind::VectorDoc => Some(TrackKind::Video),
        AssetKind::Lut3d => None,
    }
}

/// Build the default clip for dropping `asset` at `start`: duration from the
/// probe (images / probe-less assets default to 5 s), `Vector` source for
/// vector docs, clip named after the asset.
pub(crate) fn clip_for_asset(asset: &MediaAsset, start: Tick) -> Clip {
    let duration = asset
        .probe
        .as_ref()
        .map(|p| p.duration)
        .filter(|d| d.0 > 0)
        .unwrap_or(Tick(5 * TICKS_PER_SECOND));
    let source = match asset.kind {
        AssetKind::VectorDoc => ClipSource::Vector { asset: asset.id },
        _ => ClipSource::Asset { asset: asset.id },
    };
    let mut clip = Clip::new(source, start, duration);
    clip.name = crate::panels::media_pool::asset_display_name(asset);
    clip
}

/// Insert `asset` as a new clip starting at `start` on `track` (the timeline
/// drag-drop path). Kind-checked against the track; a rejected insert
/// (overlap, wrong lane) is a silent no-op like other invalid gestures.
pub(crate) fn insert_asset_clip(
    doc: &mut Document,
    history: &mut CommandHistory,
    seq: SequenceId,
    track_id: TrackId,
    asset_id: AssetId,
    start: Tick,
) -> bool {
    let Some(p) = doc.timeline.as_ref() else {
        return false;
    };
    let Some(asset) = p.media.assets.get(&asset_id) else {
        return false;
    };
    let wanted = track_kind_for_asset(asset.kind);
    let track_kind = p
        .sequences
        .get(&seq)
        .and_then(|s| s.track(track_id))
        .map(|t| t.kind);
    if wanted.is_none() || track_kind != wanted {
        return false;
    }
    let clip = clip_for_asset(asset, start);
    match ops::insert_clip(p, seq, track_id, clip) {
        Ok(cmd) => {
            commit(history, doc, cmd);
            true
        }
        Err(_) => false,
    }
}

/// Insert `asset` at the playhead on the first compatible track that accepts
/// it (double-click / context-menu path).
pub(crate) fn insert_asset_at_first_fit(
    doc: &mut Document,
    history: &mut CommandHistory,
    asset_id: AssetId,
    at: Tick,
) -> bool {
    let Some(p) = doc.timeline.as_ref() else {
        return false;
    };
    let Some(seq_id) = p.active_sequence else {
        return false;
    };
    let Some(asset) = p.media.assets.get(&asset_id) else {
        return false;
    };
    let Some(wanted) = track_kind_for_asset(asset.kind) else {
        return false;
    };
    let Some(seq) = p.sequences.get(&seq_id) else {
        return false;
    };
    let tracks: Vec<TrackId> = match wanted {
        TrackKind::Video => seq.video_tracks.iter().map(|t| t.id).collect(),
        TrackKind::Audio => seq.audio_tracks.iter().map(|t| t.id).collect(),
    };
    for track in tracks {
        if insert_asset_clip(doc, history, seq_id, track, asset_id, at) {
            return true;
        }
    }
    false
}

// ── Markers & work range ─────────────────────────────────────────────────────

/// Add a marker to a sequence at `at` (double-click on the ruler, 04 §2.6/13
/// §1.1).
pub(crate) fn add_marker(
    doc: &mut Document,
    history: &mut CommandHistory,
    seq: SequenceId,
    at: Tick,
    name: impl Into<String>,
) {
    let Some(p) = doc.timeline.as_ref() else {
        return;
    };
    if let Ok(cmd) = ops::add_marker(p, seq, Marker::new(at, name)) {
        commit(history, doc, cmd);
    }
}

pub(crate) fn remove_marker(
    doc: &mut Document,
    history: &mut CommandHistory,
    seq: SequenceId,
    marker: MarkerId,
) {
    let Some(p) = doc.timeline.as_ref() else {
        return;
    };
    if let Ok(cmd) = ops::remove_marker(p, seq, marker) {
        commit(history, doc, cmd);
    }
}

/// Change one field of a marker via `SetMarker` — the marker analogue of
/// `set_track_settings`.
fn set_marker_field(
    doc: &mut Document,
    history: &mut CommandHistory,
    seq: SequenceId,
    marker: MarkerId,
    edit: impl FnOnce(&mut Marker),
) {
    let Some(p) = doc.timeline.as_ref() else {
        return;
    };
    let Some(existing) = p
        .sequences
        .get(&seq)
        .and_then(|s| s.markers.iter().find(|m| m.id == marker).cloned())
    else {
        return;
    };
    let mut new = existing;
    edit(&mut new);
    if let Ok(cmd) = ops::set_marker(p, seq, new) {
        commit(history, doc, cmd);
    }
}

/// Rename a marker (context-menu "Rename").
pub(crate) fn rename_marker(
    doc: &mut Document,
    history: &mut CommandHistory,
    seq: SequenceId,
    marker: MarkerId,
    name: String,
) {
    set_marker_field(doc, history, seq, marker, |m| m.name = name);
}

/// Retime a marker (drag-to-retime on the ruler; commit once on release, one
/// undo step per gesture — same discipline as clip drags).
pub(crate) fn retime_marker(
    doc: &mut Document,
    history: &mut CommandHistory,
    seq: SequenceId,
    marker: MarkerId,
    at: Tick,
) {
    set_marker_field(doc, history, seq, marker, |m| m.at = at);
}

/// Set (or clear, with `None`) a sequence's preview/export work range — the
/// monitor's I/O transport buttons (04 §3.2) route through here instead of
/// mutating `Sequence::work_range` directly.
pub(crate) fn set_work_range(
    doc: &mut Document,
    history: &mut CommandHistory,
    seq: SequenceId,
    new: Option<(Tick, Tick)>,
) {
    let Some(p) = doc.timeline.as_ref() else {
        return;
    };
    if let Ok(cmd) = ops::set_work_range(p, seq, new) {
        commit(history, doc, cmd);
    }
}

#[cfg(test)]
mod format_tests {
    use super::*;
    use photonic_core::timeline::{Sequence, TimelineProject};

    fn doc_with_seq() -> (Document, CommandHistory, SequenceId) {
        let mut project = TimelineProject::new();
        let seq = Sequence::new("s", FrameRate::new(30, 1), 1920, 1080); // starts 16:9 @ index 0
        let seq_id = project.insert_sequence(seq);
        project.active_sequence = Some(seq_id);
        let mut doc = Document::new("t", 1920.0, 1080.0);
        doc.timeline = Some(project);
        (doc, CommandHistory::new(64), seq_id)
    }

    fn active(doc: &Document, seq_id: SequenceId) -> (usize, u32, u32) {
        let s = &doc.timeline.as_ref().unwrap().sequences[&seq_id];
        let f = &s.formats[s.active_format];
        (s.active_format, f.width, f.height)
    }

    #[test]
    fn switch_adds_then_reactivates_without_duplicating() {
        let (mut doc, mut h, seq) = doc_with_seq();
        // New aspect → added at index 1 and activated.
        switch_to_aspect(&mut h, &mut doc, seq, "9:16", 1080, 1920);
        assert_eq!(active(&doc, seq), (1, 1080, 1920));
        assert_eq!(doc.timeline.as_ref().unwrap().sequences[&seq].formats.len(), 2);

        // Back to the original aspect → re-activates index 0, no new format.
        switch_to_aspect(&mut h, &mut doc, seq, "16:9", 1920, 1080);
        assert_eq!(active(&doc, seq), (0, 1920, 1080));
        assert_eq!(doc.timeline.as_ref().unwrap().sequences[&seq].formats.len(), 2);

        // Repeating the current aspect is a no-op (no duplicate, still index 0).
        switch_to_aspect(&mut h, &mut doc, seq, "16:9", 1920, 1080);
        assert_eq!(doc.timeline.as_ref().unwrap().sequences[&seq].formats.len(), 2);

        // Undo the reactivation, then the add — state walks back cleanly.
        h.undo(&mut doc);
        assert_eq!(active(&doc, seq), (1, 1080, 1920));
        h.undo(&mut doc);
        assert_eq!(active(&doc, seq), (0, 1920, 1080));
        assert_eq!(doc.timeline.as_ref().unwrap().sequences[&seq].formats.len(), 1);
    }
}
