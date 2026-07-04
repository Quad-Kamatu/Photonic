use egui::{Color32, RichText, Ui};
use egui_phosphor::regular as ph;
use photonic_core::{
    layer::LayerId,
    node::NodeId,
    ops::boolean::BooleanOp,
    style::{
        FluidGradient, FluidGradientPoint, Gradient, GradientKind, GradientStop, LineJoin,
        MeshGradient, MeshGradientVertex, PatternFill, Stroke,
    },
    Color, Document, Fill, GaussianGlow, GlowEffect, PrimitiveKind, SceneNode, SceneNodeKind,
};
use uuid::Uuid;

use crate::color_edit::{srgb_color_edit, srgb_f32_color_edit};
use crate::radial_wheel::WheelAction;
use crate::tools::Tool;

mod arrange;
mod editors;
mod inspector;
mod modify;
mod layers_panel;
mod navigator;
mod toolbar;
mod tools_panel;
mod vertex_panel;

use arrange::*;
use inspector::*;
use modify::*;

pub(crate) use editors::{
    draw_fill_editor, draw_gaussian_glow_editor, draw_glow_editor, draw_stroke_editor,
};
pub use layers_panel::draw_layers_panel;
pub use toolbar::draw_toolbar;
pub use tools_panel::draw_tools_panel;
pub(crate) use vertex_panel::draw_vertex_panel;

// ─── Eyedropper types ─────────────────────────────────────────────────────────

/// Which color slot should receive the eyedropper result (node-agnostic).
#[derive(Debug, Clone)]
pub enum FillColorSlot {
    Solid,
    GradientStop(usize),
    FluidPoint(usize),
    MeshVertex(usize),
}

/// Full eyedropper target including node context.
#[derive(Debug, Clone)]
pub enum EyedropperTarget {
    NewShapeFill,
    NodeFillSolid {
        node_id: NodeId,
    },
    /// Sample one color and apply it as a solid fill to every listed node
    /// (multi-selection eyedropper). Recorded as a single undoable batch.
    NodesFillSolid {
        node_ids: Vec<NodeId>,
    },
    NodeFillGradStop {
        node_id: NodeId,
        idx: usize,
    },
    NodeFillFluid {
        node_id: NodeId,
        idx: usize,
    },
    NodeFillMesh {
        node_id: NodeId,
        idx: usize,
    },
    NodeStroke {
        node_id: NodeId,
    },
    /// Sample one color and apply it as the stroke color of every listed node
    /// (multi-selection eyedropper). Recorded as a single undoable batch;
    /// each node keeps its own stroke width/dash.
    NodesStroke {
        node_ids: Vec<NodeId>,
    },
    NodeOuterGlow {
        node_id: NodeId,
    },
    NodeInnerGlow {
        node_id: NodeId,
    },
    NodeGaussianGlow {
        node_id: NodeId,
    },
    /// Sample a color on a raster layer to begin a color-range mask-out session
    /// (hide every pixel within tolerance of the picked color).
    RasterColorRange {
        node_id: NodeId,
    },
    /// Recolor every object listed in `ids` (all currently sharing the `from`
    /// color) to the sampled color, as one undoable step. Driven by the
    /// "Fill colors in document" swatch picker's eyedropper button.
    RecolorSwatch {
        ids: Vec<NodeId>,
        from: [f32; 4],
    },
}

