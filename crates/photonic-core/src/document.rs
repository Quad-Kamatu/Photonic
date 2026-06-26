use crate::{
    annotation::{Annotation, AnnotationId},
    layer::{Layer, LayerId},
    node::{NodeId, SceneNode, SceneNodeKind},
    selection::Selection,
    Color,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Whether a guide is horizontal (fixed Y) or vertical (fixed X).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuideOrientation {
    Horizontal,
    Vertical,
}

/// A ruler guide — a horizontal or vertical reference line across the canvas.
/// Stored in the document; stripped from all export formats.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Guide {
    pub id: Uuid,
    /// Orientation of this guide.
    pub orientation: GuideOrientation,
    /// Position in document units: Y for horizontal guides, X for vertical guides.
    pub position: f64,
    /// Optional override color as [R, G, B, A] in [0,1] range.
    /// When `None`, the renderer uses the default guide color (cyan).
    #[serde(default)]
    pub color: Option<[f32; 4]>,
    /// Locked guides cannot be moved or deleted.
    #[serde(default)]
    pub locked: bool,
    /// When set, this is an angled construction line through (`position_x`, `position_y`)
    /// at the given angle in degrees (0° = horizontal, 90° = vertical, any angle allowed).
    /// The `position` and `orientation` fields are unused for angled guides.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub angle_degrees: Option<f64>,
    /// X coordinate for the origin point of an angled construction line.
    #[serde(default)]
    pub position_x: f64,
    /// Y coordinate for the origin point of an angled construction line.
    #[serde(default)]
    pub position_y: f64,
}

impl Guide {
    pub fn new(orientation: GuideOrientation, position: f64) -> Self {
        Self {
            id: Uuid::new_v4(),
            orientation,
            position,
            color: None,
            locked: false,
            angle_degrees: None,
            position_x: 0.0,
            position_y: 0.0,
        }
    }
}

/// File format version written into every saved `.photon` file.
/// Increment this when a breaking schema change is made.
pub const CURRENT_FORMAT_VERSION: u32 = 1;

fn default_format_version() -> u32 {
    CURRENT_FORMAT_VERSION
}

pub type DocumentId = Uuid;

/// A named export configuration stored in the document.
/// Profiles are applied via the `run_export_profile` MCP tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportProfile {
    /// Unique profile name used as a reference key.
    pub name: String,
    /// Target format: "svg", "png", "jpeg", or "webp".
    pub format: String,
    /// Raster-only: explicit output width in pixels.
    #[serde(default)]
    pub width: Option<u32>,
    /// Raster-only: explicit output height in pixels (overrides scale).
    #[serde(default)]
    pub height: Option<u32>,
    /// SVG-only: emit semantic `id` attributes on exported nodes (default true).
    #[serde(default)]
    pub semantic_ids: Option<bool>,
    /// SVG-only: decimal precision for coordinates (1–6, default 4).
    #[serde(default)]
    pub precision: Option<u32>,
}

impl ExportProfile {
    pub fn new_svg(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            format: "svg".to_string(),
            width: None,
            height: None,
            semantic_ids: Some(true),
            precision: Some(4),
        }
    }

    pub fn new_png(name: impl Into<String>, width: Option<u32>, height: Option<u32>) -> Self {
        Self {
            name: name.into(),
            format: "png".to_string(),
            width,
            height,
            semantic_ids: None,
            precision: None,
        }
    }
}

// ─── Symbols ─────────────────────────────────────────────────────────────────

/// A named symbol — a master node that can be placed as reusable instances.
/// Instances reference this symbol's master node via SceneNode::symbol_ref.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Symbol {
    pub id: Uuid,
    /// Unique symbol name.
    pub name: String,
    /// The master node ID. All instances share this node's geometry/style.
    pub master_node_id: NodeId,
}

impl Symbol {
    pub fn new(name: impl Into<String>, master_node_id: NodeId) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            master_node_id,
        }
    }
}

// ─── Color Swatches ───────────────────────────────────────────────────────────

/// A named document variable — a key-value pair that can be bound to text nodes
/// for data-driven design (e.g. names, prices, dates in a template).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentVariable {
    /// Unique variable name (used as the binding key).
    pub name: String,
    /// Current string value.
    pub value: String,
}

impl DocumentVariable {
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
        }
    }
}

/// A named document-level color swatch. Provides a shared color reference that
/// can be applied to any node fill or stroke; updating a swatch propagates to
/// all nodes that match its current color.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColorSwatch {
    pub id: Uuid,
    /// Unique display name.
    pub name: String,
    /// Color value as a CSS hex string (e.g. "#FF5733").
    pub color_hex: String,
}

