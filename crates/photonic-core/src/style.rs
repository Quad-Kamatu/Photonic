use crate::color::Color;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A fill applied to a shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fill {
    pub kind: FillKind,
    pub opacity: f32,
    pub enabled: bool,
}

impl Fill {
    pub fn solid(color: Color) -> Self {
        Self {
            kind: FillKind::Solid(color),
            opacity: 1.0,
            enabled: true,
        }
    }

    pub fn none() -> Self {
        Self {
            kind: FillKind::None,
            opacity: 1.0,
            enabled: false,
        }
    }

    pub fn gradient(gradient: Gradient) -> Self {
        Self {
            kind: FillKind::Gradient(gradient),
            opacity: 1.0,
            enabled: true,
        }
    }

    pub fn fluid_gradient(gradient: FluidGradient) -> Self {
        Self {
            kind: FillKind::FluidGradient(gradient),
            opacity: 1.0,
            enabled: true,
        }
    }

    pub fn mesh_gradient(gradient: MeshGradient) -> Self {
        Self {
            kind: FillKind::MeshGradient(gradient),
            opacity: 1.0,
            enabled: true,
        }
    }
}

impl Default for Fill {
    fn default() -> Self {
        Self::solid(Color::BLACK)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FillKind {
    None,
    Solid(Color),
    Gradient(Gradient),
    FluidGradient(FluidGradient),
    MeshGradient(MeshGradient),
    Pattern(PatternFill),
}

/// A built-in geometric pattern type for [`PatternFill`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PatternKind {
    /// Filled circles on a square grid.
    #[default]
    Dots,
    /// Vertical bars.
    Stripes,
    /// Thin horizontal + vertical lines.
    Grid,
    /// Alternating filled squares.
    Checkerboard,
}

/// A repeating geometric pattern fill. The shape is painted with `background`
/// (if any), then the pattern foreground (clipped to the shape) in `color`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PatternFill {
    pub kind: PatternKind,
    /// Foreground colour (dots, stripes, lines, filled cells).
    pub color: Color,
    /// Optional background colour painted behind the pattern.
    #[serde(default)]
    pub background: Option<Color>,
    /// Tile size / spacing in document units.
    pub spacing: f64,
}

impl Default for PatternFill {
    fn default() -> Self {
        Self {
            kind: PatternKind::Dots,
            color: Color::BLACK,
            background: None,
            spacing: 12.0,
        }
    }
}

impl FillKind {
    /// Sample the fill color at document-space coordinates `(x, y)`.
    /// `opacity` is the combined fill opacity × node opacity.
    pub fn sample_at(&self, x: f64, y: f64, opacity: f32) -> [f32; 4] {
        match self {
            FillKind::None => [0.0; 4],
            FillKind::Solid(col) => [col.r, col.g, col.b, col.a * opacity],
            FillKind::Gradient(g) => {
                let [r, g2, b, a] = g.sample_at(x, y);
                [r, g2, b, a * opacity]
            }
            FillKind::FluidGradient(fg) => {
                let [r, g2, b, a] = fg.sample_at(x, y);
                [r, g2, b, a * opacity]
            }
            FillKind::MeshGradient(mg) => {
                let [r, g2, b, a] = mg.sample_at(x, y);
                [r, g2, b, a * opacity]
            }
            // The base fill paints the background; the foreground pattern is
            // emitted as separate clipped geometry (see `pattern_foreground`).
            FillKind::Pattern(p) => match p.background {
                Some(c) => [c.r, c.g, c.b, c.a * opacity],
                None => [0.0; 4],
            },
        }
    }
}

