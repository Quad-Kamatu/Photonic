use crate::{
    color::Color,
    layer::{BlendMode, LayerId},
    path::PathData,
    raster::{adjust::AdjustmentSpec, image::RasterImage, mask::Mask},
    style::{Fill, LineJoin, Stroke},
    transform::Transform,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub type NodeId = Uuid;

// ── Asset Export Spec ─────────────────────────────────────────────────────────

/// Per-node export specification — equivalent to Illustrator's Asset Export panel.
/// When set, the node is included in batch asset exports.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetExportSpec {
    /// Base name used for the exported file (without extension or scale suffix).
    pub name: String,
    /// Export format: "svg", "png", "jpeg", or "webp".
    #[serde(default)]
    pub format: String,
    /// Scale multipliers for raster exports (e.g. [1.0, 2.0] → "name.png", "name@2x.png").
    /// Ignored for SVG exports.
    #[serde(default)]
    pub scales: Vec<f64>,
}

fn is_none_export_spec(v: &Option<AssetExportSpec>) -> bool {
    v.is_none()
}

// ── Glow effects ──────────────────────────────────────────────────────────────

/// A soft glow effect (outer or inner) applied to a node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GlowEffect {
    pub color: Color,
    pub opacity: f32,
    /// Spread radius in document units.
    pub size: f32,
    pub enabled: bool,
    /// Corner style used when rendering glow strokes.
    #[serde(default)]
    pub join: LineJoin,
}

impl Default for GlowEffect {
    fn default() -> Self {
        Self {
            color: Color {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 1.0,
            },
            opacity: 0.75,
            size: 10.0,
            enabled: false,
            join: LineJoin::Miter,
        }
    }
}

fn glow_is_disabled(g: &GlowEffect) -> bool {
    !g.enabled
}

// ── GaussianGlow ──────────────────────────────────────────────────────────────

/// A Gaussian-blur-based glow rendered via a GPU separable blur pass.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GaussianGlow {
    pub color: Color,
    pub opacity: f32,
    /// Blur radius (sigma) in document units.
    pub radius: f32,
    pub enabled: bool,
}

impl Default for GaussianGlow {
    fn default() -> Self {
        Self {
            color: Color {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 1.0,
            },
            opacity: 0.75,
            radius: 10.0,
            enabled: false,
        }
    }
}

fn gaussian_glow_disabled(g: &GaussianGlow) -> bool {
    !g.enabled
}

// ── DropShadow ────────────────────────────────────────────────────────────────

/// A drop shadow: an offset, Gaussian-blurred silhouette composited *beneath*
/// the object. Reuses the separable-blur pass that powers `GaussianGlow`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DropShadow {
    pub color: Color,
    pub opacity: f32,
    /// Horizontal offset in document units (positive = right).
    pub dx: f32,
    /// Vertical offset in document units (positive = down).
    pub dy: f32,
    /// Blur radius (sigma) in document units. 0 = hard-edged shadow.
    pub blur: f32,
    pub enabled: bool,
}

impl Default for DropShadow {
    fn default() -> Self {
        Self {
            color: Color {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 1.0,
            },
            opacity: 0.5,
            dx: 4.0,
            dy: 4.0,
            blur: 6.0,
            enabled: false,
        }
    }
}

fn drop_shadow_disabled(s: &DropShadow) -> bool {
    !s.enabled
}

// ── ObjectBlur ────────────────────────────────────────────────────────────────

/// A standalone Gaussian blur applied to the object itself (as opposed to a glow
/// halo). For solid fills this softens the whole silhouette edge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObjectBlur {
    /// Blur radius (sigma) in document units.
    pub radius: f32,
    pub enabled: bool,
}

impl Default for ObjectBlur {
    fn default() -> Self {
        Self {
            radius: 4.0,
            enabled: false,
        }
    }
}

fn object_blur_disabled(b: &ObjectBlur) -> bool {
    !b.enabled
}

// ── Feather ───────────────────────────────────────────────────────────────────

/// Feather: soft-fade the object's alpha boundary by a given radius.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Feather {
    /// Feather radius in document units.
    pub radius: f32,
    pub enabled: bool,
}

impl Default for Feather {
    fn default() -> Self {
        Self {
            radius: 4.0,
            enabled: false,
        }
    }
}