impl ColorSwatch {
    pub fn new(name: impl Into<String>, color_hex: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            color_hex: color_hex.into(),
        }
    }
}

// ─── Gradient Swatches ───────────────────────────────────────────────────────

/// A named gradient swatch — a reusable gradient that can be saved from any node
/// and applied to other path nodes as a fill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GradientSwatch {
    pub id: Uuid,
    /// Unique display name.
    pub name: String,
    /// The gradient fill stored as a JSON-serialized `Fill`.
    /// Stored as a string blob to avoid importing style types into document.rs.
    pub fill_json: String,
}

impl GradientSwatch {
    pub fn new(name: impl Into<String>, fill_json: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            fill_json: fill_json.into(),
        }
    }
}

// ─── Graphic Styles ───────────────────────────────────────────────────────────

/// A named graphic style — a reusable appearance preset storing fill, stroke,
/// and opacity. Stored as JSON string blobs to avoid importing style types here.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphicStyle {
    pub id: Uuid,
    /// Unique display name.
    pub name: String,
    /// JSON-serialized `Fill`.
    pub fill_json: String,
    /// JSON-serialized `Stroke`.
    pub stroke_json: String,
    /// Node opacity (0.0–1.0).
    #[serde(default = "GraphicStyle::default_opacity")]
    pub opacity: f32,
}

impl GraphicStyle {
    pub fn default_opacity() -> f32 {
        1.0
    }

    pub fn new(
        name: impl Into<String>,
        fill_json: impl Into<String>,
        stroke_json: impl Into<String>,
        opacity: f32,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            fill_json: fill_json.into(),
            stroke_json: stroke_json.into(),
            opacity,
        }
    }
}

// ─── Variable Width Profiles ─────────────────────────────────────────────────

/// A named variable-width stroke profile. Width values are sampled at uniform
/// t intervals along the path (t=0 at start, t=1 at end). When applied, the
/// average width is used for uniform rendering; the profile is stored for
/// future variable-width rendering support.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WidthProfile {
    pub id: Uuid,
    /// Unique display name.
    pub name: String,
    /// Width samples at even t intervals along the path (in document units).
    /// Must have at least 2 values.
    pub widths: Vec<f64>,
}

impl WidthProfile {
    pub fn new(name: impl Into<String>, widths: Vec<f64>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            widths,
        }
    }

    /// Compute the average width (used for uniform stroke rendering).
    pub fn average_width(&self) -> f64 {
        if self.widths.is_empty() {
            return 1.0;
        }
        self.widths.iter().sum::<f64>() / self.widths.len() as f64
    }
}

// ─── Spot Colors ─────────────────────────────────────────────────────────────

// ─── Actions (Macro Sequences) ───────────────────────────────────────────────

/// A named action set — a recorded sequence of MCP tool calls that can be replayed.
/// Steps are stored as a JSON array of `{"tool": "...", "args": {...}}` objects.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionSet {
    pub id: Uuid,
    /// Unique display name.
    pub name: String,
    /// JSON-encoded array of action steps: `[{"tool":"...","args":{...}},...]`.
    pub steps_json: String,
}

impl ActionSet {
    pub fn new(name: impl Into<String>, steps_json: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            steps_json: steps_json.into(),
        }
    }
}

// ─── Event Triggers ──────────────────────────────────────────────────────────

/// A mapping from a document lifecycle event to a named action to execute.
/// Valid event names: "on_open", "on_save", "on_node_create", "on_selection_change".
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventTrigger {
    /// The event name that fires this trigger.
    pub event: String,
    /// Name of the action set to execute when the event fires.
    pub action_name: String,
}

// ─── Document Grammar Rules ───────────────────────────────────────────────────

/// A named design rule that the document must satisfy.
/// Rule types and their `params_json` shapes:
///
/// - `palette_includes`  → `{"color_hex": "#rrggbb"}`  — a specific fill color must appear somewhere
/// - `max_colors`        → `{"count": N}`               — no more than N unique solid fill colors
/// - `min_text_size`     → `{"px": N}`                  — all text nodes must have font_size >= N
/// - `required_layer`    → `{"name": "..."}` (or `"prefix": "..."`) — a layer with this name must exist
/// - `max_node_count`    → `{"count": N}`               — total visible node count must not exceed N
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrammarRule {
    pub id: Uuid,
    /// Human-readable name for the rule.
    pub name: String,
    /// Discriminator: `palette_includes`, `max_colors`, `min_text_size`, `required_layer`, `max_node_count`.
    pub rule_type: String,
    /// JSON-encoded rule parameters (shape depends on `rule_type`).
    pub params_json: String,
}