/// An action requested by a panel widget, to be processed by the main draw loop.
#[derive(Debug)]
pub enum PanelAction {
    /// Reorder a node in z-order.
    ReorderNode { node_id: NodeId, op: ZOrderOp },
    /// Select a single node (e.g. clicked in the Layers tree).
    SelectNode { node_id: NodeId },
    /// Run a boolean operation on the two currently selected nodes.
    BooleanOp(BooleanOp),
    /// Restore the document to a named checkpoint.
    ///
    /// The right-side Change Log UI that produced this action was removed
    /// (#173) as redundant with the left-drawer Edit History; the restore
    /// handler is retained for a future menu relocation of checkpoint restore.
    #[allow(dead_code)]
    RestoreCheckpoint(Uuid),
    /// Update the fill of a selected node.
    UpdateNodeFill { node_id: NodeId, fill: Fill },
    /// Update the stroke of a selected node.
    UpdateNodeStroke { node_id: NodeId, stroke: Stroke },
    /// Apply the same fill to every listed node at once (multi-selection edit).
    /// Recorded as a single undoable batch.
    UpdateNodesFill { node_ids: Vec<NodeId>, fill: Fill },
    /// Apply the same stroke to every listed node at once (multi-selection edit).
    /// Recorded as a single undoable batch.
    UpdateNodesStroke { node_ids: Vec<NodeId>, stroke: Stroke },
    /// Convert each listed path's stroke into a filled outline shape (Illustrator
    /// "Outline Stroke"). Paths without an enabled, positive-width stroke are
    /// skipped. Recorded as a single undoable batch.
    OutlineStroke { node_ids: Vec<NodeId> },
    /// Deep-clone a node and insert the copy at a small offset.
    DuplicateNode { node_id: NodeId },
    /// Remove a specific node by ID.
    DeleteNode { node_id: NodeId },
    /// Remove all currently selected nodes.
    DeleteSelected,
    /// Create a default-sized shape at a canvas position.
    CreateShapeAtPos {
        shape: ShapeKind,
        canvas_x: f64,
        canvas_y: f64,
        fill: [f32; 4],
    },
    /// Group the currently selected nodes (requires 2+ in selection).
    GroupSelected,
    /// Export the given nodes (or the current selection when empty) as SVG and
    /// write the result to the OS clipboard.
    CopyAsSvg { node_ids: Vec<NodeId> },
    /// Diff the current document state against a saved checkpoint, populating
    /// the canvas diff-highlight overlay.
    ///
    /// The right-side Change Log UI that produced this action was removed
    /// (#173) as redundant with the left-drawer Edit History; the diff handler
    /// is retained for a future menu relocation of checkpoint diff.
    #[allow(dead_code)]
    DiffWithCheckpoint { checkpoint_id: Uuid },
    /// Clear the active diff highlight overlay.
    ClearDiff,
    /// Insert a midpoint anchor on every segment of a path node.
    AddAnchorPoints { node_id: NodeId },
    /// Open the Simplify Path dialog for a path node.
    OpenSimplifyDialog { node_id: NodeId },
    /// Open the Merge Vertices by Distance (weld) dialog for a path node.
    OpenMergeVerticesDialog { node_id: NodeId },
    /// Invert the fill/stroke colors of the given nodes. Empty vec = use selection.
    InvertColors { node_ids: Vec<NodeId> },
    /// Convert fill/stroke colors to grayscale. Empty vec = use selection.
    ConvertToGrayscale { node_ids: Vec<NodeId> },
    /// Open the Find / Replace Text dialog (document-wide, no node target needed).
    OpenFindReplaceTextDialog,
    /// Dissolve a group node, re-inserting its children in place.
    UngroupNode { node_id: NodeId },
    /// Close every open subpath in a path node (single-node) or merge two path
    /// nodes into one by connecting their nearest endpoints (two-node).
    JoinPaths { node_ids: Vec<NodeId> },
    /// Clip all selected nodes to the frontmost node's boundary (Pathfinder Crop).
    /// Empty vec = use current selection.
    PathfinderCrop { node_ids: Vec<NodeId> },
    /// Subtract all back nodes from the frontmost node (Pathfinder Minus Back).
    /// Empty vec = use current selection.
    PathfinderMinusBack { node_ids: Vec<NodeId> },
    /// Subtract the frontmost node from every back node (Pathfinder Minus Front).
    /// Empty vec = use current selection.
    PathfinderMinusFront { node_ids: Vec<NodeId> },
    /// Trim hidden areas from every selected node (Pathfinder Trim).
    /// Empty vec = use current selection.
    PathfinderTrim { node_ids: Vec<NodeId> },
    /// Convert fills to stroked outlines on selected nodes (Pathfinder Outline).
    /// Empty vec = use current selection.
    PathfinderOutline { node_ids: Vec<NodeId> },
    /// Select all document nodes sharing the given attribute with the reference node.
    SelectSame {
        node_id: NodeId,
        attribute: SelectSameAttr,
    },
    /// Reverse the winding direction of a path node.
    ReversePathDirection { node_id: NodeId },
    /// Average all anchor points of a path node to their centroid.
    AverageAnchorPoints { node_id: NodeId },
    /// Update the outer glow of a node.
    UpdateNodeOuterGlow { node_id: NodeId, glow: GlowEffect },
    /// Update the inner glow of a node.
    UpdateNodeInnerGlow { node_id: NodeId, glow: GlowEffect },
    /// Update the Gaussian glow of a node.
    UpdateNodeGaussianGlow { node_id: NodeId, glow: GaussianGlow },
    /// Lock or unlock a node (prevents canvas selection when locked).
    SetLocked { node_id: NodeId, locked: bool },
    /// Show or hide an individual node (does not affect layer visibility).
    SetVisible { node_id: NodeId, visible: bool },
    /// Move a node to an absolute canvas position by setting its translation directly.
    SetNodePosition { node_id: NodeId, x: f64, y: f64 },
    /// Resize a node to the given world-space width and height. A scale transform is
    /// composed onto the existing transform so the top-left anchor stays fixed.
    SetNodeSize {
        node_id: NodeId,
        width: f64,
        height: f64,
    },
    /// Rotate a node to an absolute angle (degrees). The rotation is applied around
    /// the node's world-space bounding-box center. Delta from the current angle is
    /// composed onto the existing transform.
    /// Rotate the selection to `angle_deg`. `node_ids[0]` is the primary whose
    /// current angle defines the delta; all are rotated about the shared center.
    RotateNode {
        node_ids: Vec<NodeId>,
        angle_deg: f64,
    },
    /// Split two overlapping path nodes at their intersection edges into distinct face nodes.
    /// Empty vec = use current selection (resolved in app.rs).
    PathfinderDivide { node_ids: Vec<NodeId> },
    /// Trim all selected nodes of hidden areas then merge same-color faces (Pathfinder Merge).
    /// Empty vec = use current selection (resolved in app.rs).
    PathfinderMerge { node_ids: Vec<NodeId> },
    /// Use the given path node as a cutting edge to divide all objects beneath it.
    /// The cutter is removed; each intersecting object below is split into inside/outside faces.
    DivideObjectsBelow { node_id: NodeId },
    /// Activate the eyedropper for the given color slot.
    StartEyedropper(EyedropperTarget),
    /// Remove the background of a raster layer with the local matting model,
    /// storing the result as a non-destructive foreground layer mask.
    RasterRemoveBackground { node_id: NodeId },
    /// Begin a color-range mask-out session on a raster layer: arms the
    /// eyedropper so the next canvas click samples the color to hide.
    StartRasterColorRange { node_id: NodeId },
    /// Live-update the active color-range session's fuzziness/contiguity and
    /// refresh its preview.
    SetRasterColorRangeParams { tolerance: f32, contiguous: bool },
    /// Commit the active color-range session as one undoable step.
    ApplyRasterColorRange,
    /// Discard the active color-range session, restoring the layer.
    CancelRasterColorRange,
    /// Remove a raster layer's non-destructive layer mask (undoable).
    ClearRasterMask { node_id: NodeId },
    /// Crop a raster layer's pixels (and mask) to the artboard bounds,
    /// discarding everything outside (undoable).
    CropRasterToArtboard { node_id: NodeId },
    /// Move the given nodes (or current selection if empty) into a new layer.
    CollectInNewLayer { node_ids: Vec<NodeId> },
    /// Blend fill colors linearly across 3+ selected path nodes.
    /// Empty vec = use current selection (resolved in app.rs).
    /// `direction`: "horizontal", "vertical", "depth", or "" for selection order.
    BlendColors {
        node_ids: Vec<NodeId>,
        direction: String,
    },
    /// Shift RGBA channel values on selected path nodes.
    AdjustColors {
        node_ids: Vec<NodeId>,
        delta_r: f32,
        delta_g: f32,
        delta_b: f32,
        delta_a: f32,
    },
    /// Move each node (or current selection if empty) into its own new layer.
    ReleaseToLayers { node_ids: Vec<NodeId> },
    /// Merge the given layers into one (bottommost in stack order is the target).
    MergeLayers { layer_ids: Vec<LayerId> },
    /// Flatten all layers in the document into a single layer.
    FlattenArtwork,
    /// Set the color tag of a layer (None = clear).
    SetLayerColor {
        layer_id: LayerId,
        color: Option<[f32; 4]>,
    },
    /// Toggle template mode on a layer.
    SetLayerTemplate {
        layer_id: LayerId,
        is_template: bool,
    },
    /// Rename a layer.
    RenameLayer { layer_id: LayerId, name: String },
    /// Align or distribute selected nodes.
    /// `operation`: left/center_horizontal/right/top/center_vertical/bottom/distribute_horizontal/distribute_vertical
    /// `key_object_id`: when set, align relative to this node's bounds (key object is not moved)
    AlignNodes {
        operation: String,
        key_object_id: Option<NodeId>,
    },
    /// Combine multiple path nodes into one compound path (even-odd fill rule creates holes).
    MakeCompoundPath { node_ids: Vec<NodeId> },
    /// Release a compound path back into individual path nodes, one per subpath.
    ReleaseCompoundPath { node_id: NodeId },
    /// Apply a shear (skew) transform to a node around its own centre.
    ShearNode {
        node_ids: Vec<NodeId>,
        shear_x: f64,
        shear_y: f64,
    },
    /// Round the position of one or more nodes to integer pixel coordinates.
    SnapToPixel { node_ids: Vec<NodeId> },
    /// Distribute node copies evenly along a guide path.
    /// `path_node_id` is the guide; `node_ids` are the sources to clone.
    DistributeOnPath {
        path_node_id: NodeId,
        node_ids: Vec<NodeId>,
        align: bool,
    },
    /// Remap every solid fill in the given nodes to the nearest color in the palette.
    RecolorArtwork {
        node_ids: Vec<NodeId>,
        palette: Vec<[f32; 4]>,
    },
    /// Live preview: set the solid fill of exactly these nodes to `to` WITHOUT
    /// recording history. Used while dragging the document color-swatch picker.
    RecolorPreview { ids: Vec<NodeId>, to: [f32; 4] },
    /// Commit a document color-swatch recolor as one undoable step: the given
    /// nodes change from `from` to `to` (undo restores `from`).
    RecolorCommit {
        ids: Vec<NodeId>,
        from: [f32; 4],
        to: [f32; 4],
    },
    /// Align selected nodes relative to the document canvas (artboard) bounds.
    AlignToArtboard { operation: String },
    /// Remove all unlocked guides from the document.
    ClearGuides,
    /// Convert anchor points to smooth joins for the given path nodes.
    ConvertToSmooth { node_ids: Vec<NodeId> },
    /// Convert anchor points to corner joins (cusps) for the given path nodes.
    ConvertToCorner { node_ids: Vec<NodeId> },
    /// Select all nodes of a given kind ("path", "text", "group", "same_layer").
    SelectByKind { kind: String, additive: bool },
    /// Apply zig-zag distortion to path nodes.
    ZigZagPath {
        node_ids: Vec<NodeId>,
        size: f64,
        ridges: usize,
        smooth: bool,
    },
    /// Apply pucker (contract inward) or bloat (expand outward) distortion.
    PuckerBloat {
        node_ids: Vec<NodeId>,
        strength: f64,
    },
    /// Roughen a path by randomly displacing points.
    RoughenPath {
        node_ids: Vec<NodeId>,
        size: f64,
        detail: usize,
        seed: u64,
    },
    /// Twirl a path — spiral rotation around centroid.
    TwirlPath {
        node_ids: Vec<NodeId>,
        angle_deg: f64,
    },
    /// Blend between two paths — create interpolated intermediate steps.
    BlendObjects {
        node_id_a: NodeId,
        node_id_b: NodeId,
        steps: usize,
    },
    /// Blend using Smooth Color mode — auto-compute steps from color distance.
    BlendObjectsSmoothColor {
        node_id_a: NodeId,
        node_id_b: NodeId,
    },
    /// Blend using Specified Distance mode — space steps by pixel distance.
    BlendObjectsSpacing {
        node_id_a: NodeId,
        node_id_b: NodeId,
        spacing: f64,
    },
    /// Apply scallop arcs to path segments.
    ScallopPath {
        node_ids: Vec<NodeId>,
        depth: f64,
        count: usize,
    },
    /// Apply crystallize spikes to path segments.
    CrystallizePath {
        node_ids: Vec<NodeId>,
        size: f64,
        count: usize,
    },
    /// Apply a named warp envelope distortion.
    WarpEnvelope {
        node_ids: Vec<NodeId>,
        warp_type: String,
        bend: f64,
    },
    /// Round sharp corners with arc fillets.
    RoundCorners { node_ids: Vec<NodeId>, radius: f64 },
    /// Move a single selected anchor of an edited path to a local position.
    SetAnchorPosition {
        node_id: NodeId,
        index: usize,
        x: f64,
        y: f64,
    },
    /// Round the selected straight corners of an edited path to `radius`.
    RoundSelectedCorners {
        node_id: NodeId,
        indices: Vec<usize>,
        radius: f64,
    },
    /// Convert the selected anchors of an edited path to smooth or corner.
    ConvertAnchorType {
        node_id: NodeId,
        indices: Vec<usize>,
        smooth: bool,
    },
    /// Delete the selected anchors of an edited path.
    DeleteAnchors {
        node_id: NodeId,
        indices: Vec<usize>,
    },
    /// Flip node(s) horizontally or vertically.
    FlipNodes {
        node_ids: Vec<NodeId>,
        horizontal: bool,
    },
    /// Set text typography properties (line height, letter spacing).
    SetTextTypography {
        node_id: NodeId,
        line_height: Option<f64>,
        letter_spacing: Option<f64>,
    },
    /// Add a drop shadow behind a node.
    AddDropShadow { node_id: NodeId },
    /// Create a sample radar chart at canvas center (5 axes, 2 series).
    CreateRadarChart,
    /// Create a sample stacked column chart at canvas center.
    CreateStackedBarChart,
    /// Create a parametric shape (Lissajous, Superellipse, Rose, etc.) at canvas center.
    CreateParametricShape { shape_type: String },
    /// Offset (expand or inset) a path node by a fixed distance, creating a copy.
    OffsetPath {
        node_ids: Vec<NodeId>,
        distance: f64,
    },
    /// Generate a Truchet tiling at canvas center.
    CreateTruchetTiling { style: String },
    /// Push selected nodes apart until their bounding boxes no longer overlap.
    DistributeNoOverlap { node_ids: Vec<NodeId> },
    /// Apply sinusoidal noise deformation to path anchor points.
    NoiseDeform {
        node_ids: Vec<NodeId>,
        amplitude: f64,
        style: String,
    },
    /// Duplicate and flip selected nodes to create mirrored copies.
    MirrorCopy { node_ids: Vec<NodeId>, axis: String },
    /// Create N evenly-spaced rotational copies around the node's center.
    RotateCopies { node_id: NodeId, count: usize },
    /// Copy appearance (fill/stroke/opacity) from source to target nodes.
    CopyAppearance {
        source_id: NodeId,
        target_ids: Vec<NodeId>,
        copy_fill: bool,
        copy_stroke: bool,
        copy_opacity: bool,
    },
    /// Remove a named export profile from the document.
    RemoveExportProfile { name: String },
    /// Pin guides at node edges/centers.
    PinObjectGuides { node_ids: Vec<NodeId> },
    /// Reverse the children order in selected group nodes.
    ReverseNodeOrder { node_ids: Vec<NodeId> },
    /// Copy document template JSON to the OS clipboard.
    CopyDocumentTemplate,
    /// Select all nodes whose fill color matches selected nodes.
    SelectSimilar {
        node_ids: Vec<NodeId>,
        match_by: String,
    },
    /// Tag a node for asset export with the given spec.
    TagNodeForExport {
        node_id: NodeId,
        name: String,
        format: String,
    },
    /// Remove the asset export tag from a node.
    RemoveExportTag { node_id: NodeId },
    /// Apply a named character style to a text node.
    ApplyCharacterStyle { node_id: NodeId, style_name: String },
    /// Delete a named character style from the document.
    DeleteCharacterStyle { name: String },
    /// Apply a named paragraph style to a text node.
    ApplyParagraphStyle { node_id: NodeId, style_name: String },
    /// Delete a named paragraph style from the document.
    DeleteParagraphStyle { name: String },
    /// Apply a named color swatch to a node's fill.
    ApplyColorSwatch {
        node_id: NodeId,
        swatch_name: String,
    },
    /// Delete a named color swatch from the document palette.
    DeleteColorSwatch { name: String },
    /// Load a predefined swatch library into the document palette.
    LoadSwatchLibrary {
        library: String,
        clear_existing: bool,
    },
    /// #207: import named color swatches from a design-tokens file (opens a picker).
    ImportDesignTokens,
    /// Apply a named width profile to the selected path node.
    ApplyWidthProfile {
        node_id: NodeId,
        profile_name: String,
    },
    /// Save a width profile from the selected node's current stroke width.
    SaveWidthProfile { stroke_width: f64, name: String },
    /// Delete a named width profile.
    DeleteWidthProfile { name: String },
    /// Rename an existing width profile (e.g. one shaped with the Width tool).
    RenameWidthProfile { old_name: String, new_name: String },
    /// Save a graphic style from the selected node.
    SaveGraphicStyle { node_id: NodeId, name: String },
    /// Apply a named graphic style to the selected node.
    ApplyGraphicStyle { node_id: NodeId, style_name: String },
    /// Delete a named graphic style.
    DeleteGraphicStyle { name: String },
    /// Save the gradient fill of a node as a named gradient swatch.
    SaveGradientSwatch { node_id: NodeId, name: String },
    /// Apply a named gradient swatch to a node's fill.
    ApplyGradientSwatch {
        node_id: NodeId,
        swatch_name: String,
    },
    /// Delete a named gradient swatch.
    DeleteGradientSwatch { name: String },
    /// Run composition analysis and store findings in app state.
    AnalyzeComposition,
    /// Detect rhythm patterns and store findings in app state.
    DetectRhythms,
    /// Define a named document grammar rule.
    DefineGrammarRule {
        name: String,
        rule_type: String,
        params_json: String,
    },
    /// Delete a named document grammar rule.
    DeleteGrammarRule { name: String },
    /// Check all grammar rules and store results in app state.
    CheckGrammar,
    /// Measure distances between selected nodes and store results.
    MeasureDistances { node_ids: Vec<NodeId> },
    /// Play a named action set.
    PlayAction { name: String },
    /// Delete a named action set.
    DeleteAction { name: String },
    /// Register an event trigger.
    RegisterEventTrigger { event: String, action_name: String },
    /// Remove an event trigger.
    RemoveEventTrigger {
        event: String,
        action_name: Option<String>,
    },
    /// Flatten transparency — bake opacity into color alpha values.
    FlattenTransparency,
    /// Apply flex layout to a group's children.
    ApplyFlexLayout {
        group_id: NodeId,
        direction: String,
        gap: f64,
        align: String,
        padding: f64,
    },
    /// Revert a node N edits back in history.
    UndoNode { node_id: NodeId, steps: usize },
    /// Apply grid layout to a group's children.
    ApplyGridLayout {
        group_id: NodeId,
        columns: usize,
        gap_x: f64,
        gap_y: f64,
    },
    /// Stack all children of a group at the same position.
    ApplyStackLayout {
        group_id: NodeId,
        align_h: String,
        align_v: String,
    },
    /// Refresh the displayed history entries (read-only trigger).
    RefreshHistory,
    /// Jump to a specific undo history index.
    JumpToHistory { index: usize },
    /// Scale and position selected (or all) nodes to fill the artboard safe area.
    FitToMargins,
    /// Add a dimension annotation between two nodes.
    AddDimension {
        from_id: photonic_core::node::NodeId,
        to_id: photonic_core::node::NodeId,
        axis: String,
    },
    /// Remove a dimension annotation by ID.
    RemoveDimension { id: uuid::Uuid },
    /// Set document bleed and slug values.
    SetDocumentBleed { bleed_mm: f64, slug_mm: f64 },
    /// Add an angled construction line.
    AddConstructionLine { x: f64, y: f64, angle_degrees: f64 },
    /// Set artboard safe-area margins.
    SetArtboardMargins {
        top: f64,
        right: f64,
        bottom: f64,
        left: f64,
    },
    /// Define or update a spot color.
    DefineSpotColor {
        name: String,
        hex: String,
        overprint: bool,
    },
    /// Apply a spot color to a node.
    ApplySpotColor { node_id: NodeId, color_name: String },
    /// Delete a spot color.
    DeleteSpotColor { name: String },
    /// Save the current document as a named branch.
    BranchCreate { name: String },
    /// Switch to a named branch (replace document).
    BranchSwitch { name: String },
    /// Delete a named branch.
    BranchDelete { name: String },
    /// Make a clipping mask from a group node (topmost child becomes clip path).
    MakeClippingMask { group_id: NodeId },
    /// Release the clipping mask from a group node.
    ReleaseClippingMask { group_id: NodeId },
    /// Place a text node along a path spine.
    SetTextPath {
        text_node_id: NodeId,
        path_node_id: NodeId,
        offset: f64,
    },
    /// Remove the path spine from a text node.
    ClearTextPath { text_node_id: NodeId },
    /// Set the layout direction of a text node (horizontal/vertical).
    SetTextDirection { node_id: NodeId, vertical: bool },
    /// Set the font style (italic/oblique/normal) on a text node.
    SetFontStyle { node_id: NodeId, style: String },
    /// Set the font weight (100–900) on a text node.
    SetFontWeight { node_id: NodeId, weight: u16 },
    /// Flow a text node inside a closed path area.
    SetTextArea {
        text_node_id: NodeId,
        area_path_id: NodeId,
    },
    /// Remove the area boundary from a text node.
    ClearTextArea { text_node_id: NodeId },
    /// Set text decoration (underline/line-through/overline/none) on a text node.
    SetTextDecoration { node_id: NodeId, decoration: String },
    /// Set paragraph spacing and indent on a text node.
    SetParagraphOptions {
        node_id: NodeId,
        spacing_before: f64,
        spacing_after: f64,
        indent: f64,
    },
    /// Set custom tab stop positions on a text node.
    SetTabStops { node_id: NodeId, stops: Vec<f64> },
    /// Clear custom tab stops from a text node.
    ClearTabStops { node_id: NodeId },
    /// Set OpenType features on a text node.
    SetOpenTypeFeatures {
        node_id: NodeId,
        features: Vec<String>,
    },
    /// Set advanced character metrics (baseline shift + super/subscript) on a text node.
    SetCharacterMetrics {
        node_id: NodeId,
        baseline_shift: f64,
        script_position: photonic_core::node::ScriptPosition,
    },
    /// Link two text nodes as a threaded text chain.
    LinkTextFrames { from_id: NodeId, to_id: NodeId },
    /// Remove a text node from its thread chain.
    UnlinkTextFrames { node_id: NodeId },
    /// Bind a text node to a document variable.
    BindTextVariable {
        node_id: NodeId,
        variable_name: String,
    },
    /// Remove the variable binding from a text node.
    UnbindTextVariable { node_id: NodeId },
    /// Apply all document variable values to bound text nodes.
    ApplyVariables,
    /// Delete a document variable.
    DeleteVariable { name: String },
    /// Define a node as a named symbol master.
    DefineSymbol { node_id: NodeId, name: String },
    /// Place an instance of a named symbol at a position.
    PlaceSymbol { symbol_name: String },
    /// Break the symbol link on an instance node.
    BreakLinkToSymbol { node_id: NodeId },
    /// Delete a symbol from the registry.
    DeleteSymbol { name: String },
    /// Assign a path node as the blend spine for a group.
    SetBlendSpine { group_id: NodeId, path_id: NodeId },
    /// Clear the blend spine assignment from a group.
    ClearBlendSpine { group_id: NodeId },
    /// Reverse the direction of the blend spine path in a group.
    ReverseBlendSpine { group_id: NodeId },
    /// Expand a blend group into individual discrete objects.
    ExpandBlend { group_id: NodeId },
    /// Load a built-in symbol library into the document.
    LoadSymbolLibrary { library_name: String },
    /// Spray N symbol instances scattered around a center point.
    SpraySymbolInstances {
        symbol_name: String,
        count: usize,
        x: f64,
        y: f64,
        spread: f64,
    },
    /// Set per-instance color overrides on a symbol instance.
    SetSymbolOverride {
        node_id: NodeId,
        fill_hex: Option<String>,
        stroke_hex: Option<String>,
    },
    /// Clear all per-instance color overrides from a symbol instance.
    ClearSymbolOverrides { node_id: NodeId },
    /// Save the current prop_search as a named workspace.
    SaveWorkspace { name: String, search_query: String },
    /// Load a workspace by name (returns search_query to apply).
    LoadWorkspace { name: String },
    /// Delete a named workspace.
    DeleteWorkspace { name: String },
    /// Recenter the canvas viewport on a canvas-space point (Navigator click).
    CenterViewOn { canvas_x: f64, canvas_y: f64 },
}

