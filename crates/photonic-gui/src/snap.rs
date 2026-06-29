//! Object-aware snapping for the Select-tool move drag (#66 / PR #137).
//!
//! While a node is dragged, [`collect_snap_candidates`] gathers the edges and
//! centers of every other visible, non-locked node (in canvas space), and
//! [`resolve_snap`] finds the closest alignment(s) within a pixel tolerance.
//! The dragged object is then nudged so its edge/center lands exactly on the
//! target, and the caller draws a dashed guide line at each active alignment.
//!
//! All coordinates are **canvas space**. The caller converts the screen-pixel
//! tolerance to canvas units (`tolerance_px / view.zoom`) before calling
//! [`resolve_snap`], and multiplies guide distances by `view.zoom` when drawing
//! the pixel-distance labels.

use photonic_core::document::Document;
use photonic_core::node::{NodeId, SceneNode};

/// Which kind of guide a candidate / active snap produces.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SnapAxis {
    /// A **vertical** guide line. Aligns an `x` coordinate
    /// (left edge / horizontal-center / right edge).
    Vertical,
    /// A **horizontal** guide line. Aligns a `y` coordinate
    /// (top edge / vertical-center / bottom edge).
    Horizontal,
}

/// A single alignment target contributed by one node.
#[derive(Clone, Copy, Debug)]
pub struct SnapCandidate {
    /// The node that owns this alignment line.
    pub node_id: NodeId,
    /// Whether `value` is an `x` (vertical guide) or `y` (horizontal guide).
    pub axis: SnapAxis,
    /// The canvas-space coordinate of the alignment line (e.g. left-edge x).
    pub value: f64,
    /// Extent of the owning node along the **perpendicular** axis, used to
    /// compute the object-to-object gap shown in the distance label. For a
    /// vertical guide this is the node's `[y0, y1]`; for a horizontal guide it
    /// is `[x0, x1]`.
    pub perp_min: f64,
    pub perp_max: f64,
}

/// One alignment that fired this frame — enough to draw a guide line + label.
#[derive(Clone, Copy, Debug)]
pub struct ActiveGuide {
    /// Orientation of the guide line.
    pub axis: SnapAxis,
    /// Canvas-space coordinate the line is drawn at (matches the target edge).
    pub coord: f64,
    /// The node we snapped to (for debugging / future labels).
    pub target_node: NodeId,
    /// Gap between the dragged object and the target object along the
    /// perpendicular axis, in **canvas units** (0 when they overlap). The
    /// caller multiplies by `view.zoom` to render a pixel-distance label.
    pub distance: f64,
}

/// Result of [`resolve_snap`].
#[derive(Clone, Debug, Default)]
pub struct SnapResult {
    /// The correction `(dx, dy)` to add to the dragged object's tentative
    /// position so its edge/center aligns with the snapped target(s). `(0, 0)`
    /// when nothing is within tolerance.
    pub corrected: (f64, f64),
    /// Active alignments this frame — at most one per axis (so at most two).
    pub active: Vec<ActiveGuide>,
}

/// Canvas-space axis-aligned bounding box of a node, `(x0, y0, x1, y1)`.
///
/// Uses the node's local bounds projected through its transform. Returns `None`
/// for nodes without computable bounds (e.g. empty groups).
fn world_aabb(node: &SceneNode) -> Option<(f64, f64, f64, f64)> {
    let local = node.local_bounds()?;
    let corners = [
        node.transform.apply(local.x0, local.y0),
        node.transform.apply(local.x1, local.y0),
        node.transform.apply(local.x0, local.y1),
        node.transform.apply(local.x1, local.y1),
    ];
    let x0 = corners.iter().map(|&(x, _)| x).fold(f64::INFINITY, f64::min);
    let y0 = corners.iter().map(|&(_, y)| y).fold(f64::INFINITY, f64::min);
    let x1 = corners
        .iter()
        .map(|&(x, _)| x)
        .fold(f64::NEG_INFINITY, f64::max);
    let y1 = corners
        .iter()
        .map(|&(_, y)| y)
        .fold(f64::NEG_INFINITY, f64::max);
    if x0.is_finite() && y0.is_finite() {
        Some((x0, y0, x1, y1))
    } else {
        None
    }
}

