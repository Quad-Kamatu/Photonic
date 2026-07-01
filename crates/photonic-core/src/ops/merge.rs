/// Merge vertices by distance (weld).
///
/// Collapses anchor points that lie spatially near/coincident into a single
/// anchor, driven by a distance `threshold`. This is cleanup tooling for the
/// aftermath of boolean ops, imports, or hand-drawing — distinct in intent from
/// [`simplify_path`](super::simplify::simplify_path), which curve-fits to
/// preserve shape via Ramer-Douglas-Peucker.
///
/// Like `simplify_path`, Bézier segments are flattened to their on-curve anchor
/// endpoints (curves collapse to straight line segments); handle reconstruction
/// is out of scope.
use crate::path::PathData;
use kurbo::{BezPath, PathEl, Point};

// ── Internal helpers ───────────────────────────────────────────────────────────

/// Greedily weld a subpath's anchor points: absorb each successive anchor into a
/// running cluster while it lies within `threshold` of the cluster centroid
/// (running mean), emitting one centroid per cluster. For closed subpaths the
/// final cluster is welded into the first when within `threshold` (wrap-around),
/// and degenerate zero-length segments are dropped.
fn weld_subpath(pts: &[Point], is_closed: bool, threshold: f64, out: &mut BezPath) {
    if pts.len() < 2 {
        // Nothing to weld — a lone anchor cannot form a segment; drop it.
        return;
    }

    let thr_sq = threshold * threshold;

    // ── Greedy running-centroid clustering ──────────────────────────────────
    let mut clusters: Vec<Point> = Vec::new();
    let mut centroid = pts[0];
    let mut count = 1u32;
    for &p in &pts[1..] {
        let dx = p.x - centroid.x;
        let dy = p.y - centroid.y;
        if dx * dx + dy * dy <= thr_sq {
            // Absorb into the running cluster (incremental mean).
            let n = count as f64;
            centroid = Point::new(
                (centroid.x * n + p.x) / (n + 1.0),
                (centroid.y * n + p.y) / (n + 1.0),
            );
            count += 1;
        } else {
            clusters.push(centroid);
            centroid = p;
            count = 1;
        }
    }
    clusters.push(centroid);

    // ── Wrap-around weld for closed subpaths ────────────────────────────────
    if is_closed && clusters.len() >= 2 {
        let first = clusters[0];
        let last = *clusters.last().unwrap();
        let dx = last.x - first.x;
        let dy = last.y - first.y;
        if dx * dx + dy * dy <= thr_sq {
            // Merge the last cluster into the first (average of the two
            // centroids) and drop the trailing cluster.
            let merged = Point::new((first.x + last.x) / 2.0, (first.y + last.y) / 2.0);
            clusters[0] = merged;
            clusters.pop();
        }
    }

    // ── Drop consecutive duplicate anchors (degenerate zero-length segs) ────
    clusters.dedup_by(|a, b| (a.x - b.x).abs() < 1e-9 && (a.y - b.y).abs() < 1e-9);

    if clusters.len() < 2 {
        // Subpath collapsed below a drawable segment — drop it.
        return;
    }

    out.move_to((clusters[0].x, clusters[0].y));
    for c in &clusters[1..] {
        out.line_to((c.x, c.y));
    }
    if is_closed {
        out.close_path();
    }
}

// ── Public API ──────────────────────────────────────────────────────────────

/// Weld anchor points within `threshold` distance of each other into single
/// anchors, returning a new `PathData`.
///
/// Each sub-path is walked from `path.to_bez_path()`; Bézier segments are
/// flattened to their on-curve endpoints, then anchors are greedily clustered by
/// a running centroid and one centroid is emitted per cluster. Closed subpaths
/// additionally weld their last cluster into the first (wrap-around). Subpaths
/// that collapse below two distinct points are dropped.
///
/// `threshold <= 0` returns `path.clone()` unchanged.
pub fn merge_vertices_by_distance(path: &PathData, threshold: f64) -> PathData {
    if threshold <= 0.0 {
        return path.clone();
    }

    let bez = path.to_bez_path();
    let mut result = BezPath::new();
    let mut current: Vec<Point> = Vec::new();
    let mut closed = false;

    for el in bez.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                weld_subpath(&current, closed, threshold, &mut result);
                current.clear();
                closed = false;
                current.push(p);
            }
            PathEl::LineTo(p) => {
                current.push(p);
            }
            // Curves collapse to their on-curve endpoint (consistent with
            // simplify.rs): handles are not reconstructed after welding.
            PathEl::CurveTo(_, _, p) | PathEl::QuadTo(_, p) => {
                current.push(p);
            }
            PathEl::ClosePath => {
                weld_subpath(&current, true, threshold, &mut result);
                current.clear();
                closed = false;
            }
        }
    }
    weld_subpath(&current, false, threshold, &mut result);

    PathData::from_bez_path(&result)
}

#[cfg(test)]
mod tests {
    use super::super::simplify::count_points;
    use super::*;

    fn open_path(pts: &[(f64, f64)]) -> PathData {
        let mut b = BezPath::new();
        b.move_to((pts[0].0, pts[0].1));
        for p in &pts[1..] {
            b.line_to((p.0, p.1));
        }
        PathData::from_bez_path(&b)
    }

    #[test]
    fn threshold_zero_returns_clone() {
        let p = open_path(&[(0.0, 0.0), (0.1, 0.0), (10.0, 0.0)]);
        let out = merge_vertices_by_distance(&p, 0.0);
        assert_eq!(count_points(&out), count_points(&p));
    }

    #[test]
    fn welds_near_coincident_anchors() {
        // Two clusters: {(0,0),(0.5,0)} and {(10,0),(10.4,0)} weld at thr=1.
        let p = open_path(&[(0.0, 0.0), (0.5, 0.0), (10.0, 0.0), (10.4, 0.0)]);
        let out = merge_vertices_by_distance(&p, 1.0);
        assert_eq!(count_points(&out), 2);
    }

    #[test]
    fn far_anchors_are_preserved() {
        let p = open_path(&[(0.0, 0.0), (10.0, 0.0), (20.0, 0.0)]);
        let out = merge_vertices_by_distance(&p, 1.0);
        assert_eq!(count_points(&out), 3);
    }

    #[test]
    fn closed_wrap_around_welds_last_into_first() {
        // Square whose closing corner sits atop the start; wrap-around weld.
        let mut b = BezPath::new();
        b.move_to((0.0, 0.0));
        b.line_to((10.0, 0.0));
        b.line_to((10.0, 10.0));
        b.line_to((0.0, 10.0));
        b.line_to((0.3, 0.3)); // near the start anchor
        b.close_path();
        let p = PathData::from_bez_path(&b);
        let out = merge_vertices_by_distance(&p, 1.0);
        // Start anchor + 3 corners = 4 distinct anchors after wrap weld.
        assert_eq!(count_points(&out), 4);
    }

    #[test]
    fn degenerate_subpath_is_dropped() {
        // All points within threshold collapse to one → cannot form a segment.
        let p = open_path(&[(0.0, 0.0), (0.2, 0.0), (0.1, 0.1)]);
        let out = merge_vertices_by_distance(&p, 5.0);
        assert_eq!(count_points(&out), 0);
    }
}