impl GrammarRule {
    pub fn new(
        name: impl Into<String>,
        rule_type: impl Into<String>,
        params_json: impl Into<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            rule_type: rule_type.into(),
            params_json: params_json.into(),
        }
    }
}

/// A named spot color — a print-production ink with a unique name, hex value,
/// and optional overprint flag. Stored in the document palette.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpotColor {
    pub id: Uuid,
    /// Unique display name (e.g. "Pantone 485 C", "PROCESS CYAN").
    pub name: String,
    /// Color value as a CSS hex string (e.g. "#FF2400").
    pub hex: String,
    /// When true, this ink overprints underlying inks rather than knocking out.
    #[serde(default)]
    pub overprint: bool,
}

impl SpotColor {
    pub fn new(name: impl Into<String>, hex: impl Into<String>, overprint: bool) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            hex: hex.into(),
            overprint,
        }
    }
}

// ─── Paragraph Styles ────────────────────────────────────────────────────────

/// A named paragraph style — text alignment, spacing, and size at the block level.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParagraphStyle {
    /// Unique name used as a reference key.
    pub name: String,
    /// Text alignment: "left", "center", "right", or "justify".
    #[serde(default)]
    pub align: Option<String>,
    /// Line height multiplier (e.g. 1.5).
    #[serde(default)]
    pub line_height: Option<f64>,
    /// Letter spacing in document units.
    #[serde(default)]
    pub letter_spacing: Option<f64>,
    /// Font size override.
    #[serde(default)]
    pub font_size: Option<f64>,
    /// Font family override.
    #[serde(default)]
    pub font_family: Option<String>,
}

// ─── Character Styles ─────────────────────────────────────────────────────────

/// A named character style preset — stores text formatting that can be
/// saved once and re-applied to any text node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterStyle {
    /// Unique name used as a reference key.
    pub name: String,
    /// Font family name (e.g. "Roboto", "sans-serif").
    #[serde(default)]
    pub font_family: Option<String>,
    /// Font size in document units.
    #[serde(default)]
    pub font_size: Option<f64>,
    /// CSS-style font weight (100–900). 400 = regular, 700 = bold.
    #[serde(default)]
    pub font_weight: Option<u16>,
    /// Fill color as CSS hex string (e.g. "#FF5733"). None = don't change.
    #[serde(default)]
    pub fill_hex: Option<String>,
    /// Letter spacing in document units.
    #[serde(default)]
    pub letter_spacing: Option<f64>,
    /// Line height multiplier (e.g. 1.5).
    #[serde(default)]
    pub line_height: Option<f64>,
}

