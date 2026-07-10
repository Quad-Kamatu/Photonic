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
//! ## P3 engine slice (this file, lower half)
//! Playback transport (10 §3.13), `render_frame_at` (10 §4), real
//! `probe_media`/`transcode_media`, and the export/job tools (10 §3.15, §6)
//! are implemented against the `photonic_video::VideoEngine` facade via the
//! lazy [`EngineBridge`] (`handlers/video_jobs.rs`). Captions/tts/grade
//! scopes remain P4+.
//!
//! ## Resolved gaps (P2 top-up, photonic-core commit ab7557f)
//! `set_work_range`/`add_marker`/`remove_marker`/`list_markers`,
//! `ripple_edit` (via `ops::ripple_trim`), `move_clip`'s cross-track path
//! (via `ops::move_clip_to_track`), and media bins (`MediaAsset.bin` +
//! `ops::{create_bin,remove_bin,assign_asset_bin}`) all landed in core and
//! are implemented below.
//!
//! ## Remaining gap
//! - `import_media`'s `content_hash` uses a stopgap `DefaultHasher` (SipHash)
//!   digest over head+tail+len, not the `xxh3` the core doc comment
//!   (media.rs:6) describes as the relink identity. `probe_media` (P3, below)
//!   refreshes it with the real `photonic_video::media::probe::content_hash`
//!   xxh3 digest.

use crate::handlers::video_jobs::{set_job_status, EngineBridge, JobStatus};
use crate::protocol::*;
use crate::server::AppState;
use base64::{engine::general_purpose, Engine as _};
use photonic_core::history::Command;
use photonic_core::timeline::{
    ops, AnimTarget, AssetId, AssetKind, Clip, ClipEffect, ClipId, ClipSource, ClipTiming,
    EditError, FormatOp, FrameRate, Keyframe, Marker, MarkerId, PropPath, Ratio, SequenceId,
    SpeedMap, Tick, TimelineCmd, TimelineProject, Track, TrackId, TrackSettings, Transition,
    TICKS_PER_SECOND,
};
use photonic_core::Color;
use photonic_video::export::convert as export_convert;
use photonic_video::export::presets as export_presets;
use photonic_video::export::render_loop;
use photonic_video::graph::eval::read_texture_rgba16f;
use photonic_video::media::ffmpeg_locate;
use photonic_video::media::probe as video_probe;
use photonic_video::{EngineCmd, ProxyMode};
use serde_json::json;
use std::sync::atomic::Ordering;
use std::sync::Mutex as StdMutex;
use std::time::{Duration, Instant};

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
///
/// The `HH:MM:SS` field counts **frames**, not wall-clock seconds: a timecode
/// addresses a position on the frame grid, so `SS` is `nominal_fps` frames
/// (spec 10 §1.3, 01 §1). Resolving via wall-clock seconds would land a
/// 1001-denominator rate (29.97 = 30000/1001) off its own frame boundaries —
/// e.g. `00:01:00:00` @ 29.97 would fall ~1.8 frames early. Instead we count
/// `total_frames = (h*3600 + m*60 + s) * nominal_fps + ff` and multiply by
/// `FrameRate::ticks_per_frame`, so the result is always frame-aligned.
///
/// Returns `None` for malformed input or out-of-range components (negative
/// values, `MM`/`SS >= 60`, or `FF >= nominal_fps`).
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
    // Nominal integer frame rate: round(num/den) (30000/1001 → 30). This is the
    // number of frame slots a `SS` second occupies on the timecode grid.
    let den = fr.den.max(1) as i64;
    let nominal_fps = ((fr.num as i64) + den / 2) / den;
    // Reject impossible components (spec 10 §1.3): no negatives, MM/SS < 60,
    // and the frame field must be within one nominal second.
    if h < 0
        || m < 0
        || s < 0
        || ff < 0
        || m >= 60
        || s >= 60
        || nominal_fps < 1
        || ff >= nominal_fps
    {
        return None;
    }
    let total_frames = (h * 3600 + m * 60 + s) * nominal_fps + ff;
    Some(Tick(total_frames * fr.ticks_per_frame().0))
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

/// Which `SequenceId` owns a marker.
fn locate_marker(p: &TimelineProject, marker: MarkerId) -> Option<SequenceId> {
    p.sequences
        .iter()
        .find(|(_, s)| s.markers.iter().any(|m| m.id == marker))
        .map(|(sid, _)| *sid)
}

/// Resolve a bin by exact name.
fn find_bin_by_name<'a>(
    p: &'a TimelineProject,
    name: &str,
) -> Option<&'a photonic_core::timeline::MediaBin> {
    p.media.bins.iter().find(|b| b.name == name)
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

pub async fn set_work_range(state: &AppState, args: SetWorkRangeArgs) -> ToolResult {
    tracing::debug!("tool: set_work_range {}", args.sequence_id);
    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    let Some(project) = doc.timeline.as_ref() else {
        return ToolResult::error("no timeline project");
    };
    let Some(seq) = project.sequences.get(&args.sequence_id) else {
        return ToolResult::error(format!("sequence {} not found", args.sequence_id));
    };
    let fr = seq.frame_rate;
    let new_range = match args.range {
        None => None,
        Some(r) => {
            let start = match resolve_tick(
                r.start_ticks,
                r.start_tc.as_deref(),
                r.start_seconds,
                Some(fr),
            ) {
                Ok(t) => t,
                Err(e) => return e,
            };
            let end = match resolve_tick(r.end_ticks, r.end_tc.as_deref(), r.end_seconds, Some(fr))
            {
                Ok(t) => t,
                Err(e) => return e,
            };
            if end <= start {
                return err_code("TickOutOfRange", "work range end must be after start");
            }
            Some((start, end))
        }
    };
    match ops::set_work_range(project, args.sequence_id, new_range) {
        Ok(cmd) => {
            history.execute_discrete(Command::Timeline(cmd), &mut doc);
            ToolResult::text("Work range updated")
        }
        Err(e) => map_edit_error(e),
    }
}

pub async fn add_marker(state: &AppState, args: AddMarkerArgs) -> ToolResult {
    tracing::debug!("tool: add_marker {}", args.sequence_id);
    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    let Some(project) = doc.timeline.as_ref() else {
        return ToolResult::error("no timeline project");
    };
    let Some(seq) = project.sequences.get(&args.sequence_id) else {
        return ToolResult::error(format!("sequence {} not found", args.sequence_id));
    };
    let at = match resolve_tick(
        args.at_ticks,
        args.at_tc.as_deref(),
        args.at_seconds,
        Some(seq.frame_rate),
    ) {
        Ok(t) => t,
        Err(e) => return e,
    };
    let color = match args.color {
        Some(hex) => match Color::from_hex(&hex) {
            Some(c) => Some(c),
            None => {
                return ToolResult::error(format!(
                    "invalid color {hex:?} — expected #rrggbb or #rrggbbaa"
                ))
            }
        },
        None => None,
    };
    let mut marker = Marker::new(at, args.name.unwrap_or_default());
    marker.color = color;
    if let Some(note) = args.note {
        marker.note = note;
    }
    let marker_id = marker.id;
    match ops::add_marker(project, args.sequence_id, marker) {
        Ok(cmd) => {
            history.execute_discrete(Command::Timeline(cmd), &mut doc);
            ToolResult::text("Added marker").with_data(json!({ "marker_id": marker_id }))
        }
        Err(e) => map_edit_error(e),
    }
}

pub async fn remove_marker(state: &AppState, args: RemoveMarkerArgs) -> ToolResult {
    tracing::debug!("tool: remove_marker {}", args.marker_id);
    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    let Some(project) = doc.timeline.as_ref() else {
        return ToolResult::error("no timeline project");
    };
    let Some(seq_id) = locate_marker(project, args.marker_id) else {
        return ToolResult::error(format!("marker {} not found", args.marker_id));
    };
    match ops::remove_marker(project, seq_id, args.marker_id) {
        Ok(cmd) => {
            history.execute_discrete(Command::Timeline(cmd), &mut doc);
            ToolResult::text("Removed marker")
        }
        Err(e) => map_edit_error(e),
    }
}