/// Discriminant for which shape the radial wheel should create.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShapeKind {
    Shape(PrimitiveKind),
    Text,
}

impl ShapeKind {
    pub fn label(self) -> &'static str {
        match self {
            ShapeKind::Shape(PrimitiveKind::Rectangle) => "Rectangle",
            ShapeKind::Shape(PrimitiveKind::RoundedRect) => "Rounded Rect",
            ShapeKind::Shape(PrimitiveKind::Ellipse) => "Ellipse",
            ShapeKind::Shape(PrimitiveKind::Polygon) => "Polygon",
            ShapeKind::Shape(PrimitiveKind::Star) => "Star",
            ShapeKind::Shape(PrimitiveKind::Line) => "Line",
            ShapeKind::Shape(PrimitiveKind::Arc) => "Arc",
            ShapeKind::Text => "Text",
        }
    }
}

impl PanelAction {
    /// Translate a `WheelAction` into the appropriate `PanelAction`.
    /// `canvas_pos` is where the wheel was opened; `fill` is the current fill color.
    pub fn from_wheel_action(wa: WheelAction, canvas_pos: (f64, f64), fill: [f32; 4]) -> Self {
        let (cx, cy) = canvas_pos;
        match wa {
            WheelAction::CreateRect => Self::CreateShapeAtPos {
                shape: ShapeKind::Shape(PrimitiveKind::Rectangle),
                canvas_x: cx,
                canvas_y: cy,
                fill,
            },
            WheelAction::CreateRoundedRect => Self::CreateShapeAtPos {
                shape: ShapeKind::Shape(PrimitiveKind::RoundedRect),
                canvas_x: cx,
                canvas_y: cy,
                fill,
            },
            WheelAction::CreateEllipse => Self::CreateShapeAtPos {
                shape: ShapeKind::Shape(PrimitiveKind::Ellipse),
                canvas_x: cx,
                canvas_y: cy,
                fill,
            },
            WheelAction::CreatePolygon => Self::CreateShapeAtPos {
                shape: ShapeKind::Shape(PrimitiveKind::Polygon),
                canvas_x: cx,
                canvas_y: cy,
                fill,
            },
            WheelAction::CreateStar => Self::CreateShapeAtPos {
                shape: ShapeKind::Shape(PrimitiveKind::Star),
                canvas_x: cx,
                canvas_y: cy,
                fill,
            },
            WheelAction::CreateText => Self::CreateShapeAtPos {
                shape: ShapeKind::Text,
                canvas_x: cx,
                canvas_y: cy,
                fill,
            },
            WheelAction::DuplicateNode(id) => Self::DuplicateNode { node_id: id },
            WheelAction::DeleteNode(id) => Self::DeleteNode { node_id: id },
            WheelAction::BringForward(id) => Self::ReorderNode {
                node_id: id,
                op: ZOrderOp::BringForward,
            },
            WheelAction::SendBackward(id) => Self::ReorderNode {
                node_id: id,
                op: ZOrderOp::SendBackward,
            },
            WheelAction::BringToFront(id) => Self::ReorderNode {
                node_id: id,
                op: ZOrderOp::BringToFront,
            },
            WheelAction::SendToBack(id) => Self::ReorderNode {
                node_id: id,
                op: ZOrderOp::SendToBack,
            },
            WheelAction::GroupSelected => Self::GroupSelected,
            WheelAction::DeleteSelected => Self::DeleteSelected,
            WheelAction::BoolUnion => Self::BooleanOp(BooleanOp::Union),
            WheelAction::BoolSubtract => Self::BooleanOp(BooleanOp::Subtract),
            WheelAction::BoolIntersect => Self::BooleanOp(BooleanOp::Intersect),
            WheelAction::BoolExclude => Self::BooleanOp(BooleanOp::Exclude),
            WheelAction::CopyAsSvg(id) => Self::CopyAsSvg { node_ids: vec![id] },
            // Empty vec signals "use the current selection" — resolved in app.rs.
            WheelAction::CopyAsSvgSelection => Self::CopyAsSvg { node_ids: vec![] },
            WheelAction::AddAnchorPoints(id) => Self::AddAnchorPoints { node_id: id },
            WheelAction::SimplifyPath(id) => Self::OpenSimplifyDialog { node_id: id },
            WheelAction::MergeVertices(id) => Self::OpenMergeVerticesDialog { node_id: id },
            WheelAction::OutlineStroke(id) => Self::OutlineStroke {
                node_ids: vec![id],
            },
            WheelAction::ReversePathDirection(id) => Self::ReversePathDirection { node_id: id },
            WheelAction::AverageAnchorPoints(id) => Self::AverageAnchorPoints { node_id: id },
            WheelAction::ClosePath(id) => Self::JoinPaths { node_ids: vec![id] },
            WheelAction::InvertColors(id) => Self::InvertColors { node_ids: vec![id] },
            WheelAction::InvertColorsSelected => Self::InvertColors { node_ids: vec![] },
            WheelAction::ConvertToGrayscale(id) => Self::ConvertToGrayscale { node_ids: vec![id] },
            WheelAction::ConvertToGrayscaleSelected => {
                Self::ConvertToGrayscale { node_ids: vec![] }
            }
            WheelAction::UngroupNode(id) => Self::UngroupNode { node_id: id },
        }
    }
}

/// Which attribute to match in Select Same operations (mirrors MCP SelectSameAttribute).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectSameAttr {
    FillColor,
    StrokeColor,
    StrokeWeight,
    Opacity,
    BlendMode,
    ObjectType,
}

#[derive(Debug)]
pub enum ZOrderOp {
    SendToBack,
    BringToFront,
    SendBackward,
    BringForward,
}




/// Draw the right properties panel.
/// Returns an optional action if the user clicked a boolean operation button.
/// In-progress edit state for the document color-swatch recolor picker.
/// Stored in egui temp memory so the inline picker survives across frames
/// without threading another `&mut` parameter through `draw_properties_panel`.
#[derive(Clone)]
pub(crate) struct RecolorSwatchEdit {
    /// Nodes captured at click time whose fill matched the clicked swatch.
    /// Preview and commit operate on exactly these, so picking a color that
    /// collides with another group never recolors the wrong objects.
    ids: Vec<NodeId>,
    /// The original color clicked — undo target / revert color.
    original: [f32; 4],
    /// The color currently shown in the document (last preview applied).
    applied: [f32; 4],
    /// The live color being edited in the picker.
    current: [f32; 4],
}

pub(crate) struct PropPanelCtx<'a> {
    pub(crate) doc: &'a Document,
    pub(crate) active_tool: Tool,
    pub(crate) fill_color: &'a mut [f32; 4],
    pub(crate) polygon_sides: &'a mut u32,
    pub(crate) star_points: &'a mut u32,
    pub(crate) star_inner_ratio: &'a mut f32,
    pub(crate) rounded_rect_radius: &'a mut f64,
    pub(crate) spiral_turns: &'a mut f32,
    pub(crate) spiral_inner_radius: &'a mut f32,
    pub(crate) spiral_segs_per_turn: &'a mut u32,
    pub(crate) selected_node: Option<&'a SceneNode>,
    pub(crate) selected_id: Option<NodeId>,
    pub(crate) selection_count: usize,
    pub(crate) selected_ids: &'a [NodeId],
    pub(crate) point_edit_node: Option<NodeId>,
    pub(crate) point_selected: &'a [usize],
    pub(crate) prop_search: &'a mut String,
    pub(crate) shear_x: &'a mut f64,
    pub(crate) shear_y: &'a mut f64,
    pub(crate) line_snap_45: &'a mut bool,
    pub(crate) color_guide_rule: &'a mut String,
    pub(crate) arc_start_angle: &'a mut f64,
    pub(crate) arc_end_angle: &'a mut f64,
    pub(crate) arc_open: &'a mut bool,
    pub(crate) grid_cols: &'a mut u32,
    pub(crate) grid_rows: &'a mut u32,
    pub(crate) polar_grid_rings: &'a mut u32,
    pub(crate) polar_grid_sectors: &'a mut u32,
    pub(crate) polar_grid_inner_ratio: &'a mut f32,
    pub(crate) recolor_palette_input: &'a mut String,
    pub(crate) magic_wand_attribute: &'a mut SelectSameAttr,
    pub(crate) magic_wand_tolerance: &'a mut f64,
    pub(crate) eraser_radius: &'a mut f64,
    /// Raster color-range mask-out: fuzziness (0..1) and contiguous (wand) flag.
    pub(crate) raster_mask_tolerance: &'a mut f32,
    pub(crate) raster_mask_contiguous: &'a mut bool,
    /// `Some(rgba)` while a color-range session is live on the selected raster
    /// layer (drives the swatch + Apply/Cancel controls). `None` otherwise.
    pub(crate) raster_color_range_target: Option<[u8; 4]>,
    /// Whether the background-removal model is already downloaded (drives the
    /// first-run "~4.7 MB download" hint on the button).
    pub(crate) rmbg_model_cached: bool,
    pub(crate) composition_findings: &'a [String],
    pub(crate) rhythm_findings: &'a [String],
    pub(crate) branch_names: &'a [String],
    pub(crate) branch_name_input: &'a mut String,
    pub(crate) swatch_library_selected: &'a mut String,
    pub(crate) graphic_style_name_input: &'a mut String,
    pub(crate) width_profile_name_input: &'a mut String,
    pub(crate) grammar_rules: &'a [(String, String)],
    pub(crate) grammar_rule_name_input: &'a mut String,
    pub(crate) grammar_rule_type_selected: &'a mut String,
    pub(crate) grammar_rule_params_input: &'a mut String,
    pub(crate) grammar_check_results: &'a [(String, bool, String)],
    pub(crate) distance_results: &'a [(String, String, f64, f64, f64)],
    pub(crate) action_names: &'a [(String, usize)],
    pub(crate) history_entries: &'a [(usize, String)],
    pub(crate) history_total: usize,
    pub(crate) bleed_mm_input: &'a mut f64,
    pub(crate) slug_mm_input: &'a mut f64,
    pub(crate) construction_angle: &'a mut f64,
    pub(crate) construction_x: &'a mut f64,
    pub(crate) construction_y: &'a mut f64,
    pub(crate) margin_top: &'a mut f64,
    pub(crate) margin_right: &'a mut f64,
    pub(crate) margin_bottom: &'a mut f64,
    pub(crate) margin_left: &'a mut f64,
    pub(crate) event_trigger_event: &'a mut String,
    pub(crate) event_trigger_action: &'a mut String,
    pub(crate) workspace_name_input: &'a mut String,
    pub(crate) action: Option<PanelAction>,
    pub(crate) q: String,
    pub(crate) forced_open: Option<bool>,
}

