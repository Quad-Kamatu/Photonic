//! Path join operations: close open subpaths and merge two open paths by
//! connecting their nearest endpoints with a straight line.

use crate::path::PathData;
use kurbo::{BezPath, PathEl, Point};

// ─── Public API ───────────────────────────────────────────────────────────────

/// Close every open subpath in `path` by appending a `ClosePath` element.
///
/// Subpaths that already end with `ClosePath` are left unchanged. A subpath
/// that has only a `MoveTo` (i.e. a stray point) is also closed.
pub fn close_open_paths(path: &PathData) -> PathData {
    let elements: Vec<PathEl> = path.to_bez_path().elements().to_vec();
    let mut out = BezPath::new();
    let mut in_subpath = false;
    let mut last_closed = false;

    for el in &elements {
        match *el {
            PathEl::MoveTo(p) => {
                if in_subpath && !last_closed {
                    out.close_path();
                }
                out.move_to(p);
                in_subpath = true;
                last_closed = false;
            }
            PathEl::LineTo(p) => {
                out.line_to(p);
                last_closed = false;
            }
            PathEl::CurveTo(c1, c2, p) => {
                out.curve_to(c1, c2, p);
                last_closed = false;
            }
            PathEl::QuadTo(c, p) => {
                out.quad_to(c, p);
                last_closed = false;
            }
            PathEl::ClosePath => {
                out.close_path();
                last_closed = true;
            }
        }
    }

    if in_subpath && !last_closed {
        out.close_path();
    }

    PathData::from_bez_path(&out)
}

/// Merge two paths by connecting their nearest open endpoints with a straight
/// line segment, returning a single merged path.
///
/// All four endpoint combinations are evaluated (`A_end→B_start`,
/// `A_end→B_end`, `A_start→B_start`, `A_start→B_end`); the shortest gap
/// wins. If either path is empty the other is returned unchanged.
pub fn join_two_paths(a: &PathData, b: &PathData) -> PathData {
    let (a_start, a_end) = match get_endpoints(a) {
        Some(e) => e,
        None => return b.clone(),
    };
    let (b_start, b_end) = match get_endpoints(b) {
        Some(e) => e,
        None => return a.clone(),
    };

    let d_ae_bs = sq_dist(a_end, b_start);
    let d_ae_be = sq_dist(a_end, b_end);
    let d_as_bs = sq_dist(a_start, b_start);
    let d_as_be = sq_dist(a_start, b_end);

    // Pre-compute reversed copies so we can take references uniformly.
    let a_rev = a.reverse();
    let b_rev = b.reverse();

    // Choose (first, second) so that first's END connects to second's START.
    let (first, second): (&PathData, &PathData) =
        if d_ae_bs <= d_ae_be && d_ae_bs <= d_as_bs && d_ae_bs <= d_as_be {
            (a, b) // A_end → B_start
        } else if d_ae_be <= d_as_bs && d_ae_be <= d_as_be {
            (a, &b_rev) // A_end → B_end  (reverse B)
        } else if d_as_bs <= d_as_be {
            (&a_rev, b) // A_start → B_start  (reverse A)
        } else {
            (&a_rev, &b_rev) // A_start → B_end  (reverse both)
        };

    concat_paths(first, second)
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Return the (start, end) of the first subpath in `path`, or `None` if empty.
fn get_endpoints(path: &PathData) -> Option<(Point, Point)> {
    let mut start: Option<Point> = None;
    let mut current = Point::ZERO;

    for el in path.to_bez_path().elements() {
        match *el {
            PathEl::MoveTo(p) => {
                if start.is_none() {
                    start = Some(p);
                }
                current = p;
            }
            PathEl::LineTo(p) => {
                current = p;
            }
            PathEl::CurveTo(_, _, p) => {
                current = p;
            }
            PathEl::QuadTo(_, p) => {
                current = p;
            }
            PathEl::ClosePath => {}
        }
    }

    start.map(|s| (s, current))
}

/// Squared Euclidean distance (avoids a sqrt for comparison purposes).
fn sq_dist(a: Point, b: Point) -> f64 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    dx * dx + dy * dy
}

/// Concatenate `second` onto `first`, bridging the gap with a single `LineTo`.
///
/// Any trailing `ClosePath` on `first` is stripped so the result stays open.
/// The leading `MoveTo` of `second` is replaced by a `LineTo` to that point.
fn concat_paths(first: &PathData, second: &PathData) -> PathData {
    let first_els: Vec<PathEl> = first.to_bez_path().elements().to_vec();
    let second_els: Vec<PathEl> = second.to_bez_path().elements().to_vec();

    let mut out = BezPath::new();

    for el in &first_els {
        match *el {
            PathEl::MoveTo(p) => out.move_to(p),
            PathEl::LineTo(p) => out.line_to(p),
            PathEl::CurveTo(c1, c2, p) => out.curve_to(c1, c2, p),
            PathEl::QuadTo(c, p) => out.quad_to(c, p),
            PathEl::ClosePath => {} // strip close so path stays open
        }
    }

    // Replace the leading MoveTo of `second` with a LineTo to bridge the gap,
    // then append the rest.
    let mut skip_first = false;
    if let Some(PathEl::MoveTo(p)) = second_els.first() {
        out.line_to(*p);
        skip_first = true;
    }

    let rest = if skip_first {
        &second_els[1..]
    } else {
        &second_els[..]
    };
    for el in rest {
        match *el {
            PathEl::MoveTo(p) => out.move_to(p),
            PathEl::LineTo(p) => out.line_to(p),
            PathEl::CurveTo(c1, c2, p) => out.curve_to(c1, c2, p),
            PathEl::QuadTo(c, p) => out.quad_to(c, p),
            PathEl::ClosePath => out.close_path(),
        }
    }

    PathData::from_bez_path(&out)
}
