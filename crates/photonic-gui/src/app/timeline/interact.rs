//! Pointer-intent computation for the timeline (04 §2.3–§2.5): edge-zone hit
//! testing, snapping-candidate selection, and the transient drag/marquee state.
//!
//! Everything above the "3/4-point source editing" section is pure or
//! session-transient — no `doc.timeline` mutation and no history calls (that is
//! `ops_bridge.rs`'s job). `mod.rs` calls these to turn raw pointer positions
//! into an intent, then hands the intent to `ops_bridge`.
//!
//! The one sanctioned exception is the Insert/Overwrite/Lift/Extract edit
//! helpers at the bottom (spec 16 §4): the story that owns `ops_bridge.rs` is a
//! separate territory, so per that spec these four `ops::*`→`CommandHistory`
//! bridges live here instead. They mirror `ops_bridge`'s discipline exactly —
//! one atomic, undoable batch per edit — and are the sole place this module
//! touches history.

use photonic_core::document::Document;
use photonic_core::history::{Command, CommandHistory};
use photonic_core::timeline::{
    ops, Clip, ClipId, ClipSource, ClipTiming, Sequence, SequenceId, Tick, TimelineCmd, TrackId,
    TrackKind,
};

/// Which part of a clip rect the pointer is over (04 §2.3: 6px edge zones = trim,
/// interior = body/move).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ClipZone {
    LeftEdge,
    Body,
    RightEdge,
}

/// The active drag gesture kind (resolved from zone + modifiers at drag-start).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum DragKind {
    Move,
    TrimStart,
    TrimEnd,
    RippleTrimStart,
    RippleTrimEnd,
    Slip,
    Slide,
    /// Roll the boundary shared with the clip to the right (`right` is that clip).
    Roll {
        right: ClipId,
    },
}

/// Transient per-gesture state, stashed in egui temp memory across frames so the
/// gesture reads its drag-start snapshot rather than re-deriving it each frame
/// (04 §7 immediate-mode mitigation).
#[derive(Clone)]
pub(crate) struct DragState {
    pub kind: DragKind,
    pub track: TrackId,
    pub clip: ClipId,
    /// Lane-space tick under the pointer when the drag began.
    pub grab_tick: Tick,
    /// Timing of the primary clip at drag-start (the undo `old`).
    pub orig: ClipTiming,
    /// Track the pointer currently hovers (for a cross-track move), else `track`.
    pub dest_track: TrackId,
    /// Snap candidates, precomputed once at drag-start in priority order (04 §2.5).
    pub candidates: Vec<Tick>,
    /// Set once the pointer has actually moved — distinguishes a click from a drag.
    pub moved: bool,
}

/// Transient marquee (rubber-band) select state.
#[derive(Clone, Copy)]
pub(crate) struct Marquee {
    pub start: egui::Pos2,
    pub additive: bool,
}

/// One painted clip rect offered up for pointer hit-testing, tagged with its
/// track and that track's lock state (04 §2.3 / 14-nle-parity QW-2).
#[derive(Clone, Copy)]
pub(crate) struct HitCandidate {
    pub track: TrackId,
    pub clip: ClipId,
    pub rect: egui::Rect,
    pub locked: bool,
}

/// Hit-test a pointer position against painted clip candidates, honoring
/// track lock: a candidate whose track is locked is never a hit. Selection,
/// drag-start, and the context-menu target all resolve through this one
/// function, so a locked track rejects every clip edit while its clips stay
/// fully visible/paintable — only interaction is gated here, not painting
/// (`clips.rs::paint_lane` draws locked lanes unconditionally, plus a
/// diagonal-hatch cue).
pub(crate) fn hit_at(
    pos: egui::Pos2,
    edge: f32,
    candidates: &[HitCandidate],
) -> Option<(TrackId, ClipId, egui::Rect, ClipZone)> {
    for h in candidates {
        if h.locked {
            continue;
        }
        if pos.y >= h.rect.top() && pos.y <= h.rect.bottom() {
            if let Some(z) = hit_zone(h.rect.left(), h.rect.right(), pos.x, edge) {
                return Some((h.track, h.clip, h.rect, z));
            }
        }
    }
    None
}

/// Hit-test a pointer x against a clip rect `[x0, x1]`. Returns the zone, or
/// `None` if the pointer is outside the rect. Clips narrower than `2*edge` split
/// at the midpoint so a tiny clip is still trimmable from both sides.
pub(crate) fn hit_zone(x0: f32, x1: f32, px: f32, edge: f32) -> Option<ClipZone> {
    if px < x0 || px > x1 {
        return None;
    }
    let width = x1 - x0;
    if width <= 2.0 * edge {
        let mid = x0 + width * 0.5;
        return Some(if px < mid {
            ClipZone::LeftEdge
        } else {
            ClipZone::RightEdge
        });
    }
    if px <= x0 + edge {
        Some(ClipZone::LeftEdge)
    } else if px >= x1 - edge {
        Some(ClipZone::RightEdge)
    } else {
        Some(ClipZone::Body)
    }
}

