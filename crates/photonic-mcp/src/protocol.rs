use photonic_core::{
    color::Color, layer::BlendMode, ops::boolean::BooleanOp, style::LineJoin, GaussianGlow,
    GlowEffect,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

// ─── JSON-RPC 2.0 envelope types ────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Option<Value>,
    pub method: String,
    pub params: Option<Value>,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

impl JsonRpcResponse {
    pub fn success(id: Option<Value>, result: impl Serialize) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(serde_json::to_value(result).unwrap()),
            error: None,
        }
    }

    pub fn error(id: Option<Value>, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
                data: None,
            }),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    pub data: Option<Value>,
}

// ─── MCP Initialize ──────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResult {
    pub protocol_version: String,
    pub capabilities: ServerCapabilities,
    pub server_info: ServerInfo,
}

#[derive(Debug, Serialize)]
pub struct ServerCapabilities {
    pub tools: ToolsCapability,
}

#[derive(Debug, Serialize)]
pub struct ToolsCapability {
    pub list_changed: bool,
}

#[derive(Debug, Serialize)]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
}

// ─── MCP Tool Call args/results ───────────────────────────────────────────────

/// Arguments for `copy_nodes_to_clipboard` tool
#[derive(Debug, Deserialize)]
pub struct CopyNodesToClipboardArgs {
    /// IDs of the nodes to copy into the clipboard.
    pub node_ids: Vec<Uuid>,
    /// Optional human-readable label for this clipboard entry.
    /// Defaults to "N node(s)".
    #[serde(default)]
    pub label: Option<String>,
}

/// Arguments for `get_clipboard_history` tool (no parameters).
#[derive(Debug, Deserialize, Default)]
pub struct GetClipboardHistoryArgs {}

/// Arguments for `paste_from_history` tool
#[derive(Debug, Deserialize)]
pub struct PasteFromHistoryArgs {
    /// Zero-based index into the clipboard ring (0 = most recent).
    pub index: usize,
    /// Horizontal offset applied to the pasted nodes (default: 0).
    #[serde(default)]
    pub offset_x: Option<f64>,
    /// Vertical offset applied to the pasted nodes (default: 0).
    #[serde(default)]
    pub offset_y: Option<f64>,
    /// Target layer for pasted nodes. Defaults to the document's active layer.
    #[serde(default)]
    pub layer_id: Option<Uuid>,
}

/// Arguments for `export_design_tokens` tool
#[derive(Debug, Deserialize, Default)]
pub struct ExportDesignTokensArgs {
    /// Output format: "json" (default) | "css" | "tailwind" | "style-dictionary"
    #[serde(default)]
    pub format: Option<String>,
}

/// Arguments for `list_audit_log` tool
#[derive(Debug, Deserialize, Default)]
pub struct ListAuditLogArgs {
    /// Maximum number of entries to return (default 50, capped at 1000).
    #[serde(default)]
    pub limit: Option<usize>,
}

/// Arguments for `export_audit_log` tool (no parameters needed).
#[derive(Debug, Deserialize, Default)]
pub struct ExportAuditLogArgs {}

/// Arguments for `export_svg` tool
#[derive(Debug, Deserialize, Default)]
pub struct ExportSvgArgs {
    /// When true, return only the inner SVG content without the outer <svg> wrapper.
    #[serde(default)]
    pub inner_only: bool,
    /// Emit slugified node/layer names as `id` attributes (default: true).
    pub semantic_ids: Option<bool>,
    /// Decimal precision for coordinate values, clamped 1–6 (default: 4).
    pub precision: Option<u8>,
}

/// Arguments for `export_selection_as_svg` tool
#[derive(Debug, Deserialize, Default)]
pub struct ExportSelectionArgs {
    /// Node IDs to export. If omitted or empty, uses the current document selection.
    #[serde(default)]
    pub node_ids: Option<Vec<String>>,
    /// When true, wrap the SVG in a React functional component (default: false).
    #[serde(default)]
    pub as_react_component: bool,
    /// Component name when `as_react_component` is true (default: "SvgIcon").
    pub component_name: Option<String>,
}

/// Arguments for `zig_zag_path` tool
#[derive(Debug, Deserialize)]
pub struct ZigZagPathArgs {
    /// Path node IDs to apply the zig-zag to.
    pub node_ids: Vec<String>,
    /// Peak-to-peak amplitude of the zig-zag (in document units). Default: 10.
    pub size: Option<f64>,
    /// Number of ridges (peaks) per original path segment. Default: 4.
    pub ridges_per_segment: Option<usize>,
    /// If true, produce smooth waves (cubic bezier); if false, sharp corners (line segments). Default: false.
    #[serde(default)]
    pub smooth: bool,
}

/// Arguments for `pucker_bloat` tool
#[derive(Debug, Deserialize)]
pub struct PuckerBloatArgs {
    /// Path node IDs to distort.
    pub node_ids: Vec<String>,
    /// Distortion strength: positive = bloat (expand outward), negative = pucker (contract inward).
    /// Range roughly -1.0 to 1.0 for subtle effects; larger values are more extreme. Default: 0.5.
    pub strength: Option<f64>,
    /// X coordinate of the distortion center. Defaults to the path's centroid.
    pub center_x: Option<f64>,
    /// Y coordinate of the distortion center. Defaults to the path's centroid.
    pub center_y: Option<f64>,
}

/// Arguments for `tag_nodes` tool
#[derive(Debug, Deserialize)]
pub struct TagNodesArgs {
    /// Node IDs. Empty = use selection.
    #[serde(default)]
    pub node_ids: Vec<String>,
    /// Tags to add.
    #[serde(default)]
    pub add: Vec<String>,
    /// Tags to remove.
    #[serde(default)]
    pub remove: Vec<String>,
}

/// Arguments for `sample_color_at` tool
#[derive(Debug, Deserialize)]
pub struct SampleColorAtArgs {
    /// Canvas X coordinate.
    pub x: f64,
    /// Canvas Y coordinate.
    pub y: f64,
}

/// Arguments for `set_active_layer` tool
#[derive(Debug, Deserialize)]
pub struct SetActiveLayerArgs {
    /// Layer UUID or name.
    pub layer_id: String,
}

/// Arguments for `delete_layer` tool
#[derive(Debug, Deserialize)]
pub struct DeleteLayerArgs {
    /// Layer UUID or name to delete.
    pub layer_id: String,
    /// If true, also delete all nodes on the layer (default: false — nodes moved to first remaining layer).
    #[serde(default)]
    pub delete_nodes: bool,
}

/// Arguments for `move_to_layer` tool
#[derive(Debug, Deserialize)]
pub struct MoveToLayerArgs {
    /// Node IDs to move. Empty = use selection.
    #[serde(default)]
    pub node_ids: Vec<String>,
    /// Target layer UUID or name.
    pub target_layer: String,
}

/// Arguments for `add_dimension_line` tool
#[derive(Debug, Deserialize)]
pub struct AddDimensionLineArgs {
    /// Start X.
    pub x1: f64,
    /// Start Y.
    pub y1: f64,
    /// End X.
    pub x2: f64,
    /// End Y.
    pub y2: f64,
    /// Offset distance for the dimension line from the measured points (default: 20).
    pub offset: Option<f64>,
    /// Font size for the label (default: 12).
    pub font_size: Option<f64>,
    /// Color hex for the dimension line (default: "#666666").
    pub color: Option<String>,
    pub layer_id: Option<String>,
}

/// Arguments for `reorder_layers` tool
#[derive(Debug, Deserialize)]
pub struct ReorderLayersArgs {
    /// New layer order as an array of layer UUIDs (bottom to top).
    pub layer_order: Vec<String>,
}

/// Arguments for `set_selection` tool
#[derive(Debug, Deserialize)]
pub struct SetSelectionArgs {
    /// Node IDs to select. Empty = clear selection.
    #[serde(default)]
    pub node_ids: Vec<String>,
    /// If true, add to existing selection instead of replacing (default: false).
    #[serde(default)]
    pub additive: bool,
}

/// Arguments for `flatten_group` tool
#[derive(Debug, Deserialize, Default)]
pub struct FlattenGroupArgs {
    /// Group node IDs to flatten. Empty = use selection.
    #[serde(default)]
    pub node_ids: Vec<String>,
}

/// Arguments for `center_on_canvas` tool
#[derive(Debug, Deserialize, Default)]
pub struct CenterOnCanvasArgs {
    /// Node IDs. Empty = use selection.
    #[serde(default)]
    pub node_ids: Vec<String>,
    /// Center horizontally (default: true).
    #[serde(default = "default_true_fn")]
    pub horizontal: bool,
    /// Center vertically (default: true).
    #[serde(default = "default_true_fn")]
    pub vertical: bool,
}

/// Arguments for `remove_fill` / `remove_stroke` tools
#[derive(Debug, Deserialize, Default)]
pub struct RemoveStyleArgs {
    /// Node IDs. Empty = use selection.
    #[serde(default)]
    pub node_ids: Vec<String>,
}

/// Arguments for `fit_to_canvas` tool
#[derive(Debug, Deserialize, Default)]
pub struct FitToCanvasArgs {
    /// Padding in document units around the edges (default: 10).
    pub padding: Option<f64>,
    /// Node IDs to fit. Empty = all visible nodes.
    #[serde(default)]
    pub node_ids: Vec<String>,
}

/// Arguments for `create_scatter_plot` tool
#[derive(Debug, Deserialize)]
pub struct CreateScatterPlotArgs {
    /// Left X.
    pub x: f64,
    /// Bottom Y.
    pub y: f64,
    /// Plot width (default: 300).
    pub width: Option<f64>,
    /// Plot height (default: 300).
    pub height: Option<f64>,
    /// Data points as [x, y] pairs.
    pub points: Vec<[f64; 2]>,
    /// Dot radius (default: 4).
    pub dot_radius: Option<f64>,
    /// Dot color hex (default: "#4E79A7").
    pub color: Option<String>,
    pub layer_id: Option<String>,
}

/// Arguments for `scatter_copies` tool
#[derive(Debug, Deserialize)]
pub struct ScatterCopiesArgs {
    /// Source node ID to scatter.
    pub node_id: String,
    /// Number of copies (default: 20).
    pub count: Option<usize>,
    /// Scatter area left X.
    pub x: f64,
    /// Scatter area top Y.
    pub y: f64,
    /// Area width.
    pub width: f64,
    /// Area height.
    pub height: f64,
    /// Random rotation range in degrees (default: 0 = no rotation). Each copy gets random rotation in [-range, +range].
    pub rotation_range: Option<f64>,
    /// Scale variation range. Each copy scaled between [1-range, 1+range]. Default: 0 (no variation).
    pub scale_range: Option<f64>,
    /// Random seed (default: 42).
    pub seed: Option<u64>,
}

/// Arguments for `create_line_chart` tool
#[derive(Debug, Deserialize)]
pub struct CreateLineChartArgs {
    /// Left X.
    pub x: f64,
    /// Bottom Y (data grows upward).
    pub y: f64,
    /// Chart width (default: 300).
    pub width: Option<f64>,
    /// Chart height (default: 200).
    pub height: Option<f64>,
    /// Data series. Each series is an array of values.
    pub series: Vec<Vec<f64>>,
    /// Colors for each series as hex.
    #[serde(default)]
    pub colors: Vec<String>,
    /// Stroke width for lines (default: 2).
    pub stroke_width: Option<f64>,
    /// Smooth the lines using Catmull-Rom interpolation (default: true).
    #[serde(default = "default_true_fn")]
    pub smooth: bool,
    /// Fill area under each line (default: false).
    #[serde(default)]
    pub fill_area: bool,
    pub layer_id: Option<String>,
}

/// Arguments for `create_bar_chart` tool
#[derive(Debug, Deserialize)]
pub struct CreateBarChartArgs {
    /// Left X.
    pub x: f64,
    /// Bottom Y (bars grow upward).
    pub y: f64,
    /// Total chart width (default: 300).
    pub width: Option<f64>,
    /// Max bar height (default: 200).
    pub height: Option<f64>,
    /// Data values — each bar height is proportional to its value.
    pub values: Vec<f64>,
    /// Bar colors as hex. Cycles if fewer than values.
    #[serde(default)]
    pub colors: Vec<String>,
    /// Labels for each bar (optional).
    #[serde(default)]
    pub labels: Vec<String>,
    /// Gap between bars as fraction of bar width (default: 0.2).
    pub gap: Option<f64>,
    /// If true, bars are horizontal instead of vertical (default: false).
    #[serde(default)]
    pub horizontal: bool,
    pub layer_id: Option<String>,
}

/// Arguments for `create_pie_chart` tool
#[derive(Debug, Deserialize)]
pub struct CreatePieChartArgs {
    /// Center X.
    pub cx: f64,
    /// Center Y.
    pub cy: f64,
    /// Radius (default: 80).
    pub radius: Option<f64>,
    /// Data values — each slice is proportional to its value.
    pub values: Vec<f64>,
    /// Slice colors as hex strings. Cycles if fewer than values.
    #[serde(default)]
    pub colors: Vec<String>,
    /// Labels for each slice (optional).
    #[serde(default)]
    pub labels: Vec<String>,
    /// Inner radius for donut chart (default: 0 = solid pie).
    pub inner_radius: Option<f64>,
    pub layer_id: Option<String>,
}

/// Shape type for `create_parametric_shape`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParametricShapeType {
    /// Lissajous curve: x = A·sin(a·t + delta), y = B·sin(b·t).
    Lissajous,
    /// Superellipse (Lamé curve): |x/a|^n + |y/b|^n = 1.
    Superellipse,
    /// Rose curve (polar): r = cos(k·θ).
    Rose,
    /// Hypotrochoid: x = (R-r)·cos(t) + d·cos((R-r)/r·t), y = (R-r)·sin(t) - d·sin((R-r)/r·t).
    Hypotrochoid,
    /// Epicycloid: (R+r) circle rolling outside base circle R. d = r for standard epicycloid.
    Epicycloid,
}

/// Arguments for `create_parametric_shape` tool
#[derive(Debug, Deserialize)]
pub struct CreateParametricShapeArgs {
    /// Center X.
    pub cx: f64,
    /// Center Y.
    pub cy: f64,
    /// Shape type.
    pub shape_type: ParametricShapeType,
    /// Overall scale / outer radius (default: 80).
    pub radius: Option<f64>,
    /// Lissajous/Superellipse: X semi-axis ratio relative to radius (default: 1.0).
    pub ratio_x: Option<f64>,
    /// Lissajous/Superellipse: Y semi-axis ratio relative to radius (default: 1.0).
    pub ratio_y: Option<f64>,
    /// Lissajous: frequency ratio `a` (default: 3).
    pub freq_a: Option<f64>,
    /// Lissajous: frequency ratio `b` (default: 2).
    pub freq_b: Option<f64>,
    /// Lissajous: phase delta in degrees (default: 90).
    pub delta_deg: Option<f64>,
    /// Superellipse: exponent n (default: 2.5; 2 = ellipse, >2 = squircle, <2 = astroid-like).
    pub exponent: Option<f64>,
    /// Rose: number of petals k. Even k → 2k petals, odd k → k petals (default: 5).
    pub petals: Option<f64>,
    /// Hypotrochoid/Epicycloid: rolling circle radius r as fraction of R (default: 0.4).
    pub inner_ratio: Option<f64>,
    /// Hypotrochoid: pen distance d as fraction of r (default: 1.0 = standard hypocycloid).
    pub pen_ratio: Option<f64>,
    /// Number of sample points to generate the path (default: 360).
    pub points: Option<usize>,
    /// Fill style.
    pub fill: Option<FillArg>,
    /// Stroke style.
    pub stroke: Option<StrokeArg>,
    pub layer_id: Option<String>,
}

/// Arguments for `create_stacked_bar_chart` tool
#[derive(Debug, Deserialize)]
pub struct CreateStackedBarChartArgs {
    /// Left X (vertical) or top-left X (horizontal).
    pub x: f64,
    /// Bottom Y (vertical bars grow upward) or top Y (horizontal bars grow rightward).
    pub y: f64,
    /// Total chart width (default: 300).
    pub width: Option<f64>,
    /// Total chart height (default: 200).
    pub height: Option<f64>,
    /// Data series. Each series is one dataset (e.g. one category). All series must have the same length.
    pub series: Vec<Vec<f64>>,
    /// Colors for each series as hex. Cycles if fewer than series count.
    #[serde(default)]
    pub colors: Vec<String>,
    /// Labels for each stack position (column/row). Optional.
    #[serde(default)]
    pub labels: Vec<String>,
    /// Series names for node labeling. Optional.
    #[serde(default)]
    pub series_names: Vec<String>,
    /// Gap between stacks as fraction of bar width (default: 0.2).
    pub gap: Option<f64>,
    /// If true, bars are horizontal instead of vertical (default: false).
    #[serde(default)]
    pub horizontal: bool,
    pub layer_id: Option<String>,
}

/// Arguments for `create_radar_chart` tool
#[derive(Debug, Deserialize)]
pub struct CreateRadarChartArgs {
    /// Center X.
    pub cx: f64,
    /// Center Y.
    pub cy: f64,
    /// Outer radius of the chart (default: 100).
    pub radius: Option<f64>,
    /// Data series. Each series is an array of values, one per axis. All series must have equal length.
    pub series: Vec<Vec<f64>>,
    /// Axis labels (one per axis). Optional.
    #[serde(default)]
    pub labels: Vec<String>,
    /// Series names for legend / node labeling. Optional.
    #[serde(default)]
    pub series_names: Vec<String>,
    /// Fill colors for each series as hex. Cycles if fewer than series.
    #[serde(default)]
    pub colors: Vec<String>,
    /// Stroke width for series polygons (default: 1.5).
    pub stroke_width: Option<f64>,
    /// Number of concentric grid rings (default: 4).
    pub grid_rings: Option<usize>,
    /// Fill series polygons with semi-transparent color (default: true).
    #[serde(default = "default_true_fn")]
    pub fill_area: bool,
    pub layer_id: Option<String>,
}

// ─── CreateTruchetTilingArgs ──────────────────────────────────────────────────

