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
    /// Resolve an object-bounding-box gradient against the filled object's
    /// document-space bbox so it can be sampled at document coordinates. Returns
    /// `self` borrowed for every other fill (no allocation).
    pub fn for_bbox(
        &self,
        min_x: f64,
        min_y: f64,
        w: f64,
        h: f64,
    ) -> std::borrow::Cow<'_, FillKind> {
        match self {
            FillKind::Gradient(g) if g.units.is_object_box() => {
                std::borrow::Cow::Owned(FillKind::Gradient(g.resolved_for_bbox(min_x, min_y, w, h)))
            }
            FillKind::FluidGradient(fg) if fg.units.is_object_box() => {
                std::borrow::Cow::Owned(FillKind::FluidGradient(
                    fg.resolved_for_bbox(min_x, min_y, w, h),
                ))
            }
            FillKind::MeshGradient(mg) if mg.units.is_object_box() => {
                std::borrow::Cow::Owned(FillKind::MeshGradient(
                    mg.resolved_for_bbox(min_x, min_y, w, h),
                ))
            }
            _ => std::borrow::Cow::Borrowed(self),
        }
    }

    /// Whether this fill is a gradient that rotates/shears with the object
    /// (resolved in the object's local space rather than the world AABB).
    pub fn gradient_follows_rotation(&self) -> bool {
        match self {
            FillKind::Gradient(g) => g.units.follows_rotation(),
            FillKind::FluidGradient(fg) => fg.units.follows_rotation(),
            FillKind::MeshGradient(mg) => mg.units.follows_rotation(),
            _ => false,
        }
    }

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
    /// Optional non-solid paint for the stroke (gradient / pattern / etc.). When
    /// `Some`, the stroke geometry is painted with this [`FillKind`] instead of
    /// the flat `color` — e.g. a gradient-stroked line icon. `None` = solid
    /// `color` stroke (the default; back-compatible with older documents).
    #[serde(default)]
    pub paint: Option<FillKind>,
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
            paint: None,
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
            paint: None,
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

/// How a gradient's `coords` are interpreted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GradientUnits {
    /// Coordinates are absolute document/world units. (Back-compat default.)
    #[default]
    UserSpaceOnUse,
    /// Coordinates are `0..1` fractions of the object's axis-aligned bounding
    /// box — the gradient scales & moves with the object but stays axis-aligned.
    ObjectBoundingBox,
    /// Like [`ObjectBoundingBox`](Self::ObjectBoundingBox) but resolved in the
    /// object's *local* space, so the gradient also **rotates/shears** with the
    /// object (true SVG `objectBoundingBox`).
    ObjectBoundingBoxRotated,
}

impl GradientUnits {
    /// Whether coords are `0..1` fractions of the object's bbox (either mode).
    pub fn is_object_box(self) -> bool {
        matches!(self, Self::ObjectBoundingBox | Self::ObjectBoundingBoxRotated)
    }
    /// Whether the gradient rotates/shears with the object (local-space).
    pub fn follows_rotation(self) -> bool {
        matches!(self, Self::ObjectBoundingBoxRotated)
    }
}

/// A linear or radial gradient.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Gradient {
    pub kind: GradientKind,
    pub stops: Vec<GradientStop>,
    /// For linear: [x0, y0, x1, y1]
    /// For radial: [cx, cy, fx, fy, r] (center, focal point, radius)
    /// Interpreted per [`Gradient::units`].
    pub coords: Vec<f64>,
    /// Color space used to blend between stops.
    #[serde(default)]
    pub interpolation: GradientInterpolation,
    /// Coordinate space of `coords`.
    #[serde(default)]
    pub units: GradientUnits,
}

impl Gradient {
    pub fn linear(x0: f64, y0: f64, x1: f64, y1: f64, stops: Vec<GradientStop>) -> Self {
        Self {
            kind: GradientKind::Linear,
            stops,
            coords: vec![x0, y0, x1, y1],
            interpolation: GradientInterpolation::default(),
            units: GradientUnits::default(),
        }
    }

    pub fn radial(cx: f64, cy: f64, r: f64, stops: Vec<GradientStop>) -> Self {
        Self {
            kind: GradientKind::Radial,
            stops,
            coords: vec![cx, cy, cx, cy, r],
            interpolation: GradientInterpolation::default(),
            units: GradientUnits::default(),
        }
    }