/// Choose a snap target for `value` from `candidates` (priority-ordered). Returns
/// the first candidate within `threshold` ticks, so a higher-priority candidate
/// wins even when a lower-priority one is marginally closer (04 §2.5).
pub(crate) fn nearest_snap(value: Tick, candidates: &[Tick], threshold: Tick) -> Option<Tick> {
    let thr = threshold.0.abs();
    candidates
        .iter()
        .copied()
        .find(|c| (c.0 - value.0).abs() <= thr)
}

/// Apply snapping (when enabled) then frame-quantize. `value` is a raw lane tick;
/// the result always lands on a frame boundary (04 §2.1).
pub(crate) fn snap_and_quantize(
    value: Tick,
    candidates: &[Tick],
    threshold: Tick,
    snap_enabled: bool,
    frame_rate: photonic_core::timeline::FrameRate,
) -> Tick {
    let snapped = if snap_enabled {
        nearest_snap(value, candidates, threshold).unwrap_or(value)
    } else {
        value
    };
    frame_rate.snap(snapped)
}

/// Build the snap-candidate list for a drag on `track`, excluding `moving`'s own
/// edges, in the priority order 04 §2.5 mandates: same-track edges → other-track
/// edges → playhead → markers.
pub(crate) fn build_snap_candidates(
    seq: &Sequence,
    track: TrackId,
    moving: ClipId,
    playhead: Tick,
) -> Vec<Tick> {
    let mut out = Vec::new();
    // 1. Same-track clip edges.
    if let Some(t) = seq.track(track) {
        for c in &t.clips {
            if c.id != moving {
                out.push(c.start);
                out.push(c.end());
            }
        }
    }
    // 2. Other-track clip edges.
    for t in seq.tracks() {
        if t.id == track {
            continue;
        }
        for c in &t.clips {
            out.push(c.start);
            out.push(c.end());
        }
    }
    // 3. Playhead.
    out.push(playhead);
    // 4. Markers.
    for m in &seq.markers {
        out.push(m.at);
    }
    out
}

/// Resolve a drag-start into its gesture kind and the *primary* clip the gesture
/// operates on (04 §2.4 modifier/hit-test scheme). Pure: the caller supplies the
/// dragged clip plus, for edge drags, the id of the neighbour that shares the
/// exact boundary on that side (`None` when there is no flush neighbour, so a
/// Roll is impossible).
///
/// Rules:
/// - Body: Alt+Shift ⇒ Slide, Alt ⇒ Slip, plain ⇒ Move (primary = `clip`).
/// - LeftEdge with a flush left neighbour and no Shift ⇒ Roll (primary = the LEFT
///   clip, `Roll.right` = `clip`); Shift ⇒ RippleTrimStart; plain ⇒ TrimStart.
/// - RightEdge with a flush right neighbour and no Shift ⇒ Roll (primary = `clip`,
///   `Roll.right` = the right neighbour); Shift ⇒ RippleTrimEnd; plain ⇒ TrimEnd.
///
/// Shift always suppresses Roll in favour of a ripple trim, matching the drag
/// commit logic in `mod.rs`.
pub(crate) fn resolve_drag_kind(
    zone: ClipZone,
    alt: bool,
    shift: bool,
    clip: ClipId,
    left_shared: Option<ClipId>,
    right_shared: Option<ClipId>,
) -> (DragKind, ClipId) {
    match zone {
        ClipZone::Body => {
            let k = if alt && shift {
                DragKind::Slide
            } else if alt {
                DragKind::Slip
            } else {
                DragKind::Move
            };
            (k, clip)
        }
        ClipZone::LeftEdge => {
            if let (Some(prev), false) = (left_shared, shift) {
                (DragKind::Roll { right: clip }, prev)
            } else if shift {
                (DragKind::RippleTrimStart, clip)
            } else {
                (DragKind::TrimStart, clip)
            }
        }
        ClipZone::RightEdge => {
            if let (Some(next), false) = (right_shared, shift) {
                (DragKind::Roll { right: next }, clip)
            } else if shift {
                (DragKind::RippleTrimEnd, clip)
            } else {
                (DragKind::TrimEnd, clip)
            }
        }
    }
}