fn feather_disabled(f: &Feather) -> bool {
    !f.enabled
}

// ── serde skip helpers ────────────────────────────────────────────────────────

fn is_one_f32(v: &f32) -> bool {
    (*v - 1.0).abs() < 1e-6
}
fn is_true(v: &bool) -> bool {
    *v
}
fn is_false(v: &bool) -> bool {
    !*v
}
fn is_empty_vec<T>(v: &[T]) -> bool {
    v.is_empty()
}
fn is_normal_blend(v: &BlendMode) -> bool {
    *v == BlendMode::Normal
}
fn is_identity_transform(v: &Transform) -> bool {
    *v == Transform::IDENTITY
}
fn default_true() -> bool {
    true
}
fn default_one_f32() -> f32 {
    1.0
}

/// A node in the scene graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneNode {
    pub id: NodeId,
    pub name: String,
    pub layer_id: LayerId,
    pub kind: SceneNodeKind,
    #[serde(default, skip_serializing_if = "is_identity_transform")]
    pub transform: Transform,
    #[serde(default = "default_one_f32", skip_serializing_if = "is_one_f32")]
    pub opacity: f32,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub visible: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub locked: bool,
    #[serde(default, skip_serializing_if = "is_normal_blend")]
    pub blend_mode: BlendMode,
    /// Optional semantic tags for AI agent queries (e.g. "background", "logo-mark")
    #[serde(default, skip_serializing_if = "is_empty_vec")]
    pub tags: Vec<String>,
    /// Chronological log of AI prompts that created or modified this node.
    /// Stored in the document; stripped from all export formats.
    #[serde(default, skip_serializing_if = "is_empty_vec")]
    pub prompt_history: Vec<String>,
    #[serde(default, skip_serializing_if = "glow_is_disabled")]
    pub outer_glow: GlowEffect,
    #[serde(default, skip_serializing_if = "glow_is_disabled")]
    pub inner_glow: GlowEffect,
    #[serde(default, skip_serializing_if = "gaussian_glow_disabled")]
    pub gaussian_glow: GaussianGlow,
    #[serde(default, skip_serializing_if = "drop_shadow_disabled")]
    pub drop_shadow: DropShadow,
    #[serde(default, skip_serializing_if = "object_blur_disabled")]
    pub object_blur: ObjectBlur,
    #[serde(default, skip_serializing_if = "feather_disabled")]
    pub feather: Feather,
    /// Optional per-asset export specification (Asset Export panel equivalent).
    #[serde(default, skip_serializing_if = "is_none_export_spec")]
    pub export_spec: Option<AssetExportSpec>,
    /// When set, this node is an instance of the named symbol (Symbols Panel).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol_ref: Option<uuid::Uuid>,
    /// Dynamic symbol fill override: hex color string replacing the master's fill for this instance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol_fill_override: Option<String>,
    /// Dynamic symbol stroke override: hex color string replacing the master's stroke for this instance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol_stroke_override: Option<String>,
}

impl SceneNode {
    /// Rough in-memory footprint of this node in bytes — dominated by raster
    /// pixel + mask buffers. Non-raster nodes are tiny by comparison, so a small
    /// constant suffices. Used to bound the *memory* cost of edit history (#194),
    /// which the compressed serialized size (base64 PNG) badly underestimates.
    pub fn mem_estimate(&self) -> u64 {
        const BASE: u64 = 256;
        match &self.kind {
            SceneNodeKind::Raster(r) => {
                BASE + r.image.pixels.len() as u64
                    + r.mask.as_ref().map_or(0, |m| m.data.len() as u64)
            }
            _ => BASE,
        }
    }