pub async fn list_markers(state: &AppState, args: ListMarkersArgs) -> ToolResult {
    tracing::debug!("tool: list_markers {}", args.sequence_id);
    let doc = state.document.lock().await;
    let Some(project) = doc.timeline.as_ref() else {
        return ToolResult::error("no timeline project");
    };
    let Some(seq) = project.sequences.get(&args.sequence_id) else {
        return ToolResult::error(format!("sequence {} not found", args.sequence_id));
    };
    ToolResult::text(format!("{} marker(s)", seq.markers.len()))
        .with_data(json!({ "markers": seq.markers }))
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
    match ops::move_clip_to_track(
        project,
        seq_id,
        track_id,
        args.clip_id,
        new_start,
        args.new_track_id,
    ) {
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

pub async fn ripple_edit(state: &AppState, args: RippleEditArgs) -> ToolResult {
    tracing::debug!("tool: ripple_edit {}", args.clip_id);
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
    // `edge` uses the same in/out vocabulary as trim_clip: in = the clip's
    // in-point (`ops::ClipEdge::Start`), out = its out-point (`::End`).
    let (core_edge, current_boundary) = match args.edge {
        ClipEdge::In => (photonic_core::timeline::ClipEdge::Start, clip.start),
        ClipEdge::Out => (photonic_core::timeline::ClipEdge::End, clip.end()),
    };
    let new_boundary = current_boundary + Tick(args.delta_ticks);
    match ops::ripple_trim(
        project,
        seq_id,
        track_id,
        args.clip_id,
        core_edge,
        new_boundary,
    ) {
        Ok(cmd) => {
            history.execute_discrete(Command::Timeline(cmd), &mut doc);
            ToolResult::text("Rippled edit")
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
    // Validate + build each asset first (no project state needed for this part).
    let mut pending = Vec::new();
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
        pending.push((p.clone(), asset));
    }

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    let mut cmds = Vec::new();
    let needs_project = doc.timeline.is_none();
    if needs_project {
        cmds.push(Command::Timeline(ops::create_project()));
    }

    // Resolve (or create) the target bin by exact name, then file every
    // imported asset under it directly at construction — cheaper and avoids
    // a chicken-and-egg AssignAssetBin-before-AddAsset ordering problem.
    let bin_id = match &args.bin {
        None => None,
        Some(bin_name) => {
            let existing = if needs_project {
                None
            } else {
                doc.timeline
                    .as_ref()
                    .and_then(|p| find_bin_by_name(p, bin_name))
                    .map(|b| b.id)
            };
            Some(existing.unwrap_or_else(|| {
                let cmd = ops::create_bin(bin_name.clone(), None);
                let id = match &cmd {
                    TimelineCmd::AddBin { bin } => bin.id,
                    _ => unreachable!("ops::create_bin always returns AddBin"),
                };
                cmds.push(Command::Timeline(cmd));
                id
            }))
        }
    };

    let mut created = Vec::new();
    for (p, mut asset) in pending {
        asset.bin = bin_id;
        let asset_id = asset.id;
        let kind = asset.kind;
        cmds.push(Command::Timeline(ops::add_asset(asset)));
        created.push(json!({
            "asset_id": asset_id, "path": p, "kind": kind, "probed": false, "bin_id": bin_id
        }));
    }
    history.execute_discrete(Command::Batch(cmds), &mut doc);

    ToolResult::text(format!(
        "Imported {} asset(s) — probing lands in P3 (ffprobe integration)",
        created.len()
    ))
    .with_data(json!({ "assets": created }))
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
    let filter_bin = match &args.bin {
        None => None,
        Some(name) => match find_bin_by_name(project, name) {
            Some(b) => Some(b.id),
            None => {
                return ToolResult::text(format!("no bin named {name:?}"))
                    .with_data(json!({ "assets": [] }))
            }
        },
    };
    let assets: Vec<_> = project
        .media
        .assets
        .values()
        .filter(|a| filter_bin.is_none() || a.bin == filter_bin)
        .map(|a| {
            json!({
                "asset_id": a.id,
                "kind": a.kind,
                "source": a.source,
                "probed": a.probe.is_some(),
                "proxy_status": a.proxy.as_ref().map(|p| p.status),
                "content_hash": a.content_hash,
                "bin_id": a.bin,
            })
        })
        .collect();
    ToolResult::text(format!("{} media asset(s)", assets.len()))
        .with_data(json!({ "assets": assets }))
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

// ─── Media bins (added for the P2 top-up — not in the original §3.1 table) ──

pub async fn create_bin(state: &AppState, args: CreateBinArgs) -> ToolResult {
    tracing::debug!("tool: create_bin {}", args.name);
    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    let needs_project = doc.timeline.is_none();
    if let Some(parent) = args.parent {
        let exists = doc
            .timeline
            .as_ref()
            .map(|p| p.media.bins.iter().any(|b| b.id == parent))
            .unwrap_or(false);
        if !exists {
            return ToolResult::error(format!("bin {parent} not found"));
        }
    }
    let mut cmds = Vec::new();
    if needs_project {
        cmds.push(Command::Timeline(ops::create_project()));
    }
    let cmd = ops::create_bin(args.name, args.parent);
    let bin_id = match &cmd {
        TimelineCmd::AddBin { bin } => bin.id,
        _ => unreachable!("ops::create_bin always returns AddBin"),
    };
    cmds.push(Command::Timeline(cmd));
    history.execute_discrete(Command::Batch(cmds), &mut doc);
    ToolResult::text("Created bin").with_data(json!({ "bin_id": bin_id }))
}

pub async fn remove_bin(state: &AppState, args: RemoveBinArgs) -> ToolResult {
    tracing::debug!("tool: remove_bin {}", args.bin_id);
    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    let Some(project) = doc.timeline.as_ref() else {
        return ToolResult::error("no timeline project");
    };
    match ops::remove_bin(project, args.bin_id) {
        Ok(cmd) => {
            history.execute_discrete(Command::Timeline(cmd), &mut doc);
            ToolResult::text("Removed bin")
        }
        Err(e) => map_edit_error(e),
    }
}

pub async fn set_asset_bin(state: &AppState, args: SetAssetBinArgs) -> ToolResult {
    tracing::debug!("tool: set_asset_bin {}", args.asset_id);
    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    let Some(project) = doc.timeline.as_ref() else {
        return ToolResult::error("no timeline project");
    };
    match ops::assign_asset_bin(project, args.asset_id, args.bin_id) {
        Ok(cmd) => {
            history.execute_discrete(Command::Timeline(cmd), &mut doc);
            ToolResult::text("Updated asset bin")
        }
        Err(e) => map_edit_error(e),
    }
}

pub async fn list_bins(state: &AppState, _args: ListBinsArgs) -> ToolResult {
    tracing::debug!("tool: list_bins");
    let doc = state.document.lock().await;
    let Some(project) = doc.timeline.as_ref() else {
        return ToolResult::text("No timeline project yet").with_data(json!({ "bins": [] }));
    };
    let bins: Vec<_> = project
        .media
        .bins
        .iter()
        .map(|b| json!({ "bin_id": b.id, "name": b.name, "parent": b.parent }))
        .collect();
    ToolResult::text(format!("{} bin(s)", bins.len())).with_data(json!({ "bins": bins }))
}

// ─── Tests ───────────────────────────────────────────────────────────────────

// ═════════════════════════════════════════════════════════════════════════════
// P3 engine slice (10-mcp-tools.md): playback (§3.13), render_frame_at
// (§3.14/§4), engine-backed media ops (§3.1 probe/proxy/transcode), export +
// the async-job tools (§3.15, §6). All engine access goes through the lazy
// [`EngineBridge`] in `handlers/video_jobs.rs` — see that module's docs for
// the shadow-document snapshot bridge and the no-GPU degradation story.
// ═════════════════════════════════════════════════════════════════════════════

/// The engine bridge, or the structured no-GPU degradation error (10 §7:
/// "fails with a clear error rather than blocking the rest of the surface").
/// `EngineUnavailable` extends the §8 taxonomy — §8 has no code for a missing
/// GPU adapter because §2 assumed a GUI-shared `GpuContext`.
fn engine_bridge(state: &AppState) -> Result<&EngineBridge, ToolResult> {
    state.video_engine.bridge().ok_or_else(|| {
        err_code(
            "EngineUnavailable",
            "no GPU adapter available — video-engine-backed tools \
             (playback/render_frame_at/export_sequence) are unavailable on this machine",
        )
    })
}

/// Frame rate + formats + active-format index of a sequence, read from the
/// REAL document (readonly borrow — design rule 5: never locks `history`).
async fn sequence_render_info(
    state: &AppState,
    seq: SequenceId,
) -> Result<
    (
        FrameRate,
        Vec<photonic_core::timeline::SequenceFormat>,
        usize,
    ),
    ToolResult,
> {
    let doc = state.document.lock().await;
    let project = doc
        .timeline
        .as_ref()
        .ok_or_else(|| ToolResult::error("no timeline project"))?;
    let s = project
        .sequences
        .get(&seq)
        .ok_or_else(|| ToolResult::error(format!("sequence {seq} not found")))?;
    Ok((s.frame_rate, s.formats.clone(), s.active_format))
}

/// The status payload every transport tool returns (`EngineStatus`, 02 §1).
fn engine_status_json(status: &photonic_video::EngineStatus) -> serde_json::Value {
    json!({
        "playhead_ticks": status.playhead.0,
        "playing": status.playing,
        "dropped_frames": status.dropped,
        "cache": {
            "hits": status.cache.hits,
            "misses": status.cache.misses,
            "resident_entries": status.cache.resident_entries,
            "resident_bytes": status.cache.resident_bytes,
        },
        "audio_xruns": status.audio_xruns,
        "doc_revision": status.doc_revision,
        "active_sequence": status.active_sequence,
        "last_error": status.last_error,
    })
}

/// Poll the published status until `pred` holds or `timeout` elapses;
/// returns the last observed status either way (confirmation is best-effort
/// — the engine applies commands within one 2 ms loop tick, but a saturated
/// box may lag; callers report the snapshot rather than fail).
async fn wait_status(
    bridge: &EngineBridge,
    timeout: Duration,
    pred: impl Fn(&photonic_video::EngineStatus) -> bool,
) -> std::sync::Arc<photonic_video::EngineStatus> {
    let deadline = Instant::now() + timeout;
    loop {
        let s = bridge.session().status();
        if pred(&s) || Instant::now() >= deadline {
            return s;
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}

/// Settle a transport command (seek/step) against the engine the way
/// `render_frame_at` does, so the returned playhead reflects the command just
/// dispatched — never the tick one engine loop behind.
///
/// The old transport predicates read the *published* playhead directly:
/// `seek` accepted `playhead >= t` and `step` accepted `playhead != before`.
/// Both can be satisfied by the **stale pre-command** status, since the engine
/// sets the clock and re-presents on a later loop iteration than the one that
/// published the status the tool first observes. A backward seek is the sharp
/// case — the old (larger) playhead is already `>= t`, so `seek F45` returned
/// the stale `F150` immediately, and a following `get_engine_status` echoed it.
///
/// Instead we wait for a *fresh* frame (newer than `prev`, pointer-compared) at
/// the exact target frame-start `frame_tick` — a stale pre-command frame has
/// the wrong tick and can't satisfy it — then, for a paused command, confirm
/// the published status caught up to `expect` (its store trails the frame store
/// by under one engine loop). On timeout (no media, or a boundary clamp that
/// produces no distinct frame) it reports the latest status unconditionally.
async fn settle_transport(
    bridge: &EngineBridge,
    prev: Option<std::sync::Arc<photonic_video::EngineFrame>>,
    frame_tick: Tick,
    seq: SequenceId,
    expect: Option<Tick>,
    timeout: Duration,
) -> std::sync::Arc<photonic_video::EngineStatus> {
    let deadline = Instant::now() + timeout;
    let produced = bridge
        .wait_fresh_frame(prev, timeout, |f| f.time == frame_tick && f.sequence == seq)
        .await
        .is_some();
    if produced {
        if let Some(exp) = expect {
            let remaining = deadline.saturating_duration_since(Instant::now());
            return wait_status(bridge, remaining, |s| s.playhead == exp).await;
        }
    }
    bridge.session().status()
}

// ─── Playback (10 §3.13) ─────────────────────────────────────────────────────
//
// These mutate **engine/session state only** — never the document, never
// `history` (01 §11: playhead is session state). Dispatch classifies them
// `ToolOutput::readonly` so no checkpoint is scheduled; §3.13's `mutating*`
// is about side-effect semantics, not the checkpoint machinery.

pub async fn play(state: &AppState, args: PlayArgs) -> ToolResult {
    tracing::debug!("tool: play");
    let bridge = match engine_bridge(state) {
        Ok(b) => b,
        Err(e) => return e,
    };
    if let Some(seq) = args.sequence_id {
        if let Err(e) = sequence_render_info(state, seq).await {
            return e;
        }
    }
    let _transport = bridge.lock_transport().await;
    bridge.sync(state).await;
    if let Some(seq) = args.sequence_id {
        bridge.session().send(EngineCmd::SetActiveSequence(seq));
    }
    if !bridge.session().send(EngineCmd::Play) {
        return ToolResult::error("engine session has shut down");
    }
    // Headless boxes with no audio device play on the soft clock (02 §4) —
    // the engine's lazy cpal open falls back internally, so `play` succeeds
    // rather than raising AudioDeviceUnavailable (10 §7's degraded row).
    let status = wait_status(bridge, Duration::from_secs(2), |s| s.playing).await;
    ToolResult::text(if status.playing {
        "playback started"
    } else {
        "play sent (engine did not confirm within 2s)"
    })
    .with_data(engine_status_json(&status))
}

pub async fn pause(state: &AppState, _args: PauseArgs) -> ToolResult {
    tracing::debug!("tool: pause");
    let bridge = match engine_bridge(state) {
        Ok(b) => b,
        Err(e) => return e,
    };
    let _transport = bridge.lock_transport().await;
    if !bridge.session().send(EngineCmd::Pause) {
        return ToolResult::error("engine session has shut down");
    }
    let status = wait_status(bridge, Duration::from_secs(2), |s| !s.playing).await;
    ToolResult::text(format!("paused at tick {}", status.playhead.0))
        .with_data(engine_status_json(&status))
}

pub async fn seek(state: &AppState, args: SeekArgs) -> ToolResult {
    tracing::debug!("tool: seek");
    let bridge = match engine_bridge(state) {
        Ok(b) => b,
        Err(e) => return e,
    };
    let (fr, _, _) = match sequence_render_info(state, args.sequence_id).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    let t = match resolve_tick(
        args.at_ticks,
        args.at_tc.as_deref(),
        args.at_seconds,
        Some(fr),
    ) {
        Ok(t) => t,
        Err(e) => return e,
    };
    if t.0 < 0 {
        return err_code("TickOutOfRange", "seek target must be >= 0")
            .with_data(json!({ "min_ticks": 0 }));
    }
    let _transport = bridge.lock_transport().await;
    bridge.sync(state).await;
    // The engine presents exact frame-start ticks (02 §4); snap the target so
    // the fresh-frame wait matches what will actually be published.
    let snapped = fr.frame_start(fr.frame_at(t));
    let was_playing = bridge.session().status().playing;
    let prev = bridge.session().latest_frame();
    bridge
        .session()
        .send(EngineCmd::SetActiveSequence(args.sequence_id));
    if !bridge.session().send(EngineCmd::Seek(t)) {
        return ToolResult::error("engine session has shut down");
    }
    // Settle against the produced frame (see `settle_transport`): a backward
    // seek used to report the stale pre-seek playhead because it was already
    // `>= t`. Paused ⇒ confirm the clock landed exactly on `t`; playing ⇒ the
    // clock keeps advancing, so the fresh target frame alone proves the seek.
    let status = settle_transport(
        bridge,
        prev,
        snapped,
        args.sequence_id,
        (!was_playing).then_some(t),
        Duration::from_secs(5),
    )
    .await;
    ToolResult::text(format!("seeked to tick {}", t.0)).with_data(engine_status_json(&status))
}

pub async fn step(state: &AppState, args: StepArgs) -> ToolResult {
    tracing::debug!("tool: step {}", args.frames);
    let bridge = match engine_bridge(state) {
        Ok(b) => b,
        Err(e) => return e,
    };
    let _transport = bridge.lock_transport().await;
    bridge.sync(state).await;
    // Step is relative, so reading a stale `before` would compound into a
    // stale result (the finding's "step responses lagged"). Sample the current
    // status, compute the exact frame the engine will snap to (mirrors
    // `PlaybackController::step`), then wait for that frame + its published
    // playhead via the same fresh-frame discipline `render_frame_at` uses.
    let before_status = bridge.session().status();
    let before = before_status.playhead;
    let prev = bridge.session().latest_frame();
    if !bridge.session().send(EngineCmd::Step(args.frames)) {
        return ToolResult::error("engine session has shut down");
    }
    let status = match before_status.active_sequence {
        // Step always pauses (02 §4); `expected` is frame-aligned, so we can
        // confirm the exact landed tick (including a clamp at frame 0, where
        // the forced re-present of the same frame satisfies the wait at once).
        Some(seq) => match sequence_render_info(state, seq).await {
            Ok((fr, _, _)) => {
                let target_frame = (fr.frame_at(before) + args.frames as i64).max(0);
                let expected = fr.frame_start(target_frame);
                settle_transport(
                    bridge,
                    prev,
                    expected,
                    seq,
                    Some(expected),
                    Duration::from_secs(5),
                )
                .await
            }
            // Sequence vanished from the real doc mid-call — best-effort pause.
            Err(_) => wait_status(bridge, Duration::from_secs(2), |s| !s.playing).await,
        },
        // No active sequence to present against — just confirm the pause.
        None => wait_status(bridge, Duration::from_secs(2), |s| !s.playing).await,
    };
    ToolResult::text(format!(
        "stepped {} frame(s) — playhead at tick {} (paused)",
        args.frames, status.playhead.0
    ))
    .with_data(engine_status_json(&status))
}

pub async fn set_loop_range(state: &AppState, args: SetLoopRangeArgs) -> ToolResult {
    tracing::debug!("tool: set_loop_range");
    let bridge = match engine_bridge(state) {
        Ok(b) => b,
        Err(e) => return e,
    };
    let (fr, _, _) = match sequence_render_info(state, args.sequence_id).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    let range = match &args.range {
        None => None,
        Some(r) => {
            let start = match resolve_tick(
                r.start_ticks,
                r.start_tc.as_deref(),
                r.start_seconds,
                Some(fr),
            ) {
                Ok(t) => t,
                Err(e) => return e,
            };
            let end = match resolve_tick(r.end_ticks, r.end_tc.as_deref(), r.end_seconds, Some(fr))
            {
                Ok(t) => t,
                Err(e) => return e,
            };
            if end <= start {
                return err_code("TickOutOfRange", "loop end must be after loop start");
            }
            Some((start, end))
        }
    };
    let _transport = bridge.lock_transport().await;
    bridge.sync(state).await;
    bridge
        .session()
        .send(EngineCmd::SetActiveSequence(args.sequence_id));
    if !bridge.session().send(EngineCmd::SetLoop(range)) {
        return ToolResult::error("engine session has shut down");
    }
    match range {
        Some((s, e)) => ToolResult::text(format!("loop range set to [{}, {})", s.0, e.0))
            .with_data(json!({ "start_ticks": s.0, "end_ticks": e.0 })),
        None => ToolResult::text("loop range cleared"),
    }
}

pub async fn set_proxy_mode(state: &AppState, args: SetProxyModeArgs) -> ToolResult {
    tracing::debug!("tool: set_proxy_mode");
    let bridge = match engine_bridge(state) {
        Ok(b) => b,
        Err(e) => return e,
    };
    let mode = match args.mode {
        ProxyModeArg::Auto => ProxyMode::Auto,
        ProxyModeArg::ForceProxy => ProxyMode::ForceProxy,
        ProxyModeArg::ForceOriginal => ProxyMode::ForceOriginal,
    };
    bridge.set_proxy_mode(mode);
    ToolResult::text(format!("proxy mode set to {mode:?}")).with_data(json!({
        "mode": format!("{mode:?}"),
        "note": "proxy generation has not landed — Auto/ForceProxy currently decode originals \
                 (proxies are never required for correctness, CAP-014)"
    }))
}

pub async fn get_engine_status(state: &AppState, _args: GetEngineStatusArgs) -> ToolResult {
    tracing::debug!("tool: get_engine_status");
    let bridge = match engine_bridge(state) {
        Ok(b) => b,
        Err(e) => return e,
    };
    bridge.sync(state).await;
    let synced = bridge.wait_engine_synced(Duration::from_secs(2)).await;
    let status = bridge.session().status();
    let mut data = engine_status_json(&status);
    data["snapshot_synced"] = json!(synced);
    ToolResult::text(format!(
        "playhead {} — {}",
        status.playhead.0,
        if status.playing { "playing" } else { "paused" }
    ))
    .with_data(data)
}

// ─── render_frame_at (10 §3.14 / §4) ─────────────────────────────────────────

pub async fn render_frame_at(state: &AppState, args: RenderFrameAtArgs) -> ToolResult {
    tracing::debug!("tool: render_frame_at");
    let bridge = match engine_bridge(state) {
        Ok(b) => b,
        Err(e) => return e,
    };
    let seq_id = args.sequence_id;
    let (fr, formats, active_format) = match sequence_render_info(state, seq_id).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    if let Some(fi) = args.format_index {
        if fi >= formats.len() {
            return ToolResult::error(format!(
                "format_index {fi} out of range — sequence has {} format(s)",
                formats.len()
            ));
        }
    }
    let format_index = args
        .format_index
        .unwrap_or(active_format)
        .min(formats.len().saturating_sub(1));
    let (w, h) = (
        formats[format_index].width.max(1),
        formats[format_index].height.max(1),
    );
    let t = match resolve_tick(
        args.at_ticks,
        args.at_tc.as_deref(),
        args.at_seconds,
        Some(fr),
    ) {
        Ok(t) => t,
        Err(e) => return e,
    };
    if t.0 < 0 {
        return err_code("TickOutOfRange", "render tick must be >= 0")
            .with_data(json!({ "min_ticks": 0 }));
    }
    // The engine presents exact frame-start ticks (02 §4) — snap down.
    let snapped = fr.frame_start(fr.frame_at(t));
    let scale = args.scale.unwrap_or(1.0);
    if !(scale > 0.0 && scale <= 1.0) {
        return ToolResult::error("scale must be in (0, 1]");
    }
    let output_format = args.output_format.unwrap_or_default();

    let started = Instant::now();
    let _transport = bridge.lock_transport().await;

    // Per-call format override applied to the SHADOW timeline only — the real
    // document's `active_format` is untouched (this tool is readonly).
    {
        let mut timeline = state.document.lock().await.timeline.clone();
        if let Some(p) = timeline.as_mut() {
            if let Some(s) = p.sequences.get_mut(&seq_id) {
                s.active_format = format_index;
            }
        }
        bridge.sync_timeline(timeline);
    }
    if !bridge.wait_engine_synced(Duration::from_secs(10)).await {
        return ToolResult::error("engine did not pick up the document snapshot within 10s");
    }

    // Per-call quality via the session proxy-mode knob (the engine's only
    // quality input, 02 §6) — restored to the sticky `set_proxy_mode` choice
    // below. In P3 preview/full render identically (no proxies exist yet);
    // the flag still flows into `Quality` so cache hashes stay honest.
    let restore_mode = bridge.proxy_mode();
    let call_mode = match args.quality {
        RenderQualityArg::Preview => ProxyMode::ForceProxy,
        RenderQualityArg::Full => ProxyMode::ForceOriginal,
    };
    bridge.session().send(EngineCmd::SetProxyMode(call_mode));
    let prev = bridge.session().latest_frame();
    bridge.session().send(EngineCmd::SetActiveSequence(seq_id));
    if !bridge.session().send(EngineCmd::Seek(snapped)) {
        bridge.session().send(EngineCmd::SetProxyMode(restore_mode));
        return ToolResult::error("engine session has shut down");
    }

    let frame = bridge
        .wait_fresh_frame(prev, Duration::from_secs(30), |f| {
            f.time == snapped && f.sequence == seq_id
        })
        .await;
    let result = match frame {
        Some(frame) => {
            // Read the LOGICAL region only — EngineFrame textures are padded
            // to the texture pool's 64 px bucket (see photonic-video
            // session.rs::pad_to_pool_bucket); content sits top-left at the
            // sequence-format size.
            let pixels = read_texture_rgba16f(bridge.engine().gpu(), &frame.texture, w, h);
            build_render_result(
                pixels,
                w,
                h,
                scale,
                output_format,
                snapped,
                started.elapsed(),
            )
        }
        None => ToolResult::error(
            "engine did not produce the requested frame within 30s — cold-seek decode \
             cost can dominate (see tool description)",
        ),
    };
    bridge.session().send(EngineCmd::SetProxyMode(restore_mode));
    result
}

/// Deterministic box downscale on linear premultiplied f32 pixels (fixed
/// iteration order ⇒ byte-stable output for the raw path).
fn box_downscale(src: &[[f32; 4]], w: u32, h: u32, ow: u32, oh: u32) -> Vec<[f32; 4]> {
    let mut out = Vec::with_capacity((ow * oh) as usize);
    for oy in 0..oh {
        let y0 = (oy as u64 * h as u64 / oh as u64) as u32;
        let y1 = (((oy as u64 + 1) * h as u64).div_ceil(oh as u64) as u32).clamp(y0 + 1, h);
        for ox in 0..ow {
            let x0 = (ox as u64 * w as u64 / ow as u64) as u32;
            let x1 = (((ox as u64 + 1) * w as u64).div_ceil(ow as u64) as u32).clamp(x0 + 1, w);
            let mut acc = [0f64; 4];
            for y in y0..y1 {
                for x in x0..x1 {
                    let p = src[(y * w + x) as usize];
                    for (a, c) in acc.iter_mut().zip(p.iter()) {
                        *a += *c as f64;
                    }
                }
            }
            let n = ((y1 - y0) as f64) * ((x1 - x0) as f64);
            out.push([
                (acc[0] / n) as f32,
                (acc[1] / n) as f32,
                (acc[2] / n) as f32,
                (acc[3] / n) as f32,
            ]);
        }
    }
    out
}

/// IEEE 754 binary32 → binary16 bits, round-to-nearest-even. Values read back
/// from an `Rgba16Float` texture are exactly representable, so the raw path
/// round-trips the GPU's own bits.
fn f32_to_f16_bits(f: f32) -> u16 {
    let bits = f.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exp = ((bits >> 23) & 0xff) as i32;
    let frac = bits & 0x007f_ffff;
    if exp == 0xff {
        // Inf / NaN (quietened).
        return sign | 0x7c00 | if frac != 0 { 0x0200 } else { 0 };
    }
    let exp = exp - 127 + 15;
    if exp >= 0x1f {
        return sign | 0x7c00; // overflow → inf
    }
    if exp <= 0 {
        if exp < -10 {
            return sign; // underflow → signed zero
        }
        // Subnormal half.
        let frac = frac | 0x0080_0000;
        let shift = (14 - exp) as u32;
        let mut sub = (frac >> shift) as u16;
        let rem = frac & ((1u32 << shift) - 1);
        let half = 1u32 << (shift - 1);
        if rem > half || (rem == half && (sub & 1) == 1) {
            sub += 1;
        }
        return sign | sub;
    }
    let mut out = sign | ((exp as u16) << 10) | ((frac >> 13) as u16);
    let rem = frac & 0x1fff;
    if rem > 0x1000 || (rem == 0x1000 && (out & 1) == 1) {
        out += 1; // carry may roll into the exponent — correct RNE behavior
    }
    out
}

fn build_render_result(
    pixels: Vec<[f32; 4]>,
    w: u32,
    h: u32,
    scale: f64,
    output_format: RenderOutputFormatArg,
    tick: Tick,
    elapsed: Duration,
) -> ToolResult {
    let ow = ((w as f64 * scale).round() as u32).clamp(1, w);
    let oh = ((h as f64 * scale).round() as u32).clamp(1, h);
    let pixels = if (ow, oh) == (w, h) {
        pixels
    } else {
        box_downscale(&pixels, w, h, ow, oh)
    };
    let render_ms = elapsed.as_millis() as u64;
    match output_format {
        RenderOutputFormatArg::Png => {
            // Reuse the export path's color math (single source of truth):
            // unpremultiply + linear→sRGB transfer + quantize.
            let flat: Vec<f32> = pixels.iter().flat_map(|p| p.iter().copied()).collect();
            let rgba8 = match export_convert::working_frame_to_rgba8(&flat, ow, oh) {
                export_convert::EncodePlanes::Rgba8 { rgba, .. } => rgba,
                _ => return ToolResult::error("internal error: unexpected plane kind"),
            };
            let Some(img) = image::RgbaImage::from_raw(ow, oh, rgba8) else {
                return ToolResult::error("internal error: pixel buffer size mismatch");
            };
            let mut png = Vec::new();
            if let Err(e) =
                img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            {
                return ToolResult::error(format!("PNG encode failed: {e}"));
            }
            ToolResult::text(format!(
                "rendered {ow}x{oh} frame at tick {} ({render_ms} ms)",
                tick.0
            ))
            .with_image(general_purpose::STANDARD.encode(&png))
            .with_data(json!({
                "width": ow, "height": oh, "tick": tick.0,
                "render_ms": render_ms, "output_format": "png",
            }))
        }
        RenderOutputFormatArg::RawRgba16f => {
            let mut bytes = Vec::with_capacity(pixels.len() * 8);
            for px in &pixels {
                for c in px {
                    bytes.extend_from_slice(&f32_to_f16_bits(*c).to_le_bytes());
                }
            }
            ToolResult::text(format!(
                "rendered {ow}x{oh} raw frame at tick {} ({render_ms} ms)",
                tick.0
            ))
            .with_data(json!({
                "width": ow, "height": oh, "tick": tick.0,
                "render_ms": render_ms, "output_format": "raw_rgba16f",
                "encoding": "interleaved RGBA, f16 little-endian, row-major, linear premultiplied (D-09)",
                "data_base64": general_purpose::STANDARD.encode(&bytes),
            }))
        }
    }
}

// ─── Engine-backed media ops (10 §3.1: probe / proxy / transcode) ────────────

/// Resolve an asset to its backing file path, with §8 error mapping.
async fn asset_file_path(
    state: &AppState,
    asset: AssetId,
) -> Result<std::path::PathBuf, ToolResult> {
    let doc = state.document.lock().await;
    let project = doc
        .timeline
        .as_ref()
        .ok_or_else(|| ToolResult::error("no timeline project"))?;
    let a = project
        .media
        .assets
        .get(&asset)
        .ok_or_else(|| ToolResult::error(format!("asset {asset} not found")))?;
    match &a.source {
        photonic_core::timeline::AssetSource::File { path, .. } => Ok(path.clone()),
        _ => Err(ToolResult::error(
            "asset is not file-backed — nothing to probe/transcode",
        )),
    }
}

pub async fn probe_media(state: &AppState, args: ProbeMediaArgs) -> ToolResult {
    tracing::debug!("tool: probe_media {}", args.asset_id);
    let path = match asset_file_path(state, args.asset_id).await {
        Ok(p) => p,
        Err(e) => return e,
    };
    if !path.exists() {
        return err_code(
            "AssetOffline",
            format!("file not found: {}", path.display()),
        );
    }
    let tools = match ffmpeg_locate::locate() {
        Ok(t) => t,
        Err(e) => {
            return err_code(
                "FfmpegUnavailable",
                format!("ffprobe not found ({e}) — set PHOTONIC_FFMPEG_DIR or install ffmpeg"),
            )
        }
    };
    let (job_id, cancel) = state
        .video_jobs
        .lock()
        .expect("job registry poisoned")
        .start("probe_media");
    let jobs = std::sync::Arc::clone(&state.video_jobs);
    let document = std::sync::Arc::clone(&state.document);
    let history = std::sync::Arc::clone(&state.history);
    let asset_id = args.asset_id;
    std::thread::spawn(move || {
        set_job_status(
            &jobs,
            job_id,
            JobStatus::Running {
                progress: 0.0,
                message: "probing".into(),
            },
        );
        if cancel.load(Ordering::Relaxed) {
            set_job_status(&jobs, job_id, JobStatus::Cancelled);
            return;
        }
        let probe = match video_probe::probe_asset(&tools, &path) {
            Ok(p) => p,
            Err(e) => {
                set_job_status(
                    &jobs,
                    job_id,
                    JobStatus::Failed {
                        error_code: "ProbeFailed".into(),
                        message: e.to_string(),
                    },
                );
                return;
            }
        };
        // xxh3 head+tail+len — the real relink identity (replaces the P2
        // SipHash stopgap noted at the top of this file).
        let hash = video_probe::content_hash(&path).ok();
        // Commit — design rule 7 lock order: document BEFORE history. This is
        // the job-completion path that mutates outside dispatch_tool_inner's
        // post-mutation hook (design rule 6), so the checkpoint is scheduled
        // here explicitly. `MediaAsset::probe` is engine-derived cache
        // ("Filled by the engine after ffprobe", core media.rs) with no
        // TimelineCmd variant — written directly, not as an undo step; a
        // `SetAssetProbe` command in core would let this use
        // `execute_discrete` (noted seam).
        let updated = {
            let mut doc = document.blocking_lock();
            let updated = match doc
                .timeline
                .as_mut()
                .and_then(|p| p.media.assets.get_mut(&asset_id))
            {
                Some(asset) => {
                    asset.probe = Some(probe.clone());
                    if hash.is_some() {
                        asset.content_hash = hash.clone();
                    }
                    true
                }
                None => false, // asset removed while probing — drop the result
            };
            let mut hist = history.blocking_lock();
            hist.schedule_mcp_checkpoint("probe_media");
            updated
        };
        set_job_status(
            &jobs,
            job_id,
            JobStatus::Done {
                result: json!({
                    "asset_id": asset_id,
                    "updated": updated,
                    "probe": probe,
                    "content_hash": hash,
                }),
            },
        );
    });
    ToolResult::text("probe job started — poll get_job_status")
        .with_data(json!({ "job_id": job_id }))
}

pub async fn generate_proxies(_state: &AppState, args: GenerateProxiesArgs) -> ToolResult {
    tracing::debug!("tool: generate_proxies ({} asset(s))", args.asset_ids.len());
    err_code(
        "NotSupportedV1",
        format!(
            "proxy generation is not built yet ({} asset(s) requested) — the engine/proxy \
             module (02 §6, 05 §2.3) has not landed and the evaluator decodes originals \
             regardless of ProxyMode (proxies are never required for correctness, CAP-014)",
            args.asset_ids.len()
        ),
    )
}

pub async fn remove_proxy(_state: &AppState, args: RemoveProxyArgs) -> ToolResult {
    tracing::debug!("tool: remove_proxy ({} asset(s))", args.asset_ids.len());
    err_code(
        "NotSupportedV1",
        "proxy generation is not built yet, so there are no proxy files to remove \
         (see generate_proxies)",
    )
}

impl TranscodePresetArg {
    /// ffmpeg encode args + output extension for each editing-intermediate
    /// preset (10 §3.1: user-picked codec, distinct from the proxy profile).
    fn ffmpeg_spec(self) -> (&'static [&'static str], &'static str, &'static str) {
        match self {
            TranscodePresetArg::ProresProxy => (
                &[
                    "-c:v",
                    "prores_ks",
                    "-profile:v",
                    "0",
                    "-vendor",
                    "apl0",
                    "-pix_fmt",
                    "yuv422p10le",
                    "-c:a",
                    "pcm_s16le",
                ],
                "mov",
                "prores_proxy",
            ),
            TranscodePresetArg::ProresLt => (
                &[
                    "-c:v",
                    "prores_ks",
                    "-profile:v",
                    "1",
                    "-vendor",
                    "apl0",
                    "-pix_fmt",
                    "yuv422p10le",
                    "-c:a",
                    "pcm_s16le",
                ],
                "mov",
                "prores_lt",
            ),
            TranscodePresetArg::DnxhrLb => (
                &[
                    "-c:v",
                    "dnxhd",
                    "-profile:v",
                    "dnxhr_lb",
                    "-pix_fmt",
                    "yuv422p",
                    "-c:a",
                    "pcm_s16le",
                ],
                "mov",
                "dnxhr_lb",
            ),
            TranscodePresetArg::H264High => (
                &[
                    "-c:v", "libx264", "-preset", "medium", "-crf", "18", "-pix_fmt", "yuv420p",
                    "-c:a", "aac",
                ],
                "mp4",
                "h264_high",
            ),
        }
    }
}

pub async fn transcode_media(state: &AppState, args: TranscodeMediaArgs) -> ToolResult {
    tracing::debug!("tool: transcode_media {}", args.asset_id);
    let input = match asset_file_path(state, args.asset_id).await {
        Ok(p) => p,
        Err(e) => return e,
    };
    if !input.exists() {
        return err_code(
            "AssetOffline",
            format!("file not found: {}", input.display()),
        );
    }
    let tools = match ffmpeg_locate::locate() {
        Ok(t) => t,
        Err(e) => {
            return err_code(
                "FfmpegUnavailable",
                format!("ffmpeg not found ({e}) — set PHOTONIC_FFMPEG_DIR or install ffmpeg"),
            )
        }
    };
    let (enc_args, ext, preset_name) = args.preset.ffmpeg_spec();
    let out_path = match &args.out_path {
        Some(p) => std::path::PathBuf::from(p),
        None => input.with_extension(format!("{preset_name}.{ext}")),
    };
    if out_path == input {
        return ToolResult::error("out_path must differ from the source file");
    }
    let (job_id, cancel) = state
        .video_jobs
        .lock()
        .expect("job registry poisoned")
        .start("transcode_media");
    let jobs = std::sync::Arc::clone(&state.video_jobs);
    let out_clone = out_path.clone();
    std::thread::spawn(move || {
        set_job_status(
            &jobs,
            job_id,
            JobStatus::Running {
                progress: 0.0,
                message: format!("transcoding to {preset_name}"),
            },
        );
        let mut cmd = std::process::Command::new(&tools.ffmpeg);
        cmd.arg("-y")
            .args(["-nostdin", "-loglevel", "error"])
            .arg("-i")
            .arg(&input)
            .args(enc_args)
            .arg(&out_clone)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped());
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                set_job_status(
                    &jobs,
                    job_id,
                    JobStatus::Failed {
                        error_code: "TranscodeFailed".into(),
                        message: format!("spawn ffmpeg: {e}"),
                    },
                );
                return;
            }
        };
        // Poll for exit / cooperative cancel (kill + clean the partial file).
        let status = loop {
            if cancel.load(Ordering::Relaxed) {
                let _ = child.kill();
                let _ = child.wait();
                let _ = std::fs::remove_file(&out_clone);
                set_job_status(&jobs, job_id, JobStatus::Cancelled);
                return;
            }
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => std::thread::sleep(Duration::from_millis(100)),
                Err(e) => {
                    set_job_status(
                        &jobs,
                        job_id,
                        JobStatus::Failed {
                            error_code: "TranscodeFailed".into(),
                            message: format!("wait ffmpeg: {e}"),
                        },
                    );
                    return;
                }
            }
        };
        if status.success() {
            set_job_status(
                &jobs,
                job_id,
                JobStatus::Done {
                    result: json!({ "output_path": out_clone, "preset": preset_name }),
                },
            );
        } else {
            let mut stderr = String::new();
            if let Some(mut s) = child.stderr.take() {
                use std::io::Read;
                let _ = s.read_to_string(&mut stderr);
            }
            let tail: String = stderr
                .chars()
                .rev()
                .take(500)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            set_job_status(
                &jobs,
                job_id,
                JobStatus::Failed {
                    error_code: "TranscodeFailed".into(),
                    message: format!("ffmpeg exited with {status}: {tail}"),
                },
            );
        }
    });
    ToolResult::text(format!(
        "transcode job started → {} — poll get_job_status",
        out_path.display()
    ))
    .with_data(json!({ "job_id": job_id, "output_path": out_path }))
}

// ─── Export (10 §3.15) + job tools (10 §6) ───────────────────────────────────

fn find_export_preset(name: &str) -> Option<export_presets::ExportPreset> {
    export_presets::built_in_presets()
        .into_iter()
        .chain(export_presets::load_custom_presets().unwrap_or_default())
        .find(|p| p.name == name)
}

pub async fn export_sequence(state: &AppState, args: ExportSequenceArgs) -> ToolResult {
    tracing::debug!("tool: export_sequence {}", args.sequence_id);
    let bridge = match engine_bridge(state) {
        Ok(b) => b,
        Err(e) => return e,
    };
    let tools = match ffmpeg_locate::locate() {
        Ok(t) => t,
        Err(e) => {
            return err_code(
                "FfmpegUnavailable",
                format!("ffmpeg not found ({e}) — set PHOTONIC_FFMPEG_DIR or install ffmpeg"),
            )
        }
    };
    let seq_id = args.sequence_id;
    // Snapshot the timeline NOW — the export renders this state even if the
    // document keeps being edited (the worker gets a frozen clone).
    let Some(project) = state.document.lock().await.timeline.clone() else {
        return ToolResult::error("no timeline project");
    };
    let Some(seq) = project.sequences.get(&seq_id) else {
        return ToolResult::error(format!("sequence {seq_id} not found"));
    };
    let seq_rate = seq.frame_rate;
    if let Some(fi) = args.format_index {
        if fi >= seq.formats.len() {
            return ToolResult::error(format!(
                "format_index {fi} out of range — sequence has {} format(s)",
                seq.formats.len()
            ));
        }
    }
    let format_index = args
        .format_index
        .unwrap_or(seq.active_format)
        .min(seq.formats.len().saturating_sub(1));
    let (fw, fh) = (
        seq.formats[format_index].width.max(1),
        seq.formats[format_index].height.max(1),
    );
    let content_end = seq
        .video_tracks
        .iter()
        .chain(seq.audio_tracks.iter())
        .flat_map(|t| t.clips.iter())
        .map(|c| c.end())
        .max()
        .unwrap_or(Tick(0));
    let (start, end) = match &args.range {
        Some(r) => {
            let s = match resolve_tick(
                r.start_ticks,
                r.start_tc.as_deref(),
                r.start_seconds,
                Some(seq_rate),
            ) {
                Ok(t) => t,
                Err(e) => return e,
            };
            let e2 = match resolve_tick(
                r.end_ticks,
                r.end_tc.as_deref(),
                r.end_seconds,
                Some(seq_rate),
            ) {
                Ok(t) => t,
                Err(e) => return e,
            };
            (s, e2)
        }
        None => seq.work_range.unwrap_or((Tick(0), content_end)),
    };
    if end <= start {
        return err_code(
            "TickOutOfRange",
            "export range is empty — sequence has no content and no explicit range was given",
        );
    }

    let preset_name = args
        .preset
        .clone()
        .unwrap_or_else(|| "Web H.264".to_string());
    let Some(mut preset) = find_export_preset(&preset_name) else {
        let names: Vec<String> = export_presets::built_in_presets()
            .into_iter()
            .chain(export_presets::load_custom_presets().unwrap_or_default())
            .map(|p| p.name)
            .collect();
        return ToolResult::error(format!(
            "no export preset named {preset_name:?} — available: {names:?}"
        ));
    };
    if let Some(o) = &args.overrides {
        match (o.width, o.height) {
            (Some(w), Some(h)) => {
                preset.resolution = export_presets::ResolutionSpec::Explicit { w, h }
            }
            (None, None) => {}
            _ => {
                return ToolResult::error(
                    "overrides.width and overrides.height must be given together",
                )
            }
        }
        if let Some(fr) = o.frame_rate {
            preset.frame_rate = export_presets::FrameRatePolicy::Explicit(fr);
        }
    }
    let out_rate = match preset.frame_rate {
        export_presets::FrameRatePolicy::Explicit(fr) => fr,
        export_presets::FrameRatePolicy::MatchSequence => seq_rate,
    };
    let (out_w, out_h) = match preset.resolution {
        export_presets::ResolutionSpec::SourceFormat => (fw, fh),
        export_presets::ResolutionSpec::Explicit { w, h } => (w.max(1), h.max(1)),
        export_presets::ResolutionSpec::Scale(s) => (
            ((fw as f32 * s).round() as u32).max(1),
            ((fh as f32 * s).round() as u32).max(1),
        ),
    };
    if out_w > fw || out_h > fh {
        return err_code(
            "NotSupportedV1",
            format!(
                "export upscaling ({out_w}x{out_h} from a {fw}x{fh} format) is not supported \
                 in P3 — the engine evaluates at format size and downscales only"
            ),
        );
    }
    // Sequence-audio muxing is the linked-audio story's seam: render
    // video-only regardless of the preset's audio spec.
    let audio_skipped = preset.audio.take().is_some();
    if let Err(e) = export_presets::validate(&preset) {
        return ToolResult::error(format!("preset invalid after overrides: {e}"));
    }
    let tpf_out = out_rate.ticks_per_frame().0.max(1);
    let total_frames = ((end.0 - start.0 + tpf_out - 1) / tpf_out).max(1) as u64;
    let out_path = std::path::PathBuf::from(&args.out_path);

    // Frozen shadow project with the export format forced active.
    let mut frozen = project;
    if let Some(s) = frozen.sequences.get_mut(&seq_id) {
        s.active_format = format_index;
    }
    let gpu = bridge.engine().gpu().clone();
    let (job_id, cancel) = state
        .video_jobs
        .lock()
        .expect("job registry poisoned")
        .start("export_sequence");
    let jobs = std::sync::Arc::clone(&state.video_jobs);
    let params = ExportJobParams {
        gpu,
        frozen,
        seq_id,
        start,
        seq_rate,
        out_rate,
        format_size: (fw, fh),
        out_size: (out_w, out_h),
        preset,
        out_path: out_path.clone(),
        total_frames,
        tools,
    };
    std::thread::spawn(move || run_export_job(jobs, job_id, cancel, params));

    ToolResult::text(format!(
        "export job started — {total_frames} frame(s) at {out_w}x{out_h} to {} — poll get_job_status",
        out_path.display()
    ))
    .with_data(json!({
        "job_id": job_id,
        "total_frames": total_frames,
        "width": out_w,
        "height": out_h,
        "preset": preset_name,
        "audio": if audio_skipped {
            "skipped — sequence-audio muxing lands with the linked-audio story (video-only export in P3)"
        } else {
            "none in preset"
        },
    }))
}

struct ExportJobParams {
    gpu: photonic_video::GpuContext,
    frozen: TimelineProject,
    seq_id: SequenceId,
    start: Tick,
    seq_rate: FrameRate,
    out_rate: FrameRate,
    format_size: (u32, u32),
    out_size: (u32, u32),
    preset: export_presets::ExportPreset,
    out_path: std::path::PathBuf,
    total_frames: u64,
    tools: ffmpeg_locate::FfmpegTools,
}

/// Export worker (10 §6): a **dedicated** `EngineSession` over the frozen
/// snapshot (isolated from the interactive session, so concurrent playback/
/// render tool calls can't disturb the export's seek-then-wait loop), feeding
/// `export::render_loop::export_frames` one readback frame per output tick.
fn run_export_job(
    jobs: std::sync::Arc<StdMutex<crate::handlers::video_jobs::JobRegistry>>,
    job_id: crate::handlers::video_jobs::JobId,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    p: ExportJobParams,
) {
    set_job_status(
        &jobs,
        job_id,
        JobStatus::Running {
            progress: 0.0,
            message: "starting engine session".into(),
        },
    );
    let engine = photonic_video::VideoEngine::new(p.gpu.clone());
    let shadow_doc = {
        let mut d = photonic_core::Document::new("export-shadow", 1.0, 1.0);
        d.timeline = Some(p.frozen);
        std::sync::Arc::new(StdMutex::new(d))
    };
    let shadow_history = std::sync::Arc::new(StdMutex::new(
        photonic_core::history::CommandHistory::new(1),
    ));
    let session = engine.open_session(shadow_doc, shadow_history);
    session.send(EngineCmd::SetActiveSequence(p.seq_id));
    // Export is always full quality (02 §7: identical headless path).
    session.send(EngineCmd::SetProxyMode(ProxyMode::ForceOriginal));

    let resolved = render_loop::ResolvedExport {
        width: p.out_size.0,
        height: p.out_size.1,
        frame_rate: p.out_rate,
        audio: None,
        out_path: p.out_path.clone(),
        colorimetry: photonic_render::color::Colorimetry::BT709_LIMITED,
    };
    let (fw, fh) = p.format_size;
    let (ow, oh) = p.out_size;
    let tpf_out = p.out_rate.ticks_per_frame().0.max(1);
    let mut prev = session.latest_frame();
    let mut frame_fail: Option<String> = None;
    let cancel_inner = std::sync::Arc::clone(&cancel);
    let frame_source = |i: u64| -> render_loop::Frame {
        // Output tick → nearest sequence frame (05 §6.2 retiming: the engine
        // presents exact sequence-grid ticks, so snap the output-grid tick).
        let t = Tick(p.start.0 + i as i64 * tpf_out);
        let snapped = p.seq_rate.frame_start(p.seq_rate.frame_at(t));
        session.send(EngineCmd::Seek(snapped));
        let deadline = Instant::now() + Duration::from_secs(30);
        let frame = loop {
            if let Some(f) = session.latest_frame() {
                let fresh = prev
                    .as_ref()
                    .map(|q| !std::sync::Arc::ptr_eq(q, &f))
                    .unwrap_or(true);
                if fresh && f.time == snapped && f.sequence == p.seq_id {
                    break Some(f);
                }
            }
            if Instant::now() >= deadline {
                break None;
            }
            std::thread::sleep(Duration::from_millis(2));
        };
        match frame {
            Some(f) => {
                prev = Some(std::sync::Arc::clone(&f));
                // Logical region only (bucket-padded texture, see render_frame_at).
                let px = read_texture_rgba16f(&p.gpu, &f.texture, fw, fh);
                let px = if (ow, oh) == (fw, fh) {
                    px
                } else {
                    box_downscale(&px, fw, fh, ow, oh)
                };
                render_loop::Frame {
                    width: ow,
                    height: oh,
                    rgba_premult: px.iter().flat_map(|q| q.iter().copied()).collect(),
                }
            }
            None => {
                // Poison the run: flag the failure, cancel between frames
                // (export_frames checks the flag before the next frame), and
                // hand back a black filler so the closure contract holds.
                frame_fail = Some(format!(
                    "frame {i} (tick {}) was not produced within 30s",
                    snapped.0
                ));
                cancel_inner.store(true, Ordering::Relaxed);
                render_loop::Frame {
                    width: ow,
                    height: oh,
                    rgba_premult: vec![0.0; (ow * oh * 4) as usize],
                }
            }
        }
    };
    let jobs_ev = std::sync::Arc::clone(&jobs);
    let on_event = |event: render_loop::ExportEvent| {
        if let render_loop::ExportEvent::Progress(pr) = event {
            set_job_status(
                &jobs_ev,
                job_id,
                JobStatus::Running {
                    progress: if pr.total > 0 {
                        pr.frame as f32 / pr.total as f32
                    } else {
                        0.0
                    },
                    message: format!(
                        "{}/{} frames ({:.1} fps, eta {:.0}s)",
                        pr.frame,
                        pr.total,
                        pr.fps,
                        pr.eta.as_secs_f32()
                    ),
                },
            );
        }
    };
    let result = render_loop::export_frames(
        &p.tools,
        &p.preset,
        &resolved,
        p.total_frames,
        frame_source,
        None,
        &cancel,
        on_event,
    );
    session.shutdown();
    match (result, frame_fail) {
        (_, Some(msg)) => set_job_status(
            &jobs,
            job_id,
            JobStatus::Failed {
                error_code: "RenderTimeout".into(),
                message: msg,
            },
        ),
        (Err(e), None) => set_job_status(
            &jobs,
            job_id,
            JobStatus::Failed {
                error_code: "ExportFailed".into(),
                message: e.to_string(),
            },
        ),
        (Ok(()), None) => {
            if cancel.load(Ordering::Relaxed) {
                set_job_status(&jobs, job_id, JobStatus::Cancelled);
            } else {
                set_job_status(
                    &jobs,
                    job_id,
                    JobStatus::Done {
                        result: json!({
                            "output_path": p.out_path,
                            "total_frames": p.total_frames,
                        }),
                    },
                );
            }
        }
    }
}

pub async fn get_job_status(state: &AppState, args: GetJobStatusArgs) -> ToolResult {
    tracing::debug!("tool: get_job_status {}", args.job_id);
    let payload = state
        .video_jobs
        .lock()
        .expect("job registry poisoned")
        .status_json(args.job_id);
    match payload {
        Some(v) => {
            let s = v["status"]["state"].as_str().unwrap_or("?").to_string();
            ToolResult::text(format!("job {} — {s}", args.job_id)).with_data(v)
        }
        None => err_code(
            "JobNotFound",
            format!(
                "job {} unknown or evicted (terminal jobs are retained 10 minutes)",
                args.job_id
            ),
        ),
    }
}

pub async fn cancel_job(state: &AppState, args: CancelJobArgs) -> ToolResult {
    tracing::debug!("tool: cancel_job {}", args.job_id);
    let outcome = state
        .video_jobs
        .lock()
        .expect("job registry poisoned")
        .request_cancel(args.job_id);
    match outcome {
        None => err_code(
            "JobNotFound",
            format!(
                "job {} unknown or evicted (terminal jobs are retained 10 minutes)",
                args.job_id
            ),
        ),
        Some(true) => ToolResult::text(
            "cancellation requested — the worker stops at its next check point \
             (between frames for exports)",
        ),
        Some(false) => ToolResult::text("job already finished — nothing to cancel"),
    }
}

pub async fn list_export_presets(_state: &AppState, _args: ListExportPresetsArgs) -> ToolResult {
    tracing::debug!("tool: list_export_presets");
    let mut out = Vec::new();
    for p in export_presets::built_in_presets() {
        let mut v = serde_json::to_value(&p).unwrap_or_default();
        v["built_in"] = json!(true);
        out.push(v);
    }
    let customs = export_presets::load_custom_presets().unwrap_or_default();
    for p in customs {
        let mut v = serde_json::to_value(&p).unwrap_or_default();
        v["built_in"] = json!(false);
        out.push(v);
    }
    ToolResult::text(format!("{} export preset(s)", out.len())).with_data(json!({ "presets": out }))
}

pub async fn save_export_preset(_state: &AppState, args: SaveExportPresetArgs) -> ToolResult {
    tracing::debug!("tool: save_export_preset {}", args.name);
    let mut preset: export_presets::ExportPreset = match serde_json::from_value(args.preset) {
        Ok(p) => p,
        Err(e) => {
            return ToolResult::error(format!(
                "invalid preset object: {e} — see list_export_presets output for the serde shape"
            ))
        }
    };
    preset.name = args.name.clone();
    if export_presets::built_in_presets()
        .iter()
        .any(|b| b.name == preset.name)
    {
        return err_code(
            "NotSupportedV1",
            format!(
                "{:?} is a built-in preset (read-only) — choose another name",
                preset.name
            ),
        );
    }
    if let Err(e) = export_presets::validate(&preset) {
        return ToolResult::error(format!("preset failed validation: {e}"));
    }
    let mut customs = match export_presets::load_custom_presets() {
        Ok(c) => c,
        Err(e) => return ToolResult::error(format!("could not read custom presets: {e}")),
    };
    let replaced = customs.iter().any(|p| p.name == preset.name);
    customs.retain(|p| p.name != preset.name);
    customs.push(preset);
    if let Err(e) = export_presets::save_custom_presets(&customs) {
        return ToolResult::error(format!("could not persist custom presets: {e}"));
    }
    ToolResult::text(format!(
        "{} custom preset {:?}",
        if replaced { "updated" } else { "saved" },
        args.name
    ))
}

pub async fn delete_export_preset(_state: &AppState, args: DeleteExportPresetArgs) -> ToolResult {
    tracing::debug!("tool: delete_export_preset {}", args.name);
    if export_presets::built_in_presets()
        .iter()
        .any(|b| b.name == args.name)
    {
        return err_code(
            "NotSupportedV1",
            format!(
                "{:?} is a built-in preset (read-only) — it cannot be deleted",
                args.name
            ),
        );
    }
    let mut customs = match export_presets::load_custom_presets() {
        Ok(c) => c,
        Err(e) => return ToolResult::error(format!("could not read custom presets: {e}")),
    };
    let before = customs.len();
    customs.retain(|p| p.name != args.name);
    if customs.len() == before {
        return ToolResult::error(format!("no custom preset named {:?}", args.name));
    }
    if let Err(e) = export_presets::save_custom_presets(&customs) {
        return ToolResult::error(format!("could not persist custom presets: {e}"));
    }
    ToolResult::text(format!("deleted custom preset {:?}", args.name))
}

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
            video_engine: Arc::new(crate::handlers::video_jobs::VideoEngineHandle::new()),
            video_jobs: Arc::new(StdMutex::new(
                crate::handlers::video_jobs::JobRegistry::new(),
            )),
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

    // ── Unit test: parse_timecode is frame-grid exact for NTSC (spec 10 §1.3) ──

    #[test]
    fn parse_timecode_ntsc_lands_on_frame_grid() {
        let fr = FrameRate::FPS_29_97; // 30000/1001
        let tpf = fr.ticks_per_frame().0;
        // 00:01:00:00 @ 29.97 is frame 1800 (60 s * 30 nominal fps), NOT 60
        // wall-clock seconds. The old wall-clock parse landed ~1.8 frames early.
        let t = parse_timecode("00:01:00:00", fr).unwrap();
        assert_eq!(t, Tick(1800 * tpf), "00:01:00:00 must be frame 1800");
        assert_eq!(t, Tick(42_378_336_000), "spec-mandated exact tick value");
        assert_eq!(fr.frame_at(t), 1800);
        // A wall-clock-seconds parse would have given 60 * TICKS_PER_SECOND,
        // which is a different (off-grid) value.
        assert_ne!(t, Tick::from_seconds(60));

        // Frame field carries within the nominal second: 00:00:00:29 = frame 29.
        assert_eq!(parse_timecode("00:00:00:29", fr).unwrap(), Tick(29 * tpf));
        // ';' (drop-frame marker) is accepted but not compensated — same value.
        assert_eq!(parse_timecode("00:01:00;00", fr).unwrap(), Tick(1800 * tpf));
    }

    #[test]
    fn parse_timecode_integer_rate_matches_wall_clock() {
        // For exact integer rates a whole-second timecode coincides with
        // wall-clock seconds (1 s = 30 frames @ 30 fps).
        let fr = FrameRate::FPS_30;
        assert_eq!(
            parse_timecode("00:00:01:00", fr).unwrap(),
            Tick::from_seconds(1)
        );
        assert_eq!(
            parse_timecode("01:00:00:00", fr).unwrap(),
            Tick::from_seconds(3600)
        );
    }

    #[test]
    fn parse_timecode_rejects_out_of_range_and_malformed() {
        let fr = FrameRate::FPS_30;
        // Frame field must be < nominal_fps (30).
        assert!(parse_timecode("00:00:00:30", fr).is_none());
        // Minutes / seconds must be < 60.
        assert!(parse_timecode("00:60:00:00", fr).is_none());
        assert!(parse_timecode("00:00:60:00", fr).is_none());
        // Negative components are rejected.
        assert!(parse_timecode("-1:00:00:00", fr).is_none());
        assert!(parse_timecode("00:-1:00:00", fr).is_none());
        assert!(parse_timecode("00:00:00:-1", fr).is_none());
        // Structurally malformed: missing frame field / wrong part count.
        assert!(parse_timecode("00:01:00", fr).is_none());
        assert!(parse_timecode("00:00:00:00:00", fr).is_none());
        assert!(parse_timecode("garbage", fr).is_none());
        assert!(parse_timecode("aa:bb:cc:dd", fr).is_none());
        // Boundary that IS valid: last frame of the second.
        assert!(parse_timecode("00:00:00:29", fr).is_some());
    }

    #[test]
    fn parse_timecode_every_valid_tc_is_frame_aligned() {
        // Property: for a spread of rates and components, any timecode the parser
        // accepts lands exactly on a frame boundary (snap is a no-op).
        for fr in [
            FrameRate::FPS_24,
            FrameRate::FPS_25,
            FrameRate::FPS_30,
            FrameRate::FPS_60,
            FrameRate::FPS_29_97,
            FrameRate::FPS_23_976,
        ] {
            let den = fr.den.max(1) as i64;
            let nominal = ((fr.num as i64) + den / 2) / den;
            for &(h, m, s) in &[(0, 0, 0), (0, 1, 0), (1, 23, 45), (2, 59, 59), (10, 0, 30)] {
                for ff in [0, 1, nominal / 2, nominal - 1] {
                    let tc = format!("{h:02}:{m:02}:{s:02}:{ff:02}");
                    let t = parse_timecode(&tc, fr)
                        .unwrap_or_else(|| panic!("{tc} @ {fr:?} should parse"));
                    assert_eq!(fr.snap(t), t, "{tc} @ {fr:?} not frame-aligned");
                    assert_eq!(
                        fr.frame_start(fr.frame_at(t)),
                        t,
                        "{tc} @ {fr:?} not on its own frame boundary"
                    );
                }
            }
        }
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

        // Cross-track move.
        let other_track = create_track(&state, &seq_id, "video").await;
        let r = call(
            &state,
            "move_clip",
            json!({ "clip_id": clip_a, "new_start_ticks": 6000, "new_track_id": other_track }),
        )
        .await;
        assert_ne!(r.is_error, Some(true), "cross-track move_clip: {r:?}");
        let r = call(&state, "list_clips", json!({ "track_id": other_track })).await;
        let clips_on_dest = data(&r)["clips"].as_array().cloned().unwrap_or_default();
        assert_eq!(
            clips_on_dest.len(),
            1,
            "clip should have landed on the destination track: {clips_on_dest:?}"
        );

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

    #[tokio::test]
    async fn ripple_edit_trims_and_shifts_later_clips() {
        let state = test_state();
        let (_, track_id) = create_seq_and_track(&state, "video").await;
        let clip_a = insert_solid_clip(&state, &track_id, 0, 1000).await;
        let clip_b = insert_solid_clip(&state, &track_id, 1000, 1000).await;

        // Trim clip_a's out-point later by 200 ticks; clip_b should shift later
        // by 200 to close the resulting gap — one undo step.
        let r = call(
            &state,
            "ripple_edit",
            json!({ "clip_id": clip_a, "edge": "out", "delta_ticks": 200 }),
        )
        .await;
        assert_ne!(r.is_error, Some(true), "ripple_edit: {r:?}");

        let r = call(&state, "get_clip", json!({ "clip_id": clip_a })).await;
        assert_eq!(data(&r)["clip"]["duration"], json!(1200));

        let r = call(&state, "get_clip", json!({ "clip_id": clip_b })).await;
        assert_eq!(
            data(&r)["clip"]["start"],
            json!(1200),
            "clip_b should shift later by 200 to close the gap"
        );

        let r = call(&state, "undo", json!({})).await;
        assert_ne!(r.is_error, Some(true), "undo: {r:?}");
        let r = call(&state, "get_clip", json!({ "clip_id": clip_a })).await;
        assert_eq!(
            data(&r)["clip"]["duration"],
            json!(1000),
            "ripple_edit should undo as one step"
        );
        let r = call(&state, "get_clip", json!({ "clip_id": clip_b })).await;
        assert_eq!(data(&r)["clip"]["start"], json!(1000));
    }

    #[tokio::test]
    async fn cross_track_move_round_trips_via_undo() {
        let state = test_state();
        let (seq_id, track_a) = create_seq_and_track(&state, "video").await;
        let track_b = create_track(&state, &seq_id, "video").await;
        let clip_id = insert_solid_clip(&state, &track_a, 0, 1000).await;

        let r = call(
            &state,
            "move_clip",
            json!({ "clip_id": clip_id, "new_start_ticks": 2000, "new_track_id": track_b }),
        )
        .await;
        assert_ne!(r.is_error, Some(true), "move_clip cross-track: {r:?}");

        let r = call(&state, "list_clips", json!({ "track_id": track_b })).await;
        assert_eq!(
            data(&r)["clips"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .len(),
            1
        );
        let r = call(&state, "list_clips", json!({ "track_id": track_a })).await;
        assert_eq!(
            data(&r)["clips"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .len(),
            0
        );

        let r = call(&state, "undo", json!({})).await;
        assert_ne!(r.is_error, Some(true), "undo: {r:?}");

        let r = call(&state, "list_clips", json!({ "track_id": track_a })).await;
        let clips_a = data(&r)["clips"].as_array().cloned().unwrap_or_default();
        assert_eq!(
            clips_a.len(),
            1,
            "clip should be back on track_a after undo"
        );
        assert_eq!(clips_a[0]["start_ticks"], json!(0));
        let r = call(&state, "list_clips", json!({ "track_id": track_b })).await;
        assert_eq!(
            data(&r)["clips"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .len(),
            0
        );
    }

    // ── Tool family: markers + work range ─────────────────────────────────────

    #[tokio::test]
    async fn family_markers_and_work_range() {
        let state = test_state();
        let (seq_id, _) = create_seq_and_track(&state, "video").await;

        let r = call(
            &state,
            "add_marker",
            json!({ "sequence_id": seq_id, "at_ticks": 100, "name": "Beat 1", "color": "#ff0000" }),
        )
        .await;
        assert_ne!(r.is_error, Some(true), "add_marker: {r:?}");
        let marker_id = data(&r)["marker_id"].clone();

        let r = call(
            &state,
            "add_marker",
            json!({ "sequence_id": seq_id, "at_ticks": 500, "name": "Beat 2" }),
        )
        .await;
        assert_ne!(r.is_error, Some(true), "add_marker 2: {r:?}");

        let r = call(&state, "list_markers", json!({ "sequence_id": seq_id })).await;
        assert_ne!(r.is_error, Some(true), "list_markers: {r:?}");
        let markers = data(&r)["markers"].as_array().cloned().unwrap_or_default();
        assert_eq!(markers.len(), 2, "{markers:?}");

        let r = call(&state, "remove_marker", json!({ "marker_id": marker_id })).await;
        assert_ne!(r.is_error, Some(true), "remove_marker: {r:?}");
        let r = call(&state, "list_markers", json!({ "sequence_id": seq_id })).await;
        assert_eq!(
            data(&r)["markers"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .len(),
            1
        );

        let r = call(
            &state,
            "set_work_range",
            json!({ "sequence_id": seq_id, "range": {"start_ticks": 0, "end_ticks": 1000} }),
        )
        .await;
        assert_ne!(r.is_error, Some(true), "set_work_range: {r:?}");

        let r = call(&state, "set_work_range", json!({ "sequence_id": seq_id })).await;
        assert_ne!(r.is_error, Some(true), "set_work_range clear: {r:?}");
    }

    // ── Tool family: media bins ────────────────────────────────────────────────

    #[tokio::test]
    async fn family_media_bins() {
        let state = test_state();
        let tmp = std::env::temp_dir().join(format!(
            "photonic_mcp_bin_test_{}.mp4",
            uuid::Uuid::new_v4()
        ));
        std::fs::write(&tmp, b"bin test bytes").unwrap();

        // import_media auto-creates the bin by name.
        let r = call(
            &state,
            "import_media",
            json!({ "paths": [tmp.to_string_lossy()], "bin": "Interviews" }),
        )
        .await;
        assert_ne!(r.is_error, Some(true), "import_media: {r:?}");
        let assets = data(&r)["assets"].as_array().cloned().unwrap_or_default();
        assert_eq!(assets.len(), 1);
        let asset_id = assets[0]["asset_id"].clone();
        assert!(
            !assets[0]["bin_id"].is_null(),
            "asset should carry a bin_id"
        );

        let r = call(&state, "list_bins", json!({})).await;
        assert_ne!(r.is_error, Some(true), "list_bins: {r:?}");
        let bins = data(&r)["bins"].as_array().cloned().unwrap_or_default();
        assert_eq!(bins.len(), 1);
        assert_eq!(bins[0]["name"], json!("Interviews"));
        let interviews_bin_id = bins[0]["bin_id"].clone();

        let r = call(&state, "list_media", json!({ "bin": "Interviews" })).await;
        assert_ne!(r.is_error, Some(true), "list_media filter: {r:?}");
        assert_eq!(
            data(&r)["assets"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .len(),
            1
        );

        let r = call(&state, "list_media", json!({ "bin": "Nonexistent" })).await;
        assert_ne!(
            r.is_error,
            Some(true),
            "list_media filter (no match): {r:?}"
        );
        assert_eq!(
            data(&r)["assets"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .len(),
            0
        );

        let r = call(
            &state,
            "set_asset_bin",
            json!({ "asset_id": asset_id, "bin_id": null }),
        )
        .await;
        assert_ne!(r.is_error, Some(true), "set_asset_bin clear: {r:?}");
        let r = call(&state, "list_media", json!({ "bin": "Interviews" })).await;
        assert_eq!(
            data(&r)["assets"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .len(),
            0
        );

        let r = call(&state, "create_bin", json!({ "name": "B-Roll" })).await;
        assert_ne!(r.is_error, Some(true), "create_bin: {r:?}");
        let broll_bin_id = data(&r)["bin_id"].clone();
        let r = call(
            &state,
            "set_asset_bin",
            json!({ "asset_id": asset_id, "bin_id": broll_bin_id }),
        )
        .await;
        assert_ne!(r.is_error, Some(true), "set_asset_bin: {r:?}");

        let r = call(&state, "remove_bin", json!({ "bin_id": interviews_bin_id })).await;
        assert_ne!(r.is_error, Some(true), "remove_bin: {r:?}");

        let _ = std::fs::remove_file(&tmp);
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
        "set_work_range",
        "add_marker",
        "remove_marker",
        "list_markers",
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
        "ripple_edit",
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
        "create_bin",
        "remove_bin",
        "set_asset_bin",
        "list_bins",
        // P3 engine slice
        "play",
        "pause",
        "seek",
        "step",
        "set_loop_range",
        "set_proxy_mode",
        "get_engine_status",
        "render_frame_at",
        "probe_media",
        "generate_proxies",
        "remove_proxy",
        "transcode_media",
        "export_sequence",
        "get_job_status",
        "cancel_job",
        "list_export_presets",
        "save_export_preset",
        "delete_export_preset",
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

    // ─── P3 engine slice tests (10 §9 hooks 4/5 + §7 headless matrix) ────────

    /// GPU-gate: engine-backed tests skip (adapter-skip convention) when the
    /// lazy bridge can't get an adapter. Detected via the structured
    /// `EngineUnavailable` error the tools themselves return.
    async fn engine_available(state: &AppState) -> bool {
        let r = call(state, "get_engine_status", json!({})).await;
        if r.is_error == Some(true) {
            eprintln!("no GPU adapter — skipping engine-backed MCP test");
            return false;
        }
        true
    }

    /// 10 §9 hook 4: registry/cancel/GC wiring through the REAL tool
    /// dispatch, against a fake job — no ffmpeg, no engine, no GPU.
    #[tokio::test]
    async fn job_tools_lifecycle_with_a_fake_job() {
        use crate::handlers::video_jobs::JobStatus;
        let state = test_state();

        // Unknown id → JobNotFound (structured, 10 §8).
        let r = call(
            &state,
            "get_job_status",
            json!({ "job_id": "00000000-0000-0000-0000-000000000000" }),
        )
        .await;
        assert_eq!(r.is_error, Some(true));
        assert_eq!(data(&r)["error_code"], "JobNotFound");

        let (job_id, cancel) = state.video_jobs.lock().unwrap().start("fake");
        let jid = json!({ "job_id": job_id });

        let r = call(&state, "get_job_status", jid.clone()).await;
        assert_ne!(r.is_error, Some(true));
        assert_eq!(data(&r)["status"]["state"], "queued");

        state.video_jobs.lock().unwrap().set_status(
            job_id,
            JobStatus::Running {
                progress: 0.25,
                message: "working".into(),
            },
        );
        let r = call(&state, "get_job_status", jid.clone()).await;
        assert_eq!(data(&r)["status"]["state"], "running");
        assert_eq!(data(&r)["status"]["progress"], 0.25);

        // cancel_job flags the worker's AtomicBool cooperatively.
        let r = call(&state, "cancel_job", jid.clone()).await;
        assert_ne!(r.is_error, Some(true));
        assert!(cancel.load(std::sync::atomic::Ordering::Relaxed));

        state.video_jobs.lock().unwrap().set_status(
            job_id,
            JobStatus::Done {
                result: json!({ "ok": true }),
            },
        );
        let r = call(&state, "get_job_status", jid.clone()).await;
        assert_eq!(data(&r)["status"]["state"], "done");
        assert_eq!(data(&r)["status"]["result"]["ok"], true);

        // Cancelling a finished job is a no-op, not an error.
        let r = call(&state, "cancel_job", jid.clone()).await;
        assert_ne!(r.is_error, Some(true));

        // GC with zero retention evicts the terminal job → JobNotFound.
        state
            .video_jobs
            .lock()
            .unwrap()
            .gc(std::time::Duration::from_secs(0));
        let r = call(&state, "get_job_status", jid).await;
        assert_eq!(r.is_error, Some(true));
        assert_eq!(data(&r)["error_code"], "JobNotFound");
    }

    /// 10 §7: the playback surface works headless — on a box with no audio
    /// device the engine plays on the soft clock; none of the transport tools
    /// error. (GPU-gated; on a no-GPU box the gate itself asserts the clean
    /// `EngineUnavailable` degradation.)
    #[tokio::test]
    async fn playback_tools_degrade_cleanly_headless() {
        let state = test_state();
        let (seq_id, track_id) = create_seq_and_track(&state, "video").await;
        insert_solid_clip(&state, &track_id, 0, TICKS_PER_SECOND * 2).await;
        if !engine_available(&state).await {
            return;
        }

        let r = call(&state, "play", json!({ "sequence_id": seq_id })).await;
        assert_ne!(r.is_error, Some(true), "play: {r:?}");
        assert_eq!(
            data(&r)["playing"],
            true,
            "headless play must run on the soft clock, not fail: {r:?}"
        );

        let r = call(&state, "pause", json!({})).await;
        assert_ne!(r.is_error, Some(true), "pause: {r:?}");
        assert_eq!(data(&r)["playing"], false);

        // Paused seek is a pure clock set — playhead confirms exactly. The
        // target sits far past any soft-clock drift the play/pause window
        // above could have accumulated (seeking past content is legal).
        let tpf = TICKS_PER_SECOND / 30;
        let target = 300 * tpf;
        let r = call(
            &state,
            "seek",
            json!({ "sequence_id": seq_id, "at_ticks": target }),
        )
        .await;
        assert_ne!(r.is_error, Some(true), "seek: {r:?}");
        assert_eq!(data(&r)["playhead_ticks"], target);

        // Step +1 pauses (already paused) and lands on the next frame start.
        let r = call(&state, "step", json!({ "frames": 1 })).await;
        assert_ne!(r.is_error, Some(true), "step: {r:?}");
        assert_eq!(data(&r)["playing"], false);
        assert_eq!(data(&r)["playhead_ticks"], target + tpf);

        let r = call(
            &state,
            "set_loop_range",
            json!({
                "sequence_id": seq_id,
                "range": { "start_ticks": 0, "end_ticks": TICKS_PER_SECOND }
            }),
        )
        .await;
        assert_ne!(r.is_error, Some(true), "set_loop_range: {r:?}");

        let r = call(
            &state,
            "set_proxy_mode",
            json!({ "mode": "force_original" }),
        )
        .await;
        assert_ne!(r.is_error, Some(true), "set_proxy_mode: {r:?}");

        let r = call(&state, "get_engine_status", json!({})).await;
        assert_ne!(r.is_error, Some(true), "get_engine_status: {r:?}");
        assert_eq!(data(&r)["active_sequence"], seq_id);
        assert_eq!(data(&r)["snapshot_synced"], true);
    }

    /// STALE-PLAYHEAD gate: a paused *backward* seek must report the tick it
    /// landed on, not the (larger) playhead one command behind. Before the
    /// fresh-frame settle, `seek`'s `playhead >= t` predicate returned the
    /// stale pre-seek tick immediately (seek F45 reporting F150), and an
    /// immediately-following `get_engine_status` echoed it. (GPU-gated; on a
    /// no-GPU box the gate asserts the clean `EngineUnavailable` degradation.)
    #[tokio::test]
    async fn seek_then_status_reports_the_seek_target_not_one_behind() {
        let state = test_state();
        let (seq_id, track_id) = create_seq_and_track(&state, "video").await;
        insert_solid_clip(&state, &track_id, 0, TICKS_PER_SECOND * 20).await;
        if !engine_available(&state).await {
            return;
        }
        let tpf = TICKS_PER_SECOND / 30;

        // Seek far forward first so the published playhead is large.
        let forward = 150 * tpf;
        let r = call(
            &state,
            "seek",
            json!({ "sequence_id": seq_id, "at_ticks": forward }),
        )
        .await;
        assert_ne!(r.is_error, Some(true), "forward seek: {r:?}");
        assert_eq!(data(&r)["playhead_ticks"], forward);

        // Now seek backward. The old `>=` predicate would report `forward`
        // (150) rather than the tick this command actually lands on (45).
        let backward = 45 * tpf;
        let r = call(
            &state,
            "seek",
            json!({ "sequence_id": seq_id, "at_ticks": backward }),
        )
        .await;
        assert_ne!(r.is_error, Some(true), "backward seek: {r:?}");
        assert_eq!(
            data(&r)["playhead_ticks"],
            backward,
            "backward seek must report the tick it landed on, not one behind"
        );

        // An immediately-following status read must agree — not lag a command.
        let r = call(&state, "get_engine_status", json!({})).await;
        assert_ne!(r.is_error, Some(true), "get_engine_status: {r:?}");
        assert_eq!(
            data(&r)["playhead_ticks"],
            backward,
            "get_engine_status must reflect the last seek, not a stale playhead"
        );
    }

    /// 10 §9 hook 5: two `render_frame_at` calls with the same args and
    /// `output_format: raw_rgba16f` are byte-identical (the evaluator's pure-
    /// function property) — plus a pixel probe and the png/scale smoke.
    #[tokio::test]
    async fn render_frame_at_is_deterministic() {
        let state = test_state();
        // Small format keeps the raw payload cheap (320*180*8 ≈ 460 KB).
        let r = call(
            &state,
            "create_sequence",
            json!({
                "name": "Render Seq", "frame_rate": {"num": 30, "den": 1},
                "formats": [{"name": "16:9", "width": 320, "height": 180}]
            }),
        )
        .await;
        assert_ne!(r.is_error, Some(true), "create_sequence: {r:?}");
        let seq_id = data(&r)["sequence_id"].clone();
        let track_id = create_track(&state, &seq_id, "video").await;
        insert_solid_clip(&state, &track_id, 0, TICKS_PER_SECOND * 2).await;
        if !engine_available(&state).await {
            return;
        }

        let args = json!({
            "sequence_id": seq_id,
            "at_seconds": 0.5,
            "quality": "full",
            "output_format": "raw_rgba16f",
        });
        let r1 = call(&state, "render_frame_at", args.clone()).await;
        assert_ne!(r1.is_error, Some(true), "render 1: {r1:?}");
        let r2 = call(&state, "render_frame_at", args).await;
        assert_ne!(r2.is_error, Some(true), "render 2: {r2:?}");
        let (d1, d2) = (data(&r1), data(&r2));
        assert_eq!(d1["width"], 320);
        assert_eq!(d1["height"], 180);
        // Exact frame tick: 0.5s @30fps = frame 15.
        assert_eq!(d1["tick"], 15 * (TICKS_PER_SECOND / 30));
        let b1 = d1["data_base64"].as_str().expect("raw payload");
        let b2 = d2["data_base64"].as_str().expect("raw payload");
        assert!(!b1.is_empty());
        assert_eq!(b1, b2, "raw_rgba16f output must be byte-deterministic");

        // Pixel probe: the solid #00ff00 clip must read back green
        // (linear premultiplied f16, interleaved RGBA little-endian).
        use base64::Engine as _;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b1)
            .expect("base64");
        assert_eq!(bytes.len(), 320 * 180 * 8);
        let px = ((180 / 2) * 320 + 320 / 2) * 8;
        let f16 = |o: usize| {
            let bits = u16::from_le_bytes([bytes[o], bytes[o + 1]]);
            // Positive-normal decode is enough for a 0..1 color probe.
            let exp = ((bits >> 10) & 0x1f) as i32;
            let frac = (bits & 0x3ff) as f32;
            if exp == 0 {
                frac * 2f32.powi(-24)
            } else {
                (1.0 + frac / 1024.0) * 2f32.powi(exp - 15)
            }
        };
        let (r, g, a) = (f16(px), f16(px + 2), f16(px + 6));
        assert!(r < 0.05, "red channel should be ~0, got {r}");
        assert!(g > 0.9, "green channel should be ~1, got {g}");
        assert!(a > 0.99, "alpha should be 1, got {a}");

        // PNG + scale smoke: returns an image content item at scaled dims.
        let r = call(
            &state,
            "render_frame_at",
            json!({
                "sequence_id": seq_id,
                "at_ticks": 0,
                "quality": "preview",
                "scale": 0.25,
            }),
        )
        .await;
        assert_ne!(r.is_error, Some(true), "png render: {r:?}");
        assert!(
            r.content
                .iter()
                .any(|c| matches!(c, ContentItem::Image { .. })),
            "png output must include an image content item"
        );
        // The data payload sits after the image item here (text, image, json).
        let d = match r.content.last() {
            Some(ContentItem::Text { text }) => serde_json::from_str::<Value>(text).unwrap(),
            other => panic!("expected trailing json payload, got {other:?}"),
        };
        assert_eq!(d["width"], 80);
        assert_eq!(d["height"], 45);
    }

    /// Export E2E (GPU + ffmpeg): a 1-second solid-color sequence through
    /// `export_sequence` → poll `get_job_status` to done → ffprobe the output
    /// (dims + duration). Skips without a GPU adapter or ffmpeg toolchain.
    #[tokio::test]
    async fn export_sequence_e2e_solid_color() {
        let state = test_state();
        let r = call(
            &state,
            "create_sequence",
            json!({
                "name": "Export Seq", "frame_rate": {"num": 30, "den": 1},
                "formats": [{"name": "1:1", "width": 128, "height": 128}]
            }),
        )
        .await;
        let seq_id = data(&r)["sequence_id"].clone();
        let track_id = create_track(&state, &seq_id, "video").await;
        insert_solid_clip(&state, &track_id, 0, TICKS_PER_SECOND).await;
        if !engine_available(&state).await {
            return;
        }
        let Some(tools) = photonic_video::media::ffmpeg_locate::locate_for_test() else {
            eprintln!("ffmpeg/ffprobe not found — skipping export E2E test");
            return;
        };

        let out = std::env::temp_dir().join(format!(
            "photonic-mcp-export-e2e-{}.mp4",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&out);
        let r = call(
            &state,
            "export_sequence",
            json!({
                "sequence_id": seq_id,
                "out_path": out.to_string_lossy(),
                "preset": "Web H.264",
            }),
        )
        .await;
        assert_ne!(r.is_error, Some(true), "export_sequence: {r:?}");
        let d = data(&r);
        assert_eq!(d["total_frames"], 30);
        let job_id = d["job_id"].clone();

        // Poll to terminal state (cold engine spin-up + encode ≪ 120 s).
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        let final_state = loop {
            let r = call(&state, "get_job_status", json!({ "job_id": job_id })).await;
            assert_ne!(r.is_error, Some(true), "get_job_status: {r:?}");
            let d = data(&r);
            let s = d["status"]["state"].as_str().unwrap().to_string();
            if s != "queued" && s != "running" {
                break d;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "export did not finish in time (last: {d})"
            );
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        };
        assert_eq!(
            final_state["status"]["state"], "done",
            "export failed: {final_state}"
        );
        assert!(out.exists(), "output file missing: {}", out.display());

        // ffprobe the result: 128x128, ~1 s of video.
        let probe = std::process::Command::new(&tools.ffprobe)
            .args([
                "-v",
                "error",
                "-print_format",
                "json",
                "-show_streams",
                "-show_format",
            ])
            .arg(&out)
            .output()
            .expect("run ffprobe");
        assert!(probe.status.success());
        let meta: serde_json::Value = serde_json::from_slice(&probe.stdout).unwrap();
        let stream = meta["streams"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["codec_type"] == "video")
            .expect("video stream");
        assert_eq!(stream["width"], 128);
        assert_eq!(stream["height"], 128);
        let duration: f64 = meta["format"]["duration"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        assert!(
            (duration - 1.0).abs() < 0.2,
            "expected ~1s of video, got {duration}s"
        );
        let _ = std::fs::remove_file(&out);
    }
}