/// Apply a marquee (rubber-band) rect over the visible clip rects, mutating
/// `selection` (04 §2.6). Replace semantics (`additive == false`) clears the
/// selection first; additive (Ctrl/Shift held) keeps it. A clip joins the
/// selection when its rect *intersects* the marquee — touching or partial
/// overlap counts, full containment is not required — and a clip already in the
/// selection is never pushed twice.
pub(crate) fn apply_marquee(
    marquee: egui::Rect,
    hits: impl IntoIterator<Item = (egui::Rect, ClipId)>,
    additive: bool,
    selection: &mut Vec<ClipId>,
) {
    if !additive {
        selection.clear();
    }
    for (rect, clip) in hits {
        if rect.intersects(marquee) && !selection.contains(&clip) {
            selection.push(clip);
        }
    }
}

// ── 3/4-point source editing (spec 16 — Insert / Overwrite / Lift / Extract) ──
//
// `PendingSource` is the GUI-session "armed" source range for a 3-point edit
// (spec 16 §1) — the source + its in/out, held ONLY in session state, never in
// the document. The four `do_*_edit` helpers turn that (plus a timeline point or
// work-range) into one atomic, undoable `TimelineCmd` batch via the pure core
// ops, and commit it as a single undo step.

/// The armed source for an Insert/Overwrite (spec 16 §1): a clip source plus its
/// trim in/out and originating track kind. Duration = `src_out − src_in`. Held
/// in GUI session state only.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PendingSource {
    /// The source op to lay down (carries the asset id for asset/vector sources).
    pub source: ClipSource,
    /// Trim in-point into the source media.
    pub src_in: Tick,
    /// Trim out-point into the source media (`source_in + duration`).
    pub src_out: Tick,
    /// Display name carried from the armed clip.
    pub name: String,
    /// Which track kind the source came from → which lane the edit targets.
    pub kind: TrackKind,
}

impl PendingSource {
    /// Arm from an existing timeline `clip` on a track of `kind` (spec 16 §4
    /// minimal arming): capture its source, trim in-point and out-point so an
    /// Insert/Overwrite lays down the same source region.
    pub fn from_clip(clip: &Clip, kind: TrackKind) -> Self {
        PendingSource {
            source: clip.source.clone(),
            src_in: clip.source_in,
            src_out: clip.source_in + clip.duration,
            name: clip.name.clone(),
            kind,
        }
    }

    /// Duration of the armed range (`src_out − src_in`); ≥ 0 for a range
    /// captured via [`PendingSource::from_clip`].
    pub fn duration(&self) -> Tick {
        self.src_out - self.src_in
    }

    /// Build the concrete clip to drop at `at` on the timeline, preserving the
    /// source trim in-point and name.
    pub fn to_clip(&self, at: Tick) -> Clip {
        let mut c = Clip::new(self.source.clone(), at, self.duration());
        c.source_in = self.src_in;
        c.name = self.name.clone();
        c
    }
}

/// Arm a 3-point source from the first selected clip on `seq` (spec 16 §4
/// minimal arming — selecting a timeline clip makes it the source). Video lanes
/// are scanned before audio (timeline order); `None` when the selection resolves
/// to no clip.
pub(crate) fn pending_source_from_selection(
    seq: &Sequence,
    selection: &[ClipId],
) -> Option<PendingSource> {
    for t in seq.tracks() {
        for c in &t.clips {
            if selection.contains(&c.id) {
                return Some(PendingSource::from_clip(c, t.kind));
            }
        }
    }
    None
}

/// Resolve which track receives an edit for `kind` (spec 16 §1, M-3 minimal
/// source-patch): the `explicit` patch target when it still exists on a lane of
/// `kind`, else the first ENABLED track of that kind, else the first track of
/// that kind. `None` = no track of that kind exists.
pub(crate) fn resolve_target_track(
    seq: &Sequence,
    kind: TrackKind,
    explicit: Option<TrackId>,
) -> Option<TrackId> {
    let lane = match kind {
        TrackKind::Video => &seq.video_tracks,
        TrackKind::Audio => &seq.audio_tracks,
    };
    if let Some(id) = explicit {
        if lane.iter().any(|t| t.id == id) {
            return Some(id);
        }
    }
    lane.iter()
        .find(|t| t.enabled)
        .or_else(|| lane.first())
        .map(|t| t.id)
}

/// Commit `cmds` as ONE undo step (spec 16 §2 "batched into one undo step").
/// A single command keeps the `Command::Timeline` shape the rest of the timeline
/// relies on rather than a one-element `Batch`; an empty batch is a no-op.
fn commit_edit(history: &mut CommandHistory, doc: &mut Document, mut cmds: Vec<TimelineCmd>) {
    match cmds.len() {
        0 => {}
        1 => history.execute_discrete(
            Command::Timeline(cmds.pop().expect("len == 1 checked above")),
            doc,
        ),
        _ => {
            let batch = cmds.into_iter().map(Command::Timeline).collect();
            history.execute_discrete(Command::Batch(batch), doc);
        }
    }
}