    pub fn new(name: impl Into<String>, layer_id: LayerId, kind: SceneNodeKind) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            layer_id,
            kind,
            transform: Transform::IDENTITY,
            opacity: 1.0,
            visible: true,
            locked: false,
            blend_mode: BlendMode::Normal,
            tags: vec![],
            prompt_history: vec![],
            outer_glow: GlowEffect::default(),
            inner_glow: GlowEffect::default(),
            gaussian_glow: GaussianGlow::default(),
            drop_shadow: DropShadow::default(),
            object_blur: ObjectBlur::default(),
            feather: Feather::default(),
            export_spec: None,
            symbol_ref: None,
            symbol_fill_override: None,
            symbol_stroke_override: None,
        }
    }

    pub fn with_transform(mut self, t: Transform) -> Self {
        self.transform = t;
        self
    }

    /// Crop this raster node's pixels — and its layer mask, in lockstep — to
    /// the axis-aligned canvas-space rectangle `(x0, y0)..(x1, y1)` (e.g. the
    /// artboard bounds), adjusting the transform so the surviving pixels stay
    /// exactly where they were on canvas.
    ///
    /// Destructive by design (pixels outside the rect are discarded), unlike
    /// the layer-mask operations above. Requires an axis-aligned, non-mirrored
    /// transform (pure translation + positive scale); rotated or flipped
    /// rasters return `Err` rather than silently resampling.
    pub fn crop_raster_to_rect(
        &mut self,
        x0: f64,
        y0: f64,
        x1: f64,
        y1: f64,
    ) -> Result<(), String> {
        let SceneNodeKind::Raster(rn) = &mut self.kind else {
            return Err("not a raster node".to_string());
        };
        if rn.is_adjustment_layer() {
            return Err("cannot crop an adjustment layer".to_string());
        }
        let [a, b, c, d, tx, ty] = self.transform.matrix;
        if b.abs() > 1e-9 || c.abs() > 1e-9 {
            return Err("crop requires an unrotated image (no rotation/shear)".to_string());
        }
        if a <= 0.0 || d <= 0.0 {
            return Err("crop requires a non-mirrored image (positive scale)".to_string());
        }

        // Image extent in canvas space.
        let (w, h) = (rn.image.width as f64, rn.image.height as f64);
        let (ix0, iy0) = (tx, ty);
        let (ix1, iy1) = (tx + a * w, ty + d * h);

        // Intersection with the crop rect.
        let cx0 = ix0.max(x0.min(x1));
        let cy0 = iy0.max(y0.min(y1));
        let cx1 = ix1.min(x0.max(x1));
        let cy1 = iy1.min(y0.max(y1));
        if cx1 <= cx0 || cy1 <= cy0 {
            return Err("image does not overlap the crop area".to_string());
        }

        // Canvas → local pixels. Outward-rounded so the crop never cuts a
        // partially-inside pixel (the artboard edge lands inside the kept run).
        let px0 = (((cx0 - tx) / a).floor().max(0.0)) as i64;
        let py0 = (((cy0 - ty) / d).floor().max(0.0)) as i64;
        let px1 = (((cx1 - tx) / a).ceil().min(w)) as i64;
        let py1 = (((cy1 - ty) / d).ceil().min(h)) as i64;
        let cw = (px1 - px0).max(1) as u32;
        let ch = (py1 - py0).max(1) as u32;
        if px0 == 0 && py0 == 0 && cw == rn.image.width && ch == rn.image.height {
            return Ok(()); // fully inside — nothing to trim
        }

        rn.image = crate::raster::geometry::crop(&rn.image, px0, py0, cw, ch);
        // The layer mask must be cut identically or every mask op afterwards
        // sees a size mismatch.
        if let Some(m) = &rn.mask {
            rn.mask = Some(m.crop(px0, py0, cw, ch));
        }
        // Keep the surviving pixels where they were: origin moves by the
        // trimmed-off left/top run, scaled into canvas units.
        self.transform.matrix[4] = tx + a * px0 as f64;
        self.transform.matrix[5] = ty + d * py0 as f64;
        Ok(())
    }

    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }

    /// Returns the bounding box of this node in local coordinates (before transform).
    pub fn local_bounds(&self) -> Option<kurbo::Rect> {
        match &self.kind {
            SceneNodeKind::Path(p) => p.path_data.bounding_box(),
            SceneNodeKind::Group(_g) => {
                // For now return None; will be computed by traversing children
                None
            }
            SceneNodeKind::Text(t) => {
                // Approximate bounds in local space. The renderer places text
                // at (0,0) in local space, so estimate width from char count
                // and height from line count × line-height.
                let line_count = t.content.lines().count().max(1);
                let max_chars = t
                    .content
                    .lines()
                    .map(|l| l.chars().count())
                    .max()
                    .unwrap_or(1)
                    .max(1);
                let width = t.font_size * 0.55 * max_chars as f64;
                let height = t.font_size * 1.2 * line_count as f64;
                Some(kurbo::Rect::new(0.0, 0.0, width, height))
            }
            SceneNodeKind::Raster(r) => Some(kurbo::Rect::new(
                0.0,
                0.0,
                r.image.width as f64,
                r.image.height as f64,
            )),
        }
    }
}