/// Arguments for `create_truchet_tiling` tool
#[derive(Debug, Deserialize)]
pub struct CreateTruchetTilingArgs {
    /// Top-left X of the tiling region.
    pub x: f64,
    /// Top-left Y of the tiling region.
    pub y: f64,
    /// Width of the tiling region (default: 200).
    pub width: Option<f64>,
    /// Height of the tiling region (default: 200).
    pub height: Option<f64>,
    /// Size of each individual tile (default: 40). Minimum 4.
    pub tile_size: Option<f64>,
    /// Tile style: "arcs" (default, quarter-circle arcs), "diagonals" (straight diagonal), or "triangles".
    #[serde(default)]
    pub style: Option<String>,
    /// Random seed for reproducible tilings (default: 42).
    pub seed: Option<u64>,
    /// Stroke/fill color for tile patterns as hex (default: "#1a1a2e").
    #[serde(default)]
    pub color: Option<String>,
    /// Background fill color as hex. If absent, no background rectangle is added.
    #[serde(default)]
    pub background: Option<String>,
    /// Stroke width for arc/diagonal styles (default: 2.0).
    pub stroke_width: Option<f64>,
    pub layer_id: Option<String>,
}

/// Arguments for `point_on_path` tool
#[derive(Debug, Deserialize)]
pub struct PointOnPathArgs {
    /// Path node ID.
    pub node_id: String,
    /// Position(s) along the path as fractions 0.0–1.0. Can be a single value or array.
    pub t: Vec<f64>,
}

/// Arguments for `create_speech_bubble` tool
#[derive(Debug, Deserialize)]
pub struct CreateSpeechBubbleArgs {
    /// Center X of the bubble body.
    pub cx: f64,
    /// Center Y of the bubble body.
    pub cy: f64,
    /// Bubble body width (default: 120).
    pub width: Option<f64>,
    /// Bubble body height (default: 60).
    pub height: Option<f64>,
    /// Corner radius (default: 15).
    pub corner_radius: Option<f64>,
    /// Tail tip X coordinate (where the tail points to).
    pub tail_x: Option<f64>,
    /// Tail tip Y coordinate.
    pub tail_y: Option<f64>,
    /// Tail width at the base (default: 20).
    pub tail_width: Option<f64>,
    #[serde(default)]
    pub fill: Option<FillArg>,
    #[serde(default)]
    pub stroke: Option<StrokeArg>,
    pub layer_id: Option<String>,
}

/// Arguments for `set_visibility` tool
#[derive(Debug, Deserialize)]
pub struct SetVisibilityArgs {
    /// Node IDs. Empty = use selection.
    #[serde(default)]
    pub node_ids: Vec<String>,
    /// Visible state. Omit to toggle.
    pub visible: Option<bool>,
}

/// Arguments for `set_locked` tool
#[derive(Debug, Deserialize)]
pub struct SetLockedArgs {
    /// Node IDs. Empty = use selection.
    #[serde(default)]
    pub node_ids: Vec<String>,
    /// Locked state. Omit to toggle.
    pub locked: Option<bool>,
}

/// Arguments for `select_all` tool
#[derive(Debug, Deserialize, Default)]
pub struct SelectAllArgs {
    /// If provided, only select nodes on this layer (UUID or name).
    pub layer_id: Option<String>,
}

/// Arguments for `deselect_all` tool (no arguments needed).
#[derive(Debug, Deserialize, Default)]
pub struct DeselectAllArgs {}

/// Arguments for `set_blend_mode` tool
#[derive(Debug, Deserialize)]
pub struct SetBlendModeArgs {
    /// Node IDs. Empty = use selection.
    #[serde(default)]
    pub node_ids: Vec<String>,
    /// Blend mode name: normal, multiply, screen, overlay, darken, lighten, color_dodge, color_burn, hard_light, soft_light, difference, exclusion, hue, saturation, color, luminosity.
    pub blend_mode: String,
}

/// Arguments for `set_opacity` tool
#[derive(Debug, Deserialize)]
pub struct SetOpacityArgs {
    /// Node IDs. Empty = use selection.
    #[serde(default)]
    pub node_ids: Vec<String>,
    /// Opacity value 0.0–1.0.
    pub opacity: f32,
}

/// Arguments for `randomize_colors` tool
#[derive(Debug, Deserialize)]
pub struct RandomizeColorsArgs {
    /// Node IDs. Empty = use selection.
    #[serde(default)]
    pub node_ids: Vec<String>,
    /// Color palette as hex strings. If omitted, generates random colors.
    #[serde(default)]
    pub palette: Vec<String>,
    /// Random seed (default: 42).
    pub seed: Option<u64>,
    /// Randomize fill colors (default: true).
    #[serde(default = "default_true_fn")]
    pub fill: bool,
    /// Randomize stroke colors (default: false).
    #[serde(default)]
    pub stroke: bool,
}

fn default_true_fn() -> bool {
    true
}

/// Arguments for `duplicate_layer` tool
#[derive(Debug, Deserialize)]
pub struct DuplicateLayerArgs {
    /// Layer ID (UUID or name) to duplicate.
    pub layer_id: String,
    /// Name for the duplicated layer. Defaults to "<original name> Copy".
    pub name: Option<String>,
}

/// Arguments for `swap_fill_stroke` tool
#[derive(Debug, Deserialize, Default)]
pub struct SwapFillStrokeArgs {
    /// Node IDs. Empty = use selection.
    #[serde(default)]
    pub node_ids: Vec<String>,
}

/// Arguments for `resize_canvas` tool
#[derive(Debug, Deserialize)]
pub struct ResizeCanvasArgs {
    /// New canvas width.
    pub width: f64,
    /// New canvas height.
    pub height: f64,
}

/// Arguments for `create_heart` tool
#[derive(Debug, Deserialize)]
pub struct CreateHeartArgs {
    /// Center X.
    pub cx: f64,
    /// Center Y (bottom tip of heart).
    pub cy: f64,
    /// Heart size (width, default: 60).
    pub size: Option<f64>,
    #[serde(default)]
    pub fill: Option<FillArg>,
    #[serde(default)]
    pub stroke: Option<StrokeArg>,
    pub layer_id: Option<String>,
}

/// Arguments for `create_gear` tool
#[derive(Debug, Deserialize)]
pub struct CreateGearArgs {
    /// Center X.
    pub cx: f64,
    /// Center Y.
    pub cy: f64,
    /// Outer radius (tip of teeth, default: 50).
    pub outer_radius: Option<f64>,
    /// Inner radius (base of teeth, default: 35).
    pub inner_radius: Option<f64>,
    /// Hole radius (center hole, default: 10). Set to 0 for no hole.
    pub hole_radius: Option<f64>,
    /// Number of teeth (default: 12).
    pub teeth: Option<usize>,
    #[serde(default)]
    pub fill: Option<FillArg>,
    #[serde(default)]
    pub stroke: Option<StrokeArg>,
    pub layer_id: Option<String>,
}

/// Arguments for `flip_nodes` tool
#[derive(Debug, Deserialize)]
pub struct FlipNodesArgs {
    /// Node IDs to flip. Empty = use current selection.
    #[serde(default)]
    pub node_ids: Vec<String>,
    /// Flip axis: "horizontal" (mirror across vertical axis) or "vertical" (mirror across horizontal axis).
    pub axis: String,
}

/// Arguments for `create_cross` tool
#[derive(Debug, Deserialize)]
pub struct CreateCrossArgs {
    /// Center X.
    pub cx: f64,
    /// Center Y.
    pub cy: f64,
    /// Total size (width and height of the cross, default: 60).
    pub size: Option<f64>,
    /// Arm thickness (default: 20).
    pub thickness: Option<f64>,
    /// Rotation in degrees (default: 0). Set to 45 for an X shape.
    pub rotation: Option<f64>,
    #[serde(default)]
    pub fill: Option<FillArg>,
    #[serde(default)]
    pub stroke: Option<StrokeArg>,
    pub layer_id: Option<String>,
}

/// Arguments for `measure_path` tool
#[derive(Debug, Deserialize)]
pub struct MeasurePathArgs {
    /// Path node ID to measure.
    pub node_id: String,
}

