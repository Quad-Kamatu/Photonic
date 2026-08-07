/// Boolean path operations: union, intersection, difference, exclusion.
///
/// Paths are first flattened to polygons (curves sampled to line segments),
/// then the `geo` crate's BooleanOps are applied, and the result is
/// converted back to a `PathData`.
use crate::path::PathData;
use geo::{BooleanOps, Coord, LineString, MultiPolygon, Polygon};
use kurbo::{BezPath, CubicBez, ParamCurve, QuadBez};
use std::{fmt, panic::AssertUnwindSafe};

const ZERO_EDGE_EPSILON: f64 = 1e-12;
const AREA_EPSILON: f64 = 1e-12;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BooleanOp {
    Union,
    Intersect,
    Subtract,
    Exclude,
    Divide,
}

/// Failure returned by path conversion or the third-party BooleanOps engine.
///
/// This is intentionally an ordinary error instead of a panic: malformed
/// imported paths are user data, and geo 0.28 has a few panic paths for
/// degenerate rings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BooleanError {
    InvalidRing {
        path: &'static str,
        ring: usize,
        reason: &'static str,
    },
    BackendFailure {
        stage: &'static str,
    },
    UnsupportedOperation,
}

impl fmt::Display for BooleanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRing { path, ring, reason } => {
                write!(f, "invalid {path} ring {ring}: {reason}")
            }
            Self::BackendFailure { stage } => {
                write!(f, "geometry backend failed during {stage}")
            }
            Self::UnsupportedOperation => {
                f.write_str("use divide_paths() for Divide — it produces multiple output paths")
            }
        }
    }
}

impl std::error::Error for BooleanError {}

/// Compute a boolean operation on two paths.
pub fn boolean_op(a: &PathData, b: &PathData, op: BooleanOp) -> Result<PathData, BooleanError> {
    let mp_a = path_to_multi_polygon(a, "first input")?;
    let mp_b = path_to_multi_polygon(b, "second input")?;

    let result = match op {
        BooleanOp::Union => safe_geo_boolean("union", || mp_a.union(&mp_b))?,
        BooleanOp::Intersect => safe_geo_boolean("intersection", || mp_a.intersection(&mp_b))?,
        BooleanOp::Subtract => safe_geo_boolean("difference", || mp_a.difference(&mp_b))?,
        BooleanOp::Exclude => safe_geo_boolean("exclusive-or", || mp_a.xor(&mp_b))?,
        BooleanOp::Divide => return Err(BooleanError::UnsupportedOperation),
    };

    multi_polygon_to_path(&result)
}

/// Divide two paths at every overlap edge, producing up to three distinct faces:
/// - face 0: region only in `a` (source index 0)
/// - face 1: overlapping region (source index 0 — inherits from `a`, the back shape)
/// - face 2: region only in `b` (source index 1)
///
/// Returns `(PathData, source_index)` pairs, skipping empty regions.
pub fn divide_paths(a: &PathData, b: &PathData) -> Result<Vec<(PathData, usize)>, BooleanError> {
    let mp_a = path_to_multi_polygon(a, "first input")?;
    let mp_b = path_to_multi_polygon(b, "second input")?;

    let mut faces: Vec<(PathData, usize)> = Vec::new();

    let a_only = safe_geo_boolean("divide a-only difference", || mp_a.difference(&mp_b))?;
    if !a_only.0.is_empty() {
        faces.push((multi_polygon_to_path(&a_only)?, 0));
    }

    let overlap = safe_geo_boolean("divide overlap intersection", || mp_a.intersection(&mp_b))?;
    if !overlap.0.is_empty() {
        faces.push((multi_polygon_to_path(&overlap)?, 0));
    }

    let b_only = safe_geo_boolean("divide b-only difference", || mp_b.difference(&mp_a))?;
    if !b_only.0.is_empty() {
        faces.push((multi_polygon_to_path(&b_only)?, 1));
    }

    Ok(faces)
}

fn safe_geo_boolean(
    stage: &'static str,
    operation: impl FnOnce() -> MultiPolygon<f64>,
) -> Result<MultiPolygon<f64>, BooleanError> {
    std::panic::catch_unwind(AssertUnwindSafe(operation))
        .map_err(|_| BooleanError::BackendFailure { stage })
}

// ─── Path → geo::MultiPolygon ─────────────────────────────────────────────────

