//! Video-domain MCP tool handlers (10-mcp-tools.md, P2 slice: timeline-EDIT
//! tools only — sequence/track/clip/effects/keyframes/media). Every mutating
//! handler here follows design rule 1 (10 §1): deserialize args, resolve
//! addressing (which sequence/track a clip/track id lives under — a plain
//! project scan, not edit logic), call the matching pure fn in
//! `photonic_core::timeline::ops`, wrap the returned `TimelineCmd` in
//! `Command::Timeline` (or `Command::Batch` for multi-command ops), and
//! execute via `history.execute_discrete` (design rule 4: one command per
//! call). Lock order is always document before history (design rule 7).
//!
//! Engine-backed tools (probe/proxy/playback/render/export/captions/tts/grade
//! scopes) are P3+ and are not implemented here.
//!
//! ## Known gaps (no backing `TimelineCmd`/op in `photonic-core` — reported,
//! not worked around by re-deriving edit logic in the handler layer)
//! - `set_work_range`, `add_marker`, `remove_marker`, `list_markers`:
//!   `Sequence::work_range`/`Sequence::markers` are data fields with no
//!   `TimelineCmd` variant or `ops.rs` fn to mutate them undoably. Not
//!   implemented.
//! - `ripple_edit` (trim one edge + ripple every later clip on the track):
//!   `ops.rs` only has `ripple_delete` (delete + ripple), used here for
//!   `remove_clip`'s `ripple` flag. There is no "trim + ripple" op. Not
//!   implemented.
//! - `move_clip`'s `new_track_id` (cross-track move): `TimelineCmd::MoveClip`
//!   has a single `track` field, no track-change support. Implemented for
//!   same-track moves only; a differing `new_track_id` returns
//!   `NotSupportedV1`.
//! - Media `bin` (folder) assignment/filtering: `MediaAsset` has no bin
//!   reference field. `import_media`/`list_media` accept the arg but flag
//!   `bin_assignment_supported`/`bin_filter_supported: false` rather than
//!   silently no-op.
//! - `content_hash` uses a stopgap `DefaultHasher` (SipHash) digest over
//!   head+tail+len, not the `xxh3` the core doc comment (media.rs:6)
//!   describes as the eventual relink identity (that's P3 engine work).

use crate::protocol::*;
use crate::server::AppState;
use photonic_core::history::Command;
use photonic_core::timeline::{
    ops, AnimTarget, AssetId, AssetKind, Clip, ClipEffect, ClipId, ClipSource, ClipTiming,
    EditError, FormatOp, FrameRate, Keyframe, PropPath, Ratio, SequenceId, SpeedMap, Tick,
    TimelineCmd, TimelineProject, Track, TrackId, TrackSettings, Transition, TICKS_PER_SECOND,
};
use photonic_core::Color;
use serde_json::json;

// ─── Shared helpers ──────────────────────────────────────────────────────────

/// A structured-error `ToolResult` per 10 §8's taxonomy.
fn err_code(code: &str, msg: impl Into<String>) -> ToolResult {
    ToolResult::error(msg).with_data(json!({ "error_code": code }))
}

/// Design rule 3: `at_ticks` > `at_tc` > `at_seconds`. `at_tc` requires a
/// resolvable sequence (frame rate) — `MissingSequenceContext` otherwise.
fn resolve_tick(
    ticks: Option<i64>,
    tc: Option<&str>,
    seconds: Option<f64>,
    frame_rate: Option<FrameRate>,
) -> Result<Tick, ToolResult> {
    if let Some(t) = ticks {
        return Ok(Tick(t));
    }
    if let Some(tc) = tc {
        let fr = frame_rate.ok_or_else(|| {
            err_code(
                "MissingSequenceContext",
                "at_tc given without a resolvable sequence context",
            )
        })?;
        return parse_timecode(tc, fr).ok_or_else(|| {
            ToolResult::error(format!(
                "invalid timecode {tc:?} — expected HH:MM:SS:FF or HH:MM:SS;FF"
            ))
        });
    }
    if let Some(s) = seconds {
        return Ok(Tick((s * TICKS_PER_SECOND as f64).round() as i64));
    }
    Err(ToolResult::error(
        "missing time value — supply one of *_ticks / *_tc / *_seconds",
    ))
}

/// `HH:MM:SS:FF` or `HH:MM:SS;FF` (the `;` denotes NTSC drop-frame by
/// convention only — drop-frame frame-count compensation is NOT applied in
/// v1; documented simplification, not a hidden bug).
fn parse_timecode(tc: &str, fr: FrameRate) -> Option<Tick> {
    let sep_pos = tc.rfind([':', ';'])?;
    let hms = &tc[..sep_pos];
    let ff_str = &tc[sep_pos + 1..];
    let parts: Vec<&str> = hms.split(':').collect();
    if parts.len() != 3 {
        return None;
    }
    let h: i64 = parts[0].parse().ok()?;
    let m: i64 = parts[1].parse().ok()?;
    let s: i64 = parts[2].parse().ok()?;
    let ff: i64 = ff_str.parse().ok()?;
    let base = Tick::from_seconds(h * 3600 + m * 60 + s);
    let frame = Tick(ff * fr.ticks_per_frame().0);
    Some(base + frame)
}

/// Which `(SequenceId, TrackId)` owns a clip — a plain project scan, not edit
/// logic (mirrors `commands.rs::find_clip_mut`'s search, read-only here).
fn locate_clip(p: &TimelineProject, clip: ClipId) -> Option<(SequenceId, TrackId)> {
    for (sid, s) in &p.sequences {
        for t in s.video_tracks.iter().chain(s.audio_tracks.iter()) {
            if t.clips.iter().any(|c| c.id == clip) {
                return Some((*sid, t.id));
            }
        }
    }
    None
}

/// Which `SequenceId` owns a track.
fn locate_track(p: &TimelineProject, track: TrackId) -> Option<SequenceId> {
    p.sequences
        .iter()
        .find(|(_, s)| s.track(track).is_some())
        .map(|(sid, _)| *sid)
}

fn map_edit_error(e: EditError) -> ToolResult {
    match e {
        EditError::NoProject => ToolResult::error("no timeline project"),
        EditError::NoSequence(id) => ToolResult::error(format!("sequence {id} not found")),
        EditError::NoTrack(id) => ToolResult::error(format!("track {id} not found")),
        EditError::NoClip(id) => ToolResult::error(format!("clip {id} not found")),
        EditError::NoAsset(id) => ToolResult::error(format!("asset {id} not found")),
        EditError::Overlap => ToolResult::error("edit would overlap another clip on the track"),
        EditError::NonPositiveDuration => {
            err_code("TickOutOfRange", "resulting clip duration must be > 0")
        }
        EditError::InvalidSplit => err_code(
            "TickOutOfRange",
            "split point must be strictly inside the clip",
        ),
        EditError::IndexOutOfRange => ToolResult::error("index out of range"),
        EditError::SequenceCycle => {
            err_code("CycleDetected", "edit would create a nested-sequence cycle")
        }
        other => ToolResult::error(format!("{other}")),
    }
}

/// Read the clip currently on `(seq, track)` — callers have already resolved
/// addressing via `locate_clip`.
fn find_clip(
    project: &TimelineProject,
    seq: SequenceId,
    track: TrackId,
    clip: ClipId,
) -> Option<&Clip> {
    project
        .sequences
        .get(&seq)?
        .track(track)?
        .clips
        .iter()
        .find(|c| c.id == clip)
}

// ─── Sequence (10 §3.2) ─────────────────────────────────────────────────────

pub async fn create_sequence(state: &AppState, args: CreateSequenceArgs) -> ToolResult {
    tracing::debug!("tool: create_sequence {}", args.name);
    if args.formats.is_empty() {
        return ToolResult::error("formats must have at least one entry");
    }
    let first = &args.formats[0];
    let mut seq = photonic_core::timeline::Sequence::new(
        &args.name,
        args.frame_rate,
        first.width,
        first.height,
    );
    seq.formats = args.formats;
    let seq_id = seq.id;

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    let needs_project = doc.timeline.is_none();
    let mut cmds = Vec::new();
    if needs_project {
        cmds.push(Command::Timeline(ops::create_project()));
    }
    cmds.push(Command::Timeline(ops::add_sequence(seq)));
    let cmd = if cmds.len() == 1 {
        cmds.into_iter().next().unwrap()
    } else {
        Command::Batch(cmds)
    };
    history.execute_discrete(cmd, &mut doc);

    ToolResult::text(format!("Created sequence \"{}\"", args.name))
        .with_data(json!({ "sequence_id": seq_id }))
}

