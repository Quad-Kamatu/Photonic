use crate::color::Color;
use crate::raster::image::RasterImage;
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

    pub fn pattern(pattern: PatternFill) -> Self {
        Self {
            kind: FillKind::Pattern(pattern),
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
    /// A tiled raster pattern fill. Self-contained: carries its own RGBA tile and
    /// an independent pattern transform applied in document space.
    Pattern(PatternFill),
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
            FillKind::Pattern(p) => {
                let [r, g2, b, a] = p.sample_at(x, y);
                [r, g2, b, a * opacity]
            }
        }
    }
}

// ─── Pattern fill (tiled raster) ─────────────────────────────────────────────

/// Tile layout for a [`PatternFill`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PatternTileType {
    /// Tiles aligned on a regular grid.
    #[default]
    Grid,
    /// Alternate rows shifted by half a tile width (running-bond brick).
    BrickByRow,
    /// Alternate columns shifted by half a tile height (stack-bond brick).
    BrickByColumn,
    /// Hexagonal-style staggered rows. With a rectangular raster tile this is a
    /// half-offset row stagger; true hex-cell clipping is a future refinement.
    Hex,
}

impl PatternTileType {
    /// Parse from a snake_case label (as used by the MCP/GUI layer).
    pub fn from_label(s: &str) -> Option<Self> {
        match s {
            "grid" => Some(Self::Grid),
            "brick_by_row" => Some(Self::BrickByRow),
            "brick_by_column" => Some(Self::BrickByColumn),
            "hex" => Some(Self::Hex),
            _ => None,
        }
    }

    /// The snake_case label for this layout.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Grid => "grid",
            Self::BrickByRow => "brick_by_row",
            Self::BrickByColumn => "brick_by_column",
            Self::Hex => "hex",
        }
    }
}

/// A tiled raster pattern fill. The `tile` pixels are embedded in the fill so the
/// pattern renders identically on every path (canvas, headless, GPU CPU-sample)
/// with no document-registry lookup at render time — exactly like a gradient
/// carries its own stops.
///
/// The pattern transform (`scale`, `rotation`, `offset`) is applied in document
/// space and is *independent* of the owning node's transform: a pattern stays
/// pinned to document space, so translating the shape scrolls the artwork
/// underneath the pattern rather than dragging the pattern with it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PatternFill {
    /// Embedded RGBA8 tile (self-contained — serialized as a base64 PNG).
    pub tile: RasterImage,
    /// Tile layout.
    #[serde(default)]
    pub tile_type: PatternTileType,
    /// Uniform scale of the pattern (1.0 = tile pixels map 1:1 to document units).
    pub scale: f64,
    /// Rotation of the pattern in radians (document space).
    pub rotation: f64,
    /// Document-space anchor / translation of the pattern origin.
    pub offset: [f64; 2],
    /// Gutter between tiles, in tile pixels. The gutter samples as transparent.
    pub spacing: f64,
}

impl PatternFill {
    /// Build a pattern from a tile with an identity transform and grid layout.
    pub fn new(tile: RasterImage) -> Self {
        Self {
            tile,
            tile_type: PatternTileType::Grid,
            scale: 1.0,
            rotation: 0.0,
            offset: [0.0, 0.0],
            spacing: 0.0,
        }
    }

    /// Sample the pattern color at document-space coordinates `(x, y)`.
    ///
    /// Maps `(x, y)` through the inverse pattern transform into tile space,
    /// applies the layout row/column shift, wraps into the
    /// `[0, tile_w + spacing) × [0, tile_h + spacing)` period, and bilinearly
    /// samples the tile. Coordinates that land in the inter-tile gutter return
    /// transparent.
    pub fn sample_at(&self, x: f64, y: f64) -> [f32; 4] {
        let tw = self.tile.width as f64;
        let th = self.tile.height as f64;
        if tw <= 0.0 || th <= 0.0 {
            return [0.0; 4];
        }

        // 1. Inverse translate.
        let px = x - self.offset[0];
        let py = y - self.offset[1];

        // 2. Inverse rotation (R(-θ)).
        let (sin, cos) = self.rotation.sin_cos();
        let rx = px * cos + py * sin;
        let ry = -px * sin + py * cos;

        // 3. Inverse scale → tile-space coordinates (units = tile pixels).
        let s = if self.scale.abs() < 1e-9 {
            1.0
        } else {
            self.scale
        };
        let tx = rx / s;
        let ty = ry / s;

        // Period (tile + gutter) in tile pixels.
        let pw = tw + self.spacing.max(0.0);
        let ph = th + self.spacing.max(0.0);

        // 4. Layout shift.
        let (mut sx, mut sy) = (tx, ty);
        match self.tile_type {
            PatternTileType::Grid => {}
            PatternTileType::BrickByRow | PatternTileType::Hex => {
                let row = (ty / ph).floor() as i64;
                if row.rem_euclid(2) == 1 {
                    sx += pw * 0.5;
                }
            }
            PatternTileType::BrickByColumn => {
                let col = (tx / pw).floor() as i64;
                if col.rem_euclid(2) == 1 {
                    sy += ph * 0.5;
                }
            }
        }

        // 5. Wrap into the tile period.
        let u = sx.rem_euclid(pw);
        let v = sy.rem_euclid(ph);

        // Gutter → transparent.
        if u >= tw || v >= th {
            return [0.0; 4];
        }

        let texel = self.tile.sample_bilinear(u as f32, v as f32);
        [
            texel[0] as f32 / 255.0,
            texel[1] as f32 / 255.0,
            texel[2] as f32 / 255.0,
            texel[3] as f32 / 255.0,
        ]
    }
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
    use crate::raster::image::RasterImage;