/// The type-specific data of a scene node.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SceneNodeKind {
    Path(PathNode),
    Group(GroupNode),
    Text(TextNode),
    Raster(RasterNode),
}

/// A vector path node — the fundamental building block.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathNode {
    pub path_data: PathData,
    pub fill: Fill,
    pub stroke: Stroke,
    /// If true, this is a compound path (multiple subpaths treated as one shape)
    #[serde(default)]
    pub is_compound: bool,
}

impl PathNode {
    pub fn new(path_data: PathData) -> Self {
        Self {
            path_data,
            fill: Fill::solid(crate::color::Color::BLACK),
            stroke: Stroke::none(),
            is_compound: false,
        }
    }

    pub fn with_fill(mut self, fill: Fill) -> Self {
        self.fill = fill;
        self
    }

    pub fn with_stroke(mut self, stroke: Stroke) -> Self {
        self.stroke = stroke;
        self
    }
}

/// A raster (pixel) node — a Photoshop-style bitmap layer.
///
/// The image occupies local rect `[0,0,width,height]`; the node `transform`
/// places, scales, and rotates it in document space exactly like a path's
/// geometry. An optional non-destructive layer `mask` gates its compositing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RasterNode {
    /// The pixel buffer (RGBA8, straight alpha).
    pub image: RasterImage,
    /// Optional non-destructive layer mask (8-bit coverage).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask: Option<Mask>,
    /// Original source file path, for relink / re-export.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_uri: Option<String>,
    /// When set, this node is a **non-destructive adjustment layer**: it carries
    /// no own pixels to composite; instead its `adjustment` is re-applied to the
    /// composite of everything beneath it (within its `mask`) on every render.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adjustment: Option<AdjustmentSpec>,
}

impl RasterNode {
    /// True if this node is a non-destructive adjustment layer.
    pub fn is_adjustment_layer(&self) -> bool {
        self.adjustment.is_some()
    }
}

impl RasterNode {
    pub fn new(image: RasterImage) -> Self {
        Self {
            image,
            mask: None,
            source_uri: None,
            adjustment: None,
        }
    }

    /// Build a non-destructive adjustment layer (no own pixels; `image` is a 1×1
    /// placeholder). The `adjustment` is applied to the composite beneath it.
    pub fn adjustment_layer(spec: AdjustmentSpec) -> Self {
        Self {
            image: RasterImage::new(1, 1),
            mask: None,
            source_uri: None,
            adjustment: Some(spec),
        }
    }

    pub fn with_source(mut self, uri: impl Into<String>) -> Self {
        self.source_uri = Some(uri.into());
        self
    }

    pub fn width(&self) -> u32 {
        self.image.width
    }

    pub fn height(&self) -> u32 {
        self.image.height
    }

    /// Non-destructively **hide** the pixels covered by `sel` (a selection such as
    /// a color-range or magic-wand mask) by subtracting it from the layer mask.
    ///
    /// This is the reversible equivalent of Photoshop's "delete a color range to
    /// transparency": the pixels stay in `image`, but the layer mask (which the
    /// compositor multiplies into source alpha) drops to `0` wherever `sel` is
    /// selected. Partial selection coverage yields partial hiding, so anti-aliased
    /// selection edges soften cleanly. Repeated calls accumulate (each subtracts
    /// more). A size mismatch is a no-op.
    pub fn hide_selection(&mut self, sel: &Mask) {
        if sel.width != self.image.width || sel.height != self.image.height {
            return;
        }
        let mut m = self
            .mask
            .take()
            .unwrap_or_else(|| Mask::full(self.image.width, self.image.height));
        m.subtract(sel);
        self.mask = Some(m);
    }