/// The root document — contains pages, layers, and the scene graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    /// File format version — used to detect incompatible saves.
    /// Defaults to `CURRENT_FORMAT_VERSION` for files that predate this field.
    #[serde(default = "default_format_version")]
    pub format_version: u32,
    pub id: DocumentId,
    pub name: String,
    /// Document dimensions in logical pixels (at 96 dpi).
    pub width: f64,
    pub height: f64,
    /// Ordered list of layer IDs (bottom to top in the stack).
    pub layer_order: Vec<LayerId>,
    /// All layers, keyed by ID.
    pub layers: HashMap<LayerId, Layer>,
    /// All nodes (across all layers), keyed by ID.
    pub nodes: HashMap<NodeId, SceneNode>,
    /// The currently active layer for new node creation.
    pub active_layer_id: Option<LayerId>,
    /// Current selection state (not serialized in the file format).
    #[serde(skip)]
    pub selection: Selection,
    /// Design annotations and review comments.
    /// Persisted in `.photonic` files; stripped from all export formats.
    #[serde(default)]
    pub annotations: HashMap<AnnotationId, Annotation>,
    /// Ruler guides — horizontal and vertical reference lines.
    /// Persisted in `.photonic` files; stripped from all export formats.
    #[serde(default)]
    pub guides: Vec<Guide>,
    /// Most recently used fill/stroke colors (capped at 20, deduped).
    /// Updated on every fill or stroke color change.
    #[serde(default)]
    pub recent_colors: Vec<Color>,
    /// Named export profiles stored with the document.
    /// Each profile captures format settings for one-command export.
    #[serde(default)]
    pub export_profiles: Vec<ExportProfile>,
    /// Named character styles for rapid text formatting.
    #[serde(default)]
    pub character_styles: Vec<CharacterStyle>,
    /// Named paragraph styles for rapid block-level text formatting.
    #[serde(default)]
    pub paragraph_styles: Vec<ParagraphStyle>,
    /// Named color swatches shared across the document.
    #[serde(default)]
    pub color_swatches: Vec<ColorSwatch>,
    /// Named document variables for data-driven design (text binding).
    #[serde(default)]
    pub variables: Vec<DocumentVariable>,
    /// Named symbols (master nodes) that can be instantiated across the document.
    #[serde(default)]
    pub symbols: Vec<Symbol>,
    /// Named gradient swatches — reusable gradients saved from node fills.
    #[serde(default)]
    pub gradient_swatches: Vec<GradientSwatch>,
    /// Named spot colors — print-production inks with overprint settings.
    #[serde(default)]
    pub spot_colors: Vec<SpotColor>,
    /// Named graphic styles — reusable fill+stroke+opacity presets.
    #[serde(default)]
    pub graphic_styles: Vec<GraphicStyle>,
    /// Named variable-width stroke profiles.
    #[serde(default)]
    pub width_profiles: Vec<WidthProfile>,
    /// Named design grammar rules — constraints the document must satisfy.
    #[serde(default)]
    pub grammar_rules: Vec<GrammarRule>,
    /// Named action sets — replayable MCP tool sequences.
    #[serde(default)]
    pub action_sets: Vec<ActionSet>,
    /// Print bleed in millimetres (added to all four sides of the artboard).
    /// 0.0 means no bleed. Typical values: 3.0 mm (EU) or 0.125 in ≈ 3.175 mm (US).
    #[serde(default)]
    pub bleed_mm: f64,
    /// Print slug area in millimetres (additional margin outside the bleed for printer marks).
    #[serde(default)]
    pub slug_mm: f64,
    /// Artboard safe-area margin from the top edge, in document units.
    #[serde(default)]
    pub margin_top: f64,
    /// Artboard safe-area margin from the right edge, in document units.
    #[serde(default)]
    pub margin_right: f64,
    /// Artboard safe-area margin from the bottom edge, in document units.
    #[serde(default)]
    pub margin_bottom: f64,
    /// Artboard safe-area margin from the left edge, in document units.
    #[serde(default)]
    pub margin_left: f64,
    /// Script event triggers — named actions to fire on specific document events.
    #[serde(default)]
    pub event_triggers: Vec<EventTrigger>,
    /// Named panel workspace presets — each stores a properties-panel filter query.
    #[serde(default)]
    pub workspaces: Vec<Workspace>,
    /// Dimension annotations — measurement lines showing distances between node pairs.
    /// Persisted in `.photonic` files; stripped from all export formats.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dimensions: Vec<DimensionAnnotation>,
}

// ─── Dimension Annotation ─────────────────────────────────────────────────────

/// A measurement annotation showing the distance between two nodes.
/// Rendered as a dimension line with arrowheads and a distance label.
/// Stripped from all export formats; not a document-level selection item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DimensionAnnotation {
    pub id: Uuid,
    /// ID of the "from" node.
    pub from_node: crate::node::NodeId,
    /// ID of the "to" node.
    pub to_node: crate::node::NodeId,
    /// Measurement axis: "x" (horizontal only), "y" (vertical only), or "diagonal" (Euclidean).
    #[serde(default = "default_axis")]
    pub axis: String,
    /// Perpendicular offset from the line between nodes in document units (for visual clearance).
    #[serde(default)]
    pub label_offset: f64,
    /// Cached X of the from-node bounding-box center at creation time.
    pub from_x: f64,
    /// Cached Y of the from-node bounding-box center at creation time.
    pub from_y: f64,
    /// Cached X of the to-node bounding-box center at creation time.
    pub to_x: f64,
    /// Cached Y of the to-node bounding-box center at creation time.
    pub to_y: f64,
}

fn default_axis() -> String {
    "diagonal".to_string()
}

impl DimensionAnnotation {
    pub fn new(
        from_node: crate::node::NodeId,
        to_node: crate::node::NodeId,
        axis: String,
        label_offset: f64,
        from_x: f64,
        from_y: f64,
        to_x: f64,
        to_y: f64,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            from_node,
            to_node,
            axis,
            label_offset,
            from_x,
            from_y,
            to_x,
            to_y,
        }
    }

    /// The measured distance in document units according to axis.
    pub fn distance(&self) -> f64 {
        match self.axis.as_str() {
            "x" => (self.to_x - self.from_x).abs(),
            "y" => (self.to_y - self.from_y).abs(),
            _ => ((self.to_x - self.from_x).powi(2) + (self.to_y - self.from_y).powi(2)).sqrt(),
        }
    }
}