/// Generate the foreground geometry of a [`PatternFill`], clipped to `shape`.
///
/// Pattern elements (dots, stripes, grid lines, checkerboard cells) are built
/// across the shape's bounding box, unioned into one compound path, and
/// intersected with the shape so the result never overflows the outline. Returns
/// an empty path for a degenerate shape or spacing.
pub fn pattern_foreground(
    shape: &crate::path::PathData,
    pf: &PatternFill,
) -> crate::path::PathData {
    use crate::path::PathData;
    use kurbo::{BezPath, Rect, Shape};

    let empty = PathData::new();
    let Some(bbox) = shape.bounding_box() else {
        return empty;
    };
    let s = pf.spacing.max(1.0);
    // Bound the element count so very fine patterns on large shapes stay cheap.
    let cols = ((bbox.width() / s).ceil() as i64 + 2).clamp(1, 400);
    let rows = ((bbox.height() / s).ceil() as i64 + 2).clamp(1, 400);
    let x0 = bbox.x0 - s;
    let y0 = bbox.y0 - s;

    let mut elems = BezPath::new();
    match pf.kind {
        PatternKind::Dots => {
            let r = s * 0.25;
            for j in 0..rows {
                for i in 0..cols {
                    let cx = x0 + (i as f64 + 0.5) * s;
                    let cy = y0 + (j as f64 + 0.5) * s;
                    elems.extend(kurbo::Circle::new((cx, cy), r).path_elements(0.2));
                }
            }
        }
        PatternKind::Stripes => {
            let w = s * 0.5;
            let h = (rows as f64 + 2.0) * s;
            for i in 0..cols {
                let sx = x0 + i as f64 * s;
                elems.extend(Rect::new(sx, y0, sx + w, y0 + h).path_elements(0.2));
            }
        }
        PatternKind::Grid => {
            let t = (s * 0.1).max(0.5);
            let w = (cols as f64 + 2.0) * s;
            let h = (rows as f64 + 2.0) * s;
            for j in 0..=rows {
                let ly = y0 + j as f64 * s;
                elems.extend(Rect::new(x0, ly, x0 + w, ly + t).path_elements(0.2));
            }
            for i in 0..=cols {
                let lx = x0 + i as f64 * s;
                elems.extend(Rect::new(lx, y0, lx + t, y0 + h).path_elements(0.2));
            }
        }
        PatternKind::Checkerboard => {
            for j in 0..rows {
                for i in 0..cols {
                    if (i + j) % 2 != 0 {
                        continue;
                    }
                    let sx = x0 + i as f64 * s;
                    let sy = y0 + j as f64 * s;
                    elems.extend(Rect::new(sx, sy, sx + s, sy + s).path_elements(0.2));
                }
            }
        }
    }

    if elems.is_empty() {
        return empty;
    }
    let elems_path = PathData::from_bez_path(&elems);
    crate::ops::boolean::boolean_op(
        &elems_path,
        shape,
        crate::ops::boolean::BooleanOp::Intersect,
    )
    .unwrap_or(empty)
}

/// Where the stroke is painted relative to the path edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum StrokeAlign {
    /// Stroke is centred on the path outline (default).
    #[default]
    Center,
    /// Stroke is painted entirely inside the filled area.
    Inside,
    /// Stroke is painted entirely outside the filled area.
    Outside,
}

/// Arrowhead style for stroke start/end.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ArrowheadStyle {
    /// No arrowhead (default).
    #[default]
    None,
    /// Filled triangular arrowhead.
    FilledArrow,
    /// Open V-shaped arrowhead (two angled lines).
    OpenArrow,
}

/// A stroke applied to a path outline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stroke {
    pub color: Color,
    pub width: f64,
    pub opacity: f32,
    pub line_cap: LineCap,
    pub line_join: LineJoin,
    pub miter_limit: f64,
    pub dash_array: Vec<f64>,
    pub dash_offset: f64,
    pub enabled: bool,
    #[serde(default)]
    pub align: StrokeAlign,
    /// Align dashes to path corners and endpoints so they never appear clipped.
    #[serde(default)]
    pub dash_corner_alignment: bool,
    /// Arrowhead drawn at the start of the path.
    #[serde(default)]
    pub arrowhead_start: ArrowheadStyle,
    /// Arrowhead drawn at the end of the path.
    #[serde(default)]
    pub arrowhead_end: ArrowheadStyle,
    /// Optional variable-width profile (id into `Document::width_profiles`).
    /// When `Some`, the renderer modulates the stroke width along the path
    /// using the profile's samples instead of the uniform `width`.
    #[serde(default)]
    pub width_profile_id: Option<Uuid>,
}

impl Stroke {
    pub fn solid(color: Color, width: f64) -> Self {
        Self {
            color,
            width,
            opacity: 1.0,
            line_cap: LineCap::Butt,
            line_join: LineJoin::Miter,
            miter_limit: 4.0,
            dash_array: vec![],
            dash_offset: 0.0,
            enabled: true,
            align: StrokeAlign::Center,
            dash_corner_alignment: false,
            arrowhead_start: ArrowheadStyle::None,
            arrowhead_end: ArrowheadStyle::None,
            width_profile_id: None,
        }
    }

