use serde::{Deserialize, Serialize};
use uuid::Uuid;
use super::*;


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

// ─── Pattern Args ─────────────────────────────────────────────────────────────

/// Arguments for `define_pattern` tool — adds a reusable tiled pattern to the
/// document pattern registry. Supply the tile via `path` (a file on disk) or
/// `data_base64` (inline image bytes).
#[derive(Debug, Deserialize)]
pub struct DefinePatternArgs {
    /// Unique pattern name. Overwrites an existing pattern with the same name.
    pub name: String,
    /// Path to a tile image file (PNG/JPEG/WebP/…). Mutually optional with `data_base64`.
    #[serde(default)]
    pub path: Option<String>,
    /// Base64-encoded tile image bytes. Mutually optional with `path`.
    #[serde(default)]
    pub data_base64: Option<String>,
    /// Tile layout: "grid" (default), "brick_by_row", "brick_by_column", or "hex".
    #[serde(default)]
    pub tile_type: Option<String>,
    /// Uniform pattern scale (default 1.0).
    #[serde(default)]
    pub scale: Option<f64>,
    /// Pattern rotation in degrees (default 0).
    #[serde(default)]
    pub rotation_degrees: Option<f64>,
    /// Document-space offset of the pattern origin (default [0, 0]).
    #[serde(default)]
    pub offset: Option<[f64; 2]>,
    /// Inter-tile gutter in tile pixels (default 0); gutter samples as transparent.
    #[serde(default)]
    pub spacing: Option<f64>,
}

/// Arguments for `apply_pattern_fill` tool — sets a registry pattern as the fill
/// of one or more path nodes. A clone of the pattern (with optional transform
/// overrides) is embedded on each node so it renders self-contained.
#[derive(Debug, Deserialize)]
pub struct ApplyPatternFillArgs {
    /// Node UUIDs or names to fill with the pattern.
    pub node_ids: Vec<String>,
    /// Name or UUID of the pattern in the document registry.
    pub pattern: String,
    /// Override the tile layout for this application.
    #[serde(default)]
    pub tile_type: Option<String>,
    /// Override the pattern scale for this application.
    #[serde(default)]
    pub scale: Option<f64>,
    /// Override the pattern rotation (degrees) for this application.
    #[serde(default)]
    pub rotation_degrees: Option<f64>,
    /// Override the pattern offset for this application.
    #[serde(default)]
    pub offset: Option<[f64; 2]>,
    /// Override the inter-tile spacing for this application.
    #[serde(default)]
    pub spacing: Option<f64>,
}

/// Arguments for `delete_pattern` tool
#[derive(Debug, Deserialize)]
pub struct DeletePatternArgs {
    /// Name of the pattern to delete from the registry.
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

// ─── Document Color Mode Args ────────────────────────────────────────────────

/// Arguments for `set_document_color_mode` tool
#[derive(Debug, Deserialize, Default)]
pub struct SetDocumentColorModeArgs {
    /// Color mode for the document. Accepted values: "rgb" or "cmyk".
    #[serde(default)]
    pub mode: Option<String>,
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

// ─── Artboard Args ───────────────────────────────────────────────────────────

/// Arguments for `list_artboards` (no parameters).
#[derive(Debug, Deserialize, Default)]
pub struct ListArtboardsArgs {}

/// Arguments for the `add_artboard` tool.
#[derive(Debug, Deserialize)]
pub struct AddArtboardArgs {
    /// Optional name. Defaults to "Artboard N".
    #[serde(default)]
    pub name: Option<String>,
    /// Top-left X in document units.
    pub x: f64,
    /// Top-left Y in document units.
    pub y: f64,
    /// Width in document units (must be > 0).
    pub width: f64,
    /// Height in document units (must be > 0).
    pub height: f64,
}

/// Arguments for the `update_artboard` tool. Only provided fields change.
#[derive(Debug, Deserialize)]
pub struct UpdateArtboardArgs {
    /// UUID of the artboard to edit (from `list_artboards`).
    pub artboard_id: Uuid,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub x: Option<f64>,
    #[serde(default)]
    pub y: Option<f64>,
    /// New width in document units (must be > 0 when provided).
    #[serde(default)]
    pub width: Option<f64>,
    /// New height in document units (must be > 0 when provided).
    #[serde(default)]
    pub height: Option<f64>,
}

/// Arguments for the `remove_artboard` tool.
#[derive(Debug, Deserialize)]
pub struct RemoveArtboardArgs {
    /// UUID of the artboard to remove (from `list_artboards`).
    pub artboard_id: Uuid,
}

/// Arguments for the `set_active_artboard` tool.
#[derive(Debug, Deserialize)]
pub struct SetActiveArtboardArgs {
    /// UUID of the artboard to make active (from `list_artboards`).
    pub artboard_id: Uuid,
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

// ─── Character Metrics Args ───────────────────────────────────────────────────

/// Arguments for `set_character_metrics` tool.
#[derive(Debug, Deserialize, Default)]
pub struct SetCharacterMetricsArgs {
    /// ID or name of the text node.
    pub node_id: String,
    /// Baseline shift in document units (positive raises the text above the
    /// baseline, negative lowers it). Pass null to leave unchanged.
    #[serde(default)]
    pub baseline_shift: Option<f64>,
    /// Script position: "normal", "superscript" (alias "super"), or "subscript"
    /// (alias "sub"). Pass null to leave unchanged.
    #[serde(default)]
    pub script_position: Option<String>,
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