/// Collect snap candidates (edges + centers) for every visible, non-locked node
/// except those in `exclude` (typically the nodes currently being dragged).
///
/// Each node contributes three vertical candidates (left, center-x, right) and
/// three horizontal candidates (top, center-y, bottom).
pub fn collect_snap_candidates(doc: &Document, exclude: &[NodeId]) -> Vec<SnapCandidate> {
    let mut out = Vec::new();
    for node in doc.nodes_in_draw_order() {
        if node.locked || exclude.contains(&node.id) {
            continue;
        }
        let Some((x0, y0, x1, y1)) = world_aabb(node) else {
            continue;
        };
        let cx = (x0 + x1) / 2.0;
        let cy = (y0 + y1) / 2.0;
        // Vertical guides snap an x coordinate; their perpendicular span is y.
        for value in [x0, cx, x1] {
            out.push(SnapCandidate {
                node_id: node.id,
                axis: SnapAxis::Vertical,
                value,
                perp_min: y0,
                perp_max: y1,
            });
        }
        // Horizontal guides snap a y coordinate; their perpendicular span is x.
        for value in [y0, cy, y1] {
            out.push(SnapCandidate {
                node_id: node.id,
                axis: SnapAxis::Horizontal,
                value,
                perp_min: x0,
                perp_max: x1,
            });
        }
    }
    out
}

/// Gap between two closed intervals, `0.0` when they overlap or touch.
fn interval_gap(a0: f64, a1: f64, b0: f64, b1: f64) -> f64 {
    if a1 < b0 {
        b0 - a1
    } else if b1 < a0 {
        a0 - b1
    } else {
        0.0
    }
}