    pub fn none() -> Self {
        Self {
            color: Color::BLACK,
            width: 1.0,
            opacity: 1.0,
            line_cap: LineCap::Butt,
            line_join: LineJoin::Miter,
            miter_limit: 4.0,
            dash_array: vec![],
            dash_offset: 0.0,
            enabled: false,
            align: StrokeAlign::Center,
            dash_corner_alignment: false,
            arrowhead_start: ArrowheadStyle::None,
            arrowhead_end: ArrowheadStyle::None,
            width_profile_id: None,
        }
    }
}

impl Default for Stroke {
    fn default() -> Self {
        Self::none()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LineCap {
    #[default]
    Butt,
    Round,
    Square,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LineJoin {
    #[default]
    Miter,
    Round,
    Bevel,
}

// ─── Linear / Radial gradient ────────────────────────────────────────────────

/// A linear or radial gradient.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Gradient {
    pub kind: GradientKind,
    pub stops: Vec<GradientStop>,
    /// For linear: [x0, y0, x1, y1] in document/world space
    /// For radial: [cx, cy, fx, fy, r] (center, focal point, radius)
    pub coords: Vec<f64>,
}

impl Gradient {
    pub fn linear(x0: f64, y0: f64, x1: f64, y1: f64, stops: Vec<GradientStop>) -> Self {
        Self {
            kind: GradientKind::Linear,
            stops,
            coords: vec![x0, y0, x1, y1],
        }
    }

    pub fn radial(cx: f64, cy: f64, r: f64, stops: Vec<GradientStop>) -> Self {
        Self {
            kind: GradientKind::Radial,
            stops,
            coords: vec![cx, cy, cx, cy, r],
        }
    }

    /// Sample the gradient color at document-space coordinates `(x, y)`.
    pub fn sample_at(&self, x: f64, y: f64) -> [f32; 4] {
        let t = match self.kind {
            GradientKind::Linear => {
                if self.coords.len() < 4 {
                    return interpolate_stops(&self.stops, 0.0);
                }
                let (x0, y0, x1, y1) = (
                    self.coords[0],
                    self.coords[1],
                    self.coords[2],
                    self.coords[3],
                );
                let dx = x1 - x0;
                let dy = y1 - y0;
                let len2 = dx * dx + dy * dy;
                if len2 < 1e-12 {
                    return interpolate_stops(&self.stops, 0.0);
                }
                let t = ((x - x0) * dx + (y - y0) * dy) / len2;
                t.clamp(0.0, 1.0) as f32
            }
            GradientKind::Radial => {
                if self.coords.len() < 5 {
                    return interpolate_stops(&self.stops, 0.0);
                }
                let (cx, cy, r) = (self.coords[0], self.coords[1], self.coords[4]);
                if r < 1e-12 {
                    return interpolate_stops(&self.stops, 0.0);
                }
                let dx = x - cx;
                let dy = y - cy;
                let dist = (dx * dx + dy * dy).sqrt();
                (dist / r).clamp(0.0, 1.0) as f32
            }
        };
        interpolate_stops(&self.stops, t)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GradientKind {
    Linear,
    Radial,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GradientStop {
    /// Position in [0.0, 1.0]
    pub offset: f32,
    pub color: Color,
}

impl GradientStop {
    pub fn new(offset: f32, color: Color) -> Self {
        Self { offset, color }
    }
}

// ─── Fluid (free-point) gradient ─────────────────────────────────────────────

/// A single control point in a fluid gradient.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FluidGradientPoint {
    /// Position in document/world space.
    pub x: f64,
    pub y: f64,
    pub color: Color,
}

impl FluidGradientPoint {
    pub fn new(x: f64, y: f64, color: Color) -> Self {
        Self { x, y, color }
    }
}

/// Fluid (free-placed) gradient: colors blended via inverse-distance weighting
/// from arbitrary control points.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FluidGradient {
    pub points: Vec<FluidGradientPoint>,
    /// IDW power parameter (default 2.0 = Shepard's method).
    pub power: f32,
}

impl FluidGradient {
    pub fn new(points: Vec<FluidGradientPoint>) -> Self {
        Self { points, power: 2.0 }
    }

    /// Sample the gradient color at document-space coordinates `(x, y)`.
    pub fn sample_at(&self, x: f64, y: f64) -> [f32; 4] {
        if self.points.is_empty() {
            return [0.5, 0.5, 0.5, 1.0];
        }
        if self.points.len() == 1 {
            let p = &self.points[0];
            return [p.color.r, p.color.g, p.color.b, p.color.a];
        }

        // Check for exact hit (avoid divide-by-zero)
        for p in &self.points {
            let dx = (x - p.x) as f32;
            let dy = (y - p.y) as f32;
            if dx * dx + dy * dy < 1e-8 {
                return [p.color.r, p.color.g, p.color.b, p.color.a];
            }
        }

        // Inverse-distance weighting (Shepard's method)
        let power = self.power;
        let mut weight_sum = 0.0f32;
        let mut cr = 0.0f32;
        let mut cg = 0.0f32;
        let mut cb = 0.0f32;
        let mut ca = 0.0f32;

        for p in &self.points {
            let dx = (x - p.x) as f32;
            let dy = (y - p.y) as f32;
            let dist = (dx * dx + dy * dy).sqrt().max(1e-6);
            let w = 1.0 / dist.powf(power);
            weight_sum += w;
            cr += w * p.color.r;
            cg += w * p.color.g;
            cb += w * p.color.b;
            ca += w * p.color.a;
        }

        [
            cr / weight_sum,
            cg / weight_sum,
            cb / weight_sum,
            ca / weight_sum,
        ]
    }
}

// ─── Mesh (vertex-grid) gradient ─────────────────────────────────────────────

/// A single vertex in a mesh gradient grid.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeshGradientVertex {
    /// Position in document/world space.
    pub x: f64,
    pub y: f64,
    pub color: Color,
}

impl MeshGradientVertex {
    pub fn new(x: f64, y: f64, color: Color) -> Self {
        Self { x, y, color }
    }
}

/// Vertex-based mesh gradient: a rows×cols grid of colored control points,
/// with bilinear interpolation within each cell.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeshGradient {
    pub rows: u32,
    pub cols: u32,
    /// `rows * cols` entries in row-major order.
    pub vertices: Vec<MeshGradientVertex>,
}

impl MeshGradient {
    pub fn new(rows: u32, cols: u32, vertices: Vec<MeshGradientVertex>) -> Self {
        Self {
            rows,
            cols,
            vertices,
        }
    }

