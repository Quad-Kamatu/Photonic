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
}