impl<'a> PropPanelCtx<'a> {
    pub(crate) fn matches(&self, label: &str) -> bool {
        self.q.is_empty() || label.to_lowercase().contains(&self.q)
    }
}

/// One of the six Canva-style drawer groups surfaced by the left icon rail.
///
/// Each group owns a disjoint slice of the ~43 property section functions; every
/// section is reachable through exactly one group. `draw_drawer` renders the
/// active group, so only one group's sections are visible at a time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum DrawerGroup {
    /// Tool palette: selection/navigation/drawing/path tools + pinned hotbar.
    Tools,
    /// Selection inspector: navigator, selected node, tool/shape options, symbol
    /// overrides, text-variable binding (+ the Direct-Select vertex panel).
    Inspector,
    /// Shape/appearance operations: combine, boolean, blend, pathfinder, etc.
    Modify,
    /// Alignment, distribution, distances, dimensions, layer ops.
    Arrange,
    /// Swatches, gradients, styles, width profiles, symbols, variables, libraries.
    Assets,
    /// Document-level tooling: export, data-viz, analysis, grammar, actions, etc.
    Document,
    /// Edit history and branches.
    History,
}

impl DrawerGroup {
    /// All groups in rail order (top to bottom).
    pub const ALL: [DrawerGroup; 7] = [
        DrawerGroup::Tools,
        DrawerGroup::Inspector,
        DrawerGroup::Modify,
        DrawerGroup::Arrange,
        DrawerGroup::Assets,
        DrawerGroup::Document,
        DrawerGroup::History,
    ];

    /// Phosphor glyph shown on the rail button.
    pub fn icon(self) -> &'static str {
        match self {
            DrawerGroup::Tools => ph::TOOLBOX,
            DrawerGroup::Inspector => ph::SLIDERS_HORIZONTAL,
            DrawerGroup::Modify => ph::MAGIC_WAND,
            DrawerGroup::Arrange => ph::ARROWS_OUT_CARDINAL,
            DrawerGroup::Assets => ph::SWATCHES,
            DrawerGroup::Document => ph::FILE_TEXT,
            DrawerGroup::History => ph::CLOCK_COUNTER_CLOCKWISE,
        }
    }

    /// Human-readable title shown in the drawer header and rail tooltip.
    pub fn title(self) -> &'static str {
        match self {
            DrawerGroup::Tools => "Tools",
            DrawerGroup::Inspector => "Inspector",
            DrawerGroup::Modify => "Modify",
            DrawerGroup::Arrange => "Arrange",
            DrawerGroup::Assets => "Assets",
            DrawerGroup::Document => "Document",
            DrawerGroup::History => "History",
        }
    }

    /// Whether this drawer has any content worth showing in the current context.
    /// Rail icons for groups returning `false` are disabled, and an open drawer
    /// that loses its content auto-collapses. Tools, Inspector (navigator + tool
    /// options), and the always-on library/document/history groups are always
    /// available; the operation drawers (Modify/Arrange) need a selection.
    pub fn has_content(self, selection_count: usize) -> bool {
        match self {
            DrawerGroup::Tools
            | DrawerGroup::Inspector
            | DrawerGroup::Assets
            | DrawerGroup::Document
            | DrawerGroup::History => true,
            DrawerGroup::Modify | DrawerGroup::Arrange => selection_count >= 1,
        }
    }
}

/// Render one drawer group: the group header + the shared search bar, then that
/// group's section functions in their original dispatch order. Returns any
/// queued [`PanelAction`].
///
/// Replaces the former always-on `draw_properties_panel` monolith: the section
/// functions are unchanged, just partitioned across the six [`DrawerGroup`]s.
pub(crate) fn draw_drawer(
    ui: &mut Ui,
    ctx: &mut PropPanelCtx,
    group: DrawerGroup,
) -> Option<PanelAction> {
    ui.label(
        RichText::new(group.title().to_uppercase())
            .small()
            .color(Color32::from_rgb(80, 80, 110)),
    );
    ui.add_space(2.0);

    // ── Search bar ────────────────────────────────────────────────
    // Filters the section headers within the *open* drawer (each section fn
    // honours `ctx.q` / `ctx.forced_open`).
    ui.horizontal(|ui| {
        let response = ui.add(
            egui::TextEdit::singleline(&mut *ctx.prop_search)
                .hint_text("Search properties…")
                .desired_width(ui.available_width() - 24.0),
        );
        if !ctx.prop_search.is_empty()
            && ui
                .small_button(ph::X)
                .on_hover_text("Clear search")
                .clicked()
        {
            ctx.prop_search.clear();
            response.surrender_focus();
        }
    });
    ui.add_space(4.0);

    // An empty query matches everything; a non-empty query forces matching
    // headers open so their contents are visible.
    ctx.q = ctx.prop_search.trim().to_lowercase();
    ctx.forced_open = if ctx.q.is_empty() { None } else { Some(true) };

    match group {
        DrawerGroup::Inspector => {
            // ── Context-aware: vertex editing (Direct Selection) ──────────
            // When a path is in point-edit mode, the inspector shows ONLY
            // anchor/vertex properties — node Transform/Fill/Stroke/Path
            // sections are suppressed, like Illustrator. This early-return
            // belongs to the Inspector drawer.
            if ctx.active_tool == Tool::DirectSelect {
                if let Some(nid) = ctx.point_edit_node {
                    if let Some(node) = ctx.doc.nodes.get(&nid) {
                        draw_vertex_panel(ui, node, nid, ctx.point_selected, &mut ctx.action);
                        return ctx.action.take();
                    }
                }
            }
            draw_navigator_section(ui, ctx);
            draw_selected_node(ui, ctx);
            draw_tool_shape_options(ui, ctx);
            draw_symbol_overrides(ui, ctx);
            draw_text_variable_binding(ui, ctx);
        }
        DrawerGroup::Modify => {
            draw_combine(ui, ctx);
            draw_boolean_ops(ui, ctx);
            draw_blend(ui, ctx);
            draw_pathfinder(ui, ctx);
            draw_distribute_on_path(ui, ctx);
            draw_compound_path(ui, ctx);
            draw_clipping_mask(ui, ctx);
            draw_blend_colors(ui, ctx);
            draw_adjust_colors(ui, ctx);
            draw_flatten_transparency(ui, ctx);
            draw_copy_appearance(ui, ctx);
        }
        DrawerGroup::Arrange => {
            draw_arrange_align(ui, ctx);
            draw_alignment(ui, ctx);
            draw_distribute_no_overlap(ui, ctx);
            draw_align_to_artboard(ui, ctx);
            draw_distances(ui, ctx);
            draw_dimension_annotations(ui, ctx);
            draw_layer_operations(ui, ctx);
        }
        DrawerGroup::Assets => {
            draw_color_swatches(ui, ctx);
            draw_spot_colors(ui, ctx);
            draw_gradient_swatches(ui, ctx);
            draw_graphic_styles(ui, ctx);
            draw_width_profiles(ui, ctx);
            draw_symbols_panel(ui, ctx);
            draw_variables(ui, ctx);
            draw_libraries_export(ui, ctx);
        }
        DrawerGroup::Document => {
            draw_export_profiles(ui, ctx);
            draw_data_visualization(ui, ctx);
            draw_analysis(ui, ctx);
            draw_composition_analysis(ui, ctx);
            draw_document_grammar(ui, ctx);
            draw_document_workflow(ui, ctx);
            draw_actions(ui, ctx);
            draw_event_triggers(ui, ctx);
            draw_workspaces(ui, ctx);
        }
        DrawerGroup::History => {
            draw_edit_history(ui, ctx);
            draw_branches(ui, ctx);
        }
        // Tools is rendered by the app layer (it needs tool state, not the
        // property ctx), so it is never routed through draw_drawer.
        DrawerGroup::Tools => {}
    }

    ctx.action.take()
}