/// Arguments for `measure_distance` tool
#[derive(Debug, Deserialize)]
pub struct MeasureDistanceArgs {
    /// First point [x, y] or node ID.
    pub from: MeasureTarget,
    /// Second point [x, y] or node ID.
    pub to: MeasureTarget,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum MeasureTarget {
    Point([f64; 2]),
    NodeId(String),
}

/// Arguments for `create_arrow_shape` tool
#[derive(Debug, Deserialize)]
pub struct CreateArrowShapeArgs {
    /// Arrow tip X.
    pub x: f64,
    /// Arrow tip Y.
    pub y: f64,
    /// Total arrow length (default: 100).
    pub length: Option<f64>,
    /// Arrow head width (default: 40).
    pub head_width: Option<f64>,
    /// Arrow head depth as fraction of length (default: 0.4).
    pub head_depth: Option<f64>,
    /// Shaft width (default: 16).
    pub shaft_width: Option<f64>,
    /// Direction in degrees. 0 = pointing right. (default: 0).
    pub direction: Option<f64>,
    #[serde(default)]
    pub fill: Option<FillArg>,
    #[serde(default)]
    pub stroke: Option<StrokeArg>,
    pub layer_id: Option<String>,
}

/// Arguments for `create_donut` tool
#[derive(Debug, Deserialize)]
pub struct CreateDonutArgs {
    /// Center X.
    pub cx: f64,
    /// Center Y.
    pub cy: f64,
    /// Outer radius (default: 50).
    pub outer_radius: Option<f64>,
    /// Inner radius (default: 25). Creates the hole.
    pub inner_radius: Option<f64>,
    /// Start angle in degrees for partial arcs (default: 0). 0 = full ring.
    pub start_angle: Option<f64>,
    /// End angle in degrees for partial arcs (default: 360). Set < 360 for a partial donut.
    pub end_angle: Option<f64>,
    #[serde(default)]
    pub fill: Option<FillArg>,
    #[serde(default)]
    pub stroke: Option<StrokeArg>,
    pub layer_id: Option<String>,
}

/// Arguments for `create_sunburst` tool
#[derive(Debug, Deserialize)]
pub struct CreateSunburstArgs {
    /// Center X coordinate.
    pub cx: f64,
    /// Center Y coordinate.
    pub cy: f64,
    /// Inner radius (default: 20).
    pub inner_radius: Option<f64>,
    /// Outer radius (default: 100).
    pub outer_radius: Option<f64>,
    /// Number of rays (default: 24). Must be even for alternating wedges.
    pub rays: Option<usize>,
    /// Fill color hex for the rays (default: "#FFD700" gold).
    pub color: Option<String>,
    /// Layer ID.
    pub layer_id: Option<String>,
}

/// Arguments for `create_wave_pattern` tool
#[derive(Debug, Deserialize)]
pub struct CreateWavePatternArgs {
    /// Left X coordinate.
    pub x: f64,
    /// Top Y coordinate.
    pub y: f64,
    /// Pattern width.
    pub width: f64,
    /// Pattern height.
    pub height: f64,
    /// Number of wave lines (default: 8).
    pub lines: Option<usize>,
    /// Wavelength in document units (default: 40).
    pub wavelength: Option<f64>,
    /// Amplitude in document units (default: 10).
    pub amplitude: Option<f64>,
    /// Stroke style.
    #[serde(default)]
    pub stroke: Option<StrokeArg>,
    /// Layer ID.
    pub layer_id: Option<String>,
}

/// Arguments for `hatch_fill` tool
#[derive(Debug, Deserialize)]
pub struct HatchFillArgs {
    /// Path node IDs to fill with hatching.
    pub node_ids: Vec<String>,
    /// Spacing between hatch lines (default: 5.0).
    pub spacing: Option<f64>,
    /// Angle of hatch lines in degrees (default: 45).
    pub angle: Option<f64>,
    /// Second angle for cross-hatching. Omit for single-direction hatching.
    pub cross_angle: Option<f64>,
    /// Line stroke width (default: 1.0).
    pub stroke_width: Option<f64>,
    /// Line color hex (default: uses path fill color).
    pub color: Option<String>,
}

/// Arguments for `stipple_fill` tool
#[derive(Debug, Deserialize)]
pub struct StippleFillArgs {
    /// Path node IDs — the shapes to fill with stipple dots.
    pub node_ids: Vec<String>,
    /// Number of dots to place (default: 200).
    pub count: Option<usize>,
    /// Dot radius in document units (default: 1.5).
    pub dot_radius: Option<f64>,
    /// Dot color hex (default: uses the path's fill color).
    pub color: Option<String>,
    /// Random seed (default: 42).
    pub seed: Option<u64>,
}

/// Arguments for `add_drop_shadow` tool
#[derive(Debug, Deserialize)]
pub struct AddDropShadowArgs {
    /// Node IDs to add shadows to.
    pub node_ids: Vec<String>,
    /// Shadow X offset (default: 5).
    pub offset_x: Option<f64>,
    /// Shadow Y offset (default: 5).
    pub offset_y: Option<f64>,
    /// Shadow color as hex (default: "#000000").
    pub color: Option<String>,
    /// Shadow opacity 0–1 (default: 0.4).
    pub opacity: Option<f32>,
}

/// Arguments for `transform_copies` tool
#[derive(Debug, Deserialize)]
pub struct TransformCopiesArgs {
    /// Source node ID to copy.
    pub node_id: String,
    /// Number of copies to create (default: 5).
    pub copies: Option<usize>,
    /// X translation offset per copy (default: 0).
    #[serde(default)]
    pub translate_x: Option<f64>,
    /// Y translation offset per copy (default: 0).
    #[serde(default)]
    pub translate_y: Option<f64>,
    /// Rotation per copy in degrees (default: 0).
    #[serde(default)]
    pub rotate: Option<f64>,
    /// Scale factor per copy (default: 1.0 = no scaling). 0.9 = shrink 10% each copy.
    #[serde(default)]
    pub scale: Option<f64>,
    /// Opacity change per copy (multiplied cumulatively, default: 1.0).
    #[serde(default)]
    pub opacity_step: Option<f32>,
}

/// Arguments for `round_corners` tool
#[derive(Debug, Deserialize)]
pub struct RoundCornersArgs {
    /// Path node IDs.
    pub node_ids: Vec<String>,
    /// Corner radius in document units (default: 10).
    pub radius: Option<f64>,
}

/// Arguments for `create_flare` tool
#[derive(Debug, Deserialize)]
pub struct CreateFlareArgs {
    /// Center X coordinate.
    pub cx: f64,
    /// Center Y coordinate.
    pub cy: f64,
    /// Halo radius in document units (default: 50).
    pub halo_radius: Option<f64>,
    /// Number of radiating rays (default: 12).
    pub ray_count: Option<usize>,
    /// Length of rays beyond the halo (default: 80).
    pub ray_length: Option<f64>,
    /// Number of concentric rings (default: 3).
    pub ring_count: Option<usize>,
    /// Halo color as hex (default: "#fffbe6" warm yellow).
    pub halo_color: Option<String>,
    /// Ray opacity 0.0–1.0 (default: 0.3).
    pub ray_opacity: Option<f32>,
    /// Layer ID (default: active layer).
    pub layer_id: Option<String>,
}

/// Arguments for `warp_envelope` tool
#[derive(Debug, Deserialize)]
pub struct WarpEnvelopeArgs {
    /// Path node IDs to warp.
    pub node_ids: Vec<String>,
    /// Warp preset: "arc", "bulge", "wave", "flag", "squeeze", "inflate", "fisheye".
    pub warp_type: String,
    /// Primary bend amount (-1.0 to 1.0, default: 0.5). Positive = bend down/right.
    pub bend: Option<f64>,
    /// Horizontal distortion (-1.0 to 1.0, default: 0). Only used by some presets.
    pub distort_h: Option<f64>,
    /// Vertical distortion (-1.0 to 1.0, default: 0). Only used by some presets.
    pub distort_v: Option<f64>,
}

/// Arguments for `crystallize_path` tool
#[derive(Debug, Deserialize)]
pub struct CrystallizePathArgs {
    /// Path node IDs to crystallize.
    pub node_ids: Vec<String>,
    /// Height of each spike in document units (default: 10).
    pub size: Option<f64>,
    /// Number of spikes per original segment (default: 3).
    pub count: Option<usize>,
}

/// Arguments for `scallop_path` tool
#[derive(Debug, Deserialize)]
pub struct ScallopPathArgs {
    /// Path node IDs to apply scallop to.
    pub node_ids: Vec<String>,
    /// Depth of each scallop arc in document units (default: 10). Positive = inward.
    pub depth: Option<f64>,
    /// Number of scallop arcs per original segment (default: 1).
    pub count: Option<usize>,
}

/// Arguments for `blend_objects` tool
#[derive(Debug, Deserialize)]
pub struct BlendObjectsArgs {
    /// First (start) path node ID.
    pub node_id_a: String,
    /// Second (end) path node ID.
    pub node_id_b: String,
    /// Number of intermediate steps to generate (default: 5). Ignored when `smooth_color` is true or `spacing` is set.
    pub steps: Option<usize>,
    /// Smooth Color mode: auto-compute steps so each step changes color by ≤ 1 RGB unit (0–255 scale).
    /// When true, `steps` is ignored.
    #[serde(default)]
    pub smooth_color: bool,
    /// Specified Distance mode: space blend steps by this many pixels (world-space).
    /// Steps = ceil(center_distance / spacing). When set, `steps` and `smooth_color` are ignored.
    pub spacing: Option<f64>,
}

/// Arguments for `twirl_path` tool
#[derive(Debug, Deserialize)]
pub struct TwirlPathArgs {
    /// Path node IDs to twirl.
    pub node_ids: Vec<String>,
    /// Rotation angle in degrees. Positive = counter-clockwise. Default: 90.
    pub angle: Option<f64>,
    /// X coordinate of twirl center. Defaults to path centroid.
    pub center_x: Option<f64>,
    /// Y coordinate of twirl center. Defaults to path centroid.
    pub center_y: Option<f64>,
}

/// Arguments for `roughen_path` tool
#[derive(Debug, Deserialize)]
pub struct RoughenPathArgs {
    /// Path node IDs to roughen.
    pub node_ids: Vec<String>,
    /// Maximum displacement in document units (default: 5.0).
    pub size: Option<f64>,
    /// Number of points to add per segment before roughening (0 = roughen existing points only). Default: 0.
    pub detail: Option<usize>,
    /// Random seed for reproducible results. Default: 42.
    pub seed: Option<u64>,
}

/// Arguments for `create_curvature_path` tool
#[derive(Debug, Deserialize)]
pub struct CreateCurvaturePathArgs {
    /// Ordered array of `[x, y]` canvas-space points the curve passes through.
    pub points: Vec<[f64; 2]>,
    /// If true, close the curve back to the first point (default: false).
    #[serde(default)]
    pub closed: bool,
    #[serde(default)]
    pub fill: Option<FillArg>,
    #[serde(default)]
    pub stroke: Option<StrokeArg>,
    /// Layer to place the node on. Defaults to the active layer.
    pub layer_id: Option<String>,
}

/// Arguments for `delete_anchor_point` tool
#[derive(Debug, Deserialize)]
pub struct DeleteAnchorPointArgs {
    /// Path node ID (UUID or name).
    pub node_id: String,
    /// Zero-based indices of BezPath elements to remove.
    pub anchor_indices: Vec<usize>,
}

/// Arguments for `export_raster` tool
#[derive(Debug, Deserialize, Default)]
pub struct ExportRasterArgs {
    /// Output format: "png" (default), "jpeg", "webp", "gif", or "tiff".
    #[serde(default)]
    pub format: Option<String>,
    /// Output width in pixels. Defaults to document width.
    pub width: Option<u32>,
    /// Output height in pixels. Defaults to document height.
    pub height: Option<u32>,
    /// JPEG quality 1–100 (default: 90). Ignored for PNG.
    pub quality: Option<u8>,
}

/// Arguments for `create_shape` tool
#[derive(Debug, Deserialize)]
pub struct CreateShapeArgs {
    pub shape_type: ShapeType,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    #[serde(default)]
    pub rx: Option<f64>,
    #[serde(default)]
    pub sides: Option<usize>,
    #[serde(default)]
    pub inner_radius: Option<f64>,
    /// Corner radius for rounded_rect shapes (default: 10.0).
    #[serde(default)]
    pub corner_radius: Option<f64>,
    /// Arc start angle in degrees (0° = 3 o'clock). Default 0.
    #[serde(default)]
    pub arc_start_angle: Option<f64>,
    /// Arc end angle in degrees. Default 270 (¾ circle).
    #[serde(default)]
    pub arc_end_angle: Option<f64>,
    /// If true, draw an open arc (no chord back to center). Default false = closed pie.
    #[serde(default)]
    pub arc_open: Option<bool>,
    #[serde(default)]
    pub fill: Option<FillArg>,
    #[serde(default)]
    pub stroke: Option<StrokeArg>,
    #[serde(default)]
    pub layer_id: Option<Uuid>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

pub use photonic_core::PrimitiveKind as ShapeType;

fn default_spiral_segs() -> usize {
    16
}

/// Arguments for `create_spiral` tool
#[derive(Debug, Deserialize)]
pub struct CreateSpiralArgs {
    /// X coordinate of the spiral center.
    pub x: f64,
    /// Y coordinate of the spiral center.
    pub y: f64,
    /// Outer (maximum) radius in document units.
    pub outer_radius: f64,
    /// Inner (minimum) radius. Use 0 for a true spiral from the center.
    #[serde(default)]
    pub inner_radius: f64,
    /// Number of full rotations (e.g. 3.0 = three turns).
    pub turns: f64,
    /// Cubic Bézier segments per full turn (default 16; higher = smoother).
    #[serde(default = "default_spiral_segs")]
    pub segments_per_turn: usize,
    #[serde(default)]
    pub fill: Option<FillArg>,
    #[serde(default)]
    pub stroke: Option<StrokeArg>,
    #[serde(default)]
    pub layer_id: Option<Uuid>,
    #[serde(default)]
    pub name: Option<String>,
}

/// Arguments for `create_polar_grid` tool
#[derive(Debug, Deserialize)]
pub struct CreatePolarGridArgs {
    /// X coordinate of the center.
    pub x: f64,
    /// Y coordinate of the center.
    pub y: f64,
    /// Outer (maximum) radius in document units.
    pub outer_radius: f64,
    /// Inner (minimum) radius. Use 0 for a full-disk polar grid (default: 0).
    #[serde(default)]
    pub inner_radius: Option<f64>,
    /// Number of concentric rings (default: 4).
    #[serde(default)]
    pub rings: Option<u32>,
    /// Number of radial sector dividers (default: 8).
    #[serde(default)]
    pub sectors: Option<u32>,
    #[serde(default)]
    pub fill: Option<FillArg>,
    #[serde(default)]
    pub stroke: Option<StrokeArg>,
    #[serde(default)]
    pub layer_id: Option<Uuid>,
    #[serde(default)]
    pub name: Option<String>,
}

/// Arguments for `create_grid` tool
#[derive(Debug, Deserialize)]
pub struct CreateGridArgs {
    /// X coordinate of the top-left corner.
    pub x: f64,
    /// Y coordinate of the top-left corner.
    pub y: f64,
    /// Total width of the grid.
    pub width: f64,
    /// Total height of the grid.
    pub height: f64,
    /// Number of columns (cell divisions horizontally). Default 4.
    #[serde(default)]
    pub cols: Option<u32>,
    /// Number of rows (cell divisions vertically). Default 4.
    #[serde(default)]
    pub rows: Option<u32>,
    #[serde(default)]
    pub fill: Option<FillArg>,
    #[serde(default)]
    pub stroke: Option<StrokeArg>,
    #[serde(default)]
    pub layer_id: Option<Uuid>,
    #[serde(default)]
    pub name: Option<String>,
}

/// Arguments for `create_path` tool
#[derive(Debug, Deserialize)]
pub struct CreatePathArgs {
    pub path_data: String,
    #[serde(default)]
    pub fill: Option<FillArg>,
    #[serde(default)]
    pub stroke: Option<StrokeArg>,
    #[serde(default)]
    pub layer_id: Option<Uuid>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub transform: Option<TransformArg>,
}

/// Arguments for `create_text` tool
#[derive(Debug, Deserialize)]
pub struct CreateTextArgs {
    pub content: String,
    pub x: f64,
    pub y: f64,
    #[serde(default)]
    pub font_family: Option<String>,
    #[serde(default)]
    pub font_size: Option<f64>,
    #[serde(default)]
    pub font_weight: Option<u16>,
    #[serde(default)]
    pub fill: Option<FillArg>,
    #[serde(default)]
    pub stroke: Option<StrokeArg>,
    /// "left" | "center" | "right"
    #[serde(default)]
    pub align: Option<String>,
    /// Line height multiplier (default: 1.2). 1.0 = tight, 2.0 = double-spaced.
    #[serde(default)]
    pub line_height: Option<f64>,
    /// Letter spacing in document units (default: 0.0). Positive = wider, negative = tighter.
    #[serde(default)]
    pub letter_spacing: Option<f64>,
    #[serde(default)]
    pub layer_id: Option<Uuid>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Glow effect parameters for `update_node`.
#[derive(Debug, Deserialize)]
pub struct GlowEffectArg {
    pub enabled: bool,
    /// Glow color as `[r, g, b, a]` in 0.0–1.0.
    pub color: [f32; 4],
    /// Overall opacity multiplier 0.0–1.0.
    pub opacity: f32,
    /// Glow spread radius in document units.
    pub size: f32,
    /// Corner join style: `"miter"` (default), `"round"`, or `"bevel"`.
    #[serde(default)]
    pub join: LineJoin,
}

impl From<GlowEffectArg> for GlowEffect {
    fn from(a: GlowEffectArg) -> Self {
        Self {
            enabled: a.enabled,
            color: Color {
                r: a.color[0],
                g: a.color[1],
                b: a.color[2],
                a: a.color[3],
            },
            opacity: a.opacity,
            size: a.size,
            join: a.join,
        }
    }
}

/// Gaussian glow effect parameters for `update_node`.
#[derive(Debug, Deserialize)]
pub struct GaussianGlowArg {
    pub enabled: bool,
    pub color: [f32; 4],
    pub opacity: f32,
    /// Blur radius (sigma) in document units.
    pub radius: f32,
}

impl From<GaussianGlowArg> for GaussianGlow {
    fn from(a: GaussianGlowArg) -> Self {
        Self {
            enabled: a.enabled,
            color: Color {
                r: a.color[0],
                g: a.color[1],
                b: a.color[2],
                a: a.color[3],
            },
            opacity: a.opacity,
            radius: a.radius,
        }
    }
}

/// Arguments for `update_node` tool
#[derive(Debug, Deserialize)]
pub struct UpdateNodeArgs {
    pub node_id: Uuid,
    #[serde(default)]
    pub fill: Option<FillArg>,
    #[serde(default)]
    pub stroke: Option<StrokeArg>,
    #[serde(default)]
    pub transform: Option<TransformArg>,
    #[serde(default)]
    pub opacity: Option<f32>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub visible: Option<bool>,
    #[serde(default)]
    pub locked: Option<bool>,
    #[serde(default)]
    pub blend_mode: Option<BlendMode>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    // ── Text-node specific ────────────────────────────────────────────────────
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub font_family: Option<String>,
    #[serde(default)]
    pub font_size: Option<f64>,
    #[serde(default)]
    pub font_weight: Option<u16>,
    /// "left" | "center" | "right"
    #[serde(default)]
    pub text_align: Option<String>,
    #[serde(default)]
    pub outer_glow: Option<GlowEffectArg>,
    #[serde(default)]
    pub inner_glow: Option<GlowEffectArg>,
    #[serde(default)]
    pub gaussian_glow: Option<GaussianGlowArg>,
}

/// Arguments for `apply_transform` tool
#[derive(Debug, Deserialize)]
pub struct ApplyTransformArgs {
    #[serde(default)]
    pub node_ids: Vec<Uuid>,
    pub operation: TransformOperation,
    #[serde(default)]
    pub translate: Option<TranslateArg>,
    #[serde(default)]
    pub rotate: Option<RotateArg>,
    #[serde(default)]
    pub scale: Option<ScaleArg>,
    #[serde(default)]
    pub matrix: Option<[f64; 6]>,
    #[serde(default)]
    pub shear: Option<ShearArg>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransformOperation {
    Translate,
    Rotate,
    Scale,
    Matrix,
    ReflectHorizontal,
    ReflectVertical,
    Shear,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TranslateArg {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RotateArg {
    pub angle_degrees: f64,
    #[serde(default)]
    pub origin_x: f64,
    #[serde(default)]
    pub origin_y: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScaleArg {
    pub sx: f64,
    pub sy: f64,
    #[serde(default)]
    pub origin_x: f64,
    #[serde(default)]
    pub origin_y: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ShearArg {
    /// Horizontal shear factor: shifts x by `shear_x * y`.
    pub shear_x: f64,
    /// Vertical shear factor: shifts y by `shear_y * x`.
    #[serde(default)]
    pub shear_y: f64,
    /// X coordinate of the shear origin (default: 0).
    #[serde(default)]
    pub origin_x: f64,
    /// Y coordinate of the shear origin (default: 0).
    #[serde(default)]
    pub origin_y: f64,
}

/// Arguments for `create_layer` tool
#[derive(Debug, Deserialize)]
pub struct CreateLayerArgs {
    pub name: String,
    #[serde(default)]
    pub position: Option<usize>,
}

/// Arguments for `collect_in_new_layer` tool
#[derive(Debug, Deserialize)]
pub struct CollectInNewLayerArgs {
    /// IDs of nodes to collect. Group children are resolved to their top-level ancestor.
    pub node_ids: Vec<Uuid>,
    /// Name for the new layer (default: "Collected Layer").
    #[serde(default)]
    pub name: Option<String>,
    /// Position in the layer stack (0 = top/front; 1 = just below top). Defaults to top of stack.
    #[serde(default)]
    pub position: Option<usize>,
}

/// Arguments for `get_document_state` tool
#[derive(Debug, Deserialize, Default)]
pub struct GetDocumentStateArgs {
    #[serde(default)]
    pub include_path_data: bool,
    #[serde(default)]
    pub layer_id: Option<Uuid>,
    /// When true, return only {id, name, kind, z_index} per node — no styles or transforms.
    /// Use this when you only need to know what nodes exist, not their appearance.
    #[serde(default)]
    pub summary_only: bool,
}

/// Arguments for `get_node` tool
#[derive(Debug, Deserialize)]
pub struct GetNodeArgs {
    #[serde(default)]
    pub node_id: Option<Uuid>,
    #[serde(default)]
    pub name: Option<String>,
}

/// Arguments for `screenshot` tool
#[derive(Debug, Deserialize, Default)]
pub struct ScreenshotArgs {
    #[serde(default)]
    pub scale: Option<f32>,
    #[serde(default)]
    pub region: Option<RegionArg>,
}

#[derive(Debug, Deserialize)]
pub struct RegionArg {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Arguments for `export` tool
#[derive(Debug, Deserialize)]
pub struct ExportArgs {
    pub format: ExportFormat,
    pub file_path: String,
    #[serde(default)]
    pub node_ids: Vec<Uuid>,
    #[serde(default)]
    pub dpi: Option<f64>,
    #[serde(default)]
    pub scale: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    Svg,
    Png,
    Jpeg,
    Pdf,
}

/// Arguments for `delete_node` tool
#[derive(Debug, Deserialize)]
pub struct DeleteNodeArgs {
    pub node_ids: Vec<Uuid>,
}

/// Arguments for `undo`/`redo` tools
#[derive(Debug, Deserialize, Default)]
pub struct UndoRedoArgs {
    #[serde(default)]
    pub steps: Option<usize>,
}

/// Arguments for `create_checkpoint` tool
#[derive(Debug, Deserialize)]
pub struct CreateCheckpointArgs {
    pub name: String,
}

/// Arguments for `restore_checkpoint` tool
#[derive(Debug, Deserialize)]
pub struct RestoreCheckpointArgs {
    pub checkpoint_id: String,
}

/// Arguments for `diff_checkpoints` tool
#[derive(Debug, Deserialize)]
pub struct DiffCheckpointsArgs {
    /// UUID of the "from" (older/baseline) checkpoint.
    pub from_id: String,
    /// UUID of the "to" (newer/current) checkpoint.
    pub to_id: String,
}

/// Arguments for `reorder_node` tool
#[derive(Debug, Deserialize)]
pub struct ReorderNodeArgs {
    pub node_id: Uuid,
    pub operation: ReorderOperation,
    /// Required when operation is move_above or move_below.
    #[serde(default)]
    pub relative_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReorderOperation {
    SendToBack,
    BringToFront,
    SendBackward,
    BringForward,
    MoveAbove,
    MoveBelow,
}

/// Arguments for `group_nodes` tool
#[derive(Debug, Deserialize)]
pub struct GroupNodesArgs {
    pub node_ids: Vec<Uuid>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub layer_id: Option<Uuid>,
}

/// Arguments for `ungroup_nodes` tool
#[derive(Debug, Deserialize)]
pub struct UngroupNodesArgs {
    pub group_id: Uuid,
}

/// Arguments for `boolean_operation` tool
#[derive(Debug, Deserialize)]
pub struct BooleanOperationArgs {
    /// Base shape — result inherits its fill and stroke.
    pub target_id: Uuid,
    /// The cutting/combining shape (relevant for subtract).
    pub tool_id: Uuid,
    pub operation: BooleanOp,
    /// If true, original nodes are preserved alongside the result. Default: false.
    #[serde(default)]
    pub keep_originals: bool,
}

fn default_true() -> bool {
    true
}

/// Arguments for `build_shape_from_points` tool
#[derive(Debug, Deserialize)]
pub struct BuildShapeFromPointsArgs {
    /// Array of [x, y] coordinate pairs defining the available vertices.
    pub points: Vec<[f64; 2]>,
    /// Indices into `points` defining the connection sequence.
    /// If omitted, connects points in order 0 → 1 → 2 → … → n-1.
    /// Use this to connect them in any custom order, e.g. [0, 2, 1, 3].
    #[serde(default)]
    pub connection_order: Option<Vec<usize>>,
    /// Whether to close the path back to the first connected point (default: true).
    #[serde(default = "default_true")]
    pub closed: bool,
    #[serde(default)]
    pub fill: Option<FillArg>,
    #[serde(default)]
    pub stroke: Option<StrokeArg>,
    #[serde(default)]
    pub layer_id: Option<Uuid>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Arguments for `align_nodes` tool
#[derive(Debug, Deserialize)]
pub struct AlignNodesArgs {
    /// IDs of the nodes to align (at least 2).
    pub node_ids: Vec<Uuid>,
    /// The alignment or distribution operation to perform.
    pub operation: AlignOperation,
    /// What to align relative to. Defaults to `selection` (the combined bounding box of all
    /// specified nodes). Use `canvas` to align relative to the document bounds.
    /// Use `key_object` combined with `key_object_id` to align to a specific node.
    #[serde(default)]
    pub anchor: AlignAnchor,
    /// When `anchor` is `key_object`, this node's bounding box is used as the fixed reference.
    /// The key object itself is not moved. Must be one of the `node_ids`.
    #[serde(default)]
    pub key_object_id: Option<Uuid>,
    /// When using `distribute_horizontal` or `distribute_vertical`, place exactly this many
    /// pixels between adjacent node edges. When omitted, nodes are evenly spaced so the two
    /// extremes stay fixed (existing behaviour).
    pub spacing: Option<f64>,
}

#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum AlignOperation {
    /// Align left edges to the reference left edge.
    Left,
    /// Align horizontal centers to the reference horizontal center.
    CenterHorizontal,
    /// Align right edges to the reference right edge.
    Right,
    /// Align top edges to the reference top edge.
    Top,
    /// Align vertical centers to the reference vertical center.
    CenterVertical,
    /// Align bottom edges to the reference bottom edge.
    Bottom,
    /// Evenly space nodes horizontally (leftmost and rightmost nodes stay fixed).
    DistributeHorizontal,
    /// Evenly space nodes vertically (topmost and bottommost nodes stay fixed).
    DistributeVertical,
}

#[derive(Debug, Deserialize, Clone, Copy, Default)]
#[serde(rename_all = "snake_case")]
pub enum AlignAnchor {
    /// Use the combined bounding box of all specified nodes as the reference. (default)
    #[default]
    Selection,
    /// Use the document canvas (0, 0, width, height) as the reference.
    Canvas,
    /// Use the bounding box of the node identified by `key_object_id` as the fixed reference.
    /// The key object itself is not moved.
    KeyObject,
}

/// Arguments for `find_nodes` tool.
/// All fields optional; combine with AND logic.
#[derive(Debug, Deserialize, Default)]
pub struct FindNodesArgs {
    /// Node must have ALL of these tags.
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    /// Node must have ANY of these tags.
    #[serde(default)]
    pub tags_any: Option<Vec<String>>,
    /// Case-insensitive substring match on node name.
    #[serde(default)]
    pub name_contains: Option<String>,
    /// "path" | "group" | "text"
    #[serde(default)]
    pub node_type: Option<String>,
    #[serde(default)]
    pub layer_id: Option<Uuid>,
    /// If true, exclude invisible nodes (default: false).
    #[serde(default)]
    pub visible_only: Option<bool>,
    /// World-space AABB filter (reuses existing RegionArg).
    #[serde(default)]
    pub in_region: Option<RegionArg>,
    /// If true, return full node JSON; default false returns minimal summary.
    #[serde(default)]
    pub include_details: Option<bool>,
    /// Max results (default: 200).
    #[serde(default)]
    pub limit: Option<usize>,
}

/// Arguments for `duplicate_nodes` tool
#[derive(Debug, Deserialize)]
pub struct DuplicateNodesArgs {
    /// IDs of the nodes to duplicate.
    pub node_ids: Vec<Uuid>,
    /// How many copies to create per source node (default: 1, max: 100).
    #[serde(default)]
    pub count: Option<usize>,
    /// Position offset applied to each successive copy.
    /// Copy N is shifted by N * offset from the original.
    /// Default: {x: 10, y: 10}.
    #[serde(default)]
    pub offset: Option<TranslateArg>,
    /// Target layer for the copies. Defaults to the source node's layer.
    #[serde(default)]
    pub layer_id: Option<Uuid>,
}

/// Arguments for `create_array` tool — repeat a node in a grid or radial pattern.
#[derive(Debug, Deserialize)]
pub struct CreateArrayArgs {
    /// The source node to repeat. It stays in place; new copies are created around it.
    pub node_id: Uuid,
    /// Layout mode: `"grid"` or `"radial"`.
    pub mode: ArrayMode,

    // ── Grid params (ignored for radial) ─────────────────────────────────
    /// Number of rows in the grid (default 2). The source is row 0, col 0.
    #[serde(default)]
    pub rows: Option<usize>,
    /// Number of columns in the grid (default 2).
    #[serde(default)]
    pub cols: Option<usize>,
    /// Horizontal distance (px) between column centres (default 100).
    #[serde(default)]
    pub col_stride: Option<f64>,
    /// Vertical distance (px) between row centres (default 100).
    #[serde(default)]
    pub row_stride: Option<f64>,

    // ── Radial params (ignored for grid) ─────────────────────────────────
    /// Total number of instances including the source (default 6, min 2).
    /// The source counts as instance 0 — so `count = 6` creates 5 new copies.
    #[serde(default)]
    pub count: Option<usize>,
    /// X coordinate of the rotation centre (default 0).
    #[serde(default)]
    pub center_x: Option<f64>,
    /// Y coordinate of the rotation centre (default 0).
    #[serde(default)]
    pub center_y: Option<f64>,
    /// Angle in degrees at which the first copy is placed, measured clockwise
    /// from the source position. Remaining copies are evenly spread to fill
    /// 360°. Default: 0 (evenly distributed starting from the source angle).
    #[serde(default)]
    pub start_angle_degrees: Option<f64>,

    // ── Common ────────────────────────────────────────────────────────────
    /// If true, wrap all copies AND the source node into a new group. Default false.
    #[serde(default)]
    pub group_result: bool,
    /// Target layer for the copies. Defaults to the source node's layer.
    #[serde(default)]
    pub layer_id: Option<Uuid>,
    /// Name prefix for generated copies, e.g. "Petal" → "Petal 1", "Petal 2", …
    /// Defaults to the source node's name.
    #[serde(default)]
    pub name_prefix: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArrayMode {
    Grid,
    Radial,
}

/// Arguments for `style_transfer` tool
#[derive(Debug, Deserialize)]
pub struct StyleTransferArgs {
    /// The node whose visual style will be copied.
    pub source_id: Uuid,
    /// One or more nodes that will receive the style.
    pub target_ids: Vec<Uuid>,
    /// Which properties to copy. Valid values: "fill", "stroke", "opacity", "blend_mode".
    /// If omitted or empty, all four are copied.
    #[serde(default)]
    pub properties: Option<Vec<String>>,
}

/// Arguments for `find_replace_style` tool.
///
/// Searches every node in the document (or a scoped subset) for matching
/// style properties, then replaces them in a single undoable batch.
///
/// **Search criteria** (at least one required):
/// - `fill_color` / `stroke_color` — match by hex color with optional tolerance
/// - `stroke_width` — match by stroke width (fractional tolerance via `color_tolerance`)
/// - `font_family` — match text nodes by font family name (case-insensitive, exact)
///
/// **Replacements** (at least one required):
/// - `new_fill_color`, `new_stroke_color`, `new_opacity`
/// - `new_stroke_width`, `new_font_family`
///
/// Color matching works on solid fills **and** individual gradient stop /
/// fluid-point / mesh-vertex colors, so a gradient that uses the target color
/// in one stop will be partially updated.
#[derive(Debug, Deserialize, Default)]
pub struct FindReplaceStyleArgs {
    /// Hex color to search for in fills (solid or gradient stops). e.g. `"#FF0000"`.
    #[serde(default)]
    pub fill_color: Option<String>,
    /// Hex color to search for in strokes (enabled strokes only). e.g. `"#000000"`.
    #[serde(default)]
    pub stroke_color: Option<String>,
    /// Stroke width to search for (on enabled strokes). e.g. `2.0`.
    /// Fractional tolerance applies: `color_tolerance = 0.1` matches ±10% of this value.
    #[serde(default)]
    pub stroke_width: Option<f64>,
    /// Font family name to search for on text nodes (case-insensitive exact match).
    /// e.g. `"Inter"`.
    #[serde(default)]
    pub font_family: Option<String>,
    /// How similar a color must be to count as a match. `0.0` = exact (default),
    /// `1.0` = any color matches.  Distance is normalized Euclidean in linear
    /// RGB: `sqrt((r₁-r₂)² + (g₁-g₂)² + (b₁-b₂)²) / √3`.
    /// Also used as fractional tolerance for `stroke_width` matching.
    #[serde(default)]
    pub color_tolerance: Option<f32>,
    /// Replace every matched fill color (solid or stop) with this hex color.
    #[serde(default)]
    pub new_fill_color: Option<String>,
    /// Replace every matched stroke color with this hex color.
    #[serde(default)]
    pub new_stroke_color: Option<String>,
    /// Override the node-level opacity for every matched node. Range 0–1.
    #[serde(default)]
    pub new_opacity: Option<f32>,
    /// Replace every matched stroke width with this value (in document units).
    #[serde(default)]
    pub new_stroke_width: Option<f64>,
    /// Replace the font family on every matched text node.
    #[serde(default)]
    pub new_font_family: Option<String>,
    /// Restrict the search to nodes on this layer.
    #[serde(default)]
    pub layer_id: Option<Uuid>,
    /// Restrict the search to these specific node IDs.
    #[serde(default)]
    pub node_ids: Option<Vec<Uuid>>,
    /// When `true`, return what would change but do not mutate the document.
    /// Useful for auditing before committing a large batch replacement.
    #[serde(default)]
    pub dry_run: bool,
}

// ─── FindReplaceTextArgs ──────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct FindReplaceTextArgs {
    /// Text to search for. Plain string by default; treated as a regex when `regex: true`.
    pub find: String,
    /// Replacement string. When `regex: true`, capture group back-references ($1, $2, …) are supported.
    pub replace: String,
    /// Treat `find` as a regular expression. Default: false.
    #[serde(default)]
    pub regex: bool,
    /// Case-sensitive match. Default: true.
    #[serde(default = "default_true")]
    pub case_sensitive: bool,
    /// Preview matches without applying changes. Default: false.
    #[serde(default)]
    pub dry_run: bool,
    /// Scope to specific text node UUIDs. Omit to search all text nodes in the document.
    #[serde(default)]
    pub node_ids: Option<Vec<Uuid>>,
}

/// Arguments for `import_svg` tool
#[derive(Debug, Deserialize)]
pub struct ImportSvgArgs {
    #[serde(default)]
    pub svg_string: Option<String>,
    #[serde(default)]
    pub file_path: Option<String>,
    #[serde(default)]
    pub layer_id: Option<Uuid>,
}

// ─── Shared argument types ───────────────────────────────────────────────────

/// A single control point for a fluid gradient (MCP input).
#[derive(Debug, Deserialize, Clone)]
pub struct FluidPointArg {
    pub x: f64,
    pub y: f64,
    pub color: String,
}

/// A single vertex for a mesh gradient (MCP input).
#[derive(Debug, Deserialize, Clone)]
pub struct MeshVertexArg {
    pub x: f64,
    pub y: f64,
    pub color: String,
}

/// Fill specification from an MCP client.
#[derive(Debug, Deserialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FillArg {
    None,
    Solid {
        color: String,
    },
    Gradient {
        gradient_type: Option<String>,
        colors: Vec<String>,
        #[serde(default)]
        offsets: Option<Vec<f32>>,
        #[serde(default)]
        coords: Option<Vec<f64>>,
    },
    /// Fluid (free-point) gradient: colors blended via inverse-distance weighting.
    ///
    /// Example:
    /// ```json
    /// {
    ///   "type": "fluid_gradient",
    ///   "points": [
    ///     {"x": 100, "y": 50,  "color": "#ff0000"},
    ///     {"x": 300, "y": 50,  "color": "#0000ff"},
    ///     {"x": 200, "y": 200, "color": "#00ff00"}
    ///   ],
    ///   "power": 2.0
    /// }
    /// ```
    FluidGradient {
        points: Vec<FluidPointArg>,
        #[serde(default)]
        power: Option<f32>,
    },
    /// Mesh (vertex-grid) gradient: rows×cols grid of coloured vertices with
    /// bilinear interpolation within each cell.
    ///
    /// Example (2×2 grid):
    /// ```json
    /// {
    ///   "type": "mesh_gradient",
    ///   "rows": 2,
    ///   "cols": 2,
    ///   "vertices": [
    ///     {"x": 0,   "y": 0,   "color": "#ff0000"},
    ///     {"x": 200, "y": 0,   "color": "#00ff00"},
    ///     {"x": 0,   "y": 200, "color": "#0000ff"},
    ///     {"x": 200, "y": 200, "color": "#ffff00"}
    ///   ]
    /// }
    /// ```
    MeshGradient {
        rows: u32,
        cols: u32,
        vertices: Vec<MeshVertexArg>,
    },
    /// Built-in geometric pattern fill.
    ///
    /// Example: `{"type":"pattern","pattern":"dots","color":"#000000","background":"#ffffff","spacing":12}`
    Pattern {
        /// "dots" | "stripes" | "grid" | "checkerboard".
        pattern: String,
        /// Foreground colour (`#rrggbb`).
        color: String,
        /// Optional background colour (`#rrggbb`).
        #[serde(default)]
        background: Option<String>,
        /// Tile size / spacing in document units (default 12).
        #[serde(default)]
        spacing: Option<f64>,
    },
}

impl FillArg {
    /// Convert to a `photonic_core::style::Fill`. Returns an error if colors can't be parsed.
    pub fn to_fill(&self) -> Result<photonic_core::style::Fill, String> {
        use photonic_core::style::{
            Fill, FluidGradient, FluidGradientPoint, Gradient, GradientStop, MeshGradient,
            MeshGradientVertex,
        };
        match self {
            FillArg::None => Ok(Fill::none()),
            FillArg::Solid { color } => {
                let c =
                    Color::from_hex(color).ok_or_else(|| format!("Invalid color: {}", color))?;
                Ok(Fill::solid(c))
            }
            FillArg::Gradient {
                gradient_type,
                colors,
                offsets,
                coords,
            } => {
                let parsed: Result<Vec<Color>, _> = colors
                    .iter()
                    .map(|c| Color::from_hex(c).ok_or_else(|| format!("Invalid color: {}", c)))
                    .collect();
                let parsed = parsed?;
                let stops: Vec<GradientStop> = parsed
                    .into_iter()
                    .enumerate()
                    .map(|(i, color)| {
                        let offset = offsets
                            .as_ref()
                            .and_then(|o| o.get(i).copied())
                            .unwrap_or(i as f32 / (colors.len() - 1).max(1) as f32);
                        GradientStop::new(offset, color)
                    })
                    .collect();

                let is_radial = gradient_type.as_deref() == Some("radial");
                let gradient = if is_radial {
                    let c = coords.as_deref().unwrap_or(&[0.5, 0.5, 0.5]);
                    Gradient::radial(
                        c.first().copied().unwrap_or(0.5),
                        c.get(1).copied().unwrap_or(0.5),
                        c.get(2).copied().unwrap_or(0.5),
                        stops,
                    )
                } else {
                    let c = coords.as_deref().unwrap_or(&[0.0, 0.0, 1.0, 0.0]);
                    Gradient::linear(
                        c.first().copied().unwrap_or(0.0),
                        c.get(1).copied().unwrap_or(0.0),
                        c.get(2).copied().unwrap_or(1.0),
                        c.get(3).copied().unwrap_or(0.0),
                        stops,
                    )
                };
                Ok(Fill::gradient(gradient))
            }
            FillArg::FluidGradient { points, power } => {
                let pts: Result<Vec<FluidGradientPoint>, String> = points
                    .iter()
                    .map(|p| {
                        let color = Color::from_hex(&p.color)
                            .ok_or_else(|| format!("Invalid color: {}", p.color))?;
                        Ok(FluidGradientPoint::new(p.x, p.y, color))
                    })
                    .collect();
                let mut fg = FluidGradient::new(pts?);
                if let Some(pw) = power {
                    fg.power = *pw;
                }
                Ok(Fill::fluid_gradient(fg))
            }
            FillArg::MeshGradient {
                rows,
                cols,
                vertices,
            } => {
                let verts: Result<Vec<MeshGradientVertex>, String> = vertices
                    .iter()
                    .map(|v| {
                        let color = Color::from_hex(&v.color)
                            .ok_or_else(|| format!("Invalid color: {}", v.color))?;
                        Ok(MeshGradientVertex::new(v.x, v.y, color))
                    })
                    .collect();
                Ok(Fill::mesh_gradient(MeshGradient::new(*rows, *cols, verts?)))
            }
            FillArg::Pattern {
                pattern,
                color,
                background,
                spacing,
            } => {
                use photonic_core::style::{FillKind, PatternFill, PatternKind};
                let kind = match pattern.as_str() {
                    "dots" => PatternKind::Dots,
                    "stripes" => PatternKind::Stripes,
                    "grid" => PatternKind::Grid,
                    "checkerboard" => PatternKind::Checkerboard,
                    other => {
                        return Err(format!(
                            "Unknown pattern '{other}' (use dots|stripes|grid|checkerboard)"
                        ))
                    }
                };
                let fg =
                    Color::from_hex(color).ok_or_else(|| format!("Invalid color: {}", color))?;
                let bg = match background {
                    Some(h) => Some(
                        Color::from_hex(h).ok_or_else(|| format!("Invalid background: {}", h))?,
                    ),
                    None => None,
                };
                Ok(Fill {
                    kind: FillKind::Pattern(PatternFill {
                        kind,
                        color: fg,
                        background: bg,
                        spacing: spacing.unwrap_or(12.0).max(1.0),
                    }),
                    opacity: 1.0,
                    enabled: true,
                })
            }
        }
    }
}

/// Stroke specification from an MCP client.
#[derive(Debug, Deserialize, Clone)]
pub struct StrokeArg {
    pub color: Option<String>,
    pub width: Option<f64>,
    pub enabled: Option<bool>,
    pub opacity: Option<f32>,
    /// "butt" | "round" | "square"
    pub line_cap: Option<String>,
    /// "miter" | "round" | "bevel"
    pub line_join: Option<String>,
    /// "center" | "inside" | "outside"
    pub align: Option<String>,
    /// Dash pattern: alternating dash and gap lengths (e.g. [8,4] or [8,4,2,4]).
    /// Up to 6 values (3 dash/gap pairs). Empty or absent = solid stroke.
    #[serde(default)]
    pub dash_array: Option<Vec<f64>>,
    /// Phase offset into the dash pattern (pixels). Default 0.
    #[serde(default)]
    pub dash_offset: Option<f64>,
    /// Align dashes to path corners and endpoints so dashes are never clipped at corners.
    #[serde(default)]
    pub dash_corner_alignment: Option<bool>,
    /// Arrowhead at the path start: "none" | "filled_arrow" | "open_arrow". Default "none".
    #[serde(default)]
    pub arrowhead_start: Option<String>,
    /// Arrowhead at the path end: "none" | "filled_arrow" | "open_arrow". Default "none".
    #[serde(default)]
    pub arrowhead_end: Option<String>,
}

impl StrokeArg {
    pub fn to_stroke(&self) -> Result<photonic_core::style::Stroke, String> {
        use photonic_core::style::{ArrowheadStyle, LineCap, LineJoin, Stroke, StrokeAlign};
        let enabled = self.enabled.unwrap_or(true);
        if !enabled {
            return Ok(Stroke::none());
        }
        let color = self
            .color
            .as_deref()
            .and_then(Color::from_hex)
            .unwrap_or(Color::BLACK);
        let width = self.width.unwrap_or(1.0);
        let mut stroke = Stroke::solid(color, width);
        if let Some(op) = self.opacity {
            stroke.opacity = op.clamp(0.0, 1.0);
        }
        if let Some(cap) = &self.line_cap {
            stroke.line_cap = match cap.to_lowercase().as_str() {
                "round" => LineCap::Round,
                "square" => LineCap::Square,
                _ => LineCap::Butt,
            };
        }
        if let Some(join) = &self.line_join {
            stroke.line_join = match join.to_lowercase().as_str() {
                "round" => LineJoin::Round,
                "bevel" => LineJoin::Bevel,
                _ => LineJoin::Miter,
            };
        }
        if let Some(align) = &self.align {
            stroke.align = match align.to_lowercase().as_str() {
                "inside" => StrokeAlign::Inside,
                "outside" => StrokeAlign::Outside,
                _ => StrokeAlign::Center,
            };
        }
        if let Some(dash) = &self.dash_array {
            // Clamp to at most 6 values (3 dash/gap pairs); reject negative values.
            let cleaned: Vec<f64> = dash.iter().take(6).map(|&v| v.max(0.0)).collect();
            stroke.dash_array = cleaned;
        }
        if let Some(offset) = self.dash_offset {
            stroke.dash_offset = offset;
        }
        if let Some(align) = self.dash_corner_alignment {
            stroke.dash_corner_alignment = align;
        }
        let parse_arrowhead = |s: &str| -> ArrowheadStyle {
            match s.to_lowercase().as_str() {
                "filled_arrow" | "filled" => ArrowheadStyle::FilledArrow,
                "open_arrow" | "open" => ArrowheadStyle::OpenArrow,
                _ => ArrowheadStyle::None,
            }
        };
        if let Some(ah) = &self.arrowhead_start {
            stroke.arrowhead_start = parse_arrowhead(ah);
        }
        if let Some(ah) = &self.arrowhead_end {
            stroke.arrowhead_end = parse_arrowhead(ah);
        }
        Ok(stroke)
    }
}

/// Transform specification from an MCP client.
#[derive(Debug, Deserialize, Clone)]
pub struct TransformArg {
    /// [a, b, c, d, e, f] affine matrix
    pub matrix: Option<[f64; 6]>,
    pub translate: Option<TranslateArg>,
    pub rotate: Option<RotateArg>,
    pub scale: Option<ScaleArg>,
}

impl TransformArg {
    pub fn to_transform(&self) -> photonic_core::transform::Transform {
        use photonic_core::transform::Transform;
        if let Some(m) = self.matrix {
            return Transform { matrix: m };
        }
        let mut t = Transform::IDENTITY;
        if let Some(s) = &self.scale {
            t = t.then(&Transform::scale_around(s.sx, s.sy, s.origin_x, s.origin_y));
        }
        if let Some(r) = &self.rotate {
            t = t.then(&Transform::rotate_around(
                r.angle_degrees.to_radians(),
                r.origin_x,
                r.origin_y,
            ));
        }
        if let Some(tr) = &self.translate {
            t = t.then(&Transform::translate(tr.x, tr.y));
        }
        t
    }
}

// ─── measure_nodes ────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct MeasureNodesArgs {
    pub node_ids: Vec<Uuid>,
}

// ─── inspect_node ─────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct InspectNodeArgs {
    /// Node ID (UUID string) or node name.
    pub id: String,
}

// ─── layout_nodes ─────────────────────────────────────────────────────────────

/// Layout algorithm used by `layout_nodes`.
#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum LayoutMode {
    /// Arrange nodes in a left-to-right grid that wraps into rows.
    Grid,
    /// Arrange nodes evenly around a circle.
    Circle,
    /// Stack nodes left-to-right along the X axis.
    StackHorizontal,
    /// Stack nodes top-to-bottom along the Y axis.
    StackVertical,
}

/// Cross-axis alignment used by stack layouts.
#[derive(Debug, Deserialize, Clone, Copy, Default)]
#[serde(rename_all = "snake_case")]
pub enum CrossAxisAlign {
    /// Align to the start edge (top for `stack_horizontal`, left for `stack_vertical`).
    #[default]
    Start,
    /// Align to the centre.
    Center,
    /// Align to the end edge.
    End,
}

/// Arguments for the `layout_nodes` tool.
#[derive(Debug, Deserialize)]
pub struct LayoutNodesArgs {
    /// IDs of the nodes to rearrange. Order determines placement.
    pub node_ids: Vec<Uuid>,

    /// Layout algorithm to apply.
    pub layout: LayoutMode,

    // ── Shared origin ─────────────────────────────────────────────────────────
    /// X origin of the layout. Defaults to the left edge of the current selection.
    #[serde(default)]
    pub x: Option<f64>,
    /// Y origin of the layout. Defaults to the top edge of the current selection.
    #[serde(default)]
    pub y: Option<f64>,

    // ── Grid ──────────────────────────────────────────────────────────────────
    /// Number of columns (default: ceil(sqrt(N))).
    #[serde(default)]
    pub columns: Option<usize>,
    /// Horizontal gap between cells in pixels (default: 20).
    #[serde(default)]
    pub gap_x: Option<f64>,
    /// Vertical gap between cells in pixels (default: 20).
    #[serde(default)]
    pub gap_y: Option<f64>,
    /// Fixed cell width. Defaults to the widest node in the set.
    #[serde(default)]
    pub cell_width: Option<f64>,
    /// Fixed cell height. Defaults to the tallest node in the set.
    #[serde(default)]
    pub cell_height: Option<f64>,

    // ── Circle ────────────────────────────────────────────────────────────────
    /// Circle centre X. Defaults to the combined bounding-box centre.
    #[serde(default)]
    pub cx: Option<f64>,
    /// Circle centre Y. Defaults to the combined bounding-box centre.
    #[serde(default)]
    pub cy: Option<f64>,
    /// Radius of the circle in pixels (default: 200).
    #[serde(default)]
    pub radius: Option<f64>,
    /// Angle of the first node in degrees, measured from the positive X axis (default: 0).
    #[serde(default)]
    pub start_angle: Option<f64>,

    // ── Stack ─────────────────────────────────────────────────────────────────
    /// Gap between successive nodes in pixels (default: 20).
    #[serde(default)]
    pub gap: Option<f64>,
    /// Cross-axis alignment: `start` / `center` / `end` (default: `start`).
    #[serde(default)]
    pub align: CrossAxisAlign,
}

// ─── set_node_size ────────────────────────────────────────────────────────────

/// Arguments for the `set_node_size` tool.
#[derive(Debug, Deserialize)]
pub struct SetNodeSizeArgs {
    /// ID of the node to resize.
    pub node_id: Uuid,
    /// Target width in pixels. Omit to derive from height (requires `maintain_aspect_ratio`).
    #[serde(default)]
    pub width: Option<f64>,
    /// Target height in pixels. Omit to derive from width (requires `maintain_aspect_ratio`).
    #[serde(default)]
    pub height: Option<f64>,
    /// When true and both dimensions are given, use the smaller scale factor for both axes
    /// so the shape fits inside the requested box without distortion.
    /// When true and only one dimension is given, scale the other axis proportionally.
    /// Default: false (each axis scaled independently to hit the exact requested size).
    #[serde(default)]
    pub maintain_aspect_ratio: bool,
    /// The point on the node's bounding box that stays fixed during the resize.
    /// Default: `top_left`.
    #[serde(default)]
    pub anchor: SizeAnchor,
}

/// Which corner/edge of the bounding box to keep fixed when resizing.
#[derive(Debug, Deserialize, Clone, Copy, Default)]
#[serde(rename_all = "snake_case")]
pub enum SizeAnchor {
    #[default]
    TopLeft,
    TopCenter,
    TopRight,
    LeftCenter,
    Center,
    RightCenter,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

// ─── auto_name_nodes ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Default)]
pub struct AutoNameNodesArgs {
    /// "selection" = active selection only; "document" = all nodes (default).
    #[serde(default)]
    pub scope: Option<String>,
    /// If true, also rename nodes that already have non-generic names. Default: false.
    #[serde(default)]
    pub overwrite: bool,
    /// If true, return proposed renames without applying them. Default: false.
    #[serde(default)]
    pub dry_run: bool,
}

// ─── get_css_preview ─────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Default)]
pub struct GetCssPreviewArgs {
    /// Node UUID or name. If omitted, the first node in document order is used.
    #[serde(default)]
    pub id: Option<String>,
}

// ─── check_style_continuity ───────────────────────────────────────────────────

#[derive(Debug, Deserialize, Default)]
pub struct CheckStyleContinuityArgs {
    /// Node UUIDs to analyse. If absent or empty, the entire document is analysed.
    #[serde(default)]
    pub node_ids: Vec<Uuid>,
    /// Which property groups to check. Valid values: "fill", "stroke", "opacity", "font".
    /// Defaults to all four when omitted.
    #[serde(default)]
    pub checks: Vec<String>,
    /// Minimum occurrences for a value to be considered "dominant". Default: 2.
    /// Nodes whose value appears fewer than this many times are flagged as outliers.
    #[serde(default)]
    pub outlier_threshold: Option<usize>,
}

// ─── SimplifyPathArgs ─────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct SimplifyPathArgs {
    /// UUID of the path node to simplify.
    pub node_id: Uuid,
    /// Ramer-Douglas-Peucker tolerance in document coordinates.
    /// Larger values remove more points. Typical range: 0.1–10.0.
    pub tolerance: f64,
    /// If true, return point counts without modifying the document. Default false.
    #[serde(default)]
    pub dry_run: bool,
}

// ─── OutlineStrokeArgs ────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct OutlineStrokeArgs {
    /// UUIDs of path nodes whose stroke should be converted to an outline path.
    pub node_ids: Vec<Uuid>,
    /// If true, the original node's stroke is removed but the node remains.
    /// If false (default), same behaviour — the original stroke is disabled and
    /// a new outline node is placed above it.
    #[serde(default)]
    pub keep_original: bool,
}

// ─── OffsetPathArgs ───────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct OffsetPathArgs {
    /// UUIDs of path nodes to offset.
    pub node_ids: Vec<Uuid>,
    /// Offset distance in document units. Positive = outset (expand), negative = inset (shrink).
    pub distance: f64,
    /// Corner join style: "miter" (default), "round", or "bevel".
    #[serde(default)]
    pub join_style: Option<String>,
    /// If true (default), add the offset path as a new node above the original.
    /// If false, replace the original node with the offset result.
    #[serde(default)]
    pub create_copy: Option<bool>,
}

// ─── SplitIntoGridArgs ────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct SplitIntoGridArgs {
    /// UUID of the source path node whose bounding box defines the grid area.
    pub node_id: Uuid,
    /// Number of rows (≥ 1).
    pub rows: usize,
    /// Number of columns (≥ 1).
    pub cols: usize,
    /// Horizontal gutter width in document units between columns (default 0).
    #[serde(default)]
    pub gutter_x: Option<f64>,
    /// Vertical gutter height in document units between rows (default 0).
    #[serde(default)]
    pub gutter_y: Option<f64>,
    /// When true, keep the original node. Default: false (original is deleted).
    #[serde(default)]
    pub keep_original: Option<bool>,
    /// Layer to place new nodes in. Defaults to the source node's layer.
    #[serde(default)]
    pub layer_id: Option<Uuid>,
}

// ─── ReleaseToLayersArgs ──────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ReleaseToLayersArgs {
    /// IDs of nodes to release. Group children are resolved to their top-level ancestor.
    pub node_ids: Vec<Uuid>,
    /// Optional prefix for the new layer names. Each layer is named
    /// "<prefix> 1", "<prefix> 2", … (default: "Layer").
    #[serde(default)]
    pub name_prefix: Option<String>,
}

// ─── MergeLayersArgs ─────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct MergeLayersArgs {
    /// IDs of the layers to merge. Must contain at least 2 entries.
    pub layer_ids: Vec<Uuid>,
    /// Optional name for the surviving (target) layer.
    /// Defaults to the name of the first layer in document order among those selected.
    #[serde(default)]
    pub target_name: Option<String>,
}

// ─── FlattenArtworkArgs ───────────────────────────────────────────────────────

/// Arguments for `update_layer`.
#[derive(Debug, Deserialize)]
pub struct UpdateLayerArgs {
    /// UUID of the layer to update.
    pub layer_id: Uuid,
    /// New name for the layer. Omit to keep existing name.
    #[serde(default)]
    pub name: Option<String>,
    /// Set layer visibility. Omit to keep existing value.
    #[serde(default)]
    pub visible: Option<bool>,
    /// Set layer lock state. Omit to keep existing value.
    #[serde(default)]
    pub locked: Option<bool>,
    /// Color tag for the layer as [r, g, b, a] with values 0.0–1.0.
    /// Pass `null` to clear the color. Omit to keep existing color.
    #[serde(default)]
    pub color: Option<Option<[f32; 4]>>,
    /// Mark this layer as a template layer (locked, dimmed reference for tracing).
    /// Omit to keep existing value.
    #[serde(default)]
    pub is_template: Option<bool>,
}

/// Arguments for `flatten_artwork`.
#[derive(Debug, Deserialize, Default)]
pub struct FlattenArtworkArgs {
    /// Optional name for the surviving layer. Defaults to the name of the
    /// bottom-most layer in document order.
    #[serde(default)]
    pub target_name: Option<String>,
}

// ─── BlendColorsArgs ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct BlendColorsArgs {
    /// Ordered list of path node UUIDs to blend. Minimum 2.
    /// The first and last nodes keep their existing solid fill colors;
    /// intermediate nodes receive linearly interpolated colors.
    pub node_ids: Vec<Uuid>,
    /// Optional axis for auto-sorting nodes before blending.
    /// "horizontal" → sort by bounding-box center X (left → right),
    /// "vertical"   → sort by bounding-box center Y (top → bottom),
    /// "depth"      → sort by z-order (bottom layer/node first).
    /// Omit to use the supplied node_ids order as-is.
    #[serde(default)]
    pub direction: Option<String>,
}

