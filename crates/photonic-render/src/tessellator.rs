use photonic_core::path::PathData;
use photonic_core::style::{LineCap, LineJoin};

// ── Corner rounding ────────────────────────────────────────────────────────────

/// Returns a new `BezPath` where every sharp LineTo→LineTo corner is replaced
/// by a cubic-bezier arc of the given radius, mirroring CSS `border-radius`.
///
/// Only straight-segment junctions are rounded; bezier curves pass through
/// unchanged.  The radius is clamped to half the shortest adjacent segment so
/// adjacent arcs never overlap.
pub fn round_corners(bez: &kurbo::BezPath, radius: f64) -> kurbo::BezPath {
    if radius <= 0.0 {
        return bez.clone();
    }

    let els: Vec<kurbo::PathEl> = bez.elements().iter().copied().collect();
    if els.is_empty() {
        return bez.clone();
    }

    // Split into per-subpath element lists.
    let mut subpaths: Vec<(Vec<kurbo::PathEl>, bool)> = Vec::new();
    let mut cur: Vec<kurbo::PathEl> = Vec::new();
    for &el in &els {
        match el {
            kurbo::PathEl::MoveTo(_) => {
                if !cur.is_empty() {
                    subpaths.push((cur.clone(), false));
                    cur.clear();
                }
                cur.push(el);
            }
            kurbo::PathEl::ClosePath => {
                cur.push(el);
                subpaths.push((cur.clone(), true));
                cur.clear();
            }
            _ => cur.push(el),
        }
    }
    if !cur.is_empty() {
        subpaths.push((cur, false));
    }

    let mut out = kurbo::BezPath::new();
    for (sp, is_closed) in subpaths {
        round_subpath(&sp, is_closed, radius, &mut out);
    }
    out
}

/// Emit a smooth corner arc from the current path position (which must be `p1`)
/// around `corner` to `p2`, using a quadratic bezier.
///
/// A quadratic bezier with the corner as its control point is guaranteed to be
/// convex and non-overshooting for any interior angle, unlike the cubic
/// `(4/3)·tan` approximation which overshoots for angles > ~100°.
fn emit_corner_arc(
    out: &mut kurbo::BezPath,
    _p1: kurbo::Point, // kept for caller symmetry; path is already positioned here
    corner: kurbo::Point,
    p2: kurbo::Point,
) {
    out.quad_to(corner, p2);
}