pub async fn delete_sequence(state: &AppState, args: DeleteSequenceArgs) -> ToolResult {
    tracing::debug!("tool: delete_sequence {}", args.sequence_id);
    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    let Some(project) = doc.timeline.as_ref() else {
        return ToolResult::error("no timeline project");
    };
    // Dangling-ref guard (10 §3.2): refuse if any clip elsewhere nests this
    // sequence. `ops::remove_sequence` does not check this itself (core
    // gap); this is a read-only precondition scan, not edit logic — mirrors
    // `ops::nests_into`'s existing cycle check used by `insert_clip`.
    let referenced = project.sequences.values().any(|s| {
        s.video_tracks.iter().chain(s.audio_tracks.iter()).any(|t| {
            t.clips.iter().any(|c| {
                matches!(&c.source, ClipSource::NestedSequence { sequence } if *sequence == args.sequence_id)
            })
        })
    });
    if referenced {
        return err_code(
            "CycleDetected",
            format!(
                "sequence {} is referenced by a NestedSequence clip elsewhere — remove that clip first",
                args.sequence_id
            ),
        );
    }
    match ops::remove_sequence(project, args.sequence_id) {
        Ok(cmd) => {
            history.execute_discrete(Command::Timeline(cmd), &mut doc);
            ToolResult::text("Deleted sequence")
        }
        Err(e) => map_edit_error(e),
    }
}

pub async fn list_sequences(state: &AppState, _args: ListSequencesArgs) -> ToolResult {
    tracing::debug!("tool: list_sequences");
    let doc = state.document.lock().await;
    let Some(project) = doc.timeline.as_ref() else {
        return ToolResult::text("No timeline project yet").with_data(json!({ "sequences": [] }));
    };
    let list: Vec<_> = project
        .sequence_order
        .iter()
        .filter_map(|id| project.sequences.get(id))
        .map(|s| {
            json!({
                "sequence_id": s.id,
                "name": s.name,
                "frame_rate": s.frame_rate,
                "formats": s.formats,
                "active_format": s.active_format,
                "video_track_count": s.video_tracks.len(),
                "audio_track_count": s.audio_tracks.len(),
                "is_active": project.active_sequence == Some(s.id),
            })
        })
        .collect();
    ToolResult::text(format!("{} sequence(s)", list.len())).with_data(json!({ "sequences": list }))
}

pub async fn set_active_sequence(state: &AppState, args: SetActiveSequenceArgs) -> ToolResult {
    tracing::debug!("tool: set_active_sequence");
    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    let Some(project) = doc.timeline.as_ref() else {
        return ToolResult::error("no timeline project");
    };
    if let Some(id) = args.sequence_id {
        if !project.sequences.contains_key(&id) {
            return ToolResult::error(format!("sequence {id} not found"));
        }
    }
    let cmd = ops::set_active_sequence(project, args.sequence_id);
    history.execute_discrete(Command::Timeline(cmd), &mut doc);
    ToolResult::text("Active sequence updated")
}

pub async fn set_sequence_format(state: &AppState, args: SetSequenceFormatArgs) -> ToolResult {
    tracing::debug!("tool: set_sequence_format {}", args.sequence_id);
    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    let Some(project) = doc.timeline.as_ref() else {
        return ToolResult::error("no timeline project");
    };
    let Some(seq) = project.sequences.get(&args.sequence_id) else {
        return ToolResult::error(format!("sequence {} not found", args.sequence_id));
    };
    let op = match args.op {
        FormatOpKind::Add => {
            let Some(format) = args.format else {
                return ToolResult::error("format is required for op=add");
            };
            FormatOp::Add { format }
        }
        FormatOpKind::Update => {
            let (Some(idx), Some(new)) = (args.format_index, args.format) else {
                return ToolResult::error("format_index and format are required for op=update");
            };
            let Some(old) = seq.formats.get(idx).cloned() else {
                return ToolResult::error(format!("format_index {idx} out of range"));
            };
            FormatOp::Update {
                index: idx,
                old,
                new,
            }
        }
        FormatOpKind::Remove => {
            let Some(idx) = args.format_index else {
                return ToolResult::error("format_index is required for op=remove");
            };
            if seq.formats.len() <= 1 {
                return ToolResult::error(
                    "cannot remove the last format — a sequence needs at least one",
                );
            }
            let Some(format) = seq.formats.get(idx).cloned() else {
                return ToolResult::error(format!("format_index {idx} out of range"));
            };
            FormatOp::Remove { index: idx, format }
        }
    };
    let cmd = ops::set_sequence_format(args.sequence_id, op);
    history.execute_discrete(Command::Timeline(cmd), &mut doc);
    ToolResult::text("Sequence format updated")
}

pub async fn set_active_format(state: &AppState, args: SetActiveFormatArgs) -> ToolResult {
    tracing::debug!("tool: set_active_format {}", args.sequence_id);
    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    let Some(project) = doc.timeline.as_ref() else {
        return ToolResult::error("no timeline project");
    };
    match ops::set_active_format(project, args.sequence_id, args.format_index) {
        Ok(cmd) => {
            history.execute_discrete(Command::Timeline(cmd), &mut doc);
            ToolResult::text("Active format updated")
        }
        Err(e) => map_edit_error(e),
    }
}

// ─── Track (10 §3.3) ────────────────────────────────────────────────────────

pub async fn add_track(state: &AppState, args: AddTrackArgs) -> ToolResult {
    tracing::debug!("tool: add_track {}", args.sequence_id);
    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    let Some(project) = doc.timeline.as_ref() else {
        return ToolResult::error("no timeline project");
    };
    let name = args.name.unwrap_or_else(|| match args.kind {
        photonic_core::timeline::TrackKind::Video => "Video".into(),
        photonic_core::timeline::TrackKind::Audio => "Audio".into(),
    });
    let track = Track::new(args.kind, name);
    let track_id = track.id;
    match ops::add_track(project, args.sequence_id, track, args.index) {
        Ok(cmd) => {
            history.execute_discrete(Command::Timeline(cmd), &mut doc);
            ToolResult::text("Added track").with_data(json!({ "track_id": track_id }))
        }
        Err(e) => map_edit_error(e),
    }
}

pub async fn remove_track(state: &AppState, args: RemoveTrackArgs) -> ToolResult {
    tracing::debug!("tool: remove_track {}", args.track_id);
    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    let Some(project) = doc.timeline.as_ref() else {
        return ToolResult::error("no timeline project");
    };
    let Some(seq_id) = locate_track(project, args.track_id) else {
        return ToolResult::error(format!("track {} not found", args.track_id));
    };
    match ops::remove_track(project, seq_id, args.track_id) {
        Ok(cmd) => {
            history.execute_discrete(Command::Timeline(cmd), &mut doc);
            ToolResult::text("Removed track")
        }
        Err(e) => map_edit_error(e),
    }
}

pub async fn set_track_prop(state: &AppState, args: SetTrackPropArgs) -> ToolResult {
    tracing::debug!("tool: set_track_prop {}", args.track_id);
    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    let Some(project) = doc.timeline.as_ref() else {
        return ToolResult::error("no timeline project");
    };
    let Some(seq_id) = locate_track(project, args.track_id) else {
        return ToolResult::error(format!("track {} not found", args.track_id));
    };
    let seq = project.sequences.get(&seq_id).unwrap();
    let Some(t) = seq.track(args.track_id) else {
        return ToolResult::error(format!("track {} not found", args.track_id));
    };
    let mut new_settings = TrackSettings::of(t);
    if let Some(n) = args.name {
        new_settings.name = n;
    }
    if let Some(e) = args.enabled {
        new_settings.enabled = e;
    }
    if let Some(l) = args.locked {
        new_settings.locked = l;
    }
    if let Some(h) = args.height_px {
        new_settings.height_px = h;
    }
    match ops::set_track_prop(project, seq_id, args.track_id, new_settings) {
        Ok(cmd) => {
            history.execute_discrete(Command::Timeline(cmd), &mut doc);
            ToolResult::text("Updated track")
        }
        Err(e) => map_edit_error(e),
    }
}

pub async fn reorder_track(state: &AppState, args: ReorderTrackArgs) -> ToolResult {
    tracing::debug!("tool: reorder_track {}", args.track_id);
    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    let Some(project) = doc.timeline.as_ref() else {
        return ToolResult::error("no timeline project");
    };
    let Some(seq_id) = locate_track(project, args.track_id) else {
        return ToolResult::error(format!("track {} not found", args.track_id));
    };
    match ops::reorder_track(project, seq_id, args.track_id, args.new_index) {
        Ok(cmds) => {
            history.execute_discrete(
                Command::Batch(cmds.into_iter().map(Command::Timeline).collect()),
                &mut doc,
            );
            ToolResult::text("Reordered track")
        }
        Err(e) => map_edit_error(e),
    }
}

// ─── Clip edit ops (10 §3.4) ────────────────────────────────────────────────