/// Resolve the best snap for a dragged object whose tentative (pre-snap)
/// bounding box is `moving_bbox = (x0, y0, x1, y1)`.
///
/// For each axis independently, the candidate edge whose `value` is closest to
/// one of the object's three edges (min / center / max) **and** within
/// `tolerance` wins. The returned `corrected` is the `(dx, dy)` nudge that makes
/// those edges coincide; `active` holds one [`ActiveGuide`] per snapped axis.
pub fn resolve_snap(
    moving_bbox: (f64, f64, f64, f64),
    candidates: &[SnapCandidate],
    tolerance: f64,
) -> SnapResult {
    let (mx0, my0, mx1, my1) = moving_bbox;
    let mcx = (mx0 + mx1) / 2.0;
    let mcy = (my0 + my1) / 2.0;

    // Best snap per axis: (signed shift to apply, the winning candidate).
    let mut best_v: Option<(f64, SnapCandidate)> = None;
    let mut best_h: Option<(f64, SnapCandidate)> = None;

    for cand in candidates {
        let edges = match cand.axis {
            SnapAxis::Vertical => [mx0, mcx, mx1],
            SnapAxis::Horizontal => [my0, mcy, my1],
        };
        for edge in edges {
            let shift = cand.value - edge; // move object by `shift` to align
            if shift.abs() > tolerance {
                continue;
            }
            let slot = match cand.axis {
                SnapAxis::Vertical => &mut best_v,
                SnapAxis::Horizontal => &mut best_h,
            };
            let better = match slot {
                Some((cur, _)) => shift.abs() < cur.abs(),
                None => true,
            };
            if better {
                *slot = Some((shift, *cand));
            }
        }
    }

    let mut result = SnapResult::default();
    let (dx, dy) = (
        best_v.map(|(s, _)| s).unwrap_or(0.0),
        best_h.map(|(s, _)| s).unwrap_or(0.0),
    );
    result.corrected = (dx, dy);

    // Bounding box after the correction, for honest gap measurement.
    let (sx0, sy0, sx1, sy1) = (mx0 + dx, my0 + dy, mx1 + dx, my1 + dy);

    if let Some((_, cand)) = best_v {
        result.active.push(ActiveGuide {
            axis: SnapAxis::Vertical,
            coord: cand.value,
            target_node: cand.node_id,
            distance: interval_gap(sy0, sy1, cand.perp_min, cand.perp_max),
        });
    }
    if let Some((_, cand)) = best_h {
        result.active.push(ActiveGuide {
            axis: SnapAxis::Horizontal,
            coord: cand.value,
            target_node: cand.node_id,
            distance: interval_gap(sx0, sx1, cand.perp_min, cand.perp_max),
        });
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn cand(axis: SnapAxis, value: f64) -> SnapCandidate {
        SnapCandidate {
            node_id: Uuid::nil(),
            axis,
            value,
            perp_min: 0.0,
            perp_max: 10.0,
        }
    }

    #[test]
    fn snaps_left_edge_within_tolerance() {
        // Dragged box left edge at 102; a candidate vertical line at 100.
        let cands = [cand(SnapAxis::Vertical, 100.0)];
        let res = resolve_snap((102.0, 50.0, 142.0, 90.0), &cands, 6.0);
        assert_eq!(res.corrected.0, -2.0); // shift left by 2 to align at x=100
        assert_eq!(res.corrected.1, 0.0);
        assert_eq!(res.active.len(), 1);
        assert_eq!(res.active[0].axis, SnapAxis::Vertical);
        assert_eq!(res.active[0].coord, 100.0);
    }

    #[test]
    fn no_snap_outside_tolerance() {
        let cands = [cand(SnapAxis::Vertical, 100.0)];
        let res = resolve_snap((120.0, 50.0, 160.0, 90.0), &cands, 6.0);
        assert_eq!(res.corrected, (0.0, 0.0));
        assert!(res.active.is_empty());
    }

    #[test]
    fn snaps_center_when_closer_than_edge() {
        // Box spans x in [90, 110] -> center 100. Candidate at 101 should snap
        // the center (shift +1) rather than the far edges.
        let cands = [cand(SnapAxis::Vertical, 101.0)];
        let res = resolve_snap((90.0, 0.0, 110.0, 20.0), &cands, 6.0);
        assert_eq!(res.corrected.0, 1.0);
    }

    #[test]
    fn snaps_both_axes_independently() {
        let cands = [
            cand(SnapAxis::Vertical, 200.0),
            cand(SnapAxis::Horizontal, 300.0),
        ];
        // left edge 203 -> -3 ; top edge 298 -> +2
        let res = resolve_snap((203.0, 298.0, 250.0, 340.0), &cands, 6.0);
        assert_eq!(res.corrected, (-3.0, 2.0));
        assert_eq!(res.active.len(), 2);
    }

    #[test]
    fn picks_closest_candidate_on_axis() {
        let cands = [
            cand(SnapAxis::Vertical, 100.0),
            cand(SnapAxis::Vertical, 104.0),
        ];
        // left edge at 103: 104 is closer (+1) than 100 (-3).
        let res = resolve_snap((103.0, 0.0, 140.0, 20.0), &cands, 6.0);
        assert_eq!(res.corrected.0, 1.0);
        assert_eq!(res.active[0].coord, 104.0);
    }

    #[test]
    fn distance_label_reports_object_gap() {
        // Vertical snap; target perp span [0,10], dragged box y in [40,60]
        // => vertical gap of 30 between the two objects.
        let cands = [cand(SnapAxis::Vertical, 100.0)];
        let res = resolve_snap((100.0, 40.0, 140.0, 60.0), &cands, 6.0);
        assert_eq!(res.active.len(), 1);
        assert_eq!(res.active[0].distance, 30.0);
    }
}