// ─── Workspace ────────────────────────────────────────────────────────────────

/// A named workspace preset that stores a properties-panel search filter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    /// Unique workspace name.
    pub name: String,
    /// Properties-panel search query that filters which sections are visible.
    #[serde(default)]
    pub search_query: String,
}

impl Document {
    /// Create a new blank document with a default layer.
    pub fn new(name: impl Into<String>, width: f64, height: f64) -> Self {
        let default_layer = Layer::new("Layer 1");
        let layer_id = default_layer.id;
        let mut layers = HashMap::new();
        layers.insert(layer_id, default_layer);

        Self {
            format_version: CURRENT_FORMAT_VERSION,
            id: Uuid::new_v4(),
            name: name.into(),
            width,
            height,
            layer_order: vec![layer_id],
            layers,
            nodes: HashMap::new(),
            active_layer_id: Some(layer_id),
            selection: Selection::new(),
            annotations: HashMap::new(),
            guides: Vec::new(),
            recent_colors: Vec::new(),
            export_profiles: Vec::new(),
            character_styles: Vec::new(),
            paragraph_styles: Vec::new(),
            color_swatches: Vec::new(),
            variables: Vec::new(),
            symbols: Vec::new(),
            gradient_swatches: Vec::new(),
            spot_colors: Vec::new(),
            graphic_styles: Vec::new(),
            width_profiles: Vec::new(),
            grammar_rules: Vec::new(),
            action_sets: Vec::new(),
            bleed_mm: 0.0,
            slug_mm: 0.0,
            margin_top: 0.0,
            margin_right: 0.0,
            margin_bottom: 0.0,
            margin_left: 0.0,
            event_triggers: Vec::new(),
            workspaces: Vec::new(),
            dimensions: Vec::new(),
        }
    }

    /// Default A4-landscape artboard.
    pub fn default_artboard() -> Self {
        Self::new("Untitled", 1123.0, 794.0)
    }

    /// Record a recently used color. Deduplicates and caps at 20 entries.
    /// New color is inserted at the front (most-recent first).
    pub fn record_recent_color(&mut self, color: Color) {
        self.recent_colors.retain(|c| {
            (c.r - color.r).abs() > f32::EPSILON
                || (c.g - color.g).abs() > f32::EPSILON
                || (c.b - color.b).abs() > f32::EPSILON
                || (c.a - color.a).abs() > f32::EPSILON
        });
        self.recent_colors.insert(0, color);
        self.recent_colors.truncate(20);
    }

    // --- Layer operations ---

    pub fn add_layer(&mut self, layer: Layer) -> LayerId {
        let id = layer.id;
        self.layers.insert(id, layer);
        self.layer_order.push(id);
        id
    }

    pub fn remove_layer(&mut self, id: &LayerId) -> Option<Layer> {
        if let Some(pos) = self.layer_order.iter().position(|l| l == id) {
            self.layer_order.remove(pos);
        }
        // Remove all nodes belonging to this layer
        self.nodes.retain(|_, node| &node.layer_id != id);
        self.layers.remove(id)
    }

    pub fn get_layer(&self, id: &LayerId) -> Option<&Layer> {
        self.layers.get(id)
    }

    pub fn get_layer_mut(&mut self, id: &LayerId) -> Option<&mut Layer> {
        self.layers.get_mut(id)
    }

    pub fn active_layer(&self) -> Option<&Layer> {
        self.active_layer_id
            .as_ref()
            .and_then(|id| self.layers.get(id))
    }

    // --- Node operations ---

    /// Add a node to the specified layer (or the active layer if None).
    pub fn add_node(&mut self, mut node: SceneNode, layer_id: Option<LayerId>) -> NodeId {
        let layer_id = layer_id
            .or(self.active_layer_id)
            .unwrap_or_else(|| *self.layer_order.last().expect("document has no layers"));

        node.layer_id = layer_id;
        let node_id = node.id;

        if let Some(layer) = self.layers.get_mut(&layer_id) {
            layer.node_ids.push(node_id);
        }
        self.nodes.insert(node_id, node);
        node_id
    }

    /// Remove a node by ID. Returns the removed node.
    pub fn remove_node(&mut self, id: &NodeId) -> Option<SceneNode> {
        if let Some(node) = self.nodes.remove(id) {
            if let Some(layer) = self.layers.get_mut(&node.layer_id) {
                layer.node_ids.retain(|nid| nid != id);
            }
            Some(node)
        } else {
            None
        }
    }

    pub fn get_node(&self, id: &NodeId) -> Option<&SceneNode> {
        self.nodes.get(id)
    }