// ─── InvertColorsArgs ─────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct InvertColorsArgs {
    /// UUIDs of path nodes to invert. If omitted, all path nodes in the document are inverted.
    #[serde(default)]
    pub node_ids: Option<Vec<Uuid>>,
}

// ─── AdjustColorsArgs ─────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Default)]
pub struct AdjustColorsArgs {
    /// UUIDs of path nodes to adjust. If omitted, all path nodes in the document are adjusted.
    #[serde(default)]
    pub node_ids: Option<Vec<Uuid>>,
    /// Amount to add to the red channel (−1.0 to 1.0). Default 0.
    #[serde(default)]
    pub delta_r: f32,
    /// Amount to add to the green channel (−1.0 to 1.0). Default 0.
    #[serde(default)]
    pub delta_g: f32,
    /// Amount to add to the blue channel (−1.0 to 1.0). Default 0.
    #[serde(default)]
    pub delta_b: f32,
    /// Amount to add to the alpha channel (−1.0 to 1.0). Default 0.
    #[serde(default)]
    pub delta_a: f32,
}

// ─── ConvertToGrayscaleArgs ───────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ConvertToGrayscaleArgs {
    /// UUIDs of path nodes to convert. If omitted, all path nodes in the document are converted.
    #[serde(default)]
    pub node_ids: Option<Vec<Uuid>>,
}