/// Round a single subpath.  Only LineTo→LineTo junctions are rounded.
fn round_subpath(sp: &[kurbo::PathEl], is_closed: bool, radius: f64, out: &mut kurbo::BezPath) {
    if sp.is_empty() {
        return;
    }
    let move_pt = match sp[0] {
        kurbo::PathEl::MoveTo(p) => p,
        _ => return,
    };

    // Collect the straight-line vertex run.  Non-LineTo elements break the run.
    // We accumulate line vertices; when a curve or ClosePath is seen we flush.

    let mut line_pts: Vec<kurbo::Point> = vec![move_pt];
    let mut move_emitted = false;

    // Helper: emit a straight-only run with rounded corners.
    // `closed` means the first and last pts are connected by the implicit close edge.
    let emit_line_run =
        |pts: &[kurbo::Point], closed: bool, move_emitted: &mut bool, out: &mut kurbo::BezPath| {
            let n = pts.len();
            if n == 0 {
                return;
            }
            if n == 1 {
                if !*move_emitted {
                    out.move_to(pts[0]);
                    *move_emitted = true;
                }
                return;
            }

            // For each corner i, compute the retreat point (on the incoming segment)
            // and advance point (on the outgoing segment).
            let clamped_r = |i: usize| -> f64 {
                let prev = pts[(i + n - 1) % n];
                let cur = pts[i];
                let next = pts[(i + 1) % n];
                let seg_in = (cur - prev).hypot();
                let seg_out = (next - cur).hypot();
                radius.min(seg_in * 0.5).min(seg_out * 0.5)
            };

            let retreat = |i: usize| -> kurbo::Point {
                let r = clamped_r(i);
                let prev = pts[(i + n - 1) % n];
                let cur = pts[i];
                let d = prev - cur;
                let len = d.hypot();
                if len > 1e-9 {
                    cur + d * (r / len)
                } else {
                    cur
                }
            };

            let advance = |i: usize| -> kurbo::Point {
                let r = clamped_r(i);
                let cur = pts[i];
                let next = pts[(i + 1) % n];
                let d = next - cur;
                let len = d.hypot();
                if len > 1e-9 {
                    cur + d * (r / len)
                } else {
                    cur
                }
            };

            // Determine path winding from signed area so we can identify convex corners.
            // Only convex corners are rounded; concave corners are left sharp to avoid
            // inward arcs that produce overlapping stroke artifacts in the glow.
            let signed_area: f64 = if closed && n >= 3 {
                (0..n)
                    .map(|i| {
                        let a = pts[i];
                        let b = pts[(i + 1) % n];
                        a.x * b.y - b.x * a.y
                    })
                    .sum::<f64>()
            } else {
                1.0
            };
            let winding = if signed_area >= 0.0 {
                1.0_f64
            } else {
                -1.0_f64
            };

            // Returns true if the turn at vertex i is convex (bends outward).
            let is_convex = |i: usize| -> bool {
                let prev = pts[(i + n - 1) % n];
                let cur = pts[i];
                let next = pts[(i + 1) % n];
                let d_in = cur - prev;
                let d_out = next - cur;
                let cross = d_in.x * d_out.y - d_in.y * d_out.x;
                cross * winding > 0.0
            };

            if closed {
                // Walk all vertices; round only convex corners.
                // For concave corners we emit a plain LineTo to the corner vertex.
                let start = if is_convex(0) { retreat(0) } else { pts[0] };
                out.move_to(start);
                *move_emitted = true;
                let mut pos = start;
                for i in 0..n {
                    if is_convex(i) {
                        let r_i = retreat(i);
                        if (pos - r_i).hypot() > 1e-6 {
                            out.line_to(r_i);
                        }
                        let adv_i = advance(i);
                        emit_corner_arc(out, r_i, pts[i], adv_i);
                        pos = adv_i;
                    } else {
                        out.line_to(pts[i]);
                        pos = pts[i];
                    }
                }
                out.close_path();
            } else {
                // Open run: only internal vertices (1..n-2) are corners.
                if !*move_emitted {
                    out.move_to(pts[0]);
                    *move_emitted = true;
                }
                let mut pos = pts[0];
                for i in 1..n - 1 {
                    if is_convex(i) {
                        let r_i = retreat(i);
                        if (pos - r_i).hypot() > 1e-6 {
                            out.line_to(r_i);
                        }
                        let adv_i = advance(i);
                        emit_corner_arc(out, r_i, pts[i], adv_i);
                        pos = adv_i;
                    } else {
                        out.line_to(pts[i]);
                        pos = pts[i];
                    }
                }
                if (pos - pts[n - 1]).hypot() > 1e-6 {
                    out.line_to(pts[n - 1]);
                }
            }
        };

    for el in &sp[1..] {
        match el {
            kurbo::PathEl::LineTo(p) => {
                line_pts.push(*p);
            }
            kurbo::PathEl::ClosePath => {
                emit_line_run(&line_pts, true, &mut move_emitted, out);
                line_pts.clear();
            }
            kurbo::PathEl::CurveTo(c1, c2, p) => {
                emit_line_run(&line_pts, false, &mut move_emitted, out);
                out.curve_to(*c1, *c2, *p);
                line_pts = vec![*p];
            }
            kurbo::PathEl::QuadTo(c, p) => {
                emit_line_run(&line_pts, false, &mut move_emitted, out);
                out.quad_to(*c, *p);
                line_pts = vec![*p];
            }
            _ => {}
        }
    }

    // Flush any remaining open line run (unclosed subpath).
    if line_pts.len() > 1 && !is_closed {
        emit_line_run(&line_pts, false, &mut move_emitted, out);
    }
}

// ── Tessellation ───────────────────────────────────────────────────────────────

/// A tessellated triangle mesh in local path coordinates.
#[derive(Debug, Default, Clone)]
pub struct Mesh {
    pub vertices: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}

impl Mesh {
    pub fn is_empty(&self) -> bool {
        self.vertices.is_empty()
    }
}

/// Tessellate a filled `PathData` into a `Mesh` using lyon.
/// Vertices are returned in path-local coordinates (transforms applied by the renderer).
/// When `even_odd` is true, uses the even-odd fill rule (for compound paths with holes).
pub fn tessellate_fill(path: &PathData, even_odd: bool) -> Mesh {
    use lyon::tessellation::{
        BuffersBuilder, FillOptions, FillRule, FillTessellator, FillVertex, VertexBuffers,
    };

    let bez = path.to_bez_path();
    if bez.elements().is_empty() {
        return Mesh::default();
    }

    let lyon_path = bezpath_to_lyon(&bez);

    let mut geometry: VertexBuffers<[f32; 2], u32> = VertexBuffers::new();
    let mut tess = FillTessellator::new();

    let fill_rule = if even_odd {
        FillRule::EvenOdd
    } else {
        FillRule::NonZero
    };
    let opts = FillOptions::default()
        .with_tolerance(0.1)
        .with_fill_rule(fill_rule);

    if tess
        .tessellate_path(
            &lyon_path,
            &opts,
            &mut BuffersBuilder::new(&mut geometry, |v: FillVertex| {
                [v.position().x, v.position().y]
            }),
        )
        .is_err()
    {
        return Mesh::default();
    }

    Mesh {
        vertices: geometry.vertices,
        indices: geometry.indices,
    }
}