/// **Insert** (spec 16 §2, Premiere `,`): open a gap at `at` on `track` and drop
/// `source` in, rippling downstream clips right. One undo step; `true` if
/// applied (a rejected edit — bad track, non-positive duration, nested cycle —
/// is a silent no-op like other invalid gestures).
pub(crate) fn do_insert_edit(
    doc: &mut Document,
    history: &mut CommandHistory,
    seq: SequenceId,
    track: TrackId,
    at: Tick,
    source: Clip,
) -> bool {
    let Some(p) = doc.timeline.as_ref() else {
        return false;
    };
    match ops::insert_edit(p, seq, track, at, source) {
        Ok(cmds) => {
            commit_edit(history, doc, cmds);
            true
        }
        Err(_) => false,
    }
}

/// **Overwrite** (spec 16 §2, Premiere `.`): drop `source` at `at` on `track`,
/// replacing whatever it covers with no ripple. One undo step; `true` if applied.
pub(crate) fn do_overwrite_edit(
    doc: &mut Document,
    history: &mut CommandHistory,
    seq: SequenceId,
    track: TrackId,
    at: Tick,
    source: Clip,
) -> bool {
    let Some(p) = doc.timeline.as_ref() else {
        return false;
    };
    match ops::overwrite_edit(p, seq, track, at, source) {
        Ok(cmds) => {
            commit_edit(history, doc, cmds);
            true
        }
        Err(_) => false,
    }
}

/// **Lift** (spec 16 §2, Premiere `;`): clear `range` on `track`, leaving a gap
/// (no ripple). One undo step; `true` if applied.
pub(crate) fn do_lift_edit(
    doc: &mut Document,
    history: &mut CommandHistory,
    seq: SequenceId,
    track: TrackId,
    range: (Tick, Tick),
) -> bool {
    let Some(p) = doc.timeline.as_ref() else {
        return false;
    };
    match ops::lift_edit(p, seq, track, range) {
        Ok(cmds) => {
            commit_edit(history, doc, cmds);
            true
        }
        Err(_) => false,
    }
}

/// **Extract** (spec 16 §2, Premiere `'`): clear `range` on `track` AND ripple
/// everything after it left to close the gap. One undo step; `true` if applied.
pub(crate) fn do_extract_edit(
    doc: &mut Document,
    history: &mut CommandHistory,
    seq: SequenceId,
    track: TrackId,
    range: (Tick, Tick),
) -> bool {
    let Some(p) = doc.timeline.as_ref() else {
        return false;
    };
    match ops::extract_edit(p, seq, track, range) {
        Ok(cmds) => {
            commit_edit(history, doc, cmds);
            true
        }
        Err(_) => false,
    }
}