// ─── Tool result type ─────────────────────────────────────────────────────────

/// Standard MCP tool result wrapper.
#[derive(Debug, Serialize)]
pub struct ToolResult {
    pub content: Vec<ContentItem>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "isError")]
    pub is_error: Option<bool>,
}

impl ToolResult {
    pub fn text(msg: impl Into<String>) -> Self {
        Self {
            content: vec![ContentItem::text(msg)],
            is_error: None,
        }
    }

    pub fn error(msg: impl Into<String>) -> Self {
        Self {
            content: vec![ContentItem::text(msg)],
            is_error: Some(true),
        }
    }

    pub fn with_data(mut self, data: impl Serialize) -> Self {
        if let Ok(v) = serde_json::to_value(data) {
            self.content.push(ContentItem::json(v));
        }
        self
    }

    pub fn with_image(mut self, base64_png: String) -> Self {
        self.content.push(ContentItem::image(base64_png));
        self
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentItem {
    Text {
        text: String,
    },
    Image {
        data: String,
        #[serde(rename = "mimeType")]
        mime_type: String,
    },
    Resource {
        resource: Value,
    },
}

impl ContentItem {
    pub fn text(msg: impl Into<String>) -> Self {
        Self::Text { text: msg.into() }
    }

    pub fn json(v: Value) -> Self {
        Self::Text {
            text: serde_json::to_string_pretty(&v).unwrap_or_default(),
        }
    }

    pub fn image(base64_png: String) -> Self {
        Self::Image {
            data: base64_png,
            mime_type: "image/png".to_string(),
        }
    }
}

// ─── Annotation Args ─────────────────────────────────────────────────────────

/// Arguments for `add_annotation`.
#[derive(Debug, Deserialize)]
pub struct AddAnnotationArgs {
    /// The comment or design note text (required, non-empty).
    pub text: String,
    /// Node to attach this annotation to. Omit for a document-level note.
    #[serde(default)]
    pub node_id: Option<Uuid>,
    /// Optional author identity (e.g. `"claude"`, `"design-reviewer"`).
    #[serde(default)]
    pub author: Option<String>,
}

/// Arguments for `add_anchor_points`.
#[derive(Debug, Deserialize, Default)]
pub struct AddAnchorPointsArgs {
    /// IDs of path nodes to subdivide.
    pub node_ids: Vec<Uuid>,
    /// Number of subdivision passes (default 1, max 8).
    #[serde(default)]
    pub passes: Option<u32>,
}

/// Arguments for `clean_up`.
#[derive(Debug, Deserialize, Default)]
pub struct CleanUpArgs {
    /// Remove paths with no drawing segments (only MoveTo or empty). Default true.
    #[serde(default)]
    pub remove_stray_points: Option<bool>,
    /// Remove paths with no visible fill AND no visible stroke. Default true.
    #[serde(default)]
    pub remove_unpainted: Option<bool>,
    /// Remove text nodes whose content is empty or whitespace-only. Default true.
    #[serde(default)]
    pub remove_empty_text: Option<bool>,
    /// If true, report what would be removed without deleting anything. Default false.
    #[serde(default)]
    pub dry_run: Option<bool>,
}

/// Arguments for `join_paths`.
#[derive(Debug, Deserialize, Default)]
pub struct JoinPathsArgs {
    /// One or two path node IDs.
    ///
    /// * **1 node** — every open subpath in the node is closed (a `ClosePath`
    ///   element is appended to each open subpath).
    /// * **2 nodes** — the two paths are merged into one by connecting their
    ///   nearest open endpoints with a straight line segment.  The result node
    ///   inherits the style of the first listed node; the second node is
    ///   removed.
    pub node_ids: Vec<Uuid>,
}

/// Arguments for `reverse_path_direction`.
#[derive(Debug, Deserialize, Default)]
pub struct ReversePathDirectionArgs {
    /// IDs of path nodes whose winding direction to reverse.
    pub node_ids: Vec<Uuid>,
}

/// Arguments for `average_anchor_points`.
#[derive(Debug, Deserialize, Default)]
pub struct AverageAnchorPointsArgs {
    /// IDs of path nodes to average.
    pub node_ids: Vec<Uuid>,
    /// Which axis to average: `"horizontal"` (X only), `"vertical"` (Y only),
    /// or `"both"` (default).
    #[serde(default)]
    pub axis: Option<String>,
}

/// Arguments for `list_annotations`.
#[derive(Debug, Deserialize, Default)]
pub struct ListAnnotationsArgs {
    /// Filter to annotations attached to a specific node.
    #[serde(default)]
    pub node_id: Option<Uuid>,
    /// When `true`, include resolved annotations. Defaults to `false`.
    #[serde(default)]
    pub include_resolved: Option<bool>,
}

/// Arguments for `resolve_annotation`.
#[derive(Debug, Deserialize)]
pub struct ResolveAnnotationArgs {
    /// UUID of the annotation to mark as resolved.
    pub annotation_id: Uuid,
}

/// Arguments for `pathfinder_crop`.
#[derive(Debug, Deserialize, Default)]
pub struct PathfinderCropArgs {
    /// Two or more path node IDs. The frontmost node (highest z-order) acts as
    /// the clipping boundary. All other nodes are clipped to that boundary in
    /// place (their paths are replaced by `path ∩ frontmost_path`). The
    /// frontmost node is removed at the end. Single undoable step.
    pub node_ids: Vec<Uuid>,
}

/// Arguments for `pathfinder_minus_back`.
#[derive(Debug, Deserialize, Default)]
pub struct PathfinderMinusBackArgs {
    /// Two or more path node IDs. The back nodes (all except the frontmost) are
    /// subtracted from the frontmost node's path; the back nodes are removed.
    /// The frontmost node's fill/stroke style is preserved. Single undoable step.
    pub node_ids: Vec<Uuid>,
}

/// Arguments for `pathfinder_minus_front`.
#[derive(Debug, Deserialize, Default)]
pub struct PathfinderMinusFrontArgs {
    /// Two or more path node IDs. The frontmost node (highest z-order) is
    /// subtracted from each back node's path; the frontmost node is removed.
    /// Each back node's fill/stroke style is preserved. Single undoable step.
    pub node_ids: Vec<Uuid>,
}

/// Arguments for `pathfinder_trim`.
#[derive(Debug, Deserialize, Default)]
pub struct PathfinderTrimArgs {
    /// Two or more path node IDs. Each node has the paths of all nodes above it
    /// (higher z-order) subtracted from it, removing hidden areas. Strokes are
    /// disabled on all result nodes. All nodes are retained (none removed).
    /// Single undoable step.
    pub node_ids: Vec<Uuid>,
}

/// Arguments for `pathfinder_outline`.
#[derive(Debug, Deserialize, Default)]
pub struct PathfinderOutlineArgs {
    /// One or more path node IDs. Each node's solid fill color is transferred to
    /// its stroke; the fill is removed; the stroke is enabled. Gradient fills
    /// fall back to black. Existing stroke width is preserved (default 1 pt if
    /// no stroke was set). Single undoable step.
    pub node_ids: Vec<Uuid>,
}

/// Arguments for `divide_objects_below`.
#[derive(Debug, Deserialize)]
pub struct DivideObjectsBelowArgs {
    /// The path node ID to use as the cutting edge. All nodes beneath it in
    /// z-order that overlap it will be split. The cutter is removed afterward.
    pub node_id: Uuid,
}

/// Arguments for `pathfinder_divide`.
#[derive(Debug, Deserialize, Default)]
pub struct PathfinderDivideArgs {
    /// Exactly two path node IDs: [back_node, front_node] (z-order). The two
    /// shapes are split at every overlap edge into up to three distinct faces.
    /// New path nodes are created for each face; the originals are removed.
    /// Face colors are inherited from whichever source shape contained them.
    pub node_ids: Vec<Uuid>,
    /// Layer to place the result nodes in. Defaults to the back node's layer.
    pub layer_id: Option<Uuid>,
}

/// Arguments for `pathfinder_merge`.
#[derive(Debug, Deserialize, Default)]
pub struct PathfinderMergeArgs {
    /// Two or more path node IDs (any order; back-to-front z-order is resolved automatically).
    /// Each node is trimmed of areas covered by nodes above it, then nodes sharing the same
    /// solid fill color are merged (unioned) into a single shape. Non-solid fills each become
    /// a separate result node. Strokes are disabled on all result nodes.
    pub node_ids: Vec<Uuid>,
    /// Layer to place the result nodes in. Defaults to the backmost source node's layer.
    pub layer_id: Option<Uuid>,
}

/// Which attribute to match against in `select_same`.
#[derive(Debug, Deserialize, Default, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SelectSameAttribute {
    /// Match nodes whose solid fill color is within tolerance of the reference.
    #[default]
    FillColor,
    /// Match nodes whose solid stroke color is within tolerance of the reference.
    StrokeColor,
    /// Match nodes whose stroke width is within tolerance of the reference.
    StrokeWeight,
    /// Match nodes whose opacity is within tolerance of the reference.
    Opacity,
    /// Match nodes that share the same blend mode as the reference.
    BlendMode,
    /// Match nodes of the same node type (path / group / text).
    ObjectType,
}

/// Arguments for `select_same`.
#[derive(Debug, Deserialize, Default)]
pub struct SelectSameArgs {
    /// ID of the reference node whose attribute value is matched against.
    pub node_id: Uuid,
    /// Which attribute to match.
    pub attribute: SelectSameAttribute,
    /// How close two values must be to count as "same". Applies to color
    /// (Euclidean RGBA distance in [0,1] space), stroke weight, and opacity.
    /// Defaults to 0.01 (exact match in practice). Ignored for blend_mode and object_type.
    #[serde(default)]
    pub tolerance: Option<f64>,
    /// If true, include the reference node itself in the results. Default: true.
    #[serde(default)]
    pub include_self: Option<bool>,
}

// ─── Compound Path Args ──────────────────────────────────────────────────────

/// Arguments for `make_compound_path`.
#[derive(Debug, Deserialize, Default)]
pub struct MakeCompoundPathArgs {
    /// IDs of the path nodes to combine into a single compound path.
    /// Must contain at least 2 path nodes. The bottommost node's fill/stroke
    /// is used for the resulting compound path.
    pub node_ids: Vec<Uuid>,
    /// Optional name for the resulting compound path node.
    #[serde(default)]
    pub name: Option<String>,
}

/// Arguments for `release_compound_path`.
#[derive(Debug, Deserialize, Default)]
pub struct ReleaseCompoundPathArgs {
    /// ID of the compound path node to release back into individual paths.
    pub node_id: Uuid,
}

// ─── ColorGuideArgs ──────────────────────────────────────────────────────────

/// Arguments for the `color_guide` tool.
#[derive(Debug, Deserialize)]
pub struct ColorGuideArgs {
    /// Base color as a hex string (#RRGGBB or #RRGGBBAA). Defaults to the
    /// solid fill of the first selected node when omitted.
    #[serde(default)]
    pub base_color: Option<String>,
    /// Harmony rule: "complementary" | "analogous" | "triadic" |
    /// "split_complementary" | "tetradic" | "monochromatic".
    /// Defaults to "complementary".
    #[serde(default)]
    pub rule: Option<String>,
}

/// Arguments for `recolor_artwork` tool
#[derive(Debug, Deserialize)]
pub struct RecolorArtworkArgs {
    /// IDs of nodes whose solid fills should be remapped. If empty, applies to all path nodes.
    #[serde(default)]
    pub node_ids: Vec<Uuid>,
    /// Target palette as hex strings (#RRGGBB or #RRGGBBAA). Each node's fill is replaced
    /// with the nearest palette color by Euclidean RGB distance.
    pub palette: Vec<String>,
}

/// Arguments for `distribute_on_path` tool
#[derive(Debug, Deserialize)]
pub struct DistributeOnPathArgs {
    /// ID of the path node to use as the distribution guide.
    pub path_node_id: Uuid,
    /// IDs of the nodes to distribute along the path. Each node is cloned `count` times.
    pub node_ids: Vec<Uuid>,
    /// Number of copies to place along the path. Defaults to the number of source nodes.
    #[serde(default)]
    pub count: Option<usize>,
    /// If true, rotate each copy to align with the path's tangent direction. Default: false.
    #[serde(default)]
    pub align_to_path: Option<bool>,
    /// Target layer for the new copies. Defaults to the guide path's layer.
    #[serde(default)]
    pub layer_id: Option<Uuid>,
}

// ─── Export Profile Args ──────────────────────────────────────────────────────

/// Arguments for `add_export_profile` tool
#[derive(Debug, Deserialize)]
pub struct AddExportProfileArgs {
    /// Unique profile name. If a profile with this name exists, it is replaced.
    pub name: String,
    /// Target format: "svg", "png", "jpeg", or "webp".
    pub format: String,
    /// Raster-only: explicit pixel width.
    pub width: Option<u32>,
    /// Raster-only: explicit pixel height (overrides scale).
    pub height: Option<u32>,
    /// SVG-only: emit semantic id attributes (default true).
    pub semantic_ids: Option<bool>,
    /// SVG-only: coordinate decimal precision 1–6 (default 4).
    pub precision: Option<u32>,
}

/// Arguments for `remove_export_profile` tool
#[derive(Debug, Deserialize)]
pub struct RemoveExportProfileArgs {
    /// Name of the profile to remove.
    pub name: String,
}

/// Arguments for `run_export_profile` tool
#[derive(Debug, Deserialize)]
pub struct RunExportProfileArgs {
    /// Name of the profile to run.
    pub name: String,
}

// ─── PinObjectGuidesArgs ──────────────────────────────────────────────────────

/// Arguments for `pin_object_guides` tool
#[derive(Debug, Deserialize)]
pub struct PinObjectGuidesArgs {
    /// UUIDs or names of nodes to pin guides from. Uses current selection if empty.
    #[serde(default)]
    pub node_ids: Vec<String>,
    /// Which edges to pin: "all" (default), "center", "edges", or a comma-separated
    /// subset of "top","bottom","left","right","center_h","center_v".
    #[serde(default)]
    pub edges: Option<String>,
}

// ─── Document Template Args ───────────────────────────────────────────────────

/// Arguments for `apply_document_template` tool
#[derive(Debug, Deserialize)]
pub struct ApplyDocumentTemplateArgs {
    /// Template JSON (from get_document_template). Canvas size, guides, export
    /// profiles, and new layers are applied to the current document.
    pub template_json: String,
}

// ─── PromptHistoryArgs ────────────────────────────────────────────────────────

/// Arguments for `set_node_prompt` tool
#[derive(Debug, Deserialize)]
pub struct SetNodePromptArgs {
    /// UUID or name of the node to annotate.
    pub node_id: String,
    /// The prompt text to record.
    pub prompt: String,
    /// How to add the prompt: "append" (default), "prepend", or "replace" (clears history first).
    #[serde(default)]
    pub mode: Option<String>,
}

/// Arguments for `get_node_prompts` tool
#[derive(Debug, Deserialize)]
pub struct GetNodePromptsArgs {
    /// UUID or name of the node.
    pub node_id: String,
}

// ─── ReverseNodeOrderArgs ─────────────────────────────────────────────────────

/// Arguments for `reverse_node_order` tool
#[derive(Debug, Deserialize)]
pub struct ReverseNodeOrderArgs {
    /// UUIDs or names of group nodes whose children order should be reversed.
    /// Uses current selection if empty.
    #[serde(default)]
    pub node_ids: Vec<String>,
}

// ─── RotateCopiesArgs ─────────────────────────────────────────────────────────

/// Arguments for `rotate_copies` tool
#[derive(Debug, Deserialize)]
pub struct RotateCopiesArgs {
    /// UUID or name of the node to copy and rotate.
    pub node_id: String,
    /// Total number of copies in the radial arrangement (including the original). Minimum: 2.
    pub count: usize,
    /// X coordinate of the rotation center in document units. Defaults to the node's bounding-box center.
    #[serde(default)]
    pub cx: Option<f64>,
    /// Y coordinate of the rotation center in document units. Defaults to the node's bounding-box center.
    #[serde(default)]
    pub cy: Option<f64>,
    /// When true, wrap all copies (including original) in a new Group node. Default: false.
    #[serde(default)]
    pub group: bool,
}

// ─── CopyAppearanceArgs ───────────────────────────────────────────────────────

/// Arguments for `copy_appearance` tool
#[derive(Debug, Deserialize)]
pub struct CopyAppearanceArgs {
    /// UUID or name of the source node to copy appearance from.
    pub source_id: String,
    /// UUIDs or names of target nodes to apply the appearance to.
    pub target_ids: Vec<String>,
    /// Copy fill. Default: true.
    #[serde(default = "default_true")]
    pub copy_fill: bool,
    /// Copy stroke. Default: true.
    #[serde(default = "default_true")]
    pub copy_stroke: bool,
    /// Copy opacity. Default: true.
    #[serde(default = "default_true")]
    pub copy_opacity: bool,
}

// ─── MirrorCopyArgs ───────────────────────────────────────────────────────────

/// Arguments for `mirror_copy` tool
#[derive(Debug, Deserialize)]
pub struct MirrorCopyArgs {
    /// UUIDs or names of nodes to mirror. Uses current selection if empty.
    #[serde(default)]
    pub node_ids: Vec<String>,
    /// "horizontal" — flip left-right (default), or "vertical" — flip top-bottom.
    #[serde(default)]
    pub axis: Option<String>,
}

// ─── NoiseDeformArgs ──────────────────────────────────────────────────────────

/// Arguments for `noise_deform` tool
#[derive(Debug, Deserialize)]
pub struct NoiseDeformArgs {
    /// UUIDs or names of path nodes to deform.
    pub node_ids: Vec<String>,
    /// Maximum displacement amplitude in document units (default: 8.0).
    pub amplitude: Option<f64>,
    /// Spatial frequency: higher = tighter waves (default: 0.05 cycles/px).
    pub frequency: Option<f64>,
    /// Phase seed — shifts the wave pattern (default: 0.0).
    pub seed: Option<f64>,
    /// Axis to deform: "both" (default), "x", or "y".
    #[serde(default)]
    pub axis: Option<String>,
}

// ─── DistributeNoOverlapArgs ──────────────────────────────────────────────────

/// Arguments for `distribute_no_overlap` tool
#[derive(Debug, Deserialize)]
pub struct DistributeNoOverlapArgs {
    /// UUIDs or names of nodes to un-overlap. Uses current selection if empty.
    #[serde(default)]
    pub node_ids: Vec<String>,
    /// Minimum gap between bounding boxes in px (default: 4.0).
    pub padding: Option<f64>,
    /// Maximum number of resolution iterations (default: 100, max: 500).
    pub max_iterations: Option<usize>,
}

/// Arguments for `snap_to_pixel` tool
#[derive(Debug, Deserialize)]
pub struct SnapToPixelArgs {
    /// IDs of nodes whose position should be rounded to the nearest integer.
    pub node_ids: Vec<Uuid>,
}

// ─── Scissors Cut Args ───────────────────────────────────────────────────────

/// Arguments for the `scissors_cut` tool.
#[derive(Debug, Deserialize)]
pub struct ScissorsCutArgs {
    /// ID of the path node to cut.
    pub node_id: Uuid,
    /// X coordinate in document (canvas) space of the cut point.
    pub canvas_x: f64,
    /// Y coordinate in document (canvas) space of the cut point.
    pub canvas_y: f64,
}

// ─── Guide Args ──────────────────────────────────────────────────────────────

/// Arguments for `add_guide` tool.
#[derive(Debug, Deserialize)]
pub struct AddGuideArgs {
    /// "horizontal" for a fixed-Y guide, "vertical" for a fixed-X guide.
    pub orientation: String,
    /// Position in document units (Y for horizontal, X for vertical).
    pub position: f64,
    /// Optional override color as [R, G, B, A] in [0,1] range.
    #[serde(default)]
    pub color: Option<[f32; 4]>,
}

/// Arguments for `remove_guide` tool.
#[derive(Debug, Deserialize)]
pub struct RemoveGuideArgs {
    /// UUID of the guide to remove.
    pub guide_id: Uuid,
}

/// Arguments for `list_guides` and `clear_guides` (no parameters).
#[derive(Debug, Deserialize, Default)]
pub struct ListGuidesArgs {}

#[derive(Debug, Deserialize, Default)]
pub struct ClearGuidesArgs {}

// ─── Magic Wand Select Args ───────────────────────────────────────────────────

/// Arguments for the `magic_wand_select` tool.
#[derive(Debug, Deserialize)]
pub struct MagicWandSelectArgs {
    /// X coordinate in document (canvas) space to click.
    pub canvas_x: f64,
    /// Y coordinate in document (canvas) space to click.
    pub canvas_y: f64,
    /// Which attribute to match across all nodes.
    #[serde(default)]
    pub attribute: SelectSameAttribute,
    /// Tolerance for numeric/color comparisons. Defaults to 0.01.
    #[serde(default)]
    pub tolerance: Option<f64>,
}

// ─── Convert Anchor Points Args ──────────────────────────────────────────────

/// Arguments for the `convert_anchor_points` tool.
#[derive(Debug, Deserialize, Default)]
pub struct ConvertAnchorPointsArgs {
    /// IDs of path nodes to convert. Non-path nodes are skipped.
    pub node_ids: Vec<Uuid>,
    /// Conversion mode: "smooth" makes junction handles collinear; "corner" retracts handles to anchor points (cusps).
    #[serde(default)]
    pub mode: ConvertAnchorMode,
}

/// Anchor point conversion mode.
#[derive(Debug, Deserialize, Default, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConvertAnchorMode {
    /// Make junction handles collinear through each interior anchor.
    #[default]
    Smooth,
    /// Retract cubic handles to their anchor points (sharp cusps).
    Corner,
}