/// Tessellate a stroked `PathData` outline into a `Mesh` using lyon.
pub fn tessellate_stroke(
    path: &PathData,
    width: f32,
    cap: LineCap,
    join: LineJoin,
    miter_limit: f32,
) -> Mesh {
    use lyon::tessellation::{
        BuffersBuilder, LineCap as LyonCap, LineJoin as LyonJoin, StrokeOptions, StrokeTessellator,
        StrokeVertex, VertexBuffers,
    };

    let bez = path.to_bez_path();
    if bez.elements().is_empty() {
        return Mesh::default();
    }

    let lyon_path = bezpath_to_lyon(&bez);

    let lyon_cap = match cap {
        LineCap::Butt => LyonCap::Butt,
        LineCap::Round => LyonCap::Round,
        LineCap::Square => LyonCap::Square,
    };
    let lyon_join = match join {
        LineJoin::Miter => LyonJoin::Miter,
        LineJoin::Round => LyonJoin::Round,
        LineJoin::Bevel => LyonJoin::Bevel,
    };

    let opts = StrokeOptions::default()
        .with_line_width(width)
        .with_tolerance(0.1)
        .with_start_cap(lyon_cap)
        .with_end_cap(lyon_cap)
        .with_line_join(lyon_join)
        .with_miter_limit(miter_limit);

    let mut geometry: VertexBuffers<[f32; 2], u32> = VertexBuffers::new();
    let mut tess = StrokeTessellator::new();

    if tess
        .tessellate_path(
            &lyon_path,
            &opts,
            &mut BuffersBuilder::new(&mut geometry, |v: StrokeVertex| {
                [v.position().x, v.position().y]
            }),
        )
        .is_err()
    {
        return Mesh::default();
    }

    Mesh {
        vertices: geometry.vertices,
        indices: geometry.indices,
    }
}

/// Tessellate a stroked `kurbo::BezPath` (already processed) into a `Mesh`.
/// Used when the path has been pre-transformed (e.g. corner-rounded).
pub fn tessellate_stroke_bez(
    bez: &kurbo::BezPath,
    width: f32,
    cap: LineCap,
    join: LineJoin,
    miter_limit: f32,
) -> Mesh {
    use lyon::tessellation::{
        BuffersBuilder, LineCap as LyonCap, LineJoin as LyonJoin, StrokeOptions, StrokeTessellator,
        StrokeVertex, VertexBuffers,
    };

    if bez.elements().is_empty() {
        return Mesh::default();
    }

    let lyon_path = bezpath_to_lyon(bez);

    let lyon_cap = match cap {
        LineCap::Butt => LyonCap::Butt,
        LineCap::Round => LyonCap::Round,
        LineCap::Square => LyonCap::Square,
    };
    let lyon_join = match join {
        LineJoin::Miter => LyonJoin::Miter,
        LineJoin::Round => LyonJoin::Round,
        LineJoin::Bevel => LyonJoin::Bevel,
    };

    let opts = StrokeOptions::default()
        .with_line_width(width)
        .with_tolerance(0.1)
        .with_start_cap(lyon_cap)
        .with_end_cap(lyon_cap)
        .with_line_join(lyon_join)
        .with_miter_limit(miter_limit);

    let mut geometry: VertexBuffers<[f32; 2], u32> = VertexBuffers::new();
    let mut tess = StrokeTessellator::new();

    if tess
        .tessellate_path(
            &lyon_path,
            &opts,
            &mut BuffersBuilder::new(&mut geometry, |v: StrokeVertex| {
                [v.position().x, v.position().y]
            }),
        )
        .is_err()
    {
        return Mesh::default();
    }

    Mesh {
        vertices: geometry.vertices,
        indices: geometry.indices,
    }
}

/// Convert a `kurbo::BezPath` into a `lyon::path::Path`.
/// Handles open and closed contours, including multiple subpaths.
fn bezpath_to_lyon(bez: &kurbo::BezPath) -> lyon::path::Path {
    use lyon::math::point;
    use lyon::path::Path as LyonPath;

    let mut builder = LyonPath::builder();
    let mut in_contour = false;

    for el in bez.elements() {
        match el {
            kurbo::PathEl::MoveTo(p) => {
                if in_contour {
                    builder.end(false);
                }
                builder.begin(point(p.x as f32, p.y as f32));
                in_contour = true;
            }
            kurbo::PathEl::LineTo(p) => {
                builder.line_to(point(p.x as f32, p.y as f32));
            }
            kurbo::PathEl::CurveTo(c1, c2, p) => {
                builder.cubic_bezier_to(
                    point(c1.x as f32, c1.y as f32),
                    point(c2.x as f32, c2.y as f32),
                    point(p.x as f32, p.y as f32),
                );
            }
            kurbo::PathEl::QuadTo(c, p) => {
                builder.quadratic_bezier_to(
                    point(c.x as f32, c.y as f32),
                    point(p.x as f32, p.y as f32),
                );
            }
            kurbo::PathEl::ClosePath => {
                builder.end(true);
                in_contour = false;
            }
        }
    }
    if in_contour {
        builder.end(false);
    }

    builder.build()
}