    fn vertex_at(&self, row: u32, col: u32) -> Option<&MeshGradientVertex> {
        if row >= self.rows || col >= self.cols {
            return None;
        }
        self.vertices.get((row * self.cols + col) as usize)
    }

    /// Sample the gradient color at document-space coordinates `(x, y)`.
    pub fn sample_at(&self, x: f64, y: f64) -> [f32; 4] {
        if self.vertices.is_empty() {
            return [0.5, 0.5, 0.5, 1.0];
        }
        if self.rows < 2 || self.cols < 2 {
            let v = &self.vertices[0];
            return [v.color.r, v.color.g, v.color.b, v.color.a];
        }

        // Compute bounding box of all vertices to map (x,y) → normalised [0,1]²
        let min_x = self
            .vertices
            .iter()
            .fold(f64::INFINITY, |acc, v| acc.min(v.x));
        let max_x = self
            .vertices
            .iter()
            .fold(f64::NEG_INFINITY, |acc, v| acc.max(v.x));
        let min_y = self
            .vertices
            .iter()
            .fold(f64::INFINITY, |acc, v| acc.min(v.y));
        let max_y = self
            .vertices
            .iter()
            .fold(f64::NEG_INFINITY, |acc, v| acc.max(v.y));

        let w = (max_x - min_x).max(1e-10);
        let h = (max_y - min_y).max(1e-10);

        let u = ((x - min_x) / w).clamp(0.0, 1.0) as f32;
        let v_coord = ((y - min_y) / h).clamp(0.0, 1.0) as f32;

        // Grid cell indices
        let ci = ((u * (self.cols - 1) as f32).floor() as u32).min(self.cols - 2);
        let ri = ((v_coord * (self.rows - 1) as f32).floor() as u32).min(self.rows - 2);

        // Local UV within cell
        let cell_u = u * (self.cols - 1) as f32 - ci as f32;
        let cell_v = v_coord * (self.rows - 1) as f32 - ri as f32;

        // Bilinear interpolation over 4 corners
        let v00 = self.vertex_at(ri, ci).unwrap();
        let v10 = self.vertex_at(ri, ci + 1).unwrap();
        let v01 = self.vertex_at(ri + 1, ci).unwrap();
        let v11 = self.vertex_at(ri + 1, ci + 1).unwrap();

        let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
        let lerp_col = |c0: &Color, c1: &Color, t: f32| -> [f32; 4] {
            [
                lerp(c0.r, c1.r, t),
                lerp(c0.g, c1.g, t),
                lerp(c0.b, c1.b, t),
                lerp(c0.a, c1.a, t),
            ]
        };

        let top = lerp_col(&v00.color, &v10.color, cell_u);
        let bot = lerp_col(&v01.color, &v11.color, cell_u);

        [
            lerp(top[0], bot[0], cell_v),
            lerp(top[1], bot[1], cell_v),
            lerp(top[2], bot[2], cell_v),
            lerp(top[3], bot[3], cell_v),
        ]
    }
}

// ─── Stop interpolation helper ────────────────────────────────────────────────

/// Interpolate a colour from a sorted stop list at parameter `t ∈ [0, 1]`.
pub fn interpolate_stops(stops: &[GradientStop], t: f32) -> [f32; 4] {
    if stops.is_empty() {
        return [0.5, 0.5, 0.5, 1.0];
    }
    if stops.len() == 1 {
        let s = &stops[0];
        return [s.color.r, s.color.g, s.color.b, s.color.a];
    }

    // Before first stop
    if t <= stops[0].offset {
        let s = &stops[0];
        return [s.color.r, s.color.g, s.color.b, s.color.a];
    }
    // After last stop
    let last = &stops[stops.len() - 1];
    if t >= last.offset {
        return [last.color.r, last.color.g, last.color.b, last.color.a];
    }

    // Find surrounding pair
    for i in 0..stops.len() - 1 {
        let s0 = &stops[i];
        let s1 = &stops[i + 1];
        if t >= s0.offset && t <= s1.offset {
            let span = s1.offset - s0.offset;
            let local_t = if span > 1e-6 {
                (t - s0.offset) / span
            } else {
                0.0
            };
            let lerp = |a: f32, b: f32| a + (b - a) * local_t;
            return [
                lerp(s0.color.r, s1.color.r),
                lerp(s0.color.g, s1.color.g),
                lerp(s0.color.b, s1.color.b),
                lerp(s0.color.a, s1.color.a),
            ];
        }
    }

    let s = &stops[stops.len() - 1];
    [s.color.r, s.color.g, s.color.b, s.color.a]
}

#[cfg(test)]
mod pattern_tests {
    use super::*;
    use crate::path::PathData;