// ─── Lasso Select Args ───────────────────────────────────────────────────────

/// Arguments for the `lasso_select` tool.
#[derive(Debug, Deserialize, Default)]
pub struct LassoSelectArgs {
    /// Polygon boundary in canvas (document) coordinates. Each element is `[x, y]`.
    /// Minimum 3 points. The polygon is automatically closed.
    pub points: Vec<[f64; 2]>,
    /// When true (default), select nodes whose bounding-box centroid is inside the polygon.
    /// When false, select nodes whose AABB fully intersects — i.e. at least one corner is inside.
    #[serde(default = "default_true")]
    pub centroid_mode: bool,
    /// When true, add to the existing selection instead of replacing it.
    #[serde(default)]
    pub additive: bool,
}

// ─── select_by_kind ──────────────────────────────────────────────────────────

/// Object kind selector for `select_by_kind`.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ObjectKindFilter {
    #[default]
    Path,
    Text,
    Group,
    /// Select all nodes on the same layer as the currently active layer.
    SameLayer,
}

/// Arguments for `select_by_kind`.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct SelectByKindArgs {
    /// Which object type to select.
    #[serde(default)]
    pub kind: ObjectKindFilter,
    /// When true, add to the existing selection instead of replacing it.
    #[serde(default)]
    pub additive: bool,
}

// ─── create_freehand_path ────────────────────────────────────────────────────