    pub fn get_node_mut(&mut self, id: &NodeId) -> Option<&mut SceneNode> {
        self.nodes.get_mut(id)
    }

    /// Find a node by name (returns first match).
    pub fn find_node_by_name(&self, name: &str) -> Option<&SceneNode> {
        self.nodes.values().find(|n| n.name == name)
    }

    /// Find nodes by tag.
    pub fn find_nodes_by_tag(&self, tag: &str) -> Vec<&SceneNode> {
        self.nodes
            .values()
            .filter(|n| n.tags.iter().any(|t| t == tag))
            .collect()
    }

    // --- Annotation operations ---

    /// Add an annotation and return its ID.
    pub fn add_annotation(&mut self, ann: Annotation) -> AnnotationId {
        let id = ann.id;
        self.annotations.insert(id, ann);
        id
    }

    /// Mark an annotation as resolved. Returns `true` if found, `false` if not.
    pub fn resolve_annotation(&mut self, id: &AnnotationId) -> bool {
        if let Some(ann) = self.annotations.get_mut(id) {
            ann.resolved = true;
            true
        } else {
            false
        }
    }

    /// Remove an annotation entirely. Returns the removed annotation if it existed.
    pub fn remove_annotation(&mut self, id: &AnnotationId) -> Option<Annotation> {
        self.annotations.remove(id)
    }

    /// Returns nodes in draw order (bottom layer, bottom node first).
    /// Group nodes are expanded recursively — their path children are yielded in place.
    pub fn nodes_in_draw_order(&self) -> Vec<&SceneNode> {
        let mut result = vec![];
        for layer_id in &self.layer_order {
            if let Some(layer) = self.layers.get(layer_id) {
                if !layer.visible {
                    continue;
                }
                for node_id in &layer.node_ids {
                    self.collect_draw_nodes(node_id, &mut result);
                }
            }
        }
        result
    }