fn draw_data_visualization(ui: &mut Ui, ctx: &mut PropPanelCtx) {
    let active_tool = ctx.active_tool;
    let polygon_sides = &mut *ctx.polygon_sides;
    let star_points = &mut *ctx.star_points;
    let star_inner_ratio = &mut *ctx.star_inner_ratio;
    let rounded_rect_radius = &mut *ctx.rounded_rect_radius;
    let spiral_turns = &mut *ctx.spiral_turns;
    let spiral_inner_radius = &mut *ctx.spiral_inner_radius;
    let spiral_segs_per_turn = &mut *ctx.spiral_segs_per_turn;
    let line_snap_45 = &mut *ctx.line_snap_45;
    let arc_start_angle = &mut *ctx.arc_start_angle;
    let arc_end_angle = &mut *ctx.arc_end_angle;
    let arc_open = &mut *ctx.arc_open;
    let grid_cols = &mut *ctx.grid_cols;
    let grid_rows = &mut *ctx.grid_rows;
    let polar_grid_rings = &mut *ctx.polar_grid_rings;
    let polar_grid_sectors = &mut *ctx.polar_grid_sectors;
    let polar_grid_inner_ratio = &mut *ctx.polar_grid_inner_ratio;
    let magic_wand_attribute = &mut *ctx.magic_wand_attribute;
    let magic_wand_tolerance = &mut *ctx.magic_wand_tolerance;
    let eraser_radius = &mut *ctx.eraser_radius;
    let q = ctx.q.as_str();
    let matches = |label: &str| -> bool { q.is_empty() || label.to_lowercase().contains(q) };
    let forced_open = ctx.forced_open;
    let mut action: Option<PanelAction> = None;
    // ── Data Visualization (always visible) ──────────────────────────────────
    if matches("Data Visualization") {
        egui::CollapsingHeader::new("Data Visualization")
            .default_open(false)
            .open(forced_open)
            .show(ui, |ui| {
                ui.label(
                    RichText::new("Insert sample charts via MCP or the button below.")
                        .weak()
                        .small(),
                );
                if ui
                    .button("Radar Chart (demo)")
                    .on_hover_text(
                        "Create a sample 5-axis radar chart with 2 series at canvas center",
                    )
                    .clicked()
                {
                    action = Some(PanelAction::CreateRadarChart);
                }
                if ui
                    .button("Stacked Column (demo)")
                    .on_hover_text(
                        "Create a sample stacked column chart with 3 series at canvas center",
                    )
                    .clicked()
                {
                    action = Some(PanelAction::CreateStackedBarChart);
                }
                ui.separator();
                ui.label(RichText::new("Parametric Curves").weak().small());
                if ui
                    .button("Lissajous (demo)")
                    .on_hover_text("Create a Lissajous figure (a=3, b=2, δ=π/4)")
                    .clicked()
                {
                    action = Some(PanelAction::CreateParametricShape {
                        shape_type: "lissajous".to_string(),
                    });
                }
                if ui
                    .button("Superellipse (demo)")
                    .on_hover_text("Create a superellipse (Lamé curve, n=2.5)")
                    .clicked()
                {
                    action = Some(PanelAction::CreateParametricShape {
                        shape_type: "superellipse".to_string(),
                    });
                }
                if ui
                    .button("Rose Curve (demo)")
                    .on_hover_text("Create a rose curve (k=5)")
                    .clicked()
                {
                    action = Some(PanelAction::CreateParametricShape {
                        shape_type: "rose".to_string(),
                    });
                }
                ui.separator();
                ui.label(RichText::new("Generative Patterns").weak().small());
                if ui
                    .button("Truchet Arcs (demo)")
                    .on_hover_text("Generate a Truchet tiling with quarter-circle arc tiles")
                    .clicked()
                {
                    action = Some(PanelAction::CreateTruchetTiling {
                        style: "arcs".to_string(),
                    });
                }
                if ui
                    .button("Truchet Triangles (demo)")
                    .on_hover_text("Generate a Truchet tiling with filled triangle tiles")
                    .clicked()
                {
                    action = Some(PanelAction::CreateTruchetTiling {
                        style: "triangles".to_string(),
                    });
                }
            });
        ui.add_space(4.0);
    }

    let tool_label = match active_tool {
        Tool::Polygon => "Polygon Options",
        Tool::Star => "Star Options",
        Tool::Spiral => "Spiral Options",
        Tool::Line => "Line Options",
        Tool::Arc => "Arc Options",
        Tool::Grid => "Grid Options",
        Tool::PolarGrid => "Polar Grid Options",
        Tool::RoundedRect => "Rounded Rect Options",
        Tool::Select => "Select Shortcuts",
        Tool::ShapeBuilder => "Shape Builder",
        Tool::DirectSelect => "Direct Select",
        Tool::MagicWand => "Magic Wand Options",
        Tool::Eraser => "Eraser Options",
        Tool::Knife => "Knife Options",
        _ => "Tool",
    };

    match active_tool {
        Tool::Polygon
        | Tool::Star
        | Tool::Spiral
        | Tool::Line
        | Tool::Arc
        | Tool::Grid
        | Tool::PolarGrid
        | Tool::RoundedRect
        | Tool::Select
        | Tool::ShapeBuilder
        | Tool::DirectSelect
        | Tool::MagicWand
        | Tool::Eraser
        | Tool::Knife => {
            if matches(tool_label) {
                egui::CollapsingHeader::new(tool_label)
                    .default_open(false)
                    .open(forced_open)
                    .show(ui, |ui| {
                        match active_tool {
                            Tool::Polygon => {
                                ui.label("Sides");
                                ui.add(egui::Slider::new(polygon_sides, 3..=32));
                            }
                            Tool::Star => {
                                ui.label("Points");
                                ui.add(egui::Slider::new(star_points, 3..=20));
                                ui.label("Inner radius ratio");
                                ui.add(egui::Slider::new(star_inner_ratio, 0.1..=0.9));
                            }
                            Tool::Spiral => {
                                ui.label("Turns");
                                ui.add(egui::Slider::new(spiral_turns, 0.25..=20.0).step_by(0.25));
                                ui.label("Inner radius (px)");
                                ui.add(egui::Slider::new(spiral_inner_radius, 0.0..=500.0).suffix("px"));
                                ui.label("Segments per turn");
                                ui.add(egui::Slider::new(spiral_segs_per_turn, 4..=64));
                            }
                            Tool::Line => {
                                ui.checkbox(line_snap_45, "Snap to 45° angles")
                                    .on_hover_text("Also hold Shift while dragging to constrain to multiples of 45°");
                                ui.label(RichText::new("Drag to draw — Shift constrains angle").weak().small());
                            }
                            Tool::Arc => {
                                ui.label("Start angle (°)");
                                let mut start = *arc_start_angle as f32;
                                if ui.add(egui::Slider::new(&mut start, 0.0..=360.0).suffix("°")).changed() {
                                    *arc_start_angle = start as f64;
                                }
                                ui.label("End angle (°)");
                                let mut end = *arc_end_angle as f32;
                                if ui.add(egui::Slider::new(&mut end, 0.0..=360.0).suffix("°")).changed() {
                                    *arc_end_angle = end as f64;
                                }
                                ui.checkbox(arc_open, "Open arc")
                                    .on_hover_text("Open: draw arc stroke only. Closed: fill pie sector back to center.");
                            }
                            Tool::Grid => {
                                ui.label("Columns");
                                ui.add(egui::Slider::new(grid_cols, 1..=32));
                                ui.label("Rows");
                                ui.add(egui::Slider::new(grid_rows, 1..=32));
                                ui.label(RichText::new("Drag to define grid bounds").weak().small());
                            }
                            Tool::PolarGrid => {
                                ui.label("Rings");
                                ui.add(egui::Slider::new(polar_grid_rings, 1..=20));
                                ui.label("Sectors");
                                ui.add(egui::Slider::new(polar_grid_sectors, 1..=36));
                                ui.label("Inner radius ratio");
                                ui.add(egui::Slider::new(polar_grid_inner_ratio, 0.0..=0.95).step_by(0.05));
                                ui.label(RichText::new("Drag to define outer bounds").weak().small());
                            }
                            Tool::RoundedRect => {
                                ui.label("Corner radius");
                                ui.add(egui::Slider::new(rounded_rect_radius, 0.0..=200.0).suffix("px"));
                            }
                            Tool::Select => {
                                ui.label(RichText::new("Ctrl+]  Bring Forward").weak().small());
                                ui.label(RichText::new("Ctrl+[  Send Backward").weak().small());
                                ui.label(RichText::new("Ctrl+Shift+]  Front").weak().small());
                                ui.label(RichText::new("Ctrl+Shift+[  Back").weak().small());
                                ui.label(RichText::new("Ctrl+G  Group (2+ selected)").weak().small());
                                ui.label(RichText::new("Ctrl+Shift+G  Ungroup").weak().small());
                                ui.label(RichText::new("Shift+Click  Multi-select").weak().small());
                            }
                            Tool::ShapeBuilder => {
                                ui.label(RichText::new("Drag across shapes → merge").weak().small());
                                ui.label(RichText::new("Alt+drag → subtract").weak().small());
                                ui.label(RichText::new("Alt+click → delete shape").weak().small());
                            }
                            Tool::DirectSelect => {
                                ui.label(RichText::new("Click body → select object").weak().small());
                                ui.label(RichText::new("Click anchor → select point").weak().small());
                                ui.label(RichText::new("Shift+click → multi-select").weak().small());
                                ui.label(RichText::new("Drag handle → reshape curve").weak().small());
                                ui.label(RichText::new("Drag ◌ widget → round corner").weak().small());
                                ui.label(RichText::new("Del → delete selected points").weak().small());
                                ui.label(RichText::new("Esc → exit point edit").weak().small());
                            }
                            Tool::MagicWand => {
                                ui.label("Match attribute");
                                egui::ComboBox::from_id_salt("mw_attr")
                                    .selected_text(match magic_wand_attribute {
                                        SelectSameAttr::FillColor    => "Fill Color",
                                        SelectSameAttr::StrokeColor  => "Stroke Color",
                                        SelectSameAttr::StrokeWeight => "Stroke Weight",
                                        SelectSameAttr::Opacity      => "Opacity",
                                        SelectSameAttr::BlendMode    => "Blend Mode",
                                        SelectSameAttr::ObjectType   => "Object Type",
                                    })
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(magic_wand_attribute, SelectSameAttr::FillColor,    "Fill Color");
                                        ui.selectable_value(magic_wand_attribute, SelectSameAttr::StrokeColor,  "Stroke Color");
                                        ui.selectable_value(magic_wand_attribute, SelectSameAttr::StrokeWeight, "Stroke Weight");
                                        ui.selectable_value(magic_wand_attribute, SelectSameAttr::Opacity,      "Opacity");
                                        ui.selectable_value(magic_wand_attribute, SelectSameAttr::BlendMode,    "Blend Mode");
                                        ui.selectable_value(magic_wand_attribute, SelectSameAttr::ObjectType,   "Object Type");
                                    });
                                ui.add_space(4.0);
                                ui.label("Tolerance");
                                let mut tol = *magic_wand_tolerance as f32;
                                if ui.add(egui::Slider::new(&mut tol, 0.0..=1.0).step_by(0.01)).changed() {
                                    *magic_wand_tolerance = tol as f64;
                                }
                                ui.label(RichText::new("Click any object → select all matching").weak().small());
                            }
                            Tool::Eraser => {
                                ui.label("Radius");
                                let mut r = *eraser_radius as f32;
                                if ui.add(egui::Slider::new(&mut r, 1.0..=200.0).suffix("px")).changed() {
                                    *eraser_radius = r as f64;
                                }
                                ui.label(RichText::new("Drag across path art → subtract a swept region").weak().small());
                                ui.label(RichText::new("Cuts every visible, unlocked path it touches").weak().small());
                            }
                            Tool::Knife => {
                                ui.label(RichText::new("Drag a line across filled paths → slice into faces").weak().small());
                                ui.label(RichText::new("Each cut face becomes its own editable path").weak().small());
                            }
                            _ => {}
                        }
                    });
            }
        }
        _ => {
            if q.is_empty() {
                ui.label(
                    RichText::new(format!(
                        "Tool: {} {}",
                        active_tool.icon(),
                        active_tool.label()
                    ))
                    .weak(),
                );
            }
        }
    }

    if action.is_some() {
        ctx.action = action;
    }
}

fn draw_export_profiles(ui: &mut Ui, ctx: &mut PropPanelCtx) {
    let doc = ctx.doc;
    let matches = |label: &str| -> bool { ctx.matches(label) };
    let forced_open = ctx.forced_open;
    let mut action: Option<PanelAction> = None;
    // ── Export Profiles (always visible) ──────────────────────────────────────
    if matches("Export Profiles") {
        egui::CollapsingHeader::new("Export Profiles")
            .default_open(false)
            .open(forced_open)
            .show(ui, |ui| {
                if doc.export_profiles.is_empty() {
                    ui.label(
                        RichText::new("No profiles. Use add_export_profile MCP tool to add one.")
                            .weak()
                            .small(),
                    );
                } else {
                    for profile in &doc.export_profiles {
                        ui.horizontal(|ui| {
                            ui.label(format!("{} ({})", profile.name, profile.format));
                            if ui
                                .small_button(ph::X)
                                .on_hover_text("Remove this profile")
                                .clicked()
                            {
                                action = Some(PanelAction::RemoveExportProfile {
                                    name: profile.name.clone(),
                                });
                            }
                        });
                    }
                }
            });
        ui.add_space(2.0);
        if ui.small_button("Copy Template JSON")
            .on_hover_text("Copy document structure (layers, guides, export profiles) to clipboard for use with apply_document_template")
            .clicked()
        {
            action = Some(PanelAction::CopyDocumentTemplate);
        }
        ui.add_space(4.0);
    }

    if action.is_some() {
        ctx.action = action;
    }
}

fn draw_libraries_export(ui: &mut Ui, ctx: &mut PropPanelCtx) {
    let matches = |label: &str| -> bool { ctx.matches(label) };
    // ── Libraries & Export ───────────────────────────────────────────────────
    if matches("Color Swatches")
        || matches("Spot Colors")
        || matches("Gradient Swatches")
        || matches("Graphic Styles")
        || matches("Width Profiles")
        || matches("Export Profiles")
        || matches("Libraries")
        || matches("Export")
    {
        ui.add_space(2.0);
        ui.separator();
        ui.label(
            RichText::new("Libraries & Export")
                .small()
                .color(Color32::from_rgb(80, 80, 110)),
        );
        ui.add_space(2.0);
    }
}

fn draw_color_swatches(ui: &mut Ui, ctx: &mut PropPanelCtx) {
    let doc = ctx.doc;
    let selected_id = ctx.selected_id;
    let swatch_library_selected = &mut *ctx.swatch_library_selected;
    let q = ctx.q.as_str();
    let matches = |label: &str| -> bool { q.is_empty() || label.to_lowercase().contains(q) };
    let forced_open = ctx.forced_open;
    let mut action: Option<PanelAction> = None;
    // ── Color Swatches ────────────────────────────────────────────────────────
    if matches("Color Swatches") {
        egui::CollapsingHeader::new("Color Swatches")
            .default_open(false)
            .open(forced_open)
            .show(ui, |ui| {
                if doc.color_swatches.is_empty() {
                    ui.label(
                        RichText::new(
                            "No swatches. Use add_color_swatch MCP tool or load a library below.",
                        )
                        .weak()
                        .small(),
                    );
                } else {
                    for swatch in &doc.color_swatches {
                        ui.horizontal(|ui| {
                            // color preview square
                            let (rect, _) = ui
                                .allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
                            if let Some(c) = photonic_core::Color::from_hex(&swatch.color_hex) {
                                ui.painter().rect_filled(
                                    rect,
                                    2.0,
                                    egui::Color32::from_rgb(
                                        (c.r * 255.0) as u8,
                                        (c.g * 255.0) as u8,
                                        (c.b * 255.0) as u8,
                                    ),
                                );
                            }
                            ui.label(RichText::new(&swatch.name).small());
                            ui.label(RichText::new(&swatch.color_hex).small().weak());
                            if let Some(sid) = selected_id {
                                if ui.small_button("Apply").clicked() {
                                    action = Some(PanelAction::ApplyColorSwatch {
                                        node_id: sid,
                                        swatch_name: swatch.name.clone(),
                                    });
                                }
                            }
                            if ui.small_button(ph::X).clicked() {
                                action = Some(PanelAction::DeleteColorSwatch {
                                    name: swatch.name.clone(),
                                });
                            }
                        });
                    }
                }
                ui.add_space(4.0);
                ui.separator();
                ui.label(RichText::new("Load Library").small().strong());
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_source("swatch_library_combo")
                        .selected_text(if swatch_library_selected.is_empty() {
                            "web"
                        } else {
                            swatch_library_selected.as_str()
                        })
                        .show_ui(ui, |ui| {
                            for lib in &[
                                "web",
                                "material",
                                "pastels",
                                "earth_tones",
                                "neon",
                                "grayscale",
                            ] {
                                ui.selectable_value(swatch_library_selected, lib.to_string(), *lib);
                            }
                        });
                    if ui.small_button("Load").clicked() {
                        let lib = if swatch_library_selected.is_empty() {
                            "web".to_string()
                        } else {
                            swatch_library_selected.clone()
                        };
                        action = Some(PanelAction::LoadSwatchLibrary {
                            library: lib,
                            clear_existing: false,
                        });
                    }
                });
                // #207: import brand swatches from a design-tokens file.
                if ui
                    .small_button("Import tokens…")
                    .on_hover_text("Register named swatches from a CSS/JSON/Style-Dictionary tokens file")
                    .clicked()
                {
                    action = Some(PanelAction::ImportDesignTokens);
                }
            });
        ui.add_space(4.0);
    }

    if action.is_some() {
        ctx.action = action;
    }
}