/// Razor/blade decision (spec 16 §4, M-4): a lane click at `at` on `clip` splits
/// it only when strictly interior (`clip.start < at < clip.end()`); a click on
/// an edge is not a split. Returns the split tick to hand to `ops_bridge::split`.
///
/// The lane-click call site lives in the timeline-panel story's `self_interact`
/// (its territory); this is the pure guard it consults, kept here alongside the
/// other timeline-interaction logic. Unused until that one-line seam is wired
/// — same `dead_code` posture the panel's own command entry points used before
/// dispatch reached them.
#[allow(dead_code)]
pub(crate) fn razor_split_tick(clip: &Clip, at: Tick) -> Option<Tick> {
    (clip.start < at && at < clip.end()).then_some(at)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photonic_core::timeline::FrameRate;

    #[test]
    fn hit_zone_classifies_edges_and_body() {
        // A 100px-wide clip from 200..300, 6px edges.
        assert_eq!(hit_zone(200.0, 300.0, 202.0, 6.0), Some(ClipZone::LeftEdge));
        assert_eq!(
            hit_zone(200.0, 300.0, 298.0, 6.0),
            Some(ClipZone::RightEdge)
        );
        assert_eq!(hit_zone(200.0, 300.0, 250.0, 6.0), Some(ClipZone::Body));
        assert_eq!(hit_zone(200.0, 300.0, 199.0, 6.0), None);
        assert_eq!(hit_zone(200.0, 300.0, 301.0, 6.0), None);
    }

    #[test]
    fn hit_zone_narrow_clip_splits_at_midpoint() {
        // 8px clip (< 2*6): no body, split at midpoint.
        assert_eq!(hit_zone(100.0, 108.0, 101.0, 6.0), Some(ClipZone::LeftEdge));
        assert_eq!(
            hit_zone(100.0, 108.0, 107.0, 6.0),
            Some(ClipZone::RightEdge)
        );
        assert!(hit_zone(100.0, 108.0, 104.0, 6.0).is_some());
    }

    #[test]
    fn hit_at_locked_track_rejects_the_edit_intent() {
        let track = TrackId::new();
        let clip = ClipId::new();
        let rect = egui::Rect::from_min_max(egui::pos2(100.0, 10.0), egui::pos2(200.0, 40.0));
        let pos = egui::pos2(150.0, 25.0); // dead center of the rect.

        // Locked track: a pointer squarely over the clip is never a hit — no
        // selection, no drag-start, no context-menu target (14-nle-parity
        // QW-2 — a locked track rejects every clip edit intent).
        let locked = [HitCandidate {
            track,
            clip,
            rect,
            locked: true,
        }];
        assert_eq!(hit_at(pos, 6.0, &locked), None);

        // Identical geometry, unlocked, resolves normally — proves the miss
        // above is the lock guard, not a bug in the rect math.
        let unlocked = [HitCandidate {
            track,
            clip,
            rect,
            locked: false,
        }];
        assert_eq!(
            hit_at(pos, 6.0, &unlocked),
            Some((track, clip, rect, ClipZone::Body))
        );
    }

    #[test]
    fn hit_at_falls_through_a_locked_candidate_to_an_unlocked_one() {
        // Two overlapping candidates at the same point (e.g. adjacent lanes'
        // edge case) — the locked one must not shadow a legitimately
        // unlocked hit later in the list.
        let locked_track = TrackId::new();
        let locked_clip = ClipId::new();
        let unlocked_track = TrackId::new();
        let unlocked_clip = ClipId::new();
        let rect = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(100.0, 30.0));
        let candidates = [
            HitCandidate {
                track: locked_track,
                clip: locked_clip,
                rect,
                locked: true,
            },
            HitCandidate {
                track: unlocked_track,
                clip: unlocked_clip,
                rect,
                locked: false,
            },
        ];
        assert_eq!(
            hit_at(egui::pos2(50.0, 15.0), 6.0, &candidates),
            Some((unlocked_track, unlocked_clip, rect, ClipZone::Body))
        );
    }

    #[test]
    fn nearest_snap_within_threshold_and_priority() {
        let value = Tick(1000);
        // First within threshold wins even if a later one is closer.
        let candidates = [Tick(1050), Tick(1010)];
        assert_eq!(nearest_snap(value, &candidates, Tick(60)), Some(Tick(1050)));
        // Nothing within threshold → None.
        assert_eq!(nearest_snap(value, &candidates, Tick(5)), None);
        // Empty candidates → None.
        assert_eq!(nearest_snap(value, &[], Tick(100)), None);
    }

    #[test]
    fn snap_and_quantize_lands_on_frame_boundary() {
        let fr = FrameRate::FPS_30;
        let tpf = fr.ticks_per_frame().0;
        // No snap candidates, snapping on → pure frame quantize.
        let v = Tick(5 * tpf + 7);
        let out = snap_and_quantize(v, &[], Tick(0), true, fr);
        assert_eq!(out.0 % tpf, 0);
        assert_eq!(out, Tick(5 * tpf));
    }

    #[test]
    fn snap_candidates_exclude_moving_and_order_by_priority() {
        let mut s = Sequence::new("s", FrameRate::FPS_30, 1920, 1080);
        let mut v = photonic_core::timeline::Track::new(TrackKind::Video, "V1");
        let moving = ClipId::new();
        let mut c0 = photonic_core::timeline::Clip::new(
            photonic_core::timeline::ClipSource::Adjustment,
            Tick(0),
            Tick(100),
        );
        c0.id = moving;
        let c1 = photonic_core::timeline::Clip::new(
            photonic_core::timeline::ClipSource::Adjustment,
            Tick(100),
            Tick(50),
        );
        v.clips.push(c0);
        v.clips.push(c1);
        let track_id = v.id;
        s.video_tracks.push(v);
        let cands = build_snap_candidates(&s, track_id, moving, Tick(42));
        // Moving clip's own edges (0, 100) are excluded; c1's edges present;
        // playhead present.
        assert!(cands.contains(&Tick(100)));
        assert!(cands.contains(&Tick(150)));
        assert!(cands.contains(&Tick(42)));
        // Playhead is after the same-track edges in priority order.
        let play_idx = cands.iter().position(|t| *t == Tick(42)).unwrap();
        let edge_idx = cands.iter().position(|t| *t == Tick(150)).unwrap();
        assert!(edge_idx < play_idx);
    }

    #[test]
    fn resolve_drag_kind_body_modifiers() {
        let clip = ClipId::new();
        // Plain body ⇒ Move.
        assert_eq!(
            resolve_drag_kind(ClipZone::Body, false, false, clip, None, None),
            (DragKind::Move, clip)
        );
        // Alt+body ⇒ Slip.
        assert_eq!(
            resolve_drag_kind(ClipZone::Body, true, false, clip, None, None),
            (DragKind::Slip, clip)
        );
        // Alt+Shift+body ⇒ Slide.
        assert_eq!(
            resolve_drag_kind(ClipZone::Body, true, true, clip, None, None),
            (DragKind::Slide, clip)
        );
        // Shift alone on the body is not a slide — still Move.
        assert_eq!(
            resolve_drag_kind(ClipZone::Body, false, true, clip, None, None),
            (DragKind::Move, clip)
        );
    }

    #[test]
    fn resolve_drag_kind_left_edge() {
        let clip = ClipId::new();
        let prev = ClipId::new();
        // Flush left neighbour, no Shift ⇒ Roll with the LEFT clip as primary and
        // `clip` as the right side of the shared boundary.
        assert_eq!(
            resolve_drag_kind(ClipZone::LeftEdge, false, false, clip, Some(prev), None),
            (DragKind::Roll { right: clip }, prev)
        );
        // Shift suppresses Roll even with a flush neighbour ⇒ RippleTrimStart.
        assert_eq!(
            resolve_drag_kind(ClipZone::LeftEdge, false, true, clip, Some(prev), None),
            (DragKind::RippleTrimStart, clip)
        );
        // No flush neighbour, no Shift ⇒ plain TrimStart.
        assert_eq!(
            resolve_drag_kind(ClipZone::LeftEdge, false, false, clip, None, None),
            (DragKind::TrimStart, clip)
        );
        // No neighbour + Shift ⇒ RippleTrimStart.
        assert_eq!(
            resolve_drag_kind(ClipZone::LeftEdge, false, true, clip, None, None),
            (DragKind::RippleTrimStart, clip)
        );
    }

    #[test]
    fn resolve_drag_kind_right_edge() {
        let clip = ClipId::new();
        let next = ClipId::new();
        // Flush right neighbour, no Shift ⇒ Roll with `clip` as primary (left of
        // the boundary) and the neighbour as the right side.
        assert_eq!(
            resolve_drag_kind(ClipZone::RightEdge, false, false, clip, None, Some(next)),
            (DragKind::Roll { right: next }, clip)
        );
        // Shift suppresses Roll ⇒ RippleTrimEnd.
        assert_eq!(
            resolve_drag_kind(ClipZone::RightEdge, false, true, clip, None, Some(next)),
            (DragKind::RippleTrimEnd, clip)
        );
        // No flush neighbour, no Shift ⇒ plain TrimEnd.
        assert_eq!(
            resolve_drag_kind(ClipZone::RightEdge, false, false, clip, None, None),
            (DragKind::TrimEnd, clip)
        );
    }

    #[test]
    fn apply_marquee_intersect_not_contains() {
        let clip = ClipId::new();
        let marquee = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(50.0, 50.0));
        // Partial overlap (not fully contained) still selects — intersect, not
        // contains, is the rule.
        let partial = egui::Rect::from_min_max(egui::pos2(40.0, 40.0), egui::pos2(100.0, 100.0));
        let mut sel = Vec::new();
        apply_marquee(marquee, [(partial, clip)], false, &mut sel);
        assert_eq!(sel, vec![clip]);

        // A rect entirely outside the marquee is not selected.
        let outside = egui::Rect::from_min_max(egui::pos2(60.0, 60.0), egui::pos2(80.0, 80.0));
        let mut sel = Vec::new();
        apply_marquee(marquee, [(outside, clip)], false, &mut sel);
        assert!(sel.is_empty());
    }

    #[test]
    fn apply_marquee_replace_vs_additive() {
        let existing = ClipId::new();
        let hit = ClipId::new();
        let inside = egui::Rect::from_min_max(egui::pos2(10.0, 10.0), egui::pos2(20.0, 20.0));
        let marquee = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(50.0, 50.0));

        // Replace: prior selection is cleared, only the hit survives.
        let mut sel = vec![existing];
        apply_marquee(marquee, [(inside, hit)], false, &mut sel);
        assert_eq!(sel, vec![hit]);

        // Additive: prior selection is retained and the hit appended.
        let mut sel = vec![existing];
        apply_marquee(marquee, [(inside, hit)], true, &mut sel);
        assert_eq!(sel, vec![existing, hit]);

        // Additive never duplicates an already-selected clip.
        let mut sel = vec![hit];
        apply_marquee(marquee, [(inside, hit)], true, &mut sel);
        assert_eq!(sel, vec![hit]);
    }
}

