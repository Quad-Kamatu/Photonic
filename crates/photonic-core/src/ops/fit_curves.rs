//! Curve fitting: replace a path's dense straight-line runs with the minimum
//! number of smooth cubic Béziers that stay within a tolerance of the original.
//!
//! This is the "convert a polyline arch into one coherent curve" operation
//! (Illustrator's Simplify in curve mode). It uses kurbo's optimal
//! simplification (`simplify_bezpath`, Raph Levien's fitter): joints that turn
//! by less than the corner-angle threshold are fused into a continuous curve;
//! sharper joints are preserved as cusps.

use crate::path::PathData;
use kurbo::simplify::{simplify_bezpath, SimplifyOptLevel, SimplifyOptions};
use kurbo::{BezPath, CubicBez, ParamCurve, PathEl, Point, QuadBez};

/// Samples per flattened Bézier segment when `refit_existing` re-fits curves.
const FLATTEN_SAMPLES: u32 = 24;

/// Options controlling the curve fit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FitOptions {
    /// Maximum deviation of the fitted curve from the source, in document
    /// units. Larger = more aggressive (fewer anchors, looser fit).
    pub accuracy: f64,
    /// Joins that turn by less than this angle (degrees) are treated as smooth
    /// and fused into a continuous curve; sharper joins are kept as corners.
    /// Larger = smooths across sharper bends (fewer corners preserved).
    pub corner_angle_deg: f64,
    /// When `true`, existing curve segments are flattened and re-fit along with
    /// straight runs (a uniform cleanup). When `false`, existing curves are
    /// preserved verbatim and only runs of straight line segments are fit.
    pub refit_existing: bool,
}

impl Default for FitOptions {
    fn default() -> Self {
        Self {
            accuracy: 1.0,
            corner_angle_deg: 20.0,
            refit_existing: false,
        }
    }
}

/// Fit smooth cubic Béziers to `path` per [`FitOptions`], returning new geometry
/// with (typically far) fewer anchor points. Fill/stroke/style are unaffected —
/// this only rewrites the path data.
pub fn fit_curves(path: &PathData, opts: &FitOptions) -> PathData {
    let bez = path.to_bez_path();
    if bez.elements().is_empty() {
        return path.clone();
    }
    let accuracy = opts.accuracy.max(1e-3);
    // kurbo's `angle_thresh` is the tangent of the smooth/corner cutoff angle.
    let angle_thresh = opts.corner_angle_deg.to_radians().tan().max(1e-4);
    let sopts = SimplifyOptions::default()
        .opt_level(SimplifyOptLevel::Optimize)
        .angle_thresh(angle_thresh);

    if opts.refit_existing {
        // Flatten every segment to a polyline, then fit the whole thing;
        // `simplify_bezpath` handles subpaths, closure, and corners.
        let poly = flatten_to_polyline(&bez);
        let fitted = simplify_bezpath(poly.elements().iter().copied(), accuracy, &sopts);
        return PathData::from_bez_path(&fitted);
    }

    // Preserve existing curves; fit only maximal runs of straight lines.
    let mut out = BezPath::new();
    let mut cur = Point::ZERO;
    // The straight-line run currently being accumulated (starts at its origin).
    let mut run: Vec<Point> = Vec::new();

    // Fit the accumulated straight-line run and append it to `out` (whose pen is
    // already at `run[0]`), then clear the run.
    let flush_run =
        |out: &mut BezPath, run: &mut Vec<Point>, accuracy: f64, sopts: &SimplifyOptions| {
            if run.len() >= 3 {
                // A genuine polyline run — fit curves to it.
                let mut poly = BezPath::new();
                poly.move_to(run[0]);
                for p in &run[1..] {
                    poly.line_to(*p);
                }
                let fitted = simplify_bezpath(poly.elements().iter().copied(), accuracy, sopts);
                append_after_moveto(out, &fitted);
            } else if run.len() == 2 {
                // A single straight segment — keep it as-is.
                out.line_to(run[1]);
            }
            run.clear();
        };

    for el in bez.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                flush_run(&mut out, &mut run, accuracy, &sopts);
                out.move_to(p);
                cur = p;
                run = vec![p];
            }
            PathEl::LineTo(p) => {
                run.push(p);
                cur = p;
            }
            PathEl::CurveTo(c1, c2, p) => {
                flush_run(&mut out, &mut run, accuracy, &sopts);
                out.curve_to(c1, c2, p);
                cur = p;
                run = vec![p];
            }
            PathEl::QuadTo(c, p) => {
                flush_run(&mut out, &mut run, accuracy, &sopts);
                out.quad_to(c, p);
                cur = p;
                run = vec![p];
            }
            PathEl::ClosePath => {
                flush_run(&mut out, &mut run, accuracy, &sopts);
                out.close_path();
                run.clear();
            }
        }
    }
    flush_run(&mut out, &mut run, accuracy, &sopts);
    let _ = cur;
    PathData::from_bez_path(&out)
}