    fn collect_draw_nodes<'a>(&'a self, node_id: &NodeId, out: &mut Vec<&'a SceneNode>) {
        if let Some(node) = self.nodes.get(node_id) {
            if !node.visible {
                return;
            }
            match &node.kind {
                SceneNodeKind::Group(g) => {
                    for child_id in &g.children {
                        self.collect_draw_nodes(child_id, out);
                    }
                }
                _ => out.push(node),
            }
        }
    }

    /// Maximum symbol-nesting depth before resolution bails out (cycle guard).
    const SYMBOL_MAX_DEPTH: u8 = 8;

    /// Resolve a node's render geometry/style from its symbol master.
    ///
    /// Symbol instances are created as a frozen clone of the master's geometry
    /// (see `place_symbol`), so edits to the master would otherwise never reach
    /// existing instances. This returns a node whose `kind` is taken from the
    /// *current* master — with the instance's own transform, opacity,
    /// visibility and per-instance fill/stroke colour overrides preserved — so
    /// master edits propagate live. Non-instances are returned borrowed and
    /// unchanged (zero-cost on the render hot path).
    ///
    /// Nested symbols (a master that is itself an instance) are followed up to
    /// [`Self::SYMBOL_MAX_DEPTH`]; cycles and dangling references fall back to
    /// rendering the instance's frozen copy.
    ///
    /// Note: only single-node masters propagate today. A group master is
    /// flattened to leaf nodes at placement time and those leaves carry no
    /// `symbol_ref`, so group/nested-group propagation is tracked as follow-up.
    pub fn resolve_render_node<'a>(&'a self, node: &'a SceneNode) -> std::borrow::Cow<'a, SceneNode> {
        use std::borrow::Cow;
        let Some(sym_id) = node.symbol_ref else {
            return Cow::Borrowed(node);
        };

        // Follow the symbol → master chain, guarding against cycles/depth.
        let mut current_sym = Some(sym_id);
        let mut master: Option<&SceneNode> = None;
        let mut depth = 0u8;
        while let Some(sid) = current_sym {
            if depth > Self::SYMBOL_MAX_DEPTH {
                // Nesting too deep (likely a cycle): render the frozen copy.
                return Cow::Borrowed(node);
            }
            let Some(sym) = self.symbols.iter().find(|s| s.id == sid) else {
                // Dangling symbol reference: render the frozen copy.
                break;
            };
            let Some(m) = self.nodes.get(&sym.master_node_id) else {
                break;
            };
            if m.id == node.id {
                // Instance is its own master — would loop forever.
                return Cow::Borrowed(node);
            }
            master = Some(m);
            current_sym = m.symbol_ref;
            depth += 1;
        }

        let Some(master) = master else {
            return Cow::Borrowed(node);
        };

        // Master geometry/style with instance placement + overrides on top.
        let mut resolved = node.clone();
        resolved.kind = master.kind.clone();
        Self::apply_symbol_overrides(&mut resolved);
        Cow::Owned(resolved)
    }

    /// Apply a symbol instance's hex colour overrides onto its (master-derived)
    /// `kind`. A fill override replaces a solid fill; a stroke override replaces
    /// the stroke colour. No-op when overrides are absent or unparseable.
    fn apply_symbol_overrides(node: &mut SceneNode) {
        use crate::style::FillKind;
        let fill = node.symbol_fill_override.as_deref().and_then(Color::from_hex);
        let stroke = node.symbol_stroke_override.as_deref().and_then(Color::from_hex);
        if fill.is_none() && stroke.is_none() {
            return;
        }
        if let SceneNodeKind::Path(pn) = &mut node.kind {
            if let Some(c) = fill {
                pn.fill.kind = FillKind::Solid(c);
                pn.fill.enabled = true;
            }
            if let Some(c) = stroke {
                pn.stroke.color = c;
                pn.stroke.enabled = true;
            }
        }
    }

    /// Returns node count across all layers.
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Find which layer contains a node and its index within that layer's node_ids.
    pub fn node_layer_and_index(&self, node_id: &NodeId) -> Option<(LayerId, usize)> {
        for layer_id in &self.layer_order {
            if let Some(layer) = self.layers.get(layer_id) {
                if let Some(pos) = layer.node_ids.iter().position(|id| id == node_id) {
                    return Some((*layer_id, pos));
                }
            }
        }
        None
    }

    /// Return the ID of the top-level ancestor of `node_id`.
    ///
    /// If the node already appears in a layer's `node_ids` it is returned as-is.
    /// If it is a group child, the search walks up the group hierarchy until a
    /// top-level node is found.  Returns `None` if the node does not exist.
    pub fn top_level_ancestor(&self, node_id: NodeId) -> Option<NodeId> {
        if self.nodes.get(&node_id).is_none() {
            return None;
        }
        // Already top-level?
        let is_top = self.layer_order.iter().any(|lid| {
            self.layers
                .get(lid)
                .map(|l| l.node_ids.contains(&node_id))
                .unwrap_or(false)
        });
        if is_top {
            return Some(node_id);
        }
        // Walk the group hierarchy
        for (&group_id, node) in &self.nodes {
            if let SceneNodeKind::Group(g) = &node.kind {
                if g.children.contains(&node_id) {
                    return self.top_level_ancestor(group_id);
                }
            }
        }
        None
    }

    /// Find the shared layer and indices for a set of nodes.
    /// Returns None if any node is missing or they span different layers.
    pub fn nodes_layer_and_indices(
        &self,
        node_ids: &[NodeId],
    ) -> Option<(LayerId, Vec<(NodeId, usize)>)> {
        if node_ids.is_empty() {
            return None;
        }
        let mut result = Vec::new();
        let mut common_layer: Option<LayerId> = None;
        for &nid in node_ids {
            let (layer_id, idx) = self.node_layer_and_index(&nid)?;
            if let Some(lid) = common_layer {
                if lid != layer_id {
                    return None; // nodes span different layers
                }
            } else {
                common_layer = Some(layer_id);
            }
            result.push((nid, idx));
        }
        Some((common_layer?, result))
    }

    /// Serialize to JSON string.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Deserialize from JSON string. Returns an error if the file's `format_version`
    /// is newer than `CURRENT_FORMAT_VERSION` (i.e. saved by a future build).
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        let doc: Self = serde_json::from_str(json)?;
        if doc.format_version > CURRENT_FORMAT_VERSION {
            return Err(serde::de::Error::custom(format!(
                "unsupported format version {} (this build supports up to version {})",
                doc.format_version, CURRENT_FORMAT_VERSION
            )));
        }
        Ok(doc)
    }
}

/// A page (for multi-page documents — future use).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Page {
    pub id: Uuid,
    pub name: String,
    pub document: Document,
}

#[cfg(test)]
mod symbol_resolution_tests {
    use super::*;
    use crate::node::PathNode;
    use crate::path::PathData;
    use crate::style::{Fill, FillKind};

    fn solid_fill(node: &SceneNode) -> Color {
        match &node.kind {
            SceneNodeKind::Path(pn) => match pn.fill.kind {
                FillKind::Solid(c) => c,
                _ => panic!("expected solid fill"),
            },
            _ => panic!("expected path node"),
        }
    }