#[cfg(test)]
mod edit_tests {
    use super::*;
    use photonic_core::timeline::{FrameRate, Sequence, TimelineProject, Track};

    fn adj_clip(start: i64, dur: i64) -> Clip {
        Clip::new(ClipSource::Adjustment, Tick(start), Tick(dur))
    }

    /// A document with one active sequence whose video/audio lanes are created
    /// empty from `(name, disabled)` specs. Returns the sequence id and the new
    /// track ids per lane.
    fn doc_with_tracks(
        video: &[(&str, bool)],
        audio: &[(&str, bool)],
    ) -> (Document, SequenceId, Vec<TrackId>, Vec<TrackId>) {
        let mut seq = Sequence::new("s", FrameRate::FPS_30, 1920, 1080);
        let mut vids = Vec::new();
        for (name, disabled) in video {
            let mut t = Track::new(TrackKind::Video, *name);
            t.enabled = !*disabled;
            vids.push(t.id);
            seq.video_tracks.push(t);
        }
        let mut auds = Vec::new();
        for (name, disabled) in audio {
            let mut t = Track::new(TrackKind::Audio, *name);
            t.enabled = !*disabled;
            auds.push(t.id);
            seq.audio_tracks.push(t);
        }
        let mut project = TimelineProject::new();
        let seq_id = project.insert_sequence(seq);
        project.active_sequence = Some(seq_id);
        let mut doc = Document::new("t", 1920.0, 1080.0);
        doc.timeline = Some(project);
        (doc, seq_id, vids, auds)
    }