fn draw_spot_colors(ui: &mut Ui, ctx: &mut PropPanelCtx) {
    let doc = ctx.doc;
    let selected_id = ctx.selected_id;
    let matches = |label: &str| -> bool { ctx.matches(label) };
    let forced_open = ctx.forced_open;
    let mut action: Option<PanelAction> = None;
    // ── Spot Colors ───────────────────────────────────────────────────────────
    if matches("Spot Colors") {
        egui::CollapsingHeader::new("Spot Colors")
            .default_open(false)
            .open(forced_open)
            .show(ui, |ui| {
                if doc.spot_colors.is_empty() {
                    ui.label(
                        RichText::new("No spot colors. Use define_spot_color MCP tool to add one.")
                            .weak()
                            .small(),
                    );
                } else {
                    for sc in &doc.spot_colors {
                        ui.horizontal(|ui| {
                            // color preview square
                            let (rect, _) = ui
                                .allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
                            if let Some(c) = photonic_core::Color::from_hex(&sc.hex) {
                                ui.painter().rect_filled(
                                    rect,
                                    2.0,
                                    egui::Color32::from_rgb(
                                        (c.r * 255.0) as u8,
                                        (c.g * 255.0) as u8,
                                        (c.b * 255.0) as u8,
                                    ),
                                );
                            }
                            ui.label(RichText::new(&sc.name).small());
                            if sc.overprint {
                                ui.label(
                                    RichText::new("OP")
                                        .small()
                                        .weak()
                                        .color(egui::Color32::from_rgb(200, 140, 40)),
                                );
                            }
                            if let Some(sid) = selected_id {
                                if ui.small_button("Apply").clicked() {
                                    action = Some(PanelAction::ApplySpotColor {
                                        node_id: sid,
                                        color_name: sc.name.clone(),
                                    });
                                }
                            }
                            if ui.small_button(ph::X).clicked() {
                                action = Some(PanelAction::DeleteSpotColor {
                                    name: sc.name.clone(),
                                });
                            }
                        });
                    }
                }
            });
        ui.add_space(4.0);
    }

    if action.is_some() {
        ctx.action = action;
    }
}

fn draw_gradient_swatches(ui: &mut Ui, ctx: &mut PropPanelCtx) {
    let doc = ctx.doc;
    let selected_node = ctx.selected_node;
    let selected_id = ctx.selected_id;
    let matches = |label: &str| -> bool { ctx.matches(label) };
    let forced_open = ctx.forced_open;
    let mut action: Option<PanelAction> = None;
    // ── Gradient Swatches ─────────────────────────────────────────────────────
    if matches("Gradient Swatches") {
        egui::CollapsingHeader::new("Gradient Swatches")
            .default_open(false)
            .open(forced_open)
            .show(ui, |ui| {
                if doc.gradient_swatches.is_empty() {
                    ui.label(RichText::new("No gradient swatches. Select a node with a gradient fill and click Save.").weak().small());
                } else {
                    for swatch in &doc.gradient_swatches {
                        ui.horizontal(|ui| {
                            // gradient preview stripe
                            let (rect, _) = ui.allocate_exact_size(egui::vec2(28.0, 14.0), egui::Sense::hover());
                            // Simple rainbow-ish stripe as a placeholder indicator
                            let p = ui.painter();
                            p.rect_filled(rect, 2.0, egui::Color32::from_rgb(80, 100, 200));
                            p.rect_filled(
                                egui::Rect::from_min_size(
                                    egui::pos2(rect.min.x + rect.width() * 0.4, rect.min.y),
                                    egui::vec2(rect.width() * 0.6, rect.height()),
                                ),
                                0.0,
                                egui::Color32::from_rgba_unmultiplied(220, 120, 50, 200),
                            );
                            ui.label(RichText::new(&swatch.name).small());
                            if let Some(sid) = selected_id {
                                if ui.small_button("Apply")
                                    .on_hover_text(format!("Apply gradient '{}' to selected node", swatch.name))
                                    .clicked()
                                {
                                    action = Some(PanelAction::ApplyGradientSwatch {
                                        node_id: sid,
                                        swatch_name: swatch.name.clone(),
                                    });
                                }
                            }
                            if ui.small_button(ph::X).clicked() {
                                action = Some(PanelAction::DeleteGradientSwatch { name: swatch.name.clone() });
                            }
                        });
                    }
                }
                // Save button — only shown for path nodes with gradient fills
                if let Some(node) = selected_node {
                    use photonic_core::style::FillKind;
                    let has_gradient = if let SceneNodeKind::Path(pn) = &node.kind {
                        matches!(pn.fill.kind, FillKind::Gradient(_) | FillKind::FluidGradient(_) | FillKind::MeshGradient(_))
                    } else {
                        false
                    };
                    if has_gradient {
                        ui.separator();
                        if ui.small_button("Save selected gradient as swatch…")
                            .on_hover_text("Save the selected node's gradient fill as a named swatch")
                            .clicked()
                        {
                            if let Some(nid) = selected_id {
                                action = Some(PanelAction::SaveGradientSwatch {
                                    node_id: nid,
                                    name: format!("{} gradient", node.name),
                                });
                            }
                        }
                    }
                }
            });
        ui.add_space(4.0);
    }

    if action.is_some() {
        ctx.action = action;
    }
}

fn draw_graphic_styles(ui: &mut Ui, ctx: &mut PropPanelCtx) {
    let doc = ctx.doc;
    let selected_id = ctx.selected_id;
    let graphic_style_name_input = &mut *ctx.graphic_style_name_input;
    let q = ctx.q.as_str();
    let matches = |label: &str| -> bool { q.is_empty() || label.to_lowercase().contains(q) };
    let forced_open = ctx.forced_open;
    let mut action: Option<PanelAction> = None;
    // ── Graphic Styles ────────────────────────────────────────────────────────
    if matches("Graphic Styles") {
        egui::CollapsingHeader::new("Graphic Styles")
            .default_open(false)
            .open(forced_open)
            .show(ui, |ui| {
                if doc.graphic_styles.is_empty() {
                    ui.label(
                        RichText::new("No styles saved. Select a node and click Save Style.")
                            .weak()
                            .small(),
                    );
                } else {
                    for gs in &doc.graphic_styles {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&gs.name).small());
                            if let Some(sid) = selected_id {
                                if ui
                                    .small_button("Apply")
                                    .on_hover_text("Apply this style to the selected node")
                                    .clicked()
                                {
                                    action = Some(PanelAction::ApplyGraphicStyle {
                                        node_id: sid,
                                        style_name: gs.name.clone(),
                                    });
                                }
                            }
                            if ui
                                .small_button(ph::X)
                                .on_hover_text("Delete this style")
                                .clicked()
                            {
                                action = Some(PanelAction::DeleteGraphicStyle {
                                    name: gs.name.clone(),
                                });
                            }
                        });
                    }
                }
                if let Some(nid) = selected_id {
                    ui.add_space(4.0);
                    ui.separator();
                    ui.label(RichText::new("Save selected node as style:").small().weak());
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(graphic_style_name_input)
                                .hint_text("Style name…")
                                .desired_width(120.0),
                        );
                        let can_save = !graphic_style_name_input.trim().is_empty();
                        if ui
                            .add_enabled(can_save, egui::Button::new("Save Style").small())
                            .clicked()
                        {
                            action = Some(PanelAction::SaveGraphicStyle {
                                node_id: nid,
                                name: graphic_style_name_input.trim().to_string(),
                            });
                        }
                    });
                }
            });
        ui.add_space(4.0);
    }

    if action.is_some() {
        ctx.action = action;
    }
}

fn draw_width_profiles(ui: &mut Ui, ctx: &mut PropPanelCtx) {
    let doc = ctx.doc;
    let selected_node = ctx.selected_node;
    let selected_id = ctx.selected_id;
    let width_profile_name_input = &mut *ctx.width_profile_name_input;
    let q = ctx.q.as_str();
    let matches = |label: &str| -> bool { q.is_empty() || label.to_lowercase().contains(q) };
    let forced_open = ctx.forced_open;
    let mut action: Option<PanelAction> = None;
    // ── Width Profiles ────────────────────────────────────────────────────────
    if matches("Width Profiles") {
        egui::CollapsingHeader::new("Width Profiles")
            .default_open(false)
            .open(forced_open)
            .show(ui, |ui| {
                if doc.width_profiles.is_empty() {
                    ui.label(
                        RichText::new(
                            "No profiles saved. Use define_width_profile or save from selection.",
                        )
                        .weak()
                        .small(),
                    );
                } else {
                    for wp in &doc.width_profiles {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format!(
                                    "{} ({} pts, avg {:.1}px)",
                                    wp.name,
                                    wp.widths.len(),
                                    wp.average_width()
                                ))
                                .small(),
                            );
                            if let Some(sid) = selected_id {
                                if ui
                                    .small_button("Apply")
                                    .on_hover_text("Set stroke width to profile average")
                                    .clicked()
                                {
                                    action = Some(PanelAction::ApplyWidthProfile {
                                        node_id: sid,
                                        profile_name: wp.name.clone(),
                                    });
                                }
                            }
                            let rename = width_profile_name_input.trim();
                            if ui
                                .add_enabled(
                                    !rename.is_empty(),
                                    egui::Button::new(ph::PENCIL).small(),
                                )
                                .on_hover_text("Rename to the text in the name field below")
                                .clicked()
                            {
                                action = Some(PanelAction::RenameWidthProfile {
                                    old_name: wp.name.clone(),
                                    new_name: rename.to_string(),
                                });
                            }
                            if ui.small_button(ph::X).clicked() {
                                action = Some(PanelAction::DeleteWidthProfile {
                                    name: wp.name.clone(),
                                });
                            }
                        });
                    }
                }
                // Save from selection
                if let Some(node) = selected_node {
                    if let SceneNodeKind::Path(ref pn) = node.kind {
                        ui.add_space(4.0);
                        ui.separator();
                        ui.label(
                            RichText::new(format!(
                                "Save current width ({:.1}px) as profile:",
                                pn.stroke.width
                            ))
                            .small()
                            .weak(),
                        );
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::TextEdit::singleline(width_profile_name_input)
                                    .hint_text("Profile name…")
                                    .desired_width(110.0),
                            );
                            let can_save = !width_profile_name_input.trim().is_empty();
                            if ui
                                .add_enabled(can_save, egui::Button::new("Save").small())
                                .clicked()
                            {
                                action = Some(PanelAction::SaveWidthProfile {
                                    stroke_width: pn.stroke.width,
                                    name: width_profile_name_input.trim().to_string(),
                                });
                            }
                        });
                    }
                }
            });
        ui.add_space(4.0);
    }

    if action.is_some() {
        ctx.action = action;
    }
}

fn draw_analysis(ui: &mut Ui, ctx: &mut PropPanelCtx) {
    let matches = |label: &str| -> bool { ctx.matches(label) };
    // ── Analysis ─────────────────────────────────────────────────────────────
    if matches("Distances")
        || matches("Dimension Annotations")
        || matches("Composition Analysis")
        || matches("Document Grammar")
        || matches("Analysis")
    {
        ui.add_space(2.0);
        ui.separator();
        ui.label(
            RichText::new("Analysis")
                .small()
                .color(Color32::from_rgb(80, 80, 110)),
        );
        ui.add_space(2.0);
    }
}

fn draw_composition_analysis(ui: &mut Ui, ctx: &mut PropPanelCtx) {
    let composition_findings = ctx.composition_findings;
    let rhythm_findings = ctx.rhythm_findings;
    let matches = |label: &str| -> bool { ctx.matches(label) };
    let forced_open = ctx.forced_open;
    let mut action: Option<PanelAction> = None;
    // ── Composition Analysis ──────────────────────────────────────────────────
    if matches("Composition Analysis") {
        egui::CollapsingHeader::new("Composition Analysis")
            .default_open(false)
            .open(forced_open)
            .show(ui, |ui| {
                ui.label(
                    RichText::new("Analyze balance, density, overlap, and color usage.")
                        .weak()
                        .small(),
                );
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    if ui.button("Analyze Canvas").clicked() {
                        action = Some(PanelAction::AnalyzeComposition);
                    }
                    if ui.button("Detect Rhythms").clicked() {
                        action = Some(PanelAction::DetectRhythms);
                    }
                });
                if !composition_findings.is_empty() {
                    ui.add_space(4.0);
                    for finding in composition_findings {
                        ui.label(RichText::new(finding).small());
                    }
                }
                if !rhythm_findings.is_empty() {
                    ui.add_space(4.0);
                    ui.label(RichText::new("Rhythms:").small().strong());
                    for finding in rhythm_findings {
                        ui.label(RichText::new(finding).small());
                    }
                }
            });
        ui.add_space(4.0);
    }

    if action.is_some() {
        ctx.action = action;
    }
}