    fn rect_shape() -> PathData {
        PathData::rect(0.0, 0.0, 100.0, 100.0)
    }

    #[test]
    fn dots_foreground_is_nonempty_and_clipped() {
        let shape = rect_shape();
        let pf = PatternFill {
            kind: PatternKind::Dots,
            color: Color::BLACK,
            background: None,
            spacing: 20.0,
        };
        let fg = pattern_foreground(&shape, &pf);
        assert!(!fg.is_empty(), "dots foreground should be non-empty");
        let bb = fg.bounding_box().expect("has geometry");
        // Clipped to the shape (allow a hair of boolean-op slack).
        assert!(bb.x0 >= -0.5 && bb.y0 >= -0.5, "bbox underflow: {bb:?}");
        assert!(bb.x1 <= 100.5 && bb.y1 <= 100.5, "bbox overflow: {bb:?}");
    }

    #[test]
    fn each_pattern_kind_produces_geometry() {
        let shape = rect_shape();
        for kind in [
            PatternKind::Dots,
            PatternKind::Stripes,
            PatternKind::Grid,
            PatternKind::Checkerboard,
        ] {
            let pf = PatternFill {
                kind,
                color: Color::BLACK,
                background: None,
                spacing: 25.0,
            };
            assert!(
                !pattern_foreground(&shape, &pf).is_empty(),
                "{kind:?} should produce geometry"
            );
        }
    }

    #[test]
    fn empty_shape_yields_empty_foreground() {
        let pf = PatternFill::default();
        assert!(pattern_foreground(&PathData::new(), &pf).is_empty());
    }

    #[test]
    fn pattern_sample_at_returns_background() {
        let kind = FillKind::Pattern(PatternFill {
            kind: PatternKind::Dots,
            color: Color::BLACK,
            background: Some(Color::new(1.0, 0.0, 0.0, 1.0)),
            spacing: 10.0,
        });
        let c = kind.sample_at(5.0, 5.0, 1.0);
        assert_eq!(
            c,
            [1.0, 0.0, 0.0, 1.0],
            "should sample the background colour"
        );
    }
}