    /// Push a clip directly onto a track (test setup only — no history).
    fn seed(doc: &mut Document, seq: SequenceId, track: TrackId, start: i64, dur: i64) {
        doc.timeline
            .as_mut()
            .unwrap()
            .sequences
            .get_mut(&seq)
            .unwrap()
            .track_mut(track)
            .unwrap()
            .clips
            .push(adj_clip(start, dur));
    }

    fn clips_of(doc: &Document, seq: SequenceId, track: TrackId) -> Vec<(Tick, Tick)> {
        doc.timeline.as_ref().unwrap().sequences[&seq]
            .track(track)
            .unwrap()
            .clips
            .iter()
            .map(|c| (c.start, c.duration))
            .collect()
    }

    fn content_end(doc: &Document, seq: SequenceId, track: TrackId) -> Tick {
        doc.timeline.as_ref().unwrap().sequences[&seq]
            .track(track)
            .unwrap()
            .clips
            .iter()
            .map(|c| c.end())
            .max()
            .unwrap_or(Tick::ZERO)
    }

    // ── Arming: clip → PendingSource (spec 16 §4 minimal) ────────────────────

    #[test]
    fn arming_captures_the_selected_clips_source_range_and_kind() {
        let mut seq = Sequence::new("s", FrameRate::FPS_30, 1920, 1080);
        let mut v = Track::new(TrackKind::Video, "V1");
        let mut c = adj_clip(0, 100);
        c.source_in = Tick(30); // trimmed 30 ticks into the source
        c.name = "shot".into();
        let id = c.id;
        v.clips.push(c);
        seq.video_tracks.push(v);

        let ps = pending_source_from_selection(&seq, &[id]).expect("armed");
        assert_eq!(ps.src_in, Tick(30));
        assert_eq!(ps.src_out, Tick(130)); // source_in + duration
        assert_eq!(ps.duration(), Tick(100));
        assert_eq!(ps.kind, TrackKind::Video);
        assert_eq!(ps.name, "shot");

        // The concrete clip lands at `at`, keeping the source trim + name.
        let placed = ps.to_clip(Tick(500));
        assert_eq!((placed.start, placed.duration), (Tick(500), Tick(100)));
        assert_eq!(placed.source_in, Tick(30));
        assert_eq!(placed.name, "shot");

        // An empty selection arms nothing.
        assert!(pending_source_from_selection(&seq, &[]).is_none());
    }

    // ── Target-track resolution (spec 16 §1 M-3 minimal) ─────────────────────

    #[test]
    fn target_resolution_prefers_explicit_then_first_enabled() {
        // V1 disabled, V2 enabled.
        let (doc, seq, vids, _a) = doc_with_tracks(&[("V1", true), ("V2", false)], &[]);
        let s = &doc.timeline.as_ref().unwrap().sequences[&seq];

        // No patch → first ENABLED video track (V2), not merely the first.
        assert_eq!(
            resolve_target_track(s, TrackKind::Video, None),
            Some(vids[1])
        );
        // Explicit valid patch wins even though V1 is disabled.
        assert_eq!(
            resolve_target_track(s, TrackKind::Video, Some(vids[0])),
            Some(vids[0])
        );
        // A stale/foreign patch id falls back to first enabled.
        assert_eq!(
            resolve_target_track(s, TrackKind::Video, Some(TrackId::new())),
            Some(vids[1])
        );
        // No audio lane exists → None.
        assert_eq!(resolve_target_track(s, TrackKind::Audio, None), None);
    }

    // ── The four edits (thin bridges over the tested core ops) ───────────────