    /// Build a master path node (red fill) + register it as symbol "sym".
    fn doc_with_symbol() -> (Document, NodeId, Uuid) {
        let mut doc = Document::new("test", 100.0, 100.0);
        let layer = doc.active_layer_id.unwrap();
        let mut master = SceneNode::new(
            "master",
            layer,
            SceneNodeKind::Path(PathNode::new(PathData::rect(0.0, 0.0, 10.0, 10.0))),
        );
        if let SceneNodeKind::Path(pn) = &mut master.kind {
            pn.fill = Fill::solid(Color::RED);
        }
        let master_id = doc.add_node(master, None);
        let sym = Symbol::new("sym", master_id);
        let sym_id = sym.id;
        doc.symbols.push(sym);
        (doc, master_id, sym_id)
    }

    fn make_instance(doc: &Document, master_id: NodeId, sym_id: Uuid) -> SceneNode {
        let mut inst = doc.nodes.get(&master_id).unwrap().clone();
        inst.id = Uuid::new_v4();
        inst.name = "instance".into();
        inst.symbol_ref = Some(sym_id);
        inst.transform = crate::transform::Transform::translate(50.0, 50.0);
        // Freeze a stale green fill on the instance copy.
        if let SceneNodeKind::Path(pn) = &mut inst.kind {
            pn.fill = Fill::solid(Color::GREEN);
        }
        inst
    }

    #[test]
    fn master_edits_propagate_to_instances() {
        let (mut doc, master_id, sym_id) = doc_with_symbol();
        let inst = make_instance(&doc, master_id, sym_id);
        let inst_id = doc.add_node(inst, None);

        // Edit the master fill to blue *after* the instance was placed.
        if let Some(SceneNodeKind::Path(pn)) =
            doc.nodes.get_mut(&master_id).map(|n| &mut n.kind)
        {
            pn.fill = Fill::solid(Color::BLUE);
        }

        let inst_ref = doc.nodes.get(&inst_id).unwrap();
        let resolved = doc.resolve_render_node(inst_ref);
        // Geometry/style now reflects the current master (blue), not the frozen green.
        assert_eq!(solid_fill(&resolved), Color::BLUE);
        // Instance placement is preserved.
        assert_eq!(resolved.transform.apply(0.0, 0.0), (50.0, 50.0));
    }

    #[test]
    fn fill_override_takes_precedence_over_master() {
        let (doc, master_id, sym_id) = doc_with_symbol();
        let mut inst = make_instance(&doc, master_id, sym_id);
        inst.symbol_fill_override = Some("#00ffff".into()); // cyan
        let resolved = doc.resolve_render_node(&inst);
        let c = solid_fill(&resolved);
        assert!((c.r - 0.0).abs() < 1e-3 && (c.g - 1.0).abs() < 1e-3 && (c.b - 1.0).abs() < 1e-3);
    }

    #[test]
    fn non_instance_is_borrowed_unchanged() {
        let (doc, master_id, _sym_id) = doc_with_symbol();
        let master = doc.nodes.get(&master_id).unwrap();
        let resolved = doc.resolve_render_node(master);
        assert!(matches!(resolved, std::borrow::Cow::Borrowed(_)));
    }

    #[test]
    fn self_referential_symbol_does_not_loop() {
        // A symbol whose master is the instance itself must not infinite-loop.
        let mut doc = Document::new("test", 100.0, 100.0);
        let layer = doc.active_layer_id.unwrap();
        let node = SceneNode::new(
            "n",
            layer,
            SceneNodeKind::Path(PathNode::new(PathData::rect(0.0, 0.0, 5.0, 5.0))),
        );
        let nid = doc.add_node(node, None);
        let sym = Symbol::new("self", nid);
        let sym_id = sym.id;
        doc.symbols.push(sym);
        // Point the node at the symbol whose master is the node itself.
        doc.nodes.get_mut(&nid).unwrap().symbol_ref = Some(sym_id);
        let n = doc.nodes.get(&nid).unwrap();
        let resolved = doc.resolve_render_node(n);
        assert!(matches!(resolved, std::borrow::Cow::Borrowed(_)));
    }

    #[test]
    fn dangling_symbol_ref_renders_frozen_copy() {
        let (doc, master_id, _sym_id) = doc_with_symbol();
        let mut inst = make_instance(&doc, master_id, Uuid::new_v4()); // unknown symbol id
        inst.symbol_ref = Some(Uuid::new_v4());
        let resolved = doc.resolve_render_node(&inst);
        // Falls back to the instance's own (green) copy.
        assert_eq!(solid_fill(&resolved), Color::GREEN);
    }
}
