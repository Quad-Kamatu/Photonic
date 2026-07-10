//! Pointer-intent computation for the timeline (04 §2.3–§2.5): edge-zone hit
//! testing, snapping-candidate selection, and the transient drag/marquee state.
//!
//! Everything here is pure or session-transient — no `doc.timeline` mutation and
//! no history calls (that is `ops_bridge.rs`'s job). `mod.rs` calls these to turn
//! raw pointer positions into an intent, then hands the intent to `ops_bridge`.

use photonic_core::timeline::{ClipId, ClipTiming, Sequence, Tick, TrackId};

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

#[cfg(test)]
mod tests {
    use super::*;
    use photonic_core::timeline::{FrameRate, TrackKind};

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