    #[test]
    fn insert_lands_source_at_the_playhead_on_the_resolved_target_one_undo_step() {
        // V1 disabled so the resolved target is V2 — proves at-playhead
        // placement lands on the patch target, not lane 0.
        let (mut doc, seq, vids, _a) = doc_with_tracks(&[("V1", true), ("V2", false)], &[]);
        let mut hist = CommandHistory::new(64);
        let target = resolve_target_track(
            &doc.timeline.as_ref().unwrap().sequences[&seq],
            TrackKind::Video,
            None,
        )
        .unwrap();
        assert_eq!(target, vids[1]);

        let playhead = Tick(200);
        assert!(do_insert_edit(
            &mut doc,
            &mut hist,
            seq,
            target,
            playhead,
            adj_clip(0, 50)
        ));
        // Source landed at the playhead on V2; V1 untouched.
        assert_eq!(clips_of(&doc, seq, vids[1]), vec![(Tick(200), Tick(50))]);
        assert!(clips_of(&doc, seq, vids[0]).is_empty());

        // Exactly one undo step removes it.
        assert!(hist.undo(&mut doc));
        assert!(clips_of(&doc, seq, vids[1]).is_empty());
    }

    #[test]
    fn insert_ripples_downstream_and_grows_the_track() {
        let (mut doc, seq, vids, _a) = doc_with_tracks(&[("V1", false)], &[]);
        let v = vids[0];
        seed(&mut doc, seq, v, 0, 100); // existing clip [0,100)
        let mut hist = CommandHistory::new(64);

        // Insert dur-50 at the head → existing clip ripples right by 50.
        assert!(do_insert_edit(
            &mut doc,
            &mut hist,
            seq,
            v,
            Tick(0),
            adj_clip(0, 50)
        ));
        assert_eq!(content_end(&doc, seq, v), Tick(150)); // grew by the source duration
        assert_eq!(
            clips_of(&doc, seq, v),
            vec![(Tick(0), Tick(50)), (Tick(50), Tick(100))]
        );
    }

    #[test]
    fn overwrite_keeps_duration_and_punches_a_hole() {
        let (mut doc, seq, vids, _a) = doc_with_tracks(&[("V1", false)], &[]);
        let v = vids[0];
        seed(&mut doc, seq, v, 0, 200); // [0,200)
        let mut hist = CommandHistory::new(64);

        assert!(do_overwrite_edit(
            &mut doc,
            &mut hist,
            seq,
            v,
            Tick(50),
            adj_clip(0, 50)
        ));
        // No ripple: overall end unchanged; a dur-50 source now occupies [50,100).
        assert_eq!(content_end(&doc, seq, v), Tick(200));
        assert!(clips_of(&doc, seq, v).contains(&(Tick(50), Tick(50))));
    }

    #[test]
    fn lift_leaves_a_gap_and_extract_closes_it() {
        // Lift: clears the range, no ripple → total end unchanged.
        let (mut doc, seq, vids, _a) = doc_with_tracks(&[("V1", false)], &[]);
        let v = vids[0];
        seed(&mut doc, seq, v, 0, 300); // [0,300)
        let mut hist = CommandHistory::new(64);
        assert!(do_lift_edit(
            &mut doc,
            &mut hist,
            seq,
            v,
            (Tick(100), Tick(200))
        ));
        assert_eq!(
            clips_of(&doc, seq, v),
            vec![(Tick(0), Tick(100)), (Tick(200), Tick(100))]
        );
        assert_eq!(content_end(&doc, seq, v), Tick(300)); // gap left, duration kept

        // Extract: clears + ripples left → shrinks by the range width.
        let (mut doc, seq, vids, _a) = doc_with_tracks(&[("V1", false)], &[]);
        let v = vids[0];
        seed(&mut doc, seq, v, 0, 300);
        let mut hist = CommandHistory::new(64);
        assert!(do_extract_edit(
            &mut doc,
            &mut hist,
            seq,
            v,
            (Tick(100), Tick(200))
        ));
        assert_eq!(
            clips_of(&doc, seq, v),
            vec![(Tick(0), Tick(100)), (Tick(100), Tick(100))]
        );
        assert_eq!(content_end(&doc, seq, v), Tick(200)); // closed the 100-tick gap
    }

    // ── Razor guard (spec 16 §4 M-4) ─────────────────────────────────────────

    #[test]
    fn razor_splits_only_the_interior_never_an_edge() {
        let c = adj_clip(100, 100); // [100,200)
        assert_eq!(razor_split_tick(&c, Tick(150)), Some(Tick(150)));
        assert_eq!(razor_split_tick(&c, Tick(100)), None); // start edge
        assert_eq!(razor_split_tick(&c, Tick(200)), None); // end edge
        assert_eq!(razor_split_tick(&c, Tick(50)), None); // outside
    }
}
