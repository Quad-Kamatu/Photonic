//! Proportional Move — the falloff math behind the Direct Select sub-variant.
//!
//! Blender-style proportional editing for vector anchor points: dragging one (or
//! a set of) "primary" anchor drags every *other* anchor in the same path by a
//! fraction of the same delta, where the fraction is a falloff of the anchor's
//! distance to the nearest primary. This module is pure — no UI, no app state —
//! so the weighting and the weighted path rewrite are unit-testable in isolation.
//!
//! Two independent, live-adjustable scales drive the falloff:
//! - **spread** — the radius `R` (canvas/local units) beyond which influence is 0;
//! - **curve** — the exponent `k` shaping the ramp from 1 at the primary to 0 at `R`.
#![allow(dead_code)]
use super::*;
use std::collections::HashMap;

/// How the distance used by the falloff is measured. Only [`Euclidean`] ships in
/// v1; [`Connected`] (arc-length along the path, Blender's "Connected" mode) is a
/// reserved seam so the toggle can be added without touching call sites.
///
/// [`Euclidean`]: DistanceMetric::Euclidean
/// [`Connected`]: DistanceMetric::Connected
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DistanceMetric {
    /// Straight-line distance in local path space from the nearest primary anchor.
    #[default]
    Euclidean,
    /// Distance measured along the path (not yet implemented — falls back to
    /// [`Euclidean`](DistanceMetric::Euclidean)).
    Connected,
}

/// Falloff weight in `[0, 1]` for a normalized distance `t = d / R`.
///
/// `f(0) = 1` (primary moves fully), `f(t>=1) = 0` (outside the radius nothing
/// moves), monotonically non-increasing in between. The curve is `(1 - t)^k`:
/// `k = 1` is linear, `k > 1` concentrates influence near the primary (sharp),
/// `k < 1` broadens it into a plateau (soft). `k` is the user's "falloff curve
/// scale". `k` is clamped to a sane positive range so scroll can't invert it.
pub fn falloff(t: f64, k: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    let k = k.clamp(MIN_CURVE, MAX_CURVE);
    (1.0 - t).powf(k)
}

/// Bounds for the spread radius (local units) as adjusted by scroll.
pub const MIN_SPREAD: f64 = 1.0;
pub const MAX_SPREAD: f64 = 100_000.0;
/// Bounds for the falloff curve exponent as adjusted by Shift+scroll.
pub const MIN_CURVE: f64 = 0.1;
pub const MAX_CURVE: f64 = 8.0;
/// Sensible starting values for a fresh tool session.
pub const DEFAULT_SPREAD: f64 = 120.0;
pub const DEFAULT_CURVE: f64 = 2.0;

/// Per-anchor falloff weights for a proportional move.
///
/// `primary` holds the element indices of the anchor(s) being dragged directly
/// (each gets weight `1.0`). Every other anchor of `bez` gets `falloff(d / R, k)`
/// where `d` is its distance (per `metric`) to the *nearest* primary anchor;
/// anchors at or beyond `R` are omitted from the map (weight 0 → untouched).
///
/// Keyed by element index, matching [`path_anchor_points`] and the ownership
/// convention in [`bez_move_anchors_weighted`].
pub fn compute_weights(
    bez: &BezPath,
    primary: &[usize],
    radius: f64,
    curve: f64,
    metric: DistanceMetric,
) -> HashMap<usize, f64> {
    // Connected mode is not implemented yet — behave as Euclidean.
    let _ = metric;
    let anchors = path_anchor_points(bez);
    let primary_pts: Vec<Point> = anchors
        .iter()
        .filter(|(i, _)| primary.contains(i))
        .map(|(_, p)| *p)
        .collect();

    let mut weights = HashMap::new();
    let radius = radius.max(f64::MIN_POSITIVE);
    for (idx, p) in &anchors {
        if primary.contains(idx) {
            weights.insert(*idx, 1.0);
            continue;
        }
        // Nearest primary distance (Euclidean in local space).
        let Some(d) = primary_pts
            .iter()
            .map(|q| ((p.x - q.x).powi(2) + (p.y - q.y).powi(2)).sqrt())
            .fold(None, |acc: Option<f64>, d| Some(acc.map_or(d, |a| a.min(d))))
        else {
            continue; // no primaries → nothing to weight against
        };
        let t = d / radius;
        if t >= 1.0 {
            continue;
        }
        let w = falloff(t, curve);
        if w > 0.0 {
            weights.insert(*idx, w);
        }
    }
    weights
}