fn draw_document_grammar(ui: &mut Ui, ctx: &mut PropPanelCtx) {
    let grammar_rules = ctx.grammar_rules;
    let grammar_rule_name_input = &mut *ctx.grammar_rule_name_input;
    let grammar_rule_type_selected = &mut *ctx.grammar_rule_type_selected;
    let grammar_rule_params_input = &mut *ctx.grammar_rule_params_input;
    let grammar_check_results = ctx.grammar_check_results;
    let q = ctx.q.as_str();
    let matches = |label: &str| -> bool { q.is_empty() || label.to_lowercase().contains(q) };
    let forced_open = ctx.forced_open;
    let mut action: Option<PanelAction> = None;
    // ── Document Grammar ─────────────────────────────────────────────────────
    if matches("Document Grammar") {
        egui::CollapsingHeader::new("Document Grammar")
            .default_open(false)
            .open(forced_open)
            .show(ui, |ui| {
                ui.label(
                    RichText::new("Define design rules the document must satisfy.")
                        .weak()
                        .small(),
                );
                ui.add_space(2.0);
                // Rule list
                if grammar_rules.is_empty() {
                    ui.label(RichText::new("No rules defined.").weak().small());
                } else {
                    for (name, rule_type) in grammar_rules {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(format!("{} ({})", name, rule_type)).small());
                            if ui
                                .small_button(ph::X)
                                .on_hover_text("Delete rule")
                                .clicked()
                            {
                                action =
                                    Some(PanelAction::DeleteGrammarRule { name: name.clone() });
                            }
                        });
                    }
                }
                ui.add_space(4.0);
                // Define new rule
                ui.label(RichText::new("Add rule:").small().strong());
                ui.add(
                    egui::TextEdit::singleline(grammar_rule_name_input)
                        .hint_text("Rule name…")
                        .desired_width(ui.available_width()),
                );
                ui.add_space(2.0);
                egui::ComboBox::from_id_salt("grammar_rule_type_combo")
                    .selected_text(if grammar_rule_type_selected.is_empty() {
                        "Rule type…"
                    } else {
                        grammar_rule_type_selected.as_str()
                    })
                    .width(ui.available_width())
                    .show_ui(ui, |ui| {
                        for rt in [
                            "palette_includes",
                            "max_colors",
                            "min_text_size",
                            "required_layer",
                            "max_node_count",
                        ] {
                            ui.selectable_value(grammar_rule_type_selected, rt.to_string(), rt);
                        }
                    });
                ui.add_space(2.0);
                ui.add(
                    egui::TextEdit::singleline(grammar_rule_params_input)
                        .hint_text(r#"Params JSON, e.g. {"count": 5}"#)
                        .desired_width(ui.available_width()),
                );
                ui.add_space(2.0);
                let can_define = !grammar_rule_name_input.trim().is_empty()
                    && !grammar_rule_type_selected.is_empty()
                    && !grammar_rule_params_input.trim().is_empty();
                if ui
                    .add_enabled(can_define, egui::Button::new("Define Rule").small())
                    .clicked()
                {
                    let name = grammar_rule_name_input.trim().to_string();
                    let rule_type = grammar_rule_type_selected.clone();
                    let params_json = grammar_rule_params_input.trim().to_string();
                    action = Some(PanelAction::DefineGrammarRule {
                        name,
                        rule_type,
                        params_json,
                    });
                    grammar_rule_name_input.clear();
                    grammar_rule_params_input.clear();
                }
                ui.add_space(4.0);
                if ui.button("Check Grammar").clicked() {
                    action = Some(PanelAction::CheckGrammar);
                }
                if !grammar_check_results.is_empty() {
                    ui.add_space(4.0);
                    for (rule_name, passed, message) in grammar_check_results {
                        let icon = if *passed { ph::CHECK } else { ph::X };
                        let color = if *passed {
                            Color32::from_rgb(60, 160, 60)
                        } else {
                            Color32::from_rgb(200, 60, 60)
                        };
                        ui.label(
                            RichText::new(format!("{} {}: {}", icon, rule_name, message))
                                .small()
                                .color(color),
                        );
                    }
                }
            });
        ui.add_space(4.0);
    }

    if action.is_some() {
        ctx.action = action;
    }
}

fn draw_document_workflow(ui: &mut Ui, ctx: &mut PropPanelCtx) {
    let matches = |label: &str| -> bool { ctx.matches(label) };
    // ── Document & Workflow ───────────────────────────────────────────────────
    if matches("Actions")
        || matches("History")
        || matches("Event Triggers")
        || matches("Workspaces")
        || matches("Branches")
        || matches("Variables")
        || matches("Variable Binding")
        || matches("Symbol Override")
        || matches("Symbols")
        || matches("Document")
        || matches("Workflow")
    {
        ui.add_space(2.0);
        ui.separator();
        ui.label(
            RichText::new("Document & Workflow")
                .small()
                .color(Color32::from_rgb(80, 80, 110)),
        );
        ui.add_space(2.0);
    }
}

fn draw_actions(ui: &mut Ui, ctx: &mut PropPanelCtx) {
    let action_names = ctx.action_names;
    let matches = |label: &str| -> bool { ctx.matches(label) };
    let forced_open = ctx.forced_open;
    let mut action: Option<PanelAction> = None;
    // ── Actions ───────────────────────────────────────────────────────────────
    if matches("Actions") {
        egui::CollapsingHeader::new("Actions")
            .default_open(false)
            .open(forced_open)
            .show(ui, |ui| {
                ui.label(
                    RichText::new(
                        "Replayable MCP tool sequences. Use define_action MCP tool to record.",
                    )
                    .weak()
                    .small(),
                );
                ui.add_space(2.0);
                if action_names.is_empty() {
                    ui.label(RichText::new("No actions defined.").weak().small());
                } else {
                    for (name, step_count) in action_names {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format!(
                                    "{} ({} step{})",
                                    name,
                                    step_count,
                                    if *step_count == 1 { "" } else { "s" }
                                ))
                                .small(),
                            );
                            if ui
                                .small_button("▶")
                                .on_hover_text(format!("Play '{}'", name))
                                .clicked()
                            {
                                action = Some(PanelAction::PlayAction { name: name.clone() });
                            }
                            if ui
                                .small_button(ph::X)
                                .on_hover_text(format!("Delete '{}'", name))
                                .clicked()
                            {
                                action = Some(PanelAction::DeleteAction { name: name.clone() });
                            }
                        });
                    }
                }
            });
        ui.add_space(4.0);
    }

    if action.is_some() {
        ctx.action = action;
    }
}

fn draw_edit_history(ui: &mut Ui, ctx: &mut PropPanelCtx) {
    let history_entries = ctx.history_entries;
    let history_total = ctx.history_total;
    let matches = |label: &str| -> bool { ctx.matches(label) };
    let mut action: Option<PanelAction> = None;
    // ── Edit History ──────────────────────────────────────────────────────────
    if matches("History") {
        egui::CollapsingHeader::new("Edit History")
            .default_open(false)
            .id_salt("history_panel")
            .show(ui, |ui| {
                ui.label(
                    RichText::new(format!("Edit history ({} steps):", history_total))
                        .weak()
                        .small(),
                );
                ui.add_space(2.0);
                if history_entries.is_empty() {
                    ui.label(RichText::new("No edits yet.").weak().small());
                } else {
                    for (step, desc) in history_entries.iter().take(20) {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(format!("{}. {}", step, desc)).small().color(
                                if *step == 1 {
                                    Color32::from_rgb(180, 210, 255)
                                } else {
                                    Color32::from_rgb(130, 130, 150)
                                },
                            ));
                        });
                    }
                }
                if history_total > 0 {
                    ui.add_space(4.0);
                    ui.separator();
                    ui.add_space(2.0);
                    thread_local! {
                        static JUMP_INDEX: std::cell::RefCell<usize> = std::cell::RefCell::new(0);
                    }
                    JUMP_INDEX.with(|v| {
                        let mut val = (*v.borrow()).min(history_total);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Jump to step:").small());
                            ui.add(
                                egui::DragValue::new(&mut val)
                                    .range(0..=history_total)
                                    .speed(1.0),
                            );
                            if ui
                                .small_button("Jump")
                                .on_hover_text(format!(
                                    "Jump to undo depth {} (0=oldest, {}=current)",
                                    val, history_total
                                ))
                                .clicked()
                            {
                                action = Some(PanelAction::JumpToHistory { index: val });
                            }
                        });
                        *v.borrow_mut() = val;
                    });
                }
            });
        ui.add_space(4.0);
    }

    if action.is_some() {
        ctx.action = action;
    }
}

fn draw_event_triggers(ui: &mut Ui, ctx: &mut PropPanelCtx) {
    let doc = ctx.doc;
    let action_names = ctx.action_names;
    let event_trigger_event = &mut *ctx.event_trigger_event;
    let event_trigger_action = &mut *ctx.event_trigger_action;
    let q = ctx.q.as_str();
    let matches = |label: &str| -> bool { q.is_empty() || label.to_lowercase().contains(q) };
    let mut action: Option<PanelAction> = None;
    // ── Event Triggers ────────────────────────────────────────────────────────
    if matches("Event Triggers") {
        egui::CollapsingHeader::new("Event Triggers")
            .default_open(false)
            .id_salt("event_triggers_panel")
            .show(ui, |ui| {
                ui.label(
                    RichText::new("Map document events to named actions.")
                        .weak()
                        .small(),
                );
                ui.add_space(2.0);

                // List existing triggers.
                let triggers: Vec<(String, String)> = doc
                    .event_triggers
                    .iter()
                    .map(|t| (t.event.clone(), t.action_name.clone()))
                    .collect();
                for (ev, an) in &triggers {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("{} → {}", ev, an)).small());
                        if ui.small_button(ph::X).clicked() {
                            action = Some(PanelAction::RemoveEventTrigger {
                                event: ev.clone(),
                                action_name: Some(an.clone()),
                            });
                        }
                    });
                }

                ui.add_space(2.0);
                ui.separator();
                ui.add_space(2.0);

                // Add new trigger.
                ui.label(RichText::new("Add trigger:").small());
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("event_trigger_event_combo")
                        .selected_text(if event_trigger_event.is_empty() {
                            "Event"
                        } else {
                            event_trigger_event.as_str()
                        })
                        .show_ui(ui, |ui| {
                            for ev in &[
                                "on_open",
                                "on_save",
                                "on_node_create",
                                "on_selection_change",
                            ] {
                                ui.selectable_value(event_trigger_event, ev.to_string(), *ev);
                            }
                        });
                });
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("event_trigger_action_combo")
                        .selected_text(if event_trigger_action.is_empty() {
                            "Action"
                        } else {
                            event_trigger_action.as_str()
                        })
                        .show_ui(ui, |ui| {
                            for (name, _) in action_names {
                                ui.selectable_value(
                                    event_trigger_action,
                                    name.clone(),
                                    name.as_str(),
                                );
                            }
                        });
                    if ui.button("Register").clicked()
                        && !event_trigger_event.is_empty()
                        && !event_trigger_action.is_empty()
                    {
                        action = Some(PanelAction::RegisterEventTrigger {
                            event: event_trigger_event.clone(),
                            action_name: event_trigger_action.clone(),
                        });
                    }
                });
            });
        ui.add_space(4.0);
    }

    if action.is_some() {
        ctx.action = action;
    }
}

fn draw_workspaces(ui: &mut Ui, ctx: &mut PropPanelCtx) {
    let doc = ctx.doc;
    let prop_search = &mut *ctx.prop_search;
    let workspace_name_input = &mut *ctx.workspace_name_input;
    let q = ctx.q.as_str();
    let matches = |label: &str| -> bool { q.is_empty() || label.to_lowercase().contains(q) };
    let mut action: Option<PanelAction> = None;
    // ── Workspaces ────────────────────────────────────────────────────────────
    if matches("Workspaces") {
        egui::CollapsingHeader::new("Workspaces")
            .default_open(false)
            .id_salt("workspaces_panel")
            .show(ui, |ui| {
                ui.label(
                    RichText::new("Named panel filter presets. Load to switch panel layout.")
                        .weak()
                        .small(),
                );
                ui.add_space(2.0);
                // Save new workspace
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(workspace_name_input)
                            .hint_text("Workspace name…")
                            .desired_width(ui.available_width() - 60.0),
                    );
                    let can_save = !workspace_name_input.trim().is_empty();
                    if ui
                        .add_enabled(can_save, egui::Button::new("Save").small())
                        .on_hover_text("Save current panel search query as a workspace")
                        .clicked()
                    {
                        action = Some(PanelAction::SaveWorkspace {
                            name: workspace_name_input.trim().to_string(),
                            search_query: prop_search.clone(),
                        });
                    }
                });
                ui.separator();
                // List workspaces
                if doc.workspaces.is_empty() {
                    ui.label(RichText::new("No workspaces saved.").weak().small());
                } else {
                    for ws in &doc.workspaces {
                        ui.horizontal(|ui| {
                            if ui
                                .button(&ws.name)
                                .on_hover_text(format!(
                                    "Load workspace '{}' (filter: {:?})",
                                    ws.name, ws.search_query
                                ))
                                .clicked()
                            {
                                action = Some(PanelAction::LoadWorkspace {
                                    name: ws.name.clone(),
                                });
                            }
                            if ui
                                .small_button(ph::X)
                                .on_hover_text(format!("Delete workspace '{}'", ws.name))
                                .clicked()
                            {
                                action = Some(PanelAction::DeleteWorkspace {
                                    name: ws.name.clone(),
                                });
                            }
                        });
                    }
                }
            });
        ui.add_space(4.0);
    }

    if action.is_some() {
        ctx.action = action;
    }
}