/// Flatten a `PathData` into a `MultiPolygon` by sampling cubic/quadratic
/// Bézier curves as line segments (8 segments per cubic, 6 per quadratic).
fn path_to_multi_polygon(
    path: &PathData,
    path_label: &'static str,
) -> Result<MultiPolygon<f64>, BooleanError> {
    let bez = path.to_bez_path();
    let mut polygons: Vec<Polygon<f64>> = Vec::new();
    let mut current_ring: Vec<Coord<f64>> = Vec::new();
    let mut ring_index = 0;

    for el in bez.elements() {
        match *el {
            kurbo::PathEl::MoveTo(p) => {
                ensure_finite_point(p.x, p.y, path_label, ring_index)?;
                if !current_ring.is_empty() {
                    flush_ring(&mut current_ring, &mut polygons, path_label, ring_index)?;
                    ring_index += 1;
                }
                current_ring.push(Coord { x: p.x, y: p.y });
            }
            kurbo::PathEl::LineTo(p) => {
                ensure_finite_point(p.x, p.y, path_label, ring_index)?;
                current_ring.push(Coord { x: p.x, y: p.y });
            }
            kurbo::PathEl::CurveTo(c1, c2, p) => {
                ensure_finite_point(c1.x, c1.y, path_label, ring_index)?;
                ensure_finite_point(c2.x, c2.y, path_label, ring_index)?;
                ensure_finite_point(p.x, p.y, path_label, ring_index)?;
                let last = current_ring.last().ok_or(BooleanError::InvalidRing {
                    path: path_label,
                    ring: ring_index,
                    reason: "curve segment has no starting point",
                })?;
                let p0 = kurbo::Point::new(last.x, last.y);
                let seg = CubicBez::new(p0, c1, c2, p);
                for i in 1..=8 {
                    let pt = seg.eval(i as f64 / 8.0);
                    ensure_finite_point(pt.x, pt.y, path_label, ring_index)?;
                    current_ring.push(Coord { x: pt.x, y: pt.y });
                }
            }
            kurbo::PathEl::QuadTo(c, p) => {
                ensure_finite_point(c.x, c.y, path_label, ring_index)?;
                ensure_finite_point(p.x, p.y, path_label, ring_index)?;
                let last = current_ring.last().ok_or(BooleanError::InvalidRing {
                    path: path_label,
                    ring: ring_index,
                    reason: "quadratic segment has no starting point",
                })?;
                let p0 = kurbo::Point::new(last.x, last.y);
                let seg = QuadBez::new(p0, c, p);
                for i in 1..=6 {
                    let pt = seg.eval(i as f64 / 6.0);
                    ensure_finite_point(pt.x, pt.y, path_label, ring_index)?;
                    current_ring.push(Coord { x: pt.x, y: pt.y });
                }
            }
            kurbo::PathEl::ClosePath => {
                if !current_ring.is_empty() {
                    flush_ring(&mut current_ring, &mut polygons, path_label, ring_index)?;
                    ring_index += 1;
                }
            }
        }
    }

    if !current_ring.is_empty() {
        flush_ring(&mut current_ring, &mut polygons, path_label, ring_index)?;
    }

    Ok(MultiPolygon::new(polygons))
}

fn ensure_finite_point(
    x: f64,
    y: f64,
    path: &'static str,
    ring: usize,
) -> Result<(), BooleanError> {
    if x.is_finite() && y.is_finite() {
        Ok(())
    } else {
        Err(BooleanError::InvalidRing {
            path,
            ring,
            reason: "contains a non-finite coordinate",
        })
    }
}

fn flush_ring(
    ring: &mut Vec<Coord<f64>>,
    polygons: &mut Vec<Polygon<f64>>,
    path: &'static str,
    ring_index: usize,
) -> Result<(), BooleanError> {
    let normalized = sanitize_ring(std::mem::take(ring), path, ring_index)?;
    polygons.push(Polygon::new(LineString::new(normalized), vec![]));
    Ok(())
}