/// Move a `BezPath`'s anchors by `w * (dx, dy)`, where `w` is each anchor's
/// weight from [`compute_weights`]. The weighted generalization of
/// [`bez_move_anchors`]: booleans become continuous weights, keeping the same
/// handle-ownership rules so a weighted anchor drags its own handles by the same
/// fraction and the path stays smooth.
///
/// For each element `j`:
/// - endpoint `p` and incoming handle `c2` belong to anchor `j` → shifted by
///   `w[j]`;
/// - outgoing handle `c1` belongs to the *previous* anchor `j-1` → shifted by
///   `w[j-1]` (skipped when `j-1` is a `ClosePath`, which owns no anchor);
/// - a `QuadTo`'s single control is shared, so it takes the larger of the two
///   incident weights (mirroring the boolean `OR` in [`bez_move_anchors`]).
///
/// Anchors absent from `weights` have weight 0 and are left untouched. Applying
/// weights of `1.0` to a set of adjacent anchors reproduces a rigid translation,
/// so this is a strict superset of the plain move.
pub fn bez_move_anchors_weighted(
    bez: &BezPath,
    weights: &HashMap<usize, f64>,
    dx: f64,
    dy: f64,
) -> BezPath {
    let els: Vec<PathEl> = bez.elements().iter().copied().collect();
    let w_of = |j: usize| weights.get(&j).copied().unwrap_or(0.0);
    let shift = |p: Point, w: f64| Point::new(p.x + w * dx, p.y + w * dy);

    let mut result = BezPath::new();
    for (j, el) in els.iter().enumerate() {
        let w_anchor = w_of(j);
        // Previous anchor owns this element's outgoing handle `c1`, unless the
        // previous element is a `ClosePath` (no anchor there).
        let w_prev = if j > 0 && !matches!(els[j - 1], PathEl::ClosePath) {
            w_of(j - 1)
        } else {
            0.0
        };
        let new_el = match *el {
            PathEl::MoveTo(p) => PathEl::MoveTo(shift(p, w_anchor)),
            PathEl::LineTo(p) => PathEl::LineTo(shift(p, w_anchor)),
            PathEl::CurveTo(c1, c2, p) => {
                PathEl::CurveTo(shift(c1, w_prev), shift(c2, w_anchor), shift(p, w_anchor))
            }
            PathEl::QuadTo(c, p) => {
                PathEl::QuadTo(shift(c, w_anchor.max(w_prev)), shift(p, w_anchor))
            }
            PathEl::ClosePath => PathEl::ClosePath,
        };
        result.push(new_el);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn falloff_endpoints_and_monotonic() {
        // f(0) = 1, f(>=1) = 0 for any curve.
        for &k in &[0.1, 0.5, 1.0, 2.0, 8.0] {
            assert!(approx(falloff(0.0, k), 1.0), "f(0) must be 1 (k={k})");
            assert!(approx(falloff(1.0, k), 0.0), "f(1) must be 0 (k={k})");
            assert!(approx(falloff(1.5, k), 0.0), "beyond radius clamps to 0");
        }
        // Monotonically non-increasing across the radius.
        let mut prev = f64::INFINITY;
        for i in 0..=20 {
            let w = falloff(i as f64 / 20.0, 2.0);
            assert!(w <= prev + 1e-12, "falloff must not increase");
            prev = w;
        }
    }

    #[test]
    fn curve_exponent_sharpens_and_broadens() {
        // At half radius: sharp (k>1) < linear (k=1) < soft (k<1).
        let sharp = falloff(0.5, 3.0);
        let linear = falloff(0.5, 1.0);
        let soft = falloff(0.5, 0.3);
        assert!(approx(linear, 0.5), "linear at half radius is 0.5");
        assert!(sharp < linear, "higher k concentrates influence near primary");
        assert!(soft > linear, "lower k broadens influence");
    }

    /// Build `M (0,0) L (10,0) L (20,0)` — three colinear anchors 10 units apart.
    fn three_point_line() -> BezPath {
        let mut b = BezPath::new();
        b.move_to((0.0, 0.0));
        b.line_to((10.0, 0.0));
        b.line_to((20.0, 0.0));
        b
    }

    #[test]
    fn weight_one_is_full_move_weight_zero_untouched() {
        let bez = three_point_line();
        // Primary = anchor at index 0; radius smaller than the gap so the others
        // are outside → weight 0.
        let w = compute_weights(&bez, &[0], 5.0, 2.0, DistanceMetric::Euclidean);
        assert!(approx(w[&0], 1.0), "primary weight is 1");
        assert!(!w.contains_key(&1), "anchor beyond radius is omitted");
        let moved = bez_move_anchors_weighted(&bez, &w, 3.0, 7.0);
        let pts = path_anchor_points(&moved);
        assert!(approx(pts[0].1.x, 3.0) && approx(pts[0].1.y, 7.0), "primary moved fully");
        assert!(approx(pts[1].1.x, 10.0) && approx(pts[1].1.y, 0.0), "far anchor untouched");
    }

    #[test]
    fn neighbor_moves_by_its_weight() {
        let bez = three_point_line();
        // Radius 20 reaches index 1 (d=10, t=0.5) and index 2 (d=20, t=1.0 → out).
        let w = compute_weights(&bez, &[0], 20.0, 1.0, DistanceMetric::Euclidean);
        assert!(approx(w[&0], 1.0));
        assert!(approx(w[&1], 0.5), "linear falloff at half radius");
        assert!(!w.contains_key(&2), "anchor exactly at radius is excluded");
        let moved = bez_move_anchors_weighted(&bez, &w, 4.0, 0.0);
        let pts = path_anchor_points(&moved);
        assert!(approx(pts[0].1.x, 4.0), "primary +4");
        assert!(approx(pts[1].1.x, 12.0), "neighbor +2 (0.5 * 4)");
        assert!(approx(pts[2].1.x, 20.0), "outside anchor unchanged");
    }

    #[test]
    fn rigid_translation_when_all_weights_one() {
        // All-ones weights must reproduce a plain rigid translation of every anchor.
        let bez = three_point_line();
        let mut w = HashMap::new();
        for (i, _) in path_anchor_points(&bez) {
            w.insert(i, 1.0);
        }
        let moved = bez_move_anchors_weighted(&bez, &w, 5.0, -3.0);
        let plain = bez_move_anchors(&bez, &[0, 1, 2], 5.0, -3.0);
        assert_eq!(moved.to_svg(), plain.to_svg(), "weighted all-ones == rigid move");
    }

    #[test]
    fn curve_handle_scales_with_its_owning_anchor() {
        // M(0,0) C c1(2,2) c2(8,2) p(10,0): the OUT handle c1 belongs to anchor 0,
        // the IN handle c2 and endpoint belong to anchor 1.
        let mut bez = BezPath::new();
        bez.move_to((0.0, 0.0));
        bez.curve_to((2.0, 2.0), (8.0, 2.0), (10.0, 0.0));
        let mut w = HashMap::new();
        w.insert(0usize, 1.0); // anchor 0 full
        w.insert(1usize, 0.5); // anchor 1 half
        let moved = bez_move_anchors_weighted(&bez, &w, 10.0, 0.0);
        let els: Vec<PathEl> = moved.elements().to_vec();
        match els[1] {
            PathEl::CurveTo(c1, c2, p) => {
                assert!(approx(c1.x, 12.0), "c1 (owned by anchor 0) moves +10");
                assert!(approx(c2.x, 13.0), "c2 (owned by anchor 1) moves +5");
                assert!(approx(p.x, 15.0), "endpoint (anchor 1) moves +5");
            }
            _ => panic!("expected CurveTo"),
        }
    }
}
