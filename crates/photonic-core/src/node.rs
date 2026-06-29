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

/// One operand of a live [`CompoundSpec`]: a sub-path (in the compound's local
/// coordinate space) and the boolean mode used to fold it into the result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompoundOperand {
    pub path_data: PathData,
    /// Combine mode. Ignored for the first operand (the base).
    pub op: crate::ops::boolean::BooleanOp,
}

/// Non-destructive (live) boolean state stored on a [`PathNode`]. The operands
/// stay editable; `PathNode::path_data` holds the baked result (recomputed via
/// [`crate::ops::boolean::eval_compound`] whenever the operands change), so every
/// existing consumer of `path_data` works unchanged.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompoundSpec {
    /// Ordered operands; the first is the base, each subsequent one is folded in.
    pub operands: Vec<CompoundOperand>,
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
    /// When set, this is a live boolean / compound shape: `path_data` is the
    /// baked result of folding these operands, which remain editable. `None` for
    /// an ordinary path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compound: Option<CompoundSpec>,
}

impl PathNode {
    pub fn new(path_data: PathData) -> Self {
        Self {
            path_data,
            fill: Fill::solid(crate::color::Color::BLACK),
            stroke: Stroke::none(),
            is_compound: false,
            compound: None,
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

    /// Create a live compound (boolean) path: `path_data` is the baked result of
    /// folding `spec`'s operands, which are retained for non-destructive editing.
    pub fn from_compound(spec: CompoundSpec) -> Self {
        let mut p = Self::new(crate::ops::boolean::eval_compound(&spec));
        p.is_compound = true;
        p.compound = Some(spec);
        p
    }

    /// Recompute `path_data` from the current compound operands. No-op for an
    /// ordinary (non-compound) path.
    pub fn rebake_compound(&mut self) {
        if let Some(spec) = &self.compound {
            self.path_data = crate::ops::boolean::eval_compound(spec);
        }
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