/// Normalize a ring into geo's one-closure representation and reject shapes
/// that cannot describe an area. This is also used on geo output so malformed
/// third-party output cannot leak non-finite coordinates into PathData.
fn sanitize_ring(
    ring: Vec<Coord<f64>>,
    path: &'static str,
    ring_index: usize,
) -> Result<Vec<Coord<f64>>, BooleanError> {
    if ring
        .iter()
        .any(|coord| !coord.x.is_finite() || !coord.y.is_finite())
    {
        return Err(BooleanError::InvalidRing {
            path,
            ring: ring_index,
            reason: "contains a non-finite coordinate",
        });
    }

    let Some(first) = ring.first().copied() else {
        return Err(BooleanError::InvalidRing {
            path,
            ring: ring_index,
            reason: "is empty",
        });
    };

    // Strip every explicit closure and then remove all adjacent zero-length
    // edges. A single exact closure is added back below.
    let mut open = ring;
    while open.len() > 1 && same_point(open[0], *open.last().unwrap()) {
        open.pop();
    }

    let mut deduped = Vec::with_capacity(open.len());
    for coord in open {
        if deduped
            .last()
            .map_or(true, |last| !same_point(*last, coord))
        {
            deduped.push(coord);
        }
    }
    while deduped.len() > 1 && same_point(deduped[0], *deduped.last().unwrap()) {
        deduped.pop();
    }

    let distinct = deduped.iter().fold(Vec::new(), |mut points, coord| {
        if !points.iter().any(|point| same_point(*point, *coord)) {
            points.push(*coord);
        }
        points
    });
    if distinct.len() < 3 {
        return Err(BooleanError::InvalidRing {
            path,
            ring: ring_index,
            reason: "must contain at least 3 distinct vertices",
        });
    }

    let mut min_x = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    let mut area2 = 0.0;
    for coord in &deduped {
        min_x = min_x.min(coord.x);
        max_x = max_x.max(coord.x);
        min_y = min_y.min(coord.y);
        max_y = max_y.max(coord.y);
    }
    let origin = deduped[0];
    for (a, b) in deduped
        .iter()
        .zip(deduped.iter().cycle().skip(1))
        .take(deduped.len())
    {
        area2 += (a.x - origin.x) * (b.y - origin.y) - (b.x - origin.x) * (a.y - origin.y);
    }
    let scale = (max_x - min_x).max(max_y - min_y);
    if !area2.is_finite() || !scale.is_finite() || area2.abs() <= AREA_EPSILON * scale * scale {
        return Err(BooleanError::InvalidRing {
            path,
            ring: ring_index,
            reason: "has near-zero area (collinear or degenerate)",
        });
    }

    deduped.push(first);
    Ok(deduped)
}

fn same_point(a: Coord<f64>, b: Coord<f64>) -> bool {
    (a.x == b.x && a.y == b.y) || (a.x - b.x).hypot(a.y - b.y) <= ZERO_EDGE_EPSILON
}

// ─── geo::MultiPolygon → Path ─────────────────────────────────────────────────

fn multi_polygon_to_path(mp: &MultiPolygon<f64>) -> Result<PathData, BooleanError> {
    let mut bez = BezPath::new();
    let mut ring_index = 0;
    for polygon in &mp.0 {
        add_ring_to_bez(&mut bez, polygon.exterior(), ring_index)?;
        ring_index += 1;
        for interior in polygon.interiors() {
            add_ring_to_bez(&mut bez, interior, ring_index)?;
            ring_index += 1;
        }
    }
    Ok(PathData::from_bez_path(&bez))
}