fn to_clip_source(project: &TimelineProject, arg: ClipSourceArg) -> Result<ClipSource, ToolResult> {
    match arg {
        ClipSourceArg::Asset { asset_id } => {
            if !project.media.assets.contains_key(&asset_id) {
                return Err(ToolResult::error(format!("asset {asset_id} not found")));
            }
            check_asset_offline(project, asset_id)?;
            Ok(ClipSource::Asset { asset: asset_id })
        }
        ClipSourceArg::Vector { asset_id } => {
            if !project.media.assets.contains_key(&asset_id) {
                return Err(ToolResult::error(format!("asset {asset_id} not found")));
            }
            Ok(ClipSource::Vector { asset: asset_id })
        }
        ClipSourceArg::NestedSequence { sequence_id } => {
            if !project.sequences.contains_key(&sequence_id) {
                return Err(ToolResult::error(format!(
                    "sequence {sequence_id} not found"
                )));
            }
            Ok(ClipSource::NestedSequence {
                sequence: sequence_id,
            })
        }
        ClipSourceArg::SolidColor { color } => {
            let c = Color::from_hex(&color).ok_or_else(|| {
                ToolResult::error(format!(
                    "invalid color {color:?} — expected #rrggbb or #rrggbbaa"
                ))
            })?;
            Ok(ClipSource::SolidColor { color: c })
        }
        ClipSourceArg::Adjustment => Ok(ClipSource::Adjustment),
    }
}

fn check_asset_offline(project: &TimelineProject, asset_id: AssetId) -> Result<(), ToolResult> {
    if let Some(a) = project.media.assets.get(&asset_id) {
        if let photonic_core::timeline::AssetSource::File { path, .. } = &a.source {
            if !path.exists() {
                return Err(err_code(
                    "AssetOffline",
                    format!("asset {asset_id} file is unreachable: {}", path.display()),
                ));
            }
        }
    }
    Ok(())
}

pub async fn insert_clip(state: &AppState, args: InsertClipArgs) -> ToolResult {
    tracing::debug!("tool: insert_clip on track {}", args.track_id);
    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    let Some(project) = doc.timeline.as_ref() else {
        return ToolResult::error("no timeline project");
    };
    let Some(seq_id) = locate_track(project, args.track_id) else {
        return ToolResult::error(format!("track {} not found", args.track_id));
    };
    let fr = project.sequences.get(&seq_id).unwrap().frame_rate;

    let start = match resolve_tick(
        args.start_ticks,
        args.start_tc.as_deref(),
        args.start_seconds,
        Some(fr),
    ) {
        Ok(t) => t,
        Err(e) => return e,
    };
    let has_source_in = args.source_in_ticks.is_some()
        || args.source_in_tc.is_some()
        || args.source_in_seconds.is_some();
    let source_in = if has_source_in {
        match resolve_tick(
            args.source_in_ticks,
            args.source_in_tc.as_deref(),
            args.source_in_seconds,
            Some(fr),
        ) {
            Ok(t) => t,
            Err(e) => return e,
        }
    } else {
        Tick::ZERO
    };
    if args.duration_ticks <= 0 {
        return err_code("TickOutOfRange", "duration_ticks must be > 0");
    }
    let source = match to_clip_source(project, args.source) {
        Ok(s) => s,
        Err(e) => return e,
    };

    let mut clip = Clip::new(source, start, Tick(args.duration_ticks));
    clip.source_in = source_in;
    if let Some(name) = args.name {
        clip.name = name;
    }
    let clip_id = clip.id;

    match ops::insert_clip(project, seq_id, args.track_id, clip) {
        Ok(cmd) => {
            history.execute_discrete(Command::Timeline(cmd), &mut doc);
            ToolResult::text("Inserted clip").with_data(json!({ "clip_id": clip_id }))
        }
        Err(e) => map_edit_error(e),
    }
}

pub async fn move_clip(state: &AppState, args: MoveClipArgs) -> ToolResult {
    tracing::debug!("tool: move_clip {}", args.clip_id);
    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    let Some(project) = doc.timeline.as_ref() else {
        return ToolResult::error("no timeline project");
    };
    let Some((seq_id, track_id)) = locate_clip(project, args.clip_id) else {
        return ToolResult::error(format!("clip {} not found", args.clip_id));
    };
    if let Some(new_track) = args.new_track_id {
        if new_track != track_id {
            return err_code(
                "NotSupportedV1",
                "cross-track move_clip is not supported in v1 (TimelineCmd::MoveClip has no track-change field) — use remove_clip then insert_clip on the target track instead",
            );
        }
    }
    let fr = project.sequences.get(&seq_id).unwrap().frame_rate;
    let new_start = match resolve_tick(
        args.new_start_ticks,
        args.new_start_tc.as_deref(),
        args.new_start_seconds,
        Some(fr),
    ) {
        Ok(t) => t,
        Err(e) => return e,
    };
    match ops::move_clip(project, seq_id, track_id, args.clip_id, new_start) {
        Ok(cmd) => {
            history.execute_discrete(Command::Timeline(cmd), &mut doc);
            ToolResult::text("Moved clip")
        }
        Err(e) => map_edit_error(e),
    }
}

pub async fn trim_clip(state: &AppState, args: TrimClipArgs) -> ToolResult {
    tracing::debug!("tool: trim_clip {}", args.clip_id);
    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    let Some(project) = doc.timeline.as_ref() else {
        return ToolResult::error("no timeline project");
    };
    let Some((seq_id, track_id)) = locate_clip(project, args.clip_id) else {
        return ToolResult::error(format!("clip {} not found", args.clip_id));
    };
    let fr = project.sequences.get(&seq_id).unwrap().frame_rate;
    let new_tick = match resolve_tick(
        args.new_ticks,
        args.new_tc.as_deref(),
        args.new_seconds,
        Some(fr),
    ) {
        Ok(t) => t,
        Err(e) => return e,
    };
    let Some(clip) = find_clip(project, seq_id, track_id, args.clip_id) else {
        return ToolResult::error(format!("clip {} not found", args.clip_id));
    };
    let new_timing = match args.edge {
        ClipEdge::In => ClipTiming {
            start: new_tick,
            duration: clip.end() - new_tick,
            source_in: clip.source_in + (new_tick - clip.start),
        },
        ClipEdge::Out => ClipTiming {
            start: clip.start,
            duration: new_tick - clip.start,
            source_in: clip.source_in,
        },
    };
    match ops::trim_clip(project, seq_id, track_id, args.clip_id, new_timing) {
        Ok(cmd) => {
            history.execute_discrete(Command::Timeline(cmd), &mut doc);
            ToolResult::text("Trimmed clip")
        }
        Err(e) => map_edit_error(e),
    }
}

pub async fn split_clip(state: &AppState, args: SplitClipArgs) -> ToolResult {
    tracing::debug!("tool: split_clip {}", args.clip_id);
    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    let Some(project) = doc.timeline.as_ref() else {
        return ToolResult::error("no timeline project");
    };
    let Some((seq_id, track_id)) = locate_clip(project, args.clip_id) else {
        return ToolResult::error(format!("clip {} not found", args.clip_id));
    };
    let fr = project.sequences.get(&seq_id).unwrap().frame_rate;
    let at = match resolve_tick(
        args.at_ticks,
        args.at_tc.as_deref(),
        args.at_seconds,
        Some(fr),
    ) {
        Ok(t) => t,
        Err(e) => return e,
    };
    match ops::split_clip(project, seq_id, track_id, args.clip_id, at) {
        Ok(cmd) => {
            let new_clip_id = match &cmd {
                TimelineCmd::SplitClip { new_clip_id, .. } => Some(*new_clip_id),
                _ => None,
            };
            history.execute_discrete(Command::Timeline(cmd), &mut doc);
            ToolResult::text("Split clip").with_data(json!({ "new_clip_id": new_clip_id }))
        }
        Err(e) => map_edit_error(e),
    }
}

pub async fn remove_clip(state: &AppState, args: RemoveClipArgs) -> ToolResult {
    tracing::debug!("tool: remove_clip {}", args.clip_id);
    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    let Some(project) = doc.timeline.as_ref() else {
        return ToolResult::error("no timeline project");
    };
    let Some((seq_id, track_id)) = locate_clip(project, args.clip_id) else {
        return ToolResult::error(format!("clip {} not found", args.clip_id));
    };
    if args.ripple {
        match ops::ripple_delete(project, seq_id, track_id, args.clip_id) {
            Ok(cmds) => {
                history.execute_discrete(
                    Command::Batch(cmds.into_iter().map(Command::Timeline).collect()),
                    &mut doc,
                );
                ToolResult::text("Removed clip (rippled)")
            }
            Err(e) => map_edit_error(e),
        }
    } else {
        match ops::remove_clip(project, seq_id, track_id, args.clip_id) {
            Ok(cmd) => {
                history.execute_discrete(Command::Timeline(cmd), &mut doc);
                ToolResult::text("Removed clip")
            }
            Err(e) => map_edit_error(e),
        }
    }
}