    /// Set the layer mask to a **foreground matte** — coverage `255` where the
    /// subject is, `0` for background — such as the alpha matte returned by a
    /// background-removal model. If a mask already exists it is intersected with
    /// the matte (so a prior hand mask is never widened), otherwise the matte
    /// becomes the mask directly. A size mismatch is a no-op.
    pub fn set_foreground_matte(&mut self, matte: Mask) {
        if matte.width != self.image.width || matte.height != self.image.height {
            return;
        }
        match self.mask.take() {
            Some(mut existing) => {
                existing.intersect(&matte);
                self.mask = Some(existing);
            }
            None => self.mask = Some(matte),
        }
    }
}

/// A group node — contains ordered child node IDs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupNode {
    /// Ordered child node IDs (bottom to top).
    pub children: Vec<NodeId>,
    /// If true, children are clipped to the group's bounding box.
    pub clip_children: bool,
    /// When set, this child node acts as the clipping path for all other children.
    /// The clip node is the topmost child (last in `children`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clip_node_id: Option<NodeId>,
    /// When set, this path node is used as the spine for blend interpolation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blend_spine_id: Option<NodeId>,
}

impl GroupNode {
    pub fn new() -> Self {
        Self {
            children: vec![],
            clip_children: false,
            clip_node_id: None,
            blend_spine_id: None,
        }
    }
}

impl Default for GroupNode {
    fn default() -> Self {
        Self::new()
    }
}

/// A text node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextNode {
    pub content: String,
    pub font_family: String,
    pub font_size: f64,
    pub font_weight: u16,
    pub fill: Fill,
    pub stroke: Stroke,
    /// Text alignment: "left", "center", "right"
    pub align: TextAlign,
    /// Line height multiplier (1.0 = single spacing, 1.5 = 150%). Default: 1.2.
    #[serde(default = "default_line_height")]
    pub line_height: f64,
    /// Letter spacing in document units. Default: 0.0.
    #[serde(default)]
    pub letter_spacing: f64,
    /// When set, the text follows the outline of this path node (Type on a Path).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_spine_id: Option<NodeId>,
    /// Start offset along the spine path, in document units. Default: 0.0.
    #[serde(default)]
    pub path_offset: f64,
    /// When true, the text is laid out vertically (top to bottom). Default: false.
    #[serde(default)]
    pub vertical: bool,
    /// When set, the text flows within the area bounded by this closed path node (Area Type).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub area_path_id: Option<NodeId>,
    /// When set, the text content is replaced by the current value of this document variable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variable_binding: Option<String>,
    /// Font style: normal, italic, or oblique. Default: Normal.
    #[serde(default)]
    pub font_style: FontStyle,
    /// The next frame in a threaded text chain (overflow flows into this node).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_frame: Option<NodeId>,
    /// The previous frame in a threaded text chain (this node receives overflow from it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prev_frame: Option<NodeId>,
    /// Active OpenType feature tags (e.g. "liga", "calt", "frac", "smcp", "sups", "ordn").
    /// Empty means use font defaults. Non-empty activates the listed features.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub opentype_features: Vec<String>,
    /// Text decoration: "" (none), "underline", "line-through", or "overline". Default: none.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text_decoration: String,
    /// Space added before each paragraph in document units. Default: 0.0.
    #[serde(default)]
    pub paragraph_spacing_before: f64,
    /// Space added after each paragraph in document units. Default: 0.0.
    #[serde(default)]
    pub paragraph_spacing_after: f64,
    /// First-line indent in document units. Default: 0.0.
    #[serde(default)]
    pub text_indent: f64,
    /// Tab stop positions in document units. Empty means default tab stops every 4 em widths.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tab_stops: Vec<f64>,
    /// Baseline shift in document units, applied to the whole node. Positive raises the
    /// text above the baseline, negative lowers it. Default: 0.0 (on the baseline).
    #[serde(default)]
    pub baseline_shift: f64,
    /// Script position for the whole node: normal, superscript, or subscript.
    /// Super/subscript render at a reduced size with a vertical offset. Default: Normal.
    #[serde(default)]
    pub script_position: ScriptPosition,
}

fn default_line_height() -> f64 {
    1.2
}