/// Arguments for `create_freehand_path`.
#[derive(Debug, Clone, Deserialize)]
pub struct CreateFreehandPathArgs {
    /// Ordered list of `[x, y]` canvas-space points defining the stroke.
    /// Must contain at least 2 points.
    pub points: Vec<[f64; 2]>,
    /// Optional fill. Defaults to no fill (stroke-only).
    #[serde(default)]
    pub fill: Option<FillArg>,
    /// Optional stroke override. Defaults to the document default stroke.
    #[serde(default)]
    pub stroke: Option<StrokeArg>,
    /// Optional name. Defaults to "Pencil".
    #[serde(default)]
    pub name: Option<String>,
}

// ─── Isolation Mode ──────────────────────────────────────────────────────────

/// Arguments for `enter_isolation_mode`.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct EnterIsolationModeArgs {
    /// The group node to isolate. Only its children will be selectable.
    pub group_id: Uuid,
}

/// Arguments for `exit_isolation_mode` — no parameters needed.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ExitIsolationModeArgs {}

// ─── select_inside_group ─────────────────────────────────────────────────────

/// Arguments for `select_inside_group`.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct SelectInsideGroupArgs {
    /// The group node whose direct children should become the new selection.
    pub group_id: Uuid,
    /// When true, add children to the existing selection instead of replacing it.
    #[serde(default)]
    pub additive: bool,
}

// ─── smooth_path ─────────────────────────────────────────────────────────────

/// Arguments for `smooth_path`.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct SmoothPathArgs {
    /// IDs of path nodes to smooth. If empty, applies to all currently selected path nodes.
    #[serde(default)]
    pub node_ids: Vec<Uuid>,
    /// Smoothing strength in [0, 0.5]. 0.25 is the classic Chaikin corner-cutting value.
    /// Values closer to 0.5 produce rounder curves. Default 0.25.
    #[serde(default = "default_smooth_factor")]
    pub factor: f64,
    /// Number of smoothing passes. More passes = smoother result. Default 2, max 8.
    #[serde(default = "default_smooth_iterations")]
    pub iterations: u32,
}

fn default_smooth_factor() -> f64 {
    0.25
}
fn default_smooth_iterations() -> u32 {
    2
}

// ─── get_recent_colors ───────────────────────────────────────────────────────

/// Arguments for `get_recent_colors` — none required (document-level query).
#[derive(Debug, Clone, Deserialize)]
pub struct GetRecentColorsArgs {}

// ─── Color Swatch Args ───────────────────────────────────────────────────────

/// Arguments for `add_color_swatch` tool
#[derive(Debug, Deserialize)]
pub struct AddColorSwatchArgs {
    /// Unique name for this swatch.
    pub name: String,
    /// Color as CSS hex e.g. "#FF5733" or "FF5733".
    pub color_hex: String,
}

/// Arguments for `apply_color_swatch` tool
#[derive(Debug, Deserialize)]
pub struct ApplyColorSwatchArgs {
    /// Name of the swatch to apply.
    pub swatch_name: String,
    /// UUIDs or names of nodes to apply fill color to. Uses current selection if empty.
    #[serde(default)]
    pub node_ids: Vec<String>,
    /// Apply to "fill" (default), "stroke", or "both".
    #[serde(default)]
    pub target: Option<String>,
}

/// Arguments for `update_color_swatch` tool
#[derive(Debug, Deserialize)]
pub struct UpdateColorSwatchArgs {
    /// Current name of the swatch to update.
    pub name: String,
    /// New color hex value. All nodes whose fill/stroke matches the old color are updated.
    #[serde(default)]
    pub new_color_hex: Option<String>,
    /// New name for the swatch.
    #[serde(default)]
    pub new_name: Option<String>,
    /// When true (default), update all nodes whose fill matches the old color.
    #[serde(default = "default_true_bool")]
    pub propagate: bool,
}

fn default_true_bool() -> bool {
    true
}

/// Arguments for `delete_color_swatch` tool
#[derive(Debug, Deserialize)]
pub struct DeleteColorSwatchArgs {
    /// Name of the swatch to delete.
    pub name: String,
}

// ─── Graphic Style Args ───────────────────────────────────────────────────────

/// Arguments for `define_graphic_style` tool
#[derive(Debug, Deserialize)]
pub struct DefineGraphicStyleArgs {
    /// Unique name for the graphic style. If it already exists, it will be overwritten.
    pub name: String,
    /// Node UUID or name to capture fill, stroke, and opacity from. Optional — omit to define from explicit parameters.
    #[serde(default)]
    pub node_id: Option<String>,
    /// Fill color as hex string (e.g. "#ff0000"). Used only if node_id is not provided.
    #[serde(default)]
    pub fill_hex: Option<String>,
    /// Stroke color as hex string. Used only if node_id is not provided.
    #[serde(default)]
    pub stroke_hex: Option<String>,
    /// Stroke width. Used only if node_id is not provided.
    #[serde(default)]
    pub stroke_width: Option<f64>,
    /// Node opacity (0.0–1.0). Default 1.0.
    #[serde(default)]
    pub opacity: Option<f32>,
}

/// Arguments for `apply_graphic_style` tool
#[derive(Debug, Deserialize)]
pub struct ApplyGraphicStyleArgs {
    /// Node UUIDs or names to apply the style to.
    pub node_ids: Vec<String>,
    /// Name of the graphic style to apply.
    pub name: String,
}

/// Arguments for `delete_graphic_style` tool
#[derive(Debug, Deserialize)]
pub struct DeleteGraphicStyleArgs {
    /// Name of the graphic style to delete.
    pub name: String,
}

// ─── Width Profile Args ───────────────────────────────────────────────────────

/// Arguments for `define_width_profile` tool
#[derive(Debug, Deserialize)]
pub struct DefineWidthProfileArgs {
    /// Unique name for the width profile. Overwrites existing profile with same name.
    pub name: String,
    /// Width samples at even t intervals along the path (in document units, ≥2 values).
    /// E.g. [1.0, 4.0, 1.0] = thin at ends, thick in the middle.
    pub widths: Vec<f64>,
}

/// Arguments for `apply_width_profile` tool
#[derive(Debug, Deserialize)]
pub struct ApplyWidthProfileArgs {
    /// Node UUIDs or names to apply the width profile to.
    pub node_ids: Vec<String>,
    /// Name of the width profile to apply.
    pub name: String,
}

/// Arguments for `delete_width_profile` tool
#[derive(Debug, Deserialize)]
pub struct DeleteWidthProfileArgs {
    /// Name of the width profile to delete.
    pub name: String,
}

/// Arguments for `set_constraint` tool
#[derive(Debug, Deserialize)]
pub struct SetConstraintArgs {
    /// Target node UUID or name whose property is driven by the expression.
    pub node_id: String,
    /// Target property: one of `x`, `y`, `opacity`, `font_size`.
    pub property: String,
    /// Arithmetic expression; may reference `nodes['<id-or-name>'].<prop>`
    /// (e.g. `nodes['logo'].x + 20`).
    pub expression: String,
}

/// Arguments for `remove_constraint` tool
#[derive(Debug, Deserialize)]
pub struct RemoveConstraintArgs {
    /// UUID of the constraint to remove.
    pub constraint_id: String,
}

// ─── Swatch Library Args ─────────────────────────────────────────────────────

/// Arguments for `load_swatch_library` tool
#[derive(Debug, Deserialize, Default)]
pub struct LoadSwatchLibraryArgs {
    /// Library name to load. One of: "web", "material", "pastels", "earth_tones", "neon", "grayscale".
    pub library: String,
    /// If true, remove all existing swatches before loading. Default: false (append).
    #[serde(default)]
    pub clear_existing: bool,
}

// ─── Symbols Args ────────────────────────────────────────────────────────────

/// Arguments for `define_symbol` tool
#[derive(Debug, Deserialize)]
pub struct DefineSymbolArgs {
    /// Node ID (UUID or name) to designate as the symbol master.
    pub node_id: String,
    /// Unique symbol name.
    pub name: String,
}

/// Arguments for `place_symbol` tool
#[derive(Debug, Deserialize)]
pub struct PlaceSymbolArgs {
    /// Symbol name to instantiate.
    pub symbol_name: String,
    /// X position of the instance (document units).
    #[serde(default)]
    pub x: f64,
    /// Y position of the instance (document units).
    #[serde(default)]
    pub y: f64,
}

/// Arguments for `break_link_to_symbol` tool
#[derive(Debug, Deserialize)]
pub struct BreakLinkToSymbolArgs {
    /// Instance node ID (UUID or name) to detach from its symbol.
    pub node_id: String,
}

/// Arguments for `delete_symbol` tool
#[derive(Debug, Deserialize)]
pub struct DeleteSymbolArgs {
    /// Symbol name to remove from the registry.
    pub name: String,
}

// ─── Gradient Swatch Args ────────────────────────────────────────────────────

/// Arguments for `save_gradient_swatch` tool
#[derive(Debug, Deserialize)]
pub struct SaveGradientSwatchArgs {
    /// Path/text node ID (UUID or name) whose gradient fill should be saved.
    pub node_id: String,
    /// Unique name for the swatch.
    pub name: String,
}

/// Arguments for `apply_gradient_swatch` tool
#[derive(Debug, Deserialize)]
pub struct ApplyGradientSwatchArgs {
    /// Path node ID(s) (UUID or name) to apply the swatch to.
    pub node_ids: Vec<String>,
    /// Name of the gradient swatch to apply.
    pub name: String,
}

/// Arguments for `delete_gradient_swatch` tool
#[derive(Debug, Deserialize)]
pub struct DeleteGradientSwatchArgs {
    /// Name of the gradient swatch to delete.
    pub name: String,
}

// ─── Navigator Args ──────────────────────────────────────────────────────────

/// Arguments for `get_canvas_overview` tool (no required parameters).
#[derive(Debug, Deserialize, Default)]
pub struct GetCanvasOverviewArgs {
    /// When true, include invisible/hidden nodes. Default: false.
    #[serde(default)]
    pub include_hidden: bool,
}

// ─── Font Style Args ─────────────────────────────────────────────────────────

/// Arguments for `set_font_style` tool
#[derive(Debug, Deserialize)]
pub struct SetFontStyleArgs {
    /// Text node ID (UUID or name).
    pub node_id: String,
    /// Font style: "normal", "italic", or "oblique".
    pub style: String,
}

/// Arguments for `set_font_weight` tool
#[derive(Debug, Deserialize)]
pub struct SetFontWeightArgs {
    /// Text node ID (UUID or name).
    pub node_id: String,
    /// Font weight (100–900, e.g. 400 = Regular, 700 = Bold).
    pub weight: u16,
}

// ─── Variables Args ───────────────────────────────────────────────────────────

/// Arguments for `define_variable` tool
#[derive(Debug, Deserialize)]
pub struct DefineVariableArgs {
    /// Unique variable name.
    pub name: String,
    /// Initial string value.
    pub value: String,
}

/// Arguments for `set_variable_value` tool
#[derive(Debug, Deserialize)]
pub struct SetVariableValueArgs {
    /// Variable name to update.
    pub name: String,
    /// New string value.
    pub value: String,
}

/// Arguments for `delete_variable` tool
#[derive(Debug, Deserialize)]
pub struct DeleteVariableArgs {
    /// Variable name to delete.
    pub name: String,
}

/// Arguments for `bind_text_variable` tool
#[derive(Debug, Deserialize)]
pub struct BindTextVariableArgs {
    /// Text node ID (UUID or name).
    pub node_id: String,
    /// Variable name to bind.
    pub variable_name: String,
}

/// Arguments for `unbind_text_variable` tool
#[derive(Debug, Deserialize)]
pub struct UnbindTextVariableArgs {
    /// Text node ID (UUID or name).
    pub node_id: String,
}

// ─── Area Type Args ───────────────────────────────────────────────────────────

/// Arguments for `set_text_area` tool
#[derive(Debug, Deserialize)]
pub struct SetTextAreaArgs {
    /// Text node ID (UUID or name) to flow inside the area path.
    pub text_node_id: String,
    /// Closed path node ID (UUID or name) that defines the text boundary.
    pub area_path_id: String,
}

/// Arguments for `clear_text_area` tool
#[derive(Debug, Deserialize)]
pub struct ClearTextAreaArgs {
    /// Text node ID (UUID or name) to remove the area boundary from.
    pub text_node_id: String,
}

// ─── Text Direction Args ─────────────────────────────────────────────────────

/// Arguments for `set_text_direction` tool
#[derive(Debug, Deserialize)]
pub struct SetTextDirectionArgs {
    /// Text node ID (UUID or name).
    pub node_id: String,
    /// When true, text flows top-to-bottom (vertical). When false, normal horizontal layout.
    pub vertical: bool,
}

// ─── Type on a Path Args ─────────────────────────────────────────────────────

/// Arguments for `set_text_path` tool
#[derive(Debug, Deserialize)]
pub struct SetTextPathArgs {
    /// Text node ID (UUID or name) to place on the path.
    pub text_node_id: String,
    /// Path node ID (UUID or name) to use as the text spine.
    pub path_node_id: String,
    /// Start offset along the path in document units. Default: 0.0.
    #[serde(default)]
    pub offset: f64,
}

/// Arguments for `clear_text_path` tool
#[derive(Debug, Deserialize)]
pub struct ClearTextPathArgs {
    /// Text node ID (UUID or name) to remove the path spine from.
    pub text_node_id: String,
}

// ─── Clipping Mask Args ──────────────────────────────────────────────────────

/// Arguments for `make_clipping_mask` tool
#[derive(Debug, Deserialize)]
pub struct MakeClippingMaskArgs {
    /// Group node ID (UUID or name). The topmost child of the group becomes the clip path.
    pub group_id: String,
}

/// Arguments for `release_clipping_mask` tool
#[derive(Debug, Deserialize)]
pub struct ReleaseClippingMaskArgs {
    /// Group node ID (UUID or name) to release the clipping mask from.
    pub group_id: String,
}

// ─── Paragraph Style Args ────────────────────────────────────────────────────