pub async fn roll_edit(state: &AppState, args: RollEditArgs) -> ToolResult {
    tracing::debug!("tool: roll_edit {} / {}", args.clip_id_a, args.clip_id_b);
    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    let Some(project) = doc.timeline.as_ref() else {
        return ToolResult::error("no timeline project");
    };
    let Some((seq_id, track_a)) = locate_clip(project, args.clip_id_a) else {
        return ToolResult::error(format!("clip {} not found", args.clip_id_a));
    };
    let Some((_, track_b)) = locate_clip(project, args.clip_id_b) else {
        return ToolResult::error(format!("clip {} not found", args.clip_id_b));
    };
    if track_a != track_b {
        return ToolResult::error("roll_edit requires both clips to be on the same track");
    }
    match ops::roll_edit(
        project,
        seq_id,
        track_a,
        args.clip_id_a,
        args.clip_id_b,
        Tick(args.delta_ticks),
    ) {
        Ok(cmd) => {
            history.execute_discrete(Command::Timeline(cmd), &mut doc);
            ToolResult::text("Rolled edit")
        }
        Err(e) => map_edit_error(e),
    }
}

pub async fn slip_clip(state: &AppState, args: SlipClipArgs) -> ToolResult {
    tracing::debug!("tool: slip_clip {}", args.clip_id);
    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    let Some(project) = doc.timeline.as_ref() else {
        return ToolResult::error("no timeline project");
    };
    let Some((seq_id, track_id)) = locate_clip(project, args.clip_id) else {
        return ToolResult::error(format!("clip {} not found", args.clip_id));
    };
    let Some(clip) = find_clip(project, seq_id, track_id, args.clip_id) else {
        return ToolResult::error(format!("clip {} not found", args.clip_id));
    };
    let new_source_in = clip.source_in + Tick(args.delta_ticks);
    match ops::slip_clip(project, seq_id, track_id, args.clip_id, new_source_in) {
        Ok(cmd) => {
            history.execute_discrete(Command::Timeline(cmd), &mut doc);
            ToolResult::text("Slipped clip")
        }
        Err(e) => map_edit_error(e),
    }
}

pub async fn slide_clip(state: &AppState, args: SlideClipArgs) -> ToolResult {
    tracing::debug!("tool: slide_clip {}", args.clip_id);
    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    let Some(project) = doc.timeline.as_ref() else {
        return ToolResult::error("no timeline project");
    };
    let Some((seq_id, track_id)) = locate_clip(project, args.clip_id) else {
        return ToolResult::error(format!("clip {} not found", args.clip_id));
    };
    match ops::slide_clip(
        project,
        seq_id,
        track_id,
        args.clip_id,
        Tick(args.delta_ticks),
    ) {
        Ok(cmd) => {
            history.execute_discrete(Command::Timeline(cmd), &mut doc);
            ToolResult::text("Slid clip")
        }
        Err(e) => map_edit_error(e),
    }
}

// ─── Clip properties (10 §3.5) ──────────────────────────────────────────────

pub async fn set_clip_prop(state: &AppState, args: SetClipPropArgs) -> ToolResult {
    tracing::debug!("tool: set_clip_prop {}", args.clip_id);
    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    let Some(project) = doc.timeline.as_ref() else {
        return ToolResult::error("no timeline project");
    };
    let Some((seq_id, track_id)) = locate_clip(project, args.clip_id) else {
        return ToolResult::error(format!("clip {} not found", args.clip_id));
    };
    let Some(clip) = find_clip(project, seq_id, track_id, args.clip_id) else {
        return ToolResult::error(format!("clip {} not found", args.clip_id));
    };
    let mut new_clip = clip.clone();
    if let Some(name) = args.name {
        new_clip.name = name;
    }
    if let Some(t) = args.transform {
        new_clip.transform.base = t;
    }
    if let Some(r) = args.reframe {
        match r.transform {
            Some(t) => {
                new_clip.reframe.insert(r.format_index, t);
            }
            None => {
                new_clip.reframe.remove(&r.format_index);
            }
        }
    }
    if let Some(en) = args.enabled {
        new_clip.enabled = en;
    }
    match ops::set_clip_prop(project, seq_id, track_id, new_clip) {
        Ok(cmd) => {
            history.execute_discrete(Command::Timeline(cmd), &mut doc);
            ToolResult::text("Updated clip")
        }
        Err(e) => map_edit_error(e),
    }
}

pub async fn set_clip_speed(state: &AppState, args: SetClipSpeedArgs) -> ToolResult {
    tracing::debug!("tool: set_clip_speed {}", args.clip_id);
    if args.ratio.den == 0 {
        return ToolResult::error("ratio.den must be > 0");
    }
    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    let Some(project) = doc.timeline.as_ref() else {
        return ToolResult::error("no timeline project");
    };
    let Some((seq_id, track_id)) = locate_clip(project, args.clip_id) else {
        return ToolResult::error(format!("clip {} not found", args.clip_id));
    };
    let Some(clip) = find_clip(project, seq_id, track_id, args.clip_id) else {
        return ToolResult::error(format!("clip {} not found", args.clip_id));
    };
    let mut new_clip = clip.clone();
    new_clip.speed = SpeedMap::Constant(Ratio::new(args.ratio.num, args.ratio.den));
    match ops::set_clip_prop(project, seq_id, track_id, new_clip) {
        Ok(cmd) => {
            history.execute_discrete(Command::Timeline(cmd), &mut doc);
            ToolResult::text("Updated clip speed")
        }
        Err(e) => map_edit_error(e),
    }
}

pub async fn set_transition(state: &AppState, args: SetTransitionArgs) -> ToolResult {
    tracing::debug!("tool: set_transition {}", args.clip_id);
    if let Some(t) = &args.transition {
        if t.duration_ticks <= 0 {
            return err_code("TickOutOfRange", "transition duration_ticks must be > 0");
        }
    }
    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    let Some(project) = doc.timeline.as_ref() else {
        return ToolResult::error("no timeline project");
    };
    let Some((seq_id, track_id)) = locate_clip(project, args.clip_id) else {
        return ToolResult::error(format!("clip {} not found", args.clip_id));
    };
    let Some(clip) = find_clip(project, seq_id, track_id, args.clip_id) else {
        return ToolResult::error(format!("clip {} not found", args.clip_id));
    };
    let mut new_clip = clip.clone();
    let transition = args.transition.map(|t| Transition {
        kind: t.kind,
        duration: Tick(t.duration_ticks),
        params: t.params,
    });
    match args.edge {
        ClipEdge::In => new_clip.transition_in = transition,
        ClipEdge::Out => new_clip.transition_out = transition,
    }
    match ops::set_clip_prop(project, seq_id, track_id, new_clip) {
        Ok(cmd) => {
            history.execute_discrete(Command::Timeline(cmd), &mut doc);
            ToolResult::text("Updated transition")
        }
        Err(e) => map_edit_error(e),
    }
}

pub async fn list_clips(state: &AppState, args: ListClipsArgs) -> ToolResult {
    tracing::debug!("tool: list_clips");
    let doc = state.document.lock().await;
    let Some(project) = doc.timeline.as_ref() else {
        return ToolResult::text("No timeline project yet").with_data(json!({ "clips": [] }));
    };
    let range = match (args.range_start_ticks, args.range_end_ticks) {
        (Some(s), Some(e)) => Some((Tick(s), Tick(e))),
        _ => None,
    };
    let mut out = Vec::new();
    for (sid, seq) in &project.sequences {
        if let Some(f) = args.sequence_id {
            if f != *sid {
                continue;
            }
        }
        for t in seq.video_tracks.iter().chain(seq.audio_tracks.iter()) {
            if let Some(f) = args.track_id {
                if f != t.id {
                    continue;
                }
            }
            for c in &t.clips {
                if let Some((rs, re)) = range {
                    if !(c.start < re && rs < c.end()) {
                        continue;
                    }
                }
                out.push(json!({
                    "clip_id": c.id,
                    "name": c.name,
                    "sequence_id": sid,
                    "track_id": t.id,
                    "start_ticks": c.start.0,
                    "duration_ticks": c.duration.0,
                    "enabled": c.enabled,
                }));
            }
        }
    }
    ToolResult::text(format!("{} clip(s)", out.len())).with_data(json!({ "clips": out }))
}

pub async fn get_clip(state: &AppState, args: GetClipArgs) -> ToolResult {
    tracing::debug!("tool: get_clip {}", args.clip_id);
    let doc = state.document.lock().await;
    let Some(project) = doc.timeline.as_ref() else {
        return ToolResult::error("no timeline project");
    };
    let Some((seq_id, track_id)) = locate_clip(project, args.clip_id) else {
        return ToolResult::error(format!("clip {} not found", args.clip_id));
    };
    let Some(clip) = find_clip(project, seq_id, track_id, args.clip_id) else {
        return ToolResult::error(format!("clip {} not found", args.clip_id));
    };
    ToolResult::text(format!("Clip \"{}\"", clip.name)).with_data(json!({
        "sequence_id": seq_id,
        "track_id": track_id,
        "clip": clip,
    }))
}

// ─── Effects (10 §3.6) ───────────────────────────────────────────────────────

