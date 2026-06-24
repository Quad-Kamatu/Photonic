/// Path simplification using the Ramer-Douglas-Peucker algorithm.
///
/// Bézier curves are first sampled to polyline segments (8 pts/cubic,
/// 6 pts/quadratic), then `geo::Simplify` reduces the point count.
/// The result is a polygonal approximation with fewer vertices.
use crate::path::PathData;
use geo::{Coord, LineString, Simplify};
use kurbo::{BezPath, CubicBez, ParamCurve, PathEl, QuadBez};

// ── Internal helper ───────────────────────────────────────────────────────────

fn flush_subpath(pts: &[Coord<f64>], is_closed: bool, out: &mut BezPath, tolerance: f64) {
    if pts.len() < 2 {
        return;
    }
    let ls = LineString::new(pts.to_vec());
    let simplified = ls.simplify(&tolerance);
    let coords: Vec<&Coord<f64>> = simplified.coords().collect();
    if coords.is_empty() {
        return;
    }
    out.move_to((coords[0].x, coords[0].y));
    for c in &coords[1..] {
        out.line_to((c.x, c.y));
    }
    if is_closed {
        out.close_path();
    }
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Simplify a path using Ramer-Douglas-Peucker.
///
/// Each sub-path's Bézier curves are sampled to line segments, then
/// `geo::Simplify` removes points within `tolerance` of the simplified line.
/// Returns a new `PathData` whose curves are all straight line segments.
pub fn simplify_path(path: &PathData, tolerance: f64) -> PathData {
    let bez = path.to_bez_path();
    let mut result = BezPath::new();
    let mut current: Vec<Coord<f64>> = Vec::new();
    let mut closed = false;

    for el in bez.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                flush_subpath(&current, closed, &mut result, tolerance);
                current.clear();
                closed = false;
                current.push(Coord { x: p.x, y: p.y });
            }
            PathEl::LineTo(p) => {
                current.push(Coord { x: p.x, y: p.y });
            }
            PathEl::CurveTo(c1, c2, p) => {
                if let Some(&last) = current.last() {
                    let seg = CubicBez::new(kurbo::Point::new(last.x, last.y), c1, c2, p);
                    for i in 1..=8 {
                        let pt = seg.eval(i as f64 / 8.0);
                        current.push(Coord { x: pt.x, y: pt.y });
                    }
                }
            }
            PathEl::QuadTo(c, p) => {
                if let Some(&last) = current.last() {
                    let seg = QuadBez::new(kurbo::Point::new(last.x, last.y), c, p);
                    for i in 1..=6 {
                        let pt = seg.eval(i as f64 / 6.0);
                        current.push(Coord { x: pt.x, y: pt.y });
                    }
                }
            }
            PathEl::ClosePath => {
                flush_subpath(&current, true, &mut result, tolerance);
                current.clear();
                closed = false;
            }
        }
    }
    flush_subpath(&current, false, &mut result, tolerance);
    PathData::from_bez_path(&result)
}

/// Count the number of anchor points (non-ClosePath elements) in a path.
pub fn count_points(path: &PathData) -> usize {
    path.to_bez_path()
        .elements()
        .iter()
        .filter(|el| !matches!(el, PathEl::ClosePath))
        .count()
}