    /// Builder: set the coordinate space.
    pub fn with_units(mut self, units: GradientUnits) -> Self {
        self.units = units;
        self
    }

    /// If this gradient is in [`GradientUnits::ObjectBoundingBox`] space, return
    /// an equivalent [`GradientUnits::UserSpaceOnUse`] gradient with `coords`
    /// mapped into the given document-space bbox (so it can be sampled directly
    /// at document coordinates). Returns `self` clone for user-space gradients.
    pub fn resolved_for_bbox(&self, min_x: f64, min_y: f64, w: f64, h: f64) -> Gradient {
        if !self.units.is_object_box() {
            return self.clone();
        }
        let w = if w.abs() < 1e-9 { 1.0 } else { w };
        let h = if h.abs() < 1e-9 { 1.0 } else { h };
        let mut g = self.clone();
        g.units = GradientUnits::UserSpaceOnUse;
        match self.kind {
            GradientKind::Linear if self.coords.len() >= 4 => {
                g.coords = vec![
                    min_x + self.coords[0] * w,
                    min_y + self.coords[1] * h,
                    min_x + self.coords[2] * w,
                    min_y + self.coords[3] * h,
                ];
            }
            GradientKind::Radial if self.coords.len() >= 5 => {
                // Isotropic radius scaled by the larger bbox dimension so the
                // default r=0.5 reaches the object edge.
                let rs = w.abs().max(h.abs());
                g.coords = vec![
                    min_x + self.coords[0] * w,
                    min_y + self.coords[1] * h,
                    min_x + self.coords[2] * w,
                    min_y + self.coords[3] * h,
                    self.coords[4] * rs,
                ];
            }
            _ => {}
        }
        g
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
        interpolate_stops_with(&self.stops, t, self.interpolation)
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
    /// Position (0..1) within the segment to the *next* stop at which the color
    /// reaches the halfway blend — Illustrator's "midpoint". 0.5 = linear.
    #[serde(default = "default_midpoint")]
    pub midpoint: f32,
}

fn default_midpoint() -> f32 {
    0.5
}

impl GradientStop {
    pub fn new(offset: f32, color: Color) -> Self {
        Self {
            offset,
            color,
            midpoint: 0.5,
        }
    }

    /// Builder: set the transition midpoint to the next stop (0..1).
    pub fn with_midpoint(mut self, midpoint: f32) -> Self {
        self.midpoint = midpoint.clamp(0.0, 1.0);
        self
    }
}

/// Color space used to interpolate between a gradient's stops.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GradientInterpolation {
    /// Straight per-channel blend in gamma sRGB (classic; can look muddy).
    #[default]
    Srgb,
    /// Perceptually-uniform blend in OKLab (avoids the desaturated "gray band").
    Oklab,
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
    /// Coordinate space of the point positions.
    #[serde(default)]
    pub units: GradientUnits,
}

impl FluidGradient {
    pub fn new(points: Vec<FluidGradientPoint>) -> Self {
        Self {
            points,
            power: 2.0,
            units: GradientUnits::default(),
        }
    }

    /// Builder: set the coordinate space.
    pub fn with_units(mut self, units: GradientUnits) -> Self {
        self.units = units;
        self
    }

    /// Resolve object-bounding-box point coords into the given document bbox.
    pub fn resolved_for_bbox(&self, min_x: f64, min_y: f64, w: f64, h: f64) -> FluidGradient {
        if !self.units.is_object_box() {
            return self.clone();
        }
        let w = if w.abs() < 1e-9 { 1.0 } else { w };
        let h = if h.abs() < 1e-9 { 1.0 } else { h };
        let mut g = self.clone();
        g.units = GradientUnits::UserSpaceOnUse;
        for p in &mut g.points {
            p.x = min_x + p.x * w;
            p.y = min_y + p.y * h;
        }
        g
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
    /// Coordinate space of the vertex positions.
    #[serde(default)]
    pub units: GradientUnits,
}

impl MeshGradient {
    pub fn new(rows: u32, cols: u32, vertices: Vec<MeshGradientVertex>) -> Self {
        Self {
            rows,
            cols,
            vertices,
            units: GradientUnits::default(),
        }
    }

    /// Builder: set the coordinate space.
    pub fn with_units(mut self, units: GradientUnits) -> Self {
        self.units = units;
        self
    }