pub async fn add_effect(state: &AppState, args: AddEffectArgs) -> ToolResult {
    tracing::debug!("tool: add_effect {}", args.clip_id);
    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    let Some(project) = doc.timeline.as_ref() else {
        return ToolResult::error("no timeline project");
    };
    let Some((seq_id, track_id)) = locate_clip(project, args.clip_id) else {
        return ToolResult::error(format!("clip {} not found", args.clip_id));
    };
    let effect = ClipEffect::new(args.kind);
    match ops::add_effect(project, seq_id, track_id, args.clip_id, effect, args.index) {
        Ok(cmd) => {
            history.execute_discrete(Command::Timeline(cmd), &mut doc);
            ToolResult::text("Added effect")
        }
        Err(e) => map_edit_error(e),
    }
}

pub async fn remove_effect(state: &AppState, args: RemoveEffectArgs) -> ToolResult {
    tracing::debug!("tool: remove_effect {}", args.clip_id);
    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    let Some(project) = doc.timeline.as_ref() else {
        return ToolResult::error("no timeline project");
    };
    let Some((seq_id, track_id)) = locate_clip(project, args.clip_id) else {
        return ToolResult::error(format!("clip {} not found", args.clip_id));
    };
    match ops::remove_effect(project, seq_id, track_id, args.clip_id, args.effect_index) {
        Ok(cmd) => {
            history.execute_discrete(Command::Timeline(cmd), &mut doc);
            ToolResult::text("Removed effect")
        }
        Err(e) => map_edit_error(e),
    }
}

pub async fn reorder_effects(state: &AppState, args: ReorderEffectsArgs) -> ToolResult {
    tracing::debug!("tool: reorder_effects {}", args.clip_id);
    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    let Some(project) = doc.timeline.as_ref() else {
        return ToolResult::error("no timeline project");
    };
    let Some((seq_id, track_id)) = locate_clip(project, args.clip_id) else {
        return ToolResult::error(format!("clip {} not found", args.clip_id));
    };
    match ops::reorder_effects(project, seq_id, track_id, args.clip_id, args.new_order) {
        Ok(cmd) => {
            history.execute_discrete(Command::Timeline(cmd), &mut doc);
            ToolResult::text("Reordered effects")
        }
        Err(e) => map_edit_error(e),
    }
}

pub async fn set_effect_param(state: &AppState, args: SetEffectParamArgs) -> ToolResult {
    tracing::debug!(
        "tool: set_effect_param {} [{}]",
        args.clip_id,
        args.effect_index
    );
    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    let Some(project) = doc.timeline.as_ref() else {
        return ToolResult::error("no timeline project");
    };
    let Some((seq_id, track_id)) = locate_clip(project, args.clip_id) else {
        return ToolResult::error(format!("clip {} not found", args.clip_id));
    };
    let Some(clip) = find_clip(project, seq_id, track_id, args.clip_id) else {
        return ToolResult::error(format!("clip {} not found", args.clip_id));
    };
    let mut new_clip = clip.clone();
    let Some(effect) = new_clip.effects.get_mut(args.effect_index) else {
        return ToolResult::error(format!("effect index {} out of range", args.effect_index));
    };
    if args.path == "enabled" {
        match args.value {
            photonic_core::timeline::PropValue::Bool(b) => effect.enabled = b,
            _ => return ToolResult::error("path \"enabled\" requires a bool value"),
        }
    } else {
        effect.params.base.set(args.path.as_str(), args.value);
    }
    match ops::set_clip_prop(project, seq_id, track_id, new_clip) {
        Ok(cmd) => {
            history.execute_discrete(Command::Timeline(cmd), &mut doc);
            ToolResult::text("Updated effect param")
        }
        Err(e) => map_edit_error(e),
    }
}

pub async fn list_effect_kinds(_state: &AppState, _args: ListEffectKindsArgs) -> ToolResult {
    tracing::debug!("tool: list_effect_kinds");
    use photonic_core::timeline::{prop_registry, EffectKind};
    let kinds = [
        EffectKind::Blur,
        EffectKind::Sharpen,
        EffectKind::Glow,
        EffectKind::ChromaKey,
        EffectKind::LumaKey,
        EffectKind::Invert,
        EffectKind::MaskShapeGen,
    ];
    let out: Vec<_> = kinds
        .iter()
        .map(|k| {
            let entries: Vec<_> = prop_registry::entries(k.target_kind())
                .iter()
                .map(|e| json!({ "path": e.path, "value_kind": format!("{:?}", e.kind), "range": e.range }))
                .collect();
            json!({ "kind": k, "params": entries })
        })
        .collect();
    ToolResult::text(format!("{} effect kind(s)", out.len()))
        .with_data(json!({ "effect_kinds": out }))
}

// ─── Keyframes (10 §3.7) ─────────────────────────────────────────────────────

fn to_anim_target(arg: &AnimTargetArg) -> AnimTarget {
    match arg {
        AnimTargetArg::ClipTransform { clip_id } => AnimTarget::ClipTransform { clip: *clip_id },
        AnimTargetArg::ClipEffect {
            clip_id,
            effect_index,
        } => AnimTarget::ClipEffect {
            clip: *clip_id,
            effect_index: *effect_index,
        },
    }
}

/// Read-only resolution of a target's owning clip and property-track lane.
fn read_target<'a>(
    project: &'a TimelineProject,
    target: &AnimTargetArg,
) -> Result<(&'a Clip, &'a [photonic_core::timeline::PropertyTrack]), ToolResult> {
    let clip_id = target.clip_id();
    let Some((seq_id, track_id)) = locate_clip(project, clip_id) else {
        return Err(ToolResult::error(format!("clip {clip_id} not found")));
    };
    let clip = find_clip(project, seq_id, track_id, clip_id)
        .ok_or_else(|| ToolResult::error(format!("clip {clip_id} not found")))?;
    match target {
        AnimTargetArg::ClipTransform { .. } => Ok((clip, clip.transform.tracks.as_slice())),
        AnimTargetArg::ClipEffect { effect_index, .. } => {
            let Some(effect) = clip.effects.get(*effect_index) else {
                return Err(ToolResult::error(format!(
                    "effect index {effect_index} out of range"
                )));
            };
            Ok((clip, effect.params.tracks.as_slice()))
        }
    }
}

fn target_frame_rate(project: &TimelineProject, clip_id: ClipId) -> Option<FrameRate> {
    let (seq_id, _) = locate_clip(project, clip_id)?;
    project.sequences.get(&seq_id).map(|s| s.frame_rate)
}

pub async fn set_keyframe(state: &AppState, args: SetKeyframeArgs) -> ToolResult {
    tracing::debug!("tool: set_keyframe {}", args.target.clip_id());
    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    let Some(project) = doc.timeline.as_ref() else {
        return ToolResult::error("no timeline project");
    };
    if let Err(e) = read_target(project, &args.target) {
        return e;
    }
    let fr = target_frame_rate(project, args.target.clip_id());
    let at = match resolve_tick(args.at_ticks, args.at_tc.as_deref(), args.at_seconds, fr) {
        Ok(t) => t,
        Err(e) => return e,
    };
    let target = to_anim_target(&args.target);
    let kf = Keyframe::new(at, args.value, args.interp);
    let cmd = ops::set_keyframe(project, target, PropPath::new(args.path), kf);
    history.execute_discrete(Command::Timeline(cmd), &mut doc);
    ToolResult::text("Set keyframe")
}

pub async fn remove_keyframe(state: &AppState, args: RemoveKeyframeArgs) -> ToolResult {
    tracing::debug!("tool: remove_keyframe {}", args.target.clip_id());
    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    let Some(project) = doc.timeline.as_ref() else {
        return ToolResult::error("no timeline project");
    };
    if let Err(e) = read_target(project, &args.target) {
        return e;
    }
    let fr = target_frame_rate(project, args.target.clip_id());
    let at = match resolve_tick(args.at_ticks, args.at_tc.as_deref(), args.at_seconds, fr) {
        Ok(t) => t,
        Err(e) => return e,
    };
    let target = to_anim_target(&args.target);
    match ops::remove_keyframe(project, target, PropPath::new(args.path), at) {
        Ok(cmd) => {
            history.execute_discrete(Command::Timeline(cmd), &mut doc);
            ToolResult::text("Removed keyframe")
        }
        Err(e) => map_edit_error(e),
    }
}

pub async fn batch_set_keyframes(state: &AppState, args: BatchSetKeyframesArgs) -> ToolResult {
    tracing::debug!("tool: batch_set_keyframes ({} ops)", args.ops.len());
    if args.ops.is_empty() {
        return ToolResult::error("ops must not be empty");
    }
    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    let Some(project) = doc.timeline.as_ref() else {
        return ToolResult::error("no timeline project");
    };
    let mut cmds = Vec::with_capacity(args.ops.len());
    for op in &args.ops {
        if let Err(e) = read_target(project, &op.target) {
            return e;
        }
        let fr = target_frame_rate(project, op.target.clip_id());
        let at = match resolve_tick(op.at_ticks, op.at_tc.as_deref(), op.at_seconds, fr) {
            Ok(t) => t,
            Err(e) => return e,
        };
        let target = to_anim_target(&op.target);
        let kf = Keyframe::new(at, op.value, op.interp);
        cmds.push(Command::Timeline(ops::set_keyframe(
            project,
            target,
            PropPath::new(op.path.clone()),
            kf,
        )));
    }
    let n = cmds.len();
    history.execute_discrete(Command::Batch(cmds), &mut doc);
    ToolResult::text(format!("Set {n} keyframe(s)"))
}

