use crate::{
    color::Color,
    layer::{BlendMode, LayerId},
    path::PathData,
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

    /// True if canvas-space point `(cx, cy)` is inside this node's filled geometry,
    /// honoring its transform and fill rule. Maps the point into node-local space
    /// via the inverse transform, then applies kurbo winding. Returns false for a
    /// singular (degenerate) transform so the caller can fall back to bbox.
    /// For non-Path kinds (Text/Group) always returns false.
    pub fn contains_canvas_point(&self, cx: f64, cy: f64) -> bool {
        use kurbo::Shape;

        let path_node = match &self.kind {
            SceneNodeKind::Path(p) => p,
            _ => return false,
        };

        let bez = path_node.path_data.to_bez_path();
        if bez.elements().is_empty() {
            return false;
        }

        let aff = self.transform.to_kurbo();
        if aff.determinant().abs() < 1e-12 {
            return false;
        }

        let p = aff.inverse() * kurbo::Point::new(cx, cy);

        let winding = bez.winding(p);
        if path_node.is_compound {
            winding % 2 != 0
        } else {
            winding != 0
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Color;
    use crate::layer::LayerId;
    use crate::path::PathData;
    use crate::style::{Fill, Stroke};
    use crate::transform::Transform;

    fn make_path_node_svg(svg: &str, transform: Transform, is_compound: bool) -> SceneNode {
        let path_data = PathData::from_svg(svg).expect("valid SVG path");
        let path_node = PathNode {
            path_data,
            fill: Fill::solid(Color::BLACK),
            stroke: Stroke::none(),
            is_compound,
        };
        let mut node = SceneNode::new("test", LayerId::default(), SceneNodeKind::Path(path_node));
        node.transform = transform;
        node
    }

    // ── 1. Simple rectangle — inside ─────────────────────────────────────────
    #[test]
    fn hit_rect_center_is_inside() {
        let node = make_path_node_svg(
            "M 0 0 L 100 0 L 100 100 L 0 100 Z",
            Transform::IDENTITY,
            false,
        );
        assert!(node.contains_canvas_point(50.0, 50.0));
    }

    // ── 2. Simple rectangle — clearly outside ────────────────────────────────
    #[test]
    fn hit_rect_outside_bbox_is_false() {
        let node = make_path_node_svg(
            "M 0 0 L 100 0 L 100 100 L 0 100 Z",
            Transform::IDENTITY,
            false,
        );
        assert!(!node.contains_canvas_point(150.0, 150.0));
    }

    // ── 3. Transform: translated node ────────────────────────────────────────
    #[test]
    fn hit_translated_node_canvas_point_maps_correctly() {
        // Rect at local (0..100, 0..100), translated by (100, 100).
        // Canvas (150,150) → local (50,50) → inside.
        let node = make_path_node_svg(
            "M 0 0 L 100 0 L 100 100 L 0 100 Z",
            Transform::translate(100.0, 100.0),
            false,
        );
        assert!(
            node.contains_canvas_point(150.0, 150.0),
            "translated inside"
        );
        // Canvas (50,50) → local (-50,-50) → outside.
        assert!(
            !node.contains_canvas_point(50.0, 50.0),
            "old location now outside"
        );
    }

    // ── 4. Concavity — L-shape, solid arm ────────────────────────────────────
    // L-shape covers left column (x=0..50, y=0..100) and top bar (x=0..100, y=0..50).
    // Empty region: x=50..100, y=50..100.
    #[test]
    fn hit_l_shape_solid_arm_inside() {
        let svg = "M 0 0 L 100 0 L 100 50 L 50 50 L 50 100 L 0 100 Z";
        let node = make_path_node_svg(svg, Transform::IDENTITY, false);
        // Point in the solid left-vertical arm
        assert!(node.contains_canvas_point(25.0, 75.0));
    }

    // ── 5. Concavity — L-shape, empty concave void ───────────────────────────
    #[test]
    fn hit_l_shape_concave_void_is_false() {
        let svg = "M 0 0 L 100 0 L 100 50 L 50 50 L 50 100 L 0 100 Z";
        let node = make_path_node_svg(svg, Transform::IDENTITY, false);
        // Point in bounding box but inside the concave void — must be false
        assert!(!node.contains_canvas_point(75.0, 75.0));
    }

    // ── 6. Compound/even-odd donut — point in hole ───────────────────────────
    // Outer rect 0..100, inner rect 25..75, both same winding direction.
    // is_compound=true → even-odd → hole at (50,50).
    #[test]
    fn hit_donut_even_odd_hole_is_false() {
        let svg = "M 0 0 L 100 0 L 100 100 L 0 100 Z M 25 25 L 75 25 L 75 75 L 25 75 Z";
        let node = make_path_node_svg(svg, Transform::IDENTITY, true);
        assert!(!node.contains_canvas_point(50.0, 50.0));
    }

    // ── 7. Compound/even-odd donut — point in ring ───────────────────────────
    #[test]
    fn hit_donut_even_odd_ring_is_true() {
        let svg = "M 0 0 L 100 0 L 100 100 L 0 100 Z M 25 25 L 75 25 L 75 75 L 25 75 Z";
        let node = make_path_node_svg(svg, Transform::IDENTITY, true);
        assert!(node.contains_canvas_point(10.0, 10.0));
    }

    // ── 8. Singular transform — must not panic, returns false ─────────────────
    #[test]
    fn hit_singular_transform_returns_false() {
        let node = make_path_node_svg(
            "M 0 0 L 100 0 L 100 100 L 0 100 Z",
            Transform::scale(0.0, 0.0),
            false,
        );
        assert!(!node.contains_canvas_point(50.0, 50.0));
    }

    // ── 9. Non-path node always returns false ────────────────────────────────
    #[test]
    fn hit_text_node_returns_false() {
        let node = SceneNode::new(
            "text",
            LayerId::default(),
            SceneNodeKind::Text(TextNode::new("hello")),
        );
        assert!(!node.contains_canvas_point(5.0, 5.0));
    }
}