    /// A 2×2 tile: red, green / blue, white (RGBA8, opaque).
    fn quad_tile() -> RasterImage {
        let pixels = vec![
            255, 0, 0, 255, // (0,0) red
            0, 255, 0, 255, // (1,0) green
            0, 0, 255, 255, // (0,1) blue
            255, 255, 255, 255, // (1,1) white
        ];
        RasterImage::from_rgba(2, 2, pixels).unwrap()
    }

    #[test]
    fn samples_tile_at_origin() {
        let p = PatternFill::new(quad_tile());
        // Bilinear sampling hits texel centers at integer tile coords: (0,0) → red.
        let c = p.sample_at(0.0, 0.0);
        assert!(c[0] > 0.9 && c[1] < 0.1 && c[2] < 0.1, "got {:?}", c);
    }

    #[test]
    fn tiling_is_periodic() {
        let p = PatternFill::new(quad_tile());
        // Period is the 2px tile width; +2 in x lands on the same texel.
        let a = p.sample_at(0.0, 0.0);
        let b = p.sample_at(2.0, 0.0);
        assert_eq!(a, b);
    }

    #[test]
    fn pattern_does_not_move_with_whole_tile_translation() {
        // The fill itself is fixed in document space: sampling the same doc point
        // always yields the same texel regardless of the owning shape's position,
        // and a whole-period offset reproduces the original.
        let p = PatternFill::new(quad_tile());
        let base = p.sample_at(0.0, 1.0); // texel (0,1) → blue
        assert!(
            base[2] > 0.9 && base[0] < 0.1,
            "expected blue, got {:?}",
            base
        );
        let shifted = p.sample_at(0.0 + 2.0, 1.0 + 2.0);
        assert_eq!(base, shifted);
    }

    #[test]
    fn spacing_gutter_is_transparent() {
        let mut p = PatternFill::new(quad_tile());
        p.spacing = 2.0; // period becomes 4px; tile occupies [0,2), gutter [2,4)
        let gutter = p.sample_at(3.0, 0.5);
        assert_eq!(
            gutter[3], 0.0,
            "gutter should be transparent, got {:?}",
            gutter
        );
        let inside = p.sample_at(0.5, 0.5);
        assert!(inside[3] > 0.9);
    }

    #[test]
    fn brick_by_row_shifts_odd_rows() {
        let p = PatternFill {
            tile_type: PatternTileType::BrickByRow,
            ..PatternFill::new(quad_tile())
        };
        // Row 0 (y in [0,2)): no shift → x=0.5 is texel (0,0) red.
        let row0 = p.sample_at(0.5, 0.5);
        // Row 1 (y in [2,4)): shifted by +1px in x → x=0.5 maps to wrapped 1.5 = texel (1, _).
        let row1 = p.sample_at(0.5, 2.5);
        assert_ne!(row0, row1, "odd row should be horizontally offset");
    }

    #[test]
    fn rotation_maps_axis() {
        // 90° rotation: a point on the +x doc axis maps onto the tile's y axis.
        let mut p = PatternFill::new(quad_tile());
        p.rotation = std::f64::consts::FRAC_PI_2;
        // Sanity: sampling stays within the tile and returns an opaque texel.
        let c = p.sample_at(1.5, 0.5);
        assert!(c[3] > 0.9, "rotated sample should be opaque, got {:?}", c);
    }

    #[test]
    fn roundtrips_through_fillkind_serde() {
        let fill = Fill::pattern(PatternFill {
            tile_type: PatternTileType::Hex,
            scale: 2.0,
            rotation: 0.5,
            offset: [10.0, 20.0],
            spacing: 1.0,
            ..PatternFill::new(quad_tile())
        });
        let json = serde_json::to_string(&fill).unwrap();
        assert!(json.contains("\"type\":\"pattern\""), "json: {}", json);
        let back: Fill = serde_json::from_str(&json).unwrap();
        match back.kind {
            FillKind::Pattern(p) => {
                assert_eq!(p.tile_type, PatternTileType::Hex);
                assert_eq!(p.scale, 2.0);
                assert_eq!(p.offset, [10.0, 20.0]);
                assert_eq!(p.tile.width, 2);
            }
            other => panic!("expected pattern, got {:?}", other),
        }
    }
}