    /// Resolve object-bounding-box vertex coords into the given document bbox.
    pub fn resolved_for_bbox(&self, min_x: f64, min_y: f64, w: f64, h: f64) -> MeshGradient {
        if !self.units.is_object_box() {
            return self.clone();
        }
        let w = if w.abs() < 1e-9 { 1.0 } else { w };
        let h = if h.abs() < 1e-9 { 1.0 } else { h };
        let mut g = self.clone();
        g.units = GradientUnits::UserSpaceOnUse;
        for v in &mut g.vertices {
            v.x = min_x + v.x * w;
            v.y = min_y + v.y * h;
        }
        g
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
/// Sample stops at `t` in gamma sRGB (back-compat; honours per-stop midpoints).
pub fn interpolate_stops(stops: &[GradientStop], t: f32) -> [f32; 4] {
    interpolate_stops_with(stops, t, GradientInterpolation::Srgb)
}

/// Sample stops at `t`, honouring each stop's midpoint and blending in the
/// requested color space.
pub fn interpolate_stops_with(
    stops: &[GradientStop],
    t: f32,
    interp: GradientInterpolation,
) -> [f32; 4] {
    let endpoint = |s: &GradientStop| [s.color.r, s.color.g, s.color.b, s.color.a];
    if stops.is_empty() {
        return [0.5, 0.5, 0.5, 1.0];
    }
    if stops.len() == 1 || t <= stops[0].offset {
        return endpoint(&stops[0]);
    }
    let last = &stops[stops.len() - 1];
    if t >= last.offset {
        return endpoint(last);
    }

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
            // Remap by the segment midpoint so the halfway blend lands at
            // `s0.midpoint` within the segment (0.5 = linear).
            let m = s0.midpoint.clamp(0.02, 0.98);
            let tt = if local_t <= m {
                0.5 * local_t / m
            } else {
                0.5 + 0.5 * (local_t - m) / (1.0 - m)
            };
            let a = endpoint(s0);
            let b = endpoint(s1);
            return match interp {
                GradientInterpolation::Srgb => lerp_rgba(a, b, tt),
                GradientInterpolation::Oklab => {
                    let la = srgb_to_oklab(a);
                    let lb = srgb_to_oklab(b);
                    let mixed = lerp_rgba(la, lb, tt);
                    oklab_to_srgb(mixed)
                }
            };
        }
    }
    endpoint(&stops[stops.len() - 1])
}

fn lerp_rgba(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
        a[3] + (b[3] - a[3]) * t,
    ]
}

// ── OKLab helpers for perceptual gradient interpolation (Björn Ottosson) ──
// Operate on gamma sRGB `[f32; 4]`; the L/a/b triple is carried in slots 0..3
// with alpha preserved in slot 3 (linear).

