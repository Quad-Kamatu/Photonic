use egui::{Color32, RichText, Ui};
use egui_phosphor::regular as ph;
use photonic_core::{
    layer::LayerId,
    node::NodeId,
    ops::boolean::BooleanOp,
    style::{
        FluidGradient, FluidGradientPoint, Gradient, GradientKind, GradientStop, LineJoin,
        MeshGradient, MeshGradientVertex, Stroke,
    },
    CheckpointInfo, Color, Document, Fill, GaussianGlow, GlowEffect, PrimitiveKind, SceneNode,
    SceneNodeKind,
};
use uuid::Uuid;

use crate::radial_wheel::WheelAction;
use crate::tools::Tool;

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
    NodeFillSolid { node_id: NodeId },
    NodeFillGradStop { node_id: NodeId, idx: usize },
    NodeFillFluid { node_id: NodeId, idx: usize },
    NodeFillMesh { node_id: NodeId, idx: usize },
    NodeStroke { node_id: NodeId },
    NodeOuterGlow { node_id: NodeId },
    NodeInnerGlow { node_id: NodeId },
    NodeGaussianGlow { node_id: NodeId },
}

/// An action requested by a panel widget, to be processed by the main draw loop.
#[derive(Debug)]
pub enum PanelAction {
    /// Reorder a node in z-order.
    ReorderNode { node_id: NodeId, op: ZOrderOp },
    /// Run a boolean operation on the two currently selected nodes.
    BooleanOp(BooleanOp),
    /// Restore the document to a named checkpoint.
    RestoreCheckpoint(Uuid),
    /// Update the fill of a selected node.
    UpdateNodeFill { node_id: NodeId, fill: Fill },
    /// Update the stroke of a selected node.
    UpdateNodeStroke { node_id: NodeId, stroke: Stroke },
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
    DiffWithCheckpoint { checkpoint_id: Uuid },
    /// Clear the active diff highlight overlay.
    ClearDiff,
    /// Insert a midpoint anchor on every segment of a path node.
    AddAnchorPoints { node_id: NodeId },
    /// Open the Simplify Path dialog for a path node.
    OpenSimplifyDialog { node_id: NodeId },
    /// Convert the stroke of a path node into a new filled outline path.
    OutlineStroke { node_id: NodeId },
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
    /// Apply a named width profile to the selected path node.
    ApplyWidthProfile {
        node_id: NodeId,
        profile_name: String,
    },
    /// Save a width profile from the selected node's current stroke width.
    SaveWidthProfile { stroke_width: f64, name: String },
    /// Delete a named width profile.
    DeleteWidthProfile { name: String },
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
            WheelAction::OutlineStroke(id) => Self::OutlineStroke { node_id: id },
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

/// Draw the horizontal toolbar (logo, doc name, zoom — no tool buttons).
pub fn draw_toolbar(ui: &mut Ui, doc_name: &str, zoom: f64, file_status: Option<&str>) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!("{} Photonic", ph::HEXAGON))
                .strong()
                .color(Color32::from_rgb(110, 86, 207)),
        );
        ui.separator();
        ui.label(doc_name);

        // Right-aligned cluster: zoom readout, then (optionally) the file-status
        // message. Both are rendered in the *same* right-to-left layout so they
        // stack next to each other instead of overlapping in the top-right.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(format!("{:.0}%", zoom * 100.0));
            ui.label("Zoom:");
            if let Some(status) = file_status {
                ui.separator();
                ui.label(RichText::new(status).weak().italics());
            }
        });
    });
}

/// Draw the vertical tools panel. Returns the newly selected tool if changed.
pub fn draw_tools_panel(ui: &mut Ui, active: Tool, pinned_tools: &[Tool]) -> Option<Tool> {
    let mut chosen = None;

    // ── Hotbar (pinned tools) ─────────────────────────────────────────────
    if !pinned_tools.is_empty() {
        ui.label(
            RichText::new(format!("{} HOTBAR", egui_phosphor::regular::PUSH_PIN))
                .small()
                .color(Color32::from_rgb(110, 86, 207)),
        );
        ui.add_space(2.0);
        for tool in pinned_tools {
            let label = format!("{} {}", tool.icon(), tool.label());
            if ui.selectable_label(*tool == active, label).clicked() {
                chosen = Some(*tool);
            }
        }
        ui.separator();
        ui.add_space(2.0);
    }

    ui.label(
        RichText::new("TOOLS")
            .small()
            .color(Color32::from_rgb(80, 80, 110)),
    );
    ui.add_space(2.0);

    // ── Selection & navigation ────────────────────────────────────────────
    for tool in [Tool::Select, Tool::DirectSelect, Tool::Pan] {
        let label = format!("{} {}", tool.icon(), tool.label());
        if ui.selectable_label(tool == active, label).clicked() {
            chosen = Some(tool);
        }
    }

    // ── Shapes group (Rectangle / Ellipse / Polygon / Star) ──────────────
    // A single button that shows the active shape's icon and opens a
    // hover popover for switching between shape types.
    {
        let popup_id = ui.make_persistent_id("shapes_popover");
        let is_shape_active = active.is_shape_creator();

        // Active shape's icon/label, or Rectangle as the default
        let (group_icon, group_label) = if is_shape_active {
            (active.icon(), active.label())
        } else {
            (Tool::Rectangle.icon(), "Shapes")
        };

        // "›" indicator signals that sub-tools are available
        let btn_text = format!("{} {}  ›", group_icon, group_label);
        let response = ui.selectable_label(is_shape_active, &btn_text);

        // Open the popover on hover
        if response.hovered() {
            ui.memory_mut(|m| m.open_popup(popup_id));
        }

        // Direct click (without hovering into the popover) activates the
        // currently-shown shape, or Rectangle when no shape is active.
        if response.clicked() && !is_shape_active {
            chosen = Some(Tool::Rectangle);
        }

        // Render the popover to the right of the button
        if ui.memory(|m| m.is_popup_open(popup_id)) {
            let pos = egui::pos2(response.rect.right() + 4.0, response.rect.top());

            let area_resp = egui::Area::new(popup_id)
                .kind(egui::UiKind::Popup)
                .order(egui::Order::Foreground)
                .pivot(egui::Align2::LEFT_TOP)
                .fixed_pos(pos)
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style())
                        .show(ui, |ui| {
                            ui.set_min_width(110.0);
                            let mut picked: Option<Tool> = None;
                            for shape in [
                                Tool::Rectangle,
                                Tool::Ellipse,
                                Tool::Polygon,
                                Tool::Star,
                                Tool::Spiral,
                            ] {
                                let label = format!("{} {}", shape.icon(), shape.label());
                                if ui.selectable_label(shape == active, label).clicked() {
                                    picked = Some(shape);
                                }
                            }
                            picked
                        })
                        .inner
                });

            // Close when the pointer leaves both the button and the popover
            let popup_rect = area_resp.response.rect;
            let pointer_in_popup = ui
                .ctx()
                .pointer_latest_pos()
                .map(|p| popup_rect.contains(p))
                .unwrap_or(false);

            if !response.hovered() && !pointer_in_popup {
                ui.memory_mut(|m| m.close_popup());
            }

            if let Some(tool) = area_resp.inner {
                chosen = Some(tool);
                ui.memory_mut(|m| m.close_popup());
            }
        }
    }

    // ── Drawing tools ─────────────────────────────────────────────────────
    for tool in [Tool::Pen, Tool::ShapeBuilder, Tool::Text] {
        let label = format!("{} {}", tool.icon(), tool.label());
        if ui.selectable_label(tool == active, label).clicked() {
            chosen = Some(tool);
        }
    }

    // ── Path editing tools ─────────────────────────────────────────────────
    ui.add_space(4.0);
    ui.separator();
    ui.add_space(2.0);
    for tool in [Tool::Scissors, Tool::MagicWand, Tool::Lasso, Tool::Pencil] {
        let label = format!("{} {}", tool.icon(), tool.label());
        if ui.selectable_label(tool == active, label).clicked() {
            chosen = Some(tool);
        }
    }

    chosen
}

/// Draw the left layers panel. Returns an optional action triggered by context menus.
pub fn draw_layers_panel(
    ui: &mut Ui,
    doc: &Document,
    selected_layer_ids: &mut Vec<LayerId>,
) -> Option<PanelAction> {
    let mut action: Option<PanelAction> = None;

    ui.label(
        RichText::new("LAYERS")
            .small()
            .color(Color32::from_rgb(80, 80, 110)),
    );
    ui.add_space(2.0);

    // Prune any stale selected_layer_ids (layers that no longer exist).
    selected_layer_ids.retain(|id| doc.layers.contains_key(id));

    // Layers from top to bottom in UI (reversed from draw order)
    for layer_id in doc.layer_order.iter().rev() {
        let Some(layer) = doc.layers.get(layer_id) else {
            continue;
        };
        let lid = *layer_id;

        ui.horizontal(|ui| {
            // Checkbox for multi-layer selection (used by Merge Layers).
            let mut checked = selected_layer_ids.contains(&lid);
            if ui.checkbox(&mut checked, "").changed() {
                if checked {
                    if !selected_layer_ids.contains(&lid) {
                        selected_layer_ids.push(lid);
                    }
                } else {
                    selected_layer_ids.retain(|id| id != &lid);
                }
            }

            // Color swatch — shows layer color tag; click cycles through preset colors.
            let swatch_color = match layer.color {
                Some([r, g, b, a]) => Color32::from_rgba_unmultiplied(
                    (r * 255.0) as u8,
                    (g * 255.0) as u8,
                    (b * 255.0) as u8,
                    (a * 255.0) as u8,
                ),
                None => Color32::from_gray(60),
            };
            // Cycle: None → Red → Orange → Yellow → Green → Blue → Purple → None
            const LAYER_COLORS: &[Option<[f32; 4]>] = &[
                None,
                Some([0.85, 0.20, 0.20, 1.0]),
                Some([0.90, 0.55, 0.10, 1.0]),
                Some([0.85, 0.80, 0.10, 1.0]),
                Some([0.20, 0.70, 0.25, 1.0]),
                Some([0.15, 0.45, 0.85, 1.0]),
                Some([0.60, 0.20, 0.80, 1.0]),
            ];
            let swatch_btn = egui::Button::new("")
                .fill(swatch_color)
                .min_size(egui::vec2(10.0, 10.0));
            if ui
                .add(swatch_btn)
                .on_hover_text("Click to cycle layer color tag")
                .clicked()
            {
                // Find the current color in the preset list and advance to next.
                let cur_idx = LAYER_COLORS
                    .iter()
                    .position(|c| *c == layer.color)
                    .unwrap_or(0);
                let next_color = LAYER_COLORS[(cur_idx + 1) % LAYER_COLORS.len()];
                action = Some(PanelAction::SetLayerColor {
                    layer_id: lid,
                    color: next_color,
                });
            }

            // Template toggle — "T" button; dimmed when not a template layer.
            let t_btn = egui::Button::new(RichText::new("T").small().color(if layer.is_template {
                Color32::from_rgb(255, 180, 60)
            } else {
                Color32::from_gray(90)
            }))
            .min_size(egui::vec2(14.0, 14.0));
            if ui
                .add(t_btn)
                .on_hover_text(if layer.is_template {
                    "Template layer (locked, dimmed) — click to disable"
                } else {
                    "Click to make this a template layer (locked, dimmed reference)"
                })
                .clicked()
            {
                action = Some(PanelAction::SetLayerTemplate {
                    layer_id: lid,
                    is_template: !layer.is_template,
                });
            }

            let layer_label = if layer.is_template {
                RichText::new(format!("{} [T]", layer.name))
                    .italics()
                    .weak()
            } else if layer.visible {
                RichText::new(format!("{}", layer.name))
            } else {
                RichText::new(format!("{} (hidden)", layer.name)).weak()
            };

            egui::CollapsingHeader::new(layer_label)
                .id_salt(lid)
                .default_open(true)
                .show(ui, |ui| {
                    let node_ids: Vec<_> = layer.node_ids.iter().rev().collect();
                    if node_ids.is_empty() {
                        ui.label(RichText::new("  (empty)").weak());
                    }
                    for node_id in node_ids {
                        if let Some(node) = doc.nodes.get(node_id) {
                            let nid = *node_id;
                            let response = match &node.kind {
                                SceneNodeKind::Group(g) => {
                                    let header = egui::CollapsingHeader::new(
                                        RichText::new(format!("  ▸ {}", node.name))
                                            .color(Color32::from_rgb(144, 119, 224)),
                                    )
                                    .id_salt(nid)
                                    .default_open(true)
                                    .show(ui, |ui| {
                                        for child_id in g.children.iter().rev() {
                                            if let Some(child) = doc.nodes.get(child_id) {
                                                ui.label(format!("    • {}", child.name));
                                            }
                                        }
                                    });
                                    header.header_response
                                }
                                _ => ui.label(format!("  • {}", node.name)),
                            };

                            response.context_menu(|ui| {
                                if ui.button("Bring to Front").clicked() {
                                    action = Some(PanelAction::ReorderNode {
                                        node_id: nid,
                                        op: ZOrderOp::BringToFront,
                                    });
                                    ui.close_menu();
                                }
                                if ui.button("Bring Forward").clicked() {
                                    action = Some(PanelAction::ReorderNode {
                                        node_id: nid,
                                        op: ZOrderOp::BringForward,
                                    });
                                    ui.close_menu();
                                }
                                if ui.button("Send Backward").clicked() {
                                    action = Some(PanelAction::ReorderNode {
                                        node_id: nid,
                                        op: ZOrderOp::SendBackward,
                                    });
                                    ui.close_menu();
                                }
                                if ui.button("Send to Back").clicked() {
                                    action = Some(PanelAction::ReorderNode {
                                        node_id: nid,
                                        op: ZOrderOp::SendToBack,
                                    });
                                    ui.close_menu();
                                }
                                ui.separator();
                                if ui.button("Collect in New Layer").clicked() {
                                    action = Some(PanelAction::CollectInNewLayer {
                                        node_ids: vec![nid],
                                    });
                                    ui.close_menu();
                                }
                            });
                        }
                    }
                });
        });
    }

    // Show "Merge Selected" when 2+ layers are checked.
    if selected_layer_ids.len() >= 2 {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui
                .button(format!("Merge {} Layers", selected_layer_ids.len()))
                .on_hover_text(
                    "Merge selected layers into one (bottom-most in stack order is kept)",
                )
                .clicked()
            {
                action = Some(PanelAction::MergeLayers {
                    layer_ids: selected_layer_ids.clone(),
                });
                selected_layer_ids.clear();
            }
        });
    }

    // Flatten Artwork button (always shown when > 1 layer)
    if doc.layer_order.len() > 1 {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui
                .button("Flatten Artwork")
                .on_hover_text("Merge all layers into one; bottom-most layer is kept")
                .clicked()
            {
                action = Some(PanelAction::FlattenArtwork);
            }
        });
    }

    ui.separator();
    ui.label(RichText::new(format!("{} objects", doc.node_count())).weak());

    action
}