impl TextNode {
    pub fn new(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            font_family: "sans-serif".to_string(),
            font_size: 16.0,
            font_weight: 400,
            fill: Fill::solid(crate::color::Color::BLACK),
            stroke: Stroke::none(),
            align: TextAlign::Left,
            line_height: 1.2,
            letter_spacing: 0.0,
            path_spine_id: None,
            path_offset: 0.0,
            vertical: false,
            area_path_id: None,
            variable_binding: None,
            font_style: FontStyle::Normal,
            next_frame: None,
            prev_frame: None,
            opentype_features: Vec::new(),
            text_decoration: String::new(),
            paragraph_spacing_before: 0.0,
            paragraph_spacing_after: 0.0,
            text_indent: 0.0,
            tab_stops: Vec::new(),
            baseline_shift: 0.0,
            script_position: ScriptPosition::Normal,
        }
    }
}

/// Script position for a text node: normal, superscript, or subscript.
///
/// Super/subscript render the whole node at a reduced font size with a vertical
/// baseline offset. This is a node-level attribute; true character-range
/// super/subscript (e.g. the "2" in "m²") requires a per-character run model and
/// is tracked as deferred work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ScriptPosition {
    #[default]
    Normal,
    Superscript,
    Subscript,
}

impl ScriptPosition {
    /// Font-size multiplier applied for this script position.
    pub fn size_scale(self) -> f64 {
        match self {
            ScriptPosition::Normal => 1.0,
            ScriptPosition::Superscript | ScriptPosition::Subscript => 0.58,
        }
    }

    /// Baseline offset in em of the *base* font size. Positive raises the glyphs
    /// (superscript), negative lowers them (subscript), zero for normal.
    pub fn baseline_offset_em(self) -> f64 {
        match self {
            ScriptPosition::Normal => 0.0,
            ScriptPosition::Superscript => 0.33,
            ScriptPosition::Subscript => -0.10,
        }
    }

    /// Parse from a lowercase string accepted by the MCP / GUI layers.
    pub fn from_str_opt(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "normal" | "" => Some(ScriptPosition::Normal),
            "superscript" | "super" | "sup" | "sups" => Some(ScriptPosition::Superscript),
            "subscript" | "sub" | "subs" => Some(ScriptPosition::Subscript),
            _ => None,
        }
    }

    /// Canonical lowercase string form.
    pub fn as_str(self) -> &'static str {
        match self {
            ScriptPosition::Normal => "normal",
            ScriptPosition::Superscript => "superscript",
            ScriptPosition::Subscript => "subscript",
        }
    }
}

/// Font style for text nodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FontStyle {
    #[default]
    Normal,
    Italic,
    Oblique,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
}

/// Canonical set of path-based shape primitives.
/// Add new geometric primitives here; MCP and GUI pick them up automatically.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrimitiveKind {
    Rectangle,
    RoundedRect,
    Ellipse,
    Polygon,
    Star,
    Line,
    Arc,
}

#[cfg(test)]
mod raster_mask_tests {
    use super::*;

    fn solid(w: u32, h: u32) -> RasterNode {
        RasterNode::new(RasterImage::filled(w, h, [10, 20, 30, 255]))
    }

    #[test]
    fn hide_selection_subtracts_from_layer_mask() {
        let mut n = solid(4, 4);
        // Select the left 2 columns.
        let sel = Mask::rect(4, 4, 0, 0, 2, 4);
        n.hide_selection(&sel);
        let m = n.mask.as_ref().expect("mask created");
        // Selected pixels hidden (0), unselected fully visible (255).
        assert_eq!(m.get(0, 0), 0);
        assert_eq!(m.get(1, 3), 0);
        assert_eq!(m.get(2, 0), 255);
        assert_eq!(m.get(3, 3), 255);
    }

    #[test]
    fn hide_selection_accumulates() {
        let mut n = solid(4, 4);
        n.hide_selection(&Mask::rect(4, 4, 0, 0, 1, 4));
        n.hide_selection(&Mask::rect(4, 4, 3, 0, 1, 4));
        let m = n.mask.as_ref().unwrap();
        assert_eq!(m.get(0, 0), 0);
        assert_eq!(m.get(3, 0), 0);
        assert_eq!(m.get(1, 0), 255);
    }

    #[test]
    fn hide_selection_size_mismatch_is_noop() {
        let mut n = solid(4, 4);
        n.hide_selection(&Mask::full(2, 2));
        assert!(n.mask.is_none());
    }

    #[test]
    fn set_foreground_matte_replaces_when_no_mask() {
        let mut n = solid(4, 4);
        let mut matte = Mask::empty(4, 4);
        matte.set(1, 1, 255);
        n.set_foreground_matte(matte);
        let m = n.mask.as_ref().unwrap();
        assert_eq!(m.get(1, 1), 255);
        assert_eq!(m.get(0, 0), 0);
    }