fn draw_branches(ui: &mut Ui, ctx: &mut PropPanelCtx) {
    let branch_names = ctx.branch_names;
    let branch_name_input = &mut *ctx.branch_name_input;
    let q = ctx.q.as_str();
    let matches = |label: &str| -> bool { q.is_empty() || label.to_lowercase().contains(q) };
    let forced_open = ctx.forced_open;
    let mut action: Option<PanelAction> = None;
    // ── Branches ──────────────────────────────────────────────────────────────
    if matches("Branches") {
        egui::CollapsingHeader::new("Branches")
            .default_open(false)
            .open(forced_open)
            .show(ui, |ui| {
                ui.label(
                    RichText::new("Fork the document state into named branches.")
                        .weak()
                        .small(),
                );
                ui.add_space(2.0);
                // Save new branch
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(branch_name_input)
                            .hint_text("Branch name…")
                            .desired_width(ui.available_width() - 60.0),
                    );
                    let can_save = !branch_name_input.trim().is_empty();
                    if ui
                        .add_enabled(can_save, egui::Button::new("Save").small())
                        .clicked()
                    {
                        let name = branch_name_input.trim().to_string();
                        action = Some(PanelAction::BranchCreate { name });
                        branch_name_input.clear();
                    }
                });
                ui.add_space(4.0);
                if branch_names.is_empty() {
                    ui.label(RichText::new("No branches yet.").weak().small());
                } else {
                    for name in branch_names {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(name).small());
                            if ui
                                .small_button("Switch")
                                .on_hover_text(format!("Restore branch '{}'", name))
                                .clicked()
                            {
                                action = Some(PanelAction::BranchSwitch { name: name.clone() });
                            }
                            if ui
                                .small_button(ph::X)
                                .on_hover_text(format!("Delete branch '{}'", name))
                                .clicked()
                            {
                                action = Some(PanelAction::BranchDelete { name: name.clone() });
                            }
                        });
                    }
                }
            });
        ui.add_space(4.0);
    }

    if action.is_some() {
        ctx.action = action;
    }
}

fn draw_variables(ui: &mut Ui, ctx: &mut PropPanelCtx) {
    let doc = ctx.doc;
    let matches = |label: &str| -> bool { ctx.matches(label) };
    let forced_open = ctx.forced_open;
    let mut action: Option<PanelAction> = None;
    // ── Variables ─────────────────────────────────────────────────────────────
    if matches("Variables") {
        egui::CollapsingHeader::new("Variables")
            .default_open(false)
            .open(forced_open)
            .show(ui, |ui| {
                if doc.variables.is_empty() {
                    ui.label(
                        RichText::new("No variables. Use define_variable MCP tool to add one.")
                            .weak()
                            .small(),
                    );
                } else {
                    for var in &doc.variables {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(format!("{} =", var.name)).small().strong());
                            ui.label(RichText::new(&var.value).small());
                            if ui.small_button(ph::X).clicked() {
                                action = Some(PanelAction::DeleteVariable {
                                    name: var.name.clone(),
                                });
                            }
                        });
                    }
                    ui.add_space(4.0);
                    if ui
                        .small_button("Apply All Variables")
                        .on_hover_text(
                            "Replace bound text node contents with current variable values",
                        )
                        .clicked()
                    {
                        action = Some(PanelAction::ApplyVariables);
                    }
                }
            });
        ui.add_space(4.0);
    }

    if action.is_some() {
        ctx.action = action;
    }
}

fn draw_symbols_panel(ui: &mut Ui, ctx: &mut PropPanelCtx) {
    let doc = ctx.doc;
    let selected_node = ctx.selected_node;
    let selected_id = ctx.selected_id;
    let mut action: Option<PanelAction> = None;
    // ── Symbols panel ────────────────────────────────────────────────────────
    {
        egui::CollapsingHeader::new("Symbols")
            .default_open(false)
            .show(ui, |ui: &mut Ui| {
                // Define as symbol — only when a node is selected
                if let (Some(node), Some(nid)) = (selected_node, selected_id) {
                    if node.symbol_ref.is_none() {
                        // Not already an instance — offer to define
                        if ui.small_button("Define as Symbol…").clicked() {
                            // Use the node's current name as default symbol name
                            action = Some(PanelAction::DefineSymbol {
                                node_id: nid,
                                name: node.name.clone(),
                            });
                        }
                    } else {
                        // This node is a symbol instance — offer break link
                        if ui.small_button("Break Link to Symbol").clicked() {
                            action = Some(PanelAction::BreakLinkToSymbol { node_id: nid });
                        }
                    }
                    ui.separator();
                }

                // Load built-in library
                egui::CollapsingHeader::new("Load Library…")
                    .default_open(false)
                    .id_salt("sym_load_lib")
                    .show(ui, |ui| {
                        ui.label(RichText::new("Add built-in symbols to this document.").weak().small());
                        ui.horizontal(|ui| {
                            if ui.small_button("Arrows").on_hover_text("Load arrow symbols (6 shapes)").clicked() {
                                action = Some(PanelAction::LoadSymbolLibrary { library_name: "arrows".to_string() });
                            }
                            if ui.small_button("Shapes").on_hover_text("Load shape symbols (diamond, star, cross, etc.)").clicked() {
                                action = Some(PanelAction::LoadSymbolLibrary { library_name: "shapes".to_string() });
                            }
                            if ui.small_button("UI Icons").on_hover_text("Load UI icon symbols (checkbox, radio, close, etc.)").clicked() {
                                action = Some(PanelAction::LoadSymbolLibrary { library_name: "ui".to_string() });
                            }
                        });
                    });
                ui.separator();
                // Symbol library list
                if doc.symbols.is_empty() {
                    ui.label(RichText::new("No symbols defined.").small().weak());
                } else {
                    for sym in &doc.symbols {
                        ui.horizontal(|ui: &mut Ui| {
                            ui.label(RichText::new(&sym.name).small());
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui: &mut Ui| {
                                if ui.small_button("Del")
                                    .on_hover_text(format!("Delete symbol '{}'", sym.name))
                                    .clicked()
                                {
                                    action = Some(PanelAction::DeleteSymbol { name: sym.name.clone() });
                                }
                                if ui.small_button("Place")
                                    .on_hover_text(format!("Place an instance of '{}'", sym.name))
                                    .clicked()
                                {
                                    action = Some(PanelAction::PlaceSymbol { symbol_name: sym.name.clone() });
                                }
                            });
                        });
                    }

                    ui.separator();
                    // Symbol Sprayer controls
                    thread_local! {
                        static SPRAY_COUNT: std::cell::RefCell<usize> = std::cell::RefCell::new(10);
                        static SPRAY_SPREAD: std::cell::RefCell<f64> = std::cell::RefCell::new(100.0);
                        static SPRAY_SYM: std::cell::RefCell<String> = std::cell::RefCell::new(String::new());
                    }
                    egui::CollapsingHeader::new("Symbol Sprayer")
                        .default_open(false)
                        .id_salt("sym_sprayer")
                        .show(ui, |ui| {
                            ui.label(RichText::new("Place N instances scattered around canvas center.").weak().small());
                            SPRAY_SYM.with(|s| {
                                let mut val = s.borrow().clone();
                                ui.horizontal(|ui| {
                                    ui.label("Symbol:");
                                    ui.text_edit_singleline(&mut val).on_hover_text("Symbol name to spray");
                                });
                                *s.borrow_mut() = val;
                            });
                            SPRAY_COUNT.with(|c| {
                                let mut val = *c.borrow();
                                ui.horizontal(|ui| {
                                    ui.label("Count:");
                                    ui.add(egui::DragValue::new(&mut val).range(1..=200).speed(1.0));
                                });
                                *c.borrow_mut() = val;
                            });
                            SPRAY_SPREAD.with(|s| {
                                let mut val = *s.borrow();
                                ui.horizontal(|ui| {
                                    ui.label("Spread:");
                                    ui.add(egui::DragValue::new(&mut val).range(1.0..=2000.0).speed(1.0));
                                });
                                *s.borrow_mut() = val;
                            });
                            if ui.button("Spray").on_hover_text("Scatter instances around (0, 0)").clicked() {
                                let sym = SPRAY_SYM.with(|s| s.borrow().clone());
                                let count = SPRAY_COUNT.with(|c| *c.borrow());
                                let spread = SPRAY_SPREAD.with(|s| *s.borrow());
                                if !sym.is_empty() {
                                    action = Some(PanelAction::SpraySymbolInstances {
                                        symbol_name: sym,
                                        count,
                                        x: 0.0,
                                        y: 0.0,
                                        spread,
                                    });
                                }
                            }
                        });
                }
            });
        ui.add_space(2.0);
    }

    if action.is_some() {
        ctx.action = action;
    }
}

// ─── Fill editor ─────────────────────────────────────────────────────────────

/// Discriminant used by the UI to select gradient type (avoids cloning the full kind).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FillType {
    Solid,
    Linear,
    Radial,
    Fluid,
    Mesh,
    Pattern,
}

/// Render a small eyedropper icon button. Returns `true` when clicked.
pub(crate) fn eyedropper_btn(ui: &mut Ui) -> bool {
    ui.small_button(ph::EYEDROPPER)
        .on_hover_text("Pick color from screen (Esc to cancel)")
        .clicked()
}

// ─── Stroke editor ────────────────────────────────────────────────────────────

// ─── Glow editor ──────────────────────────────────────────────────────────────

// ─── Gaussian Glow editor ─────────────────────────────────────────────────────

// ─── Audit panel ──────────────────────────────────────────────────────────────

/// Floating window showing recent MCP tool calls from the in-memory audit log.
pub fn draw_audit_panel(
    ctx: &egui::Context,
    audit_log: &Option<std::sync::Arc<std::sync::Mutex<photonic_core::AuditLog>>>,
    open: &mut bool,
    filter: &mut String,
) {
    egui::Window::new("MCP Audit Log")
        .id(egui::Id::new("audit_panel"))
        .default_size([560.0, 380.0])
        .min_width(400.0)
        .min_height(200.0)
        .open(open)
        .show(ctx, |ui| {
            let Some(log_arc) = audit_log else {
                ui.label("Audit log not available (headless mode).");
                return;
            };
            let (all, total) = match log_arc.lock() {
                Ok(log) => {
                    let entries: Vec<_> = log.entries().iter().rev().take(200).cloned().collect();
                    let total = log.total_recorded();
                    (entries, total)
                }
                Err(_) => {
                    ui.label("Audit log lock unavailable.");
                    return;
                }
            };

            // ── Filter bar ────────────────────────────────────────────────
            ui.horizontal(|ui| {
                ui.label("Filter:");
                ui.text_edit_singleline(filter);
                if ui.small_button(ph::X).clicked() {
                    filter.clear();
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.weak(format!("{} total recorded", total));
                });
            });
            ui.separator();

            let filter_lower = filter.to_lowercase();
            let entries: Vec<_> = if filter_lower.is_empty() {
                all
            } else {
                all.into_iter()
                    .filter(|e: &photonic_core::AuditEntry| {
                        e.tool_name.to_lowercase().contains(&filter_lower)
                    })
                    .collect()
            };

            if entries.is_empty() {
                ui.centered_and_justified(|ui| {
                    ui.weak("No audit entries yet — make an MCP tool call to see it here.");
                });
                return;
            }

            // ── Header row ────────────────────────────────────────────────
            egui::Grid::new("audit_header")
                .num_columns(4)
                .min_col_width(40.0)
                .show(ui, |ui| {
                    ui.strong("#");
                    ui.strong("Time");
                    ui.strong("Tool");
                    ui.strong("ms");
                    ui.end_row();
                });
            ui.separator();

            // ── Scrollable rows ───────────────────────────────────────────
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    egui::Grid::new("audit_entries")
                        .num_columns(4)
                        .striped(true)
                        .min_col_width(40.0)
                        .show(ui, |ui| {
                            for entry in &entries {
                                // ID
                                ui.weak(format!("{}", entry.id));

                                // Timestamp — show HH:MM:SS only
                                let ts_short =
                                    entry.timestamp.get(11..19).unwrap_or(&entry.timestamp);
                                ui.weak(ts_short);

                                // Tool name — color by error status
                                if entry.is_error {
                                    ui.colored_label(
                                        Color32::from_rgb(220, 80, 80),
                                        &entry.tool_name,
                                    );
                                } else {
                                    ui.colored_label(
                                        Color32::from_rgb(100, 200, 120),
                                        &entry.tool_name,
                                    );
                                }

                                // Duration
                                ui.weak(format!("{}ms", entry.duration_ms));

                                ui.end_row();

                                // Result summary (spans all columns)
                                if !entry.result_summary.is_empty() {
                                    ui.label(""); // id col
                                    let summary = if entry.result_summary.len() > 120 {
                                        format!("{}…", &entry.result_summary[..120])
                                    } else {
                                        entry.result_summary.clone()
                                    };
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(summary).weak().italics().size(10.5),
                                        )
                                        .wrap(),
                                    );
                                    ui.label("");
                                    ui.label("");
                                    ui.end_row();
                                }
                            }
                        });
                });
        });
}