/// Draw the right properties panel.
/// Returns an optional action if the user clicked a boolean operation button.
/// In-progress edit state for the document color-swatch recolor picker.
/// Stored in egui temp memory so the inline picker survives across frames
/// without threading another `&mut` parameter through `draw_properties_panel`.
#[derive(Clone)]
struct RecolorSwatchEdit {
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

pub fn draw_properties_panel(
    ui: &mut Ui,
    doc: &Document,
    active_tool: Tool,
    fill_color: &mut [f32; 4],
    polygon_sides: &mut u32,
    star_points: &mut u32,
    star_inner_ratio: &mut f32,
    rounded_rect_radius: &mut f64,
    spiral_turns: &mut f32,
    spiral_inner_radius: &mut f32,
    spiral_segs_per_turn: &mut u32,
    selected_node: Option<&SceneNode>,
    selected_id: Option<NodeId>,
    selection_count: usize,
    selected_ids: &[NodeId],
    prop_search: &mut String,
    shear_x: &mut f64,
    shear_y: &mut f64,
    line_snap_45: &mut bool,
    color_guide_rule: &mut String,
    arc_start_angle: &mut f64,
    arc_end_angle: &mut f64,
    arc_open: &mut bool,
    grid_cols: &mut u32,
    grid_rows: &mut u32,
    polar_grid_rings: &mut u32,
    polar_grid_sectors: &mut u32,
    polar_grid_inner_ratio: &mut f32,
    recolor_palette_input: &mut String,
    magic_wand_attribute: &mut SelectSameAttr,
    magic_wand_tolerance: &mut f64,
    composition_findings: &[String],
    rhythm_findings: &[String],
    branch_names: &[String],
    branch_name_input: &mut String,
    swatch_library_selected: &mut String,
    graphic_style_name_input: &mut String,
    width_profile_name_input: &mut String,
    grammar_rules: &[(String, String)], // (name, rule_type)
    grammar_rule_name_input: &mut String,
    grammar_rule_type_selected: &mut String,
    grammar_rule_params_input: &mut String,
    grammar_check_results: &[(String, bool, String)], // (rule_name, passed, message)
    distance_results: &[(String, String, f64, f64, f64)], // (from, to, h_gap, v_gap, center_dist)
    action_names: &[(String, usize)],                 // (name, step_count)
    history_entries: &[(usize, String)],              // (step_index, description) newest first
    history_total: usize,
    bleed_mm_input: &mut f64,
    slug_mm_input: &mut f64,
    construction_angle: &mut f64,
    construction_x: &mut f64,
    construction_y: &mut f64,
    margin_top: &mut f64,
    margin_right: &mut f64,
    margin_bottom: &mut f64,
    margin_left: &mut f64,
    event_trigger_event: &mut String,
    event_trigger_action: &mut String,
    workspace_name_input: &mut String,
) -> Option<PanelAction> {
    let mut action: Option<PanelAction> = None;

    ui.label(
        RichText::new("PROPERTIES")
            .small()
            .color(Color32::from_rgb(80, 80, 110)),
    );
    ui.add_space(2.0);

    // ── Search bar ────────────────────────────────────────────────────────────
    ui.horizontal(|ui| {
        let response = ui.add(
            egui::TextEdit::singleline(prop_search)
                .hint_text("Search properties…")
                .desired_width(ui.available_width() - 24.0),
        );
        if !prop_search.is_empty() && ui.small_button("✕").on_hover_text("Clear search").clicked()
        {
            prop_search.clear();
            response.surrender_focus();
        }
    });
    ui.add_space(4.0);

    // Helper: returns true when the section label matches the current query.
    // An empty query matches everything.
    let q = prop_search.trim().to_lowercase();
    let matches = |label: &str| -> bool { q.is_empty() || label.to_lowercase().contains(&q) };
    // When searching, force every matching header open so the user sees the contents.
    let forced_open: Option<bool> = if q.is_empty() { None } else { Some(true) };

    // ── Navigator Panel ───────────────────────────────────────────────────────
    if matches("Navigator") {
        egui::CollapsingHeader::new("Navigator")
            .default_open(false)
            .open(forced_open)
            .show(ui, |ui: &mut Ui| {
                // Collect visible nodes and compute canvas bounds
                let mut min_x = f64::MAX;
                let mut min_y = f64::MAX;
                let mut max_x = f64::MIN;
                let mut max_y = f64::MIN;
                let nav_nodes: Vec<(f64, f64, f64, f64, photonic_core::node::NodeId)> = doc
                    .nodes_in_draw_order()
                    .into_iter()
                    .filter(|n| n.visible)
                    .filter_map(|n: &SceneNode| {
                        let lb = n.local_bounds()?;
                        let (x0, y0): (f64, f64) = n.transform.apply(lb.x0, lb.y0);
                        let (x1, y1): (f64, f64) = n.transform.apply(lb.x1, lb.y1);
                        let nx = x0.min(x1);
                        let ny = y0.min(y1);
                        let nw = (x1 - x0).abs().max(1.0_f64);
                        let nh = (y1 - y0).abs().max(1.0_f64);
                        Some((nx, ny, nw, nh, n.id))
                    })
                    .collect();
                for &(nx, ny, nw, nh, _) in &nav_nodes {
                    if nx < min_x {
                        min_x = nx;
                    }
                    if ny < min_y {
                        min_y = ny;
                    }
                    if nx + nw > max_x {
                        max_x = nx + nw;
                    }
                    if ny + nh > max_y {
                        max_y = ny + nh;
                    }
                }
                if min_x == f64::MAX {
                    min_x = 0.0;
                    min_y = 0.0;
                    max_x = 800.0;
                    max_y = 600.0;
                }
                let canvas_w = (max_x - min_x).max(1.0);
                let canvas_h = (max_y - min_y).max(1.0);

                // Allocate a fixed-height thumbnail area
                let nav_w = ui.available_width().min(200.0);
                let nav_h = (nav_w * (canvas_h / canvas_w) as f32).clamp(40.0, 160.0);
                let (response, painter) =
                    ui.allocate_painter(egui::vec2(nav_w, nav_h), egui::Sense::hover());
                let rect = response.rect;
                // Background
                painter.rect_filled(rect, 2.0, egui::Color32::from_rgb(30, 30, 40));

                // Scale factor
                let sx = nav_w as f64 / canvas_w;
                let sy = nav_h as f64 / canvas_h;
                let scale = sx.min(sy) as f32;

                let off_x = rect.min.x + ((nav_w as f64 - canvas_w * scale as f64) * 0.5) as f32;
                let off_y = rect.min.y + ((nav_h as f64 - canvas_h * scale as f64) * 0.5) as f32;

                // Draw each node as a colored rect
                for &(nx, ny, nw, nh, nid) in &nav_nodes {
                    let is_selected = selected_id == Some(nid);
                    let srx = off_x + ((nx - min_x) * scale as f64) as f32;
                    let sry = off_y + ((ny - min_y) * scale as f64) as f32;
                    let srw = (nw * scale as f64).max(1.0_f64) as f32;
                    let srh = (nh * scale as f64).max(1.0_f64) as f32;
                    let r = egui::Rect::from_min_size(egui::pos2(srx, sry), egui::vec2(srw, srh));
                    let fill_color = if is_selected {
                        egui::Color32::from_rgba_unmultiplied(100, 180, 255, 180)
                    } else {
                        egui::Color32::from_rgba_unmultiplied(180, 180, 200, 120)
                    };
                    painter.rect_filled(r, 1.0, fill_color);
                    if is_selected {
                        painter.rect_stroke(r, 1.0, egui::Stroke::new(1.0, egui::Color32::WHITE));
                    }
                }

                // Stats
                ui.add_space(2.0);
                ui.label(
                    RichText::new(format!(
                        "{} nodes  {:.0}×{:.0}",
                        nav_nodes.len(),
                        canvas_w,
                        canvas_h
                    ))
                    .small()
                    .weak(),
                );
            });
        ui.add_space(2.0);
    }

    // ── Selected node info ────────────────────────────────────────────────────
    if let Some(node) = selected_node {
        ui.label(RichText::new("Selected").strong());
        ui.label(format!("Name:    {}", node.name));
        ui.label(format!("Opacity: {:.0}%", node.opacity * 100.0));

        let [_a, _b, _c, _d, tx, ty] = node.transform.matrix;
        if let Some(nid) = selected_id {
            let mut px = tx;
            let mut py = ty;
            egui::Grid::new("node_pos_grid")
                .num_columns(4)
                .spacing([4.0, 2.0])
                .show(ui, |ui| {
                    ui.label("X:");
                    let x_resp = ui.add(egui::DragValue::new(&mut px).speed(1.0).fixed_decimals(1));
                    ui.label("Y:");
                    let y_resp = ui.add(egui::DragValue::new(&mut py).speed(1.0).fixed_decimals(1));
                    ui.end_row();
                    if x_resp.changed() || y_resp.changed() {
                        action = Some(PanelAction::SetNodePosition {
                            node_id: nid,
                            x: px,
                            y: py,
                        });
                    }
                });
        } else {
            ui.label(format!("X: {:.1}   Y: {:.1}", tx, ty));
        }

        // Rotation input — available for any node type when one node is selected.
        if let Some(nid) = selected_id {
            let [a, b, _c, _d, _tx, _ty] = node.transform.matrix;
            // Extract current rotation angle in degrees from the matrix column vectors.
            let current_deg = b.atan2(a).to_degrees();
            let mut angle_deg = current_deg;
            egui::Grid::new("node_rot_grid")
                .num_columns(2)
                .spacing([4.0, 2.0])
                .show(ui, |ui| {
                    ui.label("R°:");
                    let rot_resp = ui.add(
                        egui::DragValue::new(&mut angle_deg)
                            .speed(0.5)
                            .fixed_decimals(1)
                            .suffix("°"),
                    );
                    if rot_resp.changed() {
                        // Primary first so its current angle defines the delta;
                        // the whole selection rotates about its shared center.
                        let mut node_ids = vec![nid];
                        node_ids.extend(selected_ids.iter().copied().filter(|&i| i != nid));
                        action = Some(PanelAction::RotateNode {
                            node_ids,
                            angle_deg,
                        });
                    }
                });
        }

        if let (Some(nid), photonic_core::SceneNodeKind::Path(pn)) = (selected_id, &node.kind) {
            if let Some(local_r) = pn.path_data.bounding_box() {
                // Compute world-space W/H by transforming the four local corners.
                let affine = node.transform.to_kurbo();
                let cx = [local_r.x0, local_r.x1, local_r.x1, local_r.x0];
                let cy = [local_r.y0, local_r.y0, local_r.y1, local_r.y1];
                let (mut min_x, mut max_x) = (f64::INFINITY, f64::NEG_INFINITY);
                let (mut min_y, mut max_y) = (f64::INFINITY, f64::NEG_INFINITY);
                for i in 0..4 {
                    let p = affine * kurbo::Point::new(cx[i], cy[i]);
                    if p.x < min_x {
                        min_x = p.x;
                    }
                    if p.x > max_x {
                        max_x = p.x;
                    }
                    if p.y < min_y {
                        min_y = p.y;
                    }
                    if p.y > max_y {
                        max_y = p.y;
                    }
                }
                let mut world_w = (max_x - min_x).max(0.1);
                let mut world_h = (max_y - min_y).max(0.1);
                egui::Grid::new("node_size_grid")
                    .num_columns(4)
                    .spacing([4.0, 2.0])
                    .show(ui, |ui| {
                        ui.label("W:");
                        let w_resp = ui.add(
                            egui::DragValue::new(&mut world_w)
                                .speed(1.0)
                                .fixed_decimals(1),
                        );
                        ui.label("H:");
                        let h_resp = ui.add(
                            egui::DragValue::new(&mut world_h)
                                .speed(1.0)
                                .fixed_decimals(1),
                        );
                        ui.end_row();
                        if (w_resp.changed() || h_resp.changed()) && world_w > 0.1 && world_h > 0.1
                        {
                            action = Some(PanelAction::SetNodeSize {
                                node_id: nid,
                                width: world_w,
                                height: world_h,
                            });
                        }
                    });
            }
        }

        // ── Visibility / Lock toggles ─────────────────────────────────────
        if let Some(nid) = selected_id {
            ui.horizontal(|ui| {
                let eye_icon = if node.visible { ph::EYE } else { ph::EYE_SLASH };
                let eye_tip = if node.visible {
                    "Hide this node"
                } else {
                    "Show this node"
                };
                if ui
                    .button(eye_icon.to_string())
                    .on_hover_text(eye_tip)
                    .clicked()
                {
                    action = Some(PanelAction::SetVisible {
                        node_id: nid,
                        visible: !node.visible,
                    });
                }

                let lock_icon = if node.locked { ph::LOCK } else { ph::LOCK_OPEN };
                let lock_tip = if node.locked {
                    "Unlock this node"
                } else {
                    "Lock this node (prevents canvas selection)"
                };
                if ui
                    .button(lock_icon.to_string())
                    .on_hover_text(lock_tip)
                    .clicked()
                {
                    action = Some(PanelAction::SetLocked {
                        node_id: nid,
                        locked: !node.locked,
                    });
                }
            });
        }

        // ── Path node accordions (alphabetical) ───────────────────────────
        if let (Some(nid), SceneNodeKind::Path(pn)) = (selected_id, &node.kind) {
            // Fill
            if matches("Fill") {
                egui::CollapsingHeader::new("Fill")
                    .default_open(true)
                    .open(forced_open)
                    .show(ui, |ui| {
                        let mut fill_drop: Option<FillColorSlot> = None;
                        if let Some(new_fill) = draw_fill_editor(ui, &pn.fill, &mut fill_drop) {
                            action = Some(PanelAction::UpdateNodeFill {
                                node_id: nid,
                                fill: new_fill,
                            });
                        }
                        if let Some(slot) = fill_drop {
                            action = Some(PanelAction::StartEyedropper(match slot {
                                FillColorSlot::Solid => {
                                    EyedropperTarget::NodeFillSolid { node_id: nid }
                                }
                                FillColorSlot::GradientStop(i) => {
                                    EyedropperTarget::NodeFillGradStop {
                                        node_id: nid,
                                        idx: i,
                                    }
                                }
                                FillColorSlot::FluidPoint(i) => EyedropperTarget::NodeFillFluid {
                                    node_id: nid,
                                    idx: i,
                                },
                                FillColorSlot::MeshVertex(i) => EyedropperTarget::NodeFillMesh {
                                    node_id: nid,
                                    idx: i,
                                },
                            }));
                        }
                        // ── Recent colors swatches ──────────────────────────
                        if !doc.recent_colors.is_empty() {
                            ui.add_space(4.0);
                            ui.label(RichText::new("Recent").weak().small());
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing = egui::vec2(2.0, 2.0);
                                for rc in &doc.recent_colors {
                                    let c32 = Color32::from_rgba_unmultiplied(
                                        (rc.r * 255.0) as u8,
                                        (rc.g * 255.0) as u8,
                                        (rc.b * 255.0) as u8,
                                        (rc.a * 255.0) as u8,
                                    );
                                    let (rect, resp) = ui.allocate_exact_size(
                                        egui::vec2(16.0, 16.0),
                                        egui::Sense::click(),
                                    );
                                    ui.painter().rect_filled(rect, 2.0, c32);
                                    ui.painter().rect_stroke(
                                        rect,
                                        2.0,
                                        egui::Stroke::new(0.5, Color32::from_gray(100)),
                                    );
                                    if resp.clicked() {
                                        use photonic_core::{Color, Fill};
                                        action = Some(PanelAction::UpdateNodeFill {
                                            node_id: nid,
                                            fill: Fill::solid(Color {
                                                r: rc.r,
                                                g: rc.g,
                                                b: rc.b,
                                                a: rc.a,
                                            }),
                                        });
                                    }
                                    if resp.hovered() {
                                        resp.on_hover_text(format!(
                                            "#{:02X}{:02X}{:02X}{:02X}",
                                            (rc.r * 255.0) as u8,
                                            (rc.g * 255.0) as u8,
                                            (rc.b * 255.0) as u8,
                                            (rc.a * 255.0) as u8,
                                        ));
                                    }
                                }
                            });
                        }
                    });
            }

            // Color Guide — only shown when the node has a solid fill
            if matches("Color Guide") {
                use photonic_core::style::FillKind;
                if let FillKind::Solid(base_color) = &pn.fill.kind {
                    if pn.fill.enabled {
                        let base = *base_color;
                        egui::CollapsingHeader::new("Color Guide")
                            .default_open(false)
                            .open(forced_open)
                            .show(ui, |ui| {
                                // Rule selector buttons
                                ui.horizontal_wrapped(|ui| {
                                    for rule in &[
                                        "complementary",
                                        "analogous",
                                        "triadic",
                                        "split_complementary",
                                        "tetradic",
                                        "monochromatic",
                                    ] {
                                        let selected = color_guide_rule.as_str() == *rule;
                                        let label = rule.replace('_', " ");
                                        if ui.selectable_label(selected, label).clicked() {
                                            *color_guide_rule = rule.to_string();
                                        }
                                    }
                                });
                                ui.add_space(4.0);
                                // Swatches
                                let palette = base.harmony(color_guide_rule);
                                ui.horizontal_wrapped(|ui| {
                                    for (i, swatch) in palette.iter().enumerate() {
                                        let c32 = Color32::from_rgb(
                                            (swatch.r * 255.0).round() as u8,
                                            (swatch.g * 255.0).round() as u8,
                                            (swatch.b * 255.0).round() as u8,
                                        );
                                        let (rect, resp) = ui.allocate_exact_size(
                                            egui::vec2(24.0, 24.0),
                                            egui::Sense::click(),
                                        );
                                        ui.painter().rect_filled(rect, 3.0, c32);
                                        if i == 0 {
                                            ui.painter().rect_stroke(
                                                rect,
                                                3.0,
                                                egui::Stroke::new(2.0, Color32::WHITE),
                                            );
                                        }
                                        let hex = swatch.to_hex();
                                        if resp.on_hover_text(hex).clicked() {
                                            let mut new_fill = pn.fill.clone();
                                            new_fill.kind = FillKind::Solid(*swatch);
                                            action = Some(PanelAction::UpdateNodeFill {
                                                node_id: nid,
                                                fill: new_fill,
                                            });
                                        }
                                    }
                                });
                            });
                    }
                }
            }

            // Recolor
            if matches("Recolor") {
                egui::CollapsingHeader::new("Recolor")
                    .default_open(false)
                    .open(forced_open)
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new("Map fills to nearest palette color.")
                                .weak()
                                .small(),
                        );
                        ui.label("Palette (hex, comma-separated):");
                        ui.add(
                            egui::TextEdit::singleline(recolor_palette_input)
                                .hint_text("#FF0000, #00FF00, #0000FF")
                                .desired_width(ui.available_width()),
                        );
                        if ui
                            .button("Apply to Selection")
                            .on_hover_text(
                                "Remap every solid fill to the nearest color in the palette above",
                            )
                            .clicked()
                        {
                            // Parse palette from input string.
                            let palette: Vec<[f32; 4]> = recolor_palette_input
                                .split(',')
                                .filter_map(|hex| {
                                    photonic_core::color::Color::from_hex(hex.trim())
                                        .map(|c| [c.r, c.g, c.b, c.a])
                                })
                                .collect();
                            if !palette.is_empty() {
                                action = Some(PanelAction::RecolorArtwork {
                                    node_ids: vec![nid],
                                    palette,
                                });
                            }
                        }
                    });
            }

            // Inner Glow
            if matches("Inner Glow") {
                egui::CollapsingHeader::new("Inner Glow")
                    .default_open(false)
                    .open(forced_open)
                    .show(ui, |ui| {
                        let mut d = false;
                        if let Some(new_ig) = draw_glow_editor(ui, &node.inner_glow, &mut d) {
                            action = Some(PanelAction::UpdateNodeInnerGlow {
                                node_id: nid,
                                glow: new_ig,
                            });
                        }
                        if d {
                            action = Some(PanelAction::StartEyedropper(
                                EyedropperTarget::NodeInnerGlow { node_id: nid },
                            ));
                        }
                    });
            }

            // Outer Glow
            if matches("Outer Glow") {
                egui::CollapsingHeader::new("Outer Glow")
                    .default_open(false)
                    .open(forced_open)
                    .show(ui, |ui| {
                        let mut d = false;
                        if let Some(new_og) = draw_glow_editor(ui, &node.outer_glow, &mut d) {
                            action = Some(PanelAction::UpdateNodeOuterGlow {
                                node_id: nid,
                                glow: new_og,
                            });
                        }
                        if d {
                            action = Some(PanelAction::StartEyedropper(
                                EyedropperTarget::NodeOuterGlow { node_id: nid },
                            ));
                        }
                    });
            }

            // Gaussian Glow
            if matches("Gaussian Glow") {
                egui::CollapsingHeader::new("Gaussian Glow")
                    .default_open(false)
                    .open(forced_open)
                    .show(ui, |ui| {
                        let mut d = false;
                        if let Some(new_gg) =
                            draw_gaussian_glow_editor(ui, &node.gaussian_glow, &mut d)
                        {
                            action = Some(PanelAction::UpdateNodeGaussianGlow {
                                node_id: nid,
                                glow: new_gg,
                            });
                        }
                        if d {
                            action = Some(PanelAction::StartEyedropper(
                                EyedropperTarget::NodeGaussianGlow { node_id: nid },
                            ));
                        }
                    });
            }

            // Path Operations
            if matches("Path Operations") {
                egui::CollapsingHeader::new("Path Operations")
                    .default_open(false)
                    .open(forced_open)
                    .show(ui, |ui| {
                        if ui.button("Add Anchor Points")
                            .on_hover_text("Insert a midpoint anchor on every path segment")
                            .clicked()
                        {
                            action = Some(PanelAction::AddAnchorPoints { node_id: nid });
                        }
                        ui.horizontal(|ui| {
                            if ui.button("To Smooth")
                                .on_hover_text("Make anchor junction handles collinear (smooth bezier curves)")
                                .clicked()
                            {
                                action = Some(PanelAction::ConvertToSmooth { node_ids: vec![nid] });
                            }
                            if ui.button("To Corner")
                                .on_hover_text("Retract cubic handles to anchor points (sharp cusps / straight lines)")
                                .clicked()
                            {
                                action = Some(PanelAction::ConvertToCorner { node_ids: vec![nid] });
                            }
                        });
                        if ui.button("Average Anchors")
                            .on_hover_text("Move all anchor points to their centroid")
                            .clicked()
                        {
                            action = Some(PanelAction::AverageAnchorPoints { node_id: nid });
                        }
                        if ui.button("Convert to Grayscale")
                            .on_hover_text("Convert all fill and stroke colors to grayscale")
                            .clicked()
                        {
                            action = Some(PanelAction::ConvertToGrayscale { node_ids: vec![nid] });
                        }
                        if pn.stroke.enabled {
                            if ui.button("Outline Stroke")
                                .on_hover_text("Convert this stroke into a filled closed path")
                                .clicked()
                            {
                                action = Some(PanelAction::OutlineStroke { node_id: nid });
                            }
                        }
                        if ui.button("Expand (+2 px)")
                            .on_hover_text("Offset path outward by 2 px, creating a copy")
                            .clicked()
                        {
                            action = Some(PanelAction::OffsetPath { node_ids: vec![nid], distance: 2.0 });
                        }
                        if ui.button("Contract (−2 px)")
                            .on_hover_text("Offset path inward by 2 px, creating a copy")
                            .clicked()
                        {
                            action = Some(PanelAction::OffsetPath { node_ids: vec![nid], distance: -2.0 });
                        }
                        if ui.button("Reverse Direction")
                            .on_hover_text("Reverse the winding direction of this path")
                            .clicked()
                        {
                            action = Some(PanelAction::ReversePathDirection { node_id: nid });
                        }
                        if ui.button("Close Path")
                            .on_hover_text("Append ClosePath to every open subpath")
                            .clicked()
                        {
                            action = Some(PanelAction::JoinPaths { node_ids: vec![nid] });
                        }
                        if ui.button("Divide Objects Below")
                            .on_hover_text("Use this path as a cutting edge to split all objects beneath it; cutter is removed")
                            .clicked()
                        {
                            action = Some(PanelAction::DivideObjectsBelow { node_id: nid });
                        }
                        if ui.button("Round Corners")
                            .on_hover_text("Replace sharp corners with smooth arc fillets")
                            .clicked()
                        {
                            action = Some(PanelAction::RoundCorners {
                                node_ids: vec![nid],
                                radius: 10.0,
                            });
                        }
                        if ui.button("Zig Zag")
                            .on_hover_text("Apply zig-zag wave distortion to this path")
                            .clicked()
                        {
                            action = Some(PanelAction::ZigZagPath {
                                node_ids: vec![nid],
                                size: 10.0,
                                ridges: 4,
                                smooth: false,
                            });
                        }
                        ui.horizontal(|ui| {
                            if ui.button("Pucker")
                                .on_hover_text("Contract path points inward toward centroid")
                                .clicked()
                            {
                                action = Some(PanelAction::PuckerBloat {
                                    node_ids: vec![nid],
                                    strength: -0.3,
                                });
                            }
                            if ui.button("Bloat")
                                .on_hover_text("Expand path points outward from centroid")
                                .clicked()
                            {
                                action = Some(PanelAction::PuckerBloat {
                                    node_ids: vec![nid],
                                    strength: 0.3,
                                });
                            }
                        });
                        if ui.button("Roughen")
                            .on_hover_text("Randomly displace path points for a hand-drawn look")
                            .clicked()
                        {
                            action = Some(PanelAction::RoughenPath {
                                node_ids: vec![nid],
                                size: 5.0,
                                detail: 0,
                                seed: 42,
                            });
                        }
                        ui.horizontal(|ui| {
                            if ui.button("Wave Deform")
                                .on_hover_text("Apply smooth sinusoidal displacement (both axes)")
                                .clicked()
                            {
                                action = Some(PanelAction::NoiseDeform {
                                    node_ids: vec![nid],
                                    amplitude: 8.0,
                                    style: "both".to_string(),
                                });
                            }
                            if ui.button("Swell")
                                .on_hover_text("Sinusoidal deform on Y axis only (bulge effect)")
                                .clicked()
                            {
                                action = Some(PanelAction::NoiseDeform {
                                    node_ids: vec![nid],
                                    amplitude: 12.0,
                                    style: "y".to_string(),
                                });
                            }
                        });
                        if ui.button("Twirl")
                            .on_hover_text("Spiral-rotate path points around centroid (90°)")
                            .clicked()
                        {
                            action = Some(PanelAction::TwirlPath {
                                node_ids: vec![nid],
                                angle_deg: 90.0,
                            });
                        }
                        ui.horizontal(|ui| {
                            if ui.button("Scallop")
                                .on_hover_text("Replace segments with smooth inward arcs")
                                .clicked()
                            {
                                action = Some(PanelAction::ScallopPath {
                                    node_ids: vec![nid],
                                    depth: 10.0,
                                    count: 1,
                                });
                            }
                            if ui.button("Crystallize")
                                .on_hover_text("Add sharp outward spikes to segments")
                                .clicked()
                            {
                                action = Some(PanelAction::CrystallizePath {
                                    node_ids: vec![nid],
                                    size: 10.0,
                                    count: 3,
                                });
                            }
                        });
                        if ui.button("Drop Shadow")
                            .on_hover_text("Add an offset shadow copy behind this path")
                            .clicked()
                        {
                            action = Some(PanelAction::AddDropShadow { node_id: nid });
                        }
                        ui.horizontal(|ui| {
                            for (label, warp) in &[("Arc", "arc"), ("Wave", "wave"), ("Bulge", "bulge"), ("Flag", "flag")] {
                                if ui.button(*label)
                                    .on_hover_text(format!("Apply '{}' warp envelope", warp))
                                    .clicked()
                                {
                                    action = Some(PanelAction::WarpEnvelope {
                                        node_ids: vec![nid],
                                        warp_type: warp.to_string(),
                                        bend: 0.5,
                                    });
                                }
                            }
                        });
                    });
            }

            // Shear / Skew
            if matches("Shear") {
                egui::CollapsingHeader::new("Shear")
                    .default_open(false)
                    .open(forced_open)
                    .show(ui, |ui| {
                        ui.label("Skew the node along the X or Y axis.");
                        egui::Grid::new("shear_grid")
                            .num_columns(2)
                            .spacing([8.0, 4.0])
                            .show(ui, |ui| {
                                ui.label("Shear X:");
                                ui.add(
                                    egui::DragValue::new(shear_x)
                                        .speed(0.01)
                                        .range(-10.0..=10.0),
                                );
                                ui.end_row();
                                ui.label("Shear Y:");
                                ui.add(
                                    egui::DragValue::new(shear_y)
                                        .speed(0.01)
                                        .range(-10.0..=10.0),
                                );
                                ui.end_row();
                            });
                        ui.horizontal(|ui| {
                            if ui
                                .button("Apply Shear")
                                .on_hover_text("Apply the shear transform around the node's centre")
                                .clicked()
                            {
                                if *shear_x != 0.0 || *shear_y != 0.0 {
                                    let mut node_ids = vec![nid];
                                    node_ids
                                        .extend(selected_ids.iter().copied().filter(|&i| i != nid));
                                    action = Some(PanelAction::ShearNode {
                                        node_ids,
                                        shear_x: *shear_x,
                                        shear_y: *shear_y,
                                    });
                                    *shear_x = 0.0;
                                    *shear_y = 0.0;
                                }
                            }
                            if ui
                                .button("Reset")
                                .on_hover_text("Clear shear values")
                                .clicked()
                            {
                                *shear_x = 0.0;
                                *shear_y = 0.0;
                            }
                        });
                    });
            }

            // Flip
            if matches("Flip") {
                ui.horizontal(|ui| {
                    if ui
                        .button("Flip H")
                        .on_hover_text("Flip horizontally")
                        .clicked()
                    {
                        let mut node_ids = vec![nid];
                        node_ids.extend(selected_ids.iter().copied().filter(|&i| i != nid));
                        action = Some(PanelAction::FlipNodes {
                            node_ids,
                            horizontal: true,
                        });
                    }
                    if ui
                        .button("Flip V")
                        .on_hover_text("Flip vertically")
                        .clicked()
                    {
                        let mut node_ids = vec![nid];
                        node_ids.extend(selected_ids.iter().copied().filter(|&i| i != nid));
                        action = Some(PanelAction::FlipNodes {
                            node_ids,
                            horizontal: false,
                        });
                    }
                    if ui
                        .button("Mirror H Copy")
                        .on_hover_text("Duplicate and flip a copy left-right")
                        .clicked()
                    {
                        action = Some(PanelAction::MirrorCopy {
                            node_ids: vec![nid],
                            axis: "horizontal".to_string(),
                        });
                    }
                    if ui
                        .button("Mirror V Copy")
                        .on_hover_text("Duplicate and flip a copy top-bottom")
                        .clicked()
                    {
                        action = Some(PanelAction::MirrorCopy {
                            node_ids: vec![nid],
                            axis: "vertical".to_string(),
                        });
                    }
                });
                ui.add_space(4.0);
            }

            // Radial Copies
            if matches("Radial Copies") {
                thread_local! {
                    static RADIAL_COUNT: std::cell::RefCell<usize> = std::cell::RefCell::new(6);
                }
                RADIAL_COUNT.with(|v| {
                    let mut count = *v.borrow();
                    ui.horizontal(|ui| {
                        ui.label("Radial copies:");
                        ui.add(egui::DragValue::new(&mut count).range(2..=64).speed(1.0));
                        if ui.small_button("Apply").on_hover_text(
                            format!("Create {} evenly-spaced rotational copies around this node's center", count)
                        ).clicked() {
                            action = Some(PanelAction::RotateCopies { node_id: nid, count });
                        }
                    });
                    *v.borrow_mut() = count;
                });
                ui.add_space(4.0);
            }

            // Pin Guides
            if matches("Pin Guides") {
                if ui
                    .button("Pin Guides")
                    .on_hover_text("Add ruler guides at this node's edges and center")
                    .clicked()
                {
                    action = Some(PanelAction::PinObjectGuides {
                        node_ids: vec![nid],
                    });
                }
                ui.add_space(2.0);
            }

            // Select Similar
            if matches("Select Similar") {
                if ui
                    .button("Select Similar Fill")
                    .on_hover_text("Select all nodes with the same fill color (±5 per channel)")
                    .clicked()
                {
                    action = Some(PanelAction::SelectSimilar {
                        node_ids: vec![nid],
                        match_by: "fill_color".to_string(),
                    });
                }
                ui.add_space(2.0);
            }

            // Snap to Pixel
            if matches("Snap to Pixel") {
                egui::CollapsingHeader::new("Snap to Pixel")
                    .default_open(false)
                    .open(forced_open)
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new("Round position to integer coordinates.")
                                .weak()
                                .small(),
                        );
                        if ui
                            .button("Snap to Pixel")
                            .on_hover_text("Round the node's X/Y position to the nearest integer")
                            .clicked()
                        {
                            action = Some(PanelAction::SnapToPixel {
                                node_ids: vec![nid],
                            });
                        }
                    });
            }

            // Stroke
            if matches("Stroke") {
                egui::CollapsingHeader::new("Stroke")
                    .default_open(true)
                    .open(forced_open)
                    .show(ui, |ui| {
                        let mut d = false;
                        if let Some(new_stroke) = draw_stroke_editor(ui, &pn.stroke, &mut d) {
                            action = Some(PanelAction::UpdateNodeStroke {
                                node_id: nid,
                                stroke: new_stroke,
                            });
                        }
                        if d {
                            action =
                                Some(PanelAction::StartEyedropper(EyedropperTarget::NodeStroke {
                                    node_id: nid,
                                }));
                        }
                    });
            }
        }

        // ── Group Operations ───────────────────────────────────────────────
        if let (SceneNodeKind::Group(gn), Some(gid)) = (&node.kind, selected_id) {
            if gn.children.len() > 1 && matches("Reverse Order") {
                if ui
                    .button("Reverse Order")
                    .on_hover_text(
                        "Reverse the front-to-back stacking order of this group's children",
                    )
                    .clicked()
                {
                    action = Some(PanelAction::ReverseNodeOrder {
                        node_ids: vec![gid],
                    });
                }
                ui.add_space(2.0);
            }
            if gn.children.len() > 1 && matches("Flex Layout") {
                egui::CollapsingHeader::new("Flex Layout")
                    .default_open(false)
                    .id_salt("flex_layout_header")
                    .show(ui, |ui| {
                        ui.label(RichText::new("Distribute children in a row or column.").weak().small());
                        ui.horizontal(|ui| {
                            ui.label("Direction:");
                            // We borrow the direction from a thread_local to avoid extra params
                            egui::ComboBox::from_id_salt("flex_dir")
                                .selected_text("row")
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(&mut "row".to_string(), "row".to_string(), "row");
                                    ui.selectable_value(&mut "column".to_string(), "column".to_string(), "column");
                                });
                        });
                        if ui.button("Apply Flex (row, gap=8)")
                            .on_hover_text("Distribute children left-to-right with 8px gap, centered vertically")
                            .clicked()
                        {
                            action = Some(PanelAction::ApplyFlexLayout {
                                group_id: gid,
                                direction: "row".into(),
                                gap: 8.0,
                                align: "center".into(),
                                padding: 0.0,
                            });
                        }
                        if ui.button("Apply Flex (column, gap=8)")
                            .on_hover_text("Distribute children top-to-bottom with 8px gap, centered horizontally")
                            .clicked()
                        {
                            action = Some(PanelAction::ApplyFlexLayout {
                                group_id: gid,
                                direction: "column".into(),
                                gap: 8.0,
                                align: "center".into(),
                                padding: 0.0,
                            });
                        }
                        ui.separator();
                        ui.label(RichText::new("Grid Layout").weak().small());
                        ui.horizontal(|ui| {
                            if ui.button("Grid (3 cols)")
                                .on_hover_text("Arrange children in a 3-column grid with 8px gaps")
                                .clicked()
                            {
                                action = Some(PanelAction::ApplyGridLayout {
                                    group_id: gid, columns: 3, gap_x: 8.0, gap_y: 8.0,
                                });
                            }
                            if ui.button("Grid (4 cols)")
                                .on_hover_text("Arrange children in a 4-column grid with 8px gaps")
                                .clicked()
                            {
                                action = Some(PanelAction::ApplyGridLayout {
                                    group_id: gid, columns: 4, gap_x: 8.0, gap_y: 8.0,
                                });
                            }
                            if ui.button("Stack (center)")
                                .on_hover_text("Stack all children at the same center point (Z-stack)")
                                .clicked()
                            {
                                action = Some(PanelAction::ApplyStackLayout {
                                    group_id: gid,
                                    align_h: "center".to_string(),
                                    align_v: "center".to_string(),
                                });
                            }
                        });
                    });
                ui.add_space(2.0);
            }

            // ── Expand Blend ─────────────────────────────────────────────
            if matches("Expand Blend") {
                if ui.button("Expand Blend")
                    .on_hover_text("Dissolve this group and place all child objects as standalone nodes at the parent layer")
                    .clicked()
                {
                    action = Some(PanelAction::ExpandBlend { group_id: gid });
                }
                ui.add_space(2.0);
            }

            // ── Blend Spine ───────────────────────────────────────────────
            if matches("Blend Spine") {
                egui::CollapsingHeader::new("Blend Spine")
                    .default_open(false)
                    .id_salt("blend_spine_header")
                    .show(ui, |ui| {
                        let spine_label = gn.blend_spine_id
                            .map(|id| id.to_string())
                            .unwrap_or_else(|| "None".into());
                        ui.label(RichText::new(format!("Current spine: {}", spine_label)).weak().small());
                        ui.separator();
                        ui.label(RichText::new("Select a path node from the scene by entering its ID or name:").weak().small());
                        // Use a thread_local for the input string to avoid extra params
                        thread_local! {
                            static SPINE_INPUT: std::cell::RefCell<String> = std::cell::RefCell::new(String::new());
                        }
                        SPINE_INPUT.with(|s| {
                            let mut val = s.borrow().clone();
                            ui.text_edit_singleline(&mut val);
                            *s.borrow_mut() = val;
                        });
                        ui.horizontal(|ui| {
                            if ui.button("Set Spine")
                                .on_hover_text("Assign the entered path node as the blend spine for this group")
                                .clicked()
                            {
                                let path_str = SPINE_INPUT.with(|s| s.borrow().clone());
                                if !path_str.is_empty() {
                                    if let Some(path_id) = uuid::Uuid::parse_str(&path_str).ok() {
                                        action = Some(PanelAction::SetBlendSpine { group_id: gid, path_id });
                                    }
                                }
                            }
                            if gn.blend_spine_id.is_some() {
                                if ui.button("Reverse Spine")
                                    .on_hover_text("Reverse the direction of the blend spine path, inverting the interpolation order")
                                    .clicked()
                                {
                                    action = Some(PanelAction::ReverseBlendSpine { group_id: gid });
                                }
                                if ui.button("Clear Spine")
                                    .on_hover_text("Remove the blend spine assignment from this group")
                                    .clicked()
                                {
                                    action = Some(PanelAction::ClearBlendSpine { group_id: gid });
                                }
                            }
                        });
                    });
                ui.add_space(2.0);
            }
        }

        // ── Per-Node Undo ──────────────────────────────────────────────────
        if let Some(nid) = selected_id {
            if matches("Revert Node") {
                ui.horizontal(|ui| {
                    if ui.button("↩ Revert Last Edit")
                        .on_hover_text("Undo the last edit to this node only, without affecting any other nodes")
                        .clicked()
                    {
                        action = Some(PanelAction::UndoNode { node_id: nid, steps: 1 });
                    }
                    if ui.button("↩↩ Revert 3 Edits")
                        .on_hover_text("Revert this node to its state 3 edits ago")
                        .clicked()
                    {
                        action = Some(PanelAction::UndoNode { node_id: nid, steps: 3 });
                    }
                });
                ui.add_space(2.0);
            }
        }

        // ── Prompt History (read-only) ─────────────────────────────────────
        if !node.prompt_history.is_empty() && matches("Origin") {
            egui::CollapsingHeader::new("Origin (Prompt History)")
                .default_open(false)
                .open(forced_open)
                .show(ui, |ui| {
                    ui.label(
                        RichText::new("AI prompts that created or modified this node:")
                            .weak()
                            .small(),
                    );
                    for (i, prompt) in node.prompt_history.iter().enumerate() {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(format!("{}.", i + 1)).weak().small());
                            ui.label(RichText::new(prompt).small());
                        });
                    }
                });
            ui.add_space(2.0);
        }

        // ── Asset Export ───────────────────────────────────────────────────
        if matches("Asset Export") {
            let nid = node.id;
            egui::CollapsingHeader::new("Asset Export")
                .default_open(node.export_spec.is_some())
                .open(forced_open)
                .show(ui, |ui| {
                    if let Some(spec) = &node.export_spec {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format!("Tagged: {} ({})", spec.name, spec.format))
                                    .small(),
                            );
                            if !spec.scales.is_empty() && spec.format != "svg" {
                                let scale_str: Vec<_> =
                                    spec.scales.iter().map(|s| format!("{}x", s)).collect();
                                ui.label(RichText::new(scale_str.join(", ")).weak().small());
                            }
                        });
                        if ui
                            .small_button("Remove Tag")
                            .on_hover_text("Remove this node's asset export tag")
                            .clicked()
                        {
                            action = Some(PanelAction::RemoveExportTag { node_id: nid });
                        }
                    } else {
                        ui.label(RichText::new("Not tagged for export.").weak().small());
                        if ui
                            .button("Tag as SVG Asset")
                            .on_hover_text("Tag this node for batch SVG export using its name")
                            .clicked()
                        {
                            action = Some(PanelAction::TagNodeForExport {
                                node_id: nid,
                                name: if node.name.is_empty() {
                                    format!("asset-{}", &nid.to_string()[..8])
                                } else {
                                    node.name.clone()
                                },
                                format: "svg".to_string(),
                            });
                        }
                    }
                });
            ui.add_space(2.0);
        }

        // ── Text Operations ────────────────────────────────────────────────
        if let SceneNodeKind::Text(tn) = &node.kind {
            let text_nid = node.id;
            if matches("Text Operations") {
                let mut line_h = tn.line_height;
                let mut letter_sp = tn.letter_spacing;
                egui::CollapsingHeader::new("Text Operations")
                    .default_open(true)
                    .open(forced_open)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label("Line Height");
                            if ui.add(egui::DragValue::new(&mut line_h).speed(0.05).range(0.5..=5.0)).changed() {
                                action = Some(PanelAction::SetTextTypography { node_id: text_nid, line_height: Some(line_h), letter_spacing: None });
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label("Letter Spacing");
                            if ui.add(egui::DragValue::new(&mut letter_sp).speed(0.1).range(-20.0..=50.0).suffix(" px")).changed() {
                                action = Some(PanelAction::SetTextTypography { node_id: text_nid, line_height: None, letter_spacing: Some(letter_sp) });
                            }
                        });
                        // Paragraph spacing and indent
                        ui.horizontal(|ui| {
                            let mut sp_before = tn.paragraph_spacing_before;
                            let mut sp_after  = tn.paragraph_spacing_after;
                            let mut t_indent  = tn.text_indent;
                            let mut changed = false;
                            ui.label("¶ Before:");
                            if ui.add(egui::DragValue::new(&mut sp_before).speed(0.5).range(0.0..=200.0)).changed() { changed = true; }
                            ui.label("After:");
                            if ui.add(egui::DragValue::new(&mut sp_after).speed(0.5).range(0.0..=200.0)).changed() { changed = true; }
                            ui.label("Indent:");
                            if ui.add(egui::DragValue::new(&mut t_indent).speed(0.5).range(-200.0..=200.0)).changed() { changed = true; }
                            if changed {
                                action = Some(PanelAction::SetParagraphOptions {
                                    node_id: text_nid,
                                    spacing_before: sp_before,
                                    spacing_after:  sp_after,
                                    indent:         t_indent,
                                });
                            }
                        });
                        // Tab Stops panel
                        ui.collapsing("Tab Stops", |ui| {
                            thread_local! {
                                static TAB_STOP_INPUT: std::cell::RefCell<f64> = std::cell::RefCell::new(50.0);
                            }
                            let current_stops = tn.tab_stops.clone();
                            if current_stops.is_empty() {
                                ui.label(RichText::new("Default tab spacing (every 4 em)").weak().small());
                            } else {
                                for (i, &stop) in current_stops.iter().enumerate() {
                                    ui.label(format!("  {}: {:.1} px", i + 1, stop));
                                }
                            }
                            ui.horizontal(|ui| {
                                TAB_STOP_INPUT.with(|v| {
                                    ui.label("Add stop:");
                                    ui.add(egui::DragValue::new(&mut *v.borrow_mut()).speed(1.0).range(1.0..=2000.0).suffix(" px"));
                                    if ui.small_button("+").on_hover_text("Add this tab stop position").clicked() {
                                        let new_stop = *v.borrow();
                                        let mut stops = current_stops.clone();
                                        if !stops.contains(&new_stop) {
                                            stops.push(new_stop);
                                            stops.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                                            action = Some(PanelAction::SetTabStops { node_id: text_nid, stops });
                                        }
                                    }
                                });
                            });
                            if !current_stops.is_empty() {
                                if ui.small_button("Clear All").on_hover_text("Remove all custom tab stops").clicked() {
                                    action = Some(PanelAction::ClearTabStops { node_id: text_nid });
                                }
                            }
                        });
                        if ui.button("Find / Replace…")
                            .on_hover_text("Search and replace text content across text nodes")
                            .clicked()
                        {
                            action = Some(PanelAction::OpenFindReplaceTextDialog);
                        }
                        ui.horizontal(|ui| {
                            // Bold toggle
                            let is_bold = tn.font_weight >= 700;
                            let b_label = RichText::new("B").strong();
                            let b_btn = egui::Button::new(b_label)
                                .selected(is_bold)
                                .min_size(egui::vec2(22.0, 0.0));
                            if ui.add(b_btn)
                                .on_hover_text(if is_bold { "Remove Bold (set weight 400)" } else { "Bold (set weight 700)" })
                                .clicked()
                            {
                                let new_w = if is_bold { 400 } else { 700 };
                                action = Some(PanelAction::SetFontWeight { node_id: text_nid, weight: new_w });
                            }
                            // Italic toggle
                            use photonic_core::node::FontStyle;
                            let is_italic = tn.font_style == FontStyle::Italic;
                            let i_label = RichText::new("I").italics();
                            let i_btn = egui::Button::new(i_label)
                                .selected(is_italic)
                                .min_size(egui::vec2(22.0, 0.0));
                            if ui.add(i_btn)
                                .on_hover_text(if is_italic { "Remove Italic" } else { "Italic" })
                                .clicked()
                            {
                                let new_style = if is_italic { "normal".to_string() } else { "italic".to_string() };
                                action = Some(PanelAction::SetFontStyle { node_id: text_nid, style: new_style });
                            }
                        });
                        ui.horizontal(|ui| {
                            let is_vertical = tn.vertical;
                            let label = if is_vertical { "↕ Vertical (click to switch)" } else { "↔ Horizontal (click to switch)" };
                            if ui.small_button(label)
                                .on_hover_text("Toggle between horizontal and vertical text layout")
                                .clicked()
                            {
                                action = Some(PanelAction::SetTextDirection { node_id: text_nid, vertical: !is_vertical });
                            }
                        });
                        // Decoration buttons: U (underline), S (strikethrough), O (overline)
                        ui.horizontal(|ui| {
                            let cur = tn.text_decoration.as_str();
                            let u_active = cur == "underline";
                            let s_active = cur == "line-through";
                            let o_active = cur == "overline";
                            let u_btn = egui::Button::new(RichText::new("U").underline());
                            let s_btn = egui::Button::new(RichText::new("S").strikethrough());
                            let o_btn = egui::Button::new("O̅");
                            if ui.add(u_btn.selected(u_active))
                                .on_hover_text("Underline").clicked()
                            {
                                let dec = if u_active { "" } else { "underline" };
                                action = Some(PanelAction::SetTextDecoration { node_id: text_nid, decoration: dec.to_string() });
                            }
                            if ui.add(s_btn.selected(s_active))
                                .on_hover_text("Strikethrough").clicked()
                            {
                                let dec = if s_active { "" } else { "line-through" };
                                action = Some(PanelAction::SetTextDecoration { node_id: text_nid, decoration: dec.to_string() });
                            }
                            if ui.add(o_btn.selected(o_active))
                                .on_hover_text("Overline").clicked()
                            {
                                let dec = if o_active { "" } else { "overline" };
                                action = Some(PanelAction::SetTextDecoration { node_id: text_nid, decoration: dec.to_string() });
                            }
                        });
                    });
            }
        }

        // ── Character Styles (shown for text nodes when styles exist) ─────
        if let SceneNodeKind::Text(_) = &node.kind {
            let text_nid = node.id;
            if !doc.character_styles.is_empty() && matches("Character Styles") {
                egui::CollapsingHeader::new("Character Styles")
                    .default_open(false)
                    .open(forced_open)
                    .show(ui, |ui| {
                        for style in &doc.character_styles {
                            ui.horizontal(|ui| {
                                let label = if let Some(fs) = style.font_size {
                                    format!("{} ({}pt)", style.name, fs as u32)
                                } else {
                                    style.name.clone()
                                };
                                ui.label(RichText::new(&label).small());
                                if ui
                                    .small_button("Apply")
                                    .on_hover_text(format!(
                                        "Apply '{}' to this text node",
                                        style.name
                                    ))
                                    .clicked()
                                {
                                    action = Some(PanelAction::ApplyCharacterStyle {
                                        node_id: text_nid,
                                        style_name: style.name.clone(),
                                    });
                                }
                                if ui
                                    .small_button("✕")
                                    .on_hover_text("Delete this character style")
                                    .clicked()
                                {
                                    action = Some(PanelAction::DeleteCharacterStyle {
                                        name: style.name.clone(),
                                    });
                                }
                            });
                        }
                    });
                ui.add_space(2.0);
            }
        }

        // ── Paragraph Styles (shown for text nodes when styles exist) ────
        if let SceneNodeKind::Text(_) = &node.kind {
            let text_nid = node.id;
            if !doc.paragraph_styles.is_empty() && matches("Paragraph Styles") {
                egui::CollapsingHeader::new("Paragraph Styles")
                    .default_open(false)
                    .open(forced_open)
                    .show(ui, |ui| {
                        for style in &doc.paragraph_styles {
                            ui.horizontal(|ui| {
                                let label = if let Some(a) = &style.align {
                                    format!("{} ({})", style.name, a)
                                } else {
                                    style.name.clone()
                                };
                                ui.label(RichText::new(&label).small());
                                if ui
                                    .small_button("Apply")
                                    .on_hover_text(format!(
                                        "Apply '{}' to this text node",
                                        style.name
                                    ))
                                    .clicked()
                                {
                                    action = Some(PanelAction::ApplyParagraphStyle {
                                        node_id: text_nid,
                                        style_name: style.name.clone(),
                                    });
                                }
                                if ui
                                    .small_button("✕")
                                    .on_hover_text("Delete this paragraph style")
                                    .clicked()
                                {
                                    action = Some(PanelAction::DeleteParagraphStyle {
                                        name: style.name.clone(),
                                    });
                                }
                            });
                        }
                    });
                ui.add_space(2.0);
            }
        }

        // ── Type on a Path (shown for text nodes) ────────────────────────
        if let SceneNodeKind::Text(ref tn) = node.kind {
            let text_nid = node.id;
            if matches("Type on a Path") {
                egui::CollapsingHeader::new("Type on a Path")
                    .default_open(true)
                    .open(forced_open)
                    .show(ui, |ui| {
                        if let Some(spine_id) = tn.path_spine_id {
                            let spine_name = doc.nodes.get(&spine_id)
                                .map(|n| n.name.clone())
                                .unwrap_or_else(|| spine_id.to_string());
                            ui.label(RichText::new(format!("Spine: {}", spine_name)).small());
                            ui.label(RichText::new(format!("Offset: {:.1} px", tn.path_offset)).small().weak());
                            if ui.button("Clear Path")
                                .on_hover_text("Remove the path spine and revert to normal positioned text")
                                .clicked()
                            {
                                action = Some(PanelAction::ClearTextPath { text_node_id: text_nid });
                            }
                        } else {
                            // Look for a path in the current selection to pair with
                            let path_node_id: Option<NodeId> = doc.selection.ids()
                                .find(|&&sid| sid != text_nid && doc.nodes.get(&sid).map_or(false, |n| matches!(n.kind, SceneNodeKind::Path(_))))
                                .copied();
                            if let Some(pid) = path_node_id {
                                let path_name = doc.nodes.get(&pid).map(|n| n.name.clone()).unwrap_or_default();
                                ui.label(RichText::new(format!("Selected path: {}", path_name)).small().weak());
                                if ui.button("Set as Path Spine")
                                    .on_hover_text("Place this text along the selected path")
                                    .clicked()
                                {
                                    action = Some(PanelAction::SetTextPath {
                                        text_node_id: text_nid,
                                        path_node_id: pid,
                                        offset: 0.0,
                                    });
                                }
                            } else {
                                ui.label(RichText::new("Select a text node + a path node, then click Set as Path Spine.").weak().small());
                            }
                        }
                    });
                ui.add_space(2.0);
            }
        }

        // ── Area Type (shown for text nodes) ─────────────────────────────
        if let SceneNodeKind::Text(ref tn) = node.kind {
            let text_nid = node.id;
            if matches("Area Type") {
                egui::CollapsingHeader::new("Area Type")
                    .default_open(true)
                    .open(forced_open)
                    .show(ui, |ui| {
                        if let Some(area_id) = tn.area_path_id {
                            let area_name = doc.nodes.get(&area_id)
                                .map(|n| n.name.clone())
                                .unwrap_or_else(|| area_id.to_string());
                            ui.label(RichText::new(format!("Area: {}", area_name)).small());
                            if ui.button("Clear Area")
                                .on_hover_text("Remove the area boundary and revert to normal point text")
                                .clicked()
                            {
                                action = Some(PanelAction::ClearTextArea { text_node_id: text_nid });
                            }
                        } else {
                            let area_node_id: Option<NodeId> = doc.selection.ids()
                                .find(|&&sid| sid != text_nid && doc.nodes.get(&sid).map_or(false, |n| matches!(n.kind, SceneNodeKind::Path(_))))
                                .copied();
                            if let Some(aid) = area_node_id {
                                let area_name = doc.nodes.get(&aid).map(|n| n.name.clone()).unwrap_or_default();
                                ui.label(RichText::new(format!("Selected path: {}", area_name)).small().weak());
                                if ui.button("Set as Area Boundary")
                                    .on_hover_text("Flow this text inside the selected closed path")
                                    .clicked()
                                {
                                    action = Some(PanelAction::SetTextArea {
                                        text_node_id: text_nid,
                                        area_path_id: aid,
                                    });
                                }
                            } else {
                                ui.label(RichText::new("Select a text node + a closed path, then click Set as Area Boundary.").weak().small());
                            }
                        }
                    });
                ui.add_space(2.0);
            }
        }

        // ── OpenType Features ─────────────────────────────────────────────
        if let SceneNodeKind::Text(ref tn) = node.kind {
            let text_nid = node.id;
            if matches("OpenType Features") {
                const OTF_FEATURES: &[(&str, &str)] = &[
                    ("liga", "Standard Ligatures"),
                    ("calt", "Contextual Alternates"),
                    ("frac", "Fractions"),
                    ("smcp", "Small Caps"),
                    ("sups", "Superscript"),
                    ("subs", "Subscript"),
                    ("ordn", "Ordinals"),
                    ("swsh", "Swashes"),
                    ("dlig", "Discretionary Ligatures"),
                    ("onum", "Oldstyle Figures"),
                    ("tnum", "Tabular Figures"),
                    ("zero", "Slashed Zero"),
                ];
                egui::CollapsingHeader::new("OpenType Features")
                    .default_open(false)
                    .id_salt("opentype_features_header")
                    .show(ui, |ui| {
                        let mut new_features = tn.opentype_features.clone();
                        let mut changed = false;
                        ui.label(
                            RichText::new("Enable typographic features (font support varies).")
                                .weak()
                                .small(),
                        );
                        ui.add_space(2.0);
                        for (tag, label) in OTF_FEATURES {
                            let mut enabled = new_features.contains(&tag.to_string());
                            if ui.checkbox(&mut enabled, *label).changed() {
                                changed = true;
                                if enabled {
                                    new_features.push(tag.to_string());
                                } else {
                                    new_features.retain(|f| f != *tag);
                                }
                            }
                        }
                        if changed {
                            action = Some(PanelAction::SetOpenTypeFeatures {
                                node_id: text_nid,
                                features: new_features,
                            });
                        }
                    });
                ui.add_space(2.0);
            }
        }

        // ── Text Frame Threading ─────────────────────────────────────────
        if let SceneNodeKind::Text(ref tn) = node.kind {
            let text_nid = node.id;
            if matches("Text Frame Threading") {
                egui::CollapsingHeader::new("Text Frame Threading")
                    .default_open(false)
                    .id_salt("text_frame_thread_header")
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(
                                "Chain text nodes so overflow flows from one to the next.",
                            )
                            .weak()
                            .small(),
                        );
                        ui.add_space(2.0);

                        // Show current chain state.
                        if tn.prev_frame.is_some() || tn.next_frame.is_some() {
                            if let Some(pid) = tn.prev_frame {
                                let pname = doc
                                    .nodes
                                    .get(&pid)
                                    .map(|n| n.name.clone())
                                    .unwrap_or_else(|| pid.to_string());
                                ui.label(RichText::new(format!("← from: {}", pname)).small());
                            }
                            if let Some(nid) = tn.next_frame {
                                let nname = doc
                                    .nodes
                                    .get(&nid)
                                    .map(|n| n.name.clone())
                                    .unwrap_or_else(|| nid.to_string());
                                ui.label(RichText::new(format!("→ to: {}", nname)).small());
                            }
                            if ui.button("Unlink Frame").clicked() {
                                action = Some(PanelAction::UnlinkTextFrames { node_id: text_nid });
                            }
                        } else {
                            // Find another text node in selection to link to.
                            let other_text: Option<NodeId> = doc
                                .selection
                                .ids()
                                .find(|&&sid| {
                                    sid != text_nid
                                        && doc.nodes.get(&sid).map_or(false, |n| {
                                            matches!(n.kind, SceneNodeKind::Text(_))
                                        })
                                })
                                .copied();
                            if let Some(other_id) = other_text {
                                let other_name = doc
                                    .nodes
                                    .get(&other_id)
                                    .map(|n| n.name.clone())
                                    .unwrap_or_default();
                                ui.label(
                                    RichText::new(format!("Selected text node: {}", other_name))
                                        .small()
                                        .weak(),
                                );
                                ui.horizontal(|ui| {
                                    if ui
                                        .button("Link Frame →")
                                        .on_hover_text(
                                            "This node overflows into the selected text node",
                                        )
                                        .clicked()
                                    {
                                        action = Some(PanelAction::LinkTextFrames {
                                            from_id: text_nid,
                                            to_id: other_id,
                                        });
                                    }
                                    if ui
                                        .button("← Link Frame")
                                        .on_hover_text(
                                            "The selected text node overflows into this node",
                                        )
                                        .clicked()
                                    {
                                        action = Some(PanelAction::LinkTextFrames {
                                            from_id: other_id,
                                            to_id: text_nid,
                                        });
                                    }
                                });
                            } else {
                                ui.label(
                                    RichText::new("Select two text nodes to link them.")
                                        .weak()
                                        .small(),
                                );
                            }
                        }
                    });
                ui.add_space(2.0);
            }
        }

        // ── Select Same ──────────────────────────────────────────────────
        if let Some(ref_id) = selected_id {
            if matches("Select Same") {
                egui::CollapsingHeader::new("Select Same")
                    .default_open(false)
                    .open(forced_open)
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new("Select all nodes sharing this attribute")
                                .weak()
                                .small(),
                        );
                        ui.horizontal(|ui| {
                            if ui
                                .button("Fill Color")
                                .on_hover_text("Select nodes with the same solid fill color")
                                .clicked()
                            {
                                action = Some(PanelAction::SelectSame {
                                    node_id: ref_id,
                                    attribute: SelectSameAttr::FillColor,
                                });
                            }
                            if ui
                                .button("Stroke Color")
                                .on_hover_text("Select nodes with the same stroke color")
                                .clicked()
                            {
                                action = Some(PanelAction::SelectSame {
                                    node_id: ref_id,
                                    attribute: SelectSameAttr::StrokeColor,
                                });
                            }
                        });
                        ui.horizontal(|ui| {
                            if ui
                                .button("Stroke Weight")
                                .on_hover_text("Select nodes with the same stroke width")
                                .clicked()
                            {
                                action = Some(PanelAction::SelectSame {
                                    node_id: ref_id,
                                    attribute: SelectSameAttr::StrokeWeight,
                                });
                            }
                            if ui
                                .button("Opacity")
                                .on_hover_text("Select nodes with the same opacity")
                                .clicked()
                            {
                                action = Some(PanelAction::SelectSame {
                                    node_id: ref_id,
                                    attribute: SelectSameAttr::Opacity,
                                });
                            }
                        });
                        ui.horizontal(|ui| {
                            if ui
                                .button("Blend Mode")
                                .on_hover_text("Select nodes with the same blend mode")
                                .clicked()
                            {
                                action = Some(PanelAction::SelectSame {
                                    node_id: ref_id,
                                    attribute: SelectSameAttr::BlendMode,
                                });
                            }
                            if ui
                                .button("Object Type")
                                .on_hover_text(
                                    "Select all nodes of the same type (path/group/text)",
                                )
                                .clicked()
                            {
                                action = Some(PanelAction::SelectSame {
                                    node_id: ref_id,
                                    attribute: SelectSameAttr::ObjectType,
                                });
                            }
                        });
                    });
            }
        }

        ui.add_space(4.0);
    } else {
        // ── Document Info (shown when no node is selected) ─────────────────
        ui.label(RichText::new("Document").strong());
        ui.add_space(2.0);
        ui.label(format!(
            "Canvas: {}×{}",
            doc.width as u32, doc.height as u32
        ));
        ui.label(format!("Layers: {}", doc.layers.len()));

        // Count nodes by kind
        let mut n_path = 0usize;
        let mut n_text = 0usize;
        let mut n_group = 0usize;
        for node in doc.nodes.values() {
            match &node.kind {
                SceneNodeKind::Path(_) => n_path += 1,
                SceneNodeKind::Text(_) => n_text += 1,
                SceneNodeKind::Group(_) => n_group += 1,
            }
        }
        let total = n_path + n_text + n_group;
        ui.label(format!(
            "Nodes: {} ({} path, {} text, {} group)",
            total, n_path, n_text, n_group
        ));

        // ── Print Settings ────────────────────────────────────────────────
        ui.add_space(4.0);
        egui::CollapsingHeader::new("Print Settings")
            .default_open(false)
            .id_salt("print_settings_header")
            .show(ui, |ui| {
                ui.label(
                    RichText::new("Bleed and slug for print production.")
                        .weak()
                        .small(),
                );
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    ui.label("Bleed (mm):")
                        .on_hover_text("Extra artwork past trim edge (typically 3 mm)");
                    ui.add(
                        egui::DragValue::new(bleed_mm_input)
                            .speed(0.1)
                            .range(0.0..=25.0)
                            .suffix(" mm"),
                    );
                });
                ui.horizontal(|ui| {
                    ui.label("Slug (mm):")
                        .on_hover_text("Area outside bleed for printer marks");
                    ui.add(
                        egui::DragValue::new(slug_mm_input)
                            .speed(0.1)
                            .range(0.0..=25.0)
                            .suffix(" mm"),
                    );
                });
                ui.add_space(2.0);
                if ui.button("Apply Print Settings").clicked() {
                    action = Some(PanelAction::SetDocumentBleed {
                        bleed_mm: *bleed_mm_input,
                        slug_mm: *slug_mm_input,
                    });
                }
                if doc.bleed_mm > 0.0 || doc.slug_mm > 0.0 {
                    ui.label(
                        RichText::new(format!(
                            "Current: bleed={:.2} mm, slug={:.2} mm",
                            doc.bleed_mm, doc.slug_mm
                        ))
                        .small()
                        .weak(),
                    );
                }
            });

        // ── Artboard Margins ──────────────────────────────────────────────
        ui.add_space(4.0);
        egui::CollapsingHeader::new("Artboard Margins")
            .default_open(false)
            .id_salt("artboard_margins_header")
            .show(ui, |ui| {
                ui.label(RichText::new("Safe-area guides inside the artboard boundary.").weak().small());
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    ui.label("Top:");
                    ui.add(egui::DragValue::new(margin_top).speed(1.0).range(0.0..=2000.0).suffix(" px"));
                    ui.label("Right:");
                    ui.add(egui::DragValue::new(margin_right).speed(1.0).range(0.0..=2000.0).suffix(" px"));
                });
                ui.horizontal(|ui| {
                    ui.label("Bottom:");
                    ui.add(egui::DragValue::new(margin_bottom).speed(1.0).range(0.0..=2000.0).suffix(" px"));
                    ui.label("Left:");
                    ui.add(egui::DragValue::new(margin_left).speed(1.0).range(0.0..=2000.0).suffix(" px"));
                });
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    if ui.button("Apply Margins").clicked() {
                        action = Some(PanelAction::SetArtboardMargins {
                            top: *margin_top,
                            right: *margin_right,
                            bottom: *margin_bottom,
                            left: *margin_left,
                        });
                    }
                    if ui.small_button("Reset").clicked() {
                        *margin_top = 0.0; *margin_right = 0.0;
                        *margin_bottom = 0.0; *margin_left = 0.0;
                        action = Some(PanelAction::SetArtboardMargins {
                            top: 0.0, right: 0.0, bottom: 0.0, left: 0.0
                        });
                    }
                });
                let has_margins = doc.margin_top > 0.0 || doc.margin_right > 0.0
                    || doc.margin_bottom > 0.0 || doc.margin_left > 0.0;
                if has_margins {
                    ui.label(
                        RichText::new(format!(
                            "Current: T={:.0} R={:.0} B={:.0} L={:.0}",
                            doc.margin_top, doc.margin_right, doc.margin_bottom, doc.margin_left
                        ))
                        .small().weak(),
                    );
                    ui.add_space(2.0);
                    if ui.add_enabled(
                        selection_count > 0 || !doc.nodes.is_empty(),
                        egui::Button::new("Fit to Margins"),
                    )
                    .on_hover_text("Scale and center selected nodes (or all nodes) to fill the artboard safe area")
                    .clicked()
                    {
                        action = Some(PanelAction::FitToMargins);
                    }
                }
            });

        // ── Construction Lines ────────────────────────────────────────────
        ui.add_space(4.0);
        egui::CollapsingHeader::new("Construction Lines")
            .default_open(false)
            .id_salt("construction_lines_header")
            .show(ui, |ui| {
                ui.label(
                    RichText::new("Infinite non-printing reference lines at any angle.")
                        .weak()
                        .small(),
                );
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    ui.label("X:");
                    ui.add(egui::DragValue::new(construction_x).speed(1.0));
                    ui.label("Y:");
                    ui.add(egui::DragValue::new(construction_y).speed(1.0));
                });
                ui.horizontal(|ui| {
                    ui.label("Angle:");
                    ui.add(
                        egui::DragValue::new(construction_angle)
                            .speed(1.0)
                            .range(-360.0..=360.0)
                            .suffix("°"),
                    );
                });
                if ui
                    .button("Add Construction Line")
                    .on_hover_text("Add an infinite angled reference line (non-printing)")
                    .clicked()
                {
                    action = Some(PanelAction::AddConstructionLine {
                        x: *construction_x,
                        y: *construction_y,
                        angle_degrees: *construction_angle,
                    });
                }
                ui.add_space(2.0);
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .small_button("H (0°)")
                        .on_hover_text("Add horizontal construction line")
                        .clicked()
                    {
                        action = Some(PanelAction::AddConstructionLine {
                            x: *construction_x,
                            y: *construction_y,
                            angle_degrees: 0.0,
                        });
                    }
                    if ui
                        .small_button("V (90°)")
                        .on_hover_text("Add vertical construction line")
                        .clicked()
                    {
                        action = Some(PanelAction::AddConstructionLine {
                            x: *construction_x,
                            y: *construction_y,
                            angle_degrees: 90.0,
                        });
                    }
                    if ui
                        .small_button("D (45°)")
                        .on_hover_text("Add 45° diagonal construction line")
                        .clicked()
                    {
                        action = Some(PanelAction::AddConstructionLine {
                            x: *construction_x,
                            y: *construction_y,
                            angle_degrees: 45.0,
                        });
                    }
                    if ui
                        .small_button("D (-45°)")
                        .on_hover_text("Add -45° diagonal construction line")
                        .clicked()
                    {
                        action = Some(PanelAction::AddConstructionLine {
                            x: *construction_x,
                            y: *construction_y,
                            angle_degrees: -45.0,
                        });
                    }
                });
            });

        // ── Select by Kind buttons ────────────────────────────────────────
        ui.add_space(4.0);
        ui.label(RichText::new("Select all…").small());
        ui.horizontal_wrapped(|ui| {
            if ui
                .small_button("Paths")
                .on_hover_text("Select all path/shape nodes")
                .clicked()
            {
                action = Some(PanelAction::SelectByKind {
                    kind: "path".to_string(),
                    additive: false,
                });
            }
            if ui
                .small_button("Text")
                .on_hover_text("Select all text nodes")
                .clicked()
            {
                action = Some(PanelAction::SelectByKind {
                    kind: "text".to_string(),
                    additive: false,
                });
            }
            if ui
                .small_button("Groups")
                .on_hover_text("Select all group nodes")
                .clicked()
            {
                action = Some(PanelAction::SelectByKind {
                    kind: "group".to_string(),
                    additive: false,
                });
            }
            if ui
                .small_button("On Layer")
                .on_hover_text("Select all nodes on the active layer")
                .clicked()
            {
                action = Some(PanelAction::SelectByKind {
                    kind: "same_layer".to_string(),
                    additive: false,
                });
            }
        });

        // Unique solid fill colors as swatches
        use photonic_core::style::FillKind;
        let mut fill_colors: Vec<photonic_core::color::Color> = Vec::new();
        for node in doc.nodes.values() {
            let fill_opt = match &node.kind {
                SceneNodeKind::Path(p) => Some(&p.fill),
                SceneNodeKind::Text(t) => Some(&t.fill),
                SceneNodeKind::Group(_) => None,
            };
            if let Some(fill) = fill_opt {
                if fill.enabled {
                    if let FillKind::Solid(c) = &fill.kind {
                        let hex = c.to_hex();
                        if !fill_colors.iter().any(|existing| existing.to_hex() == hex) {
                            fill_colors.push(*c);
                        }
                    }
                }
            }
            if fill_colors.len() >= 16 {
                break;
            }
        }
        if !fill_colors.is_empty() {
            ui.add_space(4.0);
            ui.label(RichText::new("Fill colors in document:").small());

            // Click a swatch to recolor every object using that exact color,
            // with a live preview while picking. The in-progress picker state
            // lives in egui temp memory.
            let edit_id = ui.make_persistent_id("recolor_swatch_edit");
            let mut edit = ui.data(|d| d.get_temp::<RecolorSwatchEdit>(edit_id));
            let mut just_opened = false;

            ui.horizontal_wrapped(|ui| {
                for c in &fill_colors {
                    let rgba = [c.r, c.g, c.b, c.a];
                    let c32 = egui::Color32::from_rgb(
                        (c.r * 255.0).round() as u8,
                        (c.g * 255.0).round() as u8,
                        (c.b * 255.0).round() as u8,
                    );
                    let (rect, resp) =
                        ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::click());
                    ui.painter().rect_filled(rect, 2.0, c32);
                    let is_editing = edit.as_ref().map_or(false, |e| e.original == rgba);
                    if resp.hovered() || is_editing {
                        ui.painter().rect_stroke(
                            rect,
                            2.0,
                            egui::Stroke::new(2.0, egui::Color32::from_rgb(110, 86, 207)),
                        );
                    }
                    // Only start a new edit when none is active — finish the
                    // current one (Apply/Cancel/click-away) before switching.
                    if resp.clicked() && edit.is_none() {
                        let ids: Vec<NodeId> = doc
                            .nodes
                            .values()
                            .filter(|n| {
                                let solid = match &n.kind {
                                    SceneNodeKind::Path(p) if p.fill.enabled => {
                                        match &p.fill.kind {
                                            FillKind::Solid(fc) => Some(fc),
                                            _ => None,
                                        }
                                    }
                                    SceneNodeKind::Text(t) if t.fill.enabled => {
                                        match &t.fill.kind {
                                            FillKind::Solid(fc) => Some(fc),
                                            _ => None,
                                        }
                                    }
                                    _ => None,
                                };
                                solid.map_or(false, |fc| fc.to_hex() == c.to_hex())
                            })
                            .map(|n| n.id)
                            .collect();
                        edit = Some(RecolorSwatchEdit {
                            ids,
                            original: rgba,
                            applied: rgba,
                            current: rgba,
                        });
                        just_opened = true;
                    }
                    resp.on_hover_text(format!(
                        "{} — click to recolor every object using this color",
                        c.to_hex()
                    ));
                }
            });

            // Inline picker shown while a swatch is being edited.
            if let Some(e) = edit.clone() {
                let mut current = e.current;
                let mut apply = false;
                let mut cancel = false;
                let frame_resp = egui::Frame::popup(ui.style())
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new("Recolor all matching objects (live)")
                                .small()
                                .strong(),
                        );
                        let mut c32 = egui::Color32::from_rgb(
                            (current[0] * 255.0).round() as u8,
                            (current[1] * 255.0).round() as u8,
                            (current[2] * 255.0).round() as u8,
                        );
                        if egui::color_picker::color_picker_color32(
                            ui,
                            &mut c32,
                            egui::color_picker::Alpha::Opaque,
                        ) {
                            current = [
                                c32.r() as f32 / 255.0,
                                c32.g() as f32 / 255.0,
                                c32.b() as f32 / 255.0,
                                e.original[3],
                            ];
                        }
                        ui.horizontal(|ui| {
                            if ui.button("Apply").clicked() {
                                apply = true;
                            }
                            if ui.button("Cancel").clicked() {
                                cancel = true;
                            }
                        });
                    })
                    .response;

                // Clicking outside the picker (e.g. another swatch, empty space)
                // keeps the change — same as Apply. Ignore the click that opened it.
                let click_away = !just_opened && frame_resp.clicked_elsewhere();

                if apply || click_away {
                    action = Some(PanelAction::RecolorCommit {
                        ids: e.ids.clone(),
                        from: e.original,
                        to: current,
                    });
                    edit = None;
                } else if cancel {
                    // Revert the live preview, no history entry.
                    if e.applied != e.original {
                        action = Some(PanelAction::RecolorPreview {
                            ids: e.ids.clone(),
                            to: e.original,
                        });
                    }
                    edit = None;
                } else {
                    // Live preview: push the new color whenever it changed.
                    if current != e.applied {
                        action = Some(PanelAction::RecolorPreview {
                            ids: e.ids.clone(),
                            to: current,
                        });
                    }
                    edit = Some(RecolorSwatchEdit {
                        ids: e.ids,
                        original: e.original,
                        applied: current,
                        current,
                    });
                }
            }

            // Persist or clear the picker state for next frame.
            match edit {
                Some(e) => ui.data_mut(|d| d.insert_temp(edit_id, e)),
                None => ui.data_mut(|d| d.remove::<RecolorSwatchEdit>(edit_id)),
            }
        }
        ui.add_space(4.0);
    }

    // ── Boolean operations (visible when exactly 2 path nodes are selected) ──
    if selection_count == 2 && matches("Boolean Operations") {
        egui::CollapsingHeader::new("Boolean Operations")
            .default_open(true)
            .open(forced_open)
            .show(ui, |ui| {
                ui.label(
                    RichText::new("lower z = target, upper z = tool")
                        .weak()
                        .small(),
                );
                ui.horizontal(|ui| {
                    if ui
                        .button("Union")
                        .on_hover_text("Merge both shapes")
                        .clicked()
                    {
                        action = Some(PanelAction::BooleanOp(BooleanOp::Union));
                    }
                    if ui
                        .button("Subtract")
                        .on_hover_text("Cut upper shape from lower")
                        .clicked()
                    {
                        action = Some(PanelAction::BooleanOp(BooleanOp::Subtract));
                    }
                });
                ui.horizontal(|ui| {
                    if ui
                        .button("Intersect")
                        .on_hover_text("Keep only the overlapping area")
                        .clicked()
                    {
                        action = Some(PanelAction::BooleanOp(BooleanOp::Intersect));
                    }
                    if ui
                        .button("Exclude")
                        .on_hover_text("Remove the overlapping area")
                        .clicked()
                    {
                        action = Some(PanelAction::BooleanOp(BooleanOp::Exclude));
                    }
                });
                if ui
                    .button("Join Paths")
                    .on_hover_text(
                        "Connect the nearest endpoints of both paths into a single merged path",
                    )
                    .clicked()
                {
                    action = Some(PanelAction::JoinPaths { node_ids: vec![] });
                }
            });
        ui.add_space(4.0);
    }

    // ── Blend (visible when exactly 2 nodes selected) ─────────────────────────
    if selection_count == 2 && matches("Blend") {
        egui::CollapsingHeader::new("Blend")
            .default_open(false)
            .open(forced_open)
            .show(ui, |ui| {
                ui.label(RichText::new("Generate intermediate steps between two paths").weak().small());
                if ui.button("Blend (5 steps)")
                    .on_hover_text("Create 5 interpolated shapes between the two selected paths")
                    .clicked()
                {
                    let ids: Vec<NodeId> = doc.selection.node_ids.iter().copied().collect();
                    if ids.len() == 2 {
                        action = Some(PanelAction::BlendObjects {
                            node_id_a: ids[0],
                            node_id_b: ids[1],
                            steps: 5,
                        });
                    }
                }
                if ui.button("Blend (Smooth Color)")
                    .on_hover_text("Auto-compute steps so each step changes color by ≤ 1/255 (Smooth Color mode)")
                    .clicked()
                {
                    let ids: Vec<NodeId> = doc.selection.node_ids.iter().copied().collect();
                    if ids.len() == 2 {
                        action = Some(PanelAction::BlendObjectsSmoothColor {
                            node_id_a: ids[0],
                            node_id_b: ids[1],
                        });
                    }
                }
                if ui.button("Blend (32 px spacing)")
                    .on_hover_text("Space blend steps 32 px apart along the line between the two shapes (Specified Distance mode)")
                    .clicked()
                {
                    let ids: Vec<NodeId> = doc.selection.node_ids.iter().copied().collect();
                    if ids.len() == 2 {
                        action = Some(PanelAction::BlendObjectsSpacing {
                            node_id_a: ids[0],
                            node_id_b: ids[1],
                            spacing: 32.0,
                        });
                    }
                }
            });
        ui.add_space(4.0);
    }

    // ── Pathfinder operations (visible when 2+ nodes selected) ───────────────
    if selection_count >= 2 && matches("Pathfinder") {
        egui::CollapsingHeader::new("Pathfinder")
            .default_open(false)
            .open(forced_open)
            .show(ui, |ui| {
                ui.label(RichText::new("Multi-object operations — frontmost = crop/subtract mask").weak().small());
                if ui.button("Crop")
                    .on_hover_text("Clip all selected shapes to the boundary of the frontmost shape; frontmost is removed")
                    .clicked()
                {
                    action = Some(PanelAction::PathfinderCrop { node_ids: vec![] });
                }
                if ui.button("Minus Back")
                    .on_hover_text("Subtract all back shapes from the frontmost shape; back shapes are removed")
                    .clicked()
                {
                    action = Some(PanelAction::PathfinderMinusBack { node_ids: vec![] });
                }
                if ui.button("Minus Front")
                    .on_hover_text("Punch the frontmost shape out of all back shapes; frontmost is removed")
                    .clicked()
                {
                    action = Some(PanelAction::PathfinderMinusFront { node_ids: vec![] });
                }
                if ui.button("Trim")
                    .on_hover_text("Remove hidden areas from each shape (parts covered by shapes above); strokes disabled")
                    .clicked()
                {
                    action = Some(PanelAction::PathfinderTrim { node_ids: vec![] });
                }
                if ui.button("Merge")
                    .on_hover_text("Trim hidden areas, then merge shapes that share the same fill color into one; strokes disabled")
                    .clicked()
                {
                    action = Some(PanelAction::PathfinderMerge { node_ids: vec![] });
                }
                if ui.button("Outline")
                    .on_hover_text("Convert fills to stroked outlines; fill color becomes stroke color, fill removed")
                    .clicked()
                {
                    action = Some(PanelAction::PathfinderOutline { node_ids: vec![] });
                }
                if selection_count == 2 {
                    if ui.button("Divide")
                        .on_hover_text("Split two shapes at every overlap edge into distinct colored face nodes")
                        .clicked()
                    {
                        action = Some(PanelAction::PathfinderDivide { node_ids: vec![] });
                    }
                }
            });
        ui.add_space(4.0);
    }

    // ── Distribute on Path (visible when 2+ nodes selected) ─────────────────
    if selection_count >= 2 && matches("Distribute on Path") {
        egui::CollapsingHeader::new("Distribute on Path")
            .default_open(false)
            .open(forced_open)
            .show(ui, |ui| {
                ui.label(RichText::new("Place copies of the selected objects along the guide path.").weak().small());
                ui.label(RichText::new("The frontmost selected path is used as the guide; all others are the objects to distribute.").weak().small());
                if ui.button("Distribute on Path")
                    .on_hover_text("Evenly place copies of selected nodes along the frontmost selected path")
                    .clicked()
                {
                    // Pass empty vecs — app.rs resolves from doc.selection.
                    action = Some(PanelAction::DistributeOnPath {
                        path_node_id: uuid::Uuid::nil(),
                        node_ids: vec![],
                        align: false,
                    });
                }
                if ui.button("Distribute + Align")
                    .on_hover_text("Same as above but rotates each copy to face along the path's tangent direction")
                    .clicked()
                {
                    action = Some(PanelAction::DistributeOnPath {
                        path_node_id: uuid::Uuid::nil(),
                        node_ids: vec![],
                        align: true,
                    });
                }
            });
        ui.add_space(4.0);
    }

    // ── Compound Path (visible when 2+ nodes selected, or 1 compound selected) ──
    let is_compound_selected = selected_node
        .and_then(|n| {
            if let photonic_core::node::SceneNodeKind::Path(ref p) = n.kind {
                Some(p.is_compound)
            } else {
                None
            }
        })
        .unwrap_or(false);
    let show_compound = (selection_count >= 2 || is_compound_selected) && matches("Compound Path");
    if show_compound {
        egui::CollapsingHeader::new("Compound Path")
            .default_open(true)
            .open(forced_open)
            .show(ui, |ui| {
                if selection_count >= 2 {
                    if ui.button("Make Compound Path")
                        .on_hover_text("Combine selected paths into one shape; overlapping areas create holes (even-odd fill rule)")
                        .clicked()
                    {
                        action = Some(PanelAction::MakeCompoundPath { node_ids: vec![] });
                    }
                }
                if is_compound_selected {
                    if let Some(nid) = selected_id {
                        if ui.button("Release Compound Path")
                            .on_hover_text("Split the compound path back into individual path nodes")
                            .clicked()
                        {
                            action = Some(PanelAction::ReleaseCompoundPath { node_id: nid });
                        }
                    }
                }
            });
        ui.add_space(4.0);
    }

    // ── Clipping Mask (visible when a Group node is selected) ─────────────────
    let is_group_selected = selected_node
        .map(|n| matches!(n.kind, photonic_core::node::SceneNodeKind::Group(_)))
        .unwrap_or(false);
    let has_clip_mask = selected_node
        .and_then(|n| {
            if let photonic_core::node::SceneNodeKind::Group(ref g) = n.kind {
                Some(g.clip_node_id.is_some())
            } else {
                None
            }
        })
        .unwrap_or(false);
    if is_group_selected && matches("Clipping Mask") {
        if let Some(gid) = selected_id {
            egui::CollapsingHeader::new("Clipping Mask")
                .default_open(true)
                .open(forced_open)
                .show(ui, |ui| {
                    if !has_clip_mask {
                        ui.label(RichText::new("Topmost child will become the clip path.").weak().small());
                        if ui.button("Make Clipping Mask")
                            .on_hover_text("Use the topmost child of this group as a clipping path for all other children")
                            .clicked()
                        {
                            action = Some(PanelAction::MakeClippingMask { group_id: gid });
                        }
                    } else {
                        ui.label(RichText::new("Clipping mask active.").small());
                        if ui.button("Release Clipping Mask")
                            .on_hover_text("Remove the clipping mask; all children revert to normal visible objects")
                            .clicked()
                        {
                            action = Some(PanelAction::ReleaseClippingMask { group_id: gid });
                        }
                    }
                });
            ui.add_space(4.0);
        }
    }

    // ── Blend Colors (visible when 3+ nodes selected) ────────────────────────
    if selection_count >= 3 && matches("Blend Colors") {
        egui::CollapsingHeader::new("Blend Colors")
            .default_open(true)
            .open(forced_open)
            .show(ui, |ui| {
                ui.label(
                    RichText::new("Interpolate fill colors from first → last node")
                        .weak()
                        .small(),
                );
                ui.horizontal(|ui| {
                    if ui
                        .button("Horizontal")
                        .on_hover_text("Sort left→right by bounding-box center X, then blend")
                        .clicked()
                    {
                        action = Some(PanelAction::BlendColors {
                            node_ids: vec![],
                            direction: "horizontal".to_string(),
                        });
                    }
                    if ui
                        .button("Vertical")
                        .on_hover_text("Sort top→bottom by bounding-box center Y, then blend")
                        .clicked()
                    {
                        action = Some(PanelAction::BlendColors {
                            node_ids: vec![],
                            direction: "vertical".to_string(),
                        });
                    }
                    if ui
                        .button("By Depth")
                        .on_hover_text("Sort bottom→top by z-order, then blend")
                        .clicked()
                    {
                        action = Some(PanelAction::BlendColors {
                            node_ids: vec![],
                            direction: "depth".to_string(),
                        });
                    }
                });
            });
        ui.add_space(4.0);
    }

    // ── Adjust Colors (visible when 1+ nodes selected) ───────────────────────
    if selection_count >= 1 && matches("Adjust Colors") {
        egui::CollapsingHeader::new("Adjust Colors")
            .default_open(false)
            .open(forced_open)
            .show(ui, |ui| {
                ui.label(RichText::new("Shift RGB(A) channel values").weak().small());

                let id_r = ui.id().with("adj_r");
                let id_g = ui.id().with("adj_g");
                let id_b = ui.id().with("adj_b");
                let id_a = ui.id().with("adj_a");

                let mut dr: f32 = ui.data(|d| d.get_temp(id_r).unwrap_or(0.0));
                let mut dg: f32 = ui.data(|d| d.get_temp(id_g).unwrap_or(0.0));
                let mut db: f32 = ui.data(|d| d.get_temp(id_b).unwrap_or(0.0));
                let mut da: f32 = ui.data(|d| d.get_temp(id_a).unwrap_or(0.0));

                ui.add(
                    egui::Slider::new(&mut dr, -1.0_f32..=1.0)
                        .text("R")
                        .step_by(0.01),
                );
                ui.add(
                    egui::Slider::new(&mut dg, -1.0_f32..=1.0)
                        .text("G")
                        .step_by(0.01),
                );
                ui.add(
                    egui::Slider::new(&mut db, -1.0_f32..=1.0)
                        .text("B")
                        .step_by(0.01),
                );
                ui.add(
                    egui::Slider::new(&mut da, -1.0_f32..=1.0)
                        .text("A")
                        .step_by(0.01),
                );

                ui.data_mut(|d| {
                    d.insert_temp(id_r, dr);
                    d.insert_temp(id_g, dg);
                    d.insert_temp(id_b, db);
                    d.insert_temp(id_a, da);
                });

                ui.horizontal(|ui| {
                    if ui
                        .button("Apply")
                        .on_hover_text("Apply channel adjustments to selected nodes")
                        .clicked()
                    {
                        action = Some(PanelAction::AdjustColors {
                            node_ids: vec![],
                            delta_r: dr,
                            delta_g: dg,
                            delta_b: db,
                            delta_a: da,
                        });
                    }
                    if ui.button("Reset").clicked() {
                        ui.data_mut(|d| {
                            d.insert_temp(id_r, 0.0_f32);
                            d.insert_temp(id_g, 0.0_f32);
                            d.insert_temp(id_b, 0.0_f32);
                            d.insert_temp(id_a, 0.0_f32);
                        });
                    }
                });
            });
        ui.add_space(4.0);
    }

    // ── Flatten Transparency ──────────────────────────────────────────────────
    if selection_count >= 1 && matches("Flatten Transparency") {
        egui::CollapsingHeader::new("Flatten Transparency")
            .default_open(false)
            .open(forced_open)
            .show(ui, |ui| {
                ui.label(RichText::new("Bake opacity into color alphas for print-ready output.").weak().small());
                if ui.button("Flatten Transparency")
                    .on_hover_text("Premultiply node and fill opacity into color alpha values, then set opacity to 1.0")
                    .clicked()
                {
                    action = Some(PanelAction::FlattenTransparency);
                }
            });
        ui.add_space(4.0);
    }

    // ── Copy Appearance (visible when 2+ nodes selected) ─────────────────────
    if selection_count >= 2 && matches("Copy Appearance") {
        thread_local! {
            static COPY_FILL: std::cell::RefCell<bool> = std::cell::RefCell::new(true);
            static COPY_STROKE: std::cell::RefCell<bool> = std::cell::RefCell::new(true);
            static COPY_OPACITY: std::cell::RefCell<bool> = std::cell::RefCell::new(false);
        }
        egui::CollapsingHeader::new("Copy Appearance")
            .default_open(false)
            .open(forced_open)
            .show(ui, |ui| {
                ui.label(
                    RichText::new("Source: first selected  →  all others")
                        .weak()
                        .small(),
                );
                COPY_FILL.with(|cf| {
                    COPY_STROKE.with(|cs| {
                        COPY_OPACITY.with(|co| {
                            let mut fill = *cf.borrow();
                            let mut stroke = *cs.borrow();
                            let mut opacity = *co.borrow();
                            ui.horizontal(|ui| {
                                ui.checkbox(&mut fill, "Fill");
                                ui.checkbox(&mut stroke, "Stroke");
                                ui.checkbox(&mut opacity, "Opacity");
                            });
                            *cf.borrow_mut() = fill;
                            *cs.borrow_mut() = stroke;
                            *co.borrow_mut() = opacity;
                            if ui
                                .add_enabled(
                                    fill || stroke || opacity,
                                    egui::Button::new("Apply Eyedropper"),
                                )
                                .on_hover_text(
                                    "Copy selected attributes from the first node to all others",
                                )
                                .clicked()
                            {
                                if let Some(src) = selected_ids.first().copied() {
                                    let targets: Vec<NodeId> =
                                        selected_ids.iter().skip(1).copied().collect();
                                    if !targets.is_empty() {
                                        action = Some(PanelAction::CopyAppearance {
                                            source_id: src,
                                            target_ids: targets,
                                            copy_fill: fill,
                                            copy_stroke: stroke,
                                            copy_opacity: opacity,
                                        });
                                    }
                                }
                            }
                        })
                    })
                });
            });
        ui.add_space(4.0);
    }

    // ── Alignment (visible when 2+ nodes selected) ───────────────────────────
    if selection_count >= 2 && matches("Alignment") {
        egui::CollapsingHeader::new("Alignment")
            .default_open(false)
            .open(forced_open)
            .show(ui, |ui| {
                ui.label(
                    RichText::new("Align relative to selection bounds")
                        .weak()
                        .small(),
                );
                ui.horizontal(|ui| {
                    if ui
                        .button("Left")
                        .on_hover_text("Align left edges")
                        .clicked()
                    {
                        action = Some(PanelAction::AlignNodes {
                            operation: "left".into(),
                            key_object_id: None,
                        });
                    }
                    if ui
                        .button("Center H")
                        .on_hover_text("Align horizontal centers")
                        .clicked()
                    {
                        action = Some(PanelAction::AlignNodes {
                            operation: "center_horizontal".into(),
                            key_object_id: None,
                        });
                    }
                    if ui
                        .button("Right")
                        .on_hover_text("Align right edges")
                        .clicked()
                    {
                        action = Some(PanelAction::AlignNodes {
                            operation: "right".into(),
                            key_object_id: None,
                        });
                    }
                });
                ui.horizontal(|ui| {
                    if ui.button("Top").on_hover_text("Align top edges").clicked() {
                        action = Some(PanelAction::AlignNodes {
                            operation: "top".into(),
                            key_object_id: None,
                        });
                    }
                    if ui
                        .button("Center V")
                        .on_hover_text("Align vertical centers")
                        .clicked()
                    {
                        action = Some(PanelAction::AlignNodes {
                            operation: "center_vertical".into(),
                            key_object_id: None,
                        });
                    }
                    if ui
                        .button("Bottom")
                        .on_hover_text("Align bottom edges")
                        .clicked()
                    {
                        action = Some(PanelAction::AlignNodes {
                            operation: "bottom".into(),
                            key_object_id: None,
                        });
                    }
                });
                ui.add_space(2.0);
                ui.label(RichText::new("Distribute").weak().small());
                ui.horizontal(|ui| {
                    if ui
                        .button("Dist H")
                        .on_hover_text("Distribute horizontal spacing evenly")
                        .clicked()
                    {
                        action = Some(PanelAction::AlignNodes {
                            operation: "distribute_horizontal".into(),
                            key_object_id: None,
                        });
                    }
                    if ui
                        .button("Dist V")
                        .on_hover_text("Distribute vertical spacing evenly")
                        .clicked()
                    {
                        action = Some(PanelAction::AlignNodes {
                            operation: "distribute_vertical".into(),
                            key_object_id: None,
                        });
                    }
                });
                // Key Object: when a single node ID is provided (selected_id),
                // expose buttons to align the rest of the selection to it.
                if let Some(key_id) = selected_id {
                    ui.add_space(2.0);
                    ui.separator();
                    ui.label(
                        RichText::new("Align to Key Object (primary selection)")
                            .weak()
                            .small(),
                    );
                    ui.horizontal(|ui| {
                        for (label, op) in [
                            ("Left", "left"),
                            ("Ctr H", "center_horizontal"),
                            ("Right", "right"),
                        ] {
                            if ui
                                .button(label)
                                .on_hover_text(format!("Align {} to key object", label))
                                .clicked()
                            {
                                action = Some(PanelAction::AlignNodes {
                                    operation: op.into(),
                                    key_object_id: Some(key_id),
                                });
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        for (label, op) in [
                            ("Top", "top"),
                            ("Ctr V", "center_vertical"),
                            ("Bot", "bottom"),
                        ] {
                            if ui
                                .button(label)
                                .on_hover_text(format!("Align {} to key object", label))
                                .clicked()
                            {
                                action = Some(PanelAction::AlignNodes {
                                    operation: op.into(),
                                    key_object_id: Some(key_id),
                                });
                            }
                        }
                    });
                }
            });
        ui.add_space(4.0);
    }

    // ── Distribute No Overlap (visible when 2+ nodes selected) ──────────────
    if selection_count >= 2 && matches("Distribute No Overlap") {
        egui::CollapsingHeader::new("Distribute No Overlap")
            .default_open(false)
            .open(forced_open)
            .show(ui, |ui| {
                ui.label(
                    RichText::new("Push selected nodes apart until no bounding boxes overlap.")
                        .weak()
                        .small(),
                );
                if ui
                    .button("Distribute (No Overlap)")
                    .on_hover_text(
                        "Iteratively push nodes apart until no bounding boxes overlap (4 px gap)",
                    )
                    .clicked()
                {
                    let ids: Vec<NodeId> = doc.selection.node_ids.iter().copied().collect();
                    action = Some(PanelAction::DistributeNoOverlap { node_ids: ids });
                }
            });
        ui.add_space(4.0);
    }

    // ── Align to Artboard (visible when 1+ nodes selected) ───────────────────
    if selection_count >= 1 && matches("Align to Artboard") {
        egui::CollapsingHeader::new("Align to Artboard")
            .default_open(false)
            .open(forced_open)
            .show(ui, |ui| {
                ui.label(
                    RichText::new("Align relative to document canvas")
                        .weak()
                        .small(),
                );
                ui.horizontal(|ui| {
                    if ui
                        .button("Left")
                        .on_hover_text("Align left edge to artboard left")
                        .clicked()
                    {
                        action = Some(PanelAction::AlignToArtboard {
                            operation: "left".into(),
                        });
                    }
                    if ui
                        .button("Center H")
                        .on_hover_text("Center horizontally on artboard")
                        .clicked()
                    {
                        action = Some(PanelAction::AlignToArtboard {
                            operation: "center_horizontal".into(),
                        });
                    }
                    if ui
                        .button("Right")
                        .on_hover_text("Align right edge to artboard right")
                        .clicked()
                    {
                        action = Some(PanelAction::AlignToArtboard {
                            operation: "right".into(),
                        });
                    }
                });
                ui.horizontal(|ui| {
                    if ui
                        .button("Top")
                        .on_hover_text("Align top edge to artboard top")
                        .clicked()
                    {
                        action = Some(PanelAction::AlignToArtboard {
                            operation: "top".into(),
                        });
                    }
                    if ui
                        .button("Center V")
                        .on_hover_text("Center vertically on artboard")
                        .clicked()
                    {
                        action = Some(PanelAction::AlignToArtboard {
                            operation: "center_vertical".into(),
                        });
                    }
                    if ui
                        .button("Bottom")
                        .on_hover_text("Align bottom edge to artboard bottom")
                        .clicked()
                    {
                        action = Some(PanelAction::AlignToArtboard {
                            operation: "bottom".into(),
                        });
                    }
                });
            });
        ui.add_space(4.0);
    }

    // ── Layer Operations (visible when 2+ nodes selected) ────────────────────
    if selection_count >= 2 && matches("Layer Operations") {
        egui::CollapsingHeader::new("Layer Operations")
            .default_open(false)
            .open(forced_open)
            .show(ui, |ui| {
                if ui
                    .button("Release to Layers")
                    .on_hover_text("Move each selected node into its own newly created layer")
                    .clicked()
                {
                    action = Some(PanelAction::ReleaseToLayers { node_ids: vec![] });
                }
            });
        ui.add_space(4.0);
    }

    // ── Tool / shape options ──────────────────────────────────────────────────
    if matches("New Shape Fill") {
        egui::CollapsingHeader::new("New Shape Fill")
            .default_open(false)
            .open(forced_open)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.color_edit_button_rgba_unmultiplied(fill_color);
                    if eyedropper_btn(ui) {
                        action = Some(PanelAction::StartEyedropper(EyedropperTarget::NewShapeFill));
                    }
                });
            });
    }

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
        | Tool::MagicWand => {
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
                                ui.label(RichText::new("Click shape → enter point edit").weak().small());
                                ui.label(RichText::new("Click point → select anchor").weak().small());
                                ui.label(RichText::new("Drag point → move anchor").weak().small());
                                ui.label(RichText::new("Ctrl+click → multi-select").weak().small());
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
                                .small_button("✕")
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
                            if ui.small_button("✕").clicked() {
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
            });
        ui.add_space(4.0);
    }

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
                            if ui.small_button("✕").clicked() {
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
                            if ui.small_button("✕").clicked() {
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
                                .small_button("✕")
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
                                    "{} (avg {:.1}px)",
                                    wp.name,
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
                            if ui.small_button("✕").clicked() {
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

    // ── Distances ─────────────────────────────────────────────────────────────
    if selection_count >= 2 && matches("Distances") {
        egui::CollapsingHeader::new("Distances")
            .default_open(true)
            .open(forced_open)
            .show(ui, |ui| {
                ui.label(
                    RichText::new("Edge gaps and center distances between selected nodes.")
                        .weak()
                        .small(),
                );
                ui.add_space(2.0);
                if ui.button("Measure Selected").clicked() {
                    action = Some(PanelAction::MeasureDistances {
                        node_ids: selected_ids.to_vec(),
                    });
                }
                if !distance_results.is_empty() {
                    ui.add_space(4.0);
                    for (from, to, h_gap, v_gap, center_dist) in distance_results {
                        ui.label(
                            RichText::new(format!(
                                "{} → {}: H gap {:.1}px, V gap {:.1}px, C→C {:.1}px",
                                from, to, h_gap, v_gap, center_dist
                            ))
                            .small(),
                        );
                    }
                }
            });
        ui.add_space(4.0);
    }

    // ── Dimension Annotations ─────────────────────────────────────────────────
    if (selection_count == 2 || !doc.dimensions.is_empty()) && matches("Dimension Annotations") {
        egui::CollapsingHeader::new("Dimension Annotations")
            .default_open(true)
            .open(forced_open)
            .show(ui, |ui| {
                ui.label(
                    RichText::new("Add measurement lines between nodes.")
                        .weak()
                        .small(),
                );
                ui.add_space(2.0);
                if selection_count == 2 {
                    let from_id = selected_ids[0];
                    let to_id = selected_ids[1];
                    ui.horizontal(|ui| {
                        if ui
                            .small_button("Add ↔ H")
                            .on_hover_text("Add horizontal (X-axis) dimension")
                            .clicked()
                        {
                            action = Some(PanelAction::AddDimension {
                                from_id,
                                to_id,
                                axis: "x".to_string(),
                            });
                        }
                        if ui
                            .small_button("Add ↕ V")
                            .on_hover_text("Add vertical (Y-axis) dimension")
                            .clicked()
                        {
                            action = Some(PanelAction::AddDimension {
                                from_id,
                                to_id,
                                axis: "y".to_string(),
                            });
                        }
                        if ui
                            .small_button("Add ↗ D")
                            .on_hover_text("Add diagonal (Euclidean) dimension")
                            .clicked()
                        {
                            action = Some(PanelAction::AddDimension {
                                from_id,
                                to_id,
                                axis: "diagonal".to_string(),
                            });
                        }
                    });
                }
                if !doc.dimensions.is_empty() {
                    ui.add_space(4.0);
                    let to_remove: Option<uuid::Uuid> = {
                        let mut remove_id: Option<uuid::Uuid> = None;
                        for dim in &doc.dimensions {
                            ui.horizontal(|ui| {
                                let label = format!("{} axis: {:.1}px", dim.axis, dim.distance());
                                ui.label(RichText::new(&label).small());
                                if ui
                                    .small_button("✕")
                                    .on_hover_text("Remove this dimension")
                                    .clicked()
                                {
                                    remove_id = Some(dim.id);
                                }
                            });
                        }
                        remove_id
                    };
                    if let Some(id) = to_remove {
                        action = Some(PanelAction::RemoveDimension { id });
                    }
                }
            });
        ui.add_space(4.0);
    }

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
                            if ui.small_button("✕").on_hover_text("Delete rule").clicked() {
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
                        let icon = if *passed { "✓" } else { "✗" };
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
                                .small_button("✕")
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

    // ── Edit History ──────────────────────────────────────────────────────────
    if matches("History") {
        egui::CollapsingHeader::new("Edit History")
            .default_open(false)
            .id_salt("history_panel")
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("Edit history ({} steps):", history_total))
                            .weak()
                            .small(),
                    );
                    if ui
                        .small_button("⟳")
                        .on_hover_text("Refresh history list")
                        .clicked()
                    {
                        action = Some(PanelAction::RefreshHistory);
                    }
                });
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
                        if ui.small_button("✕").clicked() {
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
                                .small_button("✕")
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
                                .small_button("✕")
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
                            if ui.small_button("✕").clicked() {
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

    // ── Text node: Variable Binding (shown when a text node is selected) ──────
    if let Some(node) = selected_node {
        if let SceneNodeKind::Text(ref tn) = node.kind {
            let text_nid = node.id;
            if !doc.variables.is_empty() && matches("Variable Binding") {
                egui::CollapsingHeader::new("Variable Binding")
                    .default_open(true)
                    .open(forced_open)
                    .show(ui, |ui| {
                        if let Some(ref binding) = tn.variable_binding {
                            ui.label(RichText::new(format!("Bound to: {}", binding)).small());
                            if ui.small_button("Unbind").clicked() {
                                action =
                                    Some(PanelAction::UnbindTextVariable { node_id: text_nid });
                            }
                        } else {
                            ui.label(RichText::new("Bind to variable:").small().weak());
                            for var in &doc.variables {
                                if ui
                                    .small_button(&var.name)
                                    .on_hover_text(format!(
                                        "Bind this text node to '{}' (current value: {})",
                                        var.name, var.value
                                    ))
                                    .clicked()
                                {
                                    action = Some(PanelAction::BindTextVariable {
                                        node_id: text_nid,
                                        variable_name: var.name.clone(),
                                    });
                                }
                            }
                        }
                    });
                ui.add_space(2.0);
            }
        }
    }

    // ── Symbol Instance Overrides ────────────────────────────────────────────
    if let (Some(node), Some(nid)) = (selected_node, selected_id) {
        if node.symbol_ref.is_some() && matches("Symbol Override") {
            egui::CollapsingHeader::new("Symbol Override")
                .default_open(true)
                .id_salt("sym_override_panel")
                .show(ui, |ui| {
                    ui.label(RichText::new("Per-instance color overrides (Dynamic Symbol).").weak().small());
                    let fill_disp = node.symbol_fill_override.as_deref().unwrap_or("(master)");
                    let stroke_disp = node.symbol_stroke_override.as_deref().unwrap_or("(master)");
                    ui.label(RichText::new(format!("Fill: {}  Stroke: {}", fill_disp, stroke_disp)).small());
                    ui.separator();
                    thread_local! {
                        static FILL_HEX: std::cell::RefCell<String> = std::cell::RefCell::new(String::new());
                        static STROKE_HEX: std::cell::RefCell<String> = std::cell::RefCell::new(String::new());
                    }
                    ui.horizontal(|ui| {
                        ui.label("Fill:");
                        FILL_HEX.with(|s| {
                            let mut val = s.borrow().clone();
                            ui.add(egui::TextEdit::singleline(&mut val).hint_text("#rrggbb").desired_width(70.0));
                            *s.borrow_mut() = val;
                        });
                        ui.label("Stroke:");
                        STROKE_HEX.with(|s| {
                            let mut val = s.borrow().clone();
                            ui.add(egui::TextEdit::singleline(&mut val).hint_text("#rrggbb").desired_width(70.0));
                            *s.borrow_mut() = val;
                        });
                    });
                    ui.horizontal(|ui| {
                        if ui.button("Apply Override")
                            .on_hover_text("Apply fill/stroke color overrides to this instance")
                            .clicked()
                        {
                            let fill_val = FILL_HEX.with(|s| s.borrow().clone());
                            let stroke_val = STROKE_HEX.with(|s| s.borrow().clone());
                            let fill_opt = if fill_val.trim().is_empty() { None } else { Some(fill_val.trim().to_string()) };
                            let stroke_opt = if stroke_val.trim().is_empty() { None } else { Some(stroke_val.trim().to_string()) };
                            if fill_opt.is_some() || stroke_opt.is_some() {
                                action = Some(PanelAction::SetSymbolOverride {
                                    node_id: nid,
                                    fill_hex: fill_opt,
                                    stroke_hex: stroke_opt,
                                });
                            }
                        }
                        if node.symbol_fill_override.is_some() || node.symbol_stroke_override.is_some() {
                            if ui.button("Clear Override")
                                .on_hover_text("Reset this instance to master fill/stroke")
                                .clicked()
                            {
                                action = Some(PanelAction::ClearSymbolOverrides { node_id: nid });
                            }
                        }
                    });
                });
            ui.add_space(2.0);
        }
    }

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

    action
}

// ─── Fill editor ─────────────────────────────────────────────────────────────

/// Discriminant used by the UI to select gradient type (avoids cloning the full kind).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FillType {
    Solid,
    Linear,
    Radial,
    Fluid,
    Mesh,
}

/// Render a small eyedropper icon button. Returns `true` when clicked.
fn eyedropper_btn(ui: &mut Ui) -> bool {
    ui.small_button(ph::EYEDROPPER)
        .on_hover_text("Pick color from screen (Esc to cancel)")
        .clicked()
}

/// Draw a compact fill editor for a path node's fill.
/// Returns `Some(new_fill)` if the user changed anything.
/// Sets `*dropper` to the chosen slot when the eyedropper button is clicked.
fn draw_fill_editor(ui: &mut Ui, fill: &Fill, dropper: &mut Option<FillColorSlot>) -> Option<Fill> {
    use photonic_core::style::FillKind;

    let current_type = match &fill.kind {
        FillKind::None | FillKind::Solid(_) => FillType::Solid,
        FillKind::Gradient(g) => match g.kind {
            GradientKind::Linear => FillType::Linear,
            GradientKind::Radial => FillType::Radial,
        },
        FillKind::FluidGradient(_) => FillType::Fluid,
        FillKind::MeshGradient(_) => FillType::Mesh,
    };

    let mut chosen_type = current_type;
    let mut changed = false;

    // Fill type selector
    ui.horizontal(|ui| {
        for (label, t) in [
            ("Solid", FillType::Solid),
            ("Linear", FillType::Linear),
            ("Radial", FillType::Radial),
            ("Fluid", FillType::Fluid),
            ("Mesh", FillType::Mesh),
        ] {
            if ui.selectable_label(chosen_type == t, label).clicked() {
                chosen_type = t;
                changed = true;
            }
        }
    });

    // If type changed, build a default fill for the new type
    if changed && chosen_type != current_type {
        // Inherit the first colour from the current fill where possible
        let base_color = match &fill.kind {
            FillKind::Solid(c) => *c,
            FillKind::Gradient(g) => g.stops.first().map(|s| s.color).unwrap_or(Color::BLACK),
            FillKind::FluidGradient(fg) => {
                fg.points.first().map(|p| p.color).unwrap_or(Color::BLACK)
            }
            FillKind::MeshGradient(mg) => {
                mg.vertices.first().map(|v| v.color).unwrap_or(Color::BLACK)
            }
            FillKind::None => Color::BLACK,
        };
        let white = Color {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 1.0,
        };
        let new_fill = match chosen_type {
            FillType::Solid => Fill::solid(base_color),
            FillType::Linear => Fill::gradient(Gradient::linear(
                0.0,
                0.0,
                200.0,
                0.0,
                vec![
                    GradientStop::new(0.0, base_color),
                    GradientStop::new(1.0, white),
                ],
            )),
            FillType::Radial => Fill::gradient(Gradient::radial(
                100.0,
                100.0,
                100.0,
                vec![
                    GradientStop::new(0.0, base_color),
                    GradientStop::new(1.0, white),
                ],
            )),
            FillType::Fluid => Fill::fluid_gradient(FluidGradient::new(vec![
                FluidGradientPoint::new(50.0, 50.0, base_color),
                FluidGradientPoint::new(150.0, 50.0, white),
                FluidGradientPoint::new(
                    100.0,
                    150.0,
                    Color {
                        r: 1.0,
                        g: 0.5,
                        b: 0.0,
                        a: 1.0,
                    },
                ),
            ])),
            FillType::Mesh => {
                let r = Color {
                    r: 1.0,
                    g: 0.2,
                    b: 0.2,
                    a: 1.0,
                };
                let g = Color {
                    r: 0.2,
                    g: 1.0,
                    b: 0.2,
                    a: 1.0,
                };
                let b = Color {
                    r: 0.2,
                    g: 0.2,
                    b: 1.0,
                    a: 1.0,
                };
                Fill::mesh_gradient(MeshGradient::new(
                    2,
                    2,
                    vec![
                        MeshGradientVertex::new(0.0, 0.0, base_color),
                        MeshGradientVertex::new(200.0, 0.0, r),
                        MeshGradientVertex::new(0.0, 200.0, g),
                        MeshGradientVertex::new(200.0, 200.0, b),
                    ],
                ))
            }
        };
        return Some(new_fill);
    }

    // ── Type-specific controls ────────────────────────────────────────────
    match &fill.kind {
        FillKind::None => {
            ui.label(RichText::new("(no fill)").weak().small());
        }
        FillKind::Solid(col) => {
            let mut rgba = [col.r, col.g, col.b, col.a];
            ui.horizontal(|ui| {
                if ui.color_edit_button_rgba_unmultiplied(&mut rgba).changed() {
                    // handled below after horizontal
                }
                if eyedropper_btn(ui) {
                    *dropper = Some(FillColorSlot::Solid);
                }
            });
            if rgba != [col.r, col.g, col.b, col.a] {
                return Some(Fill::solid(Color {
                    r: rgba[0],
                    g: rgba[1],
                    b: rgba[2],
                    a: rgba[3],
                }));
            }
        }

        FillKind::Gradient(g) => {
            let mut new_g = g.clone();
            let mut grad_changed = false;

            // Coordinate inputs
            match g.kind {
                GradientKind::Linear => {
                    if g.coords.len() >= 4 {
                        ui.label(RichText::new("Start / End").small().weak());
                        ui.horizontal(|ui| {
                            let mut x0 = g.coords[0] as f32;
                            let mut y0 = g.coords[1] as f32;
                            if ui
                                .add(egui::DragValue::new(&mut x0).prefix("x0: ").speed(1.0))
                                .changed()
                            {
                                new_g.coords[0] = x0 as f64;
                                grad_changed = true;
                            }
                            if ui
                                .add(egui::DragValue::new(&mut y0).prefix("y0: ").speed(1.0))
                                .changed()
                            {
                                new_g.coords[1] = y0 as f64;
                                grad_changed = true;
                            }
                        });
                        ui.horizontal(|ui| {
                            let mut x1 = g.coords[2] as f32;
                            let mut y1 = g.coords[3] as f32;
                            if ui
                                .add(egui::DragValue::new(&mut x1).prefix("x1: ").speed(1.0))
                                .changed()
                            {
                                new_g.coords[2] = x1 as f64;
                                grad_changed = true;
                            }
                            if ui
                                .add(egui::DragValue::new(&mut y1).prefix("y1: ").speed(1.0))
                                .changed()
                            {
                                new_g.coords[3] = y1 as f64;
                                grad_changed = true;
                            }
                        });
                    }
                }
                GradientKind::Radial => {
                    if g.coords.len() >= 5 {
                        ui.label(RichText::new("Center / Radius").small().weak());
                        ui.horizontal(|ui| {
                            let mut cx = g.coords[0] as f32;
                            let mut cy = g.coords[1] as f32;
                            if ui
                                .add(egui::DragValue::new(&mut cx).prefix("cx: ").speed(1.0))
                                .changed()
                            {
                                new_g.coords[0] = cx as f64;
                                new_g.coords[2] = cx as f64;
                                grad_changed = true;
                            }
                            if ui
                                .add(egui::DragValue::new(&mut cy).prefix("cy: ").speed(1.0))
                                .changed()
                            {
                                new_g.coords[1] = cy as f64;
                                new_g.coords[3] = cy as f64;
                                grad_changed = true;
                            }
                        });
                        let mut r = g.coords[4] as f32;
                        if ui
                            .add(
                                egui::DragValue::new(&mut r)
                                    .prefix("r: ")
                                    .speed(1.0)
                                    .range(1.0..=10000.0),
                            )
                            .changed()
                        {
                            new_g.coords[4] = r as f64;
                            grad_changed = true;
                        }
                    }
                }
            }

            // Stop editor
            ui.label(RichText::new("Stops").small().weak());
            let mut stop_changed = false;
            let mut remove_idx: Option<usize> = None;
            let stop_count = new_g.stops.len();
            for i in 0..stop_count {
                let mut rgba = {
                    let s = &new_g.stops[i];
                    [s.color.r, s.color.g, s.color.b, s.color.a]
                };
                let mut off = new_g.stops[i].offset;
                let can_remove = stop_count > 2;
                ui.horizontal(|ui| {
                    if ui.color_edit_button_rgba_unmultiplied(&mut rgba).changed() {
                        stop_changed = true;
                    }
                    if eyedropper_btn(ui) {
                        *dropper = Some(FillColorSlot::GradientStop(i));
                    }
                    if ui
                        .add(egui::DragValue::new(&mut off).speed(0.01).range(0.0..=1.0))
                        .changed()
                    {
                        stop_changed = true;
                    }
                    if can_remove && ui.small_button("✕").clicked() {
                        remove_idx = Some(i);
                    }
                });
                if stop_changed {
                    new_g.stops[i].color = Color {
                        r: rgba[0],
                        g: rgba[1],
                        b: rgba[2],
                        a: rgba[3],
                    };
                    new_g.stops[i].offset = off;
                }
            }
            if let Some(idx) = remove_idx {
                new_g.stops.remove(idx);
                stop_changed = true;
            }
            if ui.small_button("+ Stop").clicked() {
                let off = new_g
                    .stops
                    .last()
                    .map(|s| (s.offset + 1.0) / 2.0)
                    .unwrap_or(1.0);
                new_g.stops.push(GradientStop::new(off, Color::WHITE));
                stop_changed = true;
            }

            if grad_changed || stop_changed {
                let mut new_fill = fill.clone();
                new_fill.kind = photonic_core::style::FillKind::Gradient(new_g);
                return Some(new_fill);
            }
        }

        FillKind::FluidGradient(fg) => {
            let mut new_fg = fg.clone();
            let mut fg_changed = false;
            let mut remove_idx: Option<usize> = None;

            ui.label(RichText::new("Control Points").small().weak());
            let pt_count = new_fg.points.len();
            for i in 0..pt_count {
                let mut rgba = {
                    let p = &new_fg.points[i];
                    [p.color.r, p.color.g, p.color.b, p.color.a]
                };
                let mut x = new_fg.points[i].x as f32;
                let mut y = new_fg.points[i].y as f32;
                let can_remove = pt_count > 1;
                let mut pt_changed = false;
                ui.horizontal(|ui| {
                    if ui.color_edit_button_rgba_unmultiplied(&mut rgba).changed() {
                        pt_changed = true;
                    }
                    if eyedropper_btn(ui) {
                        *dropper = Some(FillColorSlot::FluidPoint(i));
                    }
                    if ui
                        .add(egui::DragValue::new(&mut x).prefix("x:").speed(1.0))
                        .changed()
                    {
                        pt_changed = true;
                    }
                    if ui
                        .add(egui::DragValue::new(&mut y).prefix("y:").speed(1.0))
                        .changed()
                    {
                        pt_changed = true;
                    }
                    if can_remove && ui.small_button("✕").clicked() {
                        remove_idx = Some(i);
                    }
                });
                if pt_changed {
                    new_fg.points[i].color = Color {
                        r: rgba[0],
                        g: rgba[1],
                        b: rgba[2],
                        a: rgba[3],
                    };
                    new_fg.points[i].x = x as f64;
                    new_fg.points[i].y = y as f64;
                    fg_changed = true;
                }
            }
            if let Some(idx) = remove_idx {
                new_fg.points.remove(idx);
                fg_changed = true;
            }
            if ui.small_button("+ Point").clicked() {
                new_fg
                    .points
                    .push(FluidGradientPoint::new(100.0, 100.0, Color::WHITE));
                fg_changed = true;
            }
            ui.horizontal(|ui| {
                ui.label(RichText::new("Power:").small());
                let mut p = new_fg.power;
                if ui
                    .add(egui::DragValue::new(&mut p).speed(0.1).range(0.5..=8.0))
                    .changed()
                {
                    new_fg.power = p;
                    fg_changed = true;
                }
            });

            if fg_changed {
                let mut new_fill = fill.clone();
                new_fill.kind = photonic_core::style::FillKind::FluidGradient(new_fg);
                return Some(new_fill);
            }
        }

        FillKind::MeshGradient(mg) => {
            let mut new_mg = mg.clone();
            let mut mg_changed = false;
            let mut mesh_drop_idx: Option<usize> = None;

            ui.label(
                RichText::new(format!("Grid {}×{}", mg.rows, mg.cols))
                    .small()
                    .weak(),
            );
            egui::ScrollArea::vertical()
                .id_salt("mesh_grad_scroll")
                .max_height(180.0)
                .show(ui, |ui| {
                    for row in 0..new_mg.rows {
                        ui.horizontal(|ui| {
                            for col in 0..new_mg.cols {
                                let idx = (row * new_mg.cols + col) as usize;
                                if let Some(v) = new_mg.vertices.get_mut(idx) {
                                    let mut rgba = [v.color.r, v.color.g, v.color.b, v.color.a];
                                    if ui
                                        .color_edit_button_rgba_unmultiplied(&mut rgba)
                                        .on_hover_text(format!("({},{})", row, col))
                                        .changed()
                                    {
                                        v.color = Color {
                                            r: rgba[0],
                                            g: rgba[1],
                                            b: rgba[2],
                                            a: rgba[3],
                                        };
                                        mg_changed = true;
                                    }
                                    if eyedropper_btn(ui) {
                                        mesh_drop_idx = Some(idx);
                                    }
                                }
                            }
                        });
                    }
                });
            if let Some(idx) = mesh_drop_idx {
                *dropper = Some(FillColorSlot::MeshVertex(idx));
            }

            // Grid resize buttons
            ui.horizontal(|ui| {
                if new_mg.rows < 8 && ui.small_button("+ Row").clicked() {
                    let new_row: Vec<MeshGradientVertex> = (0..new_mg.cols)
                        .map(|c| {
                            let x = new_mg
                                .vertices
                                .get(((new_mg.rows - 1) * new_mg.cols + c) as usize)
                                .map(|v| v.x)
                                .unwrap_or(0.0);
                            let prev_y = new_mg
                                .vertices
                                .get(((new_mg.rows - 1) * new_mg.cols + c) as usize)
                                .map(|v| v.y)
                                .unwrap_or(0.0);
                            MeshGradientVertex::new(x, prev_y + 50.0, Color::WHITE)
                        })
                        .collect();
                    new_mg.rows += 1;
                    new_mg.vertices.extend(new_row);
                    mg_changed = true;
                }
                if new_mg.cols < 8 && ui.small_button("+ Col").clicked() {
                    let old_cols = new_mg.cols;
                    new_mg.cols += 1;
                    // Insert a new vertex at the end of each row
                    let mut new_verts: Vec<MeshGradientVertex> = Vec::new();
                    for r in 0..new_mg.rows {
                        for c in 0..old_cols {
                            let v = new_mg.vertices[(r * old_cols + c) as usize].clone();
                            new_verts.push(v);
                        }
                        let prev_x = new_mg.vertices[((r + 1) * old_cols - 1) as usize].x;
                        let prev_y = new_mg.vertices[((r + 1) * old_cols - 1) as usize].y;
                        new_verts.push(MeshGradientVertex::new(
                            prev_x + 50.0,
                            prev_y,
                            Color::WHITE,
                        ));
                    }
                    new_mg.vertices = new_verts;
                    mg_changed = true;
                }
            });

            if mg_changed {
                let mut new_fill = fill.clone();
                new_fill.kind = photonic_core::style::FillKind::MeshGradient(new_mg);
                return Some(new_fill);
            }
        }
    }

    None
}

// ─── Stroke editor ────────────────────────────────────────────────────────────

/// Draw a compact stroke editor. Returns `Some(new_stroke)` if the user changed anything.
/// Sets `*dropper` to `true` when the eyedropper button is clicked.
fn draw_stroke_editor(ui: &mut Ui, stroke: &Stroke, dropper: &mut bool) -> Option<Stroke> {
    use photonic_core::style::{LineCap, LineJoin, StrokeAlign};

    let mut new_stroke = stroke.clone();
    let mut changed = false;

    // Enable / disable toggle
    let mut enabled = new_stroke.enabled;
    if ui.checkbox(&mut enabled, "Enabled").changed() {
        new_stroke.enabled = enabled;
        changed = true;
    }

    if new_stroke.enabled {
        // Color
        let c = &new_stroke.color;
        let mut rgba = [c.r, c.g, c.b, c.a];
        ui.horizontal(|ui| {
            if ui.color_edit_button_rgba_unmultiplied(&mut rgba).changed() {
                changed = true;
            }
            if eyedropper_btn(ui) {
                *dropper = true;
            }
        });
        if changed {
            new_stroke.color = Color {
                r: rgba[0],
                g: rgba[1],
                b: rgba[2],
                a: rgba[3],
            };
        }

        // Width
        ui.horizontal(|ui| {
            ui.label("Width");
            let mut w = new_stroke.width as f32;
            if ui
                .add(egui::DragValue::new(&mut w).range(0.0..=500.0).speed(0.5))
                .changed()
            {
                new_stroke.width = w as f64;
                changed = true;
            }
        });

        // Opacity
        ui.horizontal(|ui| {
            ui.label("Opacity");
            let mut op = new_stroke.opacity;
            if ui.add(egui::Slider::new(&mut op, 0.0..=1.0)).changed() {
                new_stroke.opacity = op;
                changed = true;
            }
        });

        // Line cap
        ui.horizontal(|ui| {
            ui.label("Cap");
            for (label, cap) in [
                ("Butt", LineCap::Butt),
                ("Round", LineCap::Round),
                ("Square", LineCap::Square),
            ] {
                if ui
                    .selectable_label(new_stroke.line_cap == cap, label)
                    .clicked()
                {
                    new_stroke.line_cap = cap;
                    changed = true;
                }
            }
        });

        // Line join
        ui.horizontal(|ui| {
            ui.label("Join");
            for (label, join) in [
                ("Miter", LineJoin::Miter),
                ("Round", LineJoin::Round),
                ("Bevel", LineJoin::Bevel),
            ] {
                if ui
                    .selectable_label(new_stroke.line_join == join, label)
                    .clicked()
                {
                    new_stroke.line_join = join;
                    changed = true;
                }
            }
        });

        // Stroke alignment
        ui.horizontal(|ui| {
            ui.label("Align");
            for (label, align) in [
                ("Center", StrokeAlign::Center),
                ("Inside", StrokeAlign::Inside),
                ("Outside", StrokeAlign::Outside),
            ] {
                if ui
                    .selectable_label(new_stroke.align == align, label)
                    .clicked()
                {
                    new_stroke.align = align;
                    changed = true;
                }
            }
        });

        // Dash controls
        let mut dashes_on = !new_stroke.dash_array.is_empty();
        if ui.checkbox(&mut dashes_on, "Dashed").changed() {
            if dashes_on {
                new_stroke.dash_array = vec![8.0, 4.0];
            } else {
                new_stroke.dash_array.clear();
            }
            changed = true;
        }
        if dashes_on {
            // Ensure pairs up to 3 (6 values); pad if needed for the UI.
            while new_stroke.dash_array.len() < 2 {
                new_stroke.dash_array.push(4.0);
            }
            ui.label(RichText::new("Dash / Gap pairs (up to 3):").weak().small());
            let pair_count = (new_stroke.dash_array.len() / 2).max(1).min(3);
            for i in 0..pair_count {
                let dash_idx = i * 2;
                let gap_idx = i * 2 + 1;
                ui.horizontal(|ui| {
                    ui.label(format!("Pair {}:", i + 1));
                    let mut dash_val =
                        new_stroke.dash_array.get(dash_idx).copied().unwrap_or(8.0) as f32;
                    let mut gap_val =
                        new_stroke.dash_array.get(gap_idx).copied().unwrap_or(4.0) as f32;
                    if ui
                        .add(
                            egui::DragValue::new(&mut dash_val)
                                .range(0.5..=500.0)
                                .speed(0.5)
                                .prefix("—"),
                        )
                        .changed()
                    {
                        if new_stroke.dash_array.len() <= dash_idx {
                            new_stroke.dash_array.resize(dash_idx + 1, 0.0);
                        }
                        new_stroke.dash_array[dash_idx] = dash_val as f64;
                        changed = true;
                    }
                    ui.label("·");
                    if ui
                        .add(
                            egui::DragValue::new(&mut gap_val)
                                .range(0.0..=500.0)
                                .speed(0.5),
                        )
                        .changed()
                    {
                        if new_stroke.dash_array.len() <= gap_idx {
                            new_stroke.dash_array.resize(gap_idx + 1, 0.0);
                        }
                        new_stroke.dash_array[gap_idx] = gap_val as f64;
                        changed = true;
                    }
                });
            }
            // Add/remove pair buttons
            ui.horizontal(|ui| {
                if pair_count < 3 {
                    if ui
                        .small_button("+ Pair")
                        .on_hover_text("Add a dash/gap pair")
                        .clicked()
                    {
                        new_stroke.dash_array.extend_from_slice(&[8.0, 4.0]);
                        changed = true;
                    }
                }
                if pair_count > 1 {
                    if ui
                        .small_button("− Pair")
                        .on_hover_text("Remove the last dash/gap pair")
                        .clicked()
                    {
                        new_stroke
                            .dash_array
                            .truncate(new_stroke.dash_array.len().saturating_sub(2));
                        changed = true;
                    }
                }
            });
            // Dash offset
            ui.horizontal(|ui| {
                ui.label("Offset:");
                let mut offset = new_stroke.dash_offset as f32;
                if ui
                    .add(egui::DragValue::new(&mut offset).speed(0.5))
                    .changed()
                {
                    new_stroke.dash_offset = offset as f64;
                    changed = true;
                }
            });
            // Align dashes to corners
            let mut align_corners = new_stroke.dash_corner_alignment;
            if ui.checkbox(&mut align_corners, "Align to corners")
                .on_hover_text("Adjust dash spacing so dashes start and end cleanly at path corners and endpoints")
                .changed()
            {
                new_stroke.dash_corner_alignment = align_corners;
                changed = true;
            }
        }
    }

    // ── Arrowheads ──────────────────────────────────────────────────────────
    {
        use photonic_core::style::ArrowheadStyle;
        let arrow_label = |s: ArrowheadStyle| match s {
            ArrowheadStyle::None => "None",
            ArrowheadStyle::FilledArrow => "Filled",
            ArrowheadStyle::OpenArrow => "Open",
        };
        ui.horizontal(|ui| {
            ui.label("Arrow start");
            egui::ComboBox::new("arrow_start", "")
                .selected_text(arrow_label(new_stroke.arrowhead_start))
                .show_ui(ui, |ui| {
                    for style in [
                        ArrowheadStyle::None,
                        ArrowheadStyle::FilledArrow,
                        ArrowheadStyle::OpenArrow,
                    ] {
                        if ui
                            .selectable_label(
                                new_stroke.arrowhead_start == style,
                                arrow_label(style),
                            )
                            .clicked()
                        {
                            new_stroke.arrowhead_start = style;
                            changed = true;
                        }
                    }
                });
        });
        ui.horizontal(|ui| {
            ui.label("Arrow end");
            egui::ComboBox::new("arrow_end", "")
                .selected_text(arrow_label(new_stroke.arrowhead_end))
                .show_ui(ui, |ui| {
                    for style in [
                        ArrowheadStyle::None,
                        ArrowheadStyle::FilledArrow,
                        ArrowheadStyle::OpenArrow,
                    ] {
                        if ui
                            .selectable_label(new_stroke.arrowhead_end == style, arrow_label(style))
                            .clicked()
                        {
                            new_stroke.arrowhead_end = style;
                            changed = true;
                        }
                    }
                });
        });
    }

    if changed {
        Some(new_stroke)
    } else {
        None
    }
}

// ─── Glow editor ──────────────────────────────────────────────────────────────

/// Renders a compact editor for a `GlowEffect`. Returns `Some(updated)` on any change.
/// Sets `*dropper` to `true` when the eyedropper button is clicked.
fn draw_glow_editor(ui: &mut Ui, glow: &GlowEffect, dropper: &mut bool) -> Option<GlowEffect> {
    let mut new_glow = glow.clone();
    let mut changed = false;

    ui.horizontal(|ui| {
        if ui.checkbox(&mut new_glow.enabled, "Enabled").changed() {
            changed = true;
        }
    });

    if new_glow.enabled {
        ui.horizontal(|ui| {
            ui.label("Color");
            let mut rgba = [
                new_glow.color.r,
                new_glow.color.g,
                new_glow.color.b,
                new_glow.color.a,
            ];
            if ui.color_edit_button_rgba_unmultiplied(&mut rgba).changed() {
                new_glow.color = Color {
                    r: rgba[0],
                    g: rgba[1],
                    b: rgba[2],
                    a: rgba[3],
                };
                changed = true;
            }
            if eyedropper_btn(ui) {
                *dropper = true;
            }
        });
        ui.horizontal(|ui| {
            ui.label("Opacity");
            if ui
                .add(egui::Slider::new(&mut new_glow.opacity, 0.0..=1.0))
                .changed()
            {
                changed = true;
            }
        });
        ui.horizontal(|ui| {
            ui.label("Size");
            if ui
                .add(egui::Slider::new(&mut new_glow.size, 1.0..=100.0).suffix("px"))
                .changed()
            {
                changed = true;
            }
        });
        ui.horizontal(|ui| {
            ui.label("Corners");
            let options = [
                (LineJoin::Miter, "Miter"),
                (LineJoin::Round, "Round"),
                (LineJoin::Bevel, "Bevel"),
            ];
            for (variant, label) in options {
                if ui
                    .selectable_label(new_glow.join == variant, label)
                    .clicked()
                {
                    new_glow.join = variant;
                    changed = true;
                }
            }
        });
    }

    if changed {
        Some(new_glow)
    } else {
        None
    }
}

// ─── Gaussian Glow editor ─────────────────────────────────────────────────────

/// Renders a compact editor for a `GaussianGlow`. Returns `Some(updated)` on any change.
/// Sets `*dropper` to `true` when the eyedropper button is clicked.
fn draw_gaussian_glow_editor(
    ui: &mut Ui,
    glow: &GaussianGlow,
    dropper: &mut bool,
) -> Option<GaussianGlow> {
    let mut new_glow = glow.clone();
    let mut changed = false;

    ui.horizontal(|ui| {
        if ui.checkbox(&mut new_glow.enabled, "Enabled").changed() {
            changed = true;
        }
    });

    if new_glow.enabled {
        ui.horizontal(|ui| {
            ui.label("Color");
            let mut rgba = [
                new_glow.color.r,
                new_glow.color.g,
                new_glow.color.b,
                new_glow.color.a,
            ];
            if ui.color_edit_button_rgba_unmultiplied(&mut rgba).changed() {
                new_glow.color = Color {
                    r: rgba[0],
                    g: rgba[1],
                    b: rgba[2],
                    a: rgba[3],
                };
                changed = true;
            }
            if eyedropper_btn(ui) {
                *dropper = true;
            }
        });
        ui.horizontal(|ui| {
            ui.label("Opacity");
            if ui
                .add(egui::Slider::new(&mut new_glow.opacity, 0.0..=1.0))
                .changed()
            {
                changed = true;
            }
        });
        ui.horizontal(|ui| {
            ui.label("Radius");
            if ui
                .add(egui::Slider::new(&mut new_glow.radius, 1.0..=200.0).suffix("px"))
                .changed()
            {
                changed = true;
            }
        });
    }

    if changed {
        Some(new_glow)
    } else {
        None
    }
}

// ─── Change log panel ─────────────────────────────────────────────────────────

/// Draw the change-log panel showing the last 50 checkpoints (newest first).
/// Returns a `RestoreCheckpoint` action when the user clicks an entry.
pub fn draw_changelog_panel(
    ui: &mut Ui,
    checkpoints: &[CheckpointInfo],
    max_height: f32,
) -> Option<PanelAction> {
    let mut result = None;

    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!("{} Change Log", ph::CLOCK_CLOCKWISE))
                .strong()
                .small(),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                RichText::new(format!("{}/50", checkpoints.len()))
                    .weak()
                    .small(),
            );
        });
    });

    egui::ScrollArea::vertical()
        .id_salt("changelog_scroll")
        .max_height(max_height)
        .show(ui, |ui| {
            if checkpoints.is_empty() {
                ui.label(RichText::new("No changes yet").weak().italics().small());
                return;
            }

            let now_secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();

            for cp in checkpoints.iter().rev() {
                let age = now_secs.saturating_sub(cp.created_at);
                let time_str = if age < 60 {
                    format!("{}s ago", age)
                } else if age < 3600 {
                    format!("{}m ago", age / 60)
                } else {
                    format!("{}h ago", age / 3600)
                };

                ui.horizontal(|ui| {
                    let resp = ui.add(
                        egui::Label::new(
                            RichText::new(&cp.name)
                                .small()
                                .color(Color32::from_rgb(122, 122, 154)),
                        )
                        .sense(egui::Sense::click())
                        .truncate(),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(&time_str).weak().small());
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new(format!("{}", ph::GIT_DIFF)).small(),
                                )
                                .small()
                                .frame(false),
                            )
                            .on_hover_text("Diff with current document")
                            .clicked()
                        {
                            result = Some(PanelAction::DiffWithCheckpoint {
                                checkpoint_id: cp.id,
                            });
                        }
                    });
                    if resp.clicked() {
                        result = Some(PanelAction::RestoreCheckpoint(cp.id));
                    }
                    if resp.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                });
            }
        });

    result
}

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
                if ui.small_button("✕").clicked() {
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