    #[test]
    fn crop_to_rect_trims_and_repositions() {
        // 10×10 image at canvas (-3, -2); crop to an 8×6 artboard at origin.
        let mut n = SceneNode::new("img", Uuid::nil(), SceneNodeKind::Raster(solid(10, 10)));
        n.transform = Transform::translate(-3.0, -2.0);
        n.crop_raster_to_rect(0.0, 0.0, 8.0, 6.0).expect("crop ok");
        let SceneNodeKind::Raster(rn) = &n.kind else {
            unreachable!()
        };
        // Pixels left of x=3 / above y=2 trimmed; artboard right/bottom edge
        // clips the rest: local 3..=10 clamps to width 8-… → (-3 + 3) = 0.
        assert_eq!(n.transform.matrix[4], 0.0);
        assert_eq!(n.transform.matrix[5], 0.0);
        assert_eq!((rn.image.width, rn.image.height), (7, 6));
    }

    #[test]
    fn crop_to_rect_crops_mask_in_lockstep() {
        let mut node = solid(6, 6);
        node.hide_selection(&Mask::rect(6, 6, 0, 0, 6, 3)); // hide top half
        let mut n = SceneNode::new("img", Uuid::nil(), SceneNodeKind::Raster(node));
        n.transform = Transform::translate(2.0, 2.0);
        // Crop to a rect covering only the bottom-right 3×3 of the image.
        n.crop_raster_to_rect(5.0, 5.0, 8.0, 8.0).expect("crop ok");
        let SceneNodeKind::Raster(rn) = &n.kind else {
            unreachable!()
        };
        assert_eq!((rn.image.width, rn.image.height), (3, 3));
        let m = rn.mask.as_ref().expect("mask kept");
        assert_eq!((m.width, m.height), (3, 3));
        // Bottom half of the original mask was fully visible → all 255 here.
        assert!(m.data.iter().all(|&v| v == 255));
        assert_eq!(n.transform.matrix[4], 5.0);
        assert_eq!(n.transform.matrix[5], 5.0);
    }

    #[test]
    fn crop_to_rect_rejects_rotation_and_no_overlap() {
        let mut n = SceneNode::new("img", Uuid::nil(), SceneNodeKind::Raster(solid(4, 4)));
        n.transform = Transform {
            matrix: [0.7, 0.7, -0.7, 0.7, 0.0, 0.0],
        };
        assert!(n.crop_raster_to_rect(0.0, 0.0, 2.0, 2.0).is_err());

        let mut n = SceneNode::new("img", Uuid::nil(), SceneNodeKind::Raster(solid(4, 4)));
        n.transform = Transform::translate(100.0, 100.0);
        assert!(n.crop_raster_to_rect(0.0, 0.0, 8.0, 8.0).is_err());
    }

    #[test]
    fn crop_to_rect_fully_inside_is_noop() {
        let mut n = SceneNode::new("img", Uuid::nil(), SceneNodeKind::Raster(solid(4, 4)));
        n.transform = Transform::translate(2.0, 2.0);
        n.crop_raster_to_rect(0.0, 0.0, 100.0, 100.0).expect("ok");
        let SceneNodeKind::Raster(rn) = &n.kind else {
            unreachable!()
        };
        assert_eq!((rn.image.width, rn.image.height), (4, 4));
        assert_eq!(n.transform.matrix[4], 2.0);
    }

    #[test]
    fn set_foreground_matte_intersects_existing_mask() {
        let mut n = solid(4, 4);
        // Existing mask hides the top-left pixel.
        n.hide_selection(&Mask::rect(4, 4, 0, 0, 1, 1));
        // Matte keeps only the top row.
        let matte = Mask::rect(4, 4, 0, 0, 4, 1);
        n.set_foreground_matte(matte);
        let m = n.mask.as_ref().unwrap();
        // Intersection: top-left stays hidden (0), rest of top row visible,
        // everything below the top row hidden by the matte.
        assert_eq!(m.get(0, 0), 0);
        assert_eq!(m.get(1, 0), 255);
        assert_eq!(m.get(0, 1), 0);
    }
}