pub async fn get_keyframes(state: &AppState, args: GetKeyframesArgs) -> ToolResult {
    tracing::debug!("tool: get_keyframes {}", args.target.clip_id());
    let doc = state.document.lock().await;
    let Some(project) = doc.timeline.as_ref() else {
        return ToolResult::error("no timeline project");
    };
    match read_target(project, &args.target) {
        Ok((_, tracks)) => ToolResult::text(format!("{} property track(s)", tracks.len()))
            .with_data(json!({ "tracks": tracks })),
        Err(e) => e,
    }
}

// ─── Media (P2 subset: import/relink/list/remove) ───────────────────────────

fn guess_asset_kind(path: &std::path::Path) -> Option<AssetKind> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "mp4" | "mov" | "mkv" | "avi" | "webm" | "m4v" => AssetKind::Video,
        "mp3" | "wav" | "aac" | "flac" | "ogg" | "m4a" => AssetKind::Audio,
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "tiff" | "tif" => AssetKind::Image,
        "svg" | "photon" => AssetKind::VectorDoc,
        "cube" => AssetKind::Lut3d,
        _ => return None,
    })
}

/// A stopgap content identity (head+tail+len, `DefaultHasher`/SipHash) — NOT
/// the `xxh3` the core data model's doc comment (media.rs:6) describes as the
/// eventual relink identity; that lands with the P3 engine work. Good enough
/// to detect an exact-byte-match relink candidate now.
fn content_hash(path: &std::path::Path) -> Option<String> {
    use std::hash::{Hash, Hasher};
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    const CHUNK: u64 = 4096;
    let head_len = CHUNK.min(len) as usize;
    let mut head = vec![0u8; head_len];
    f.read_exact(&mut head).ok()?;
    let mut tail = Vec::new();
    if len > CHUNK {
        let tail_len = CHUNK.min(len);
        f.seek(SeekFrom::End(-(tail_len as i64))).ok()?;
        tail = vec![0u8; tail_len as usize];
        f.read_exact(&mut tail).ok()?;
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    len.hash(&mut hasher);
    head.hash(&mut hasher);
    tail.hash(&mut hasher);
    Some(format!("siphash64:{:016x}", hasher.finish()))
}

pub async fn import_media(state: &AppState, args: ImportMediaArgs) -> ToolResult {
    tracing::debug!("tool: import_media ({} path(s))", args.paths.len());
    if args.paths.is_empty() {
        return ToolResult::error("paths must not be empty");
    }
    let mut created = Vec::new();
    let mut cmds = Vec::new();
    for p in &args.paths {
        let path = std::path::PathBuf::from(p);
        let Some(kind) = guess_asset_kind(&path) else {
            return ToolResult::error(format!(
                "cannot infer media kind for {p:?} — unrecognized extension"
            ));
        };
        if !path.exists() {
            return err_code("AssetOffline", format!("file not found: {p}"));
        }
        let hash = content_hash(&path);
        let mut asset = photonic_core::timeline::MediaAsset::from_file(kind, path.clone());
        asset.content_hash = hash;
        let asset_id = asset.id;
        cmds.push(Command::Timeline(ops::add_asset(asset)));
        created.push(json!({ "asset_id": asset_id, "path": p, "kind": kind, "probed": false }));
    }

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    if doc.timeline.is_none() {
        cmds.insert(0, Command::Timeline(ops::create_project()));
    }
    history.execute_discrete(Command::Batch(cmds), &mut doc);

    let mut data = json!({ "assets": created });
    if args.bin.is_some() {
        data["bin_assignment_supported"] = json!(false);
    }
    ToolResult::text(format!(
        "Imported {} asset(s) — probing lands in P3 (ffprobe integration)",
        created.len()
    ))
    .with_data(data)
}

pub async fn relink_media(state: &AppState, args: RelinkMediaArgs) -> ToolResult {
    tracing::debug!("tool: relink_media {}", args.asset_id);
    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    let Some(project) = doc.timeline.as_ref() else {
        return ToolResult::error("no timeline project");
    };
    match ops::relink_asset(
        project,
        args.asset_id,
        std::path::PathBuf::from(&args.new_path),
    ) {
        Ok(cmd) => {
            history.execute_discrete(Command::Timeline(cmd), &mut doc);
            ToolResult::text("Relinked asset")
        }
        Err(e) => map_edit_error(e),
    }
}

pub async fn list_media(state: &AppState, args: ListMediaArgs) -> ToolResult {
    tracing::debug!("tool: list_media");
    let doc = state.document.lock().await;
    let Some(project) = doc.timeline.as_ref() else {
        return ToolResult::text("No timeline project yet").with_data(json!({ "assets": [] }));
    };
    let assets: Vec<_> = project
        .media
        .assets
        .values()
        .map(|a| {
            json!({
                "asset_id": a.id,
                "kind": a.kind,
                "source": a.source,
                "probed": a.probe.is_some(),
                "proxy_status": a.proxy.as_ref().map(|p| p.status),
                "content_hash": a.content_hash,
            })
        })
        .collect();
    let mut data = json!({ "assets": assets });
    if args.bin.is_some() {
        data["bin_filter_supported"] = json!(false);
    }
    ToolResult::text(format!("{} media asset(s)", assets.len())).with_data(data)
}

pub async fn remove_asset(state: &AppState, args: RemoveAssetArgs) -> ToolResult {
    tracing::debug!("tool: remove_asset {}", args.asset_id);
    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    let Some(project) = doc.timeline.as_ref() else {
        return ToolResult::error("no timeline project");
    };
    match ops::remove_asset(project, args.asset_id) {
        Ok(cmd) => {
            history.execute_discrete(Command::Timeline(cmd), &mut doc);
            ToolResult::text("Removed asset")
        }
        Err(e) => map_edit_error(e),
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::McpServerConfig;
    use photonic_core::{AuditLog, Document};
    use serde_json::{json, Value};
    use std::sync::{Arc, Mutex as StdMutex};
    use tokio::sync::Mutex;

    fn test_state() -> AppState {
        let (tx, _rx) = std::sync::mpsc::channel();
        AppState {
            document: Arc::new(Mutex::new(Document::new("t", 100.0, 100.0))),
            history: Arc::new(Mutex::new(photonic_core::history::CommandHistory::new(100))),
            capture_tx: Arc::new(StdMutex::new(tx)),
            config: McpServerConfig::default(),
            audit_log: Arc::new(StdMutex::new(AuditLog::new())),
            clipboard_ring: Arc::new(crate::handlers::clipboard::new_clipboard_ring()),
        }
    }

    /// Dispatch through the real JSON-RPC tool-call path (args deserialize +
    /// `dispatch_tool_inner` match arm + handler + audit log) — the closest
    /// in-process equivalent to an agent's `tools/call`.
    async fn call(state: &AppState, name: &str, args: Value) -> ToolResult {
        crate::dispatch::dispatch_tool(state, name, args)
            .await
            .unwrap_or_else(|e| panic!("dispatch({name}) failed: {e}"))
    }

    /// The JSON payload attached via `ToolResult::with_data` (second content item).
    fn data(r: &ToolResult) -> Value {
        match r.content.get(1) {
            Some(ContentItem::Text { text }) => serde_json::from_str(text).unwrap_or(Value::Null),
            _ => Value::Null,
        }
    }

    async fn create_track(state: &AppState, seq_id: &Value, kind: &str) -> Value {
        let r = call(
            state,
            "add_track",
            json!({ "sequence_id": seq_id, "kind": kind }),
        )
        .await;
        assert_ne!(r.is_error, Some(true), "add_track: {r:?}");
        data(&r)["track_id"].clone()
    }

    async fn create_seq_and_track(state: &AppState, kind: &str) -> (Value, Value) {
        let r = call(
            state,
            "create_sequence",
            json!({
                "name": "Test Seq", "frame_rate": {"num": 30, "den": 1},
                "formats": [{"name": "16:9", "width": 1920, "height": 1080}]
            }),
        )
        .await;
        assert_ne!(r.is_error, Some(true), "create_sequence: {r:?}");
        let seq_id = data(&r)["sequence_id"].clone();
        let track_id = create_track(state, &seq_id, kind).await;
        (seq_id, track_id)
    }

    async fn insert_solid_clip(state: &AppState, track_id: &Value, start: i64, dur: i64) -> Value {
        let r = call(
            state,
            "insert_clip",
            json!({
                "track_id": track_id, "start_ticks": start, "duration_ticks": dur,
                "source": {"kind": "solid_color", "color": "#00ff00"}
            }),
        )
        .await;
        assert_ne!(r.is_error, Some(true), "insert_clip: {r:?}");
        data(&r)["clip_id"].clone()
    }

    // ── Unit test: resolve_tick precedence (design rule 3) ──────────────────

    #[test]
    fn resolve_tick_precedence_and_missing_context() {
        let fr = FrameRate::FPS_30;
        // ticks beats tc and seconds.
        assert_eq!(
            resolve_tick(Some(100), Some("00:00:01:00"), Some(5.0), Some(fr)).unwrap(),
            Tick(100)
        );
        // tc beats seconds.
        assert_eq!(
            resolve_tick(None, Some("00:00:01:00"), Some(5.0), Some(fr)).unwrap(),
            Tick::from_seconds(1)
        );
        // seconds is the last fallback.
        assert_eq!(
            resolve_tick(None, None, Some(2.0), Some(fr)).unwrap(),
            Tick::from_seconds(2)
        );
        // tc with no sequence context -> MissingSequenceContext.
        let err = resolve_tick(None, Some("00:00:01:00"), None, None).unwrap_err();
        assert_eq!(err.is_error, Some(true));
        assert_eq!(data(&err)["error_code"], json!("MissingSequenceContext"));
        // nothing supplied -> plain error.
        assert!(resolve_tick(None, None, None, Some(fr)).is_err());
    }

    // ── Tool family: sequence → track → clip → split → list (per work order) ─

    #[tokio::test]
    async fn family_sequence_track_clip_split_list() {
        let state = test_state();
        let (_, track_id) = create_seq_and_track(&state, "video").await;

        let clip_id = insert_solid_clip(&state, &track_id, 0, 1000).await;

        let r = call(
            &state,
            "split_clip",
            json!({ "clip_id": clip_id, "at_ticks": 500 }),
        )
        .await;
        assert_ne!(r.is_error, Some(true), "split_clip: {r:?}");
        let new_clip_id = data(&r)["new_clip_id"].clone();
        assert!(!new_clip_id.is_null());

        let r = call(&state, "list_clips", json!({})).await;
        assert_ne!(r.is_error, Some(true), "list_clips: {r:?}");
        let clips = data(&r)["clips"].as_array().cloned().unwrap_or_default();
        assert_eq!(
            clips.len(),
            2,
            "expected 2 clips after split, got {clips:?}"
        );
    }

    // ── Tool family: track lifecycle ─────────────────────────────────────────

    #[tokio::test]
    async fn family_track_lifecycle() {
        let state = test_state();
        let (seq_id, track_id) = create_seq_and_track(&state, "audio").await;

        let r = call(
            &state,
            "set_track_prop",
            json!({ "track_id": track_id, "name": "Music", "locked": true }),
        )
        .await;
        assert_ne!(r.is_error, Some(true), "set_track_prop: {r:?}");

        let track_id2 = create_track(&state, &seq_id, "audio").await;
        let r = call(
            &state,
            "reorder_track",
            json!({ "track_id": track_id2, "new_index": 0 }),
        )
        .await;
        assert_ne!(r.is_error, Some(true), "reorder_track: {r:?}");

        let r = call(&state, "remove_track", json!({ "track_id": track_id })).await;
        assert_ne!(r.is_error, Some(true), "remove_track: {r:?}");
    }

    // ── Tool family: clip edit ops ────────────────────────────────────────────

    #[tokio::test]
    async fn family_clip_edit_ops() {
        let state = test_state();
        let (seq_id, track_id) = create_seq_and_track(&state, "video").await;

        let clip_a = insert_solid_clip(&state, &track_id, 0, 1000).await;
        let clip_b = insert_solid_clip(&state, &track_id, 1000, 1000).await;

        let r = call(
            &state,
            "roll_edit",
            json!({ "clip_id_a": clip_a, "clip_id_b": clip_b, "delta_ticks": 100 }),
        )
        .await;
        assert_ne!(r.is_error, Some(true), "roll_edit: {r:?}");

        let r = call(
            &state,
            "move_clip",
            json!({ "clip_id": clip_a, "new_start_ticks": 5000 }),
        )
        .await;
        assert_ne!(r.is_error, Some(true), "move_clip: {r:?}");

        // Cross-track move is a documented v1 gap — NotSupportedV1.
        let other_track = create_track(&state, &seq_id, "video").await;
        let r = call(
            &state,
            "move_clip",
            json!({ "clip_id": clip_a, "new_start_ticks": 6000, "new_track_id": other_track }),
        )
        .await;
        assert_eq!(r.is_error, Some(true));
        assert_eq!(data(&r)["error_code"], json!("NotSupportedV1"));

        let r = call(
            &state,
            "trim_clip",
            json!({ "clip_id": clip_b, "edge": "out", "new_ticks": 1500 }),
        )
        .await;
        assert_ne!(r.is_error, Some(true), "trim_clip: {r:?}");

        let r = call(
            &state,
            "slip_clip",
            json!({ "clip_id": clip_b, "delta_ticks": 10 }),
        )
        .await;
        assert_ne!(r.is_error, Some(true), "slip_clip: {r:?}");

        // Fresh isolated pair for slide_clip so it doesn't interact with clip_a/clip_b's post-roll state.
        let track2 = create_track(&state, &seq_id, "video").await;
        let clip_c = insert_solid_clip(&state, &track2, 0, 1000).await;
        let clip_d = insert_solid_clip(&state, &track2, 1000, 1000).await;
        let r = call(
            &state,
            "slide_clip",
            json!({ "clip_id": clip_d, "delta_ticks": 50 }),
        )
        .await;
        assert_ne!(r.is_error, Some(true), "slide_clip: {r:?}");

        let r = call(
            &state,
            "remove_clip",
            json!({ "clip_id": clip_c, "ripple": true }),
        )
        .await;
        assert_ne!(r.is_error, Some(true), "remove_clip ripple: {r:?}");
    }

    #[tokio::test]
    async fn insert_clip_rejects_overlap() {
        let state = test_state();
        let (_, track_id) = create_seq_and_track(&state, "video").await;
        let _ = insert_solid_clip(&state, &track_id, 0, 1000).await;
        let r = call(
            &state,
            "insert_clip",
            json!({
                "track_id": track_id, "start_ticks": 500, "duration_ticks": 1000,
                "source": {"kind": "solid_color", "color": "#0000ff"}
            }),
        )
        .await;
        assert_eq!(r.is_error, Some(true), "overlapping insert should fail");
    }

    // ── Tool family: clip properties ─────────────────────────────────────────

    #[tokio::test]
    async fn family_clip_properties() {
        let state = test_state();
        let (_, track_id) = create_seq_and_track(&state, "video").await;
        let clip_id = insert_solid_clip(&state, &track_id, 0, 1000).await;

        let r = call(
            &state,
            "set_clip_prop",
            json!({
                "clip_id": clip_id, "name": "Renamed", "enabled": false,
                "transform": {"x": 10.0, "y": 20.0, "scale_x": 1.0, "scale_y": 1.0, "rotation": 0.0, "anchor_x": 0.0, "anchor_y": 0.0, "opacity": 0.5}
            }),
        )
        .await;
        assert_ne!(r.is_error, Some(true), "set_clip_prop: {r:?}");

        let r = call(
            &state,
            "set_clip_speed",
            json!({ "clip_id": clip_id, "ratio": {"num": 2, "den": 1} }),
        )
        .await;
        assert_ne!(r.is_error, Some(true), "set_clip_speed: {r:?}");

        let r = call(
            &state,
            "set_transition",
            json!({
                "clip_id": clip_id, "edge": "in",
                "transition": {"kind": "cross_dissolve", "duration_ticks": 200}
            }),
        )
        .await;
        assert_ne!(r.is_error, Some(true), "set_transition: {r:?}");

        let r = call(&state, "get_clip", json!({ "clip_id": clip_id })).await;
        assert_ne!(r.is_error, Some(true), "get_clip: {r:?}");
        let clip_json = data(&r)["clip"].clone();
        assert_eq!(clip_json["name"], json!("Renamed"));
        assert_eq!(clip_json["enabled"], json!(false));
    }

    // ── Tool family: effects ──────────────────────────────────────────────────

    #[tokio::test]
    async fn family_effects() {
        let state = test_state();
        let (_, track_id) = create_seq_and_track(&state, "video").await;
        let clip_id = insert_solid_clip(&state, &track_id, 0, 1000).await;

        let r = call(
            &state,
            "add_effect",
            json!({ "clip_id": clip_id, "kind": "blur" }),
        )
        .await;
        assert_ne!(r.is_error, Some(true), "add_effect(blur): {r:?}");
        let r = call(
            &state,
            "add_effect",
            json!({ "clip_id": clip_id, "kind": "glow" }),
        )
        .await;
        assert_ne!(r.is_error, Some(true), "add_effect(glow): {r:?}");

        let r = call(
            &state,
            "set_effect_param",
            json!({
                "clip_id": clip_id, "effect_index": 0, "path": "params.radius",
                "value": {"t": "float", "v": 12.0}
            }),
        )
        .await;
        assert_ne!(r.is_error, Some(true), "set_effect_param: {r:?}");

        let r = call(
            &state,
            "reorder_effects",
            json!({ "clip_id": clip_id, "new_order": [1, 0] }),
        )
        .await;
        assert_ne!(r.is_error, Some(true), "reorder_effects: {r:?}");

        let r = call(
            &state,
            "remove_effect",
            json!({ "clip_id": clip_id, "effect_index": 0 }),
        )
        .await;
        assert_ne!(r.is_error, Some(true), "remove_effect: {r:?}");

        let r = call(&state, "list_effect_kinds", json!({})).await;
        assert_ne!(r.is_error, Some(true), "list_effect_kinds: {r:?}");
        let kinds = data(&r)["effect_kinds"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        assert_eq!(kinds.len(), 7);
    }

    // ── Tool family: keyframes ─────────────────────────────────────────────

    #[tokio::test]
    async fn family_keyframes() {
        let state = test_state();
        let (_, track_id) = create_seq_and_track(&state, "video").await;
        let clip_id = insert_solid_clip(&state, &track_id, 0, 1000).await;

        let r = call(
            &state,
            "set_keyframe",
            json!({
                "target": "clip_transform", "clip_id": clip_id, "path": "transform.x",
                "at_ticks": 0, "value": {"t": "float", "v": 0.0}, "interp": {"kind": "linear"}
            }),
        )
        .await;
        assert_ne!(r.is_error, Some(true), "set_keyframe: {r:?}");

        let r = call(
            &state,
            "batch_set_keyframes",
            json!({
                "ops": [
                    {"target": "clip_transform", "clip_id": clip_id, "path": "transform.x", "at_ticks": 500, "value": {"t": "float", "v": 50.0}, "interp": {"kind": "linear"}},
                    {"target": "clip_transform", "clip_id": clip_id, "path": "transform.x", "at_ticks": 1000, "value": {"t": "float", "v": 100.0}, "interp": {"kind": "hold"}}
                ]
            }),
        )
        .await;
        assert_ne!(r.is_error, Some(true), "batch_set_keyframes: {r:?}");

        let r = call(
            &state,
            "get_keyframes",
            json!({ "target": "clip_transform", "clip_id": clip_id }),
        )
        .await;
        assert_ne!(r.is_error, Some(true), "get_keyframes: {r:?}");
        let tracks = data(&r)["tracks"].as_array().cloned().unwrap_or_default();
        assert_eq!(tracks.len(), 1);
        let kfs = tracks[0]["keyframes"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        assert_eq!(kfs.len(), 3, "expected 3 keyframes, got {kfs:?}");

        let r = call(
            &state,
            "remove_keyframe",
            json!({ "target": "clip_transform", "clip_id": clip_id, "path": "transform.x", "at_ticks": 500 }),
        )
        .await;
        assert_ne!(r.is_error, Some(true), "remove_keyframe: {r:?}");
    }

    // ── Tool family: media ─────────────────────────────────────────────────

    #[tokio::test]
    async fn family_media() {
        let state = test_state();
        let tmp =
            std::env::temp_dir().join(format!("photonic_mcp_test_{}.mp4", uuid::Uuid::new_v4()));
        std::fs::write(&tmp, b"fake mp4 bytes for content-hash test").unwrap();

        let r = call(
            &state,
            "import_media",
            json!({ "paths": [tmp.to_string_lossy()] }),
        )
        .await;
        assert_ne!(r.is_error, Some(true), "import_media: {r:?}");
        let assets = data(&r)["assets"].as_array().cloned().unwrap_or_default();
        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0]["probed"], json!(false));
        let asset_id = assets[0]["asset_id"].clone();

        let r = call(&state, "list_media", json!({})).await;
        assert_ne!(r.is_error, Some(true), "list_media: {r:?}");
        let list = data(&r)["assets"].as_array().cloned().unwrap_or_default();
        assert_eq!(list.len(), 1);
        assert!(list[0]["content_hash"]
            .as_str()
            .unwrap_or_default()
            .starts_with("siphash64:"));

        let tmp2 =
            std::env::temp_dir().join(format!("photonic_mcp_test_{}.mp4", uuid::Uuid::new_v4()));
        std::fs::write(&tmp2, b"other bytes").unwrap();
        let r = call(
            &state,
            "relink_media",
            json!({ "asset_id": asset_id, "new_path": tmp2.to_string_lossy() }),
        )
        .await;
        assert_ne!(r.is_error, Some(true), "relink_media: {r:?}");

        let r = call(&state, "remove_asset", json!({ "asset_id": asset_id })).await;
        assert_ne!(r.is_error, Some(true), "remove_asset: {r:?}");

        let _ = std::fs::remove_file(&tmp);
        let _ = std::fs::remove_file(&tmp2);
    }

    #[tokio::test]
    async fn import_media_missing_file_is_asset_offline() {
        let state = test_state();
        let r = call(
            &state,
            "import_media",
            json!({ "paths": ["/nonexistent/path/does-not-exist.mp4"] }),
        )
        .await;
        assert_eq!(r.is_error, Some(true));
        assert_eq!(data(&r)["error_code"], json!("AssetOffline"));
    }

    // ── delete_sequence dangling-reference guard ─────────────────────────────

    #[tokio::test]
    async fn delete_sequence_blocks_dangling_nested_reference() {
        let state = test_state();
        let (inner_seq, _) = create_seq_and_track(&state, "video").await;
        let (_, outer_track) = create_seq_and_track(&state, "video").await;

        let r = call(
            &state,
            "insert_clip",
            json!({
                "track_id": outer_track, "start_ticks": 0, "duration_ticks": 1000,
                "source": {"kind": "nested_sequence", "sequence_id": inner_seq}
            }),
        )
        .await;
        assert_ne!(r.is_error, Some(true), "insert nested_sequence clip: {r:?}");

        let r = call(
            &state,
            "delete_sequence",
            json!({ "sequence_id": inner_seq }),
        )
        .await;
        assert_eq!(
            r.is_error,
            Some(true),
            "delete of a referenced sequence should be blocked"
        );
        assert_eq!(data(&r)["error_code"], json!("CycleDetected"));
    }

    // ── Undo round-trip through the shared CommandHistory ───────────────────

    #[tokio::test]
    async fn undo_reverts_a_timeline_edit() {
        let state = test_state();
        let r = call(
            &state,
            "create_sequence",
            json!({
                "name": "UndoSeq", "frame_rate": {"num": 30, "den": 1},
                "formats": [{"name": "16:9", "width": 1920, "height": 1080}]
            }),
        )
        .await;
        assert_ne!(r.is_error, Some(true), "create_sequence: {r:?}");
        {
            let doc = state.document.lock().await;
            assert!(doc.timeline.is_some());
            assert_eq!(doc.timeline.as_ref().unwrap().sequences.len(), 1);
        }

        let r = call(&state, "undo", json!({})).await;
        assert_ne!(r.is_error, Some(true), "undo: {r:?}");
        {
            let doc = state.document.lock().await;
            // create_sequence batches [CreateProject, AddSequence] as ONE undo
            // step (design rule 4) when the project didn't exist yet — one
            // undo call reverts both, back to no timeline project at all.
            assert!(
                doc.timeline.is_none(),
                "expected timeline project to be undone"
            );
        }
    }

    // ── Schema/args/dispatch consistency (10 §9.3) ───────────────────────────

    const VIDEO_TOOL_NAMES: &[&str] = &[
        "create_sequence",
        "delete_sequence",
        "list_sequences",
        "set_active_sequence",
        "set_sequence_format",
        "set_active_format",
        "add_track",
        "remove_track",
        "set_track_prop",
        "reorder_track",
        "insert_clip",
        "move_clip",
        "trim_clip",
        "split_clip",
        "remove_clip",
        "roll_edit",
        "slip_clip",
        "slide_clip",
        "set_clip_prop",
        "set_clip_speed",
        "set_transition",
        "list_clips",
        "get_clip",
        "add_effect",
        "remove_effect",
        "reorder_effects",
        "set_effect_param",
        "list_effect_kinds",
        "set_keyframe",
        "remove_keyframe",
        "batch_set_keyframes",
        "get_keyframes",
        "import_media",
        "relink_media",
        "list_media",
        "remove_asset",
    ];

    #[test]
    fn every_video_tool_is_in_the_schema() {
        let schema = crate::schema_gen::tool_list();
        let names: Vec<String> = schema
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect();
        for tool in VIDEO_TOOL_NAMES {
            assert!(
                names.contains(&tool.to_string()),
                "tool {tool} missing from schema_gen::tool_list()"
            );
        }
    }

    #[tokio::test]
    async fn every_video_tool_has_a_dispatch_arm() {
        let state = test_state();
        for tool in VIDEO_TOOL_NAMES {
            let result = crate::dispatch::dispatch_tool(&state, tool, json!({})).await;
            if let Err(msg) = result {
                assert!(
                    !msg.starts_with("Unknown tool"),
                    "{tool}: no dispatch_tool_inner arm found ({msg})"
                );
            }
        }
    }
}