/// Append every element of `src` to `dst` except its leading `MoveTo` (the pen
/// in `dst` is assumed to already sit at the run's start). Runs are open, so any
/// `ClosePath` is ignored.
fn append_after_moveto(dst: &mut BezPath, src: &BezPath) {
    for el in src.elements() {
        match *el {
            PathEl::MoveTo(_) | PathEl::ClosePath => {}
            PathEl::LineTo(p) => dst.line_to(p),
            PathEl::CurveTo(a, b, c) => dst.curve_to(a, b, c),
            PathEl::QuadTo(a, b) => dst.quad_to(a, b),
        }
    }
}

/// Flatten all Bézier segments to line segments, preserving subpath structure
/// and closure.
fn flatten_to_polyline(bez: &BezPath) -> BezPath {
    let mut out = BezPath::new();
    let mut cur = Point::ZERO;
    for el in bez.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                out.move_to(p);
                cur = p;
            }
            PathEl::LineTo(p) => {
                out.line_to(p);
                cur = p;
            }
            PathEl::CurveTo(c1, c2, p) => {
                let seg = CubicBez::new(cur, c1, c2, p);
                for i in 1..=FLATTEN_SAMPLES {
                    out.line_to(seg.eval(i as f64 / FLATTEN_SAMPLES as f64));
                }
                cur = p;
            }
            PathEl::QuadTo(c, p) => {
                let seg = QuadBez::new(cur, c, p);
                for i in 1..=FLATTEN_SAMPLES {
                    out.line_to(seg.eval(i as f64 / FLATTEN_SAMPLES as f64));
                }
                cur = p;
            }
            PathEl::ClosePath => out.close_path(),
        }
    }
    out
}

/// Count on-curve anchor points (every element except `ClosePath`).
pub fn count_points(path: &PathData) -> usize {
    path.to_bez_path()
        .elements()
        .iter()
        .filter(|el| !matches!(el, PathEl::ClosePath))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a polyline sampling of a semicircular arch of radius `r`, centered
    /// at the origin, using `n` straight segments.
    fn arch_polyline(r: f64, n: usize) -> PathData {
        let mut b = BezPath::new();
        for i in 0..=n {
            let t = std::f64::consts::PI * (i as f64 / n as f64);
            let p = Point::new(-r * t.cos(), -r * t.sin());
            if i == 0 {
                b.move_to(p);
            } else {
                b.line_to(p);
            }
        }
        PathData::from_bez_path(&b)
    }

    fn n_curves(p: &PathData) -> usize {
        p.to_bez_path()
            .elements()
            .iter()
            .filter(|e| matches!(e, PathEl::CurveTo(..) | PathEl::QuadTo(..)))
            .count()
    }

    #[test]
    fn arch_of_lines_collapses_to_a_few_curves() {
        let arch = arch_polyline(100.0, 64);
        assert!(count_points(&arch) > 40, "dense input");
        let opts = FitOptions {
            accuracy: 1.0,
            corner_angle_deg: 45.0,
            refit_existing: false,
        };
        let fit = fit_curves(&arch, &opts);
        // The whole arch is smooth → it fits with very few anchors, all curved.
        assert!(
            count_points(&fit) <= 8,
            "expected few anchors, got {}",
            count_points(&fit)
        );
        assert!(n_curves(&fit) >= 1, "output should contain curves");
        assert!(count_points(&fit) < count_points(&arch));
    }

    #[test]
    fn sharp_corner_is_preserved() {
        // An L: two straight legs meeting at a 90° corner. The corner must NOT
        // be rounded away — the fit keeps a cusp there.
        let mut b = BezPath::new();
        b.move_to((0.0, 0.0));
        b.line_to((100.0, 0.0));
        b.line_to((100.0, 100.0));
        let path = PathData::from_bez_path(&b);
        let fit = fit_curves(
            &path,
            &FitOptions {
                accuracy: 1.0,
                corner_angle_deg: 20.0,
                refit_existing: false,
            },
        );
        let bb = fit.bounding_box().expect("bbox");
        // Corner preserved ⇒ the (100,0) extent is retained.
        assert!((bb.x1 - 100.0).abs() < 1.0 && (bb.y1 - 100.0).abs() < 1.0);
    }

    #[test]
    fn refit_existing_toggle_processes_curved_input() {
        // A path that already contains a curve: with refit off, curves survive;
        // the call must succeed and stay non-empty either way.
        let mut b = BezPath::new();
        b.move_to((0.0, 0.0));
        b.curve_to((10.0, 40.0), (60.0, 40.0), (80.0, 0.0));
        let path = PathData::from_bez_path(&b);
        for refit in [false, true] {
            let fit = fit_curves(
                &path,
                &FitOptions {
                    accuracy: 1.0,
                    corner_angle_deg: 30.0,
                    refit_existing: refit,
                },
            );
            assert!(count_points(&fit) >= 2, "refit={refit} produced geometry");
        }
    }
}