fn add_ring_to_bez(
    bez: &mut BezPath,
    ring: &LineString<f64>,
    ring_index: usize,
) -> Result<(), BooleanError> {
    let coords: Vec<Coord<f64>> = ring.coords().copied().collect();
    if coords.is_empty() {
        return Err(BooleanError::InvalidRing {
            path: "geo output",
            ring: ring_index,
            reason: "is empty",
        });
    }
    let coords = sanitize_ring(coords, "geo output", ring_index)?;
    bez.move_to((coords[0].x, coords[0].y));
    for coord in &coords[1..coords.len() - 1] {
        bez.line_to((coord.x, coord.y));
    }
    bez.close_path();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn rectangle(x: f64, y: f64, width: f64, height: f64) -> PathData {
        PathData::rect(x, y, width, height)
    }

    fn path_is_finite(path: &PathData) -> bool {
        path.to_bez_path().elements().iter().all(|el| match el {
            kurbo::PathEl::MoveTo(p) | kurbo::PathEl::LineTo(p) => {
                p.x.is_finite() && p.y.is_finite()
            }
            kurbo::PathEl::CurveTo(a, b, p) => [a, b, p]
                .iter()
                .all(|point| point.x.is_finite() && point.y.is_finite()),
            kurbo::PathEl::QuadTo(a, p) => [a, p]
                .iter()
                .all(|point| point.x.is_finite() && point.y.is_finite()),
            kurbo::PathEl::ClosePath => true,
        })
    }

    #[test]
    fn sanitizer_removes_duplicate_edges_and_keeps_one_closure() {
        let ring = vec![
            Coord { x: 0.0, y: 0.0 },
            Coord { x: 0.0, y: 0.0 },
            Coord { x: 10.0, y: 0.0 },
            Coord { x: 10.0, y: 10.0 },
            Coord { x: 0.0, y: 10.0 },
            Coord { x: 0.0, y: 10.0 },
            Coord { x: 0.0, y: 0.0 },
            Coord { x: 0.0, y: 0.0 },
        ];
        let sanitized = sanitize_ring(ring, "test", 0).unwrap();
        assert_eq!(sanitized.len(), 5);
        assert_eq!(sanitized.first(), sanitized.last());
        assert_ne!(sanitized[0], sanitized[1]);
        assert_ne!(sanitized[3], sanitized[4]);
    }

    #[test]
    fn sanitizer_rejects_nonfinite_short_and_collinear_rings() {
        let cases = [
            (
                vec![
                    Coord { x: 0.0, y: 0.0 },
                    Coord { x: 1.0, y: 0.0 },
                    Coord {
                        x: f64::NAN,
                        y: 1.0,
                    },
                ],
                "non-finite",
            ),
            (
                vec![Coord { x: 0.0, y: 0.0 }, Coord { x: 1.0, y: 0.0 }],
                "3 distinct",
            ),
            (
                vec![
                    Coord { x: 0.0, y: 0.0 },
                    Coord { x: 1.0, y: 0.0 },
                    Coord { x: 2.0, y: 0.0 },
                ],
                "near-zero",
            ),
        ];
        for (ring, expected) in cases {
            let error = sanitize_ring(ring, "test", 0).unwrap_err().to_string();
            assert!(error.contains(expected), "{error}");
        }
    }

    #[test]
    fn boolean_operations_return_clear_errors_for_degenerate_paths() {
        let degenerate = PathData::from_svg("M 0 0 L 1 0 L 2 0 Z").unwrap();
        let error = boolean_op(
            &degenerate,
            &rectangle(0.0, 0.0, 10.0, 10.0),
            BooleanOp::Union,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("first input ring 0"), "{error}");
        assert!(error.contains("near-zero area"), "{error}");
        assert!(divide_paths(&degenerate, &rectangle(0.0, 0.0, 10.0, 10.0)).is_err());
    }

    #[test]
    fn geo_panics_are_returned_as_structured_errors() {
        let error =
            safe_geo_boolean("test operation", || panic!("synthetic geo panic")).unwrap_err();
        assert!(matches!(
            error,
            BooleanError::BackendFailure {
                stage: "test operation"
            }
        ));
        assert_eq!(
            error.to_string(),
            "geometry backend failed during test operation"
        );
    }

    #[test]
    fn valid_boolean_output_is_finite() {
        let result = boolean_op(
            &rectangle(0.0, 0.0, 10.0, 10.0),
            &rectangle(5.0, 5.0, 10.0, 10.0),
            BooleanOp::Union,
        )
        .unwrap();
        assert!(path_is_finite(&result));
    }

    proptest! {
        #[test]
        fn boolean_ops_never_panic_and_never_emit_nonfinite_paths(
            coords in prop::array::uniform8(-1_000.0f64..1_000.0f64),
        ) {
            let mut first = BezPath::new();
            first.move_to((coords[0], coords[1]));
            first.line_to((coords[2], coords[3]));
            first.line_to((coords[4], coords[5]));
            first.line_to((coords[6], coords[7]));
            first.close_path();

            let second = rectangle(-250.0, -250.0, 500.0, 500.0);
            let first = PathData::from_bez_path(&first);
            let result = std::panic::catch_unwind(|| boolean_op(&first, &second, BooleanOp::Union));
            prop_assert!(result.is_ok());
            if let Ok(Ok(path)) = result {
                prop_assert!(path_is_finite(&path));
            }
        }
    }
}