fn srgb_gamma_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}
fn srgb_linear_to_gamma(c: f32) -> f32 {
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

fn srgb_to_oklab(rgba: [f32; 4]) -> [f32; 4] {
    let r = srgb_gamma_to_linear(rgba[0]);
    let g = srgb_gamma_to_linear(rgba[1]);
    let b = srgb_gamma_to_linear(rgba[2]);
    let l = 0.412_221_47 * r + 0.536_332_54 * g + 0.051_445_995 * b;
    let m = 0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b;
    let s = 0.088_302_46 * r + 0.281_718_84 * g + 0.629_978_7 * b;
    let l_ = l.cbrt();
    let m_ = m.cbrt();
    let s_ = s.cbrt();
    [
        0.210_454_26 * l_ + 0.793_617_8 * m_ - 0.004_072_047 * s_,
        1.977_998_5 * l_ - 2.428_592_2 * m_ + 0.450_593_7 * s_,
        0.025_904_037 * l_ + 0.782_771_77 * m_ - 0.808_675_77 * s_,
        rgba[3],
    ]
}

fn oklab_to_srgb(lab: [f32; 4]) -> [f32; 4] {
    let l_ = lab[0] + 0.396_337_78 * lab[1] + 0.215_803_76 * lab[2];
    let m_ = lab[0] - 0.105_561_346 * lab[1] - 0.063_854_17 * lab[2];
    let s_ = lab[0] - 0.089_484_18 * lab[1] - 1.291_485_5 * lab[2];
    let l = l_ * l_ * l_;
    let m = m_ * m_ * m_;
    let s = s_ * s_ * s_;
    let r = 4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s;
    let g = -1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s;
    let b = -0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s;
    [
        srgb_linear_to_gamma(r.clamp(0.0, 1.0)),
        srgb_linear_to_gamma(g.clamp(0.0, 1.0)),
        srgb_linear_to_gamma(b.clamp(0.0, 1.0)),
        lab[3],
    ]
}

#[cfg(test)]
mod gradient_interp_tests {
    use super::*;

    fn stops() -> Vec<GradientStop> {
        vec![
            GradientStop::new(0.0, Color::new(1.0, 0.0, 0.0, 1.0)),
            GradientStop::new(1.0, Color::new(0.0, 0.0, 1.0, 1.0)),
        ]
    }

    #[test]
    fn midpoint_half_is_linear() {
        // Default midpoint 0.5 → exact channel average at t=0.5.
        let c = interpolate_stops(&stops(), 0.5);
        assert!((c[0] - 0.5).abs() < 1e-4 && (c[2] - 0.5).abs() < 1e-4);
    }

    #[test]
    fn midpoint_shifts_halfway_point() {
        // Midpoint at 0.25 → the halfway color arrives earlier (t=0.25).
        let mut s = stops();
        s[0].midpoint = 0.25;
        let c = interpolate_stops(&s, 0.25);
        assert!((c[0] - 0.5).abs() < 1e-3, "expected half-red at t=0.25, got {c:?}");
    }

    #[test]
    fn oklab_midpoint_avoids_srgb_average() {
        // At t=0.5 the OKLab blend of red↔blue differs from the sRGB average
        // (the perceptual path does not pass through the same muddy midpoint).
        let s = stops();
        let srgb = interpolate_stops_with(&s, 0.5, GradientInterpolation::Srgb);
        let oklab = interpolate_stops_with(&s, 0.5, GradientInterpolation::Oklab);
        let diff: f32 = (0..3).map(|i| (srgb[i] - oklab[i]).abs()).sum();
        assert!(diff > 0.05, "oklab should differ from srgb midpoint: {srgb:?} vs {oklab:?}");
    }

    #[test]
    fn oklab_endpoints_are_exact() {
        let s = stops();
        let a = interpolate_stops_with(&s, 0.0, GradientInterpolation::Oklab);
        let b = interpolate_stops_with(&s, 1.0, GradientInterpolation::Oklab);
        assert!((a[0] - 1.0).abs() < 1e-3 && a[2].abs() < 1e-3);
        assert!(b[0].abs() < 1e-3 && (b[2] - 1.0).abs() < 1e-3);
    }

    #[test]
    fn legacy_stop_deserializes_with_default_midpoint() {
        let json = r#"{"offset":0.3,"color":{"r":1.0,"g":0.0,"b":0.0,"a":1.0}}"#;
        let s: GradientStop = serde_json::from_str(json).unwrap();
        assert_eq!(s.midpoint, 0.5);
    }

    #[test]
    fn object_bbox_gradient_resolves_into_bbox() {
        // A 0..1 linear gradient resolved into a bbox at (100,200) size 50×80.
        let g = Gradient::linear(0.0, 0.0, 1.0, 0.0, stops())
            .with_units(GradientUnits::ObjectBoundingBox);
        let r = g.resolved_for_bbox(100.0, 200.0, 50.0, 80.0);
        assert_eq!(r.units, GradientUnits::UserSpaceOnUse);
        assert_eq!(r.coords, vec![100.0, 200.0, 150.0, 200.0]);
        // Sampling the resolved gradient in document space hits the endpoints.
        let a = r.sample_at(100.0, 200.0);
        let b = r.sample_at(150.0, 200.0);
        assert!((a[0] - 1.0).abs() < 1e-3 && a[2] < 1e-3); // red
        assert!(b[2] > 0.99 && b[0] < 1e-3); // blue
    }

    #[test]
    fn user_space_gradient_unchanged_by_resolve() {
        let g = Gradient::linear(10.0, 0.0, 90.0, 0.0, stops());
        assert_eq!(g.resolved_for_bbox(0.0, 0.0, 5.0, 5.0).coords, g.coords);
    }

    #[test]
    fn legacy_gradient_defaults_to_user_space() {
        let json = r#"{"kind":"linear","stops":[],"coords":[0.0,0.0,1.0,0.0]}"#;
        let g: Gradient = serde_json::from_str(json).unwrap();
        assert_eq!(g.units, GradientUnits::UserSpaceOnUse);
    }
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