/// Arguments for `create_paragraph_style` tool
#[derive(Debug, Deserialize)]
pub struct CreateParagraphStyleArgs {
    /// Unique name. Replaces any existing style with the same name.
    pub name: String,
    /// Source text node UUID or name to capture layout from.
    #[serde(default)]
    pub source_node_id: Option<String>,
    /// Text alignment: "left", "center", "right", or "justify".
    #[serde(default)]
    pub align: Option<String>,
    /// Line height multiplier.
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

/// Arguments for `apply_paragraph_style` tool
#[derive(Debug, Deserialize)]
pub struct ApplyParagraphStyleArgs {
    /// Name of the style to apply.
    pub style_name: String,
    /// UUIDs or names of text nodes. Uses current selection if empty.
    #[serde(default)]
    pub node_ids: Vec<String>,
}

/// Arguments for `delete_paragraph_style` tool
#[derive(Debug, Deserialize)]
pub struct DeleteParagraphStyleArgs {
    /// Name of the style to delete.
    pub name: String,
}

// ─── Character Style Args ────────────────────────────────────────────────────

/// Arguments for `create_character_style` tool
#[derive(Debug, Deserialize)]
pub struct CreateCharacterStyleArgs {
    /// Unique name for this style. If a style with this name already exists it is replaced.
    pub name: String,
    /// Source node UUID or name to capture style from. All specified fields override the node's values.
    #[serde(default)]
    pub source_node_id: Option<String>,
    /// Font family override.
    #[serde(default)]
    pub font_family: Option<String>,
    /// Font size override.
    #[serde(default)]
    pub font_size: Option<f64>,
    /// Font weight override (100–900).
    #[serde(default)]
    pub font_weight: Option<u16>,
    /// Fill color override as CSS hex (e.g. "#FF5733").
    #[serde(default)]
    pub fill_hex: Option<String>,
    /// Letter spacing override in document units.
    #[serde(default)]
    pub letter_spacing: Option<f64>,
    /// Line height multiplier override.
    #[serde(default)]
    pub line_height: Option<f64>,
}

/// Arguments for `apply_character_style` tool
#[derive(Debug, Deserialize)]
pub struct ApplyCharacterStyleArgs {
    /// Name of the style to apply.
    pub style_name: String,
    /// UUIDs or names of text nodes to apply the style to. Uses current selection if empty.
    #[serde(default)]
    pub node_ids: Vec<String>,
}

/// Arguments for `delete_character_style` tool
#[derive(Debug, Deserialize)]
pub struct DeleteCharacterStyleArgs {
    /// Name of the style to delete.
    pub name: String,
}

// ─── Asset Export Args ───────────────────────────────────────────────────────

/// Arguments for `tag_node_for_export` tool
#[derive(Debug, Deserialize)]
pub struct TagNodeForExportArgs {
    /// UUID or name of the node to tag.
    pub node_id: String,
    /// Base name for the exported asset (without extension). Leave empty to remove the tag.
    pub name: String,
    /// Export format: "svg" (default), "png", "jpeg", or "webp".
    #[serde(default)]
    pub format: Option<String>,
    /// Scale multipliers for raster exports (e.g. [1.0, 2.0]).  Ignored for SVG.
    #[serde(default)]
    pub scales: Vec<f64>,
}

/// Arguments for `export_tagged_assets` tool
#[derive(Debug, Deserialize, Default)]
pub struct ExportTaggedAssetsArgs {
    /// When true, only export nodes whose `name` contains this string.
    #[serde(default)]
    pub filter: Option<String>,
}

// ─── SelectSimilarArgs ───────────────────────────────────────────────────────

/// Arguments for `select_similar` tool
#[derive(Debug, Deserialize)]
pub struct SelectSimilarArgs {
    /// UUID or name of the reference node(s). If empty, uses the current selection.
    #[serde(default)]
    pub node_ids: Vec<String>,
    /// Comma-separated attributes to match. Any of: fill_color, stroke_color,
    /// stroke_width, kind, opacity, tags. Default: "fill_color".
    #[serde(default)]
    pub match_by: Option<String>,
    /// Color match tolerance 0–255 per channel. Default: 5.
    #[serde(default)]
    pub tolerance: Option<u8>,
    /// When true, add matches to the existing selection instead of replacing it. Default: false.
    #[serde(default)]
    pub additive: bool,
}

// ─── Flatten Transparency Args ───────────────────────────────────────────────

/// Arguments for `flatten_transparency` tool
#[derive(Debug, Deserialize, Default)]
pub struct FlattenTransparencyArgs {
    /// Optional subset of node UUIDs or names to process. Defaults to all nodes.
    #[serde(default)]
    pub node_ids: Vec<String>,
}

// ─── Construction Line Args ──────────────────────────────────────────────────

/// Arguments for `add_construction_line` tool
#[derive(Debug, Deserialize)]
pub struct AddConstructionLineArgs {
    /// X coordinate (document units) for the line's origin point.
    pub x: f64,
    /// Y coordinate (document units) for the line's origin point.
    pub y: f64,
    /// Angle of the line in degrees. 0° = horizontal, 90° = vertical, 45° = diagonal.
    pub angle_degrees: f64,
    /// Optional color as a hex string (e.g. "#FF8800"). Default: orange.
    #[serde(default)]
    pub color: Option<String>,
}

// ─── Document Bleed Args ─────────────────────────────────────────────────────

/// Arguments for `set_document_bleed` tool
#[derive(Debug, Deserialize, Default)]
pub struct SetDocumentBleedArgs {
    /// Bleed size in millimetres (all four sides). Pass `null` to leave unchanged. Default: no change.
    #[serde(default)]
    pub bleed_mm: Option<f64>,
    /// Slug size in millimetres (area outside bleed for printer marks). Pass `null` to leave unchanged.
    #[serde(default)]
    pub slug_mm: Option<f64>,
}

// ─── Artboard Margins Args ───────────────────────────────────────────────────

/// Arguments for `set_artboard_margins` tool.
#[derive(Debug, Deserialize, Default)]
pub struct SetArtboardMarginsArgs {
    /// Top margin in document units. Pass `null` to leave unchanged.
    #[serde(default)]
    pub top: Option<f64>,
    /// Right margin in document units. Pass `null` to leave unchanged.
    #[serde(default)]
    pub right: Option<f64>,
    /// Bottom margin in document units. Pass `null` to leave unchanged.
    #[serde(default)]
    pub bottom: Option<f64>,
    /// Left margin in document units. Pass `null` to leave unchanged.
    #[serde(default)]
    pub left: Option<f64>,
}

// ─── Text Frame Threading Args ───────────────────────────────────────────────

/// Arguments for `link_text_frames` tool.
#[derive(Debug, Deserialize)]
pub struct LinkTextFramesArgs {
    /// ID or name of the source (upstream) text node — overflow flows out from here.
    pub from_id: String,
    /// ID or name of the destination (downstream) text node — overflow flows into here.
    pub to_id: String,
}

/// Arguments for `unlink_text_frames` tool.
#[derive(Debug, Deserialize)]
pub struct UnlinkTextFramesArgs {
    /// ID or name of a text node to remove from any thread chain.
    pub node_id: String,
}

// ─── Event Trigger Args ───────────────────────────────────────────────────────

/// Arguments for `register_event_trigger` tool.
#[derive(Debug, Deserialize)]
pub struct RegisterEventTriggerArgs {
    /// Event name: "on_open", "on_save", "on_node_create", or "on_selection_change".
    pub event: String,
    /// Name of the action set to execute when the event fires.
    pub action_name: String,
}

/// Arguments for `remove_event_trigger` tool.
#[derive(Debug, Deserialize)]
pub struct RemoveEventTriggerArgs {
    /// Event name to remove triggers for.
    pub event: String,
    /// Optional: only remove the trigger pointing to this action name.
    /// If omitted, removes all triggers for the event.
    #[serde(default)]
    pub action_name: Option<String>,
}

// ─── OpenType Feature Args ───────────────────────────────────────────────────

/// Arguments for `set_opentype_features` tool.
#[derive(Debug, Deserialize)]
pub struct SetOpenTypeFeaturesArgs {
    /// ID or name of the text node to update.
    pub node_id: String,
    /// OpenType feature tags to apply, e.g. ["liga", "calt", "frac"].
    pub features: Vec<String>,
    /// How to apply: "set" replaces all features, "add" appends unique entries,
    /// "remove" removes listed entries. Default: "set".
    #[serde(default)]
    pub mode: String,
}

/// Arguments for `get_opentype_features` tool.
#[derive(Debug, Deserialize)]
pub struct GetOpenTypeFeaturesArgs {
    /// ID or name of the text node.
    pub node_id: String,
}

// ─── Text Decoration Args ─────────────────────────────────────────────────────

/// Arguments for `set_text_decoration` tool.
#[derive(Debug, Deserialize)]
pub struct SetTextDecorationArgs {
    /// ID or name of the text node.
    pub node_id: String,
    /// Decoration: "" or "none" (removes decoration), "underline", "line-through", or "overline".
    pub decoration: String,
}

// ─── Paragraph Options Args ───────────────────────────────────────────────────

/// Arguments for `set_paragraph_options` tool.
#[derive(Debug, Deserialize, Default)]
pub struct SetParagraphOptionsArgs {
    /// ID or name of the text node.
    pub node_id: String,
    /// Space before each paragraph in document units. Pass null to leave unchanged.
    #[serde(default)]
    pub spacing_before: Option<f64>,
    /// Space after each paragraph in document units. Pass null to leave unchanged.
    #[serde(default)]
    pub spacing_after: Option<f64>,
    /// First-line indent in document units. Pass null to leave unchanged.
    #[serde(default)]
    pub indent: Option<f64>,
}

// ─── List History Args ────────────────────────────────────────────────────────

/// Arguments for `list_history` tool
#[derive(Debug, Deserialize, Default)]
pub struct ListHistoryArgs {
    /// Maximum number of history entries to return, newest first. Default: 20.
    #[serde(default)]
    pub limit: Option<usize>,
}

/// Arguments for `jump_to_history` tool
#[derive(Debug, Deserialize, Default)]
pub struct JumpToHistoryArgs {
    /// Target undo-stack depth to jump to.
    /// 0 = fully undone (empty document). undo_depth() = current state (no change).
    /// Values beyond undo_depth() + redo_depth() are clamped to the maximum.
    pub index: usize,
}

// ─── Dimension Annotation Args ────────────────────────────────────────────────

/// Arguments for `fit_to_margins` tool
#[derive(Debug, Deserialize)]
pub struct FitToMarginsArgs {
    /// Node UUIDs or names to fit. If empty or omitted, all visible nodes on all layers are fitted.
    #[serde(default)]
    pub node_ids: Vec<String>,
    /// When true (default), preserve each node's aspect ratio while scaling.
    #[serde(default = "default_true")]
    pub uniform: bool,
    /// Additional inset inside the margin rectangle in document units. Default: 0.
    #[serde(default)]
    pub padding: f64,
}

/// Arguments for `add_dimension` tool
#[derive(Debug, Deserialize)]
pub struct AddDimensionArgs {
    /// UUID or name of the first node.
    pub from_node_id: String,
    /// UUID or name of the second node.
    pub to_node_id: String,
    /// Measurement axis: "x" (horizontal), "y" (vertical), or "diagonal" (Euclidean). Default: "diagonal".
    #[serde(default)]
    pub axis: Option<String>,
    /// Perpendicular offset from the dimension line in document units (for visual clearance). Default: 20.
    #[serde(default)]
    pub label_offset: Option<f64>,
}

/// Arguments for `remove_dimension` tool
#[derive(Debug, Deserialize)]
pub struct RemoveDimensionArgs {
    /// UUID of the dimension annotation to remove.
    pub id: String,
}

// ─── Undo Node Args ───────────────────────────────────────────────────────────

/// Arguments for `undo_node` tool
#[derive(Debug, Deserialize)]
pub struct UndoNodeArgs {
    /// UUID or name of the node to revert.
    pub node_id: String,
    /// How many node-specific history steps to revert. Default: 1.
    #[serde(default)]
    pub steps: Option<usize>,
}

// ─── Spot Color Args ──────────────────────────────────────────────────────────

/// Arguments for `define_spot_color` tool
#[derive(Debug, Deserialize)]
pub struct DefineSpotColorArgs {
    /// Unique name for the spot color (e.g. "Pantone 485 C").
    pub name: String,
    /// Hex color value (e.g. "#FF2400"). With or without leading #.
    pub hex: String,
    /// When true, this ink overprints underlying inks. Default: false.
    #[serde(default)]
    pub overprint: bool,
}

/// Arguments for `apply_spot_color` tool
#[derive(Debug, Deserialize)]
pub struct ApplySpotColorArgs {
    /// UUID(s) or name(s) of nodes to apply the spot color to.
    pub node_ids: Vec<String>,
    /// Name of the spot color to apply.
    pub name: String,
}

/// Arguments for `delete_spot_color` tool
#[derive(Debug, Deserialize)]
pub struct DeleteSpotColorArgs {
    /// Name of the spot color to delete.
    pub name: String,
}

// ─── Branch Args ─────────────────────────────────────────────────────────────

/// Arguments for `branch_create` tool
#[derive(Debug, Deserialize)]
pub struct BranchCreateArgs {
    /// Name for the new branch. Overwrites any existing branch with this name.
    pub name: String,
}

/// Arguments for `branch_switch` tool
#[derive(Debug, Deserialize)]
pub struct BranchSwitchArgs {
    /// Name of the branch to restore.
    pub name: String,
}

/// Arguments for `branch_delete` tool
#[derive(Debug, Deserialize)]
pub struct BranchDeleteArgs {
    /// Name of the branch to delete.
    pub name: String,
}

// ─── Composition Analysis Args ───────────────────────────────────────────────

/// Arguments for `apply_flex_layout` tool
#[derive(Debug, Deserialize, Default)]
pub struct ApplyFlexLayoutArgs {
    /// UUID or name of the Group node whose children will be repositioned.
    pub group_id: String,
    /// Main axis direction: `"row"` (left to right) or `"column"` (top to bottom). Default: `"row"`.
    #[serde(default)]
    pub direction: Option<String>,
    /// Gap in document units between consecutive children. Default: 8.0.
    #[serde(default)]
    pub gap: Option<f64>,
    /// Cross-axis alignment: `"start"`, `"center"`, or `"end"`. Default: `"center"`.
    #[serde(default)]
    pub align: Option<String>,
    /// Padding around the group's content area (offsets the starting position). Default: 0.0.
    #[serde(default)]
    pub padding: Option<f64>,
}

/// Arguments for `apply_stack_layout` tool
#[derive(Debug, Deserialize, Default)]
pub struct ApplyStackLayoutArgs {
    /// UUID or name of the Group node whose children will be stacked.
    pub group_id: String,
    /// Horizontal anchor for stacking: "left", "center" (default), or "right".
    #[serde(default)]
    pub align_h: Option<String>,
    /// Vertical anchor for stacking: "top", "center" (default), or "bottom".
    #[serde(default)]
    pub align_v: Option<String>,
}

/// Arguments for `apply_grid_layout` tool
#[derive(Debug, Deserialize, Default)]
pub struct ApplyGridLayoutArgs {
    /// UUID or name of the Group node whose children will be laid out.
    pub group_id: String,
    /// Number of columns. Default: 3.
    #[serde(default)]
    pub columns: Option<usize>,
    /// Horizontal gap between columns in document units. Default: 8.0.
    #[serde(default)]
    pub gap_x: Option<f64>,
    /// Vertical gap between rows in document units. Default: 8.0.
    #[serde(default)]
    pub gap_y: Option<f64>,
    /// Padding around the grid origin. Default: 0.0.
    #[serde(default)]
    pub padding: Option<f64>,
}

/// Arguments for `analyze_composition` tool
#[derive(Debug, Deserialize, Default)]
pub struct AnalyzeCompositionArgs {
    /// Optional subset of node UUIDs/names to analyze. Defaults to all visible nodes.
    #[serde(default)]
    pub node_ids: Vec<String>,
}

/// One step in a recorded action sequence.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ActionStep {
    pub tool: String,
    pub args: serde_json::Value,
}

/// Arguments for `define_action` tool
#[derive(Debug, Deserialize, Default)]
pub struct DefineActionArgs {
    /// Unique name for this action set.
    pub name: String,
    /// Ordered list of tool steps to execute when the action is played.
    pub steps: Vec<ActionStep>,
}

/// Arguments for `play_action` tool
#[derive(Debug, Deserialize, Default)]
pub struct PlayActionArgs {
    /// Name of the action set to play.
    pub name: String,
    /// Optional node ID substitutions: each key is a node UUID/name in the recorded steps;
    /// its value is the new UUID/name to use during playback.
    #[serde(default)]
    pub substitutions: std::collections::HashMap<String, String>,
}

/// Arguments for `delete_action` tool
#[derive(Debug, Deserialize, Default)]
pub struct DeleteActionArgs {
    /// Name of the action set to delete.
    pub name: String,
}

/// Arguments for `measure_distances` tool
#[derive(Debug, Deserialize, Default)]
pub struct MeasureDistancesArgs {
    /// Node UUIDs or names to measure distances between. Must have at least 2 nodes.
    pub node_ids: Vec<String>,
}

/// Arguments for `define_grammar_rule` tool
#[derive(Debug, Deserialize, Default)]
pub struct DefineGrammarRuleArgs {
    /// Human-readable name for the rule (used as a reference key in results).
    pub name: String,
    /// Rule type discriminator: `palette_includes`, `max_colors`, `min_text_size`, `required_layer`, `max_node_count`.
    pub rule_type: String,
    /// JSON object with rule-type-specific parameters.
    /// `palette_includes`: `{"color_hex": "#rrggbb"}`
    /// `max_colors`:       `{"count": N}`
    /// `min_text_size`:    `{"px": N}`
    /// `required_layer`:   `{"name": "..."}` or `{"prefix": "..."}`
    /// `max_node_count`:   `{"count": N}`
    pub params: serde_json::Value,
}

/// Arguments for `delete_grammar_rule` tool
#[derive(Debug, Deserialize, Default)]
pub struct DeleteGrammarRuleArgs {
    /// Name of the grammar rule to remove.
    pub name: String,
}

/// Arguments for `check_grammar` tool
#[derive(Debug, Deserialize, Default)]
pub struct CheckGrammarArgs {
    /// Optional subset of rule names to check. Defaults to all rules.
    #[serde(default)]
    pub rule_names: Vec<String>,
}

/// Arguments for `detect_rhythms` tool
#[derive(Debug, Deserialize, Default)]
pub struct DetectRhythmsArgs {
    /// Optional subset of node UUIDs/names to analyze. Defaults to all visible top-level nodes.
    #[serde(default)]
    pub node_ids: Vec<String>,
    /// Minimum number of nodes that must share a pattern for it to be reported (default 3).
    #[serde(default)]
    pub min_count: Option<usize>,
}

/// Arguments for `set_blend_spine` tool
#[derive(Debug, Deserialize, Default)]
pub struct SetBlendSpineArgs {
    /// UUID or name of the group node to configure as a blend.
    pub group_id: String,
    /// UUID or name of the path node (child of the group) to use as the blend spine.
    pub path_id: String,
}

/// Arguments for `clear_blend_spine` tool
#[derive(Debug, Deserialize, Default)]
pub struct ClearBlendSpineArgs {
    /// UUID or name of the group node whose blend spine should be cleared.
    pub group_id: String,
}

/// Arguments for `reverse_blend_spine` tool
#[derive(Debug, Deserialize, Default)]
pub struct ReverseBlendSpineArgs {
    /// UUID or name of the group node whose blend spine path should be reversed.
    pub group_id: String,
}

/// Arguments for `expand_blend` tool
#[derive(Debug, Deserialize, Default)]
pub struct ExpandBlendArgs {
    /// UUID or name of the blend group to expand into individual discrete objects.
    pub group_id: String,
}

/// Arguments for `save_workspace` tool
#[derive(Debug, Deserialize, Default)]
pub struct SaveWorkspaceArgs {
    /// Name for the workspace. Overwrites any existing workspace with the same name.
    pub name: String,
    /// Properties-panel search query to save (e.g. "text font" to show text panels).
    /// Pass empty string to save an "all panels" workspace.
    #[serde(default)]
    pub search_query: String,
}

/// Arguments for `load_workspace` tool
#[derive(Debug, Deserialize, Default)]
pub struct LoadWorkspaceArgs {
    /// Name of the workspace to load.
    pub name: String,
}

/// Arguments for `delete_workspace` tool
#[derive(Debug, Deserialize, Default)]
pub struct DeleteWorkspaceArgs {
    /// Name of the workspace to delete.
    pub name: String,
}

/// Arguments for `set_symbol_override` tool
#[derive(Debug, Deserialize, Default)]
pub struct SetSymbolOverrideArgs {
    /// UUID or name of the symbol instance node.
    pub node_id: String,
    /// Hex fill color override (e.g. "#ff0000"). Pass null to leave unchanged.
    #[serde(default)]
    pub fill_hex: Option<String>,
    /// Hex stroke color override (e.g. "#000000"). Pass null to leave unchanged.
    #[serde(default)]
    pub stroke_hex: Option<String>,
}

/// Arguments for `clear_symbol_overrides` tool
#[derive(Debug, Deserialize, Default)]
pub struct ClearSymbolOverridesArgs {
    /// UUID or name of the symbol instance node to reset to master defaults.
    pub node_id: String,
}

/// Arguments for `spray_symbol_instances` tool
#[derive(Debug, Deserialize, Default)]
pub struct SpraySymbolInstancesArgs {
    /// Name of the symbol to spray.
    pub symbol_name: String,
    /// Number of instances to place (1–200).
    pub count: usize,
    /// Center X coordinate of the spray area (canvas units).
    pub x: f64,
    /// Center Y coordinate of the spray area (canvas units).
    pub y: f64,
    /// Radius of the spray scatter area in canvas units. Default: 100.
    #[serde(default)]
    pub spread: f64,
}

/// Arguments for `load_symbol_library` tool
#[derive(Debug, Deserialize, Default)]
pub struct LoadSymbolLibraryArgs {
    /// Built-in library to load: "arrows", "shapes", or "ui".
    pub library_name: String,
}

/// Arguments for `set_tab_stops` tool
#[derive(Debug, Deserialize, Default)]
pub struct SetTabStopsArgs {
    /// UUID or name of the text node to update.
    pub node_id: String,
    /// Tab stop positions in document units (sorted ascending). Replaces all existing stops.
    pub stops: Vec<f64>,
}

/// Arguments for `clear_tab_stops` tool
#[derive(Debug, Deserialize, Default)]
pub struct ClearTabStopsArgs {
    /// UUID or name of the text node to reset to default tab stops.
    pub node_id: String,
}

// ─── SSE Events ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DocumentEvent {
    NodeAdded { node_id: Uuid, layer_id: Uuid },
    NodeUpdated { node_id: Uuid },
    NodeRemoved { node_id: Uuid },
    LayerAdded { layer_id: Uuid },
    LayerRemoved { layer_id: Uuid },
    DocumentChanged,
    RenderComplete { frame_ms: f32, node_count: usize },
}
