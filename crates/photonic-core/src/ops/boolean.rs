/// Boolean path operations: union, intersection, difference, exclusion.
///
/// Paths are first flattened to polygons (curves sampled to line segments),
/// then the `geo` crate's `BooleanOps` are applied, and the result is
/// converted back to a `PathData`.
use crate::path::PathData;
use geo::{BooleanOps, Coord, LineString, MultiPolygon, Polygon};
use kurbo::{BezPath, CubicBez, ParamCurve, QuadBez};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BooleanOp {
    Union,
    Intersect,
    Subtract,
    Exclude,
    Divide,
}

/// Compute a boolean operation on two paths.
/// Returns the resulting path, or an error string if the operation fails.
pub fn boolean_op(a: &PathData, b: &PathData, op: BooleanOp) -> Result<PathData, String> {
    let mp_a = path_to_multi_polygon(a);
    let mp_b = path_to_multi_polygon(b);

    let result = match op {
        BooleanOp::Union => mp_a.union(&mp_b),
        BooleanOp::Intersect => mp_a.intersection(&mp_b),
        BooleanOp::Subtract => mp_a.difference(&mp_b),
        BooleanOp::Exclude => mp_a.xor(&mp_b),
        BooleanOp::Divide => {
            return Err(
                "Use divide_paths() for Divide — it produces multiple output paths".to_string(),
            )
        }
    };

    Ok(multi_polygon_to_path(&result))
}

/// Divide two paths at every overlap edge, producing up to three distinct faces:
/// - face 0: region only in `a` (source index 0)
/// - face 1: overlapping region (source index 0 — inherits from `a`, the back shape)
/// - face 2: region only in `b` (source index 1)
///
/// Returns `(PathData, source_index)` pairs, skipping empty regions.
pub fn divide_paths(a: &PathData, b: &PathData) -> Vec<(PathData, usize)> {
    let mp_a = path_to_multi_polygon(a);
    let mp_b = path_to_multi_polygon(b);

    let mut faces: Vec<(PathData, usize)> = Vec::new();

    let a_only = mp_a.difference(&mp_b);
    if !a_only.0.is_empty() {
        faces.push((multi_polygon_to_path(&a_only), 0));
    }

    let overlap = mp_a.intersection(&mp_b);
    if !overlap.0.is_empty() {
        faces.push((multi_polygon_to_path(&overlap), 0));
    }

    let b_only = mp_b.difference(&mp_a);
    if !b_only.0.is_empty() {
        faces.push((multi_polygon_to_path(&b_only), 1));
    }

    faces
}

// ─── Path → geo::MultiPolygon ─────────────────────────────────────────────────

/// Flatten a `PathData` into a `MultiPolygon` by sampling cubic/quadratic
/// Bézier curves as line segments (8 segments per cubic, 6 per quadratic).
fn path_to_multi_polygon(path: &PathData) -> MultiPolygon<f64> {
    let bez = path.to_bez_path();
    let mut polygons: Vec<Polygon<f64>> = Vec::new();
    let mut current_ring: Vec<Coord<f64>> = Vec::new();

    for el in bez.elements() {
        match *el {
            kurbo::PathEl::MoveTo(p) => {
                if current_ring.len() >= 3 {
                    flush_ring(&mut current_ring, &mut polygons);
                }
                current_ring.clear();
                current_ring.push(Coord { x: p.x, y: p.y });
            }
            kurbo::PathEl::LineTo(p) => {
                current_ring.push(Coord { x: p.x, y: p.y });
            }
            kurbo::PathEl::CurveTo(c1, c2, p) => {
                if let Some(&last) = current_ring.last() {
                    let p0 = kurbo::Point::new(last.x, last.y);
                    let seg = CubicBez::new(p0, c1, c2, p);
                    for i in 1..=8 {
                        let pt = seg.eval(i as f64 / 8.0);
                        current_ring.push(Coord { x: pt.x, y: pt.y });
                    }
                }
            }
            kurbo::PathEl::QuadTo(c, p) => {
                if let Some(&last) = current_ring.last() {
                    let p0 = kurbo::Point::new(last.x, last.y);
                    let seg = QuadBez::new(p0, c, p);
                    for i in 1..=6 {
                        let pt = seg.eval(i as f64 / 6.0);
                        current_ring.push(Coord { x: pt.x, y: pt.y });
                    }
                }
            }
            kurbo::PathEl::ClosePath => {
                if current_ring.len() >= 3 {
                    flush_ring(&mut current_ring, &mut polygons);
                }
                current_ring.clear();
            }
        }
    }

    if current_ring.len() >= 3 {
        flush_ring(&mut current_ring, &mut polygons);
    }

    MultiPolygon::new(polygons)
}

fn flush_ring(ring: &mut Vec<Coord<f64>>, polygons: &mut Vec<Polygon<f64>>) {
    // geo requires closed rings (first coord == last coord)
    if ring.first() != ring.last() {
        let first = *ring.first().unwrap();
        ring.push(first);
    }
    let ls = LineString::new(ring.clone());
    polygons.push(Polygon::new(ls, vec![]));
    ring.clear();
}

// ─── geo::MultiPolygon → Path ─────────────────────────────────────────────────

fn multi_polygon_to_path(mp: &MultiPolygon<f64>) -> PathData {
    let mut bez = BezPath::new();
    for polygon in &mp.0 {
        add_ring_to_bez(&mut bez, polygon.exterior());
        for interior in polygon.interiors() {
            add_ring_to_bez(&mut bez, interior);
        }
    }
    PathData::from_bez_path(&bez)
}

fn add_ring_to_bez(bez: &mut BezPath, ring: &LineString<f64>) {
    let coords: Vec<&Coord<f64>> = ring.coords().collect();
    // geo rings are always closed (first == last), skip the repeated last coord
    let n = if coords.len() > 1 && coords.first() == coords.last() {
        coords.len() - 1
    } else {
        coords.len()
    };
    if n < 3 {
        return;
    }
    bez.move_to((coords[0].x, coords[0].y));
    for coord in &coords[1..n] {
        bez.line_to((coord.x, coord.y));
    }
    bez.close_path();
}
