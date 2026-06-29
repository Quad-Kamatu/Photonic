use crate::protocol::{
    AddAnchorPointsArgs, AddDimensionLineArgs, AddDropShadowArgs, AddGuideArgs, AdjustColorsArgs,
    AlignAnchor, AlignNodesArgs, AlignOperation, ApplyCharacterStyleArgs, ApplyFlexLayoutArgs,
    ApplyGridLayoutArgs, ApplyParagraphStyleArgs, ApplyStackLayoutArgs, ApplyTransformArgs,
    ArrayMode, AutoNameNodesArgs, AverageAnchorPointsArgs, BindTextVariableArgs, BlendColorsArgs,
    BlendObjectsArgs, BooleanOperationArgs, BuildShapeFromPointsArgs, CenterOnCanvasArgs,
    CheckStyleContinuityArgs, CleanUpArgs, ClearBlendSpineArgs, ClearGuidesArgs,
    ClearSymbolOverridesArgs, ClearTabStopsArgs, ClearTextAreaArgs, ClearTextPathArgs,
    ConvertAnchorMode, ConvertAnchorPointsArgs, ConvertToGrayscaleArgs, CopyAppearanceArgs,
    CreateArrayArgs, CreateArrowShapeArgs, CreateBarChartArgs, CreateCharacterStyleArgs,
    CreateCrossArgs, CreateCurvaturePathArgs, CreateDonutArgs, CreateFlareArgs,
    CreateFreehandPathArgs, CreateGearArgs, CreateGridArgs, CreateHeartArgs, CreateLineChartArgs,
    CreateParagraphStyleArgs, CreateParametricShapeArgs, CreatePathArgs, CreatePieChartArgs,
    CreatePolarGridArgs, CreateRadarChartArgs, CreateScatterPlotArgs, CreateShapeArgs,
    CreateSpeechBubbleArgs, CreateSpiralArgs, CreateStackedBarChartArgs, CreateSunburstArgs,
    CreateTextArgs, CreateTruchetTilingArgs, CreateWavePatternArgs, CrossAxisAlign,
    CrystallizePathArgs, DeleteAnchorPointArgs, DeleteCharacterStyleArgs, DeleteNodeArgs,
    DeleteParagraphStyleArgs, DeselectAllArgs, DistributeNoOverlapArgs, DistributeOnPathArgs,
    DivideObjectsBelowArgs, DuplicateNodesArgs, EnterIsolationModeArgs, ExitIsolationModeArgs,
    ExpandBlendArgs, ExportTaggedAssetsArgs, FillArg, FindNodesArgs, FindReplaceStyleArgs,
    FindReplaceTextArgs, FitToCanvasArgs, FlattenGroupArgs, FlattenTransparencyArgs, FlipNodesArgs,
    GetCssPreviewArgs, GetNodeArgs, GetNodePromptsArgs, GetOpenTypeFeaturesArgs,
    GetRecentColorsArgs, GroupNodesArgs, HatchFillArgs, InspectNodeArgs, InvertColorsArgs,
    JoinPathsArgs, LassoSelectArgs, LayoutMode, LayoutNodesArgs, LinkTextFramesArgs,
    ListGuidesArgs, MagicWandSelectArgs, MakeClippingMaskArgs, MakeCompoundPathArgs,
    MeasureDistanceArgs, MeasurePathArgs, MeasureTarget, MirrorCopyArgs, MoveToLayerArgs,
    NoiseDeformArgs, ObjectKindFilter, OffsetPathArgs, OutlineStrokeArgs, ParametricShapeType,
    PathfinderCropArgs, PathfinderDivideArgs, PathfinderMergeArgs, PathfinderMinusBackArgs,
    PathfinderMinusFrontArgs, PathfinderOutlineArgs, PathfinderTrimArgs, PinObjectGuidesArgs,
    PointOnPathArgs, PuckerBloatArgs, RandomizeColorsArgs, RecolorArtworkArgs,
    ReleaseClippingMaskArgs, ReleaseCompoundPathArgs, RemoveGuideArgs, RemoveStyleArgs,
    ReorderNodeArgs, ReorderOperation, ReverseBlendSpineArgs, ReverseNodeOrderArgs,
    ReversePathDirectionArgs, RotateCopiesArgs, RoughenPathArgs, RoundCornersArgs,
    SampleColorAtArgs, ScallopPathArgs, ScatterCopiesArgs, ScissorsCutArgs, SelectAllArgs,
    SelectByKindArgs, SelectInsideGroupArgs, SelectSameArgs, SelectSameAttribute,
    SelectSimilarArgs, SetBlendModeArgs, SetBlendSpineArgs, SetFontStyleArgs, SetFontWeightArgs,
    SetLockedArgs, SetNodePromptArgs, SetOpacityArgs, SetOpenTypeFeaturesArgs,
    SetParagraphOptionsArgs, SetSelectionArgs, SetSymbolOverrideArgs, SetTabStopsArgs,
    SetTextAreaArgs, SetTextDecorationArgs, SetTextDirectionArgs, SetTextPathArgs,
    SetVisibilityArgs, ShapeType, SimplifyPathArgs, SmoothPathArgs, SnapToPixelArgs,
    SplitIntoGridArgs, StippleFillArgs, StrokeArg, StyleTransferArgs, SwapFillStrokeArgs,
    TagNodeForExportArgs, TagNodesArgs, ToolResult, TransformCopiesArgs, TwirlPathArgs,
    UnbindTextVariableArgs, UndoNodeArgs, UngroupNodesArgs, UnlinkTextFramesArgs, UpdateNodeArgs,
    WarpEnvelopeArgs, ZigZagPathArgs,
};
use crate::server::AppState;
use kurbo;
use photonic_core::{
    document::{Guide, GuideOrientation},
    history::Command,
    layer::BlendMode,
    node::{FontStyle, GroupNode, NodeId, PathNode, SceneNode, SceneNodeKind, TextNode},
    path::PathData,
    transform::Transform,
};

/// Apply optional fill and stroke arguments to a `PathNode`.
/// Returns `Err(message)` if either color fails to parse.
fn apply_style(
    path_node: &mut photonic_core::node::PathNode,
    fill: Option<FillArg>,
    stroke: Option<StrokeArg>,
) -> Result<(), String> {
    if let Some(fill_arg) = fill {
        path_node.fill = fill_arg.to_fill()?;
    }
    if let Some(stroke_arg) = stroke {
        path_node.stroke = stroke_arg.to_stroke()?;
    }
    Ok(())
}

pub async fn create_shape(state: &AppState, args: CreateShapeArgs) -> ToolResult {
    tracing::debug!("tool: create_shape {:?}", args.shape_type);
    let path_data = match args.shape_type {
        ShapeType::Rectangle => PathData::rect(args.x, args.y, args.width, args.height),
        ShapeType::RoundedRect => PathData::rounded_rect(
            args.x,
            args.y,
            args.width,
            args.height,
            args.corner_radius.unwrap_or(10.0),
        ),
        ShapeType::Ellipse => {
            let cx = args.x + args.width / 2.0;
            let cy = args.y + args.height / 2.0;
            PathData::ellipse(cx, cy, args.width / 2.0, args.height / 2.0)
        }
        ShapeType::Polygon => {
            let cx = args.x + args.width / 2.0;
            let cy = args.y + args.height / 2.0;
            let r = args.width.min(args.height) / 2.0;
            PathData::regular_polygon(cx, cy, r, args.sides.unwrap_or(6))
        }
        ShapeType::Star => {
            let cx = args.x + args.width / 2.0;
            let cy = args.y + args.height / 2.0;
            let outer = args.width.min(args.height) / 2.0;
            let inner = outer * args.inner_radius.unwrap_or(0.4);
            PathData::star(cx, cy, outer, inner, args.sides.unwrap_or(5))
        }
        ShapeType::Line => {
            PathData::line(args.x, args.y, args.x + args.width, args.y + args.height)
        }
        ShapeType::Arc => {
            let cx = args.x + args.width / 2.0;
            let cy = args.y + args.height / 2.0;
            let rx = args.width.abs() / 2.0;
            let ry = args.height.abs() / 2.0;
            let start = args.arc_start_angle.unwrap_or(0.0);
            let end = args.arc_end_angle.unwrap_or(270.0);
            let open = args.arc_open.unwrap_or(false);
            PathData::arc(cx, cy, rx, ry, start, end, !open)
        }
    };

    let mut path_node = PathNode::new(path_data);
    if let Err(e) = apply_style(&mut path_node, args.fill, args.stroke) {
        return ToolResult::error(e);
    }

    let shape_name = args
        .name
        .unwrap_or_else(|| format!("{:?}", args.shape_type));

    let mut doc = state.document.lock().await;
    let mut node = SceneNode::new(
        &shape_name,
        uuid::Uuid::nil(),
        SceneNodeKind::Path(path_node),
    );
    if !args.tags.is_empty() {
        node.tags = args.tags;
    }

    let node_id = node.id;
    let cmd = Command::AddNode {
        node,
        layer_id: args.layer_id,
    };
    let mut history = state.history.lock().await;
    history.execute(cmd.clone(), &mut doc);

    ToolResult::text(format!(
        "Created {} '{}' (id: {})",
        format!("{:?}", args.shape_type).to_lowercase(),
        shape_name,
        node_id
    ))
    .with_data(serde_json::json!({ "node_id": node_id }))
}

pub async fn create_path(state: &AppState, args: CreatePathArgs) -> ToolResult {
    tracing::debug!("tool: create_path (data len={})", args.path_data.len());
    let path_data = match PathData::from_svg(&args.path_data) {
        Ok(p) => p,
        Err(e) => return ToolResult::error(format!("Invalid SVG path data: {}", e)),
    };

    let mut path_node = PathNode::new(path_data);
    if let Err(e) = apply_style(&mut path_node, args.fill, args.stroke) {
        return ToolResult::error(e);
    }

    let name = args.name.unwrap_or_else(|| "Path".to_string());
    let mut doc = state.document.lock().await;
    let mut node = SceneNode::new(&name, uuid::Uuid::nil(), SceneNodeKind::Path(path_node));

    if let Some(t_arg) = args.transform {
        node.transform = t_arg.to_transform();
    }
    if !args.tags.is_empty() {
        node.tags = args.tags;
    }

    let node_id = node.id;
    let cmd = Command::AddNode {
        node,
        layer_id: args.layer_id,
    };
    let mut history = state.history.lock().await;
    history.execute(cmd, &mut doc);

    ToolResult::text(format!("Created path '{}' (id: {})", name, node_id))
        .with_data(serde_json::json!({ "node_id": node_id }))
}

pub async fn create_curvature_path(state: &AppState, args: CreateCurvaturePathArgs) -> ToolResult {
    tracing::debug!("tool: create_curvature_path (points={})", args.points.len());

    if args.points.len() < 2 {
        return ToolResult::error("At least 2 points are required");
    }

    // Build a smooth cubic bezier path through the points using Catmull-Rom interpolation.
    let pts: Vec<kurbo::Point> = args
        .points
        .iter()
        .map(|p| kurbo::Point::new(p[0], p[1]))
        .collect();
    let bez = catmull_rom_to_bezier(&pts, args.closed);

    let path_data = PathData::from_bez_path(&bez);
    let mut path_node = PathNode::new(path_data);
    if let Err(e) = apply_style(&mut path_node, args.fill, args.stroke) {
        return ToolResult::error(e);
    }

    let layer_id = args.layer_id.and_then(|s| uuid::Uuid::parse_str(&s).ok());

    let name = "Curvature Path".to_string();
    let mut doc = state.document.lock().await;
    let node = SceneNode::new(&name, uuid::Uuid::nil(), SceneNodeKind::Path(path_node));
    let node_id = node.id;
    let cmd = Command::AddNode { node, layer_id };
    let mut history = state.history.lock().await;
    history.execute(cmd, &mut doc);

    ToolResult::text(format!(
        "Created smooth curve through {} points (id: {})",
        pts.len(),
        node_id
    ))
    .with_data(serde_json::json!({ "node_id": node_id, "point_count": pts.len() }))
}

/// Convert a sequence of points to a smooth cubic bezier path using Catmull-Rom interpolation.
/// The tension parameter is fixed at 0 (uniform Catmull-Rom = smooth interpolation).
fn catmull_rom_to_bezier(points: &[kurbo::Point], closed: bool) -> kurbo::BezPath {
    let n = points.len();
    let mut path = kurbo::BezPath::new();

    if n < 2 {
        if n == 1 {
            path.move_to(points[0]);
        }
        return path;
    }

    if n == 2 {
        // Straight line for 2 points.
        path.move_to(points[0]);
        path.line_to(points[1]);
        if closed {
            path.close_path();
        }
        return path;
    }

    // For Catmull-Rom → cubic bezier conversion:
    // Given four points P0, P1, P2, P3, the cubic bezier between P1 and P2 has:
    //   cp1 = P1 + (P2 - P0) / 6
    //   cp2 = P2 - (P3 - P1) / 6
    //
    // For endpoints of an open curve, we mirror the missing point.

    let get_point = |i: isize| -> kurbo::Point {
        if closed {
            points[((i % n as isize) + n as isize) as usize % n]
        } else {
            if i < 0 {
                // Mirror: P[-1] = 2*P[0] - P[1]
                kurbo::Point::new(
                    2.0 * points[0].x - points[1].x,
                    2.0 * points[0].y - points[1].y,
                )
            } else if i >= n as isize {
                // Mirror: P[n] = 2*P[n-1] - P[n-2]
                kurbo::Point::new(
                    2.0 * points[n - 1].x - points[n - 2].x,
                    2.0 * points[n - 1].y - points[n - 2].y,
                )
            } else {
                points[i as usize]
            }
        }
    };

    path.move_to(points[0]);

    let segments = if closed { n } else { n - 1 };
    for i in 0..segments {
        let p0 = get_point(i as isize - 1);
        let p1 = get_point(i as isize);
        let p2 = get_point(i as isize + 1);
        let p3 = get_point(i as isize + 2);

        let cp1 = kurbo::Point::new(p1.x + (p2.x - p0.x) / 6.0, p1.y + (p2.y - p0.y) / 6.0);
        let cp2 = kurbo::Point::new(p2.x - (p3.x - p1.x) / 6.0, p2.y - (p3.y - p1.y) / 6.0);

        path.curve_to(cp1, cp2, p2);
    }

    if closed {
        path.close_path();
    }

    path
}

pub async fn create_flare(state: &AppState, args: CreateFlareArgs) -> ToolResult {
    tracing::debug!("tool: create_flare");
    use kurbo::Shape;
    use photonic_core::color::Color;
    use photonic_core::style::{Fill, FillKind, Stroke};

    let cx = args.cx;
    let cy = args.cy;
    let halo_r = args.halo_radius.unwrap_or(50.0);
    let ray_count = args.ray_count.unwrap_or(12).max(2);
    let ray_len = args.ray_length.unwrap_or(80.0);
    let ring_count = args.ring_count.unwrap_or(3);
    let ray_opacity = args.ray_opacity.unwrap_or(0.3);

    let halo_color = args.halo_color.as_deref().unwrap_or("#fffbe6");
    let halo_c = Color::from_hex(halo_color).unwrap_or(Color::new(1.0, 0.98, 0.9, 0.6));

    let layer_id = args.layer_id.and_then(|s| uuid::Uuid::parse_str(&s).ok());

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let actual_layer = layer_id
        .or(doc.active_layer_id)
        .unwrap_or(uuid::Uuid::nil());
    let mut child_ids = Vec::new();

    // 1. Create halo circle (semi-transparent filled ellipse).
    {
        let path = kurbo::Ellipse::new((cx, cy), (halo_r, halo_r), 0.0).to_path(0.1);
        let mut pn = PathNode::new(PathData::from_bez_path(&path));
        pn.fill = Fill {
            kind: FillKind::Solid(Color::new(halo_c.r, halo_c.g, halo_c.b, 0.6)),
            ..Default::default()
        };
        pn.stroke = Stroke::none();
        let node = SceneNode::new("Flare Halo", actual_layer, SceneNodeKind::Path(pn));
        let nid = node.id;
        child_ids.push(nid);
        history.execute(
            Command::AddNode {
                node,
                layer_id: Some(actual_layer),
            },
            &mut doc,
        );
    }

    // 2. Create radiating rays (thin triangles).
    for i in 0..ray_count {
        let angle = std::f64::consts::TAU * i as f64 / ray_count as f64;
        let half_width = std::f64::consts::TAU / ray_count as f64 * 0.15; // thin ray

        let tip_x = cx + (halo_r + ray_len) * angle.cos();
        let tip_y = cy + (halo_r + ray_len) * angle.sin();
        let base_l_x = cx + halo_r * 0.8 * (angle - half_width).cos();
        let base_l_y = cy + halo_r * 0.8 * (angle - half_width).sin();
        let base_r_x = cx + halo_r * 0.8 * (angle + half_width).cos();
        let base_r_y = cy + halo_r * 0.8 * (angle + half_width).sin();

        let mut bez = kurbo::BezPath::new();
        bez.move_to((base_l_x, base_l_y));
        bez.line_to((tip_x, tip_y));
        bez.line_to((base_r_x, base_r_y));
        bez.close_path();

        let mut pn = PathNode::new(PathData::from_bez_path(&bez));
        pn.fill = Fill {
            kind: FillKind::Solid(Color::new(halo_c.r, halo_c.g, halo_c.b, ray_opacity)),
            ..Default::default()
        };
        pn.stroke = Stroke::none();
        let mut node = SceneNode::new(
            &format!("Flare Ray {}", i + 1),
            actual_layer,
            SceneNodeKind::Path(pn),
        );
        node.opacity = ray_opacity;
        let nid = node.id;
        child_ids.push(nid);
        history.execute(
            Command::AddNode {
                node,
                layer_id: Some(actual_layer),
            },
            &mut doc,
        );
    }

    // 3. Create concentric rings.
    for i in 0..ring_count {
        let ring_r = halo_r * (1.5 + i as f64 * 0.8);
        let ring_opacity = 0.15 / (i as f32 + 1.0);
        let path = kurbo::Ellipse::new((cx, cy), (ring_r, ring_r), 0.0).to_path(0.1);
        let mut pn = PathNode::new(PathData::from_bez_path(&path));
        pn.fill = Fill::none();
        pn.stroke = Stroke {
            color: Color::new(halo_c.r, halo_c.g, halo_c.b, ring_opacity),
            width: 1.5,
            ..Default::default()
        };
        let node = SceneNode::new(
            &format!("Flare Ring {}", i + 1),
            actual_layer,
            SceneNodeKind::Path(pn),
        );
        let nid = node.id;
        child_ids.push(nid);
        history.execute(
            Command::AddNode {
                node,
                layer_id: Some(actual_layer),
            },
            &mut doc,
        );
    }

    // 4. Group all flare parts.
    let group = SceneNode::new(
        "Lens Flare",
        actual_layer,
        SceneNodeKind::Group(photonic_core::node::GroupNode::new()),
    );
    let group_id = group.id;
    history.execute(
        Command::GroupNodes {
            group,
            layer_id: actual_layer,
            insert_index: 0,
            children: child_ids.clone(),
        },
        &mut doc,
    );

    ToolResult::text(format!(
        "Created lens flare at ({cx}, {cy}) — {} rays, {} rings, halo r={halo_r}",
        ray_count, ring_count
    ))
    .with_data(serde_json::json!({
        "group_id": group_id,
        "child_count": child_ids.len(),
    }))
}

pub async fn create_spiral(state: &AppState, args: CreateSpiralArgs) -> ToolResult {
    tracing::debug!("tool: create_spiral turns={}", args.turns);

    if args.outer_radius <= 0.0 {
        return ToolResult::error("outer_radius must be greater than 0");
    }
    if args.turns <= 0.0 {
        return ToolResult::error("turns must be greater than 0");
    }

    let path_data = PathData::spiral(
        args.x,
        args.y,
        args.outer_radius,
        args.inner_radius,
        args.turns,
        args.segments_per_turn,
    );

    let mut path_node = PathNode::new(path_data);
    if let Err(e) = apply_style(&mut path_node, args.fill, args.stroke) {
        return ToolResult::error(e);
    }

    let name = args.name.unwrap_or_else(|| "Spiral".to_string());
    let mut doc = state.document.lock().await;
    let node = SceneNode::new(&name, uuid::Uuid::nil(), SceneNodeKind::Path(path_node));
    let node_id = node.id;
    let cmd = Command::AddNode {
        node,
        layer_id: args.layer_id,
    };
    let mut history = state.history.lock().await;
    history.execute(cmd, &mut doc);

    ToolResult::text(format!(
        "Created spiral '{}' ({} turns, outer_r={}, inner_r={}) id: {}",
        name, args.turns, args.outer_radius, args.inner_radius, node_id
    ))
    .with_data(serde_json::json!({ "node_id": node_id }))
}

pub async fn create_polar_grid(state: &AppState, args: CreatePolarGridArgs) -> ToolResult {
    if args.outer_radius <= 0.0 {
        return ToolResult::error("outer_radius must be greater than 0");
    }
    let inner_r = args.inner_radius.unwrap_or(0.0).max(0.0);
    let rings = args.rings.unwrap_or(4).max(1);
    let sectors = args.sectors.unwrap_or(8).max(1);

    let path_data =
        PathData::polar_grid(args.x, args.y, args.outer_radius, inner_r, rings, sectors);

    let mut path_node = PathNode::new(path_data);
    if let Err(e) = apply_style(&mut path_node, args.fill, args.stroke) {
        return ToolResult::error(e);
    }

    let name = args
        .name
        .unwrap_or_else(|| format!("Polar Grid {}r {}s", rings, sectors));
    let mut doc = state.document.lock().await;
    let node = SceneNode::new(&name, uuid::Uuid::nil(), SceneNodeKind::Path(path_node));
    let node_id = node.id;
    let cmd = Command::AddNode {
        node,
        layer_id: args.layer_id,
    };
    let mut history = state.history.lock().await;
    history.execute(cmd, &mut doc);

    ToolResult::text(format!(
        "Created polar grid '{}' ({} rings, {} sectors, outer_r={}) id: {}",
        name, rings, sectors, args.outer_radius, node_id
    ))
    .with_data(serde_json::json!({ "node_id": node_id, "rings": rings, "sectors": sectors }))
}

pub async fn create_grid(state: &AppState, args: CreateGridArgs) -> ToolResult {
    if args.width <= 0.0 || args.height <= 0.0 {
        return ToolResult::error("width and height must be greater than 0");
    }
    let cols = args.cols.unwrap_or(4).max(1);
    let rows = args.rows.unwrap_or(4).max(1);

    let path_data = PathData::grid(args.x, args.y, args.width, args.height, cols, rows);

    let mut path_node = PathNode::new(path_data);
    if let Err(e) = apply_style(&mut path_node, args.fill, args.stroke) {
        return ToolResult::error(e);
    }

    let name = args
        .name
        .unwrap_or_else(|| format!("Grid {}×{}", cols, rows));
    let mut doc = state.document.lock().await;
    let node = SceneNode::new(&name, uuid::Uuid::nil(), SceneNodeKind::Path(path_node));
    let node_id = node.id;
    let cmd = Command::AddNode {
        node,
        layer_id: args.layer_id,
    };
    let mut history = state.history.lock().await;
    history.execute(cmd, &mut doc);

    ToolResult::text(format!(
        "Created grid '{}' ({}×{} cells, {}×{} size) id: {}",
        name, cols, rows, args.width, args.height, node_id
    ))
    .with_data(serde_json::json!({ "node_id": node_id, "cols": cols, "rows": rows }))
}

pub async fn create_text(state: &AppState, args: CreateTextArgs) -> ToolResult {
    use photonic_core::node::TextAlign;
    tracing::debug!(
        "tool: create_text {:?}",
        &args.content[..args.content.len().min(40)]
    );

    let mut text_node = TextNode::new(&args.content);
    if let Some(ff) = args.font_family {
        text_node.font_family = ff;
    }
    if let Some(fs) = args.font_size {
        text_node.font_size = fs;
    }
    if let Some(fw) = args.font_weight {
        text_node.font_weight = fw;
    }
    if let Some(ref a) = args.align {
        text_node.align = match a.as_str() {
            "center" => TextAlign::Center,
            "right" => TextAlign::Right,
            _ => TextAlign::Left,
        };
    }
    if let Some(lh) = args.line_height {
        text_node.line_height = lh;
    }
    if let Some(ls) = args.letter_spacing {
        text_node.letter_spacing = ls;
    }
    if let Some(fill_arg) = args.fill {
        match fill_arg.to_fill() {
            Ok(f) => text_node.fill = f,
            Err(e) => return ToolResult::error(e),
        }
    }
    if let Some(stroke_arg) = args.stroke {
        match stroke_arg.to_stroke() {
            Ok(s) => text_node.stroke = s,
            Err(e) => return ToolResult::error(e),
        }
    }

    let name = args.name.unwrap_or_else(|| "Text".to_string());
    let mut node = SceneNode::new(&name, uuid::Uuid::nil(), SceneNodeKind::Text(text_node));
    node.transform = Transform::translate(args.x, args.y);
    if !args.tags.is_empty() {
        node.tags = args.tags;
    }

    let mut doc = state.document.lock().await;
    let node_id = node.id;
    let cmd = Command::AddNode {
        node,
        layer_id: args.layer_id,
    };
    let mut history = state.history.lock().await;
    history.execute(cmd, &mut doc);

    ToolResult::text(format!("Created text '{}' (id: {})", name, node_id))
        .with_data(serde_json::json!({ "node_id": node_id }))
}

pub async fn update_node(state: &AppState, args: UpdateNodeArgs) -> ToolResult {
    tracing::debug!("tool: update_node {}", args.node_id);
    // Read phase: clone the node, then immediately release the doc lock.
    let old_node = {
        let doc = state.document.lock().await;
        match doc.get_node(&args.node_id) {
            Some(n) => n.clone(),
            None => return ToolResult::error(format!("Node {} not found", args.node_id)),
        }
    }; // doc lock released here

    // Prepare phase: build the updated node — no locks held.
    let mut new_node = old_node.clone();

    if let Some(name) = args.name {
        new_node.name = name;
    }
    if let Some(opacity) = args.opacity {
        new_node.opacity = opacity;
    }
    if let Some(visible) = args.visible {
        new_node.visible = visible;
    }
    if let Some(locked) = args.locked {
        new_node.locked = locked;
    }
    if let Some(blend_mode) = args.blend_mode {
        if blend_mode != BlendMode::Normal {
            return ToolResult::error(
                "Blend modes other than 'normal' are not yet rendered. \
                 Set blend_mode to 'normal' (or omit it) until blend mode \
                 rendering is implemented.",
            );
        }
        new_node.blend_mode = blend_mode;
    }
    if let Some(tags) = args.tags {
        new_node.tags = tags;
    }
    if let Some(og) = args.outer_glow {
        new_node.outer_glow = og.into();
    }
    if let Some(ig) = args.inner_glow {
        new_node.inner_glow = ig.into();
    }
    if let Some(gg) = args.gaussian_glow {
        new_node.gaussian_glow = gg.into();
    }
    if let Some(ds) = args.drop_shadow {
        new_node.drop_shadow = ds.into();
    }
    if let Some(ob) = args.object_blur {
        new_node.object_blur = ob.into();
    }
    if let Some(ft) = args.feather {
        new_node.feather = ft.into();
    }
    if let Some(t_arg) = args.transform {
        new_node.transform = t_arg.to_transform();
    }

    match &mut new_node.kind {
        SceneNodeKind::Path(ref mut path_node) => {
            if let Err(e) = apply_style(path_node, args.fill, args.stroke) {
                return ToolResult::error(e);
            }
        }
        SceneNodeKind::Text(ref mut text_node) => {
            use photonic_core::node::TextAlign;
            if let Some(content) = args.content {
                text_node.content = content;
            }
            if let Some(ff) = args.font_family {
                text_node.font_family = ff;
            }
            if let Some(fs) = args.font_size {
                text_node.font_size = fs;
            }
            if let Some(fw) = args.font_weight {
                text_node.font_weight = fw;
            }
            if let Some(ref a) = args.text_align {
                text_node.align = match a.as_str() {
                    "center" => TextAlign::Center,
                    "right" => TextAlign::Right,
                    _ => TextAlign::Left,
                };
            }
            if let Some(fill_arg) = args.fill {
                match fill_arg.to_fill() {
                    Ok(f) => text_node.fill = f,
                    Err(e) => return ToolResult::error(e),
                }
            }
            if let Some(stroke_arg) = args.stroke {
                match stroke_arg.to_stroke() {
                    Ok(s) => text_node.stroke = s,
                    Err(e) => return ToolResult::error(e),
                }
            }
        }
        SceneNodeKind::Group(_) => {}
    }

    // Write phase: acquire both locks, execute synchronously, release both.
    let cmd = Command::UpdateNode {
        old: old_node,
        new: new_node,
    };
    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    history.execute(cmd, &mut doc);

    ToolResult::text(format!("Updated node {}", args.node_id))
}

pub async fn delete_nodes(state: &AppState, args: DeleteNodeArgs) -> ToolResult {
    tracing::debug!("tool: delete_nodes (count={})", args.node_ids.len());
    let count = args.node_ids.len();
    // Batch all removals into one command so the doc lock is held only once.
    let cmd = Command::Batch(
        args.node_ids
            .iter()
            .map(|&node_id| Command::RemoveNode { node_id })
            .collect(),
    );
    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    history.execute(cmd, &mut doc);
    ToolResult::text(format!("Deleted {} node(s)", count))
}

pub async fn get_node(state: &AppState, args: GetNodeArgs) -> ToolResult {
    let doc = state.document.lock().await;

    let node = if let Some(id) = args.node_id {
        doc.get_node(&id).cloned()
    } else if let Some(name) = &args.name {
        doc.find_node_by_name(name).cloned()
    } else {
        return ToolResult::error("Provide either node_id or name");
    };

    match node {
        Some(n) => ToolResult::text(format!("Node '{}'", n.name)).with_data(&n),
        None => ToolResult::error("Node not found"),
    }
}

pub async fn build_shape_from_points(
    state: &AppState,
    args: BuildShapeFromPointsArgs,
) -> ToolResult {
    if args.points.len() < 2 {
        return ToolResult::error("At least 2 points are required");
    }

    // Build the ordered index sequence to traverse
    let order: Vec<usize> = match &args.connection_order {
        Some(o) => o.clone(),
        None => (0..args.points.len()).collect(),
    };

    if order.is_empty() {
        return ToolResult::error("connection_order must contain at least one index");
    }

    // Validate all indices
    for &idx in &order {
        if idx >= args.points.len() {
            return ToolResult::error(format!(
                "connection_order index {} is out of bounds (have {} points)",
                idx,
                args.points.len()
            ));
        }
    }

    // Build SVG path string from ordered points
    let first = args.points[order[0]];
    let mut svg = format!("M {} {}", first[0], first[1]);
    for &idx in order.iter().skip(1) {
        let p = args.points[idx];
        svg.push_str(&format!(" L {} {}", p[0], p[1]));
    }
    if args.closed {
        svg.push_str(" Z");
    }

    let path_data = match PathData::from_svg(&svg) {
        Ok(p) => p,
        Err(e) => return ToolResult::error(format!("Failed to build path: {}", e)),
    };

    let mut path_node = PathNode::new(path_data);
    if let Err(e) = apply_style(&mut path_node, args.fill, args.stroke) {
        return ToolResult::error(e);
    }

    let name = args.name.unwrap_or_else(|| "Custom Shape".to_string());
    let mut doc = state.document.lock().await;
    let mut node = SceneNode::new(&name, uuid::Uuid::nil(), SceneNodeKind::Path(path_node));
    if !args.tags.is_empty() {
        node.tags = args.tags;
    }

    let node_id = node.id;
    let cmd = Command::AddNode {
        node,
        layer_id: args.layer_id,
    };
    let mut history = state.history.lock().await;
    history.execute(cmd, &mut doc);

    ToolResult::text(format!(
        "Created '{}' from {} points (id: {})",
        name,
        args.points.len(),
        node_id
    ))
    .with_data(serde_json::json!({
        "node_id": node_id,
    }))
}

pub async fn reorder_node(state: &AppState, args: ReorderNodeArgs) -> ToolResult {
    let mut doc = state.document.lock().await;

    let (layer_id, current_index) = match doc.node_layer_and_index(&args.node_id) {
        Some(v) => v,
        None => return ToolResult::error(format!("Node {} not found", args.node_id)),
    };

    let layer_len = doc
        .layers
        .get(&layer_id)
        .map(|l| l.node_ids.len())
        .unwrap_or(0);
    if layer_len == 0 {
        return ToolResult::error("Layer is empty");
    }

    let new_index = match args.operation {
        ReorderOperation::SendToBack => 0,
        ReorderOperation::BringToFront => layer_len - 1,
        ReorderOperation::SendBackward => current_index.saturating_sub(1),
        ReorderOperation::BringForward => (current_index + 1).min(layer_len - 1),
        ReorderOperation::MoveAbove | ReorderOperation::MoveBelow => {
            let rel_id = match args.relative_id {
                Some(id) => id,
                None => {
                    return ToolResult::error("relative_id is required for move_above / move_below")
                }
            };
            let (rel_layer, rel_index) = match doc.node_layer_and_index(&rel_id) {
                Some(v) => v,
                None => return ToolResult::error(format!("Relative node {} not found", rel_id)),
            };
            if rel_layer != layer_id {
                return ToolResult::error("Nodes must be in the same layer");
            }
            // Compute position in the post-removal list (removing our node first)
            let adj_rel = if current_index < rel_index {
                rel_index - 1
            } else {
                rel_index
            };
            match args.operation {
                ReorderOperation::MoveAbove => (adj_rel + 1).min(layer_len - 1),
                ReorderOperation::MoveBelow => adj_rel,
                _ => unreachable!(),
            }
        }
    };

    let cmd = Command::ReorderNode {
        layer_id,
        node_id: args.node_id,
        old_index: current_index,
        new_index,
    };
    let mut history = state.history.lock().await;
    history.execute(cmd, &mut doc);

    ToolResult::text(format!(
        "Reordered node {} from z-index {} to {}",
        args.node_id, current_index, new_index
    ))
}

pub async fn group_nodes(state: &AppState, args: GroupNodesArgs) -> ToolResult {
    if args.node_ids.len() < 2 {
        return ToolResult::error("group_nodes requires at least 2 node_ids");
    }

    let mut doc = state.document.lock().await;

    let (layer_id, mut indexed) = match doc.nodes_layer_and_indices(&args.node_ids) {
        Some(v) => v,
        None => return ToolResult::error("All nodes must exist and belong to the same layer"),
    };

    // Sort children bottom-to-top (ascending index)
    indexed.sort_by_key(|(_, idx)| *idx);
    let children: Vec<NodeId> = indexed.iter().map(|(id, _)| *id).collect();
    let insert_index = indexed[0].1; // position of bottom-most child

    let group_name = args.name.unwrap_or_else(|| "Group".to_string());
    let group_kind = SceneNodeKind::Group(GroupNode {
        children: children.clone(),
        clip_children: false,
        clip_node_id: None,
        blend_spine_id: None,
    });
    let group = SceneNode::new(&group_name, layer_id, group_kind);
    let group_id = group.id;

    let cmd = Command::GroupNodes {
        group,
        layer_id,
        insert_index,
        children,
    };
    let mut history = state.history.lock().await;
    history.execute(cmd, &mut doc);

    ToolResult::text(format!(
        "Grouped {} nodes into '{}' (id: {})",
        args.node_ids.len(),
        group_name,
        group_id
    ))
    .with_data(serde_json::json!({ "group_id": group_id }))
}

pub async fn ungroup_nodes(state: &AppState, args: UngroupNodesArgs) -> ToolResult {
    let mut doc = state.document.lock().await;

    let group_node = match doc.get_node(&args.group_id) {
        Some(n) => n.clone(),
        None => return ToolResult::error(format!("Node {} not found", args.group_id)),
    };

    let children = match &group_node.kind {
        SceneNodeKind::Group(g) => g.children.clone(),
        _ => return ToolResult::error("Node is not a group"),
    };

    let (layer_id, group_index) = match doc.node_layer_and_index(&args.group_id) {
        Some(v) => v,
        None => return ToolResult::error("Group node has no layer position"),
    };

    let child_count = children.len();
    let cmd = Command::UngroupNodes {
        group: group_node,
        layer_id,
        group_index,
        children,
    };
    let mut history = state.history.lock().await;
    history.execute(cmd, &mut doc);

    ToolResult::text(format!(
        "Ungrouped {} into {} child node(s)",
        args.group_id, child_count
    ))
}

pub async fn boolean_operation(state: &AppState, args: BooleanOperationArgs) -> ToolResult {
    use photonic_core::ops::boolean::boolean_op;

    let mut doc = state.document.lock().await;

    // Clone both nodes
    let target_node = match doc.get_node(&args.target_id) {
        Some(n) => n.clone(),
        None => return ToolResult::error(format!("target node {} not found", args.target_id)),
    };
    let tool_node = match doc.get_node(&args.tool_id) {
        Some(n) => n.clone(),
        None => return ToolResult::error(format!("tool node {} not found", args.tool_id)),
    };

    // Both must be path nodes
    let (target_path_node, tool_path_node) = match (&target_node.kind, &tool_node.kind) {
        (SceneNodeKind::Path(tp), SceneNodeKind::Path(op)) => (tp.clone(), op.clone()),
        _ => return ToolResult::error("Both nodes must be path nodes"),
    };

    // Bake each node's transform into its path data
    let target_baked = apply_affine_to_path(
        &target_path_node.path_data,
        target_node.transform.to_kurbo(),
    );
    let tool_baked =
        apply_affine_to_path(&tool_path_node.path_data, tool_node.transform.to_kurbo());

    let result_path = match boolean_op(&target_baked, &tool_baked, args.operation) {
        Ok(p) => p,
        Err(e) => return ToolResult::error(format!("Boolean operation failed: {}", e)),
    };

    // Determine target's layer and z-position for result placement
    let (layer_id, target_index) = match doc.node_layer_and_index(&args.target_id) {
        Some(v) => v,
        None => return ToolResult::error("Could not determine target node position"),
    };
    let tool_index = doc
        .node_layer_and_index(&args.tool_id)
        .map(|(_, i)| i)
        .unwrap_or(0);

    // Build result node (inherits fill/stroke from target)
    use photonic_core::ops::boolean::BooleanOp;
    let op_name = match args.operation {
        BooleanOp::Union => "union",
        BooleanOp::Subtract => "subtract",
        BooleanOp::Intersect => "intersect",
        BooleanOp::Exclude => "exclude",
        BooleanOp::Divide => "divide",
    };
    let result_name = format!("{} {} {}", target_node.name, op_name, tool_node.name);
    let mut result_path_node = PathNode::new(result_path);
    result_path_node.fill = target_path_node.fill.clone();
    result_path_node.stroke = target_path_node.stroke.clone();

    let result_node = SceneNode::new(
        &result_name,
        layer_id,
        SceneNodeKind::Path(result_path_node),
    );
    let result_id = result_node.id;

    let original_len = doc
        .layers
        .get(&layer_id)
        .map(|l| l.node_ids.len())
        .unwrap_or(2);

    let cmd = if args.keep_originals {
        Command::AddNode {
            node: result_node,
            layer_id: Some(layer_id),
        }
    } else {
        // After removing tool and target, result appends at original_len - 2.
        // Then reorder result to target's original z-position.
        let tool_is_below = tool_index < target_index;
        let adjusted_target = if tool_is_below {
            target_index.saturating_sub(1)
        } else {
            target_index
        };
        let result_pos_after_add = original_len.saturating_sub(2);

        Command::Batch(vec![
            Command::RemoveNode {
                node_id: args.tool_id,
            },
            Command::RemoveNode {
                node_id: args.target_id,
            },
            Command::AddNode {
                node: result_node,
                layer_id: Some(layer_id),
            },
            Command::ReorderNode {
                layer_id,
                node_id: result_id,
                old_index: result_pos_after_add,
                new_index: adjusted_target,
            },
        ])
    };

    let mut history = state.history.lock().await;
    history.execute(cmd, &mut doc);

    ToolResult::text(format!(
        "Boolean {} complete — result '{}' (id: {})",
        op_name, result_name, result_id
    ))
    .with_data(serde_json::json!({ "result_id": result_id }))
}

/// Apply a kurbo Affine transform to every point in a PathData, baking
/// the transform into the path coordinates. Used before boolean operations.
fn apply_affine_to_path(path: &PathData, affine: kurbo::Affine) -> PathData {
    use kurbo::{BezPath, PathEl};
    let mut result = BezPath::new();
    for el in path.to_bez_path().elements() {
        let transformed = match *el {
            PathEl::MoveTo(p) => PathEl::MoveTo(affine * p),
            PathEl::LineTo(p) => PathEl::LineTo(affine * p),
            PathEl::CurveTo(c1, c2, p) => PathEl::CurveTo(affine * c1, affine * c2, affine * p),
            PathEl::QuadTo(c, p) => PathEl::QuadTo(affine * c, affine * p),
            PathEl::ClosePath => PathEl::ClosePath,
        };
        result.push(transformed);
    }
    PathData::from_bez_path(&result)
}

pub async fn apply_transform(state: &AppState, args: ApplyTransformArgs) -> ToolResult {
    tracing::debug!(
        "tool: apply_transform {:?} on {} nodes",
        args.operation,
        args.node_ids.len()
    );
    use photonic_core::{ops::transform_ops, transform::Transform};

    // Read phase: collect the nodes we need, then release the doc lock immediately.
    // Holding a tokio MutexGuard across `.await` blocks the render thread's
    // blocking_lock() call for the entire duration of the loop — causing the
    // window to appear frozen / "Not Responding".
    let old_nodes: Vec<_> = {
        let doc = state.document.lock().await;
        let ids: Vec<_> = if args.node_ids.is_empty() {
            doc.selection.ids().copied().collect()
        } else {
            args.node_ids.clone()
        };
        if ids.is_empty() {
            return ToolResult::error("No nodes specified and no active selection");
        }
        ids.iter()
            .filter_map(|id| doc.get_node(id).cloned())
            .collect()
    }; // doc lock released here

    if old_nodes.is_empty() {
        return ToolResult::error("No nodes specified and no active selection");
    }

    // Prepare phase: compute every transform in-place — no locks held.
    let mut commands: Vec<Command> = Vec::with_capacity(old_nodes.len());
    for node in old_nodes {
        let old = node.clone();
        let mut new_node = node;
        match &args.operation {
            crate::protocol::TransformOperation::Translate => {
                if let Some(t) = &args.translate {
                    transform_ops::translate(&mut new_node, t.x, t.y);
                }
            }
            crate::protocol::TransformOperation::Rotate => {
                if let Some(r) = &args.rotate {
                    transform_ops::rotate(&mut new_node, r.angle_degrees, r.origin_x, r.origin_y);
                }
            }
            crate::protocol::TransformOperation::Scale => {
                if let Some(s) = &args.scale {
                    transform_ops::scale(&mut new_node, s.sx, s.sy, s.origin_x, s.origin_y);
                }
            }
            crate::protocol::TransformOperation::Matrix => {
                if let Some(m) = args.matrix {
                    transform_ops::set_transform(&mut new_node, Transform { matrix: m });
                }
            }
            crate::protocol::TransformOperation::ReflectHorizontal => {
                let cx = new_node
                    .local_bounds()
                    .map(|b| b.x0 + b.width() / 2.0)
                    .unwrap_or(0.0);
                transform_ops::reflect_horizontal(&mut new_node, cx);
            }
            crate::protocol::TransformOperation::ReflectVertical => {
                let cy = new_node
                    .local_bounds()
                    .map(|b| b.y0 + b.height() / 2.0)
                    .unwrap_or(0.0);
                transform_ops::reflect_vertical(&mut new_node, cy);
            }
            crate::protocol::TransformOperation::Shear => {
                if let Some(s) = &args.shear {
                    transform_ops::shear(
                        &mut new_node,
                        s.shear_x,
                        s.shear_y,
                        s.origin_x,
                        s.origin_y,
                    );
                }
            }
        }
        commands.push(Command::UpdateNode { old, new: new_node });
    }

    // Write phase: acquire both locks once, apply all updates as a single
    // batch, then release both immediately. No `.await` between the two lock
    // acquisitions so the render thread is unblocked as quickly as possible.
    let node_count = commands.len();
    let cmd = Command::Batch(commands);
    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    history.execute(cmd, &mut doc);

    ToolResult::text(format!("Transformed {} node(s)", node_count))
}

/// Align or distribute multiple nodes by their world-space bounding boxes.
///
/// Alignment snaps each node's edge/center to the reference edge/center.
/// Distribution evenly spaces nodes between the two extreme nodes (which stay fixed).
pub async fn align_nodes(state: &AppState, args: AlignNodesArgs) -> ToolResult {
    use photonic_core::transform::Transform;

    if args.node_ids.len() < 2 {
        return ToolResult::error("align_nodes requires at least 2 node IDs");
    }

    // Read phase: clone nodes and capture canvas dimensions under a brief lock.
    let (nodes, canvas_w, canvas_h) = {
        let doc = state.document.lock().await;
        let nodes: Vec<SceneNode> = args
            .node_ids
            .iter()
            .filter_map(|id| doc.nodes.get(id).cloned())
            .collect();
        (nodes, doc.width, doc.height)
    };

    if nodes.len() < 2 {
        return ToolResult::error(format!(
            "Could not find enough nodes — requested {}, found {}",
            args.node_ids.len(),
            nodes.len()
        ));
    }

    // Compute the world-space axis-aligned bounding box for a node.
    // The node's transform is applied to all four corners of the local bbox.
    let world_bounds = |node: &SceneNode| -> Option<(f64, f64, f64, f64)> {
        let local = node.local_bounds()?;
        let corners = [
            (local.x0, local.y0),
            (local.x1, local.y0),
            (local.x1, local.y1),
            (local.x0, local.y1),
        ];
        let pts: Vec<(f64, f64)> = corners
            .iter()
            .map(|(x, y)| node.transform.apply(*x, *y))
            .collect();
        let min_x = pts.iter().map(|(x, _)| *x).fold(f64::INFINITY, f64::min);
        let min_y = pts.iter().map(|(_, y)| *y).fold(f64::INFINITY, f64::min);
        let max_x = pts
            .iter()
            .map(|(x, _)| *x)
            .fold(f64::NEG_INFINITY, f64::max);
        let max_y = pts
            .iter()
            .map(|(_, y)| *y)
            .fold(f64::NEG_INFINITY, f64::max);
        Some((min_x, min_y, max_x, max_y))
    };

    // Pair each node with its world bounds, skipping nodes without computable bounds (groups).
    let node_bounds: Vec<(SceneNode, (f64, f64, f64, f64))> = nodes
        .iter()
        .filter_map(|n| world_bounds(n).map(|b| (n.clone(), b)))
        .collect();

    if node_bounds.is_empty() {
        return ToolResult::error(
            "None of the specified nodes have computable bounds (groups are not supported)",
        );
    }

    // Reference rectangle used as the alignment target.
    let (ref_x0, ref_y0, ref_x1, ref_y1) = match args.anchor {
        AlignAnchor::Canvas => (0.0, 0.0, canvas_w, canvas_h),
        AlignAnchor::KeyObject => {
            // Use the bounds of the designated key object as the fixed reference.
            // Fall back to selection bounds if key_object_id is absent or not found.
            if let Some(key_id) = args.key_object_id {
                if let Some((_, b)) = node_bounds.iter().find(|(n, _)| n.id == key_id) {
                    (b.0, b.1, b.2, b.3)
                } else {
                    // Key object not in the resolved set — fall back to selection.
                    let x0 = node_bounds
                        .iter()
                        .map(|(_, b)| b.0)
                        .fold(f64::INFINITY, f64::min);
                    let y0 = node_bounds
                        .iter()
                        .map(|(_, b)| b.1)
                        .fold(f64::INFINITY, f64::min);
                    let x1 = node_bounds
                        .iter()
                        .map(|(_, b)| b.2)
                        .fold(f64::NEG_INFINITY, f64::max);
                    let y1 = node_bounds
                        .iter()
                        .map(|(_, b)| b.3)
                        .fold(f64::NEG_INFINITY, f64::max);
                    (x0, y0, x1, y1)
                }
            } else {
                // No key_object_id — treat as selection.
                let x0 = node_bounds
                    .iter()
                    .map(|(_, b)| b.0)
                    .fold(f64::INFINITY, f64::min);
                let y0 = node_bounds
                    .iter()
                    .map(|(_, b)| b.1)
                    .fold(f64::INFINITY, f64::min);
                let x1 = node_bounds
                    .iter()
                    .map(|(_, b)| b.2)
                    .fold(f64::NEG_INFINITY, f64::max);
                let y1 = node_bounds
                    .iter()
                    .map(|(_, b)| b.3)
                    .fold(f64::NEG_INFINITY, f64::max);
                (x0, y0, x1, y1)
            }
        }
        AlignAnchor::Selection => {
            let x0 = node_bounds
                .iter()
                .map(|(_, b)| b.0)
                .fold(f64::INFINITY, f64::min);
            let y0 = node_bounds
                .iter()
                .map(|(_, b)| b.1)
                .fold(f64::INFINITY, f64::min);
            let x1 = node_bounds
                .iter()
                .map(|(_, b)| b.2)
                .fold(f64::NEG_INFINITY, f64::max);
            let y1 = node_bounds
                .iter()
                .map(|(_, b)| b.3)
                .fold(f64::NEG_INFINITY, f64::max);
            (x0, y0, x1, y1)
        }
    };

    // Compute phase: build UpdateNode commands for each affected node.
    let commands: Vec<Command> = match args.operation {
        AlignOperation::DistributeHorizontal => {
            let mut sorted = node_bounds.clone();
            sorted.sort_by(|(_, a), (_, b)| {
                a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal)
            });
            let n = sorted.len();
            let gap = if let Some(s) = args.spacing {
                s
            } else {
                let total_w: f64 = sorted.iter().map(|(_, b)| b.2 - b.0).sum();
                let avail = sorted[n - 1].1 .2 - sorted[0].1 .0;
                (avail - total_w) / (n - 1).max(1) as f64
            };
            // First node is always the anchor; subsequent nodes are placed relative to it.
            let mut cursor = sorted[0].1 .0;
            let mut cmds = Vec::new();
            for (node, bounds) in &sorted {
                let w = bounds.2 - bounds.0;
                let dx = cursor - bounds.0;
                cursor += w + gap;
                if dx.abs() > 1e-9 {
                    let old = node.clone();
                    let mut new = old.clone();
                    new.transform = new.transform.then(&Transform::translate(dx, 0.0));
                    cmds.push(Command::UpdateNode { old, new });
                }
            }
            cmds
        }
        AlignOperation::DistributeVertical => {
            let mut sorted = node_bounds.clone();
            sorted.sort_by(|(_, a), (_, b)| {
                a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal)
            });
            let n = sorted.len();
            let gap = if let Some(s) = args.spacing {
                s
            } else {
                let total_h: f64 = sorted.iter().map(|(_, b)| b.3 - b.1).sum();
                let avail = sorted[n - 1].1 .3 - sorted[0].1 .1;
                (avail - total_h) / (n - 1).max(1) as f64
            };
            // First node is always the anchor; subsequent nodes are placed relative to it.
            let mut cursor = sorted[0].1 .1;
            let mut cmds = Vec::new();
            for (node, bounds) in &sorted {
                let h = bounds.3 - bounds.1;
                let dy = cursor - bounds.1;
                cursor += h + gap;
                if dy.abs() > 1e-9 {
                    let old = node.clone();
                    let mut new = old.clone();
                    new.transform = new.transform.then(&Transform::translate(0.0, dy));
                    cmds.push(Command::UpdateNode { old, new });
                }
            }
            cmds
        }
        _ => {
            // Positional alignments: snap each node to one edge or center of the reference rect.
            let ref_cx = (ref_x0 + ref_x1) / 2.0;
            let ref_cy = (ref_y0 + ref_y1) / 2.0;
            node_bounds
                .iter()
                .filter_map(|(node, bounds)| {
                    let (nx0, ny0, nx1, ny1) = *bounds;
                    let ncx = (nx0 + nx1) / 2.0;
                    let ncy = (ny0 + ny1) / 2.0;
                    let (dx, dy) = match args.operation {
                        AlignOperation::Left => (ref_x0 - nx0, 0.0),
                        AlignOperation::CenterHorizontal => (ref_cx - ncx, 0.0),
                        AlignOperation::Right => (ref_x1 - nx1, 0.0),
                        AlignOperation::Top => (0.0, ref_y0 - ny0),
                        AlignOperation::CenterVertical => (0.0, ref_cy - ncy),
                        AlignOperation::Bottom => (0.0, ref_y1 - ny1),
                        _ => unreachable!(),
                    };
                    if dx.abs() < 1e-9 && dy.abs() < 1e-9 {
                        return None;
                    }
                    let old = node.clone();
                    let mut new = old.clone();
                    new.transform = new.transform.then(&Transform::translate(dx, dy));
                    Some(Command::UpdateNode { old, new })
                })
                .collect()
        }
    };

    if commands.is_empty() {
        return ToolResult::text("All nodes are already aligned — no changes made");
    }

    let moved = commands.len();
    let batch = Command::Batch(commands);
    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    history.execute(batch, &mut doc);

    let op_name = match args.operation {
        AlignOperation::Left => "left",
        AlignOperation::CenterHorizontal => "center_horizontal",
        AlignOperation::Right => "right",
        AlignOperation::Top => "top",
        AlignOperation::CenterVertical => "center_vertical",
        AlignOperation::Bottom => "bottom",
        AlignOperation::DistributeHorizontal => "distribute_horizontal",
        AlignOperation::DistributeVertical => "distribute_vertical",
    };
    let anchor_name = match args.anchor {
        AlignAnchor::Selection => "selection",
        AlignAnchor::Canvas => "canvas",
        AlignAnchor::KeyObject => "key_object",
    };
    let spacing_note = match args.spacing {
        Some(s)
            if matches!(
                args.operation,
                AlignOperation::DistributeHorizontal | AlignOperation::DistributeVertical
            ) =>
        {
            format!(", spacing: {}px", s)
        }
        _ => String::new(),
    };
    ToolResult::text(format!(
        "Aligned {} node(s) — operation: {}, anchor: {}{}",
        moved, op_name, anchor_name, spacing_note
    ))
}

pub async fn find_nodes(state: &AppState, args: FindNodesArgs) -> ToolResult {
    tracing::debug!("tool: find_nodes");
    let doc = state.document.lock().await;

    let limit = args.limit.unwrap_or(200).max(1);
    let visible_only = args.visible_only.unwrap_or(false);
    let include_details = args.include_details.unwrap_or(false);
    let name_lower = args.name_contains.as_deref().map(|s| s.to_lowercase());

    let region_rect: Option<kurbo::Rect> = args
        .in_region
        .as_ref()
        .map(|r| kurbo::Rect::new(r.x, r.y, r.x + r.width, r.y + r.height));

    let mut matched: Vec<serde_json::Value> = Vec::new();
    let mut truncated = false;

    'outer: for node in doc.nodes.values() {
        if visible_only && !node.visible {
            continue;
        }

        if let Some(lid) = args.layer_id {
            if node.layer_id != lid {
                continue;
            }
        }

        if let Some(ref nt) = args.node_type {
            let kind_str = match &node.kind {
                SceneNodeKind::Path(_) => "path",
                SceneNodeKind::Group(_) => "group",
                SceneNodeKind::Text(_) => "text",
            };
            if kind_str != nt.as_str() {
                continue;
            }
        }

        if let Some(ref required) = args.tags {
            if !required.iter().all(|t| node.tags.contains(t)) {
                continue;
            }
        }

        if let Some(ref any) = args.tags_any {
            if !any.is_empty() && !any.iter().any(|t| node.tags.contains(t)) {
                continue;
            }
        }

        if let Some(ref needle) = name_lower {
            if !node.name.to_lowercase().contains(needle.as_str()) {
                continue;
            }
        }

        // Spatial filter: groups/text have no local_bounds → always pass.
        if let Some(filter_rect) = region_rect {
            if let Some(lb) = node.local_bounds() {
                let t = &node.transform;
                let corners = [
                    t.apply(lb.x0, lb.y0),
                    t.apply(lb.x1, lb.y0),
                    t.apply(lb.x0, lb.y1),
                    t.apply(lb.x1, lb.y1),
                ];
                let wx0 = corners
                    .iter()
                    .map(|(x, _)| *x)
                    .fold(f64::INFINITY, f64::min);
                let wy0 = corners
                    .iter()
                    .map(|(_, y)| *y)
                    .fold(f64::INFINITY, f64::min);
                let wx1 = corners
                    .iter()
                    .map(|(x, _)| *x)
                    .fold(f64::NEG_INFINITY, f64::max);
                let wy1 = corners
                    .iter()
                    .map(|(_, y)| *y)
                    .fold(f64::NEG_INFINITY, f64::max);
                let no_overlap = wx1 < filter_rect.x0
                    || wx0 > filter_rect.x1
                    || wy1 < filter_rect.y0
                    || wy0 > filter_rect.y1;
                if no_overlap {
                    continue;
                }
            }
        }

        let entry = if include_details {
            serde_json::to_value(node).unwrap_or_default()
        } else {
            let kind_str = match &node.kind {
                SceneNodeKind::Path(_) => "path",
                SceneNodeKind::Group(_) => "group",
                SceneNodeKind::Text(_) => "text",
            };
            serde_json::json!({
                "id":       node.id,
                "name":     node.name,
                "type":     kind_str,
                "tags":     node.tags,
                "layer_id": node.layer_id,
                "visible":  node.visible,
            })
        };
        matched.push(entry);

        if matched.len() >= limit {
            truncated = true;
            break 'outer;
        }
    }

    let count = matched.len();
    ToolResult::text(format!(
        "Found {} node(s){}",
        count,
        if truncated {
            " (results truncated)"
        } else {
            ""
        }
    ))
    .with_data(serde_json::json!({
        "nodes":     matched,
        "count":     count,
        "truncated": truncated,
    }))
}

/// Deep-clone a node subtree rooted at `root_id`, remapping all IDs to fresh UUIDs.
///
/// Returns a flat `Vec<SceneNode>` in add-order: root first, then descendants (DFS).
/// The returned root node already has its `layer_id` set to `target_layer`.
/// An incremental translate of `(dx, dy)` is composed onto the root's existing transform.
fn clone_subtree(
    doc: &photonic_core::document::Document,
    root_id: uuid::Uuid,
    target_layer: uuid::Uuid,
    dx: f64,
    dy: f64,
) -> Vec<SceneNode> {
    use photonic_core::transform::Transform;
    use std::collections::HashMap;

    // Collect nodes in DFS order (root first).
    let mut visit_order: Vec<uuid::Uuid> = Vec::new();
    let mut stack = vec![root_id];
    while let Some(id) = stack.pop() {
        if let Some(node) = doc.nodes.get(&id) {
            visit_order.push(id);
            if let SceneNodeKind::Group(ref g) = node.kind {
                // Push children in reverse so they come out in correct order.
                for child_id in g.children.iter().rev() {
                    stack.push(*child_id);
                }
            }
        }
    }

    // Build old→new ID mapping.
    let id_map: HashMap<uuid::Uuid, uuid::Uuid> = visit_order
        .iter()
        .map(|old| (*old, uuid::Uuid::new_v4()))
        .collect();

    // Clone each node, remapping IDs and children.
    let mut result = Vec::with_capacity(visit_order.len());
    for (idx, old_id) in visit_order.iter().enumerate() {
        if let Some(src) = doc.nodes.get(old_id) {
            let mut cloned = src.clone();
            cloned.id = id_map[old_id];

            if idx == 0 {
                // Root: apply target layer and offset transform.
                cloned.layer_id = target_layer;
                cloned.transform = cloned.transform.then(&Transform::translate(dx, dy));
            } else {
                // Non-root children stay in whatever layer the group tracks them in,
                // but their parent group's reference is via children list, not layer.
                // Keep the original layer_id (they're owned by the group, not the layer).
            }

            // Remap group children.
            if let SceneNodeKind::Group(ref mut g) = cloned.kind {
                g.children = g.children.iter().map(|cid| id_map[cid]).collect();
            }

            result.push(cloned);
        }
    }
    result
}

/// Duplicate one or more nodes, optionally creating multiple offset copies.
///
/// Each copy is a full deep clone (groups and all descendants get fresh UUIDs).
/// Copy N is shifted by N × offset from the original position.
/// All copies land in a single undoable batch.
pub async fn duplicate_nodes(state: &AppState, args: DuplicateNodesArgs) -> ToolResult {
    let count = args.count.unwrap_or(1).clamp(1, 100);
    let offset_x = args.offset.as_ref().map(|o| o.x).unwrap_or(10.0);
    let offset_y = args.offset.as_ref().map(|o| o.y).unwrap_or(10.0);

    // Read phase: validate IDs and collect source layer_ids.
    let source_info: Vec<(uuid::Uuid, uuid::Uuid)> = {
        let doc = state.document.lock().await;
        let mut out = Vec::new();
        for id in &args.node_ids {
            match doc.nodes.get(id) {
                Some(n) => out.push((*id, n.layer_id)),
                None => return ToolResult::error(format!("Node {} not found", id)),
            }
        }
        out
    };

    // Clone phase: build all AddNode commands without holding any lock.
    let mut commands: Vec<Command> = Vec::new();
    let mut root_ids: Vec<uuid::Uuid> = Vec::new();

    for copy_idx in 1..=count {
        let dx = offset_x * copy_idx as f64;
        let dy = offset_y * copy_idx as f64;

        // Acquire a read-only snapshot of the document for this copy pass.
        let doc = state.document.lock().await;

        for (src_id, src_layer) in &source_info {
            let target_layer = args.layer_id.unwrap_or(*src_layer);
            let nodes = clone_subtree(&doc, *src_id, target_layer, dx, dy);
            if let Some(root) = nodes.first() {
                root_ids.push(root.id);
            }
            for node in nodes {
                commands.push(Command::AddNode {
                    layer_id: Some(node.layer_id),
                    node,
                });
            }
        }

        drop(doc); // Release before next iteration
    }

    if commands.is_empty() {
        return ToolResult::error("Nothing to duplicate");
    }

    // Write phase: execute as a single batch for a clean one-step undo.
    let cmd = Command::Batch(commands);
    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    history.execute(cmd, &mut doc);

    let total_roots = root_ids.len();
    ToolResult::text(format!(
        "Duplicated {} source node(s) × {} copies = {} new root node(s)",
        source_info.len(),
        count,
        total_roots
    ))
    .with_data(serde_json::json!({ "node_ids": root_ids }))
}

/// Repeat a node in a grid or radial pattern, producing a single undoable batch.
///
/// **Grid mode**: The source is treated as cell (row 0, col 0). A new clone is
/// created for every other cell, translated by (col × col_stride, row × row_stride).
///
/// **Radial mode**: The source is treated as instance 0. Each additional instance i
/// is the source rotated around (center_x, center_y) by (i × 360 / count) degrees,
/// so the total visual count (source + copies) equals `count`.
///
/// When `group_result` is true the source and all copies are wrapped into a new
/// group node as part of the same undo step.
pub async fn create_array(state: &AppState, args: CreateArrayArgs) -> ToolResult {
    use photonic_core::transform::Transform;

    // ── Read phase: validate source ───────────────────────────────────────
    let (src_id, src_layer, src_name, src_z) = {
        let doc = state.document.lock().await;
        match doc.nodes.get(&args.node_id) {
            Some(n) => {
                let z = doc.node_layer_and_index(&n.id).map(|(_, i)| i).unwrap_or(0);
                (n.id, n.layer_id, n.name.clone(), z)
            }
            None => return ToolResult::error(format!("Source node {} not found", args.node_id)),
        }
    };

    let target_layer = args.layer_id.unwrap_or(src_layer);
    let prefix = args.name_prefix.unwrap_or_else(|| src_name.clone());

    // ── Compute per-copy transforms ───────────────────────────────────────
    // Each transform is applied on top of the source's existing transform.
    // The source is NOT moved — it is implicitly "instance 0".
    let copy_transforms: Vec<(String, Transform)> = match args.mode {
        ArrayMode::Grid => {
            let rows = args.rows.unwrap_or(2).max(1);
            let cols = args.cols.unwrap_or(2).max(1);
            if rows * cols < 2 {
                return ToolResult::error("Grid must have at least 2 cells (rows × cols ≥ 2)");
            }
            let dx = args.col_stride.unwrap_or(100.0);
            let dy = args.row_stride.unwrap_or(100.0);

            let mut out = Vec::with_capacity(rows * cols - 1);
            let mut n = 1usize;
            for r in 0..rows {
                for c in 0..cols {
                    if r == 0 && c == 0 {
                        continue; // source already occupies (0, 0)
                    }
                    out.push((
                        format!("{} {}", prefix, n),
                        Transform::translate(c as f64 * dx, r as f64 * dy),
                    ));
                    n += 1;
                }
            }
            out
        }

        ArrayMode::Radial => {
            let count = args.count.unwrap_or(6);
            if count < 2 {
                return ToolResult::error("Radial count must be ≥ 2");
            }
            let cx = args.center_x.unwrap_or(0.0);
            let cy = args.center_y.unwrap_or(0.0);
            let start_deg = args.start_angle_degrees.unwrap_or(0.0);
            let step_deg = 360.0 / count as f64;

            (1..count)
                .map(|i| {
                    let angle_rad = (start_deg + i as f64 * step_deg).to_radians();
                    (
                        format!("{} {}", prefix, i),
                        Transform::rotate_around(angle_rad, cx, cy),
                    )
                })
                .collect()
        }
    };

    if copy_transforms.is_empty() {
        return ToolResult::error("No copies to create");
    }

    // ── Clone phase ────────────────────────────────────────────────────────
    let mut commands: Vec<Command> = Vec::new();
    let mut new_root_ids: Vec<uuid::Uuid> = Vec::new();

    for (copy_name, extra_transform) in &copy_transforms {
        // Acquire a fresh snapshot for each clone pass (matches duplicate_nodes pattern).
        let doc = state.document.lock().await;
        // clone_subtree with (dx=0, dy=0) preserves the source's transform exactly;
        // we then compose our extra_transform on top of it.
        let mut nodes = clone_subtree(&doc, src_id, target_layer, 0.0, 0.0);
        drop(doc);

        if let Some(root) = nodes.first_mut() {
            root.name = copy_name.clone();
            root.transform = root.transform.then(extra_transform);
            new_root_ids.push(root.id);
        }

        for node in nodes {
            commands.push(Command::AddNode {
                layer_id: Some(node.layer_id),
                node,
            });
        }
    }

    // ── Optional group ─────────────────────────────────────────────────────
    // Runs AFTER all AddNodes so every child already exists in the document.
    let mut group_id: Option<uuid::Uuid> = None;
    if args.group_result {
        let gid = uuid::Uuid::new_v4();
        let mut all_children = vec![src_id];
        all_children.extend_from_slice(&new_root_ids);

        let group_kind = SceneNodeKind::Group(GroupNode {
            children: all_children.clone(),
            clip_children: false,
            clip_node_id: None,
            blend_spine_id: None,
        });
        let group_name = format!("{} Array", src_name);
        let mut group_node = SceneNode::new(&group_name, target_layer, group_kind);
        group_node.id = gid;

        // insert_index: place the group where the source currently lives.
        // After GroupNodes removes source + copies from the layer and inserts the
        // group at src_z, the result sits at the same z-stack position.
        commands.push(Command::GroupNodes {
            group: group_node,
            layer_id: target_layer,
            insert_index: src_z,
            children: all_children,
        });

        group_id = Some(gid);
    }

    // ── Write phase ────────────────────────────────────────────────────────
    let cmd = Command::Batch(commands);
    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    history.execute(cmd, &mut doc);

    let mode_label = match args.mode {
        ArrayMode::Grid => "grid",
        ArrayMode::Radial => "radial",
    };
    ToolResult::text(format!(
        "Created {} {} array: {} new copies of '{}'{}",
        mode_label,
        mode_label,
        new_root_ids.len(),
        src_name,
        if group_id.is_some() { " (grouped)" } else { "" },
    ))
    .with_data(serde_json::json!({
        "source_id": src_id,
        "node_ids":  new_root_ids,
        "group_id":  group_id,
    }))
}

/// Copy the visual style of one node onto many targets in a single undoable step.
///
/// Copyable properties: fill, stroke (path nodes only), opacity, blend_mode (all node types).
/// Pass `properties` to copy a subset; omit it to copy all four.
pub async fn style_transfer(state: &AppState, args: StyleTransferArgs) -> ToolResult {
    tracing::debug!("tool: style_transfer (targets={})", args.target_ids.len());

    if args.target_ids.is_empty() {
        return ToolResult::error("target_ids must contain at least one node ID");
    }

    // ── Read phase ─────────────────────────────────────────────────────────
    let (source_node, target_nodes) = {
        let doc = state.document.lock().await;
        let source = match doc.get_node(&args.source_id).cloned() {
            Some(n) => n,
            None => return ToolResult::error(format!("Source node {} not found", args.source_id)),
        };
        let targets: Vec<SceneNode> = args
            .target_ids
            .iter()
            .filter_map(|id| doc.get_node(id).cloned())
            .collect();
        (source, targets)
    };

    if target_nodes.is_empty() {
        return ToolResult::error("None of the target_ids were found in the document");
    }

    // ── Prepare phase ──────────────────────────────────────────────────────
    let copy_fill = style_prop_enabled(&args.properties, "fill");
    let copy_stroke = style_prop_enabled(&args.properties, "stroke");
    let copy_opacity = style_prop_enabled(&args.properties, "opacity");
    let copy_blend_mode = style_prop_enabled(&args.properties, "blend_mode");

    // Extract source path-level style once (only meaningful if source is a Path).
    let src_fill = if copy_fill {
        if let SceneNodeKind::Path(ref p) = source_node.kind {
            Some(p.fill.clone())
        } else {
            None
        }
    } else {
        None
    };
    let src_stroke = if copy_stroke {
        if let SceneNodeKind::Path(ref p) = source_node.kind {
            Some(p.stroke.clone())
        } else {
            None
        }
    } else {
        None
    };

    let mut commands: Vec<Command> = Vec::with_capacity(target_nodes.len());

    for old_node in target_nodes {
        let mut new_node = old_node.clone();

        if copy_opacity {
            new_node.opacity = source_node.opacity;
        }
        if copy_blend_mode {
            // Blend modes other than Normal are not yet rendered; always apply Normal.
            new_node.blend_mode = BlendMode::Normal;
        }
        if let SceneNodeKind::Path(ref mut tp) = new_node.kind {
            if let Some(ref fill) = src_fill {
                tp.fill = fill.clone();
            }
            if let Some(ref stroke) = src_stroke {
                tp.stroke = stroke.clone();
            }
        }

        commands.push(Command::UpdateNode {
            old: old_node,
            new: new_node,
        });
    }

    let updated = commands.len();

    // ── Write phase ────────────────────────────────────────────────────────
    let cmd = Command::Batch(commands);
    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    history.execute(cmd, &mut doc);

    ToolResult::text(format!(
        "Style transferred from '{}' to {} node(s)",
        source_node.name, updated
    ))
    .with_data(serde_json::json!({
        "source_id": args.source_id,
        "updated":   updated,
    }))
}

/// Returns true if `prop` should be copied given the optional property filter list.
/// An absent or empty list means "copy everything".
fn style_prop_enabled(properties: &Option<Vec<String>>, prop: &str) -> bool {
    match properties {
        None => true,
        Some(v) if v.is_empty() => true,
        Some(v) => v.iter().any(|p| p == prop),
    }
}

/// Measure the world-space bounding boxes and spatial relationships of one or
/// more nodes. Applies each node's transform to its local bounds to produce the
/// actual axis-aligned bounding box (AABB) on screen.
///
/// Returns per-node `world_bounds` and `center`, the `combined_bounds` of the
/// entire selection, and — when exactly two nodes are provided — pairwise
/// `center_to_center_distance` and `angle_degrees` (0° = right, 90° = down).
pub async fn measure_nodes(
    state: &AppState,
    args: crate::protocol::MeasureNodesArgs,
) -> ToolResult {
    if args.node_ids.is_empty() {
        return ToolResult::error("node_ids must not be empty");
    }

    /// Transform a node's local AABB into world space by applying its affine
    /// transform to all four corners and taking the bounding box of the result.
    fn world_aabb(node: &SceneNode) -> Option<[f64; 4]> {
        let local = node.local_bounds()?;
        let affine = node.transform.to_kurbo();
        let pts = [
            affine * kurbo::Point::new(local.x0, local.y0),
            affine * kurbo::Point::new(local.x1, local.y0),
            affine * kurbo::Point::new(local.x1, local.y1),
            affine * kurbo::Point::new(local.x0, local.y1),
        ];
        let x0 = pts.iter().map(|p| p.x).fold(f64::INFINITY, f64::min);
        let y0 = pts.iter().map(|p| p.y).fold(f64::INFINITY, f64::min);
        let x1 = pts.iter().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max);
        let y1 = pts.iter().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max);
        Some([x0, y0, x1 - x0, y1 - y0])
    }

    fn r2(v: f64) -> f64 {
        (v * 100.0).round() / 100.0
    }

    // Collect measurements under a single read lock.
    struct Item {
        id: uuid::Uuid,
        name: String,
        aabb: Option<[f64; 4]>,
    }

    let items: Vec<Item> = {
        let doc = state.document.lock().await;
        let mut out = Vec::with_capacity(args.node_ids.len());
        for id in &args.node_ids {
            let Some(node) = doc.get_node(id) else {
                return ToolResult::error(format!("Node not found: {}", id));
            };
            out.push(Item {
                id: *id,
                name: node.name.clone(),
                aabb: world_aabb(node),
            });
        }
        out
    };

    // Combined bounding box over all nodes that have known bounds.
    let combined = {
        let rects: Vec<[f64; 4]> = items.iter().filter_map(|m| m.aabb).collect();
        if rects.is_empty() {
            None
        } else {
            let x0 = rects.iter().map(|r| r[0]).fold(f64::INFINITY, f64::min);
            let y0 = rects.iter().map(|r| r[1]).fold(f64::INFINITY, f64::min);
            let x1 = rects
                .iter()
                .map(|r| r[0] + r[2])
                .fold(f64::NEG_INFINITY, f64::max);
            let y1 = rects
                .iter()
                .map(|r| r[1] + r[3])
                .fold(f64::NEG_INFINITY, f64::max);
            Some([x0, y0, x1 - x0, y1 - y0])
        }
    };

    // Pairwise metrics only when exactly two nodes are given.
    let pairwise = if items.len() == 2 {
        let center = |aabb: [f64; 4]| (aabb[0] + aabb[2] / 2.0, aabb[1] + aabb[3] / 2.0);
        match (items[0].aabb, items[1].aabb) {
            (Some(a), Some(b)) => {
                let (ax, ay) = center(a);
                let (bx, by) = center(b);
                let dx = bx - ax;
                let dy = by - ay;
                let dist = (dx * dx + dy * dy).sqrt();
                let angle = dy.atan2(dx).to_degrees();
                Some(serde_json::json!({
                    "center_to_center_distance": r2(dist),
                    "angle_degrees": r2(angle),
                }))
            }
            _ => None,
        }
    } else {
        None
    };

    // Serialize per-node results.
    let nodes_json: Vec<_> = items
        .iter()
        .map(|m| {
            let bounds_json = m.aabb.map(|[x, y, w, h]| {
                serde_json::json!({ "x": r2(x), "y": r2(y), "width": r2(w), "height": r2(h) })
            });
            let center_json = m.aabb.map(
                |[x, y, w, h]| serde_json::json!({ "x": r2(x + w / 2.0), "y": r2(y + h / 2.0) }),
            );
            serde_json::json!({
                "id": m.id,
                "name": m.name,
                "world_bounds": bounds_json,
                "center": center_json,
            })
        })
        .collect();

    let combined_json = combined.map(|[x, y, w, h]| {
        serde_json::json!({ "x": r2(x), "y": r2(y), "width": r2(w), "height": r2(h) })
    });

    let mut data = serde_json::json!({
        "nodes": nodes_json,
        "combined_bounds": combined_json,
    });
    if let Some(p) = pairwise {
        data["pairwise"] = p;
    }

    ToolResult::text(format!("Measured {} node(s)", items.len())).with_data(data)
}

/// Resize a node to exact pixel dimensions in one step.
///
/// Eliminates the two-round-trip pattern of `measure_nodes` → compute scale →
/// `apply_transform`. The world-space AABB of the node is computed internally;
/// a scale transform is derived and composed onto the node's existing transform
/// so that the result has the requested dimensions.
pub async fn set_node_size(state: &AppState, args: crate::protocol::SetNodeSizeArgs) -> ToolResult {
    use crate::protocol::SizeAnchor;
    use photonic_core::{history::Command, transform::Transform};

    // ── 1. Validate args ─────────────────────────────────────────────────────
    if args.width.is_none() && args.height.is_none() {
        return ToolResult::error("At least one of `width` or `height` must be provided");
    }
    if let Some(w) = args.width {
        if w <= 0.0 {
            return ToolResult::error("`width` must be greater than 0");
        }
    }
    if let Some(h) = args.height {
        if h <= 0.0 {
            return ToolResult::error("`height` must be greater than 0");
        }
    }

    // ── 2. Compute world AABB (same logic as `measure_nodes`) ────────────────
    let (old_node, aabb) = {
        let doc = state.document.lock().await;
        let Some(node) = doc.get_node(&args.node_id) else {
            return ToolResult::error(format!("Node not found: {}", args.node_id));
        };
        let Some(local) = node.local_bounds() else {
            return ToolResult::error(
                "Cannot resize this node — it has no computable bounding box (e.g. empty group)",
            );
        };
        let affine = node.transform.to_kurbo();
        let pts = [
            affine * kurbo::Point::new(local.x0, local.y0),
            affine * kurbo::Point::new(local.x1, local.y0),
            affine * kurbo::Point::new(local.x1, local.y1),
            affine * kurbo::Point::new(local.x0, local.y1),
        ];
        let x0 = pts.iter().map(|p| p.x).fold(f64::INFINITY, f64::min);
        let y0 = pts.iter().map(|p| p.y).fold(f64::INFINITY, f64::min);
        let x1 = pts.iter().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max);
        let y1 = pts.iter().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max);
        (node.clone(), [x0, y0, x1 - x0, y1 - y0])
    };

    let [ax, ay, cur_w, cur_h] = aabb;

    if cur_w < 1e-9 || cur_h < 1e-9 {
        return ToolResult::error(
            "Cannot resize: the node's bounding box has zero or near-zero dimensions",
        );
    }

    // ── 3. Compute scale factors ─────────────────────────────────────────────
    let (mut sx, mut sy) = match (args.width, args.height) {
        (Some(tw), Some(th)) => (tw / cur_w, th / cur_h),
        (Some(tw), None) => (tw / cur_w, tw / cur_w), // both uniform until aspect check
        (None, Some(th)) => (th / cur_h, th / cur_h),
        (None, None) => unreachable!(),
    };

    // When both dimensions are given and aspect ratio must be maintained, fit
    // inside the requested box (use the smaller of the two scale factors).
    if args.maintain_aspect_ratio {
        if let (Some(tw), Some(th)) = (args.width, args.height) {
            let s = (tw / cur_w).min(th / cur_h);
            sx = s;
            sy = s;
        }
        // single-dimension + maintain_aspect_ratio: already set sx==sy above
    } else if args.width.is_some() && args.height.is_some() {
        // both given, no aspect constraint: scale axes independently (already done above)
    }

    // ── 4. Anchor point in world space ───────────────────────────────────────
    let (origin_x, origin_y) = match args.anchor {
        SizeAnchor::TopLeft => (ax, ay),
        SizeAnchor::TopCenter => (ax + cur_w / 2.0, ay),
        SizeAnchor::TopRight => (ax + cur_w, ay),
        SizeAnchor::LeftCenter => (ax, ay + cur_h / 2.0),
        SizeAnchor::Center => (ax + cur_w / 2.0, ay + cur_h / 2.0),
        SizeAnchor::RightCenter => (ax + cur_w, ay + cur_h / 2.0),
        SizeAnchor::BottomLeft => (ax, ay + cur_h),
        SizeAnchor::BottomCenter => (ax + cur_w / 2.0, ay + cur_h),
        SizeAnchor::BottomRight => (ax + cur_w, ay + cur_h),
    };

    // ── 5. Build new transform ───────────────────────────────────────────────
    // Compose: existing local→world transform, then world-space scale around anchor.
    let scale_t = Transform::scale_around(sx, sy, origin_x, origin_y);
    let new_transform = old_node.transform.then(&scale_t);

    let mut new_node = old_node.clone();
    new_node.transform = new_transform;

    let cmd = Command::UpdateNode {
        old: old_node.clone(),
        new: new_node,
    };
    {
        let mut doc = state.document.lock().await;
        let mut history = state.history.lock().await;
        history.execute(cmd, &mut doc);
    }

    let new_w = (cur_w * sx * 100.0).round() / 100.0;
    let new_h = (cur_h * sy * 100.0).round() / 100.0;

    ToolResult::text(format!(
        "Resized '{}' to {:.2}×{:.2} px (was {:.2}×{:.2} px)",
        old_node.name, new_w, new_h, cur_w, cur_h
    ))
    .with_data(serde_json::json!({
        "node_id": args.node_id,
        "previous": { "width": (cur_w * 100.0).round() / 100.0, "height": (cur_h * 100.0).round() / 100.0 },
        "new":      { "width": new_w, "height": new_h },
        "scale":    { "sx": (sx * 10000.0).round() / 10000.0, "sy": (sy * 10000.0).round() / 10000.0 },
    }))
}

/// Find nodes by fill or stroke color and replace those colors — plus
/// optionally node-level opacity — in a single undoable batch.
///
/// This is the "Find & Replace" for color. It eliminates the common
/// AI-agent pattern of: get_document_state → iterate nodes → call
/// update_node for each match (N round-trips, N undo steps).  A single
/// `find_replace_style` call handles the entire document in one step.
///
/// It is equally useful for humans doing brand refreshes: swap every
/// instance of a brand color across the whole file without touching
/// anything else.
///
/// Gradient support: matching checks solid fills *and* individual stop /
/// control-point colors inside linear, radial, fluid, and mesh gradients.
/// Only the matching colors within each gradient are replaced; unmatched
/// stops are left untouched.
///
/// `dry_run: true` returns a preview of what would change without mutating.
pub async fn find_replace_style(state: &AppState, args: FindReplaceStyleArgs) -> ToolResult {
    use photonic_core::color::Color;
    use photonic_core::style::FillKind;

    // ── 1. Parse search colors ────────────────────────────────────────────────
    let find_fill: Option<Color> = match &args.fill_color {
        Some(hex) => match Color::from_hex(hex) {
            Some(c) => Some(c),
            None => return ToolResult::error(format!("Invalid fill_color: '{}'", hex)),
        },
        None => None,
    };

    let find_stroke: Option<Color> = match &args.stroke_color {
        Some(hex) => match Color::from_hex(hex) {
            Some(c) => Some(c),
            None => return ToolResult::error(format!("Invalid stroke_color: '{}'", hex)),
        },
        None => None,
    };

    if find_fill.is_none()
        && find_stroke.is_none()
        && args.stroke_width.is_none()
        && args.font_family.is_none()
    {
        return ToolResult::error(
            "At least one search criterion must be specified: fill_color, stroke_color, stroke_width, or font_family",
        );
    }

    if args.new_fill_color.is_none()
        && args.new_stroke_color.is_none()
        && args.new_opacity.is_none()
        && args.new_stroke_width.is_none()
        && args.new_font_family.is_none()
    {
        return ToolResult::error(
            "At least one replacement must be specified: new_fill_color, new_stroke_color, new_opacity, new_stroke_width, or new_font_family",
        );
    }

    // ── 2. Parse replacement colors ───────────────────────────────────────────
    let new_fill: Option<Color> = match &args.new_fill_color {
        Some(hex) => match Color::from_hex(hex) {
            Some(c) => Some(c),
            None => return ToolResult::error(format!("Invalid new_fill_color: '{}'", hex)),
        },
        None => None,
    };

    let new_stroke: Option<Color> = match &args.new_stroke_color {
        Some(hex) => match Color::from_hex(hex) {
            Some(c) => Some(c),
            None => return ToolResult::error(format!("Invalid new_stroke_color: '{}'", hex)),
        },
        None => None,
    };

    let tolerance = args.color_tolerance.unwrap_or(0.0).clamp(0.0, 1.0);

    // Width tolerance: fractional — tolerance=0.1 means ±10% of the target value.
    // When tolerance=0.0 we use a tiny epsilon to handle f64 round-trips cleanly.
    let width_tolerance_abs = |target: f64| -> f64 {
        if tolerance == 0.0 {
            1e-9
        } else {
            target * (tolerance as f64)
        }
    };

    // ── 3. Color distance helper (normalized to [0, 1]) ───────────────────────
    // Euclidean distance in linear RGB divided by √3 (the maximum possible distance).
    let color_near = |a: Color, b: Color| -> bool {
        let dr = a.r - b.r;
        let dg = a.g - b.g;
        let db = a.b - b.b;
        let dist = ((dr * dr + dg * dg + db * db) / 3.0_f32).sqrt();
        dist <= tolerance
    };

    // ── 4. Collect candidate nodes ────────────────────────────────────────────
    let candidates: Vec<SceneNode> = {
        let doc = state.document.lock().await;
        match &args.node_ids {
            Some(ids) => ids
                .iter()
                .filter_map(|id| doc.nodes.get(id).cloned())
                .collect(),
            None => doc
                .nodes
                .values()
                .filter(|n| args.layer_id.map_or(true, |lid| n.layer_id == lid))
                .cloned()
                .collect(),
        }
    };

    // ── 5. Match and build replacements ──────────────────────────────────────
    let mut commands: Vec<Command> = Vec::new();
    let mut changed: Vec<serde_json::Value> = Vec::new();

    for node in &candidates {
        let mut new_node = node.clone();
        let mut changes: Vec<String> = Vec::new();
        let mut fill_matched = false;
        let mut stroke_matched = false;
        let mut width_matched = false;
        let mut font_matched = false;

        match &mut new_node.kind {
            SceneNodeKind::Path(path) => {
                // Fill search
                if let Some(target) = find_fill {
                    match &mut path.fill.kind {
                        FillKind::Solid(c) if color_near(*c, target) => {
                            fill_matched = true;
                            if let Some(nc) = new_fill {
                                changes.push(format!("fill: {} → {}", c.to_hex(), nc.to_hex()));
                                *c = nc;
                            }
                        }
                        FillKind::Gradient(g) => {
                            for stop in &mut g.stops {
                                if color_near(stop.color, target) {
                                    fill_matched = true;
                                    if let Some(nc) = new_fill {
                                        changes.push(format!(
                                            "gradient stop @{:.0}%: {} → {}",
                                            stop.offset * 100.0,
                                            stop.color.to_hex(),
                                            nc.to_hex()
                                        ));
                                        stop.color = nc;
                                    }
                                }
                            }
                        }
                        FillKind::FluidGradient(fg) => {
                            for pt in &mut fg.points {
                                if color_near(pt.color, target) {
                                    fill_matched = true;
                                    if let Some(nc) = new_fill {
                                        changes.push(format!(
                                            "fluid point ({:.0},{:.0}): {} → {}",
                                            pt.x,
                                            pt.y,
                                            pt.color.to_hex(),
                                            nc.to_hex()
                                        ));
                                        pt.color = nc;
                                    }
                                }
                            }
                        }
                        FillKind::MeshGradient(mg) => {
                            for v in &mut mg.vertices {
                                if color_near(v.color, target) {
                                    fill_matched = true;
                                    if let Some(nc) = new_fill {
                                        changes.push(format!(
                                            "mesh vertex ({:.0},{:.0}): {} → {}",
                                            v.x,
                                            v.y,
                                            v.color.to_hex(),
                                            nc.to_hex()
                                        ));
                                        v.color = nc;
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }

                // Stroke color search
                if let Some(target) = find_stroke {
                    if path.stroke.enabled && color_near(path.stroke.color, target) {
                        stroke_matched = true;
                        if let Some(nc) = new_stroke {
                            changes.push(format!(
                                "stroke: {} → {}",
                                path.stroke.color.to_hex(),
                                nc.to_hex()
                            ));
                            path.stroke.color = nc;
                        }
                    }
                }

                // Stroke width search
                if let Some(target_w) = args.stroke_width {
                    if path.stroke.enabled
                        && (path.stroke.width - target_w).abs() <= width_tolerance_abs(target_w)
                    {
                        width_matched = true;
                        if let Some(nw) = args.new_stroke_width {
                            changes.push(format!("stroke-width: {} → {}", path.stroke.width, nw));
                            path.stroke.width = nw;
                        }
                    }
                }
            }

            SceneNodeKind::Text(text) => {
                // Text nodes carry their own fill and stroke
                if let Some(target) = find_fill {
                    if let FillKind::Solid(c) = &mut text.fill.kind {
                        if color_near(*c, target) {
                            fill_matched = true;
                            if let Some(nc) = new_fill {
                                changes.push(format!("fill: {} → {}", c.to_hex(), nc.to_hex()));
                                *c = nc;
                            }
                        }
                    }
                }
                if let Some(target) = find_stroke {
                    if text.stroke.enabled && color_near(text.stroke.color, target) {
                        stroke_matched = true;
                        if let Some(nc) = new_stroke {
                            changes.push(format!(
                                "stroke: {} → {}",
                                text.stroke.color.to_hex(),
                                nc.to_hex()
                            ));
                            text.stroke.color = nc;
                        }
                    }
                }

                // Stroke width search on text
                if let Some(target_w) = args.stroke_width {
                    if text.stroke.enabled
                        && (text.stroke.width - target_w).abs() <= width_tolerance_abs(target_w)
                    {
                        width_matched = true;
                        if let Some(nw) = args.new_stroke_width {
                            changes.push(format!("stroke-width: {} → {}", text.stroke.width, nw));
                            text.stroke.width = nw;
                        }
                    }
                }

                // Font family search (text nodes only)
                if let Some(ref target_ff) = args.font_family {
                    if text.font_family.to_lowercase() == target_ff.to_lowercase() {
                        font_matched = true;
                        if let Some(ref nff) = args.new_font_family {
                            changes.push(format!("font-family: {} → {}", text.font_family, nff));
                            text.font_family = nff.clone();
                        }
                    }
                }
            }

            SceneNodeKind::Group(_) => {
                // Groups carry no direct fill/stroke — skip style matching.
            }
        }

        // Node-level opacity override applied to any matched node.
        let any_matched = fill_matched || stroke_matched || width_matched || font_matched;
        if any_matched {
            if let Some(new_op) = args.new_opacity {
                let new_op = new_op.clamp(0.0, 1.0);
                if (new_node.opacity - new_op).abs() > 1e-4 {
                    changes.push(format!("opacity: {:.2} → {:.2}", node.opacity, new_op));
                    new_node.opacity = new_op;
                }
            }
        }

        if !changes.is_empty() {
            changed.push(serde_json::json!({
                "node_id": node.id,
                "name": node.name,
                "changes": changes,
            }));
            if !args.dry_run {
                commands.push(Command::UpdateNode {
                    old: node.clone(),
                    new: new_node,
                });
            }
        }
    }

    // ── 6. Dry-run: report without mutating ───────────────────────────────────
    if args.dry_run {
        let msg = if changed.is_empty() {
            "dry_run: no nodes match the search criteria".to_string()
        } else {
            format!("dry_run: {} node(s) would be updated", changed.len())
        };
        return ToolResult::text(msg).with_data(serde_json::json!({ "matches": changed }));
    }

    // ── 7. Execute batch (single undo step) ───────────────────────────────────
    if commands.is_empty() {
        return ToolResult::text("No nodes matched the search criteria — nothing changed")
            .with_data(serde_json::json!({ "changed": [] }));
    }

    let count = commands.len();
    {
        let mut doc = state.document.lock().await;
        let mut history = state.history.lock().await;
        history.execute(Command::Batch(commands), &mut doc);
    }

    ToolResult::text(format!("Updated {} node(s)", count))
        .with_data(serde_json::json!({ "changed": changed }))
}

// ─── find_replace_text ───────────────────────────────────────────────────────

/// Search and replace text content across text nodes.
pub async fn find_replace_text(state: &AppState, args: FindReplaceTextArgs) -> ToolResult {
    // 1. Build the regex pattern
    let pattern = if args.regex {
        args.find.clone()
    } else {
        regex::escape(&args.find)
    };
    let pattern = if args.case_sensitive {
        pattern
    } else {
        format!("(?i){}", pattern)
    };
    let re = match regex::Regex::new(&pattern) {
        Ok(r) => r,
        Err(e) => return ToolResult::error(format!("Invalid regex: {}", e)),
    };

    // 2. Collect candidate text nodes
    let candidates: Vec<SceneNode> = {
        let doc = state.document.lock().await;
        match &args.node_ids {
            Some(ids) => ids
                .iter()
                .filter_map(|id| doc.nodes.get(id).cloned())
                .collect(),
            None => doc
                .nodes
                .values()
                .filter(|n| matches!(n.kind, SceneNodeKind::Text(_)))
                .cloned()
                .collect(),
        }
    };

    if candidates.is_empty() {
        return ToolResult::text("No text nodes found.")
            .with_data(serde_json::json!({ "changed": [] }));
    }

    // 3. Apply replacements
    let mut commands: Vec<Command> = Vec::new();
    let mut changed: Vec<serde_json::Value> = Vec::new();

    for node in &candidates {
        if let SceneNodeKind::Text(tn) = &node.kind {
            let new_content = re
                .replace_all(&tn.content, args.replace.as_str())
                .into_owned();
            if new_content != tn.content {
                changed.push(serde_json::json!({
                    "id":          node.id,
                    "name":        node.name,
                    "old_content": tn.content,
                    "new_content": new_content,
                }));
                if !args.dry_run {
                    let mut new_node = node.clone();
                    if let SceneNodeKind::Text(ref mut new_tn) = new_node.kind {
                        new_tn.content = new_content;
                    }
                    commands.push(Command::UpdateNode {
                        old: node.clone(),
                        new: new_node,
                    });
                }
            }
        }
    }

    if changed.is_empty() {
        return ToolResult::text("No text nodes matched the search pattern.")
            .with_data(serde_json::json!({ "changed": [] }));
    }

    if args.dry_run {
        return ToolResult::text(format!(
            "dry_run: {} text node(s) would be updated.",
            changed.len()
        ))
        .with_data(serde_json::json!({ "changed": changed }));
    }

    // 4. Execute as a single undo-able batch
    let count = commands.len();
    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    history.execute(Command::Batch(commands), &mut doc);
    history.schedule_mcp_checkpoint(format!("Find/replace text ({} nodes)", count));

    ToolResult::text(format!("Updated {} text node(s).", count))
        .with_data(serde_json::json!({ "changed": changed }))
}

// ─── layout_nodes ────────────────────────────────────────────────────────────

/// Rearrange a set of existing nodes according to a spatial layout algorithm.
///
/// Supports four layouts:
/// - `grid`             — left-to-right, wrapping rows
/// - `circle`           — evenly spaced around a circle
/// - `stack_horizontal` — left-to-right with a gap
/// - `stack_vertical`   — top-to-bottom with a gap
pub async fn layout_nodes(state: &AppState, args: LayoutNodesArgs) -> ToolResult {
    if args.node_ids.is_empty() {
        return ToolResult::error("node_ids must not be empty");
    }

    // ── 1. Read phase: collect nodes + their world-space AABB ────────────────
    struct NodeItem {
        node: SceneNode,
        /// world-space AABB: (x0, y0, x1, y1)
        bounds: (f64, f64, f64, f64),
    }

    let world_bounds = |node: &SceneNode| -> Option<(f64, f64, f64, f64)> {
        let local = node.local_bounds()?;
        let corners = [
            (local.x0, local.y0),
            (local.x1, local.y0),
            (local.x1, local.y1),
            (local.x0, local.y1),
        ];
        let pts: Vec<(f64, f64)> = corners
            .iter()
            .map(|(x, y)| node.transform.apply(*x, *y))
            .collect();
        let x0 = pts.iter().map(|(x, _)| *x).fold(f64::INFINITY, f64::min);
        let y0 = pts.iter().map(|(_, y)| *y).fold(f64::INFINITY, f64::min);
        let x1 = pts
            .iter()
            .map(|(x, _)| *x)
            .fold(f64::NEG_INFINITY, f64::max);
        let y1 = pts
            .iter()
            .map(|(_, y)| *y)
            .fold(f64::NEG_INFINITY, f64::max);
        Some((x0, y0, x1, y1))
    };

    let items: Vec<NodeItem> = {
        let doc = state.document.lock().await;
        let mut out = Vec::with_capacity(args.node_ids.len());
        for id in &args.node_ids {
            let Some(node) = doc.nodes.get(id).cloned() else {
                return ToolResult::error(format!("Node not found: {}", id));
            };
            let Some(bounds) = world_bounds(&node) else {
                return ToolResult::error(format!(
                    "Node '{}' has no computable bounds (groups are not supported)",
                    node.name
                ));
            };
            out.push(NodeItem { node, bounds });
        }
        out
    };

    let n = items.len();

    // Combined bounding box of the current selection.
    let sel_x0 = items
        .iter()
        .map(|i| i.bounds.0)
        .fold(f64::INFINITY, f64::min);
    let sel_y0 = items
        .iter()
        .map(|i| i.bounds.1)
        .fold(f64::INFINITY, f64::min);
    let sel_x1 = items
        .iter()
        .map(|i| i.bounds.2)
        .fold(f64::NEG_INFINITY, f64::max);
    let sel_y1 = items
        .iter()
        .map(|i| i.bounds.3)
        .fold(f64::NEG_INFINITY, f64::max);

    // ── 2. Compute target positions per layout ────────────────────────────────
    // Returns (target_x, target_y) for the top-left corner of each node's AABB.
    let targets: Vec<(f64, f64)> = match args.layout {
        // ── Grid ─────────────────────────────────────────────────────────────
        LayoutMode::Grid => {
            let cols = args
                .columns
                .unwrap_or_else(|| (n as f64).sqrt().ceil() as usize)
                .max(1);
            let gap_x = args.gap_x.unwrap_or(20.0);
            let gap_y = args.gap_y.unwrap_or(20.0);

            // Default cell size = widest / tallest node; overridable per axis.
            let cell_w = args.cell_width.unwrap_or_else(|| {
                items
                    .iter()
                    .map(|i| i.bounds.2 - i.bounds.0)
                    .fold(0.0_f64, f64::max)
            });
            let cell_h = args.cell_height.unwrap_or_else(|| {
                items
                    .iter()
                    .map(|i| i.bounds.3 - i.bounds.1)
                    .fold(0.0_f64, f64::max)
            });

            let origin_x = args.x.unwrap_or(sel_x0);
            let origin_y = args.y.unwrap_or(sel_y0);

            items
                .iter()
                .enumerate()
                .map(|(idx, item)| {
                    let col = idx % cols;
                    let row = idx / cols;
                    let cell_x = origin_x + col as f64 * (cell_w + gap_x);
                    let cell_y = origin_y + row as f64 * (cell_h + gap_y);
                    // Centre the node inside its cell.
                    let node_w = item.bounds.2 - item.bounds.0;
                    let node_h = item.bounds.3 - item.bounds.1;
                    (
                        cell_x + (cell_w - node_w) / 2.0,
                        cell_y + (cell_h - node_h) / 2.0,
                    )
                })
                .collect()
        }

        // ── Circle ────────────────────────────────────────────────────────────
        LayoutMode::Circle => {
            let centre_x = args.cx.unwrap_or((sel_x0 + sel_x1) / 2.0);
            let centre_y = args.cy.unwrap_or((sel_y0 + sel_y1) / 2.0);
            let radius = args.radius.unwrap_or(200.0);
            let start_deg = args.start_angle.unwrap_or(0.0);
            let angle_step = 360.0 / n as f64;

            items
                .iter()
                .enumerate()
                .map(|(idx, item)| {
                    let angle = (start_deg + idx as f64 * angle_step).to_radians();
                    let node_cx = centre_x + radius * angle.cos();
                    let node_cy = centre_y + radius * angle.sin();
                    let node_w = item.bounds.2 - item.bounds.0;
                    let node_h = item.bounds.3 - item.bounds.1;
                    (node_cx - node_w / 2.0, node_cy - node_h / 2.0)
                })
                .collect()
        }

        // ── Stack horizontal ──────────────────────────────────────────────────
        LayoutMode::StackHorizontal => {
            let gap = args.gap.unwrap_or(20.0);
            let origin_x = args.x.unwrap_or(sel_x0);
            let origin_y = args.y.unwrap_or(sel_y0);

            // Cross-axis (Y) reference.
            let tallest = items
                .iter()
                .map(|i| i.bounds.3 - i.bounds.1)
                .fold(0.0_f64, f64::max);
            let cross_ref_y = match args.align {
                CrossAxisAlign::Start => origin_y,
                CrossAxisAlign::Center => origin_y + tallest / 2.0,
                CrossAxisAlign::End => origin_y + tallest,
            };

            let mut cursor = origin_x;
            items
                .iter()
                .map(|item| {
                    let w = item.bounds.2 - item.bounds.0;
                    let h = item.bounds.3 - item.bounds.1;
                    let tx = cursor;
                    let ty = match args.align {
                        CrossAxisAlign::Start => cross_ref_y,
                        CrossAxisAlign::Center => cross_ref_y - h / 2.0,
                        CrossAxisAlign::End => cross_ref_y - h,
                    };
                    cursor += w + gap;
                    (tx, ty)
                })
                .collect()
        }

        // ── Stack vertical ────────────────────────────────────────────────────
        LayoutMode::StackVertical => {
            let gap = args.gap.unwrap_or(20.0);
            let origin_x = args.x.unwrap_or(sel_x0);
            let origin_y = args.y.unwrap_or(sel_y0);

            // Cross-axis (X) reference.
            let widest = items
                .iter()
                .map(|i| i.bounds.2 - i.bounds.0)
                .fold(0.0_f64, f64::max);
            let cross_ref_x = match args.align {
                CrossAxisAlign::Start => origin_x,
                CrossAxisAlign::Center => origin_x + widest / 2.0,
                CrossAxisAlign::End => origin_x + widest,
            };

            let mut cursor = origin_y;
            items
                .iter()
                .map(|item| {
                    let w = item.bounds.2 - item.bounds.0;
                    let h = item.bounds.3 - item.bounds.1;
                    let ty = cursor;
                    let tx = match args.align {
                        CrossAxisAlign::Start => cross_ref_x,
                        CrossAxisAlign::Center => cross_ref_x - w / 2.0,
                        CrossAxisAlign::End => cross_ref_x - w,
                    };
                    cursor += h + gap;
                    (tx, ty)
                })
                .collect()
        }
    };

    // ── 3. Build UpdateNode commands ──────────────────────────────────────────
    let commands: Vec<Command> = items
        .iter()
        .zip(targets.iter())
        .filter_map(|(item, (tx, ty))| {
            let dx = tx - item.bounds.0;
            let dy = ty - item.bounds.1;
            if dx.abs() < 1e-9 && dy.abs() < 1e-9 {
                return None; // already in position
            }
            let old = item.node.clone();
            let mut new = old.clone();
            new.transform = new.transform.then(&Transform::translate(dx, dy));
            Some(Command::UpdateNode { old, new })
        })
        .collect();

    if commands.is_empty() {
        return ToolResult::text("All nodes are already in the target positions — nothing changed");
    }

    let moved = commands.len();
    {
        let mut doc = state.document.lock().await;
        let mut history = state.history.lock().await;
        history.execute(Command::Batch(commands), &mut doc);
    }

    ToolResult::text(format!(
        "layout_nodes: moved {} of {} node(s) using {:?} layout",
        moved, n, args.layout
    ))
    .with_data(serde_json::json!({ "moved": moved, "total": n }))
}

/// Return computed geometry and structure data for a single node.
pub async fn inspect_node(state: &AppState, args: InspectNodeArgs) -> ToolResult {
    use kurbo::Shape;

    // Resolve node and clone the full node map under a brief lock.
    let (node, node_map) = {
        let doc = state.document.lock().await;
        let found = if let Ok(uuid) = uuid::Uuid::parse_str(&args.id) {
            doc.get_node(&uuid).cloned()
        } else {
            doc.find_node_by_name(&args.id).cloned()
        };
        let Some(node) = found else {
            return ToolResult::error(format!("Node not found: {}", args.id));
        };
        let node_map = doc.nodes.clone();
        (node, node_map)
    };

    // ── shared helpers ────────────────────────────────────────────────────────

    fn world_aabb_of(node: &SceneNode) -> Option<[f64; 4]> {
        let local = node.local_bounds()?;
        let affine = node.transform.to_kurbo();
        let pts = [
            affine * kurbo::Point::new(local.x0, local.y0),
            affine * kurbo::Point::new(local.x1, local.y0),
            affine * kurbo::Point::new(local.x1, local.y1),
            affine * kurbo::Point::new(local.x0, local.y1),
        ];
        let x0 = pts.iter().map(|p| p.x).fold(f64::INFINITY, f64::min);
        let y0 = pts.iter().map(|p| p.y).fold(f64::INFINITY, f64::min);
        let x1 = pts.iter().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max);
        let y1 = pts.iter().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max);
        Some([x0, y0, x1 - x0, y1 - y0])
    }

    fn union_aabb(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
        let x0 = a[0].min(b[0]);
        let y0 = a[1].min(b[1]);
        let x1 = (a[0] + a[2]).max(b[0] + b[2]);
        let y1 = (a[1] + a[3]).max(b[1] + b[3]);
        [x0, y0, x1 - x0, y1 - y0]
    }

    fn r2(v: f64) -> f64 {
        (v * 100.0).round() / 100.0
    }

    fn aabb_to_json(aabb: [f64; 4]) -> serde_json::Value {
        serde_json::json!({ "x": aabb[0], "y": aabb[1], "width": aabb[2], "height": aabb[3] })
    }

    let id_str = node.id.to_string();
    let name = node.name.clone();

    // ── per-kind computation ──────────────────────────────────────────────────

    match &node.kind {
        SceneNodeKind::Path(path_node) => {
            let bez = path_node.path_data.to_bez_path();

            let anchor_count = bez
                .elements()
                .iter()
                .filter(|e| !matches!(e, kurbo::PathEl::ClosePath))
                .count();

            let area = r2(bez.area().abs());
            let perimeter = r2(bez.perimeter(1e-3));

            let (centroid_x, centroid_y) = if let Some(local) = node.local_bounds() {
                let cx = (local.x0 + local.x1) / 2.0;
                let cy = (local.y0 + local.y1) / 2.0;
                let p = node.transform.to_kurbo() * kurbo::Point::new(cx, cy);
                (r2(p.x), r2(p.y))
            } else {
                (0.0, 0.0)
            };

            let world_bounds = world_aabb_of(&node).map(aabb_to_json);
            let local_bounds = node.local_bounds().map(|r| {
                serde_json::json!({
                    "x": r2(r.x0), "y": r2(r.y0),
                    "width": r2(r.x1 - r.x0), "height": r2(r.y1 - r.y0)
                })
            });

            let data = serde_json::json!({
                "id": id_str,
                "name": name,
                "type": "path",
                "world_bounds": world_bounds,
                "local_bounds": local_bounds,
                "perimeter": perimeter,
                "area": area,
                "centroid": { "x": centroid_x, "y": centroid_y },
                "anchor_count": anchor_count,
                "is_compound": path_node.is_compound,
            });

            ToolResult::text(format!(
                "inspect_node '{}': path with {} anchor(s), area={}, perimeter={}, compound={}",
                name, anchor_count, area, perimeter, path_node.is_compound
            ))
            .with_data(data)
        }

        SceneNodeKind::Group(group_node) => {
            let child_count = group_node.children.len();

            // DFS to collect all descendant node IDs.
            let mut stack: Vec<NodeId> = group_node.children.clone();
            let mut descendants: Vec<NodeId> = Vec::new();
            while let Some(id) = stack.pop() {
                descendants.push(id);
                if let Some(n) = node_map.get(&id) {
                    if let SceneNodeKind::Group(g) = &n.kind {
                        stack.extend(g.children.iter().copied());
                    }
                }
            }
            let descendant_count = descendants.len();

            // Collect stats from all descendants.
            let mut total_anchor_count: usize = 0;
            let mut fill_colors: std::collections::HashSet<String> =
                std::collections::HashSet::new();
            let mut stroke_colors: std::collections::HashSet<String> =
                std::collections::HashSet::new();
            let mut world_bounds: Option<[f64; 4]> = None;

            for id in &descendants {
                let Some(n) = node_map.get(id) else { continue };
                match &n.kind {
                    SceneNodeKind::Path(p) => {
                        let bez = p.path_data.to_bez_path();
                        total_anchor_count += bez
                            .elements()
                            .iter()
                            .filter(|e| !matches!(e, kurbo::PathEl::ClosePath))
                            .count();
                        if p.fill.enabled {
                            if let photonic_core::style::FillKind::Solid(color) = &p.fill.kind {
                                fill_colors.insert(color.to_hex());
                            }
                        }
                        if p.stroke.enabled {
                            stroke_colors.insert(p.stroke.color.to_hex());
                        }
                        if let Some(aabb) = world_aabb_of(n) {
                            world_bounds = Some(match world_bounds {
                                None => aabb,
                                Some(r) => union_aabb(r, aabb),
                            });
                        }
                    }
                    SceneNodeKind::Text(t) => {
                        if t.fill.enabled {
                            if let photonic_core::style::FillKind::Solid(color) = &t.fill.kind {
                                fill_colors.insert(color.to_hex());
                            }
                        }
                        if t.stroke.enabled {
                            stroke_colors.insert(t.stroke.color.to_hex());
                        }
                    }
                    SceneNodeKind::Group(_) => {} // handled by DFS stack
                }
            }

            let mut fill_list: Vec<String> = fill_colors.into_iter().collect();
            fill_list.sort();
            let mut stroke_list: Vec<String> = stroke_colors.into_iter().collect();
            stroke_list.sort();

            let data = serde_json::json!({
                "id": id_str,
                "name": name,
                "type": "group",
                "world_bounds": world_bounds.map(aabb_to_json),
                "child_count": child_count,
                "descendant_count": descendant_count,
                "total_anchor_count": total_anchor_count,
                "unique_fill_colors": fill_list,
                "unique_stroke_colors": stroke_list,
            });

            ToolResult::text(format!(
                "inspect_node '{}': group, {} child(ren), {} descendant(s), {} total anchor(s)",
                name, child_count, descendant_count, total_anchor_count
            ))
            .with_data(data)
        }

        SceneNodeKind::Text(text_node) => {
            let line_count = text_node.content.lines().count().max(1);
            let char_count = text_node.content.chars().count();
            let world_bounds = world_aabb_of(&node).map(aabb_to_json);

            let data = serde_json::json!({
                "id": id_str,
                "name": name,
                "type": "text",
                "world_bounds": world_bounds,
                "line_count": line_count,
                "char_count": char_count,
                "font_family": text_node.font_family,
                "font_size": text_node.font_size,
                "font_weight": text_node.font_weight,
            });

            ToolResult::text(format!(
                "inspect_node '{}': text, {} char(s), {} line(s), font '{}'",
                name, char_count, line_count, text_node.font_family
            ))
            .with_data(data)
        }
    }
}

// ─── auto_name_nodes ──────────────────────────────────────────────────────────

/// Returns true if `name` looks like an auto-generated default (should be renamed).
fn is_generic_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    let generic_prefixes = [
        "path",
        "ellipse",
        "rectangle",
        "rect",
        "polygon",
        "star",
        "line",
        "group",
        "text",
        "shape",
        "node",
        "layer",
    ];
    if generic_prefixes.iter().any(|p| lower.starts_with(p)) {
        return true;
    }
    uuid::Uuid::parse_str(name).is_ok()
}

/// Map an RGB colour (0..1 linear sRGB) to a short English label.
fn color_label(r: f32, g: f32, b: f32) -> &'static str {
    if r > 0.85 && g > 0.85 && b > 0.85 {
        return "white";
    }
    if r < 0.15 && g < 0.15 && b < 0.15 {
        return "black";
    }
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let chroma = max - min;
    if chroma < 0.12 {
        return if max > 0.6 { "light gray" } else { "gray" };
    }
    if r > 0.5 && g > 0.35 && b < 0.25 {
        return "orange";
    }
    if r > 0.6 && g > 0.6 && b < 0.3 {
        return "yellow";
    }
    if r > 0.6 && b > 0.6 && g < 0.3 {
        return "magenta";
    }
    if g > 0.5 && b > 0.5 && r < 0.3 {
        return "cyan";
    }
    if r >= g && r >= b && r > 0.5 && g < 0.5 {
        return "red";
    }
    if g >= r && g >= b && g > 0.5 && r < 0.5 {
        return "green";
    }
    if b >= r && b >= g && b > 0.4 {
        return "blue";
    }
    if max < 0.4 {
        return "dark";
    }
    "colored"
}

/// Generate a descriptive name for a node based on its type and properties.
fn generate_name(node: &SceneNode) -> String {
    use photonic_core::style::FillKind;

    match &node.kind {
        SceneNodeKind::Text(t) => {
            let preview: String = t.content.chars().take(24).collect();
            let preview = preview.trim().to_string();
            if preview.is_empty() {
                "empty text".to_string()
            } else {
                format!("text: {}", preview)
            }
        }
        SceneNodeKind::Group(g) => {
            format!("group ({} items)", g.children.len())
        }
        SceneNodeKind::Path(p) => {
            // ── color part ────────────────────────────────────────────────────
            let color_part: String = if !p.fill.enabled {
                if p.stroke.enabled {
                    "outline".to_string()
                } else {
                    "empty".to_string()
                }
            } else {
                match &p.fill.kind {
                    FillKind::Solid(c) => color_label(c.r, c.g, c.b).to_string(),
                    FillKind::Gradient(_)
                    | FillKind::FluidGradient(_)
                    | FillKind::MeshGradient(_) => "gradient".to_string(),
                    FillKind::None => "outline".to_string(),
                }
            };
            // ── geometry part ─────────────────────────────────────────────────
            let geo_part: String = match p.path_data.bounding_box() {
                None => "shape".to_string(),
                Some(bb) => {
                    let w = (bb.x1 - bb.x0).abs();
                    let h = (bb.y1 - bb.y0).abs();
                    let area = w * h;
                    let size = if area < 2500.0 {
                        "small"
                    } else if area < 22500.0 {
                        "medium"
                    } else {
                        "large"
                    };
                    let ratio = if h > 0.0 { w / h } else { 1.0 };
                    let shape = if ratio > 2.5 {
                        "wide bar"
                    } else if ratio < 0.4 {
                        "tall bar"
                    } else if (0.85..=1.18).contains(&ratio) {
                        "square"
                    } else {
                        "shape"
                    };
                    format!("{} {}", size, shape)
                }
            };
            format!("{} {}", color_part, geo_part)
        }
    }
}

pub async fn auto_name_nodes(state: &AppState, args: AutoNameNodesArgs) -> ToolResult {
    tracing::debug!("tool: auto_name_nodes");

    // ── Phase 1: collect target node IDs and clone nodes ─────────────────────
    let (_target_ids, nodes_snapshot) = {
        let doc = state.document.lock().await;
        let scope = args.scope.as_deref().unwrap_or("document");
        let ids: Vec<NodeId> = if scope == "selection" {
            doc.selection.ids().copied().collect()
        } else {
            doc.nodes.keys().copied().collect()
        };
        let snapshot: Vec<SceneNode> = ids
            .iter()
            .filter_map(|id| doc.nodes.get(id).cloned())
            .collect();
        (ids, snapshot)
    }; // lock released

    if nodes_snapshot.is_empty() {
        return ToolResult::text("No nodes to rename");
    }

    // ── Phase 2: compute renames ──────────────────────────────────────────────
    let renames: Vec<(SceneNode, String)> = nodes_snapshot
        .into_iter()
        .filter(|n| args.overwrite || is_generic_name(&n.name))
        .map(|n| {
            let new_name = generate_name(&n);
            (n, new_name)
        })
        .collect();

    if renames.is_empty() {
        return ToolResult::text(
            "No nodes with generic names found. Pass overwrite:true to rename all nodes.",
        );
    }

    let rename_list: Vec<serde_json::Value> = renames
        .iter()
        .map(|(n, new_name)| {
            serde_json::json!({
                "id": n.id.to_string(),
                "old_name": n.name,
                "new_name": new_name,
            })
        })
        .collect();

    if args.dry_run {
        return ToolResult::text(format!("dry_run: would rename {} node(s)", renames.len()))
            .with_data(serde_json::json!({
                "renamed": renames.len(),
                "dry_run": true,
                "renames": rename_list,
            }));
    }

    // ── Phase 3: apply renames ────────────────────────────────────────────────
    let commands: Vec<Command> = renames
        .into_iter()
        .map(|(old_node, new_name)| {
            let mut new_node = old_node.clone();
            new_node.name = new_name;
            Command::UpdateNode {
                old: old_node,
                new: new_node,
            }
        })
        .collect();

    let count = commands.len();
    let batch = Command::Batch(commands);
    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    history.execute(batch, &mut doc);

    ToolResult::text(format!("Renamed {} node(s)", count)).with_data(serde_json::json!({
        "renamed": count,
        "dry_run": false,
        "renames": rename_list,
    }))
}

// ─── CSS Preview ──────────────────────────────────────────────────────────────

/// Return a CSS representation of a node's visual properties for developer
/// handoff. Read-only — does not modify the document.
pub async fn get_css_preview(state: &AppState, args: GetCssPreviewArgs) -> ToolResult {
    use photonic_core::{
        style::{Fill, FillKind, GradientKind, Stroke},
        transform::Transform,
    };

    // ── Helpers ───────────────────────────────────────────────────────────────

    /// Format a color as `rgba(r, g, b, a)` or `#rrggbb` when fully opaque.
    fn color_css(r: f32, g: f32, b: f32, a: f32) -> String {
        if (a - 1.0).abs() < 0.004 {
            format!(
                "#{:02x}{:02x}{:02x}",
                (r * 255.0).round() as u8,
                (g * 255.0).round() as u8,
                (b * 255.0).round() as u8,
            )
        } else {
            format!(
                "rgba({}, {}, {}, {:.3})",
                (r * 255.0).round() as u8,
                (g * 255.0).round() as u8,
                (b * 255.0).round() as u8,
                a,
            )
        }
    }

    /// Convert a `Fill` to one or two CSS lines and an optional note.
    fn fill_to_css(fill: &Fill, lines: &mut Vec<String>, notes: &mut Vec<String>) {
        if !fill.enabled {
            return;
        }
        let opacity = fill.opacity;
        match &fill.kind {
            FillKind::None => {}
            FillKind::Solid(c) => {
                let a = c.a * opacity;
                lines.push(format!(
                    "background-color: {};",
                    color_css(c.r, c.g, c.b, a)
                ));
            }
            FillKind::Gradient(g) => {
                if g.stops.is_empty() {
                    return;
                }
                let stops: Vec<String> = g
                    .stops
                    .iter()
                    .map(|s| {
                        let a = s.color.a * opacity;
                        format!(
                            "{} {:.1}%",
                            color_css(s.color.r, s.color.g, s.color.b, a),
                            s.offset * 100.0
                        )
                    })
                    .collect();
                let stops_str = stops.join(", ");
                match g.kind {
                    GradientKind::Linear => {
                        let (dx, dy) = if g.coords.len() >= 4 {
                            (g.coords[2] - g.coords[0], g.coords[3] - g.coords[1])
                        } else {
                            (1.0, 0.0)
                        };
                        // CSS gradient angle: 0deg = upward, increases clockwise.
                        // atan2(dx, -dy) converts vector direction to CSS convention.
                        let angle = dy.atan2(dx).to_degrees() + 90.0;
                        lines.push(format!(
                            "background: linear-gradient({:.1}deg, {});",
                            angle, stops_str
                        ));
                    }
                    GradientKind::Radial => {
                        let (cx, cy) = if g.coords.len() >= 2 {
                            (g.coords[0], g.coords[1])
                        } else {
                            (0.0, 0.0)
                        };
                        lines.push(format!(
                            "background: radial-gradient(circle at {:.1}px {:.1}px, {});",
                            cx, cy, stops_str
                        ));
                    }
                }
            }
            FillKind::FluidGradient(fg) => {
                if let Some(first) = fg.points.first() {
                    let c = &first.color;
                    let a = c.a * opacity;
                    lines.push(format!(
                        "background-color: {}; /* approximated from fluid gradient */",
                        color_css(c.r, c.g, c.b, a)
                    ));
                    notes.push(
                        "Fluid gradient has no direct CSS equivalent — shown as approximated solid from the first control point."
                            .to_string(),
                    );
                }
            }
            FillKind::MeshGradient(mg) => {
                if let Some(first) = mg.vertices.first() {
                    let c = &first.color;
                    let a = c.a * opacity;
                    lines.push(format!(
                        "background-color: {}; /* approximated from mesh gradient */",
                        color_css(c.r, c.g, c.b, a)
                    ));
                    notes.push(
                        "Mesh gradient has no direct CSS equivalent — shown as approximated solid from the first vertex."
                            .to_string(),
                    );
                }
            }
        }
    }

    /// Convert a `Stroke` to a CSS `outline` line (preserves layout dimensions).
    fn stroke_to_css(stroke: &Stroke) -> Option<String> {
        if !stroke.enabled || stroke.width <= 0.0 {
            return None;
        }
        let a = stroke.color.a * stroke.opacity;
        let color = color_css(stroke.color.r, stroke.color.g, stroke.color.b, a);
        // Use outline so the stroke does not affect the element's box dimensions.
        Some(format!("outline: {:.2}px solid {};", stroke.width, color))
    }

    /// Convert a `Transform` to a CSS `transform` line, or `None` if identity.
    fn transform_to_css(t: &Transform) -> Option<String> {
        if t.is_identity() {
            return None;
        }
        let m = t.matrix;
        // CSS matrix(a, b, c, d, e, f) matches SVG / affine conventions.
        Some(format!(
            "transform: matrix({:.6}, {:.6}, {:.6}, {:.6}, {:.6}, {:.6});",
            m[0], m[1], m[2], m[3], m[4], m[5]
        ))
    }

    /// Compute the world-space AABB [x, y, w, h] of a node.
    fn world_aabb(node: &SceneNode) -> Option<[f64; 4]> {
        let local = node.local_bounds()?;
        let affine = node.transform.to_kurbo();
        let pts = [
            affine * kurbo::Point::new(local.x0, local.y0),
            affine * kurbo::Point::new(local.x1, local.y0),
            affine * kurbo::Point::new(local.x1, local.y1),
            affine * kurbo::Point::new(local.x0, local.y1),
        ];
        let x0 = pts.iter().map(|p| p.x).fold(f64::INFINITY, f64::min);
        let y0 = pts.iter().map(|p| p.y).fold(f64::INFINITY, f64::min);
        let x1 = pts.iter().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max);
        let y1 = pts.iter().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max);
        Some([x0, y0, x1 - x0, y1 - y0])
    }

    // ── Resolve node ──────────────────────────────────────────────────────────

    let node = {
        let doc = state.document.lock().await;
        if let Some(id_str) = &args.id {
            if let Ok(uuid) = uuid::Uuid::parse_str(id_str) {
                doc.get_node(&uuid).cloned()
            } else {
                doc.find_node_by_name(id_str).cloned()
            }
        } else {
            doc.nodes.values().next().cloned()
        }
    };

    let Some(node) = node else {
        let desc = args.id.as_deref().unwrap_or("<first node>");
        return ToolResult::error(format!("Node not found: {}", desc));
    };

    // ── Build CSS lines ───────────────────────────────────────────────────────

    let mut lines: Vec<String> = Vec::new();
    let mut notes: Vec<String> = Vec::new();

    // Size from world bounding box (ignoring rotation for width/height).
    if let Some([_x, _y, w, h]) = world_aabb(&node) {
        lines.push(format!("width: {:.2}px;", w));
        lines.push(format!("height: {:.2}px;", h));
    }

    // Node-kind–specific properties.
    match &node.kind {
        SceneNodeKind::Path(p) => {
            fill_to_css(&p.fill, &mut lines, &mut notes);
            if let Some(s) = stroke_to_css(&p.stroke) {
                lines.push(s);
            }
        }
        SceneNodeKind::Text(t) => {
            // Text colour from fill.
            if t.fill.enabled {
                match &t.fill.kind {
                    FillKind::Solid(c) => {
                        let a = c.a * t.fill.opacity;
                        lines.push(format!("color: {};", color_css(c.r, c.g, c.b, a)));
                    }
                    _ => {
                        fill_to_css(&t.fill, &mut lines, &mut notes);
                    }
                }
            }
            if let Some(s) = stroke_to_css(&t.stroke) {
                lines.push(s);
            }
            lines.push(format!("font-family: \"{}\";", t.font_family));
            lines.push(format!("font-size: {}px;", t.font_size));
            lines.push(format!("font-weight: {};", t.font_weight));
            let align_str = match t.align {
                photonic_core::node::TextAlign::Left => "left",
                photonic_core::node::TextAlign::Center => "center",
                photonic_core::node::TextAlign::Right => "right",
            };
            lines.push(format!("text-align: {};", align_str));
        }
        SceneNodeKind::Group(_) => {
            notes.push(
                "Group nodes have no fill or stroke — CSS shown covers size and positioning only."
                    .to_string(),
            );
        }
    }

    // Opacity (node-level).
    if (node.opacity - 1.0).abs() > 1e-4 {
        lines.push(format!("opacity: {:.3};", node.opacity));
    }

    // Blend mode.
    if node.blend_mode != BlendMode::Normal {
        let bm = format!("{:?}", node.blend_mode);
        // Convert PascalCase to kebab-case (e.g. ColorDodge → color-dodge).
        let kebab = bm
            .chars()
            .enumerate()
            .flat_map(|(i, c)| {
                if c.is_uppercase() && i > 0 {
                    vec!['-', c.to_lowercase().next().unwrap()]
                } else {
                    vec![c.to_lowercase().next().unwrap()]
                }
            })
            .collect::<String>();
        lines.push(format!("mix-blend-mode: {};", kebab));
    }

    // Transform (only if non-identity).
    if let Some(t) = transform_to_css(&node.transform) {
        lines.push(t);
    }

    // ── Assemble CSS block ────────────────────────────────────────────────────

    let node_type = match &node.kind {
        SceneNodeKind::Path(_) => "path",
        SceneNodeKind::Text(_) => "text",
        SceneNodeKind::Group(_) => "group",
    };

    let css_block = if lines.is_empty() {
        format!("/* Photonic node: \"{}\" — no CSS properties */", node.name)
    } else {
        format!(
            "/* Photonic node: \"{}\" */\n{}",
            node.name,
            lines.join("\n")
        )
    };

    ToolResult::text(format!("CSS preview for '{}'", node.name)).with_data(serde_json::json!({
        "node_id":   node.id.to_string(),
        "node_name": node.name,
        "node_type": node_type,
        "css":       css_block,
        "notes":     notes,
    }))
}

// ─── check_style_continuity ───────────────────────────────────────────────────

/// Analyse style consistency across the document or a node subset.
/// Returns a structured report identifying dominant values and outliers per
/// checked property (fill color, stroke width, opacity, font family).
/// Read-only — makes no changes to the document.
pub async fn check_style_continuity(
    state: &AppState,
    args: CheckStyleContinuityArgs,
) -> ToolResult {
    use photonic_core::style::FillKind;
    use std::collections::HashMap;

    let doc = state.document.lock().await;

    // ── Build the node list ───────────────────────────────────────────────────
    let nodes: Vec<&photonic_core::node::SceneNode> = if args.node_ids.is_empty() {
        doc.nodes.values().collect()
    } else {
        args.node_ids
            .iter()
            .filter_map(|id| doc.nodes.get(id))
            .collect()
    };

    // Determine which property groups to check (default: all four).
    let all_checks = args.checks.is_empty();
    let check_fill = all_checks || args.checks.iter().any(|c| c == "fill");
    let check_stroke = all_checks || args.checks.iter().any(|c| c == "stroke");
    let check_opacity = all_checks || args.checks.iter().any(|c| c == "opacity");
    let check_font = all_checks || args.checks.iter().any(|c| c == "font");

    let threshold = args.outlier_threshold.unwrap_or(2);

    // ── Property buckets: value → Vec<(node_id_str, node_name)> ──────────────
    // Each bucket accumulates (string_value, node_id, node_name) entries.
    let mut fill_bucket: Vec<(String, String, String)> = Vec::new();
    let mut stroke_bucket: Vec<(String, String, String)> = Vec::new();
    let mut opacity_bucket: Vec<(String, String, String)> = Vec::new();
    let mut font_bucket: Vec<(String, String, String)> = Vec::new();

    for node in &nodes {
        let nid = node.id.to_string();
        let nname = node.name.clone();

        match &node.kind {
            SceneNodeKind::Path(p) => {
                if check_fill {
                    if p.fill.enabled {
                        if let FillKind::Solid(c) = &p.fill.kind {
                            fill_bucket.push((c.to_hex(), nid.clone(), nname.clone()));
                        }
                    }
                }
                if check_stroke && p.stroke.enabled {
                    let w = format!("{:.2}", p.stroke.width);
                    stroke_bucket.push((w, nid.clone(), nname.clone()));
                }
                if check_opacity {
                    let op = format!("{:.2}", node.opacity);
                    opacity_bucket.push((op, nid.clone(), nname.clone()));
                }
            }
            SceneNodeKind::Text(t) => {
                if check_fill {
                    if t.fill.enabled {
                        if let FillKind::Solid(c) = &t.fill.kind {
                            fill_bucket.push((c.to_hex(), nid.clone(), nname.clone()));
                        }
                    }
                }
                if check_stroke && t.stroke.enabled {
                    let w = format!("{:.2}", t.stroke.width);
                    stroke_bucket.push((w, nid.clone(), nname.clone()));
                }
                if check_opacity {
                    let op = format!("{:.2}", node.opacity);
                    opacity_bucket.push((op, nid.clone(), nname.clone()));
                }
                if check_font {
                    font_bucket.push((t.font_family.clone(), nid.clone(), nname.clone()));
                }
            }
            SceneNodeKind::Group(_) => {
                // Groups are included only for opacity analysis, not fill/stroke/font.
                if check_opacity {
                    let op = format!("{:.2}", node.opacity);
                    opacity_bucket.push((op, nid.clone(), nname.clone()));
                }
            }
        }
    }

    // ── Analyse a bucket: return (dominant_values, outliers) ─────────────────
    // outliers: Vec<(value, node_id, node_name)>
    fn analyse_bucket(
        bucket: &[(String, String, String)],
        threshold: usize,
    ) -> (Vec<String>, Vec<(String, String, String)>) {
        if bucket.is_empty() {
            return (vec![], vec![]);
        }
        // Count frequency per value.
        let mut freq: HashMap<&str, usize> = HashMap::new();
        for (val, _, _) in bucket {
            *freq.entry(val.as_str()).or_insert(0) += 1;
        }
        let dominant: Vec<String> = freq
            .iter()
            .filter(|(_, &count)| count >= threshold)
            .map(|(v, _)| v.to_string())
            .collect();

        // Only flag outliers when at least one dominant value exists.
        if dominant.is_empty() {
            return (vec![], vec![]);
        }
        let outliers: Vec<(String, String, String)> = bucket
            .iter()
            .filter(|(val, _, _)| freq[val.as_str()] < threshold)
            .map(|(v, id, name)| (v.clone(), id.clone(), name.clone()))
            .collect();
        (dominant, outliers)
    }

    // ── Run analysis ─────────────────────────────────────────────────────────
    let (fill_dominant, fill_outliers) = analyse_bucket(&fill_bucket, threshold);
    let (stroke_dominant, stroke_outliers) = analyse_bucket(&stroke_bucket, threshold);
    let (opacity_dominant, opacity_outliers) = analyse_bucket(&opacity_bucket, threshold);
    let (font_dominant, font_outliers) = analyse_bucket(&font_bucket, threshold);

    // ── Build consistent summary ──────────────────────────────────────────────
    let mut consistent = serde_json::Map::new();
    let count_dominant = |bucket: &[(String, String, String)], dominant: &[String]| {
        bucket
            .iter()
            .filter(|(v, _, _)| dominant.contains(v))
            .count()
    };
    if !fill_dominant.is_empty() {
        consistent.insert(
            "fill_color".to_string(),
            serde_json::json!({
                "dominant_values": fill_dominant,
                "node_count": count_dominant(&fill_bucket, &fill_dominant),
            }),
        );
    }
    if !stroke_dominant.is_empty() {
        consistent.insert(
            "stroke_width".to_string(),
            serde_json::json!({
                "dominant_values": stroke_dominant,
                "node_count": count_dominant(&stroke_bucket, &stroke_dominant),
            }),
        );
    }
    if !opacity_dominant.is_empty() {
        consistent.insert(
            "opacity".to_string(),
            serde_json::json!({
                "dominant_values": opacity_dominant,
                "node_count": count_dominant(&opacity_bucket, &opacity_dominant),
            }),
        );
    }
    if !font_dominant.is_empty() {
        consistent.insert(
            "font_family".to_string(),
            serde_json::json!({
                "dominant_values": font_dominant,
                "node_count": count_dominant(&font_bucket, &font_dominant),
            }),
        );
    }

    // ── Build outlier list ────────────────────────────────────────────────────
    let mut outlier_items: Vec<serde_json::Value> = Vec::new();

    let mut push_outliers = |property: &str,
                             outliers: &[(String, String, String)],
                             dominant: &[String],
                             total: usize| {
        for (val, nid, nname) in outliers {
            let dominant_str = dominant.first().map(String::as_str).unwrap_or("?");
            let message = match property {
                "fill_color" => format!(
                    "Fill color {} is used by 1 node; {} other(s) use dominant values",
                    val,
                    total - 1
                ),
                "stroke_width" => format!(
                    "Stroke width {} px; {} other node(s) use {}",
                    val,
                    total - 1,
                    dominant_str
                ),
                "opacity" => format!(
                    "Opacity {}; {} other node(s) use {}",
                    val,
                    total - 1,
                    dominant_str
                ),
                "font_family" => format!(
                    "Font \"{}\" differs from dominant \"{}\" (used by {} node(s))",
                    val,
                    dominant_str,
                    total - 1
                ),
                _ => format!("{} value {} is an outlier", property, val),
            };
            outlier_items.push(serde_json::json!({
                "property":      property,
                "node_id":       nid,
                "node_name":     nname,
                "value":         val,
                "dominant_value": dominant_str,
                "message":       message,
            }));
        }
    };

    let fill_total = fill_bucket.len();
    let stroke_total = stroke_bucket.len();
    let opacity_total = opacity_bucket.len();
    let font_total = font_bucket.len();

    // Retrieve dominant slices before moving into closure (borrow checker).
    let fill_dom_snap: Vec<String> = consistent
        .get("fill_color")
        .and_then(|v| v["dominant_values"].as_array())
        .map(|a| {
            a.iter()
                .filter_map(|s| s.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let stroke_dom_snap: Vec<String> = consistent
        .get("stroke_width")
        .and_then(|v| v["dominant_values"].as_array())
        .map(|a| {
            a.iter()
                .filter_map(|s| s.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let opacity_dom_snap: Vec<String> = consistent
        .get("opacity")
        .and_then(|v| v["dominant_values"].as_array())
        .map(|a| {
            a.iter()
                .filter_map(|s| s.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let font_dom_snap: Vec<String> = consistent
        .get("font_family")
        .and_then(|v| v["dominant_values"].as_array())
        .map(|a| {
            a.iter()
                .filter_map(|s| s.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    push_outliers("fill_color", &fill_outliers, &fill_dom_snap, fill_total);
    push_outliers(
        "stroke_width",
        &stroke_outliers,
        &stroke_dom_snap,
        stroke_total,
    );
    push_outliers(
        "opacity",
        &opacity_outliers,
        &opacity_dom_snap,
        opacity_total,
    );
    push_outliers("font_family", &font_outliers, &font_dom_snap, font_total);

    let outlier_count = outlier_items.len();
    let nodes_analysed = nodes.len();

    let summary = if outlier_count == 0 {
        format!(
            "Style is consistent across {} nodes — no outliers found.",
            nodes_analysed
        )
    } else {
        format!(
            "{} style outlier(s) found across {} nodes.",
            outlier_count, nodes_analysed
        )
    };

    ToolResult::text(summary).with_data(serde_json::json!({
        "nodes_analysed": nodes_analysed,
        "outlier_count":  outlier_count,
        "consistent":     consistent,
        "outliers":       outlier_items,
    }))
}

/// Insert a new anchor point at the midpoint of every path segment for each
/// supplied node. Non-path nodes are silently skipped.
pub async fn add_anchor_points(state: &AppState, args: AddAnchorPointsArgs) -> ToolResult {
    if args.node_ids.is_empty() {
        return ToolResult::error("node_ids must not be empty");
    }

    let passes = args.passes.unwrap_or(1).min(8).max(1);

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let mut commands = Vec::new();
    let mut modified = 0usize;
    let mut skipped = 0usize;

    for node_id in &args.node_ids {
        let node = match doc.nodes.get(node_id) {
            Some(n) => n.clone(),
            None => {
                skipped += 1;
                continue;
            }
        };

        match &node.kind {
            SceneNodeKind::Path(pn) => {
                let new_path = pn.path_data.subdivide(passes);
                let mut new_node = node.clone();
                if let SceneNodeKind::Path(ref mut new_pn) = new_node.kind {
                    new_pn.path_data = new_path;
                }
                commands.push(Command::UpdateNode {
                    old: node,
                    new: new_node,
                });
                modified += 1;
            }
            _ => {
                skipped += 1;
            }
        }
    }

    if commands.is_empty() {
        return ToolResult::error("No path nodes found in node_ids");
    }

    for cmd in commands {
        history.execute(cmd, &mut doc);
    }

    let summary = format!(
        "Added anchor points to {} node(s) ({} pass{}){}",
        modified,
        passes,
        if passes == 1 { "" } else { "es" },
        if skipped > 0 {
            format!(" — {} non-path node(s) skipped", skipped)
        } else {
            String::new()
        },
    );
    ToolResult::text(summary).with_data(serde_json::json!({
        "modified": modified,
        "skipped":  skipped,
        "passes":   passes,
    }))
}

pub async fn delete_anchor_point(state: &AppState, args: DeleteAnchorPointArgs) -> ToolResult {
    tracing::debug!("tool: delete_anchor_point");

    if args.anchor_indices.is_empty() {
        return ToolResult::error("anchor_indices must not be empty");
    }

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    // Resolve node by UUID or name.
    let nid = if let Ok(uuid) = uuid::Uuid::parse_str(&args.node_id) {
        uuid
    } else {
        match doc.find_node_by_name(&args.node_id) {
            Some(n) => n.id,
            None => return ToolResult::error(format!("Node not found: {}", args.node_id)),
        }
    };

    let node = match doc.nodes.get(&nid) {
        Some(n) => n.clone(),
        None => return ToolResult::error(format!("Node not found: {}", args.node_id)),
    };

    let pn = match &node.kind {
        SceneNodeKind::Path(pn) => pn,
        _ => return ToolResult::error("Node is not a path"),
    };

    let bez = pn.path_data.to_bez_path();
    let el_count = bez.elements().len();

    // Validate indices.
    for &idx in &args.anchor_indices {
        if idx >= el_count {
            return ToolResult::error(format!(
                "Anchor index {idx} out of range (path has {el_count} elements)"
            ));
        }
    }

    // Remove elements (same algorithm as GUI's bez_remove_elements).
    let remove_set: std::collections::HashSet<usize> =
        args.anchor_indices.iter().copied().collect();
    let mut result = kurbo::BezPath::new();
    let mut needs_move = true;
    for (i, el) in bez.elements().iter().enumerate() {
        if remove_set.contains(&i) {
            needs_move = true;
            continue;
        }
        if needs_move {
            let endpoint = match el {
                kurbo::PathEl::MoveTo(p) | kurbo::PathEl::LineTo(p) => Some(*p),
                kurbo::PathEl::CurveTo(_, _, p) => Some(*p),
                kurbo::PathEl::QuadTo(_, p) => Some(*p),
                kurbo::PathEl::ClosePath => None,
            };
            if let Some(p) = endpoint {
                result.push(kurbo::PathEl::MoveTo(p));
                needs_move = false;
                if !matches!(el, kurbo::PathEl::MoveTo(_)) {
                    result.push(*el);
                }
            }
        } else {
            result.push(*el);
        }
    }

    let mut new_node = node.clone();
    if let SceneNodeKind::Path(ref mut new_pn) = new_node.kind {
        new_pn.path_data = PathData::from_bez_path(&result);
    }

    let removed_count = remove_set.len();
    let new_count = result.elements().len();
    history.execute(
        Command::UpdateNode {
            old: node,
            new: new_node,
        },
        &mut doc,
    );

    ToolResult::text(format!(
        "Deleted {removed_count} anchor(s) — {el_count} → {new_count} elements"
    ))
    .with_data(serde_json::json!({
        "removed": removed_count,
        "elements_before": el_count,
        "elements_after": new_count,
    }))
}

pub async fn zig_zag_path(state: &AppState, args: ZigZagPathArgs) -> ToolResult {
    tracing::debug!("tool: zig_zag_path");

    if args.node_ids.is_empty() {
        return ToolResult::error("node_ids must not be empty");
    }

    let size = args.size.unwrap_or(10.0);
    let ridges = args.ridges_per_segment.unwrap_or(4).max(1);
    let smooth = args.smooth;

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let mut commands = Vec::new();
    let mut modified = 0usize;
    let mut skipped = 0usize;

    for node_id_str in &args.node_ids {
        let nid = match uuid::Uuid::parse_str(node_id_str) {
            Ok(id) => id,
            Err(_) => match doc.find_node_by_name(node_id_str) {
                Some(n) => n.id,
                None => {
                    skipped += 1;
                    continue;
                }
            },
        };
        let node = match doc.nodes.get(&nid) {
            Some(n) => n.clone(),
            None => {
                skipped += 1;
                continue;
            }
        };
        let pn = match &node.kind {
            SceneNodeKind::Path(pn) => pn,
            _ => {
                skipped += 1;
                continue;
            }
        };

        let bez = pn.path_data.to_bez_path();
        let new_bez = apply_zig_zag(&bez, size, ridges, smooth);

        let mut new_node = node.clone();
        if let SceneNodeKind::Path(ref mut new_pn) = new_node.kind {
            new_pn.path_data = PathData::from_bez_path(&new_bez);
        }
        commands.push(Command::UpdateNode {
            old: node,
            new: new_node,
        });
        modified += 1;
    }

    if commands.is_empty() {
        return ToolResult::error("No path nodes found in node_ids");
    }

    for cmd in commands {
        history.execute(cmd, &mut doc);
    }

    ToolResult::text(format!(
        "Applied zig-zag to {} node(s) (size={size}, ridges={ridges}, smooth={smooth}){}",
        modified,
        if skipped > 0 {
            format!(" — {} skipped", skipped)
        } else {
            String::new()
        },
    ))
    .with_data(serde_json::json!({ "modified": modified, "skipped": skipped }))
}

/// Apply zig-zag distortion to every segment of a BezPath.
fn apply_zig_zag(bez: &kurbo::BezPath, size: f64, ridges: usize, smooth: bool) -> kurbo::BezPath {
    use kurbo::{PathEl, Point};

    let mut result = kurbo::BezPath::new();
    let mut current = Point::ZERO;
    let mut subpath_start = Point::ZERO;

    for el in bez.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                result.move_to(p);
                current = p;
                subpath_start = p;
            }
            PathEl::ClosePath => {
                // Zig-zag the closing segment from current to subpath_start.
                if current != subpath_start {
                    zig_zag_segment(&mut result, current, subpath_start, size, ridges, smooth);
                }
                result.close_path();
                current = subpath_start;
            }
            _ => {
                // Flatten curves to a line for simplicity, then zig-zag.
                let seg = match *el {
                    PathEl::LineTo(p) => {
                        current = p;
                        p
                    }
                    PathEl::CurveTo(_, _, p) => {
                        current = p;
                        p
                    }
                    PathEl::QuadTo(_, p) => {
                        current = p;
                        p
                    }
                    _ => unreachable!(),
                };
                let start = match result.elements().last() {
                    Some(PathEl::MoveTo(p)) => *p,
                    _ => {
                        // Walk backward to find the last endpoint.
                        let els = result.elements();
                        let mut pt = Point::ZERO;
                        for e in els.iter().rev() {
                            match e {
                                PathEl::MoveTo(p)
                                | PathEl::LineTo(p)
                                | PathEl::CurveTo(_, _, p)
                                | PathEl::QuadTo(_, p) => {
                                    pt = *p;
                                    break;
                                }
                                PathEl::ClosePath => {}
                            }
                        }
                        pt
                    }
                };
                zig_zag_segment(&mut result, start, seg, size, ridges, smooth);
            }
        }
    }
    result
}

/// Emit zig-zag points between `from` and `to`, appending to `path`.
/// Does NOT emit a MoveTo — assumes the pen is already at `from`.
fn zig_zag_segment(
    path: &mut kurbo::BezPath,
    from: kurbo::Point,
    to: kurbo::Point,
    size: f64,
    ridges: usize,
    smooth: bool,
) {
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1e-9 {
        path.line_to(to);
        return;
    }

    // Unit tangent and normal.
    let tx = dx / len;
    let ty = dy / len;
    let nx = -ty;
    let ny = tx;

    // Total subdivisions = ridges * 2 (each ridge has a peak and a valley).
    let steps = ridges * 2;
    let step_len = len / steps as f64;

    for i in 1..=steps {
        let t = i as f64 / steps as f64;
        let px = from.x + dx * t;
        let py = from.y + dy * t;

        // Alternate displacement: odd = +size/2, even = -size/2.
        // Last point (i == steps) has zero displacement to land on `to`.
        let disp = if i == steps {
            0.0
        } else if i % 2 == 1 {
            size / 2.0
        } else {
            -size / 2.0
        };

        let pt = kurbo::Point::new(px + nx * disp, py + ny * disp);

        if smooth && i < steps {
            // Smooth: use cubic bezier with handles along the tangent direction.
            let handle_len = step_len * 0.3;
            // Previous point displacement.
            let prev_disp = if i == 1 {
                0.0 // from point has no displacement
            } else if (i - 1) % 2 == 1 {
                size / 2.0
            } else {
                -size / 2.0
            };
            let prev_t = (i - 1) as f64 / steps as f64;
            let prev_x = from.x + dx * prev_t + nx * prev_disp;
            let prev_y = from.y + dy * prev_t + ny * prev_disp;

            let cp1 = kurbo::Point::new(prev_x + tx * handle_len, prev_y + ty * handle_len);
            let cp2 = kurbo::Point::new(pt.x - tx * handle_len, pt.y - ty * handle_len);
            path.curve_to(cp1, cp2, pt);
        } else {
            path.line_to(pt);
        }
    }
}

pub async fn pucker_bloat(state: &AppState, args: PuckerBloatArgs) -> ToolResult {
    tracing::debug!("tool: pucker_bloat");

    if args.node_ids.is_empty() {
        return ToolResult::error("node_ids must not be empty");
    }

    let strength = args.strength.unwrap_or(0.5);

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let mut commands = Vec::new();
    let mut modified = 0usize;
    let mut skipped = 0usize;

    for node_id_str in &args.node_ids {
        let nid = match uuid::Uuid::parse_str(node_id_str) {
            Ok(id) => id,
            Err(_) => match doc.find_node_by_name(node_id_str) {
                Some(n) => n.id,
                None => {
                    skipped += 1;
                    continue;
                }
            },
        };
        let node = match doc.nodes.get(&nid) {
            Some(n) => n.clone(),
            None => {
                skipped += 1;
                continue;
            }
        };
        let pn = match &node.kind {
            SceneNodeKind::Path(pn) => pn,
            _ => {
                skipped += 1;
                continue;
            }
        };

        let bez = pn.path_data.to_bez_path();

        // Determine center — use args or compute centroid.
        let center = if let (Some(cx), Some(cy)) = (args.center_x, args.center_y) {
            kurbo::Point::new(cx, cy)
        } else {
            path_centroid(&bez)
        };

        let new_bez = apply_pucker_bloat(&bez, strength, center);

        let mut new_node = node.clone();
        if let SceneNodeKind::Path(ref mut new_pn) = new_node.kind {
            new_pn.path_data = PathData::from_bez_path(&new_bez);
        }
        commands.push(Command::UpdateNode {
            old: node,
            new: new_node,
        });
        modified += 1;
    }

    if commands.is_empty() {
        return ToolResult::error("No path nodes found in node_ids");
    }

    for cmd in commands {
        history.execute(cmd, &mut doc);
    }

    let label = if strength >= 0.0 { "bloat" } else { "pucker" };
    ToolResult::text(format!(
        "Applied {label} (strength={strength}) to {modified} node(s){}",
        if skipped > 0 {
            format!(" — {skipped} skipped")
        } else {
            String::new()
        },
    ))
    .with_data(serde_json::json!({ "modified": modified, "skipped": skipped }))
}

/// Compute the centroid of all on-curve points in a BezPath.
fn path_centroid(bez: &kurbo::BezPath) -> kurbo::Point {
    let mut sum_x = 0.0;
    let mut sum_y = 0.0;
    let mut count = 0usize;
    for el in bez.elements() {
        let pt = match *el {
            kurbo::PathEl::MoveTo(p) | kurbo::PathEl::LineTo(p) => Some(p),
            kurbo::PathEl::CurveTo(_, _, p) => Some(p),
            kurbo::PathEl::QuadTo(_, p) => Some(p),
            kurbo::PathEl::ClosePath => None,
        };
        if let Some(p) = pt {
            sum_x += p.x;
            sum_y += p.y;
            count += 1;
        }
    }
    if count == 0 {
        kurbo::Point::ZERO
    } else {
        kurbo::Point::new(sum_x / count as f64, sum_y / count as f64)
    }
}

/// Displace every point in a BezPath radially from `center`.
/// Positive strength = bloat (outward), negative = pucker (inward).
fn apply_pucker_bloat(bez: &kurbo::BezPath, strength: f64, center: kurbo::Point) -> kurbo::BezPath {
    let displace = |p: kurbo::Point| -> kurbo::Point {
        let dx = p.x - center.x;
        let dy = p.y - center.y;
        let dist = (dx * dx + dy * dy).sqrt();
        if dist < 1e-9 {
            return p;
        }
        // Displacement proportional to distance from center.
        let factor = 1.0 + strength;
        kurbo::Point::new(center.x + dx * factor, center.y + dy * factor)
    };

    let mut result = kurbo::BezPath::new();
    for el in bez.elements() {
        match *el {
            kurbo::PathEl::MoveTo(p) => result.move_to(displace(p)),
            kurbo::PathEl::LineTo(p) => result.line_to(displace(p)),
            kurbo::PathEl::CurveTo(c1, c2, p) => {
                result.curve_to(displace(c1), displace(c2), displace(p))
            }
            kurbo::PathEl::QuadTo(c, p) => result.quad_to(displace(c), displace(p)),
            kurbo::PathEl::ClosePath => result.close_path(),
        }
    }
    result
}

pub async fn roughen_path(state: &AppState, args: RoughenPathArgs) -> ToolResult {
    tracing::debug!("tool: roughen_path");

    if args.node_ids.is_empty() {
        return ToolResult::error("node_ids must not be empty");
    }

    let size = args.size.unwrap_or(5.0);
    let detail = args.detail.unwrap_or(0);
    let seed = args.seed.unwrap_or(42);

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let mut commands = Vec::new();
    let mut modified = 0usize;
    let mut skipped = 0usize;

    for node_id_str in &args.node_ids {
        let nid = match uuid::Uuid::parse_str(node_id_str) {
            Ok(id) => id,
            Err(_) => match doc.find_node_by_name(node_id_str) {
                Some(n) => n.id,
                None => {
                    skipped += 1;
                    continue;
                }
            },
        };
        let node = match doc.nodes.get(&nid) {
            Some(n) => n.clone(),
            None => {
                skipped += 1;
                continue;
            }
        };
        let pn = match &node.kind {
            SceneNodeKind::Path(pn) => pn,
            _ => {
                skipped += 1;
                continue;
            }
        };

        let mut bez = pn.path_data.to_bez_path();

        // Subdivide for extra detail before roughening.
        for _ in 0..detail {
            bez = subdivide_bez(&bez);
        }

        let new_bez = apply_roughen(&bez, size, seed + modified as u64);

        let mut new_node = node.clone();
        if let SceneNodeKind::Path(ref mut new_pn) = new_node.kind {
            new_pn.path_data = PathData::from_bez_path(&new_bez);
        }
        commands.push(Command::UpdateNode {
            old: node,
            new: new_node,
        });
        modified += 1;
    }

    if commands.is_empty() {
        return ToolResult::error("No path nodes found in node_ids");
    }

    for cmd in commands {
        history.execute(cmd, &mut doc);
    }

    ToolResult::text(format!(
        "Roughened {} node(s) (size={size}, detail={detail}){}",
        modified,
        if skipped > 0 {
            format!(" — {skipped} skipped")
        } else {
            String::new()
        },
    ))
    .with_data(serde_json::json!({ "modified": modified, "skipped": skipped }))
}

/// Simple xorshift64 PRNG returning values in [-1.0, 1.0].
fn xorshift64(state: &mut u64) -> f64 {
    let mut s = *state;
    s ^= s << 13;
    s ^= s >> 7;
    s ^= s << 17;
    *state = s;
    // Map to [-1, 1]
    (s as f64 / u64::MAX as f64) * 2.0 - 1.0
}

/// Subdivide every segment of a BezPath once (insert midpoints).
fn subdivide_bez(bez: &kurbo::BezPath) -> kurbo::BezPath {
    let mut result = kurbo::BezPath::new();
    let mut current = kurbo::Point::ZERO;

    for el in bez.elements() {
        match *el {
            kurbo::PathEl::MoveTo(p) => {
                result.move_to(p);
                current = p;
            }
            kurbo::PathEl::LineTo(p) => {
                let mid = kurbo::Point::new((current.x + p.x) / 2.0, (current.y + p.y) / 2.0);
                result.line_to(mid);
                result.line_to(p);
                current = p;
            }
            kurbo::PathEl::CurveTo(c1, c2, p) => {
                // De Casteljau subdivision at t=0.5
                let m01 = mid(current, c1);
                let m12 = mid(c1, c2);
                let m23 = mid(c2, p);
                let m012 = mid(m01, m12);
                let m123 = mid(m12, m23);
                let m0123 = mid(m012, m123);
                result.curve_to(m01, m012, m0123);
                result.curve_to(m123, m23, p);
                current = p;
            }
            kurbo::PathEl::QuadTo(c, p) => {
                let mc0 = mid(current, c);
                let mc1 = mid(c, p);
                let m = mid(mc0, mc1);
                result.quad_to(mc0, m);
                result.quad_to(mc1, p);
                current = p;
            }
            kurbo::PathEl::ClosePath => {
                result.close_path();
            }
        }
    }
    result
}

fn mid(a: kurbo::Point, b: kurbo::Point) -> kurbo::Point {
    kurbo::Point::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0)
}

/// Displace every point in a BezPath by a random amount up to `size`.
fn apply_roughen(bez: &kurbo::BezPath, size: f64, seed: u64) -> kurbo::BezPath {
    let mut rng = seed.max(1); // avoid zero state

    let displace = |p: kurbo::Point, rng: &mut u64| -> kurbo::Point {
        let dx = xorshift64(rng) * size;
        let dy = xorshift64(rng) * size;
        kurbo::Point::new(p.x + dx, p.y + dy)
    };

    let mut result = kurbo::BezPath::new();
    for el in bez.elements() {
        match *el {
            kurbo::PathEl::MoveTo(p) => result.move_to(displace(p, &mut rng)),
            kurbo::PathEl::LineTo(p) => result.line_to(displace(p, &mut rng)),
            kurbo::PathEl::CurveTo(c1, c2, p) => {
                result.curve_to(
                    displace(c1, &mut rng),
                    displace(c2, &mut rng),
                    displace(p, &mut rng),
                );
            }
            kurbo::PathEl::QuadTo(c, p) => {
                result.quad_to(displace(c, &mut rng), displace(p, &mut rng));
            }
            kurbo::PathEl::ClosePath => result.close_path(),
        }
    }
    result
}

pub async fn twirl_path(state: &AppState, args: TwirlPathArgs) -> ToolResult {
    tracing::debug!("tool: twirl_path");

    if args.node_ids.is_empty() {
        return ToolResult::error("node_ids must not be empty");
    }

    let angle_deg = args.angle.unwrap_or(90.0);
    let angle_rad = angle_deg.to_radians();

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let mut commands = Vec::new();
    let mut modified = 0usize;
    let mut skipped = 0usize;

    for node_id_str in &args.node_ids {
        let nid = match uuid::Uuid::parse_str(node_id_str) {
            Ok(id) => id,
            Err(_) => match doc.find_node_by_name(node_id_str) {
                Some(n) => n.id,
                None => {
                    skipped += 1;
                    continue;
                }
            },
        };
        let node = match doc.nodes.get(&nid) {
            Some(n) => n.clone(),
            None => {
                skipped += 1;
                continue;
            }
        };
        let pn = match &node.kind {
            SceneNodeKind::Path(pn) => pn,
            _ => {
                skipped += 1;
                continue;
            }
        };

        let bez = pn.path_data.to_bez_path();
        let center = if let (Some(cx), Some(cy)) = (args.center_x, args.center_y) {
            kurbo::Point::new(cx, cy)
        } else {
            path_centroid(&bez)
        };

        let new_bez = apply_twirl(&bez, angle_rad, center);

        let mut new_node = node.clone();
        if let SceneNodeKind::Path(ref mut new_pn) = new_node.kind {
            new_pn.path_data = PathData::from_bez_path(&new_bez);
        }
        commands.push(Command::UpdateNode {
            old: node,
            new: new_node,
        });
        modified += 1;
    }

    if commands.is_empty() {
        return ToolResult::error("No path nodes found in node_ids");
    }

    for cmd in commands {
        history.execute(cmd, &mut doc);
    }

    ToolResult::text(format!(
        "Twirled {} node(s) by {angle_deg}°{}",
        modified,
        if skipped > 0 {
            format!(" — {skipped} skipped")
        } else {
            String::new()
        },
    ))
    .with_data(serde_json::json!({ "modified": modified, "skipped": skipped }))
}

/// Twirl: rotate each point around `center` by an angle that decreases
/// with distance from center (points near center rotate more → spiral).
fn apply_twirl(bez: &kurbo::BezPath, angle_rad: f64, center: kurbo::Point) -> kurbo::BezPath {
    // Find max distance from center to determine falloff.
    let mut max_dist = 0.0f64;
    for el in bez.elements() {
        let pts: Vec<kurbo::Point> = match *el {
            kurbo::PathEl::MoveTo(p) | kurbo::PathEl::LineTo(p) => vec![p],
            kurbo::PathEl::CurveTo(c1, c2, p) => vec![c1, c2, p],
            kurbo::PathEl::QuadTo(c, p) => vec![c, p],
            kurbo::PathEl::ClosePath => vec![],
        };
        for p in pts {
            let d = ((p.x - center.x).powi(2) + (p.y - center.y).powi(2)).sqrt();
            if d > max_dist {
                max_dist = d;
            }
        }
    }

    if max_dist < 1e-9 {
        return bez.clone();
    }

    let twirl_point = |p: kurbo::Point| -> kurbo::Point {
        let dx = p.x - center.x;
        let dy = p.y - center.y;
        let dist = (dx * dx + dy * dy).sqrt();
        // Rotation angle falls off linearly: full angle at center, 0 at max_dist.
        let t = 1.0 - (dist / max_dist).min(1.0);
        let a = angle_rad * t;
        let cos_a = a.cos();
        let sin_a = a.sin();
        kurbo::Point::new(
            center.x + dx * cos_a - dy * sin_a,
            center.y + dx * sin_a + dy * cos_a,
        )
    };

    let mut result = kurbo::BezPath::new();
    for el in bez.elements() {
        match *el {
            kurbo::PathEl::MoveTo(p) => result.move_to(twirl_point(p)),
            kurbo::PathEl::LineTo(p) => result.line_to(twirl_point(p)),
            kurbo::PathEl::CurveTo(c1, c2, p) => {
                result.curve_to(twirl_point(c1), twirl_point(c2), twirl_point(p))
            }
            kurbo::PathEl::QuadTo(c, p) => result.quad_to(twirl_point(c), twirl_point(p)),
            kurbo::PathEl::ClosePath => result.close_path(),
        }
    }
    result
}

pub async fn blend_objects(state: &AppState, args: BlendObjectsArgs) -> ToolResult {
    tracing::debug!("tool: blend_objects");
    use photonic_core::color::Color;
    use photonic_core::style::{Fill, FillKind};

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    // Resolve both nodes.
    let resolve = |id_str: &str| -> Option<NodeId> {
        uuid::Uuid::parse_str(id_str)
            .ok()
            .or_else(|| doc.find_node_by_name(id_str).map(|n| n.id))
    };
    let nid_a = match resolve(&args.node_id_a) {
        Some(id) => id,
        None => return ToolResult::error(format!("Node A not found: {}", args.node_id_a)),
    };
    let nid_b = match resolve(&args.node_id_b) {
        Some(id) => id,
        None => return ToolResult::error(format!("Node B not found: {}", args.node_id_b)),
    };

    let node_a = doc.nodes.get(&nid_a).cloned();
    let node_b = doc.nodes.get(&nid_b).cloned();

    let (node_a, node_b) = match (node_a, node_b) {
        (Some(a), Some(b)) => (a, b),
        _ => return ToolResult::error("One or both nodes not found"),
    };

    let (pn_a, pn_b) = match (&node_a.kind, &node_b.kind) {
        (SceneNodeKind::Path(a), SceneNodeKind::Path(b)) => (a, b),
        _ => return ToolResult::error("Both nodes must be paths"),
    };

    let bez_a = pn_a.path_data.to_bez_path();
    let bez_b = pn_b.path_data.to_bez_path();

    if bez_a.elements().len() != bez_b.elements().len() {
        return ToolResult::error(format!(
            "Path element counts differ ({} vs {}). Both paths must have the same number of elements for blending. Use add_anchor_points to equalize.",
            bez_a.elements().len(), bez_b.elements().len()
        ));
    }

    // Extract solid fill colors for interpolation.
    let color_a = solid_fill_of(&pn_a.fill);
    let color_b = solid_fill_of(&pn_b.fill);

    // Get translation components for position interpolation.
    let tx_a = (node_a.transform.matrix[4], node_a.transform.matrix[5]);
    let tx_b = (node_b.transform.matrix[4], node_b.transform.matrix[5]);

    // ── Compute steps based on chosen mode ──────────────────────────────────
    let steps = if let Some(sp) = args.spacing {
        // Specified Distance: steps = ceil(center_distance / spacing)
        if sp <= 0.0 {
            return ToolResult::error("spacing must be positive");
        }
        let dx = tx_b.0 - tx_a.0;
        let dy = tx_b.1 - tx_a.1;
        let dist = (dx * dx + dy * dy).sqrt();
        ((dist / sp).ceil() as usize).saturating_sub(1).max(1)
    } else if args.smooth_color {
        // Smooth Color: auto-compute steps so color changes by ≤ 1/255 per step.
        if let (Some(ca), Some(cb)) = (&color_a, &color_b) {
            let dr = ((cb.r - ca.r).abs() * 255.0) as f64;
            let dg = ((cb.g - ca.g).abs() * 255.0) as f64;
            let db = ((cb.b - ca.b).abs() * 255.0) as f64;
            let max_delta = dr.max(dg).max(db);
            (max_delta.ceil() as usize).max(1)
        } else {
            // No solid fill to measure; fall back to default
            args.steps.unwrap_or(5).max(1)
        }
    } else {
        args.steps.unwrap_or(5).max(1)
    };

    let layer_id = node_a.layer_id;
    let mut created_ids = Vec::new();

    for i in 1..=steps {
        let t = i as f64 / (steps + 1) as f64;

        // Interpolate path geometry.
        let mut interp_bez = kurbo::BezPath::new();
        for (ea, eb) in bez_a.elements().iter().zip(bez_b.elements().iter()) {
            match (*ea, *eb) {
                (kurbo::PathEl::MoveTo(a), kurbo::PathEl::MoveTo(b)) => {
                    interp_bez.move_to(lerp_point(a, b, t));
                }
                (kurbo::PathEl::LineTo(a), kurbo::PathEl::LineTo(b)) => {
                    interp_bez.line_to(lerp_point(a, b, t));
                }
                (kurbo::PathEl::CurveTo(a1, a2, a3), kurbo::PathEl::CurveTo(b1, b2, b3)) => {
                    interp_bez.curve_to(
                        lerp_point(a1, b1, t),
                        lerp_point(a2, b2, t),
                        lerp_point(a3, b3, t),
                    );
                }
                (kurbo::PathEl::QuadTo(a1, a2), kurbo::PathEl::QuadTo(b1, b2)) => {
                    interp_bez.quad_to(lerp_point(a1, b1, t), lerp_point(a2, b2, t));
                }
                (kurbo::PathEl::ClosePath, kurbo::PathEl::ClosePath) => {
                    interp_bez.close_path();
                }
                _ => {
                    // Mismatched element types — fall back to element from A.
                    interp_bez.push(*ea);
                }
            }
        }

        let mut new_pn = pn_a.clone();
        new_pn.path_data = PathData::from_bez_path(&interp_bez);

        // Interpolate fill color.
        if let (Some(ca), Some(cb)) = (&color_a, &color_b) {
            new_pn.fill = Fill {
                kind: FillKind::Solid(Color::new(
                    ca.r + (cb.r - ca.r) * t as f32,
                    ca.g + (cb.g - ca.g) * t as f32,
                    ca.b + (cb.b - ca.b) * t as f32,
                    ca.a + (cb.a - ca.a) * t as f32,
                )),
                ..pn_a.fill.clone()
            };
        }

        // Interpolate opacity.
        let opacity = node_a.opacity + (node_b.opacity - node_a.opacity) * t as f32;

        let name = format!("Blend {}/{}", i, steps);
        let mut node = SceneNode::new(&name, layer_id, SceneNodeKind::Path(new_pn));
        node.opacity = opacity;

        // Interpolate transform (translation only for simplicity).
        let interp_tx = (
            tx_a.0 + (tx_b.0 - tx_a.0) * t,
            tx_a.1 + (tx_b.1 - tx_a.1) * t,
        );
        node.transform = Transform::translate(interp_tx.0, interp_tx.1);

        let nid = node.id;
        created_ids.push(nid);
        history.execute(
            Command::AddNode {
                node,
                layer_id: Some(layer_id),
            },
            &mut doc,
        );
    }

    let mode = if args.spacing.is_some() {
        "spacing"
    } else if args.smooth_color {
        "smooth_color"
    } else {
        "steps"
    };
    ToolResult::text(format!(
        "Created {} blend steps between '{}' and '{}' (mode: {})",
        steps, node_a.name, node_b.name, mode
    ))
    .with_data(serde_json::json!({
        "steps": steps,
        "mode": mode,
        "created_ids": created_ids,
    }))
}

fn lerp_point(a: kurbo::Point, b: kurbo::Point, t: f64) -> kurbo::Point {
    kurbo::Point::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t)
}

pub async fn create_parametric_shape(
    state: &AppState,
    args: CreateParametricShapeArgs,
) -> ToolResult {
    tracing::debug!("tool: create_parametric_shape");

    let cx = args.cx;
    let cy = args.cy;
    let radius = args.radius.unwrap_or(80.0);
    let n_pts = args.points.unwrap_or(360).max(3).min(4096);
    let rx = radius * args.ratio_x.unwrap_or(1.0);
    let ry = radius * args.ratio_y.unwrap_or(1.0);

    // Generate (x, y) sample points in object space.
    let pts: Vec<(f64, f64)> = match args.shape_type {
        ParametricShapeType::Lissajous => {
            let freq_a = args.freq_a.unwrap_or(3.0);
            let freq_b = args.freq_b.unwrap_or(2.0);
            let delta = args.delta_deg.unwrap_or(90.0).to_radians();
            (0..n_pts)
                .map(|i| {
                    let t = i as f64 / n_pts as f64 * std::f64::consts::TAU;
                    (rx * (freq_a * t + delta).sin(), ry * (freq_b * t).sin())
                })
                .collect()
        }
        ParametricShapeType::Superellipse => {
            let n = args.exponent.unwrap_or(2.5).max(0.1);
            (0..n_pts)
                .map(|i| {
                    let t = i as f64 / n_pts as f64 * std::f64::consts::TAU;
                    let cos_t = t.cos();
                    let sin_t = t.sin();
                    // |x/rx|^n + |y/ry|^n = 1 → parameterized as x = rx·sgn(cos)·|cos|^(2/n)
                    let x = rx * cos_t.signum() * cos_t.abs().powf(2.0 / n);
                    let y = ry * sin_t.signum() * sin_t.abs().powf(2.0 / n);
                    (x, y)
                })
                .collect()
        }
        ParametricShapeType::Rose => {
            let k = args.petals.unwrap_or(5.0);
            (0..n_pts)
                .map(|i| {
                    // Integrate over 2π (even k) or π (odd k) for a closed rose.
                    let t_max = if (k.round() as i64 % 2 == 0) && k.fract() < 1e-9 {
                        std::f64::consts::TAU
                    } else {
                        std::f64::consts::PI
                    };
                    let t = i as f64 / n_pts as f64 * t_max;
                    let r = radius * (k * t).cos();
                    (r * t.cos(), r * t.sin())
                })
                .collect()
        }
        ParametricShapeType::Hypotrochoid => {
            let r_ratio = args.inner_ratio.unwrap_or(0.4).clamp(0.01, 0.99);
            let pen_r = args.pen_ratio.unwrap_or(1.0);
            let big_r = radius / (1.0 + r_ratio * (pen_r - 1.0).abs().max(1.0) + r_ratio);
            let r = big_r * r_ratio;
            let d = r * pen_r;
            (0..n_pts)
                .map(|i| {
                    let t =
                        i as f64 / n_pts as f64 * std::f64::consts::TAU * r_ratio.recip().ceil();
                    let x = (big_r - r) * t.cos() + d * ((big_r - r) / r * t).cos();
                    let y = (big_r - r) * t.sin() - d * ((big_r - r) / r * t).sin();
                    (x, y)
                })
                .collect()
        }
        ParametricShapeType::Epicycloid => {
            let r_ratio = args.inner_ratio.unwrap_or(0.3).clamp(0.01, 0.99);
            let big_r = radius / (1.0 + r_ratio);
            let r = big_r * r_ratio;
            let d = r * args.pen_ratio.unwrap_or(1.0);
            let loops = (1.0 / r_ratio).round().max(1.0) as usize;
            (0..n_pts)
                .map(|i| {
                    let t = i as f64 / n_pts as f64 * std::f64::consts::TAU * loops as f64;
                    let x = (big_r + r) * t.cos() - d * ((big_r + r) / r * t).cos();
                    let y = (big_r + r) * t.sin() - d * ((big_r + r) / r * t).sin();
                    (x, y)
                })
                .collect()
        }
    };

    if pts.is_empty() {
        return ToolResult::error("no points generated");
    }

    // Build BezPath from sample points (polyline, closed).
    let mut bez = kurbo::BezPath::new();
    for (i, (px, py)) in pts.iter().enumerate() {
        let pt = kurbo::Point::new(cx + px, cy + py);
        if i == 0 {
            bez.move_to(pt);
        } else {
            bez.line_to(pt);
        }
    }
    bez.close_path();

    let mut pn = PathNode::new(PathData::from_bez_path(&bez));
    if let Err(e) = apply_style(&mut pn, args.fill, args.stroke) {
        return ToolResult::error(e);
    }

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let layer_id = args
        .layer_id
        .and_then(|s| uuid::Uuid::parse_str(&s).ok())
        .or(doc.active_layer_id)
        .unwrap_or(uuid::Uuid::nil());

    let shape_name = match args.shape_type {
        ParametricShapeType::Lissajous => "Lissajous Curve",
        ParametricShapeType::Superellipse => "Superellipse",
        ParametricShapeType::Rose => "Rose Curve",
        ParametricShapeType::Hypotrochoid => "Hypotrochoid",
        ParametricShapeType::Epicycloid => "Epicycloid",
    };
    let node = SceneNode::new(shape_name, layer_id, SceneNodeKind::Path(pn));
    let node_id = node.id;
    history.execute(
        Command::AddNode {
            node,
            layer_id: Some(layer_id),
        },
        &mut doc,
    );

    ToolResult::text(format!(
        "Created {shape_name} at ({cx},{cy}) with {n_pts} points"
    ))
    .with_data(serde_json::json!({ "node_id": node_id }))
}

pub async fn create_truchet_tiling(state: &AppState, args: CreateTruchetTilingArgs) -> ToolResult {
    tracing::debug!("tool: create_truchet_tiling");

    let x = args.x;
    let y = args.y;
    let width = args.width.unwrap_or(200.0).max(4.0);
    let height = args.height.unwrap_or(200.0).max(4.0);
    let ts = args.tile_size.unwrap_or(40.0).clamp(4.0, 400.0);
    let seed = args.seed.unwrap_or(42);
    let style = args.style.as_deref().unwrap_or("arcs");
    let sw = args.stroke_width.unwrap_or(2.0).max(0.1);

    // Parse colors.
    let tile_color = args
        .color
        .as_deref()
        .and_then(|s| photonic_core::Color::from_hex(s))
        .unwrap_or(photonic_core::Color::new(0.10, 0.10, 0.18, 1.0));

    let cols = (width / ts).ceil() as usize;
    let rows = (height / ts).ceil() as usize;

    // Cap at 50×50 to avoid creating thousands of nodes.
    let cols = cols.min(50);
    let rows = rows.min(50);

    // Simple LCG pseudo-random number generator (no external deps).
    let mut rng_state = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    let mut next_bool = move || -> bool {
        rng_state = rng_state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (rng_state >> 33) & 1 == 0
    };

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let layer_id = args
        .layer_id
        .as_deref()
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
        .or(doc.active_layer_id)
        .unwrap_or(uuid::Uuid::nil());

    let mut child_ids: Vec<photonic_core::node::NodeId> = Vec::new();

    // Optional background rectangle.
    if let Some(bg_hex) = &args.background {
        if let Some(bg_color) = photonic_core::Color::from_hex(bg_hex) {
            let mut bg_bez = kurbo::BezPath::new();
            bg_bez.move_to(kurbo::Point::new(x, y));
            bg_bez.line_to(kurbo::Point::new(x + width, y));
            bg_bez.line_to(kurbo::Point::new(x + width, y + height));
            bg_bez.line_to(kurbo::Point::new(x, y + height));
            bg_bez.close_path();
            let mut bg_pn = PathNode::new(photonic_core::path::PathData::from_bez_path(&bg_bez));
            bg_pn.fill = photonic_core::style::Fill::solid(bg_color);
            bg_pn.stroke = photonic_core::style::Stroke::none();
            let bg_node = SceneNode::new("background", layer_id, SceneNodeKind::Path(bg_pn));
            let bg_id = bg_node.id;
            history.execute(
                photonic_core::history::Command::AddNode {
                    node: bg_node,
                    layer_id: Some(layer_id),
                },
                &mut doc,
            );
            child_ids.push(bg_id);
        }
    }

    for row in 0..rows {
        for col in 0..cols {
            let tx = x + col as f64 * ts;
            let ty = y + row as f64 * ts;
            let flip = next_bool(); // each tile is one of 2 orientations

            let mut bez = kurbo::BezPath::new();

            match style {
                "diagonals" => {
                    // Two diagonal lines per tile — one of two crossing patterns.
                    if flip {
                        // top-left to bottom-right
                        bez.move_to(kurbo::Point::new(tx, ty));
                        bez.line_to(kurbo::Point::new(tx + ts, ty + ts));
                    } else {
                        // top-right to bottom-left
                        bez.move_to(kurbo::Point::new(tx + ts, ty));
                        bez.line_to(kurbo::Point::new(tx, ty + ts));
                    }
                }
                "triangles" => {
                    // Filled triangle (one of two orientations).
                    if flip {
                        bez.move_to(kurbo::Point::new(tx, ty));
                        bez.line_to(kurbo::Point::new(tx + ts, ty));
                        bez.line_to(kurbo::Point::new(tx, ty + ts));
                    } else {
                        bez.move_to(kurbo::Point::new(tx + ts, ty));
                        bez.line_to(kurbo::Point::new(tx + ts, ty + ts));
                        bez.line_to(kurbo::Point::new(tx, ty + ts));
                    }
                    bez.close_path();
                }
                _ => {
                    // "arcs" (default): two quarter-circle arcs connecting mid-edge pairs.
                    let mid = ts / 2.0;
                    let r = mid; // arc radius = half tile side
                    if flip {
                        // Arc: top-mid → left-mid  AND  bottom-mid → right-mid
                        // Approximate arc with a cubic Bézier (kappa ≈ 0.5523).
                        let k = r * 0.5523;
                        // Arc 1: top-mid → left-mid (curves through top-left corner)
                        let p0 = kurbo::Point::new(tx + mid, ty);
                        let p3 = kurbo::Point::new(tx, ty + mid);
                        bez.move_to(p0);
                        bez.curve_to(
                            kurbo::Point::new(tx + mid - k, ty),
                            kurbo::Point::new(tx, ty + mid - k),
                            p3,
                        );
                        // Arc 2: bottom-mid → right-mid (curves through bottom-right corner)
                        let q0 = kurbo::Point::new(tx + mid, ty + ts);
                        let q3 = kurbo::Point::new(tx + ts, ty + mid);
                        bez.move_to(q0);
                        bez.curve_to(
                            kurbo::Point::new(tx + mid + k, ty + ts),
                            kurbo::Point::new(tx + ts, ty + mid + k),
                            q3,
                        );
                    } else {
                        // Arc: top-mid → right-mid  AND  bottom-mid → left-mid
                        let k = r * 0.5523;
                        // Arc 1: top-mid → right-mid (curves through top-right corner)
                        let p0 = kurbo::Point::new(tx + mid, ty);
                        let p3 = kurbo::Point::new(tx + ts, ty + mid);
                        bez.move_to(p0);
                        bez.curve_to(
                            kurbo::Point::new(tx + mid + k, ty),
                            kurbo::Point::new(tx + ts, ty + mid - k),
                            p3,
                        );
                        // Arc 2: bottom-mid → left-mid (curves through bottom-left corner)
                        let q0 = kurbo::Point::new(tx + mid, ty + ts);
                        let q3 = kurbo::Point::new(tx, ty + mid);
                        bez.move_to(q0);
                        bez.curve_to(
                            kurbo::Point::new(tx + mid - k, ty + ts),
                            kurbo::Point::new(tx, ty + mid + k),
                            q3,
                        );
                    }
                }
            }

            let mut pn = PathNode::new(photonic_core::path::PathData::from_bez_path(&bez));
            match style {
                "triangles" => {
                    pn.fill = photonic_core::style::Fill::solid(tile_color);
                    pn.stroke = photonic_core::style::Stroke::none();
                }
                _ => {
                    pn.fill = photonic_core::style::Fill::none();
                    pn.stroke = photonic_core::style::Stroke::solid(tile_color, sw);
                }
            }

            let label = format!("tile_{row}_{col}");
            let node = SceneNode::new(&label, layer_id, SceneNodeKind::Path(pn));
            let nid = node.id;
            history.execute(
                photonic_core::history::Command::AddNode {
                    node,
                    layer_id: Some(layer_id),
                },
                &mut doc,
            );
            child_ids.push(nid);
        }
    }

    // Group all tiles.
    let group = SceneNode::new(
        "Truchet Tiling",
        layer_id,
        SceneNodeKind::Group(GroupNode::new()),
    );
    let group_id = group.id.to_string();
    history.execute(
        photonic_core::history::Command::GroupNodes {
            group,
            layer_id,
            insert_index: 0,
            children: child_ids,
        },
        &mut doc,
    );

    ToolResult::text(format!("Created Truchet tiling: {cols}×{rows} tiles")).with_data(
        serde_json::json!({
            "group_id": group_id,
            "cols": cols,
            "rows": rows,
            "tiles": cols * rows,
        }),
    )
}

pub async fn create_heart(state: &AppState, args: CreateHeartArgs) -> ToolResult {
    tracing::debug!("tool: create_heart");

    let s = args.size.unwrap_or(60.0);
    let cx = args.cx;
    let cy = args.cy;
    let half = s / 2.0;

    // Heart shape using cubic bezier curves.
    // Bottom tip at (cx, cy), top center dip, two rounded lobes.
    let mut bez = kurbo::BezPath::new();

    // Start at bottom tip.
    bez.move_to((cx, cy));

    // Left lobe: bottom tip → left side → top-left lobe → center dip
    bez.curve_to(
        (cx - half * 0.3, cy - half * 0.6), // cp1
        (cx - half, cy - half * 0.9),       // cp2
        (cx - half, cy - half * 1.2),       // left peak
    );
    bez.curve_to(
        (cx - half, cy - half * 1.6),       // cp1
        (cx - half * 0.4, cy - half * 1.7), // cp2
        (cx, cy - half * 1.4),              // center dip
    );

    // Right lobe: center dip → top-right lobe → right side → bottom tip
    bez.curve_to(
        (cx + half * 0.4, cy - half * 1.7), // cp1
        (cx + half, cy - half * 1.6),       // cp2
        (cx + half, cy - half * 1.2),       // right peak
    );
    bez.curve_to(
        (cx + half, cy - half * 0.9),       // cp1
        (cx + half * 0.3, cy - half * 0.6), // cp2
        (cx, cy),                           // back to bottom tip
    );
    bez.close_path();

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let layer_id = args
        .layer_id
        .and_then(|s| uuid::Uuid::parse_str(&s).ok())
        .or(doc.active_layer_id)
        .unwrap_or(uuid::Uuid::nil());

    let mut pn = PathNode::new(PathData::from_bez_path(&bez));
    // Default to red fill if none specified.
    if args.fill.is_none() && args.stroke.is_none() {
        pn.fill = photonic_core::style::Fill {
            kind: photonic_core::style::FillKind::Solid(photonic_core::color::Color::new(
                0.9, 0.1, 0.2, 1.0,
            )),
            ..Default::default()
        };
        pn.stroke = photonic_core::style::Stroke::none();
    } else {
        if let Err(e) = apply_style(&mut pn, args.fill, args.stroke) {
            return ToolResult::error(e);
        }
    }

    let node = SceneNode::new("Heart", layer_id, SceneNodeKind::Path(pn));
    let node_id = node.id;
    history.execute(
        Command::AddNode {
            node,
            layer_id: Some(layer_id),
        },
        &mut doc,
    );

    ToolResult::text(format!("Created heart at ({cx},{cy}), size={s}"))
        .with_data(serde_json::json!({ "node_id": node_id }))
}

pub async fn create_gear(state: &AppState, args: CreateGearArgs) -> ToolResult {
    tracing::debug!("tool: create_gear");
    use kurbo::Shape;

    let outer_r = args.outer_radius.unwrap_or(50.0);
    let inner_r = args.inner_radius.unwrap_or(35.0);
    let hole_r = args.hole_radius.unwrap_or(10.0);
    let teeth = args.teeth.unwrap_or(12).max(3);
    let cx = args.cx;
    let cy = args.cy;

    let mut bez = kurbo::BezPath::new();
    let tooth_angle = std::f64::consts::TAU / teeth as f64;

    // Each tooth has 4 points: inner-left, outer-left, outer-right, inner-right.
    // The tooth occupies half the angular span, gap occupies the other half.
    let tooth_frac = 0.4; // fraction of tooth_angle occupied by tooth top
    let gap_frac = 1.0 - tooth_frac;

    for i in 0..teeth {
        let base_a = tooth_angle * i as f64;
        let a0 = base_a; // start of gap (inner)
        let a1 = base_a + tooth_angle * gap_frac * 0.5; // start of tooth (inner→outer)
        let a2 = base_a + tooth_angle * (gap_frac * 0.5 + tooth_frac * 0.25); // outer left
        let a3 = base_a + tooth_angle * (1.0 - gap_frac * 0.5 - tooth_frac * 0.25); // outer right
        let a4 = base_a + tooth_angle * (1.0 - gap_frac * 0.5); // end of tooth (outer→inner)

        let pts = [
            (inner_r, a0),
            (inner_r, a1),
            (outer_r, a2),
            (outer_r, a3),
            (inner_r, a4),
        ];

        for (j, &(r, a)) in pts.iter().enumerate() {
            let px = cx + r * a.cos();
            let py = cy + r * a.sin();
            if i == 0 && j == 0 {
                bez.move_to((px, py));
            } else {
                bez.line_to((px, py));
            }
        }
    }
    bez.close_path();

    // Add center hole as reversed circle.
    if hole_r > 0.0 {
        let hole = kurbo::Ellipse::new((cx, cy), (hole_r, hole_r), 0.0).to_path(0.1);
        let hole_els: Vec<_> = hole.elements().to_vec();
        let reversed = reverse_bez(&hole_els);
        for el in &reversed {
            bez.push(*el);
        }
    }

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let layer_id = args
        .layer_id
        .and_then(|s| uuid::Uuid::parse_str(&s).ok())
        .or(doc.active_layer_id)
        .unwrap_or(uuid::Uuid::nil());

    let mut pn = PathNode::new(PathData::from_bez_path(&bez));
    if let Err(e) = apply_style(&mut pn, args.fill, args.stroke) {
        return ToolResult::error(e);
    }

    let node = SceneNode::new("Gear", layer_id, SceneNodeKind::Path(pn));
    let node_id = node.id;
    history.execute(
        Command::AddNode {
            node,
            layer_id: Some(layer_id),
        },
        &mut doc,
    );

    ToolResult::text(format!(
        "Created gear at ({cx},{cy}) — {teeth} teeth, outer={outer_r}, inner={inner_r}"
    ))
    .with_data(serde_json::json!({ "node_id": node_id, "teeth": teeth }))
}

pub async fn tag_nodes(state: &AppState, args: TagNodesArgs) -> ToolResult {
    tracing::debug!("tool: tag_nodes");

    if args.add.is_empty() && args.remove.is_empty() {
        return ToolResult::error("Specify at least one tag to add or remove");
    }

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let node_ids: Vec<NodeId> = if args.node_ids.is_empty() {
        doc.selection.node_ids.iter().copied().collect()
    } else {
        args.node_ids
            .iter()
            .filter_map(|s| {
                uuid::Uuid::parse_str(s)
                    .ok()
                    .or_else(|| doc.find_node_by_name(s).map(|n| n.id))
            })
            .collect()
    };

    if node_ids.is_empty() {
        return ToolResult::error("No nodes specified and nothing selected");
    }

    let mut modified = 0usize;
    for nid in &node_ids {
        let node = match doc.nodes.get(nid) {
            Some(n) => n.clone(),
            None => continue,
        };
        let mut new_node = node.clone();
        // Remove specified tags.
        for tag in &args.remove {
            new_node.tags.retain(|t| t != tag);
        }
        // Add specified tags (avoid duplicates).
        for tag in &args.add {
            if !new_node.tags.contains(tag) {
                new_node.tags.push(tag.clone());
            }
        }
        if new_node.tags != node.tags {
            history.execute(
                Command::UpdateNode {
                    old: node,
                    new: new_node,
                },
                &mut doc,
            );
            modified += 1;
        }
    }

    ToolResult::text(format!(
        "Tagged {modified} node(s) — added [{}], removed [{}]",
        args.add.join(", "),
        args.remove.join(", ")
    ))
    .with_data(serde_json::json!({ "modified": modified }))
}

pub async fn sample_color_at(state: &AppState, args: SampleColorAtArgs) -> ToolResult {
    tracing::debug!("tool: sample_color_at");
    use kurbo::Shape;
    use photonic_core::style::FillKind;

    let doc = state.document.lock().await;
    let pt = kurbo::Point::new(args.x, args.y);

    // Find the topmost visible node whose bounding box contains the point.
    // We iterate layers top-to-bottom, nodes top-to-bottom.
    for lid in doc.layer_order.iter().rev() {
        let layer = match doc.layers.get(lid) {
            Some(l) if l.visible => l,
            _ => continue,
        };
        for nid in layer.node_ids.iter().rev() {
            let node = match doc.nodes.get(nid) {
                Some(n) if n.visible => n,
                _ => continue,
            };
            if let SceneNodeKind::Path(pn) = &node.kind {
                let bez = pn.path_data.to_bez_path();
                // Simple hit test: check if point is inside the path (winding rule).
                if bez.winding(pt) != 0 {
                    // Found it — return fill and stroke colors.
                    let fill_hex = match &pn.fill.kind {
                        FillKind::Solid(c) => Some(c.to_hex()),
                        _ => None,
                    };
                    let stroke_hex = if pn.stroke.enabled {
                        Some(pn.stroke.color.to_hex())
                    } else {
                        None
                    };

                    return ToolResult::text(format!(
                        "Sampled '{}': fill={}, stroke={}",
                        node.name,
                        fill_hex.as_deref().unwrap_or("none"),
                        stroke_hex.as_deref().unwrap_or("none"),
                    ))
                    .with_data(serde_json::json!({
                        "node_id": nid,
                        "node_name": node.name,
                        "fill_color": fill_hex,
                        "stroke_color": stroke_hex,
                        "opacity": node.opacity,
                    }));
                }
            }
        }
    }

    ToolResult::text(format!("No node at ({}, {})", args.x, args.y))
        .with_data(serde_json::json!({ "node_id": null, "fill_color": null }))
}

pub async fn move_to_layer(state: &AppState, args: MoveToLayerArgs) -> ToolResult {
    tracing::debug!("tool: move_to_layer");

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    // Resolve target layer.
    let target_lid = if let Ok(uuid) = uuid::Uuid::parse_str(&args.target_layer) {
        uuid
    } else {
        match doc.layers.values().find(|l| l.name == args.target_layer) {
            Some(l) => l.id,
            None => return ToolResult::error(format!("Layer not found: {}", args.target_layer)),
        }
    };

    if !doc.layers.contains_key(&target_lid) {
        return ToolResult::error("Target layer not found");
    }

    let node_ids: Vec<NodeId> = if args.node_ids.is_empty() {
        doc.selection.node_ids.iter().copied().collect()
    } else {
        args.node_ids
            .iter()
            .filter_map(|s| {
                uuid::Uuid::parse_str(s)
                    .ok()
                    .or_else(|| doc.find_node_by_name(s).map(|n| n.id))
            })
            .collect()
    };

    if node_ids.is_empty() {
        return ToolResult::error("No nodes specified and nothing selected");
    }

    let mut moved = 0usize;
    for nid in &node_ids {
        let node = match doc.nodes.get(nid) {
            Some(n) => n,
            None => continue,
        };
        let old_layer_id = node.layer_id;
        if old_layer_id == target_lid {
            continue;
        }

        let old_index = doc
            .layers
            .get(&old_layer_id)
            .and_then(|l| l.node_ids.iter().position(|id| id == nid))
            .unwrap_or(0);

        let new_index = doc
            .layers
            .get(&target_lid)
            .map(|l| l.node_ids.len())
            .unwrap_or(0);

        history.execute(
            Command::MoveNodeToLayer {
                node_id: *nid,
                old_layer_id,
                new_layer_id: target_lid,
                old_index,
                new_index,
            },
            &mut doc,
        );
        moved += 1;
    }

    ToolResult::text(format!(
        "Moved {moved} node(s) to layer '{}'",
        args.target_layer
    ))
    .with_data(serde_json::json!({ "moved": moved, "target_layer": target_lid }))
}

pub async fn add_dimension_line(state: &AppState, args: AddDimensionLineArgs) -> ToolResult {
    tracing::debug!("tool: add_dimension_line");
    use photonic_core::color::Color;
    use photonic_core::node::TextNode;
    use photonic_core::style::{Fill, FillKind, Stroke};

    let offset = args.offset.unwrap_or(20.0);
    let font_size = args.font_size.unwrap_or(12.0);
    let color_hex = args.color.as_deref().unwrap_or("#666666");
    let color = Color::from_hex(color_hex).unwrap_or(Color::new(0.4, 0.4, 0.4, 1.0));

    let dx = args.x2 - args.x1;
    let dy = args.y2 - args.y1;
    let dist = (dx * dx + dy * dy).sqrt();
    if dist < 1e-9 {
        return ToolResult::error("Points are too close together");
    }

    // Normal direction (perpendicular to the line).
    let nx = -dy / dist;
    let ny = dx / dist;

    // Offset points for the dimension line.
    let ox1 = args.x1 + nx * offset;
    let oy1 = args.y1 + ny * offset;
    let ox2 = args.x2 + nx * offset;
    let oy2 = args.y2 + ny * offset;

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let layer_id = args
        .layer_id
        .and_then(|s| uuid::Uuid::parse_str(&s).ok())
        .or(doc.active_layer_id)
        .unwrap_or(uuid::Uuid::nil());

    let mut child_ids = Vec::new();

    // 1. Extension lines from measured points to dimension line.
    let ext_overshoot = 5.0;
    for &(px, py, ox, oy) in &[(args.x1, args.y1, ox1, oy1), (args.x2, args.y2, ox2, oy2)] {
        let mut bez = kurbo::BezPath::new();
        bez.move_to((px + nx * 3.0, py + ny * 3.0)); // Small gap from the point.
        bez.line_to((ox + nx * ext_overshoot, oy + ny * ext_overshoot));
        let mut pn = PathNode::new(PathData::from_bez_path(&bez));
        pn.fill = Fill::none();
        pn.stroke = Stroke {
            color,
            width: 0.5,
            enabled: true,
            ..Default::default()
        };
        let node = SceneNode::new("Dim Ext", layer_id, SceneNodeKind::Path(pn));
        child_ids.push(node.id);
        history.execute(
            Command::AddNode {
                node,
                layer_id: Some(layer_id),
            },
            &mut doc,
        );
    }

    // 2. Dimension line with arrowheads.
    let arrow_size = 6.0;
    let tx = dx / dist;
    let ty = dy / dist;
    let mut dim_line = kurbo::BezPath::new();
    dim_line.move_to((ox1, oy1));
    dim_line.line_to((ox2, oy2));
    // Left arrowhead.
    dim_line.move_to((ox1, oy1));
    dim_line.line_to((
        ox1 + tx * arrow_size + nx * arrow_size * 0.3,
        oy1 + ty * arrow_size + ny * arrow_size * 0.3,
    ));
    dim_line.move_to((ox1, oy1));
    dim_line.line_to((
        ox1 + tx * arrow_size - nx * arrow_size * 0.3,
        oy1 + ty * arrow_size - ny * arrow_size * 0.3,
    ));
    // Right arrowhead.
    dim_line.move_to((ox2, oy2));
    dim_line.line_to((
        ox2 - tx * arrow_size + nx * arrow_size * 0.3,
        oy2 - ty * arrow_size + ny * arrow_size * 0.3,
    ));
    dim_line.move_to((ox2, oy2));
    dim_line.line_to((
        ox2 - tx * arrow_size - nx * arrow_size * 0.3,
        oy2 - ty * arrow_size - ny * arrow_size * 0.3,
    ));

    let mut pn = PathNode::new(PathData::from_bez_path(&dim_line));
    pn.fill = Fill::none();
    pn.stroke = Stroke {
        color,
        width: 1.0,
        enabled: true,
        ..Default::default()
    };
    let node = SceneNode::new("Dim Line", layer_id, SceneNodeKind::Path(pn));
    child_ids.push(node.id);
    history.execute(
        Command::AddNode {
            node,
            layer_id: Some(layer_id),
        },
        &mut doc,
    );

    // 3. Text label at midpoint.
    let mid_x = (ox1 + ox2) / 2.0;
    let mid_y = (oy1 + oy2) / 2.0;
    let label = format!("{:.1}", dist);
    let mut text_node = TextNode::new(&label);
    text_node.font_size = font_size;
    text_node.fill = Fill {
        kind: FillKind::Solid(color),
        ..Default::default()
    };
    let mut node = SceneNode::new("Dim Label", layer_id, SceneNodeKind::Text(text_node));
    node.transform = Transform::translate(
        mid_x - font_size * label.len() as f64 * 0.3,
        mid_y - font_size * 0.7,
    );
    child_ids.push(node.id);
    history.execute(
        Command::AddNode {
            node,
            layer_id: Some(layer_id),
        },
        &mut doc,
    );

    // 4. Group everything.
    let group = SceneNode::new(
        "Dimension",
        layer_id,
        SceneNodeKind::Group(GroupNode::new()),
    );
    let group_id = group.id;
    history.execute(
        Command::GroupNodes {
            group,
            layer_id,
            insert_index: 0,
            children: child_ids.clone(),
        },
        &mut doc,
    );

    ToolResult::text(format!("Added dimension line: {:.1} units", dist))
        .with_data(serde_json::json!({ "group_id": group_id, "distance": dist }))
}

pub async fn set_selection(state: &AppState, args: SetSelectionArgs) -> ToolResult {
    tracing::debug!("tool: set_selection");

    let mut doc = state.document.lock().await;

    if !args.additive {
        doc.selection.clear();
    }

    let mut added = 0usize;
    for id_str in &args.node_ids {
        let nid = uuid::Uuid::parse_str(id_str)
            .ok()
            .or_else(|| doc.find_node_by_name(id_str).map(|n| n.id));
        if let Some(id) = nid {
            if doc.nodes.contains_key(&id) {
                doc.selection.add(id);
                added += 1;
            }
        }
    }

    let total = doc.selection.node_ids.len();
    ToolResult::text(format!("Selection: {added} added, {total} total"))
        .with_data(serde_json::json!({ "added": added, "total": total }))
}

pub async fn get_selection(state: &AppState) -> ToolResult {
    tracing::debug!("tool: get_selection");

    let doc = state.document.lock().await;
    let ids: Vec<NodeId> = doc.selection.node_ids.iter().copied().collect();
    let count = ids.len();

    let nodes_info: Vec<serde_json::Value> = ids
        .iter()
        .filter_map(|nid| {
            doc.nodes.get(nid).map(|n| {
                let kind = match &n.kind {
                    SceneNodeKind::Path(_) => "path",
                    SceneNodeKind::Text(_) => "text",
                    SceneNodeKind::Group(_) => "group",
                };
                serde_json::json!({
                    "id": nid,
                    "name": n.name,
                    "kind": kind,
                    "visible": n.visible,
                    "locked": n.locked,
                })
            })
        })
        .collect();

    if count == 0 {
        ToolResult::text("Nothing selected")
            .with_data(serde_json::json!({ "count": 0, "nodes": [] }))
    } else {
        ToolResult::text(format!("{count} node(s) selected"))
            .with_data(serde_json::json!({ "count": count, "nodes": nodes_info }))
    }
}

pub async fn flatten_group(state: &AppState, args: FlattenGroupArgs) -> ToolResult {
    tracing::debug!("tool: flatten_group");

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let node_ids: Vec<NodeId> = if args.node_ids.is_empty() {
        doc.selection.node_ids.iter().copied().collect()
    } else {
        args.node_ids
            .iter()
            .filter_map(|s| {
                uuid::Uuid::parse_str(s)
                    .ok()
                    .or_else(|| doc.find_node_by_name(s).map(|n| n.id))
            })
            .collect()
    };

    if node_ids.is_empty() {
        return ToolResult::error("No nodes specified and nothing selected");
    }

    // Collect all group IDs that need flattening (recursive).
    fn collect_groups(doc: &photonic_core::Document, nid: NodeId, result: &mut Vec<NodeId>) {
        if let Some(node) = doc.nodes.get(&nid) {
            if let SceneNodeKind::Group(g) = &node.kind {
                // Depth-first: flatten children first.
                for &child_id in &g.children {
                    collect_groups(doc, child_id, result);
                }
                result.push(nid);
            }
        }
    }

    let mut groups_to_ungroup = Vec::new();
    for &nid in &node_ids {
        collect_groups(&doc, nid, &mut groups_to_ungroup);
    }

    if groups_to_ungroup.is_empty() {
        return ToolResult::error("No groups found to flatten");
    }

    // Ungroup from innermost to outermost (depth-first order).
    let mut ungrouped = 0usize;
    for group_id in &groups_to_ungroup {
        // Re-check because previous ungroupings may have changed the tree.
        let node = match doc.nodes.get(group_id) {
            Some(n) => n.clone(),
            None => continue,
        };
        if let SceneNodeKind::Group(g) = &node.kind {
            let children = g.children.clone();
            let layer_id = node.layer_id;

            // Find the group's index in its layer.
            let group_index = doc
                .layers
                .get(&layer_id)
                .and_then(|l| l.node_ids.iter().position(|id| id == group_id))
                .unwrap_or(0);

            history.execute(
                Command::UngroupNodes {
                    group: node,
                    layer_id,
                    group_index,
                    children,
                },
                &mut doc,
            );
            ungrouped += 1;
        }
    }

    ToolResult::text(format!("Flattened {ungrouped} group(s)"))
        .with_data(serde_json::json!({ "ungrouped": ungrouped }))
}

pub async fn center_on_canvas(state: &AppState, args: CenterOnCanvasArgs) -> ToolResult {
    tracing::debug!("tool: center_on_canvas");
    use kurbo::Shape;

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let canvas_cx = doc.width / 2.0;
    let canvas_cy = doc.height / 2.0;

    let node_ids: Vec<NodeId> = if args.node_ids.is_empty() {
        doc.selection.node_ids.iter().copied().collect()
    } else {
        args.node_ids
            .iter()
            .filter_map(|s| {
                uuid::Uuid::parse_str(s)
                    .ok()
                    .or_else(|| doc.find_node_by_name(s).map(|n| n.id))
            })
            .collect()
    };

    if node_ids.is_empty() {
        return ToolResult::error("No nodes specified and nothing selected");
    }

    // Compute combined bbox.
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for nid in &node_ids {
        if let Some(node) = doc.nodes.get(nid) {
            if let SceneNodeKind::Path(pn) = &node.kind {
                let bb = pn.path_data.to_bez_path().bounding_box();
                let tx = node.transform.matrix[4];
                let ty = node.transform.matrix[5];
                min_x = min_x.min(bb.x0 + tx);
                min_y = min_y.min(bb.y0 + ty);
                max_x = max_x.max(bb.x1 + tx);
                max_y = max_y.max(bb.y1 + ty);
            } else {
                let tx = node.transform.matrix[4];
                let ty = node.transform.matrix[5];
                min_x = min_x.min(tx);
                min_y = min_y.min(ty);
                max_x = max_x.max(tx);
                max_y = max_y.max(ty);
            }
        }
    }

    if min_x >= max_x && min_y >= max_y {
        return ToolResult::error("No measurable artwork");
    }

    let art_cx = (min_x + max_x) / 2.0;
    let art_cy = (min_y + max_y) / 2.0;
    let dx = if args.horizontal {
        canvas_cx - art_cx
    } else {
        0.0
    };
    let dy = if args.vertical {
        canvas_cy - art_cy
    } else {
        0.0
    };

    let mut modified = 0usize;
    for nid in &node_ids {
        if let Some(node) = doc.nodes.get(nid) {
            let mut new_node = node.clone();
            new_node.transform.matrix[4] += dx;
            new_node.transform.matrix[5] += dy;
            history.execute(
                Command::UpdateNode {
                    old: node.clone(),
                    new: new_node,
                },
                &mut doc,
            );
            modified += 1;
        }
    }

    ToolResult::text(format!(
        "Centered {modified} node(s) on canvas (dx={dx:.1}, dy={dy:.1})"
    ))
    .with_data(serde_json::json!({ "modified": modified, "dx": dx, "dy": dy }))
}

pub async fn remove_fill(state: &AppState, args: RemoveStyleArgs) -> ToolResult {
    tracing::debug!("tool: remove_fill");

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let node_ids: Vec<NodeId> = if args.node_ids.is_empty() {
        doc.selection.node_ids.iter().copied().collect()
    } else {
        args.node_ids
            .iter()
            .filter_map(|s| {
                uuid::Uuid::parse_str(s)
                    .ok()
                    .or_else(|| doc.find_node_by_name(s).map(|n| n.id))
            })
            .collect()
    };

    let mut modified = 0usize;
    for nid in &node_ids {
        let node = match doc.nodes.get(nid) {
            Some(n) => n.clone(),
            None => continue,
        };
        let mut new_node = node.clone();
        match &mut new_node.kind {
            SceneNodeKind::Path(pn) => {
                pn.fill = photonic_core::style::Fill::none();
            }
            SceneNodeKind::Text(tn) => {
                tn.fill = photonic_core::style::Fill::none();
            }
            _ => continue,
        }
        history.execute(
            Command::UpdateNode {
                old: node,
                new: new_node,
            },
            &mut doc,
        );
        modified += 1;
    }

    ToolResult::text(format!("Removed fill from {modified} node(s)"))
        .with_data(serde_json::json!({ "modified": modified }))
}

pub async fn remove_stroke(state: &AppState, args: RemoveStyleArgs) -> ToolResult {
    tracing::debug!("tool: remove_stroke");

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let node_ids: Vec<NodeId> = if args.node_ids.is_empty() {
        doc.selection.node_ids.iter().copied().collect()
    } else {
        args.node_ids
            .iter()
            .filter_map(|s| {
                uuid::Uuid::parse_str(s)
                    .ok()
                    .or_else(|| doc.find_node_by_name(s).map(|n| n.id))
            })
            .collect()
    };

    let mut modified = 0usize;
    for nid in &node_ids {
        let node = match doc.nodes.get(nid) {
            Some(n) => n.clone(),
            None => continue,
        };
        let mut new_node = node.clone();
        match &mut new_node.kind {
            SceneNodeKind::Path(pn) => {
                pn.stroke = photonic_core::style::Stroke::none();
            }
            SceneNodeKind::Text(tn) => {
                tn.stroke = photonic_core::style::Stroke::none();
            }
            _ => continue,
        }
        history.execute(
            Command::UpdateNode {
                old: node,
                new: new_node,
            },
            &mut doc,
        );
        modified += 1;
    }

    ToolResult::text(format!("Removed stroke from {modified} node(s)"))
        .with_data(serde_json::json!({ "modified": modified }))
}

pub async fn fit_to_canvas(state: &AppState, args: FitToCanvasArgs) -> ToolResult {
    tracing::debug!("tool: fit_to_canvas");
    use kurbo::Shape;

    let padding = args.padding.unwrap_or(10.0);

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let canvas_w = doc.width;
    let canvas_h = doc.height;

    // Gather target nodes.
    let target_ids: Vec<NodeId> = if args.node_ids.is_empty() {
        doc.nodes.keys().copied().collect()
    } else {
        args.node_ids
            .iter()
            .filter_map(|s| {
                uuid::Uuid::parse_str(s)
                    .ok()
                    .or_else(|| doc.find_node_by_name(s).map(|n| n.id))
            })
            .collect()
    };

    if target_ids.is_empty() {
        return ToolResult::error("No nodes to fit");
    }

    // Compute combined bounding box of all target paths.
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;

    for nid in &target_ids {
        if let Some(node) = doc.nodes.get(nid) {
            if let SceneNodeKind::Path(pn) = &node.kind {
                let bez = pn.path_data.to_bez_path();
                let bb = bez.bounding_box();
                let tx = node.transform.matrix[4];
                let ty = node.transform.matrix[5];
                min_x = min_x.min(bb.x0 + tx);
                min_y = min_y.min(bb.y0 + ty);
                max_x = max_x.max(bb.x1 + tx);
                max_y = max_y.max(bb.y1 + ty);
            }
        }
    }

    if min_x >= max_x || min_y >= max_y {
        return ToolResult::error("No measurable artwork found");
    }

    let art_w = max_x - min_x;
    let art_h = max_y - min_y;
    let art_cx = (min_x + max_x) / 2.0;
    let art_cy = (min_y + max_y) / 2.0;

    let target_w = canvas_w - 2.0 * padding;
    let target_h = canvas_h - 2.0 * padding;
    if target_w <= 0.0 || target_h <= 0.0 {
        return ToolResult::error("Canvas too small for the specified padding");
    }

    let scale = (target_w / art_w).min(target_h / art_h).min(1.0); // Don't scale up
    let canvas_cx = canvas_w / 2.0;
    let canvas_cy = canvas_h / 2.0;

    // Apply uniform scale + translate to center.
    let mut modified = 0usize;
    for nid in &target_ids {
        if let Some(node) = doc.nodes.get(nid) {
            if let SceneNodeKind::Path(pn) = &node.kind {
                let bez = pn.path_data.to_bez_path();
                let mut new_bez = kurbo::BezPath::new();

                for el in bez.elements() {
                    let xform = |p: kurbo::Point| -> kurbo::Point {
                        let nx = (p.x + node.transform.matrix[4] - art_cx) * scale + canvas_cx;
                        let ny = (p.y + node.transform.matrix[5] - art_cy) * scale + canvas_cy;
                        kurbo::Point::new(nx, ny)
                    };
                    match *el {
                        kurbo::PathEl::MoveTo(p) => new_bez.move_to(xform(p)),
                        kurbo::PathEl::LineTo(p) => new_bez.line_to(xform(p)),
                        kurbo::PathEl::CurveTo(c1, c2, p) => {
                            new_bez.curve_to(xform(c1), xform(c2), xform(p))
                        }
                        kurbo::PathEl::QuadTo(c, p) => new_bez.quad_to(xform(c), xform(p)),
                        kurbo::PathEl::ClosePath => new_bez.close_path(),
                    }
                }

                let mut new_node = node.clone();
                if let SceneNodeKind::Path(ref mut np) = new_node.kind {
                    np.path_data = PathData::from_bez_path(&new_bez);
                }
                new_node.transform = Transform::default();
                history.execute(
                    Command::UpdateNode {
                        old: node.clone(),
                        new: new_node,
                    },
                    &mut doc,
                );
                modified += 1;
            }
        }
    }

    ToolResult::text(format!(
        "Fit {modified} node(s) to canvas (scale={scale:.2})"
    ))
    .with_data(serde_json::json!({ "modified": modified, "scale": scale }))
}

pub async fn create_scatter_plot(state: &AppState, args: CreateScatterPlotArgs) -> ToolResult {
    tracing::debug!("tool: create_scatter_plot");
    use kurbo::Shape;
    use photonic_core::color::Color;
    use photonic_core::style::{Fill, FillKind, Stroke};

    if args.points.is_empty() {
        return ToolResult::error("points must not be empty");
    }

    let plot_w = args.width.unwrap_or(300.0);
    let plot_h = args.height.unwrap_or(300.0);
    let dot_r = args.dot_radius.unwrap_or(4.0);
    let color = args.color.as_deref().unwrap_or("#4E79A7");
    let dot_color = Color::from_hex(color).unwrap_or(Color::new(0.3, 0.47, 0.65, 1.0));

    // Find data bounds.
    let mut min_x = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for &[px, py] in &args.points {
        min_x = min_x.min(px);
        max_x = max_x.max(px);
        min_y = min_y.min(py);
        max_y = max_y.max(py);
    }
    let range_x = (max_x - min_x).max(1e-9);
    let range_y = (max_y - min_y).max(1e-9);

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let layer_id = args
        .layer_id
        .and_then(|s| uuid::Uuid::parse_str(&s).ok())
        .or(doc.active_layer_id)
        .unwrap_or(uuid::Uuid::nil());

    // Build all dots as a single compound path.
    let mut bez = kurbo::BezPath::new();
    for &[px, py] in &args.points {
        let cx = args.x + ((px - min_x) / range_x) * plot_w;
        let cy = args.y - ((py - min_y) / range_y) * plot_h;
        let circle = kurbo::Circle::new((cx, cy), dot_r);
        for el in circle.to_path(0.1).elements() {
            bez.push(*el);
        }
    }

    let mut pn = PathNode::new(PathData::from_bez_path(&bez));
    pn.fill = Fill {
        kind: FillKind::Solid(dot_color),
        ..Default::default()
    };
    pn.stroke = Stroke::none();

    let node = SceneNode::new("Scatter Plot", layer_id, SceneNodeKind::Path(pn));
    let node_id = node.id;
    history.execute(
        Command::AddNode {
            node,
            layer_id: Some(layer_id),
        },
        &mut doc,
    );

    ToolResult::text(format!(
        "Created scatter plot at ({},{}) — {} points",
        args.x,
        args.y,
        args.points.len()
    ))
    .with_data(serde_json::json!({ "node_id": node_id, "point_count": args.points.len() }))
}

pub async fn scatter_copies(state: &AppState, args: ScatterCopiesArgs) -> ToolResult {
    tracing::debug!("tool: scatter_copies");

    let count = args.count.unwrap_or(20).max(1);
    let rot_range = args.rotation_range.unwrap_or(0.0).abs();
    let scale_range = args.scale_range.unwrap_or(0.0).abs();
    let seed = args.seed.unwrap_or(42).max(1);

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let src_nid = match uuid::Uuid::parse_str(&args.node_id) {
        Ok(id) => id,
        Err(_) => match doc.find_node_by_name(&args.node_id) {
            Some(n) => n.id,
            None => return ToolResult::error(format!("Node not found: {}", args.node_id)),
        },
    };
    let source = match doc.nodes.get(&src_nid) {
        Some(n) => n.clone(),
        None => return ToolResult::error("Source node not found"),
    };

    let layer_id = source.layer_id;
    let mut rng = seed;
    let mut created_ids = Vec::new();

    for i in 0..count {
        let rx = (xorshift64(&mut rng) * 0.5 + 0.5) * args.width + args.x;
        let ry = (xorshift64(&mut rng) * 0.5 + 0.5) * args.height + args.y;
        let rot = if rot_range > 0.0 {
            xorshift64(&mut rng) * rot_range
        } else {
            0.0
        };
        let rot_rad = rot.to_radians();
        let s = if scale_range > 0.0 {
            1.0 + xorshift64(&mut rng) * scale_range
        } else {
            1.0
        };

        let cos_r = rot_rad.cos();
        let sin_r = rot_rad.sin();

        let mut new_node = source.clone();
        new_node.id = uuid::Uuid::new_v4();
        new_node.name = format!("{} #{}", source.name, i + 1);
        new_node.transform = Transform {
            matrix: [s * cos_r, s * sin_r, -s * sin_r, s * cos_r, rx, ry],
        };

        let nid = new_node.id;
        created_ids.push(nid);
        history.execute(
            Command::AddNode {
                node: new_node,
                layer_id: Some(layer_id),
            },
            &mut doc,
        );
    }

    ToolResult::text(format!(
        "Scattered {} copies of '{}' in area ({},{}) {}×{}",
        count, source.name, args.x, args.y, args.width, args.height
    ))
    .with_data(serde_json::json!({ "count": count, "created_ids": created_ids }))
}

pub async fn create_line_chart(state: &AppState, args: CreateLineChartArgs) -> ToolResult {
    tracing::debug!("tool: create_line_chart");
    use photonic_core::color::Color;
    use photonic_core::style::{Fill, FillKind, Stroke};

    if args.series.is_empty() || args.series.iter().all(|s| s.is_empty()) {
        return ToolResult::error("At least one non-empty data series required");
    }

    let chart_w = args.width.unwrap_or(300.0);
    let chart_h = args.height.unwrap_or(200.0);
    let stroke_w = args.stroke_width.unwrap_or(2.0);
    let x = args.x;
    let y = args.y;

    // Find global min/max across all series.
    let mut all_max = f64::NEG_INFINITY;
    let mut all_min = f64::INFINITY;
    let mut max_len = 0usize;
    for series in &args.series {
        for &v in series {
            all_max = all_max.max(v);
            all_min = all_min.min(v);
        }
        max_len = max_len.max(series.len());
    }
    if max_len < 2 {
        return ToolResult::error("Each series needs at least 2 data points");
    }
    let range = (all_max - all_min).max(1e-9);

    let default_colors = ["#4E79A7", "#F28E2B", "#E15759", "#76B7B2", "#59A14F"];
    let colors: Vec<Color> = if args.colors.is_empty() {
        default_colors
            .iter()
            .filter_map(|h| Color::from_hex(h))
            .collect()
    } else {
        args.colors
            .iter()
            .filter_map(|h| Color::from_hex(h))
            .collect()
    };

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let layer_id = args
        .layer_id
        .and_then(|s| uuid::Uuid::parse_str(&s).ok())
        .or(doc.active_layer_id)
        .unwrap_or(uuid::Uuid::nil());

    let mut child_ids = Vec::new();

    for (si, series) in args.series.iter().enumerate() {
        if series.len() < 2 {
            continue;
        }
        let color = colors[si % colors.len()];

        // Convert data points to canvas coordinates.
        let pts: Vec<kurbo::Point> = series
            .iter()
            .enumerate()
            .map(|(i, &v)| {
                let px = x + (i as f64 / (series.len() - 1) as f64) * chart_w;
                let py = y - ((v - all_min) / range) * chart_h;
                kurbo::Point::new(px, py)
            })
            .collect();

        let line_path = if args.smooth && pts.len() >= 3 {
            // Catmull-Rom smooth.
            catmull_rom_to_bezier(&pts, false)
        } else {
            let mut bez = kurbo::BezPath::new();
            for (i, &p) in pts.iter().enumerate() {
                if i == 0 {
                    bez.move_to(p);
                } else {
                    bez.line_to(p);
                }
            }
            bez
        };

        if args.fill_area {
            // Create filled area: line path + close to baseline.
            let mut area = line_path.clone();
            area.line_to((pts.last().unwrap().x, y));
            area.line_to((pts[0].x, y));
            area.close_path();

            let mut pn = PathNode::new(PathData::from_bez_path(&area));
            pn.fill = Fill {
                kind: FillKind::Solid(Color::new(color.r, color.g, color.b, 0.2)),
                ..Default::default()
            };
            pn.stroke = Stroke::none();
            let node = SceneNode::new(
                &format!("Series {} Area", si + 1),
                layer_id,
                SceneNodeKind::Path(pn),
            );
            child_ids.push(node.id);
            history.execute(
                Command::AddNode {
                    node,
                    layer_id: Some(layer_id),
                },
                &mut doc,
            );
        }

        // Stroke line.
        let mut pn = PathNode::new(PathData::from_bez_path(&line_path));
        pn.fill = Fill::none();
        pn.stroke = Stroke {
            color,
            width: stroke_w,
            enabled: true,
            ..Default::default()
        };
        let node = SceneNode::new(
            &format!("Series {}", si + 1),
            layer_id,
            SceneNodeKind::Path(pn),
        );
        child_ids.push(node.id);
        history.execute(
            Command::AddNode {
                node,
                layer_id: Some(layer_id),
            },
            &mut doc,
        );
    }

    let group = SceneNode::new(
        "Line Chart",
        layer_id,
        SceneNodeKind::Group(photonic_core::node::GroupNode::new()),
    );
    let group_id = group.id;
    history.execute(
        Command::GroupNodes {
            group,
            layer_id,
            insert_index: 0,
            children: child_ids.clone(),
        },
        &mut doc,
    );

    ToolResult::text(format!(
        "Created line chart at ({x},{y}) — {} series, {} max points",
        args.series.len(),
        max_len
    ))
    .with_data(serde_json::json!({ "group_id": group_id, "series_count": args.series.len() }))
}

pub async fn create_bar_chart(state: &AppState, args: CreateBarChartArgs) -> ToolResult {
    tracing::debug!("tool: create_bar_chart");
    use kurbo::Shape;
    use photonic_core::color::Color;
    use photonic_core::style::{Fill, FillKind, Stroke};

    if args.values.is_empty() {
        return ToolResult::error("values must not be empty");
    }

    let chart_w = args.width.unwrap_or(300.0);
    let chart_h = args.height.unwrap_or(200.0);
    let gap_frac = args.gap.unwrap_or(0.2).clamp(0.0, 0.9);
    let n = args.values.len();
    let max_val = args
        .values
        .iter()
        .cloned()
        .fold(f64::NEG_INFINITY, f64::max);
    if max_val <= 0.0 {
        return ToolResult::error("At least one value must be positive");
    }

    let default_colors = [
        "#4E79A7", "#F28E2B", "#E15759", "#76B7B2", "#59A14F", "#EDC948", "#B07AA1", "#FF9DA7",
        "#9C755F", "#BAB0AC",
    ];
    let colors: Vec<Color> = if args.colors.is_empty() {
        default_colors
            .iter()
            .filter_map(|h| Color::from_hex(h))
            .collect()
    } else {
        args.colors
            .iter()
            .filter_map(|h| Color::from_hex(h))
            .collect()
    };

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let layer_id = args
        .layer_id
        .and_then(|s| uuid::Uuid::parse_str(&s).ok())
        .or(doc.active_layer_id)
        .unwrap_or(uuid::Uuid::nil());

    let mut child_ids = Vec::new();

    if args.horizontal {
        let bar_total = chart_h / n as f64;
        let bar_h = bar_total * (1.0 - gap_frac);
        let bar_gap = bar_total * gap_frac;

        for (i, &val) in args.values.iter().enumerate() {
            let bar_w = (val / max_val) * chart_w;
            let bx = args.x;
            let by = args.y - chart_h + (i as f64 * bar_total) + bar_gap / 2.0;

            let rect = kurbo::Rect::new(bx, by, bx + bar_w, by + bar_h);
            let mut pn = PathNode::new(PathData::from_bez_path(&rect.to_path(0.0)));
            pn.fill = Fill {
                kind: FillKind::Solid(colors[i % colors.len()]),
                ..Default::default()
            };
            pn.stroke = Stroke::none();

            let label = args
                .labels
                .get(i)
                .cloned()
                .unwrap_or_else(|| format!("Bar {}", i + 1));
            let node = SceneNode::new(&label, layer_id, SceneNodeKind::Path(pn));
            child_ids.push(node.id);
            history.execute(
                Command::AddNode {
                    node,
                    layer_id: Some(layer_id),
                },
                &mut doc,
            );
        }
    } else {
        let bar_total = chart_w / n as f64;
        let bar_w = bar_total * (1.0 - gap_frac);
        let bar_gap = bar_total * gap_frac;

        for (i, &val) in args.values.iter().enumerate() {
            let bar_h = (val / max_val) * chart_h;
            let bx = args.x + (i as f64 * bar_total) + bar_gap / 2.0;
            let by = args.y - bar_h;

            let rect = kurbo::Rect::new(bx, by, bx + bar_w, args.y);
            let mut pn = PathNode::new(PathData::from_bez_path(&rect.to_path(0.0)));
            pn.fill = Fill {
                kind: FillKind::Solid(colors[i % colors.len()]),
                ..Default::default()
            };
            pn.stroke = Stroke::none();

            let label = args
                .labels
                .get(i)
                .cloned()
                .unwrap_or_else(|| format!("Bar {}", i + 1));
            let node = SceneNode::new(&label, layer_id, SceneNodeKind::Path(pn));
            child_ids.push(node.id);
            history.execute(
                Command::AddNode {
                    node,
                    layer_id: Some(layer_id),
                },
                &mut doc,
            );
        }
    }

    let group = SceneNode::new(
        "Bar Chart",
        layer_id,
        SceneNodeKind::Group(photonic_core::node::GroupNode::new()),
    );
    let group_id = group.id;
    history.execute(
        Command::GroupNodes {
            group,
            layer_id,
            insert_index: 0,
            children: child_ids.clone(),
        },
        &mut doc,
    );

    ToolResult::text(format!(
        "Created {} bar chart at ({},{}) — {} bars",
        if args.horizontal {
            "horizontal"
        } else {
            "vertical"
        },
        args.x,
        args.y,
        n
    ))
    .with_data(serde_json::json!({ "group_id": group_id, "bars": n }))
}

pub async fn create_stacked_bar_chart(
    state: &AppState,
    args: CreateStackedBarChartArgs,
) -> ToolResult {
    tracing::debug!("tool: create_stacked_bar_chart");
    use kurbo::Shape;
    use photonic_core::color::Color;
    use photonic_core::style::{Fill, FillKind, Stroke};

    if args.series.is_empty() {
        return ToolResult::error("series must not be empty");
    }
    let n_stacks = args.series[0].len();
    if n_stacks == 0 {
        return ToolResult::error("each series must have at least one value");
    }
    for (i, s) in args.series.iter().enumerate() {
        if s.len() != n_stacks {
            return ToolResult::error(format!(
                "all series must have the same length; series 0 has {} values but series {} has {}",
                n_stacks,
                i,
                s.len()
            ));
        }
    }

    let chart_w = args.width.unwrap_or(300.0);
    let chart_h = args.height.unwrap_or(200.0);
    let gap_frac = args.gap.unwrap_or(0.2).clamp(0.0, 0.9);

    // Max stack total for normalization.
    let max_total = (0..n_stacks)
        .map(|ci| args.series.iter().map(|s| s[ci]).sum::<f64>())
        .fold(0.0_f64, f64::max);
    if max_total <= 0.0 {
        return ToolResult::error("at least one value must be positive");
    }

    let default_colors = [
        "#4E79A7", "#F28E2B", "#E15759", "#76B7B2", "#59A14F", "#EDC948", "#B07AA1", "#FF9DA7",
        "#9C755F", "#BAB0AC",
    ];
    let parsed_user: Vec<Color> = args
        .colors
        .iter()
        .filter_map(|h| Color::from_hex(h))
        .collect();
    let colors: Vec<Color> = if parsed_user.is_empty() {
        default_colors
            .iter()
            .filter_map(|h| Color::from_hex(h))
            .collect()
    } else {
        parsed_user
    };

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let layer_id = args
        .layer_id
        .and_then(|s| uuid::Uuid::parse_str(&s).ok())
        .or(doc.active_layer_id)
        .unwrap_or(uuid::Uuid::nil());

    let mut child_ids = Vec::new();

    if args.horizontal {
        let bar_total = chart_h / n_stacks as f64;
        let bar_h = bar_total * (1.0 - gap_frac);
        let bar_gap = bar_total * gap_frac;

        for ci in 0..n_stacks {
            let by = args.y - chart_h + (ci as f64 * bar_total) + bar_gap / 2.0;
            let mut cursor_x = args.x;
            for (si, series) in args.series.iter().enumerate() {
                let val = series[ci];
                if val <= 0.0 {
                    cursor_x += 0.0;
                    continue;
                }
                let seg_w = (val / max_total) * chart_w;
                let rect = kurbo::Rect::new(cursor_x, by, cursor_x + seg_w, by + bar_h);
                let mut pn = PathNode::new(PathData::from_bez_path(&rect.to_path(0.0)));
                pn.fill = Fill {
                    kind: FillKind::Solid(colors[si % colors.len()]),
                    ..Default::default()
                };
                pn.stroke = Stroke::none();
                let sname = args
                    .series_names
                    .get(si)
                    .cloned()
                    .unwrap_or_else(|| format!("Series {}", si + 1));
                let lname = args
                    .labels
                    .get(ci)
                    .cloned()
                    .unwrap_or_else(|| format!("Bar {}", ci + 1));
                let node = SceneNode::new(
                    format!("{sname} / {lname}"),
                    layer_id,
                    SceneNodeKind::Path(pn),
                );
                child_ids.push(node.id);
                history.execute(
                    Command::AddNode {
                        node,
                        layer_id: Some(layer_id),
                    },
                    &mut doc,
                );
                cursor_x += seg_w;
            }
        }
    } else {
        let bar_total = chart_w / n_stacks as f64;
        let bar_w = bar_total * (1.0 - gap_frac);
        let bar_gap = bar_total * gap_frac;

        for ci in 0..n_stacks {
            let bx = args.x + (ci as f64 * bar_total) + bar_gap / 2.0;
            let mut cursor_y = args.y; // top of stack grows upward
            for (si, series) in args.series.iter().enumerate() {
                let val = series[ci];
                if val <= 0.0 {
                    continue;
                }
                let seg_h = (val / max_total) * chart_h;
                let rect = kurbo::Rect::new(bx, cursor_y - seg_h, bx + bar_w, cursor_y);
                let mut pn = PathNode::new(PathData::from_bez_path(&rect.to_path(0.0)));
                pn.fill = Fill {
                    kind: FillKind::Solid(colors[si % colors.len()]),
                    ..Default::default()
                };
                pn.stroke = Stroke::none();
                let sname = args
                    .series_names
                    .get(si)
                    .cloned()
                    .unwrap_or_else(|| format!("Series {}", si + 1));
                let lname = args
                    .labels
                    .get(ci)
                    .cloned()
                    .unwrap_or_else(|| format!("Bar {}", ci + 1));
                let node = SceneNode::new(
                    format!("{sname} / {lname}"),
                    layer_id,
                    SceneNodeKind::Path(pn),
                );
                child_ids.push(node.id);
                history.execute(
                    Command::AddNode {
                        node,
                        layer_id: Some(layer_id),
                    },
                    &mut doc,
                );
                cursor_y -= seg_h;
            }
        }
    }

    let label = format!(
        "Stacked {} Chart",
        if args.horizontal { "Bar" } else { "Column" }
    );
    let group = SceneNode::new(
        &label,
        layer_id,
        SceneNodeKind::Group(photonic_core::node::GroupNode::new()),
    );
    let group_id = group.id;
    history.execute(
        Command::GroupNodes {
            group,
            layer_id,
            insert_index: 0,
            children: child_ids.clone(),
        },
        &mut doc,
    );

    ToolResult::text(format!(
        "Created stacked {} chart at ({},{}) — {} stacks, {} series",
        if args.horizontal { "bar" } else { "column" },
        args.x,
        args.y,
        n_stacks,
        args.series.len()
    ))
    .with_data(serde_json::json!({
        "group_id": group_id,
        "stacks": n_stacks,
        "series": args.series.len(),
    }))
}

pub async fn create_pie_chart(state: &AppState, args: CreatePieChartArgs) -> ToolResult {
    tracing::debug!("tool: create_pie_chart");
    use photonic_core::color::Color;
    use photonic_core::style::{Fill, FillKind, Stroke};

    if args.values.is_empty() {
        return ToolResult::error("values must not be empty");
    }

    let total: f64 = args.values.iter().sum();
    if total <= 0.0 {
        return ToolResult::error("Sum of values must be positive");
    }

    let radius = args.radius.unwrap_or(80.0);
    let inner_r = args.inner_radius.unwrap_or(0.0).max(0.0);
    let cx = args.cx;
    let cy = args.cy;

    // Default palette if none provided.
    let default_colors = [
        "#4E79A7", "#F28E2B", "#E15759", "#76B7B2", "#59A14F", "#EDC948", "#B07AA1", "#FF9DA7",
        "#9C755F", "#BAB0AC",
    ];

    let colors: Vec<Color> = if args.colors.is_empty() {
        default_colors
            .iter()
            .filter_map(|h| Color::from_hex(h))
            .collect()
    } else {
        args.colors
            .iter()
            .filter_map(|h| Color::from_hex(h))
            .collect()
    };

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let layer_id = args
        .layer_id
        .and_then(|s| uuid::Uuid::parse_str(&s).ok())
        .or(doc.active_layer_id)
        .unwrap_or(uuid::Uuid::nil());

    let mut child_ids = Vec::new();
    let mut start_angle = -std::f64::consts::FRAC_PI_2; // Start from top (12 o'clock).
    let n_segs = 32;

    for (i, &val) in args.values.iter().enumerate() {
        let sweep = (val / total) * std::f64::consts::TAU;
        let end_angle = start_angle + sweep;
        let color = colors[i % colors.len()];

        let mut bez = kurbo::BezPath::new();

        if inner_r > 0.0 {
            // Donut slice: outer arc → line to inner → inner arc reversed → close.
            for j in 0..=n_segs {
                let t = j as f64 / n_segs as f64;
                let a = start_angle + sweep * t;
                let pt = kurbo::Point::new(cx + radius * a.cos(), cy + radius * a.sin());
                if j == 0 {
                    bez.move_to(pt);
                } else {
                    bez.line_to(pt);
                }
            }
            for j in (0..=n_segs).rev() {
                let t = j as f64 / n_segs as f64;
                let a = start_angle + sweep * t;
                let pt = kurbo::Point::new(cx + inner_r * a.cos(), cy + inner_r * a.sin());
                bez.line_to(pt);
            }
        } else {
            // Solid pie: center → arc → close.
            bez.move_to((cx, cy));
            for j in 0..=n_segs {
                let t = j as f64 / n_segs as f64;
                let a = start_angle + sweep * t;
                bez.line_to((cx + radius * a.cos(), cy + radius * a.sin()));
            }
        }
        bez.close_path();

        let mut pn = PathNode::new(PathData::from_bez_path(&bez));
        pn.fill = Fill {
            kind: FillKind::Solid(color),
            ..Default::default()
        };
        pn.stroke = Stroke {
            color: Color::WHITE,
            width: 1.0,
            enabled: true,
            ..Default::default()
        };

        let label = args
            .labels
            .get(i)
            .cloned()
            .unwrap_or_else(|| format!("Slice {}", i + 1));
        let node = SceneNode::new(&label, layer_id, SceneNodeKind::Path(pn));
        let nid = node.id;
        child_ids.push(nid);
        history.execute(
            Command::AddNode {
                node,
                layer_id: Some(layer_id),
            },
            &mut doc,
        );

        start_angle = end_angle;
    }

    // Group all slices.
    let group = SceneNode::new(
        "Pie Chart",
        layer_id,
        SceneNodeKind::Group(photonic_core::node::GroupNode::new()),
    );
    let group_id = group.id;
    history.execute(
        Command::GroupNodes {
            group,
            layer_id,
            insert_index: 0,
            children: child_ids.clone(),
        },
        &mut doc,
    );

    ToolResult::text(format!(
        "Created pie chart at ({cx},{cy}) — {} slices, r={radius}",
        args.values.len()
    ))
    .with_data(serde_json::json!({
        "group_id": group_id,
        "slices": args.values.len(),
    }))
}

pub async fn create_radar_chart(state: &AppState, args: CreateRadarChartArgs) -> ToolResult {
    tracing::debug!("tool: create_radar_chart");
    use photonic_core::color::Color;
    use photonic_core::style::{Fill, FillKind, Stroke};

    if args.series.is_empty() {
        return ToolResult::error("series must not be empty");
    }
    let n_axes = args.series[0].len();
    if n_axes < 3 {
        return ToolResult::error("each series must have at least 3 values (axes)");
    }
    for (i, s) in args.series.iter().enumerate() {
        if s.len() != n_axes {
            return ToolResult::error(format!(
                "all series must have the same length; series 0 has {} values but series {} has {}",
                n_axes,
                i,
                s.len()
            ));
        }
    }

    let radius = args.radius.unwrap_or(100.0);
    let grid_rings = args.grid_rings.unwrap_or(4).max(1);
    let stroke_w = args.stroke_width.unwrap_or(1.5);
    let cx = args.cx;
    let cy = args.cy;

    let default_colors = [
        "#4E79A7", "#F28E2B", "#E15759", "#76B7B2", "#59A14F", "#EDC948", "#B07AA1", "#FF9DA7",
        "#9C755F", "#BAB0AC",
    ];
    let parsed_user: Vec<Color> = args
        .colors
        .iter()
        .filter_map(|h| Color::from_hex(h))
        .collect();
    let colors: Vec<Color> = if parsed_user.is_empty() {
        default_colors
            .iter()
            .filter_map(|h| Color::from_hex(h))
            .collect()
    } else {
        parsed_user
    };

    // Compute axis angles: evenly distributed, starting at top (−π/2).
    let axis_angle = |i: usize| -> f64 {
        -std::f64::consts::FRAC_PI_2 + (i as f64 / n_axes as f64) * std::f64::consts::TAU
    };

    // Max value across all series per axis for normalization.
    let axis_max: Vec<f64> = (0..n_axes)
        .map(|ai| args.series.iter().map(|s| s[ai]).fold(0.0_f64, f64::max))
        .collect();

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let layer_id = args
        .layer_id
        .and_then(|s| uuid::Uuid::parse_str(&s).ok())
        .or(doc.active_layer_id)
        .unwrap_or(uuid::Uuid::nil());

    let mut child_ids: Vec<uuid::Uuid> = Vec::new();

    // ── Grid rings ──────────────────────────────────────────────────────────
    for ring in 1..=grid_rings {
        let r = radius * (ring as f64 / grid_rings as f64);
        let mut bez = kurbo::BezPath::new();
        for i in 0..n_axes {
            let angle = axis_angle(i);
            let pt = kurbo::Point::new(cx + r * angle.cos(), cy + r * angle.sin());
            if i == 0 {
                bez.move_to(pt);
            } else {
                bez.line_to(pt);
            }
        }
        bez.close_path();
        let mut pn = PathNode::new(PathData::from_bez_path(&bez));
        pn.fill = Fill {
            kind: FillKind::None,
            ..Default::default()
        };
        pn.stroke = Stroke::solid(Color::new(0.7, 0.7, 0.75, 1.0), 0.75);
        let node = SceneNode::new(
            &format!("Grid Ring {ring}"),
            layer_id,
            SceneNodeKind::Path(pn),
        );
        child_ids.push(node.id);
        history.execute(
            Command::AddNode {
                node,
                layer_id: Some(layer_id),
            },
            &mut doc,
        );
    }

    // ── Axis lines ──────────────────────────────────────────────────────────
    for i in 0..n_axes {
        let angle = axis_angle(i);
        let tip = kurbo::Point::new(cx + radius * angle.cos(), cy + radius * angle.sin());
        let mut bez = kurbo::BezPath::new();
        bez.move_to(kurbo::Point::new(cx, cy));
        bez.line_to(tip);
        let mut pn = PathNode::new(PathData::from_bez_path(&bez));
        pn.fill = Fill {
            kind: FillKind::None,
            ..Default::default()
        };
        pn.stroke = Stroke::solid(Color::new(0.7, 0.7, 0.75, 1.0), 0.75);
        let label = args
            .labels
            .get(i)
            .cloned()
            .unwrap_or_else(|| format!("Axis {}", i + 1));
        let node = SceneNode::new(&format!("Axis {label}"), layer_id, SceneNodeKind::Path(pn));
        child_ids.push(node.id);
        history.execute(
            Command::AddNode {
                node,
                layer_id: Some(layer_id),
            },
            &mut doc,
        );
    }

    // ── Data series polygons ────────────────────────────────────────────────
    for (si, series) in args.series.iter().enumerate() {
        let color = colors[si % colors.len()];
        let mut bez = kurbo::BezPath::new();
        for (ai, &val) in series.iter().enumerate() {
            let max = if axis_max[ai] > 0.0 {
                axis_max[ai]
            } else {
                1.0
            };
            let r = radius * (val / max).clamp(0.0, 1.0);
            let angle = axis_angle(ai);
            let pt = kurbo::Point::new(cx + r * angle.cos(), cy + r * angle.sin());
            if ai == 0 {
                bez.move_to(pt);
            } else {
                bez.line_to(pt);
            }
        }
        bez.close_path();
        let mut pn = PathNode::new(PathData::from_bez_path(&bez));
        if args.fill_area {
            pn.fill = Fill {
                kind: FillKind::Solid(Color::new(color.r, color.g, color.b, 0.2)),
                ..Default::default()
            };
        } else {
            pn.fill = Fill {
                kind: FillKind::None,
                ..Default::default()
            };
        }
        pn.stroke = Stroke::solid(color, stroke_w);
        let series_name = args
            .series_names
            .get(si)
            .cloned()
            .unwrap_or_else(|| format!("Series {}", si + 1));
        let node = SceneNode::new(&series_name, layer_id, SceneNodeKind::Path(pn));
        child_ids.push(node.id);
        history.execute(
            Command::AddNode {
                node,
                layer_id: Some(layer_id),
            },
            &mut doc,
        );
    }

    let group = SceneNode::new(
        "Radar Chart",
        layer_id,
        SceneNodeKind::Group(photonic_core::node::GroupNode::new()),
    );
    let group_id = group.id;
    history.execute(
        Command::GroupNodes {
            group,
            layer_id,
            insert_index: 0,
            children: child_ids.clone(),
        },
        &mut doc,
    );

    ToolResult::text(format!(
        "Created radar chart at ({cx},{cy}) — {} axes, {} series, r={radius}",
        n_axes,
        args.series.len()
    ))
    .with_data(serde_json::json!({
        "group_id": group_id,
        "axes": n_axes,
        "series": args.series.len(),
    }))
}

pub async fn point_on_path(state: &AppState, args: PointOnPathArgs) -> ToolResult {
    tracing::debug!("tool: point_on_path");
    use kurbo::{ParamCurve, ParamCurveArclen};

    let doc = state.document.lock().await;

    let nid = match uuid::Uuid::parse_str(&args.node_id) {
        Ok(id) => id,
        Err(_) => match doc.find_node_by_name(&args.node_id) {
            Some(n) => n.id,
            None => return ToolResult::error(format!("Node not found: {}", args.node_id)),
        },
    };
    let node = match doc.nodes.get(&nid) {
        Some(n) => n,
        None => return ToolResult::error("Node not found"),
    };
    let pn = match &node.kind {
        SceneNodeKind::Path(pn) => pn,
        _ => return ToolResult::error("Node is not a path"),
    };

    let bez = pn.path_data.to_bez_path();
    let segments: Vec<kurbo::PathSeg> = bez.segments().collect();
    if segments.is_empty() {
        return ToolResult::error("Path has no segments");
    }

    let accuracy = 0.5;
    let seg_lengths: Vec<f64> = segments.iter().map(|s| s.arclen(accuracy)).collect();
    let total_length: f64 = seg_lengths.iter().sum();
    if total_length < 1e-9 {
        return ToolResult::error("Path has zero length");
    }

    let mut results = Vec::new();

    for &t_val in &args.t {
        let t = t_val.clamp(0.0, 1.0);
        let target_len = t * total_length;

        let mut accum = 0.0;
        let mut pt = kurbo::Point::ZERO;
        let mut tangent_angle: f64 = 0.0;

        for (seg, &seg_len) in segments.iter().zip(seg_lengths.iter()) {
            if accum + seg_len >= target_len || seg_len < 1e-9 {
                let local_t = if seg_len > 1e-9 {
                    (target_len - accum) / seg_len
                } else {
                    0.5
                };
                pt = seg.eval(local_t.clamp(0.0, 1.0));
                let dt = 0.001;
                let p0 = seg.eval((local_t - dt).max(0.0));
                let p1 = seg.eval((local_t + dt).min(1.0));
                tangent_angle = (p1.y - p0.y).atan2(p1.x - p0.x);
                break;
            }
            accum += seg_len;
        }

        results.push(serde_json::json!({
            "t": t_val,
            "x": pt.x,
            "y": pt.y,
            "tangent_degrees": tangent_angle.to_degrees(),
        }));
    }

    let summary = if results.len() == 1 {
        let r = &results[0];
        format!(
            "t={}: ({:.1}, {:.1}) angle={:.1}°",
            r["t"], r["x"], r["y"], r["tangent_degrees"]
        )
    } else {
        format!("{} points sampled along path", results.len())
    };

    ToolResult::text(summary).with_data(serde_json::json!({
        "points": results,
        "total_length": total_length,
    }))
}

pub async fn create_speech_bubble(state: &AppState, args: CreateSpeechBubbleArgs) -> ToolResult {
    tracing::debug!("tool: create_speech_bubble");

    let w = args.width.unwrap_or(120.0);
    let h = args.height.unwrap_or(60.0);
    let r = args.corner_radius.unwrap_or(15.0).min(w / 2.0).min(h / 2.0);
    let tail_x = args.tail_x.unwrap_or(args.cx - 10.0);
    let tail_y = args.tail_y.unwrap_or(args.cy + h / 2.0 + 30.0);
    let tail_w = args.tail_width.unwrap_or(20.0);

    let left = args.cx - w / 2.0;
    let right = args.cx + w / 2.0;
    let top = args.cy - h / 2.0;
    let bottom = args.cy + h / 2.0;

    // Rounded rectangle with tail integrated into the bottom edge.
    // Tail connects at the bottom edge between two points.
    let tail_base_left = (args.cx - tail_w / 2.0).max(left + r);
    let tail_base_right = (args.cx + tail_w / 2.0).min(right - r);

    let mut bez = kurbo::BezPath::new();

    // Start at top-left corner after the radius.
    bez.move_to((left + r, top));
    bez.line_to((right - r, top));
    // Top-right corner.
    bez.quad_to((right, top), (right, top + r));
    bez.line_to((right, bottom - r));
    // Bottom-right corner.
    bez.quad_to((right, bottom), (right - r, bottom));
    // Bottom edge → tail.
    bez.line_to((tail_base_right, bottom));
    bez.line_to((tail_x, tail_y));
    bez.line_to((tail_base_left, bottom));
    // Continue bottom edge.
    bez.line_to((left + r, bottom));
    // Bottom-left corner.
    bez.quad_to((left, bottom), (left, bottom - r));
    bez.line_to((left, top + r));
    // Top-left corner.
    bez.quad_to((left, top), (left + r, top));
    bez.close_path();

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let layer_id = args
        .layer_id
        .and_then(|s| uuid::Uuid::parse_str(&s).ok())
        .or(doc.active_layer_id)
        .unwrap_or(uuid::Uuid::nil());

    let mut pn = PathNode::new(PathData::from_bez_path(&bez));
    if args.fill.is_none() && args.stroke.is_none() {
        pn.fill = photonic_core::style::Fill {
            kind: photonic_core::style::FillKind::Solid(photonic_core::color::Color::WHITE),
            ..Default::default()
        };
        pn.stroke = photonic_core::style::Stroke {
            color: photonic_core::color::Color::BLACK,
            width: 2.0,
            enabled: true,
            ..Default::default()
        };
    } else if let Err(e) = apply_style(&mut pn, args.fill, args.stroke) {
        return ToolResult::error(e);
    }

    let node = SceneNode::new("Speech Bubble", layer_id, SceneNodeKind::Path(pn));
    let node_id = node.id;
    history.execute(
        Command::AddNode {
            node,
            layer_id: Some(layer_id),
        },
        &mut doc,
    );

    ToolResult::text(format!(
        "Created speech bubble at ({},{}), {w}×{h}",
        args.cx, args.cy
    ))
    .with_data(serde_json::json!({ "node_id": node_id }))
}

pub async fn set_visibility(state: &AppState, args: SetVisibilityArgs) -> ToolResult {
    tracing::debug!("tool: set_visibility");

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let node_ids: Vec<NodeId> = if args.node_ids.is_empty() {
        doc.selection.node_ids.iter().copied().collect()
    } else {
        args.node_ids
            .iter()
            .filter_map(|s| {
                uuid::Uuid::parse_str(s)
                    .ok()
                    .or_else(|| doc.find_node_by_name(s).map(|n| n.id))
            })
            .collect()
    };

    if node_ids.is_empty() {
        return ToolResult::error("No nodes specified and nothing selected");
    }

    let mut modified = 0usize;
    for nid in &node_ids {
        let node = match doc.nodes.get(nid) {
            Some(n) => n.clone(),
            None => continue,
        };
        let mut new_node = node.clone();
        new_node.visible = args.visible.unwrap_or(!node.visible);
        history.execute(
            Command::UpdateNode {
                old: node,
                new: new_node,
            },
            &mut doc,
        );
        modified += 1;
    }

    let state_label = if args.visible == Some(true) {
        "visible"
    } else if args.visible == Some(false) {
        "hidden"
    } else {
        "toggled"
    };
    ToolResult::text(format!("Set {modified} node(s) to {state_label}"))
        .with_data(serde_json::json!({ "modified": modified }))
}

pub async fn set_locked(state: &AppState, args: SetLockedArgs) -> ToolResult {
    tracing::debug!("tool: set_locked");

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let node_ids: Vec<NodeId> = if args.node_ids.is_empty() {
        doc.selection.node_ids.iter().copied().collect()
    } else {
        args.node_ids
            .iter()
            .filter_map(|s| {
                uuid::Uuid::parse_str(s)
                    .ok()
                    .or_else(|| doc.find_node_by_name(s).map(|n| n.id))
            })
            .collect()
    };

    if node_ids.is_empty() {
        return ToolResult::error("No nodes specified and nothing selected");
    }

    let mut modified = 0usize;
    for nid in &node_ids {
        let node = match doc.nodes.get(nid) {
            Some(n) => n.clone(),
            None => continue,
        };
        let mut new_node = node.clone();
        new_node.locked = args.locked.unwrap_or(!node.locked);
        history.execute(
            Command::UpdateNode {
                old: node,
                new: new_node,
            },
            &mut doc,
        );
        modified += 1;
    }

    let state_label = if args.locked == Some(true) {
        "locked"
    } else if args.locked == Some(false) {
        "unlocked"
    } else {
        "toggled"
    };
    ToolResult::text(format!("Set {modified} node(s) to {state_label}"))
        .with_data(serde_json::json!({ "modified": modified }))
}

pub async fn select_all(state: &AppState, args: SelectAllArgs) -> ToolResult {
    tracing::debug!("tool: select_all");

    let mut doc = state.document.lock().await;

    let layer_filter = args.layer_id.and_then(|s| {
        uuid::Uuid::parse_str(&s)
            .ok()
            .or_else(|| doc.layers.values().find(|l| l.name == s).map(|l| l.id))
    });

    doc.selection.clear();
    let mut count = 0usize;

    let nids: Vec<NodeId> = doc.nodes.keys().copied().collect();
    for nid in nids {
        if let Some(lid) = layer_filter {
            if let Some(node) = doc.nodes.get(&nid) {
                if node.layer_id != lid {
                    continue;
                }
            }
        }
        doc.selection.add(nid);
        count += 1;
    }

    ToolResult::text(format!("Selected {count} node(s)"))
        .with_data(serde_json::json!({ "selected": count }))
}

pub async fn deselect_all(state: &AppState, _args: DeselectAllArgs) -> ToolResult {
    tracing::debug!("tool: deselect_all");

    let mut doc = state.document.lock().await;
    let prev_count = doc.selection.node_ids.len();
    doc.selection.clear();

    ToolResult::text(format!("Deselected {prev_count} node(s)"))
        .with_data(serde_json::json!({ "deselected": prev_count }))
}

pub async fn set_blend_mode(state: &AppState, args: SetBlendModeArgs) -> ToolResult {
    tracing::debug!("tool: set_blend_mode");
    use photonic_core::layer::BlendMode;

    let mode = match args.blend_mode.as_str() {
        "normal" => BlendMode::Normal,
        "multiply" => BlendMode::Multiply,
        "screen" => BlendMode::Screen,
        "overlay" => BlendMode::Overlay,
        "darken" => BlendMode::Darken,
        "lighten" => BlendMode::Lighten,
        "color_dodge" => BlendMode::ColorDodge,
        "color_burn" => BlendMode::ColorBurn,
        "hard_light" => BlendMode::HardLight,
        "soft_light" => BlendMode::SoftLight,
        "difference" => BlendMode::Difference,
        "exclusion" => BlendMode::Exclusion,
        "hue" => BlendMode::Hue,
        "saturation" => BlendMode::Saturation,
        "color" => BlendMode::Color,
        "luminosity" => BlendMode::Luminosity,
        other => return ToolResult::error(format!("Unknown blend mode: '{other}'")),
    };

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let node_ids: Vec<NodeId> = if args.node_ids.is_empty() {
        doc.selection.node_ids.iter().copied().collect()
    } else {
        args.node_ids
            .iter()
            .filter_map(|s| {
                uuid::Uuid::parse_str(s)
                    .ok()
                    .or_else(|| doc.find_node_by_name(s).map(|n| n.id))
            })
            .collect()
    };

    if node_ids.is_empty() {
        return ToolResult::error("No nodes specified and nothing selected");
    }

    let mut modified = 0usize;
    for nid in &node_ids {
        let node = match doc.nodes.get(nid) {
            Some(n) => n.clone(),
            None => continue,
        };
        let mut new_node = node.clone();
        new_node.blend_mode = mode;
        history.execute(
            Command::UpdateNode {
                old: node,
                new: new_node,
            },
            &mut doc,
        );
        modified += 1;
    }

    ToolResult::text(format!(
        "Set blend mode to '{}' on {modified} node(s)",
        args.blend_mode
    ))
    .with_data(serde_json::json!({ "modified": modified, "blend_mode": args.blend_mode }))
}

pub async fn set_opacity(state: &AppState, args: SetOpacityArgs) -> ToolResult {
    tracing::debug!("tool: set_opacity");

    let opacity = args.opacity.clamp(0.0, 1.0);

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let node_ids: Vec<NodeId> = if args.node_ids.is_empty() {
        doc.selection.node_ids.iter().copied().collect()
    } else {
        args.node_ids
            .iter()
            .filter_map(|s| {
                uuid::Uuid::parse_str(s)
                    .ok()
                    .or_else(|| doc.find_node_by_name(s).map(|n| n.id))
            })
            .collect()
    };

    if node_ids.is_empty() {
        return ToolResult::error("No nodes specified and nothing selected");
    }

    let mut modified = 0usize;
    for nid in &node_ids {
        let node = match doc.nodes.get(nid) {
            Some(n) => n.clone(),
            None => continue,
        };
        let mut new_node = node.clone();
        new_node.opacity = opacity;
        history.execute(
            Command::UpdateNode {
                old: node,
                new: new_node,
            },
            &mut doc,
        );
        modified += 1;
    }

    ToolResult::text(format!("Set opacity to {opacity} on {modified} node(s)"))
        .with_data(serde_json::json!({ "modified": modified, "opacity": opacity }))
}

pub async fn randomize_colors(state: &AppState, args: RandomizeColorsArgs) -> ToolResult {
    tracing::debug!("tool: randomize_colors");
    use photonic_core::color::Color;
    use photonic_core::style::{Fill, FillKind};

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let node_ids: Vec<NodeId> = if args.node_ids.is_empty() {
        doc.selection.node_ids.iter().copied().collect()
    } else {
        args.node_ids
            .iter()
            .filter_map(|s| {
                uuid::Uuid::parse_str(s)
                    .ok()
                    .or_else(|| doc.find_node_by_name(s).map(|n| n.id))
            })
            .collect()
    };

    if node_ids.is_empty() {
        return ToolResult::error("No nodes specified and nothing selected");
    }

    // Parse palette or generate random colors.
    let palette: Vec<Color> = if args.palette.is_empty() {
        let mut rng = args.seed.unwrap_or(42).max(1);
        (0..10)
            .map(|_| {
                let r = (xorshift64(&mut rng) * 0.5 + 0.5) as f32;
                let g = (xorshift64(&mut rng) * 0.5 + 0.5) as f32;
                let b = (xorshift64(&mut rng) * 0.5 + 0.5) as f32;
                Color::new(r, g, b, 1.0)
            })
            .collect()
    } else {
        args.palette
            .iter()
            .filter_map(|h| Color::from_hex(h))
            .collect()
    };

    if palette.is_empty() {
        return ToolResult::error("No valid colors in palette");
    }

    let mut rng = args.seed.unwrap_or(42).max(1);
    let mut modified = 0usize;

    for nid in &node_ids {
        let node = match doc.nodes.get(nid) {
            Some(n) => n.clone(),
            None => continue,
        };

        let mut new_node = node.clone();
        let mut pick = || -> Color {
            let idx = ((xorshift64(&mut rng) * 0.5 + 0.5) * palette.len() as f64) as usize
                % palette.len();
            palette[idx]
        };

        match &mut new_node.kind {
            SceneNodeKind::Path(pn) => {
                if args.fill {
                    pn.fill = Fill {
                        kind: FillKind::Solid(pick()),
                        ..Default::default()
                    };
                }
                if args.stroke && pn.stroke.enabled {
                    pn.stroke.color = pick();
                }
            }
            SceneNodeKind::Text(tn) => {
                if args.fill {
                    tn.fill = Fill {
                        kind: FillKind::Solid(pick()),
                        ..Default::default()
                    };
                }
            }
            _ => continue,
        }

        history.execute(
            Command::UpdateNode {
                old: node,
                new: new_node,
            },
            &mut doc,
        );
        modified += 1;
    }

    ToolResult::text(format!(
        "Randomized colors on {modified} node(s) from {} palette colors",
        palette.len()
    ))
    .with_data(serde_json::json!({ "modified": modified }))
}

pub async fn swap_fill_stroke(state: &AppState, args: SwapFillStrokeArgs) -> ToolResult {
    tracing::debug!("tool: swap_fill_stroke");
    use photonic_core::style::{Fill, FillKind, Stroke};

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let node_ids: Vec<NodeId> = if args.node_ids.is_empty() {
        doc.selection.node_ids.iter().copied().collect()
    } else {
        args.node_ids
            .iter()
            .filter_map(|s| {
                uuid::Uuid::parse_str(s)
                    .ok()
                    .or_else(|| doc.find_node_by_name(s).map(|n| n.id))
            })
            .collect()
    };

    if node_ids.is_empty() {
        return ToolResult::error("No nodes specified and nothing selected");
    }

    let mut modified = 0usize;

    for nid in &node_ids {
        let node = match doc.nodes.get(nid) {
            Some(n) => n.clone(),
            None => continue,
        };

        let mut new_node = node.clone();
        match &mut new_node.kind {
            SceneNodeKind::Path(pn) => {
                // Extract fill color → new stroke, stroke color → new fill.
                let old_fill_color = match &pn.fill.kind {
                    FillKind::Solid(c) => Some(*c),
                    _ => None,
                };
                let old_stroke_color = pn.stroke.color;
                let old_stroke_width = pn.stroke.width;
                let old_stroke_enabled = pn.stroke.enabled;

                // Set fill from old stroke.
                if old_stroke_enabled {
                    pn.fill = Fill {
                        kind: FillKind::Solid(old_stroke_color),
                        ..Default::default()
                    };
                } else {
                    pn.fill = Fill::none();
                }

                // Set stroke from old fill.
                if let Some(fc) = old_fill_color {
                    pn.stroke = Stroke {
                        color: fc,
                        width: if old_stroke_width > 0.0 {
                            old_stroke_width
                        } else {
                            1.0
                        },
                        enabled: true,
                        ..Default::default()
                    };
                } else {
                    pn.stroke = Stroke::none();
                }
            }
            SceneNodeKind::Text(tn) => {
                let old_fill_color = match &tn.fill.kind {
                    FillKind::Solid(c) => Some(*c),
                    _ => None,
                };
                let old_stroke_color = tn.stroke.color;
                let old_stroke_enabled = tn.stroke.enabled;

                if old_stroke_enabled {
                    tn.fill = Fill {
                        kind: FillKind::Solid(old_stroke_color),
                        ..Default::default()
                    };
                } else {
                    tn.fill = Fill::none();
                }
                if let Some(fc) = old_fill_color {
                    tn.stroke = Stroke {
                        color: fc,
                        width: 1.0,
                        enabled: true,
                        ..Default::default()
                    };
                } else {
                    tn.stroke = Stroke::none();
                }
            }
            _ => continue,
        }

        history.execute(
            Command::UpdateNode {
                old: node,
                new: new_node,
            },
            &mut doc,
        );
        modified += 1;
    }

    ToolResult::text(format!("Swapped fill and stroke on {modified} node(s)"))
        .with_data(serde_json::json!({ "modified": modified }))
}

pub async fn flip_nodes(state: &AppState, args: FlipNodesArgs) -> ToolResult {
    tracing::debug!("tool: flip_nodes");
    use kurbo::Shape;

    let flip_h = args.axis == "horizontal";
    let flip_v = args.axis == "vertical";
    if !flip_h && !flip_v {
        return ToolResult::error("axis must be 'horizontal' or 'vertical'");
    }

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let node_ids: Vec<NodeId> = if args.node_ids.is_empty() {
        doc.selection.node_ids.iter().copied().collect()
    } else {
        args.node_ids
            .iter()
            .filter_map(|s| {
                uuid::Uuid::parse_str(s)
                    .ok()
                    .or_else(|| doc.find_node_by_name(s).map(|n| n.id))
            })
            .collect()
    };

    if node_ids.is_empty() {
        return ToolResult::error("No nodes specified and nothing selected");
    }

    let mut modified = 0usize;

    for nid in &node_ids {
        let node = match doc.nodes.get(nid) {
            Some(n) => n.clone(),
            None => continue,
        };

        let mut new_node = node.clone();

        match &mut new_node.kind {
            SceneNodeKind::Path(pn) => {
                let bez = pn.path_data.to_bez_path();
                let bbox = bez.bounding_box();
                let cx = bbox.x0 + bbox.width() / 2.0;
                let cy = bbox.y0 + bbox.height() / 2.0;

                let flip_point = |p: kurbo::Point| -> kurbo::Point {
                    kurbo::Point::new(
                        if flip_h { 2.0 * cx - p.x } else { p.x },
                        if flip_v { 2.0 * cy - p.y } else { p.y },
                    )
                };

                let mut new_bez = kurbo::BezPath::new();
                for el in bez.elements() {
                    match *el {
                        kurbo::PathEl::MoveTo(p) => new_bez.move_to(flip_point(p)),
                        kurbo::PathEl::LineTo(p) => new_bez.line_to(flip_point(p)),
                        kurbo::PathEl::CurveTo(c1, c2, p) => {
                            new_bez.curve_to(flip_point(c1), flip_point(c2), flip_point(p))
                        }
                        kurbo::PathEl::QuadTo(c, p) => {
                            new_bez.quad_to(flip_point(c), flip_point(p))
                        }
                        kurbo::PathEl::ClosePath => new_bez.close_path(),
                    }
                }
                pn.path_data = PathData::from_bez_path(&new_bez);
            }
            SceneNodeKind::Text(_) | SceneNodeKind::Group(_) => {
                // For text/groups, flip via transform scale.
                if flip_h {
                    new_node.transform.matrix[0] *= -1.0;
                    new_node.transform.matrix[2] *= -1.0;
                }
                if flip_v {
                    new_node.transform.matrix[1] *= -1.0;
                    new_node.transform.matrix[3] *= -1.0;
                }
            }
        }

        history.execute(
            Command::UpdateNode {
                old: node,
                new: new_node,
            },
            &mut doc,
        );
        modified += 1;
    }

    let axis_label = if flip_h { "horizontally" } else { "vertically" };
    ToolResult::text(format!("Flipped {modified} node(s) {axis_label}"))
        .with_data(serde_json::json!({ "modified": modified, "axis": args.axis }))
}

pub async fn create_cross(state: &AppState, args: CreateCrossArgs) -> ToolResult {
    tracing::debug!("tool: create_cross");

    let size = args.size.unwrap_or(60.0);
    let thick = args.thickness.unwrap_or(20.0).min(size);
    let rot_deg = args.rotation.unwrap_or(0.0);

    let half_s = size / 2.0;
    let half_t = thick / 2.0;

    // Cross shape centered at origin, 12-point polygon:
    //   -half_t,-half_s → half_t,-half_s → half_t,-half_t → half_s,-half_t →
    //   half_s,half_t → half_t,half_t → half_t,half_s → -half_t,half_s →
    //   -half_t,half_t → -half_s,half_t → -half_s,-half_t → -half_t,-half_t
    let pts = [
        (-half_t, -half_s),
        (half_t, -half_s),
        (half_t, -half_t),
        (half_s, -half_t),
        (half_s, half_t),
        (half_t, half_t),
        (half_t, half_s),
        (-half_t, half_s),
        (-half_t, half_t),
        (-half_s, half_t),
        (-half_s, -half_t),
        (-half_t, -half_t),
    ];

    let rad = rot_deg.to_radians();
    let cos_r = rad.cos();
    let sin_r = rad.sin();

    let mut bez = kurbo::BezPath::new();
    for (i, &(px, py)) in pts.iter().enumerate() {
        let rx = px * cos_r - py * sin_r + args.cx;
        let ry = px * sin_r + py * cos_r + args.cy;
        if i == 0 {
            bez.move_to((rx, ry));
        } else {
            bez.line_to((rx, ry));
        }
    }
    bez.close_path();

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let layer_id = args
        .layer_id
        .and_then(|s| uuid::Uuid::parse_str(&s).ok())
        .or(doc.active_layer_id)
        .unwrap_or(uuid::Uuid::nil());

    let mut pn = PathNode::new(PathData::from_bez_path(&bez));
    if let Err(e) = apply_style(&mut pn, args.fill, args.stroke) {
        return ToolResult::error(e);
    }

    let node = SceneNode::new("Cross", layer_id, SceneNodeKind::Path(pn));
    let node_id = node.id;
    history.execute(
        Command::AddNode {
            node,
            layer_id: Some(layer_id),
        },
        &mut doc,
    );

    ToolResult::text(format!(
        "Created cross at ({},{}), size={size}, thickness={thick}",
        args.cx, args.cy
    ))
    .with_data(serde_json::json!({ "node_id": node_id }))
}

pub async fn measure_path(state: &AppState, args: MeasurePathArgs) -> ToolResult {
    tracing::debug!("tool: measure_path");
    use kurbo::{ParamCurveArclen, Shape};

    let doc = state.document.lock().await;

    let nid = match uuid::Uuid::parse_str(&args.node_id) {
        Ok(id) => id,
        Err(_) => match doc.find_node_by_name(&args.node_id) {
            Some(n) => n.id,
            None => return ToolResult::error(format!("Node not found: {}", args.node_id)),
        },
    };

    let node = match doc.nodes.get(&nid) {
        Some(n) => n,
        None => return ToolResult::error("Node not found"),
    };

    let pn = match &node.kind {
        SceneNodeKind::Path(pn) => pn,
        _ => return ToolResult::error("Node is not a path"),
    };

    let bez = pn.path_data.to_bez_path();
    let el_count = bez.elements().len();

    // Count segments and compute arc length.
    let segments: Vec<kurbo::PathSeg> = bez.segments().collect();
    let seg_count = segments.len();
    let total_length: f64 = segments.iter().map(|s| s.arclen(0.5)).sum();

    // Bounding box.
    let bbox = bez.bounding_box();
    let is_closed = bez
        .elements()
        .iter()
        .any(|e| matches!(e, kurbo::PathEl::ClosePath));

    // Count anchor points (MoveTo + LineTo + CurveTo + QuadTo endpoints).
    let anchor_count = bez
        .elements()
        .iter()
        .filter(|e| !matches!(e, kurbo::PathEl::ClosePath))
        .count();

    ToolResult::text(format!(
        "Path '{}': length={:.1}, {anchor_count} anchors, {seg_count} segments, {}",
        node.name,
        total_length,
        if is_closed { "closed" } else { "open" },
    ))
    .with_data(serde_json::json!({
        "total_length": total_length,
        "element_count": el_count,
        "segment_count": seg_count,
        "anchor_count": anchor_count,
        "closed": is_closed,
        "bounding_box": {
            "x": bbox.x0,
            "y": bbox.y0,
            "width": bbox.width(),
            "height": bbox.height(),
        },
    }))
}

pub async fn measure_distance(state: &AppState, args: MeasureDistanceArgs) -> ToolResult {
    tracing::debug!("tool: measure_distance");

    let doc = state.document.lock().await;

    let resolve = |target: &MeasureTarget| -> Result<kurbo::Point, String> {
        match target {
            MeasureTarget::Point(p) => Ok(kurbo::Point::new(p[0], p[1])),
            MeasureTarget::NodeId(id_str) => {
                let nid = uuid::Uuid::parse_str(id_str)
                    .ok()
                    .or_else(|| doc.find_node_by_name(id_str).map(|n| n.id));
                let nid = nid.ok_or_else(|| format!("Node not found: {id_str}"))?;
                let node = doc
                    .nodes
                    .get(&nid)
                    .ok_or_else(|| format!("Node not found: {id_str}"))?;
                // Compute center from path bounding box or transform translation.
                match &node.kind {
                    SceneNodeKind::Path(pn) => {
                        use kurbo::Shape;
                        let bez = pn.path_data.to_bez_path();
                        let b = bez.bounding_box();
                        Ok(kurbo::Point::new(
                            b.x0 + b.width() / 2.0 + node.transform.matrix[4],
                            b.y0 + b.height() / 2.0 + node.transform.matrix[5],
                        ))
                    }
                    _ => Ok(kurbo::Point::new(
                        node.transform.matrix[4],
                        node.transform.matrix[5],
                    )),
                }
            }
        }
    };

    let p1 = match resolve(&args.from) {
        Ok(p) => p,
        Err(e) => return ToolResult::error(e),
    };
    let p2 = match resolve(&args.to) {
        Ok(p) => p,
        Err(e) => return ToolResult::error(e),
    };

    let dx = p2.x - p1.x;
    let dy = p2.y - p1.y;
    let distance = (dx * dx + dy * dy).sqrt();
    let angle = dy.atan2(dx).to_degrees();

    ToolResult::text(format!(
        "Distance: {:.2} — from ({:.1},{:.1}) to ({:.1},{:.1}), Δx={:.1}, Δy={:.1}, angle={:.1}°",
        distance, p1.x, p1.y, p2.x, p2.y, dx, dy, angle
    ))
    .with_data(serde_json::json!({
        "distance": distance,
        "dx": dx,
        "dy": dy,
        "angle_degrees": angle,
        "from": [p1.x, p1.y],
        "to": [p2.x, p2.y],
    }))
}

pub async fn create_arrow_shape(state: &AppState, args: CreateArrowShapeArgs) -> ToolResult {
    tracing::debug!("tool: create_arrow_shape");

    let length = args.length.unwrap_or(100.0);
    let head_w = args.head_width.unwrap_or(40.0);
    let head_depth_frac = args.head_depth.unwrap_or(0.4).clamp(0.1, 0.9);
    let shaft_w = args.shaft_width.unwrap_or(16.0);
    let dir_deg = args.direction.unwrap_or(0.0);

    let head_len = length * head_depth_frac;
    let _shaft_len = length - head_len;
    let half_head = head_w / 2.0;
    let half_shaft = shaft_w / 2.0;

    // Build arrow pointing right (direction=0), then rotate.
    // Tip at origin, shaft extends to the left.
    //
    //        (0,0) ← tip
    //       / |
    //      /  |  head_len
    //     /   |
    //    (-head_len, -half_head)  ← top wing
    //    |    (-head_len, -half_shaft) ← shaft top
    //    |    |
    //    |    (-length, -half_shaft) ← shaft end top
    //    |    (-length, +half_shaft) ← shaft end bottom
    //    |    (-head_len, +half_shaft) ← shaft bottom
    //    (-head_len, +half_head) ← bottom wing
    //     \   |
    //      \  |
    //       \ |

    let pts = [
        (0.0, 0.0),               // tip
        (-head_len, -half_head),  // top wing
        (-head_len, -half_shaft), // shaft top start
        (-length, -half_shaft),   // shaft top end
        (-length, half_shaft),    // shaft bottom end
        (-head_len, half_shaft),  // shaft bottom start
        (-head_len, half_head),   // bottom wing
    ];

    // Rotate all points by direction.
    let rad = dir_deg.to_radians();
    let cos_d = rad.cos();
    let sin_d = rad.sin();

    let mut bez = kurbo::BezPath::new();
    for (i, &(px, py)) in pts.iter().enumerate() {
        let rx = px * cos_d - py * sin_d + args.x;
        let ry = px * sin_d + py * cos_d + args.y;
        if i == 0 {
            bez.move_to((rx, ry));
        } else {
            bez.line_to((rx, ry));
        }
    }
    bez.close_path();

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let layer_id = args
        .layer_id
        .and_then(|s| uuid::Uuid::parse_str(&s).ok())
        .or(doc.active_layer_id)
        .unwrap_or(uuid::Uuid::nil());

    let mut pn = PathNode::new(PathData::from_bez_path(&bez));
    if let Err(e) = apply_style(&mut pn, args.fill, args.stroke) {
        return ToolResult::error(e);
    }

    let node = SceneNode::new("Arrow", layer_id, SceneNodeKind::Path(pn));
    let node_id = node.id;
    history.execute(
        Command::AddNode {
            node,
            layer_id: Some(layer_id),
        },
        &mut doc,
    );

    ToolResult::text(format!(
        "Created arrow shape at ({},{}) length={length} dir={dir_deg}°",
        args.x, args.y
    ))
    .with_data(serde_json::json!({ "node_id": node_id }))
}

pub async fn create_donut(state: &AppState, args: CreateDonutArgs) -> ToolResult {
    tracing::debug!("tool: create_donut");
    use kurbo::Shape;

    let outer_r = args.outer_radius.unwrap_or(50.0).max(1.0);
    let inner_r = args
        .inner_radius
        .unwrap_or(25.0)
        .max(0.0)
        .min(outer_r - 0.1);
    let start_deg = args.start_angle.unwrap_or(0.0);
    let end_deg = args.end_angle.unwrap_or(360.0);
    let cx = args.cx;
    let cy = args.cy;

    let is_full = (end_deg - start_deg).abs() >= 359.9;

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let layer_id = args
        .layer_id
        .and_then(|s| uuid::Uuid::parse_str(&s).ok())
        .or(doc.active_layer_id)
        .unwrap_or(uuid::Uuid::nil());

    let mut bez = kurbo::BezPath::new();

    if is_full {
        // Full donut: outer circle CW, inner circle CCW (for even-odd fill rule).
        let outer = kurbo::Ellipse::new((cx, cy), (outer_r, outer_r), 0.0).to_path(0.1);
        for el in outer.elements() {
            bez.push(*el);
        }

        // Inner circle: reverse direction for hole.
        let inner = kurbo::Ellipse::new((cx, cy), (inner_r, inner_r), 0.0).to_path(0.1);
        let inner_els: Vec<_> = inner.elements().to_vec();
        // Reverse the inner path.
        let reversed = reverse_bez(&inner_els);
        for el in &reversed {
            bez.push(*el);
        }
    } else {
        // Partial donut (arc segment).
        let start_rad = start_deg.to_radians();
        let end_rad = end_deg.to_radians();
        let n_segs = 32;

        // Outer arc from start to end.
        for i in 0..=n_segs {
            let t = i as f64 / n_segs as f64;
            let a = start_rad + (end_rad - start_rad) * t;
            let pt = kurbo::Point::new(cx + outer_r * a.cos(), cy + outer_r * a.sin());
            if i == 0 {
                bez.move_to(pt);
            } else {
                bez.line_to(pt);
            }
        }
        // Line to inner arc end.
        let inner_end =
            kurbo::Point::new(cx + inner_r * end_rad.cos(), cy + inner_r * end_rad.sin());
        bez.line_to(inner_end);
        // Inner arc from end back to start.
        for i in (0..=n_segs).rev() {
            let t = i as f64 / n_segs as f64;
            let a = start_rad + (end_rad - start_rad) * t;
            let pt = kurbo::Point::new(cx + inner_r * a.cos(), cy + inner_r * a.sin());
            bez.line_to(pt);
        }
        bez.close_path();
    }

    let mut pn = PathNode::new(PathData::from_bez_path(&bez));
    if let Err(e) = apply_style(&mut pn, args.fill, args.stroke) {
        return ToolResult::error(e);
    }

    let node = SceneNode::new("Donut", layer_id, SceneNodeKind::Path(pn));
    let node_id = node.id;
    history.execute(
        Command::AddNode {
            node,
            layer_id: Some(layer_id),
        },
        &mut doc,
    );

    ToolResult::text(format!(
        "Created donut at ({cx},{cy}) — outer={outer_r}, inner={inner_r}"
    ))
    .with_data(serde_json::json!({ "node_id": node_id }))
}

/// Reverse a sequence of BezPath elements.
fn reverse_bez(els: &[kurbo::PathEl]) -> Vec<kurbo::PathEl> {
    // Collect endpoints in reverse, rebuild path.
    let mut points: Vec<kurbo::Point> = Vec::new();
    for el in els {
        match *el {
            kurbo::PathEl::MoveTo(p) | kurbo::PathEl::LineTo(p) => points.push(p),
            kurbo::PathEl::CurveTo(_, _, p) | kurbo::PathEl::QuadTo(_, p) => points.push(p),
            kurbo::PathEl::ClosePath => {}
        }
    }
    points.reverse();
    let mut result = Vec::new();
    for (i, &p) in points.iter().enumerate() {
        if i == 0 {
            result.push(kurbo::PathEl::MoveTo(p));
        } else {
            result.push(kurbo::PathEl::LineTo(p));
        }
    }
    result.push(kurbo::PathEl::ClosePath);
    result
}

pub async fn create_sunburst(state: &AppState, args: CreateSunburstArgs) -> ToolResult {
    tracing::debug!("tool: create_sunburst");
    use photonic_core::color::Color;
    use photonic_core::style::{Fill, FillKind, Stroke};

    let inner_r = args.inner_radius.unwrap_or(20.0).max(0.0);
    let outer_r = args.outer_radius.unwrap_or(100.0).max(1.0);
    let rays = args.rays.unwrap_or(24).max(4);
    let cx = args.cx;
    let cy = args.cy;

    let ray_color = args.color.as_deref().unwrap_or("#FFD700");
    let color = Color::from_hex(ray_color).unwrap_or(Color::new(1.0, 0.84, 0.0, 1.0));

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let layer_id = args
        .layer_id
        .and_then(|s| uuid::Uuid::parse_str(&s).ok())
        .or(doc.active_layer_id)
        .unwrap_or(uuid::Uuid::nil());

    // Build alternating wedges as a single compound path.
    let mut bez = kurbo::BezPath::new();
    let wedge_angle = std::f64::consts::TAU / rays as f64;

    for i in (0..rays).step_by(2) {
        let a0 = wedge_angle * i as f64;
        let a1 = wedge_angle * (i + 1) as f64;

        // Inner arc start → outer arc start → outer arc end → inner arc end → close.
        let i0 = kurbo::Point::new(cx + inner_r * a0.cos(), cy + inner_r * a0.sin());
        let o0 = kurbo::Point::new(cx + outer_r * a0.cos(), cy + outer_r * a0.sin());
        let o1 = kurbo::Point::new(cx + outer_r * a1.cos(), cy + outer_r * a1.sin());
        let i1 = kurbo::Point::new(cx + inner_r * a1.cos(), cy + inner_r * a1.sin());

        // Approximate the arc with a line (for simplicity — each wedge is ~15° which is fine).
        bez.move_to(i0);
        bez.line_to(o0);
        // Outer arc (approximate with a quadratic through the midpoint).
        let mid_a = (a0 + a1) / 2.0;
        let outer_mid = kurbo::Point::new(cx + outer_r * mid_a.cos(), cy + outer_r * mid_a.sin());
        // Control point for quadratic arc approximation:
        let arc_cp = kurbo::Point::new(
            2.0 * outer_mid.x - 0.5 * (o0.x + o1.x),
            2.0 * outer_mid.y - 0.5 * (o0.y + o1.y),
        );
        bez.quad_to(arc_cp, o1);
        bez.line_to(i1);
        // Inner arc back.
        let inner_mid = kurbo::Point::new(cx + inner_r * mid_a.cos(), cy + inner_r * mid_a.sin());
        let inner_cp = kurbo::Point::new(
            2.0 * inner_mid.x - 0.5 * (i1.x + i0.x),
            2.0 * inner_mid.y - 0.5 * (i1.y + i0.y),
        );
        bez.quad_to(inner_cp, i0);
        bez.close_path();
    }

    let mut pn = PathNode::new(PathData::from_bez_path(&bez));
    pn.fill = Fill {
        kind: FillKind::Solid(color),
        ..Default::default()
    };
    pn.stroke = Stroke::none();

    let node = SceneNode::new("Sunburst", layer_id, SceneNodeKind::Path(pn));
    let node_id = node.id;
    history.execute(
        Command::AddNode {
            node,
            layer_id: Some(layer_id),
        },
        &mut doc,
    );

    ToolResult::text(format!(
        "Created sunburst at ({cx},{cy}) — {rays} rays, inner={inner_r}, outer={outer_r}"
    ))
    .with_data(serde_json::json!({ "node_id": node_id, "rays": rays }))
}

pub async fn create_wave_pattern(state: &AppState, args: CreateWavePatternArgs) -> ToolResult {
    tracing::debug!("tool: create_wave_pattern");

    let lines = args.lines.unwrap_or(8).max(1);
    let wavelength = args.wavelength.unwrap_or(40.0).max(1.0);
    let amplitude = args.amplitude.unwrap_or(10.0);

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let layer_id = args
        .layer_id
        .and_then(|s| uuid::Uuid::parse_str(&s).ok())
        .or(doc.active_layer_id)
        .unwrap_or(uuid::Uuid::nil());

    let mut bez = kurbo::BezPath::new();
    let line_spacing = args.height / lines as f64;
    let points_per_wave = 20; // subdivision for smooth sine
    let total_points = (args.width / wavelength * points_per_wave as f64) as usize + 1;

    for line_idx in 0..lines {
        let base_y = args.y + line_spacing * (line_idx as f64 + 0.5);

        // Generate sine wave points.
        let mut wave_pts: Vec<kurbo::Point> = Vec::with_capacity(total_points);
        for i in 0..=total_points {
            let t = i as f64 / total_points as f64;
            let wx = args.x + t * args.width;
            let phase = t * args.width / wavelength * std::f64::consts::TAU;
            let wy = base_y + amplitude * phase.sin();
            wave_pts.push(kurbo::Point::new(wx, wy));
        }

        // Convert to smooth bezier using Catmull-Rom.
        if wave_pts.len() >= 2 {
            bez.move_to(wave_pts[0]);
            for i in 0..wave_pts.len() - 1 {
                let p0 = if i > 0 {
                    wave_pts[i - 1]
                } else {
                    kurbo::Point::new(
                        2.0 * wave_pts[0].x - wave_pts[1].x,
                        2.0 * wave_pts[0].y - wave_pts[1].y,
                    )
                };
                let p1 = wave_pts[i];
                let p2 = wave_pts[i + 1];
                let p3 = if i + 2 < wave_pts.len() {
                    wave_pts[i + 2]
                } else {
                    let n = wave_pts.len();
                    kurbo::Point::new(
                        2.0 * wave_pts[n - 1].x - wave_pts[n - 2].x,
                        2.0 * wave_pts[n - 1].y - wave_pts[n - 2].y,
                    )
                };
                let cp1 = kurbo::Point::new(p1.x + (p2.x - p0.x) / 6.0, p1.y + (p2.y - p0.y) / 6.0);
                let cp2 = kurbo::Point::new(p2.x - (p3.x - p1.x) / 6.0, p2.y - (p3.y - p1.y) / 6.0);
                bez.curve_to(cp1, cp2, p2);
            }
        }
    }

    let mut pn = PathNode::new(PathData::from_bez_path(&bez));
    pn.fill = photonic_core::style::Fill::none();

    if let Some(stroke_arg) = args.stroke {
        match stroke_arg.to_stroke() {
            Ok(s) => pn.stroke = s,
            Err(e) => return ToolResult::error(e),
        }
    } else {
        pn.stroke = photonic_core::style::Stroke {
            color: photonic_core::color::Color::new(0.2, 0.4, 0.8, 1.0),
            width: 1.5,
            ..Default::default()
        };
    }

    let node = SceneNode::new("Wave Pattern", layer_id, SceneNodeKind::Path(pn));
    let node_id = node.id;
    history.execute(
        Command::AddNode {
            node,
            layer_id: Some(layer_id),
        },
        &mut doc,
    );

    ToolResult::text(format!(
        "Created wave pattern — {} lines, wavelength={wavelength}, amplitude={amplitude}",
        lines
    ))
    .with_data(serde_json::json!({ "node_id": node_id, "lines": lines }))
}

pub async fn hatch_fill(state: &AppState, args: HatchFillArgs) -> ToolResult {
    tracing::debug!("tool: hatch_fill");
    use kurbo::Shape;
    use photonic_core::color::Color;
    use photonic_core::style::{Fill, Stroke};

    if args.node_ids.is_empty() {
        return ToolResult::error("node_ids must not be empty");
    }

    let spacing = args.spacing.unwrap_or(5.0).max(0.5);
    let angle_deg = args.angle.unwrap_or(45.0);
    let stroke_w = args.stroke_width.unwrap_or(1.0);

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let mut created = 0usize;
    let mut skipped = 0usize;

    let angles: Vec<f64> = {
        let mut a = vec![angle_deg];
        if let Some(ca) = args.cross_angle {
            a.push(ca);
        }
        a
    };

    for node_id_str in &args.node_ids {
        let nid = match uuid::Uuid::parse_str(node_id_str) {
            Ok(id) => id,
            Err(_) => match doc.find_node_by_name(node_id_str) {
                Some(n) => n.id,
                None => {
                    skipped += 1;
                    continue;
                }
            },
        };
        let node = match doc.nodes.get(&nid) {
            Some(n) => n.clone(),
            None => {
                skipped += 1;
                continue;
            }
        };
        let pn = match &node.kind {
            SceneNodeKind::Path(pn) => pn,
            _ => {
                skipped += 1;
                continue;
            }
        };

        let bez = pn.path_data.to_bez_path();
        let bbox = bez.bounding_box();
        let bw = bbox.width();
        let bh = bbox.height();
        if bw < 1e-9 || bh < 1e-9 {
            skipped += 1;
            continue;
        }

        let hatch_color = if let Some(ref hex) = args.color {
            Color::from_hex(hex).unwrap_or(Color::BLACK)
        } else {
            match &pn.fill.kind {
                photonic_core::style::FillKind::Solid(c) => *c,
                _ => Color::BLACK,
            }
        };

        let layer_id = node.layer_id;
        let cx = bbox.x0 + bw / 2.0;
        let cy = bbox.y0 + bh / 2.0;
        let diag = (bw * bw + bh * bh).sqrt();

        let mut hatch_path = kurbo::BezPath::new();

        for angle in &angles {
            let rad = angle.to_radians();
            let cos_a = rad.cos();
            let sin_a = rad.sin();

            // Direction perpendicular to hatch lines.
            let perp_x = -sin_a;
            let perp_y = cos_a;

            let n_lines = (diag / spacing) as i32 + 1;

            for i in -n_lines..=n_lines {
                let offset = i as f64 * spacing;
                // Line center point offset perpendicular to the hatch direction.
                let lx = cx + perp_x * offset;
                let ly = cy + perp_y * offset;

                // Line endpoints extending in the hatch direction.
                let p0 = kurbo::Point::new(lx - cos_a * diag, ly - sin_a * diag);
                let p1 = kurbo::Point::new(lx + cos_a * diag, ly + sin_a * diag);

                // Sample points along the line and find segments inside the path.
                let samples = 100;
                let mut inside = false;
                let mut seg_start = p0;

                for s in 0..=samples {
                    let t = s as f64 / samples as f64;
                    let pt = kurbo::Point::new(p0.x + (p1.x - p0.x) * t, p0.y + (p1.y - p0.y) * t);
                    let is_inside = bez.winding(pt) != 0;

                    if is_inside && !inside {
                        seg_start = pt;
                        inside = true;
                    } else if !is_inside && inside {
                        hatch_path.move_to(seg_start);
                        hatch_path.line_to(pt);
                        inside = false;
                    }
                }
                if inside {
                    hatch_path.move_to(seg_start);
                    hatch_path.line_to(p1);
                }
            }
        }

        if hatch_path.elements().is_empty() {
            skipped += 1;
            continue;
        }

        let mut hatch_pn = PathNode::new(PathData::from_bez_path(&hatch_path));
        hatch_pn.fill = Fill::none();
        hatch_pn.stroke = Stroke {
            color: hatch_color,
            width: stroke_w,
            ..Default::default()
        };

        let hatch_node = SceneNode::new(
            &format!("{} Hatch", node.name),
            layer_id,
            SceneNodeKind::Path(hatch_pn),
        );
        history.execute(
            Command::AddNode {
                node: hatch_node,
                layer_id: Some(layer_id),
            },
            &mut doc,
        );
        created += 1;
    }

    if created == 0 {
        return ToolResult::error("No valid path nodes found for hatch fill");
    }

    ToolResult::text(format!(
        "Created hatch fill for {} node(s) (spacing={spacing}, angle={angle_deg}°){}",
        created,
        if skipped > 0 {
            format!(" — {skipped} skipped")
        } else {
            String::new()
        },
    ))
    .with_data(serde_json::json!({ "created": created, "skipped": skipped }))
}

pub async fn stipple_fill(state: &AppState, args: StippleFillArgs) -> ToolResult {
    tracing::debug!("tool: stipple_fill");
    use kurbo::Shape;
    use photonic_core::color::Color;
    use photonic_core::style::{Fill, FillKind, Stroke};

    if args.node_ids.is_empty() {
        return ToolResult::error("node_ids must not be empty");
    }

    let count = args.count.unwrap_or(200).max(1);
    let dot_r = args.dot_radius.unwrap_or(1.5);
    let seed = args.seed.unwrap_or(42).max(1);

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let mut created_groups = 0usize;
    let mut skipped = 0usize;

    for node_id_str in &args.node_ids {
        let nid = match uuid::Uuid::parse_str(node_id_str) {
            Ok(id) => id,
            Err(_) => match doc.find_node_by_name(node_id_str) {
                Some(n) => n.id,
                None => {
                    skipped += 1;
                    continue;
                }
            },
        };
        let node = match doc.nodes.get(&nid) {
            Some(n) => n.clone(),
            None => {
                skipped += 1;
                continue;
            }
        };
        let pn = match &node.kind {
            SceneNodeKind::Path(pn) => pn,
            _ => {
                skipped += 1;
                continue;
            }
        };

        let bez = pn.path_data.to_bez_path();
        let bbox = bez.bounding_box();
        let bw = bbox.width();
        let bh = bbox.height();
        if bw < 1e-9 || bh < 1e-9 {
            skipped += 1;
            continue;
        }

        // Determine dot color.
        let dot_color = if let Some(ref hex) = args.color {
            Color::from_hex(hex).unwrap_or(Color::BLACK)
        } else {
            match &pn.fill.kind {
                FillKind::Solid(c) => *c,
                _ => Color::BLACK,
            }
        };

        let layer_id = node.layer_id;

        // Generate dots using rejection sampling.
        let mut rng = seed;
        let mut dot_path = kurbo::BezPath::new();
        let mut placed = 0usize;
        let max_attempts = count * 20; // prevent infinite loop on very small shapes

        for _ in 0..max_attempts {
            if placed >= count {
                break;
            }
            let rx = xorshift64(&mut rng) * 0.5 + 0.5; // [0, 1]
            let ry = xorshift64(&mut rng) * 0.5 + 0.5;
            let px = bbox.x0 + rx * bw;
            let py = bbox.y0 + ry * bh;
            let pt = kurbo::Point::new(px, py);

            // Test if point is inside the path.
            if bez.winding(pt) != 0 {
                // Add a small circle at this point.
                let circle = kurbo::Circle::new(pt, dot_r);
                for el in circle.to_path(0.1).elements() {
                    dot_path.push(*el);
                }
                placed += 1;
            }
        }

        if placed == 0 {
            skipped += 1;
            continue;
        }

        let mut dot_pn = PathNode::new(PathData::from_bez_path(&dot_path));
        dot_pn.fill = Fill {
            kind: FillKind::Solid(dot_color),
            ..Default::default()
        };
        dot_pn.stroke = Stroke::none();

        let dot_node = SceneNode::new(
            &format!("{} Stipple", node.name),
            layer_id,
            SceneNodeKind::Path(dot_pn),
        );
        history.execute(
            Command::AddNode {
                node: dot_node,
                layer_id: Some(layer_id),
            },
            &mut doc,
        );
        created_groups += 1;
    }

    if created_groups == 0 {
        return ToolResult::error("No valid path nodes found for stipple fill");
    }

    ToolResult::text(format!(
        "Created stipple fill for {} node(s) ({count} dots each){}",
        created_groups,
        if skipped > 0 {
            format!(" — {skipped} skipped")
        } else {
            String::new()
        },
    ))
    .with_data(serde_json::json!({ "created": created_groups, "skipped": skipped }))
}

pub async fn add_drop_shadow(state: &AppState, args: AddDropShadowArgs) -> ToolResult {
    tracing::debug!("tool: add_drop_shadow");
    use photonic_core::color::Color;
    use photonic_core::style::{Fill, FillKind};

    if args.node_ids.is_empty() {
        return ToolResult::error("node_ids must not be empty");
    }

    let ox = args.offset_x.unwrap_or(5.0);
    let oy = args.offset_y.unwrap_or(5.0);
    let shadow_opacity = args.opacity.unwrap_or(0.4);
    let shadow_color = args.color.as_deref().unwrap_or("#000000");
    let sc = Color::from_hex(shadow_color).unwrap_or(Color::new(0.0, 0.0, 0.0, 1.0));

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let mut created = 0usize;
    let mut skipped = 0usize;

    for node_id_str in &args.node_ids {
        let nid = match uuid::Uuid::parse_str(node_id_str) {
            Ok(id) => id,
            Err(_) => match doc.find_node_by_name(node_id_str) {
                Some(n) => n.id,
                None => {
                    skipped += 1;
                    continue;
                }
            },
        };
        let node = match doc.nodes.get(&nid) {
            Some(n) => n.clone(),
            None => {
                skipped += 1;
                continue;
            }
        };

        // Create shadow: duplicate node, offset, recolor, place below original.
        let mut shadow = node.clone();
        shadow.id = uuid::Uuid::new_v4();
        shadow.name = format!("{} Shadow", node.name);
        shadow.opacity = shadow_opacity;

        // Apply offset to transform.
        shadow.transform.matrix[4] += ox;
        shadow.transform.matrix[5] += oy;

        // Recolor: set fill to shadow color for paths, set text fill for text.
        match &mut shadow.kind {
            SceneNodeKind::Path(pn) => {
                pn.fill = Fill {
                    kind: FillKind::Solid(sc),
                    ..Default::default()
                };
                pn.stroke = photonic_core::style::Stroke::none();
            }
            SceneNodeKind::Text(tn) => {
                tn.fill = Fill {
                    kind: FillKind::Solid(sc),
                    ..Default::default()
                };
                tn.stroke = photonic_core::style::Stroke::none();
            }
            SceneNodeKind::Group(_) => {
                // For groups, just offset and set opacity — child colors preserved as silhouette.
            }
        }

        history.execute(
            Command::AddNode {
                node: shadow,
                layer_id: Some(node.layer_id),
            },
            &mut doc,
        );
        created += 1;
    }

    if created == 0 {
        return ToolResult::error("No valid nodes found");
    }

    ToolResult::text(format!(
        "Added drop shadow to {} node(s) (offset=[{ox},{oy}], opacity={shadow_opacity}){}",
        created,
        if skipped > 0 {
            format!(" — {skipped} skipped")
        } else {
            String::new()
        },
    ))
    .with_data(serde_json::json!({ "created": created, "skipped": skipped }))
}

pub async fn transform_copies(state: &AppState, args: TransformCopiesArgs) -> ToolResult {
    tracing::debug!("tool: transform_copies");

    let copies = args.copies.unwrap_or(5).max(1);
    let tx = args.translate_x.unwrap_or(0.0);
    let ty = args.translate_y.unwrap_or(0.0);
    let rot_deg = args.rotate.unwrap_or(0.0);
    let scale_factor = args.scale.unwrap_or(1.0);
    let opacity_step = args.opacity_step.unwrap_or(1.0);

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let nid = match uuid::Uuid::parse_str(&args.node_id) {
        Ok(id) => id,
        Err(_) => match doc.find_node_by_name(&args.node_id) {
            Some(n) => n.id,
            None => return ToolResult::error(format!("Node not found: {}", args.node_id)),
        },
    };

    let source = match doc.nodes.get(&nid) {
        Some(n) => n.clone(),
        None => return ToolResult::error("Source node not found"),
    };

    let layer_id = source.layer_id;
    let mut created_ids = Vec::new();

    // Each copy accumulates transforms from the previous.
    let base_matrix = source.transform.matrix;

    for i in 1..=copies {
        let mut new_node = source.clone();
        new_node.id = uuid::Uuid::new_v4();
        new_node.name = format!("{} Copy {}", source.name, i);

        // Compute cumulative transform: translate, rotate, scale applied i times.
        let cumulative_tx = tx * i as f64;
        let cumulative_ty = ty * i as f64;
        let cumulative_rot = (rot_deg * i as f64).to_radians();
        let cumulative_scale = scale_factor.powi(i as i32);
        let cumulative_opacity = opacity_step.powi(i as i32);

        // Build the incremental transform matrix.
        let cos_r = cumulative_rot.cos();
        let sin_r = cumulative_rot.sin();
        let s = cumulative_scale;

        // Transform: scale * rotate, then translate
        // [s*cos  -s*sin  tx]
        // [s*sin   s*cos  ty]
        let inc_matrix = [
            s * cos_r,
            s * sin_r,
            -s * sin_r,
            s * cos_r,
            cumulative_tx,
            cumulative_ty,
        ];

        // Compose: inc_matrix * base_matrix
        let a = inc_matrix;
        let b = base_matrix;
        new_node.transform = Transform {
            matrix: [
                a[0] * b[0] + a[2] * b[1],
                a[1] * b[0] + a[3] * b[1],
                a[0] * b[2] + a[2] * b[3],
                a[1] * b[2] + a[3] * b[3],
                a[0] * b[4] + a[2] * b[5] + a[4],
                a[1] * b[4] + a[3] * b[5] + a[5],
            ],
        };

        new_node.opacity = (source.opacity * cumulative_opacity).clamp(0.0, 1.0);

        let copy_id = new_node.id;
        created_ids.push(copy_id);
        history.execute(
            Command::AddNode {
                node: new_node,
                layer_id: Some(layer_id),
            },
            &mut doc,
        );
    }

    ToolResult::text(format!(
        "Created {} copies of '{}' (translate=[{tx},{ty}], rotate={rot_deg}°, scale={scale_factor})",
        copies, source.name
    ))
    .with_data(serde_json::json!({
        "copies": copies,
        "created_ids": created_ids,
    }))
}

pub async fn round_corners(state: &AppState, args: RoundCornersArgs) -> ToolResult {
    tracing::debug!("tool: round_corners");

    if args.node_ids.is_empty() {
        return ToolResult::error("node_ids must not be empty");
    }

    let radius = args.radius.unwrap_or(10.0).max(0.0);

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let mut commands = Vec::new();
    let mut modified = 0usize;
    let mut skipped = 0usize;

    for node_id_str in &args.node_ids {
        let nid = match uuid::Uuid::parse_str(node_id_str) {
            Ok(id) => id,
            Err(_) => match doc.find_node_by_name(node_id_str) {
                Some(n) => n.id,
                None => {
                    skipped += 1;
                    continue;
                }
            },
        };
        let node = match doc.nodes.get(&nid) {
            Some(n) => n.clone(),
            None => {
                skipped += 1;
                continue;
            }
        };
        let pn = match &node.kind {
            SceneNodeKind::Path(pn) => pn,
            _ => {
                skipped += 1;
                continue;
            }
        };

        let bez = pn.path_data.to_bez_path();
        let new_bez = apply_round_corners(&bez, radius);

        let mut new_node = node.clone();
        if let SceneNodeKind::Path(ref mut new_pn) = new_node.kind {
            new_pn.path_data = PathData::from_bez_path(&new_bez);
        }
        commands.push(Command::UpdateNode {
            old: node,
            new: new_node,
        });
        modified += 1;
    }

    if commands.is_empty() {
        return ToolResult::error("No path nodes found in node_ids");
    }

    for cmd in commands {
        history.execute(cmd, &mut doc);
    }

    ToolResult::text(format!(
        "Rounded corners on {} node(s) (radius={radius}){}",
        modified,
        if skipped > 0 {
            format!(" — {skipped} skipped")
        } else {
            String::new()
        },
    ))
    .with_data(serde_json::json!({ "modified": modified, "skipped": skipped }))
}

/// Round corners of a BezPath by replacing sharp corners with quadratic bezier arcs.
fn apply_round_corners(bez: &kurbo::BezPath, radius: f64) -> kurbo::BezPath {
    // Collect subpaths as sequences of endpoints.
    let elements = bez.elements();
    if elements.is_empty() || radius <= 0.0 {
        return bez.clone();
    }

    // For each subpath, collect the line endpoints and process corners.
    let mut result = kurbo::BezPath::new();
    let mut subpath: Vec<kurbo::Point> = Vec::new();
    let mut is_closed = false;

    let flush = |result: &mut kurbo::BezPath, pts: &[kurbo::Point], closed: bool, radius: f64| {
        if pts.len() < 2 {
            if let Some(&p) = pts.first() {
                result.move_to(p);
            }
            return;
        }

        let n = pts.len();
        let effective_n = if closed { n } else { n };

        for i in 0..effective_n {
            let prev = if i == 0 {
                if closed {
                    pts[n - 1]
                } else {
                    pts[0]
                }
            } else {
                pts[i - 1]
            };
            let curr = pts[i];
            let next = if i == n - 1 {
                if closed {
                    pts[0]
                } else {
                    pts[n - 1]
                }
            } else {
                pts[i + 1]
            };

            let is_endpoint = (!closed) && (i == 0 || i == n - 1);

            if is_endpoint {
                if i == 0 {
                    result.move_to(curr);
                } else {
                    result.line_to(curr);
                }
            } else {
                // Compute the fillet at this corner.
                let dx_in = curr.x - prev.x;
                let dy_in = curr.y - prev.y;
                let len_in = (dx_in * dx_in + dy_in * dy_in).sqrt();

                let dx_out = next.x - curr.x;
                let dy_out = next.y - curr.y;
                let len_out = (dx_out * dx_out + dy_out * dy_out).sqrt();

                if len_in < 1e-9 || len_out < 1e-9 {
                    if i == 0 {
                        result.move_to(curr);
                    } else {
                        result.line_to(curr);
                    }
                    continue;
                }

                // Clamp radius to half the shortest adjacent segment.
                let max_r = (len_in / 2.0).min(len_out / 2.0);
                let r = radius.min(max_r);

                // Points on incoming and outgoing segments at distance r from corner.
                let fillet_start =
                    kurbo::Point::new(curr.x - (dx_in / len_in) * r, curr.y - (dy_in / len_in) * r);
                let fillet_end = kurbo::Point::new(
                    curr.x + (dx_out / len_out) * r,
                    curr.y + (dy_out / len_out) * r,
                );

                if i == 0 && closed {
                    result.move_to(fillet_start);
                } else if i == 0 {
                    result.move_to(fillet_start);
                } else {
                    result.line_to(fillet_start);
                }

                // Quadratic bezier with control point at the original corner
                // produces a smooth fillet arc.
                result.quad_to(curr, fillet_end);
            }
        }

        if closed {
            result.close_path();
        }
    };

    for el in elements {
        match *el {
            kurbo::PathEl::MoveTo(p) => {
                if !subpath.is_empty() {
                    flush(&mut result, &subpath, is_closed, radius);
                }
                subpath.clear();
                subpath.push(p);
                is_closed = false;
            }
            kurbo::PathEl::LineTo(p) => {
                subpath.push(p);
            }
            kurbo::PathEl::CurveTo(_, _, p) | kurbo::PathEl::QuadTo(_, p) => {
                // For curves, just keep the endpoint (fillet only applies to line corners).
                subpath.push(p);
            }
            kurbo::PathEl::ClosePath => {
                is_closed = true;
            }
        }
    }

    if !subpath.is_empty() {
        flush(&mut result, &subpath, is_closed, radius);
    }

    result
}

pub async fn warp_envelope(state: &AppState, args: WarpEnvelopeArgs) -> ToolResult {
    tracing::debug!("tool: warp_envelope");

    if args.node_ids.is_empty() {
        return ToolResult::error("node_ids must not be empty");
    }

    let valid_types = [
        "arc",
        "arc_lower",
        "arc_upper",
        "arch",
        "bulge",
        "wave",
        "flag",
        "squeeze",
        "inflate",
        "fisheye",
        "shell_lower",
        "shell_upper",
        "fish",
        "rise",
        "twist",
    ];
    if !valid_types.contains(&args.warp_type.as_str()) {
        return ToolResult::error(format!(
            "Unknown warp_type: '{}'. Use one of: {}",
            args.warp_type,
            valid_types.join(", ")
        ));
    }

    let bend = args.bend.unwrap_or(0.5);
    let dh = args.distort_h.unwrap_or(0.0);
    let dv = args.distort_v.unwrap_or(0.0);

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let mut commands = Vec::new();
    let mut modified = 0usize;
    let mut skipped = 0usize;

    for node_id_str in &args.node_ids {
        let nid = match uuid::Uuid::parse_str(node_id_str) {
            Ok(id) => id,
            Err(_) => match doc.find_node_by_name(node_id_str) {
                Some(n) => n.id,
                None => {
                    skipped += 1;
                    continue;
                }
            },
        };
        let node = match doc.nodes.get(&nid) {
            Some(n) => n.clone(),
            None => {
                skipped += 1;
                continue;
            }
        };
        let pn = match &node.kind {
            SceneNodeKind::Path(pn) => pn,
            _ => {
                skipped += 1;
                continue;
            }
        };

        let bez = pn.path_data.to_bez_path();
        let new_bez = apply_warp_envelope(&bez, &args.warp_type, bend, dh, dv);

        let mut new_node = node.clone();
        if let SceneNodeKind::Path(ref mut new_pn) = new_node.kind {
            new_pn.path_data = PathData::from_bez_path(&new_bez);
        }
        commands.push(Command::UpdateNode {
            old: node,
            new: new_node,
        });
        modified += 1;
    }

    if commands.is_empty() {
        return ToolResult::error("No path nodes found in node_ids");
    }

    for cmd in commands {
        history.execute(cmd, &mut doc);
    }

    ToolResult::text(format!(
        "Applied '{}' warp to {} node(s) (bend={bend}){}",
        args.warp_type, modified,
        if skipped > 0 { format!(" — {skipped} skipped") } else { String::new() },
    ))
    .with_data(serde_json::json!({ "modified": modified, "skipped": skipped, "warp_type": args.warp_type }))
}

/// Apply a named warp envelope to a BezPath.
/// Points are normalized to [0,1] based on bounding box, warped, then scaled back.
fn apply_warp_envelope(
    bez: &kurbo::BezPath,
    warp_type: &str,
    bend: f64,
    dh: f64,
    dv: f64,
) -> kurbo::BezPath {
    // Compute bounding box.
    let mut min_x = f64::MAX;
    let mut min_y = f64::MAX;
    let mut max_x = f64::MIN;
    let mut max_y = f64::MIN;

    for el in bez.elements() {
        let pts: Vec<kurbo::Point> = match *el {
            kurbo::PathEl::MoveTo(p) | kurbo::PathEl::LineTo(p) => vec![p],
            kurbo::PathEl::CurveTo(c1, c2, p) => vec![c1, c2, p],
            kurbo::PathEl::QuadTo(c, p) => vec![c, p],
            kurbo::PathEl::ClosePath => vec![],
        };
        for p in pts {
            min_x = min_x.min(p.x);
            min_y = min_y.min(p.y);
            max_x = max_x.max(p.x);
            max_y = max_y.max(p.y);
        }
    }

    let w = max_x - min_x;
    let h = max_y - min_y;
    if w < 1e-9 || h < 1e-9 {
        return bez.clone();
    }

    let warp_point = |p: kurbo::Point| -> kurbo::Point {
        // Normalize to [0,1].
        let nx = (p.x - min_x) / w;
        let ny = (p.y - min_y) / h;

        let (dx, dy) = match warp_type {
            "arc" => {
                // Bend along an arc: vertical displacement follows sin(π*x).
                (
                    dh * (ny - 0.5) * w,
                    bend * (nx * (1.0 - nx) * 4.0) * h * 0.25,
                )
            }
            "bulge" => {
                // Horizontal expansion in the middle.
                let cx = nx - 0.5;
                let cy = ny - 0.5;
                let r = (cx * cx + cy * cy).sqrt().min(0.5);
                let factor = bend * (1.0 - r * 2.0).max(0.0);
                (cx * factor * w, cy * factor * h)
            }
            "wave" => {
                // Sinusoidal wave.
                (
                    dh * (std::f64::consts::PI * 2.0 * ny).sin() * w * 0.1,
                    bend * (std::f64::consts::PI * 2.0 * nx).sin() * h * 0.25,
                )
            }
            "flag" => {
                // Flag wave: amplitude increases with x.
                (
                    0.0,
                    bend * nx * (std::f64::consts::PI * 2.0 * ny).sin() * h * 0.25,
                )
            }
            "squeeze" => {
                // Compress horizontally in the middle, expand at edges.
                let cy = ny - 0.5;
                (
                    bend * cy * cy * (nx - 0.5) * w * -2.0,
                    dv * (nx - 0.5) * h * 0.1,
                )
            }
            "inflate" => {
                // Expand everything from center.
                let cx = nx - 0.5;
                let cy = ny - 0.5;
                let dist = (cx * cx + cy * cy).sqrt();
                let factor = bend * (1.0 - dist * 2.0).max(0.0);
                (cx * factor * w * 0.5, cy * factor * h * 0.5)
            }
            "fisheye" => {
                // Fisheye lens distortion.
                let cx = nx - 0.5;
                let cy = ny - 0.5;
                let r = (cx * cx + cy * cy).sqrt();
                if r < 1e-9 {
                    (0.0, 0.0)
                } else {
                    let factor = bend * r;
                    (cx * factor * w * 0.5, cy * factor * h * 0.5)
                }
            }
            "arc_lower" => {
                // Bend only the bottom edge.
                (0.0, bend * ny * (nx * (1.0 - nx) * 4.0) * h * 0.25)
            }
            "arc_upper" => {
                // Bend only the top edge.
                (0.0, bend * (1.0 - ny) * (nx * (1.0 - nx) * 4.0) * h * 0.25)
            }
            "arch" => {
                // Arch: arc on top, flat on bottom (semicircular arch).
                let arch_amt = (1.0 - ny) * bend * (nx * (1.0 - nx) * 4.0) * h * 0.25;
                (0.0, -arch_amt)
            }
            "shell_lower" => {
                // Shell: curl the bottom inward.
                let t = ny;
                (bend * t * (nx - 0.5) * w * 0.5, bend * t * t * h * 0.2)
            }
            "shell_upper" => {
                // Shell: curl the top inward.
                let t = 1.0 - ny;
                (bend * t * (nx - 0.5) * w * 0.5, -bend * t * t * h * 0.2)
            }
            "fish" => {
                // Fish: pinch horizontally at top and bottom, expand at middle.
                let cy = ny - 0.5;
                let factor = bend * (1.0 - 4.0 * cy * cy);
                (factor * (nx - 0.5) * w * 0.3, 0.0)
            }
            "rise" => {
                // Rise: progressive vertical displacement increasing left to right.
                (0.0, bend * nx * nx * h * 0.3)
            }
            "twist" => {
                // Twist: rotate progressively from bottom to top.
                let angle = bend * (ny - 0.5) * std::f64::consts::PI;
                let cx = nx - 0.5;
                let cos_a = angle.cos();
                let sin_a = angle.sin();
                ((cx * cos_a - cx) * w, (cx * sin_a) * w)
            }
            _ => (0.0, 0.0),
        };

        kurbo::Point::new(p.x + dx, p.y + dy)
    };

    let mut result = kurbo::BezPath::new();
    for el in bez.elements() {
        match *el {
            kurbo::PathEl::MoveTo(p) => result.move_to(warp_point(p)),
            kurbo::PathEl::LineTo(p) => result.line_to(warp_point(p)),
            kurbo::PathEl::CurveTo(c1, c2, p) => {
                result.curve_to(warp_point(c1), warp_point(c2), warp_point(p))
            }
            kurbo::PathEl::QuadTo(c, p) => result.quad_to(warp_point(c), warp_point(p)),
            kurbo::PathEl::ClosePath => result.close_path(),
        }
    }
    result
}

pub async fn scallop_path(state: &AppState, args: ScallopPathArgs) -> ToolResult {
    tracing::debug!("tool: scallop_path");

    if args.node_ids.is_empty() {
        return ToolResult::error("node_ids must not be empty");
    }

    let depth = args.depth.unwrap_or(10.0);
    let count = args.count.unwrap_or(1).max(1);

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let mut commands = Vec::new();
    let mut modified = 0usize;
    let mut skipped = 0usize;

    for node_id_str in &args.node_ids {
        let nid = match uuid::Uuid::parse_str(node_id_str) {
            Ok(id) => id,
            Err(_) => match doc.find_node_by_name(node_id_str) {
                Some(n) => n.id,
                None => {
                    skipped += 1;
                    continue;
                }
            },
        };
        let node = match doc.nodes.get(&nid) {
            Some(n) => n.clone(),
            None => {
                skipped += 1;
                continue;
            }
        };
        let pn = match &node.kind {
            SceneNodeKind::Path(pn) => pn,
            _ => {
                skipped += 1;
                continue;
            }
        };

        let bez = pn.path_data.to_bez_path();
        let new_bez = apply_scallop(&bez, depth, count);

        let mut new_node = node.clone();
        if let SceneNodeKind::Path(ref mut new_pn) = new_node.kind {
            new_pn.path_data = PathData::from_bez_path(&new_bez);
        }
        commands.push(Command::UpdateNode {
            old: node,
            new: new_node,
        });
        modified += 1;
    }

    if commands.is_empty() {
        return ToolResult::error("No path nodes found in node_ids");
    }

    for cmd in commands {
        history.execute(cmd, &mut doc);
    }

    ToolResult::text(format!(
        "Applied scallop to {} node(s) (depth={depth}, count={count}){}",
        modified,
        if skipped > 0 {
            format!(" — {skipped} skipped")
        } else {
            String::new()
        },
    ))
    .with_data(serde_json::json!({ "modified": modified, "skipped": skipped }))
}

/// Replace each line/curve segment with scallop arcs (smooth inward curves).
fn apply_scallop(bez: &kurbo::BezPath, depth: f64, count: usize) -> kurbo::BezPath {
    let mut result = kurbo::BezPath::new();
    let mut current = kurbo::Point::ZERO;
    let mut subpath_start = kurbo::Point::ZERO;

    for el in bez.elements() {
        match *el {
            kurbo::PathEl::MoveTo(p) => {
                result.move_to(p);
                current = p;
                subpath_start = p;
            }
            kurbo::PathEl::ClosePath => {
                if current != subpath_start {
                    scallop_segment(&mut result, current, subpath_start, depth, count);
                }
                result.close_path();
                current = subpath_start;
            }
            _ => {
                let endpoint = match *el {
                    kurbo::PathEl::LineTo(p)
                    | kurbo::PathEl::CurveTo(_, _, p)
                    | kurbo::PathEl::QuadTo(_, p) => p,
                    _ => unreachable!(),
                };
                let start = {
                    let els = result.elements();
                    let mut pt = kurbo::Point::ZERO;
                    for e in els.iter().rev() {
                        match e {
                            kurbo::PathEl::MoveTo(p)
                            | kurbo::PathEl::LineTo(p)
                            | kurbo::PathEl::CurveTo(_, _, p)
                            | kurbo::PathEl::QuadTo(_, p) => {
                                pt = *p;
                                break;
                            }
                            kurbo::PathEl::ClosePath => {}
                        }
                    }
                    pt
                };
                scallop_segment(&mut result, start, endpoint, depth, count);
                current = endpoint;
            }
        }
    }
    result
}

/// Emit scallop arcs between `from` and `to`.
fn scallop_segment(
    path: &mut kurbo::BezPath,
    from: kurbo::Point,
    to: kurbo::Point,
    depth: f64,
    count: usize,
) {
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1e-9 {
        path.line_to(to);
        return;
    }

    // Normal points inward (to the right of the direction).
    let nx = dy / len;
    let ny = -dx / len;

    for i in 0..count {
        let t0 = i as f64 / count as f64;
        let t1 = (i + 1) as f64 / count as f64;
        let tmid = (t0 + t1) / 2.0;

        let p0 = kurbo::Point::new(from.x + dx * t0, from.y + dy * t0);
        let p1 = kurbo::Point::new(from.x + dx * t1, from.y + dy * t1);
        let pmid = kurbo::Point::new(
            from.x + dx * tmid + nx * depth,
            from.y + dy * tmid + ny * depth,
        );

        // Quadratic bezier through the midpoint creates a smooth arc.
        // Control point for quadratic that passes through pmid at t=0.5:
        // Q = 2*pmid - 0.5*(p0 + p1)
        let qx = 2.0 * pmid.x - 0.5 * (p0.x + p1.x);
        let qy = 2.0 * pmid.y - 0.5 * (p0.y + p1.y);

        path.quad_to(kurbo::Point::new(qx, qy), p1);
    }
}

pub async fn crystallize_path(state: &AppState, args: CrystallizePathArgs) -> ToolResult {
    tracing::debug!("tool: crystallize_path");

    if args.node_ids.is_empty() {
        return ToolResult::error("node_ids must not be empty");
    }

    let size = args.size.unwrap_or(10.0);
    let count = args.count.unwrap_or(3).max(1);

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let mut commands = Vec::new();
    let mut modified = 0usize;
    let mut skipped = 0usize;

    for node_id_str in &args.node_ids {
        let nid = match uuid::Uuid::parse_str(node_id_str) {
            Ok(id) => id,
            Err(_) => match doc.find_node_by_name(node_id_str) {
                Some(n) => n.id,
                None => {
                    skipped += 1;
                    continue;
                }
            },
        };
        let node = match doc.nodes.get(&nid) {
            Some(n) => n.clone(),
            None => {
                skipped += 1;
                continue;
            }
        };
        let pn = match &node.kind {
            SceneNodeKind::Path(pn) => pn,
            _ => {
                skipped += 1;
                continue;
            }
        };

        let bez = pn.path_data.to_bez_path();
        let new_bez = apply_crystallize(&bez, size, count);

        let mut new_node = node.clone();
        if let SceneNodeKind::Path(ref mut new_pn) = new_node.kind {
            new_pn.path_data = PathData::from_bez_path(&new_bez);
        }
        commands.push(Command::UpdateNode {
            old: node,
            new: new_node,
        });
        modified += 1;
    }

    if commands.is_empty() {
        return ToolResult::error("No path nodes found in node_ids");
    }

    for cmd in commands {
        history.execute(cmd, &mut doc);
    }

    ToolResult::text(format!(
        "Applied crystallize to {} node(s) (size={size}, count={count}){}",
        modified,
        if skipped > 0 {
            format!(" — {skipped} skipped")
        } else {
            String::new()
        },
    ))
    .with_data(serde_json::json!({ "modified": modified, "skipped": skipped }))
}

/// Add sharp outward spikes along each segment.
fn apply_crystallize(bez: &kurbo::BezPath, size: f64, count: usize) -> kurbo::BezPath {
    let mut result = kurbo::BezPath::new();
    let mut current = kurbo::Point::ZERO;
    let mut subpath_start = kurbo::Point::ZERO;

    for el in bez.elements() {
        match *el {
            kurbo::PathEl::MoveTo(p) => {
                result.move_to(p);
                current = p;
                subpath_start = p;
            }
            kurbo::PathEl::ClosePath => {
                if current != subpath_start {
                    crystallize_segment(&mut result, current, subpath_start, size, count);
                }
                result.close_path();
                current = subpath_start;
            }
            _ => {
                let endpoint = match *el {
                    kurbo::PathEl::LineTo(p)
                    | kurbo::PathEl::CurveTo(_, _, p)
                    | kurbo::PathEl::QuadTo(_, p) => p,
                    _ => unreachable!(),
                };
                let start = {
                    let els = result.elements();
                    let mut pt = kurbo::Point::ZERO;
                    for e in els.iter().rev() {
                        match e {
                            kurbo::PathEl::MoveTo(p)
                            | kurbo::PathEl::LineTo(p)
                            | kurbo::PathEl::CurveTo(_, _, p)
                            | kurbo::PathEl::QuadTo(_, p) => {
                                pt = *p;
                                break;
                            }
                            kurbo::PathEl::ClosePath => {}
                        }
                    }
                    pt
                };
                crystallize_segment(&mut result, start, endpoint, size, count);
                current = endpoint;
            }
        }
    }
    result
}

/// Emit sharp triangular spikes between `from` and `to`.
fn crystallize_segment(
    path: &mut kurbo::BezPath,
    from: kurbo::Point,
    to: kurbo::Point,
    size: f64,
    count: usize,
) {
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1e-9 {
        path.line_to(to);
        return;
    }

    // Normal points outward (opposite to scallop).
    let nx = -dy / len;
    let ny = dx / len;

    // Each spike is a triangle: base_start → peak → base_end.
    for i in 0..count {
        let t_peak = (i as f64 + 0.5) / count as f64;
        let t_base_end = (i + 1) as f64 / count as f64;

        // Spike peak displaced outward.
        let peak = kurbo::Point::new(
            from.x + dx * t_peak + nx * size,
            from.y + dy * t_peak + ny * size,
        );
        let base_end = kurbo::Point::new(from.x + dx * t_base_end, from.y + dy * t_base_end);

        path.line_to(peak);
        path.line_to(base_end);
    }
}

fn solid_fill_of(fill: &photonic_core::style::Fill) -> Option<photonic_core::color::Color> {
    match &fill.kind {
        photonic_core::style::FillKind::Solid(c) => Some(*c),
        _ => None,
    }
}

/// Remove degenerate content from the document:
/// - stray points: paths with no drawing segments (only MoveTo or empty)
/// - unpainted objects: paths with no visible fill and no visible stroke
/// - empty text: text nodes with whitespace-only content
pub async fn clean_up(state: &AppState, args: CleanUpArgs) -> ToolResult {
    use kurbo::PathEl;
    use photonic_core::style::FillKind;

    tracing::debug!("tool: clean_up");

    let remove_stray = args.remove_stray_points.unwrap_or(true);
    let remove_unpaint = args.remove_unpainted.unwrap_or(true);
    let remove_empty = args.remove_empty_text.unwrap_or(true);
    let dry_run = args.dry_run.unwrap_or(false);

    // ── Phase 1: identify nodes to remove (read-only, single lock acquisition) ──
    let to_delete: Vec<(NodeId, &'static str)> = {
        let doc = state.document.lock().await;
        let mut found: Vec<(NodeId, &'static str)> = Vec::new();

        for node in doc.nodes.values() {
            match &node.kind {
                SceneNodeKind::Path(path_node) => {
                    // Stray point: path with no drawing segments
                    if remove_stray {
                        let bez = path_node.path_data.to_bez_path();
                        let has_segment = bez.elements().iter().any(|el| {
                            matches!(
                                el,
                                PathEl::LineTo(_) | PathEl::CurveTo(..) | PathEl::QuadTo(..)
                            )
                        });
                        if !has_segment {
                            found.push((node.id, "stray_point"));
                            continue;
                        }
                    }
                    // Unpainted: no visible fill and no visible stroke
                    if remove_unpaint {
                        let has_fill = path_node.fill.enabled
                            && !matches!(path_node.fill.kind, FillKind::None)
                            && path_node.fill.opacity > 0.0;
                        let has_stroke = path_node.stroke.enabled
                            && path_node.stroke.width > 0.0
                            && path_node.stroke.opacity > 0.0;
                        if !has_fill && !has_stroke {
                            found.push((node.id, "unpainted"));
                        }
                    }
                }
                SceneNodeKind::Text(text_node) => {
                    if remove_empty && text_node.content.trim().is_empty() {
                        found.push((node.id, "empty_text"));
                    }
                }
                SceneNodeKind::Group(_) => {}
            }
        }
        found
    }; // doc lock released

    let count = to_delete.len();
    let items: Vec<serde_json::Value> = to_delete
        .iter()
        .map(|(id, reason)| serde_json::json!({ "id": id, "reason": reason }))
        .collect();

    if count == 0 {
        return ToolResult::text("Nothing to clean up").with_data(serde_json::json!({
            "dry_run": dry_run,
            "removed": 0,
            "items":   [],
        }));
    }

    if dry_run {
        return ToolResult::text(format!("Dry run — {} node(s) would be removed", count))
            .with_data(serde_json::json!({
                "dry_run":      true,
                "would_remove": count,
                "items":        items,
            }));
    }

    // ── Phase 2: delete (acquire both locks) ─────────────────────────────────
    let ids: Vec<NodeId> = to_delete.iter().map(|(id, _)| *id).collect();
    let cmd = Command::Batch(
        ids.iter()
            .map(|&node_id| Command::RemoveNode { node_id })
            .collect(),
    );
    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    history.execute(cmd, &mut doc);

    ToolResult::text(format!("Cleaned up {} node(s)", count)).with_data(serde_json::json!({
        "dry_run": false,
        "removed": count,
        "items":   items,
    }))
}

// ── simplify_path ─────────────────────────────────────────────────────────────

pub async fn simplify_path(state: &AppState, args: SimplifyPathArgs) -> ToolResult {
    use photonic_core::ops::simplify::{count_points, simplify_path as do_simplify};

    if args.tolerance <= 0.0 {
        return ToolResult::error("tolerance must be > 0");
    }

    let mut doc = state.document.lock().await;
    let old_node = match doc.get_node(&args.node_id) {
        Some(n) => n.clone(),
        None => return ToolResult::error(format!("Node not found: {}", args.node_id)),
    };

    let path_node = match &old_node.kind {
        SceneNodeKind::Path(p) => p.clone(),
        _ => return ToolResult::error("Node must be a path node"),
    };

    let original_count = count_points(&path_node.path_data);
    let simplified_data = do_simplify(&path_node.path_data, args.tolerance);
    let simplified_count = count_points(&simplified_data);
    let pct = 100.0 * (1.0 - simplified_count as f64 / original_count.max(1) as f64);

    if args.dry_run {
        return ToolResult::text(format!(
            "Dry run: '{}' — {} points → {} points ({:.0}% reduction)",
            old_node.name, original_count, simplified_count, pct
        ))
        .with_data(serde_json::json!({
            "node_id": args.node_id,
            "node_name": old_node.name,
            "original_points": original_count,
            "simplified_points": simplified_count,
            "applied": false,
        }));
    }

    let mut new_path_node = PathNode::new(simplified_data);
    new_path_node.fill = path_node.fill.clone();
    new_path_node.stroke = path_node.stroke.clone();
    new_path_node.is_compound = path_node.is_compound;

    let mut new_node = old_node.clone();
    new_node.kind = SceneNodeKind::Path(new_path_node);

    let cmd = Command::UpdateNode {
        old: old_node,
        new: new_node.clone(),
    };
    let mut history = state.history.lock().await;
    history.execute(cmd, &mut doc);

    ToolResult::text(format!(
        "Simplified '{}': {} → {} points ({:.0}% reduction)",
        new_node.name, original_count, simplified_count, pct
    ))
    .with_data(serde_json::json!({
        "node_id": args.node_id,
        "node_name": new_node.name,
        "original_points": original_count,
        "simplified_points": simplified_count,
        "applied": true,
    }))
}

// ── invert_colors ─────────────────────────────────────────────────────────────

pub async fn invert_colors(state: &AppState, args: InvertColorsArgs) -> ToolResult {
    use photonic_core::style::FillKind;

    // 1. Collect candidate path nodes
    let candidates: Vec<SceneNode> = {
        let doc = state.document.lock().await;
        match &args.node_ids {
            Some(ids) => ids
                .iter()
                .filter_map(|id| doc.nodes.get(id).cloned())
                .collect(),
            None => doc
                .nodes
                .values()
                .filter(|n| matches!(n.kind, SceneNodeKind::Path(_)))
                .cloned()
                .collect(),
        }
    };

    if candidates.is_empty() {
        return ToolResult::text("No path nodes found to invert.");
    }

    // 2. Build UpdateNode commands
    let mut commands: Vec<Command> = Vec::new();
    let mut count = 0usize;

    for node in &candidates {
        let mut new_node = node.clone();
        let mut modified = false;

        match &mut new_node.kind {
            SceneNodeKind::Path(path) => {
                match &mut path.fill.kind {
                    FillKind::Solid(c) => *c = c.invert(),
                    FillKind::Gradient(g) => {
                        for stop in &mut g.stops {
                            stop.color = stop.color.invert();
                        }
                    }
                    FillKind::FluidGradient(fg) => {
                        for pt in &mut fg.points {
                            pt.color = pt.color.invert();
                        }
                    }
                    FillKind::MeshGradient(mg) => {
                        for v in &mut mg.vertices {
                            v.color = v.color.invert();
                        }
                    }
                    FillKind::None => {}
                }
                if path.stroke.enabled {
                    path.stroke.color = path.stroke.color.invert();
                }
                modified = true;
            }
            _ => {}
        }

        if modified {
            commands.push(Command::UpdateNode {
                old: node.clone(),
                new: new_node,
            });
            count += 1;
        }
    }

    if count == 0 {
        return ToolResult::text("Selected nodes contain no path nodes.");
    }

    // 3. Execute as a single undo-able batch
    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    history.execute(Command::Batch(commands), &mut doc);
    history.schedule_mcp_checkpoint(format!("Invert colors ({} nodes)", count));

    ToolResult::text(format!("Inverted colors on {} node(s).", count))
}

// ─── adjust_colors ─────────────────────────────────────────────────────────────

/// Shift RGB(A) channel values across selected artwork.
/// Each channel delta is added to the existing value and clamped to [0, 1].
pub async fn adjust_colors(state: &AppState, args: AdjustColorsArgs) -> ToolResult {
    use photonic_core::style::FillKind;

    let dr = args.delta_r;
    let dg = args.delta_g;
    let db = args.delta_b;
    let da = args.delta_a;

    if dr == 0.0 && dg == 0.0 && db == 0.0 && da == 0.0 {
        return ToolResult::text("No channel deltas specified; nothing to adjust.");
    }

    let shift_color = |c: photonic_core::Color| -> photonic_core::Color {
        photonic_core::Color {
            r: (c.r + dr).clamp(0.0, 1.0),
            g: (c.g + dg).clamp(0.0, 1.0),
            b: (c.b + db).clamp(0.0, 1.0),
            a: (c.a + da).clamp(0.0, 1.0),
        }
    };

    let candidates: Vec<SceneNode> = {
        let doc = state.document.lock().await;
        match &args.node_ids {
            Some(ids) => ids
                .iter()
                .filter_map(|id| doc.nodes.get(id).cloned())
                .collect(),
            None => doc
                .nodes
                .values()
                .filter(|n| matches!(n.kind, SceneNodeKind::Path(_)))
                .cloned()
                .collect(),
        }
    };

    if candidates.is_empty() {
        return ToolResult::text("No path nodes found to adjust.");
    }

    let mut commands: Vec<Command> = Vec::new();
    let mut count = 0usize;

    for node in &candidates {
        let mut new_node = node.clone();
        if let SceneNodeKind::Path(path) = &mut new_node.kind {
            match &mut path.fill.kind {
                FillKind::Solid(c) => *c = shift_color(*c),
                FillKind::Gradient(g) => {
                    for stop in &mut g.stops {
                        stop.color = shift_color(stop.color);
                    }
                }
                FillKind::FluidGradient(fg) => {
                    for pt in &mut fg.points {
                        pt.color = shift_color(pt.color);
                    }
                }
                FillKind::MeshGradient(mg) => {
                    for v in &mut mg.vertices {
                        v.color = shift_color(v.color);
                    }
                }
                FillKind::None => {}
            }
            if path.stroke.enabled {
                path.stroke.color = shift_color(path.stroke.color);
            }
            commands.push(Command::UpdateNode {
                old: node.clone(),
                new: new_node,
            });
            count += 1;
        }
    }

    if count == 0 {
        return ToolResult::text("Selected nodes contain no path nodes.");
    }

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    history.execute(Command::Batch(commands), &mut doc);
    history.schedule_mcp_checkpoint(format!("Adjust colors ({} nodes)", count));

    ToolResult::text(format!("Adjusted colors on {} node(s).", count)).with_data(
        serde_json::json!({
            "modified_count": count,
            "delta_r": dr, "delta_g": dg, "delta_b": db, "delta_a": da,
        }),
    )
}

pub async fn convert_to_grayscale(state: &AppState, args: ConvertToGrayscaleArgs) -> ToolResult {
    use photonic_core::style::FillKind;

    // 1. Collect candidate path nodes
    let candidates: Vec<SceneNode> = {
        let doc = state.document.lock().await;
        match &args.node_ids {
            Some(ids) => ids
                .iter()
                .filter_map(|id| doc.nodes.get(id).cloned())
                .collect(),
            None => doc
                .nodes
                .values()
                .filter(|n| matches!(n.kind, SceneNodeKind::Path(_)))
                .cloned()
                .collect(),
        }
    };

    if candidates.is_empty() {
        return ToolResult::text("No path nodes found to convert.");
    }

    // 2. Build UpdateNode commands
    let mut commands: Vec<Command> = Vec::new();
    let mut count = 0usize;

    for node in &candidates {
        let mut new_node = node.clone();
        let mut modified = false;

        match &mut new_node.kind {
            SceneNodeKind::Path(path) => {
                match &mut path.fill.kind {
                    FillKind::Solid(c) => *c = c.to_grayscale(),
                    FillKind::Gradient(g) => {
                        for stop in &mut g.stops {
                            stop.color = stop.color.to_grayscale();
                        }
                    }
                    FillKind::FluidGradient(fg) => {
                        for pt in &mut fg.points {
                            pt.color = pt.color.to_grayscale();
                        }
                    }
                    FillKind::MeshGradient(mg) => {
                        for v in &mut mg.vertices {
                            v.color = v.color.to_grayscale();
                        }
                    }
                    FillKind::None => {}
                }
                if path.stroke.enabled {
                    path.stroke.color = path.stroke.color.to_grayscale();
                }
                modified = true;
            }
            _ => {}
        }

        if modified {
            commands.push(Command::UpdateNode {
                old: node.clone(),
                new: new_node,
            });
            count += 1;
        }
    }

    if count == 0 {
        return ToolResult::text("Selected nodes contain no path nodes.");
    }

    // 3. Execute as a single undo-able batch
    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    history.execute(Command::Batch(commands), &mut doc);
    history.schedule_mcp_checkpoint(format!("Convert to grayscale ({} nodes)", count));

    ToolResult::text(format!("Converted {} node(s) to grayscale.", count))
}

/// Reverse the winding direction of path node(s). Non-path nodes are silently skipped.
pub async fn reverse_path_direction(
    state: &AppState,
    args: ReversePathDirectionArgs,
) -> ToolResult {
    if args.node_ids.is_empty() {
        return ToolResult::error("node_ids must not be empty");
    }

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let mut commands = Vec::new();
    let mut modified = 0usize;
    let mut skipped = 0usize;

    for node_id in &args.node_ids {
        let node = match doc.nodes.get(node_id) {
            Some(n) => n.clone(),
            None => {
                skipped += 1;
                continue;
            }
        };

        match &node.kind {
            SceneNodeKind::Path(pn) => {
                let new_path = pn.path_data.reverse();
                let mut new_node = node.clone();
                if let SceneNodeKind::Path(ref mut new_pn) = new_node.kind {
                    new_pn.path_data = new_path;
                }
                commands.push(Command::UpdateNode {
                    old: node,
                    new: new_node,
                });
                modified += 1;
            }
            _ => {
                skipped += 1;
            }
        }
    }

    if commands.is_empty() {
        return ToolResult::error("No path nodes found in node_ids");
    }

    for cmd in commands {
        history.execute(cmd, &mut doc);
    }

    let summary = format!(
        "Reversed path direction on {} node(s){}",
        modified,
        if skipped > 0 {
            format!(" — {} non-path node(s) skipped", skipped)
        } else {
            String::new()
        },
    );
    ToolResult::text(summary).with_data(serde_json::json!({
        "modified": modified,
        "skipped":  skipped,
    }))
}

// ── average_anchor_points ───────────────────────────────────────────────────────

pub async fn average_anchor_points(state: &AppState, args: AverageAnchorPointsArgs) -> ToolResult {
    if args.node_ids.is_empty() {
        return ToolResult::error("node_ids must not be empty");
    }

    let (avg_x, avg_y) = match args.axis.as_deref().unwrap_or("both") {
        "horizontal" => (true, false),
        "vertical" => (false, true),
        _ => (true, true), // "both" or any unrecognised value
    };

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let mut commands = Vec::new();
    let mut modified = 0usize;
    let mut skipped = 0usize;

    for node_id in &args.node_ids {
        let node = match doc.nodes.get(node_id) {
            Some(n) => n.clone(),
            None => {
                skipped += 1;
                continue;
            }
        };

        let pn = match &node.kind {
            SceneNodeKind::Path(p) => p.clone(),
            _ => {
                skipped += 1;
                continue;
            }
        };

        let new_path = pn.path_data.average_anchor_points(avg_x, avg_y);
        let mut new_node = node.clone();
        if let SceneNodeKind::Path(ref mut new_pn) = new_node.kind {
            new_pn.path_data = new_path;
        }
        commands.push(Command::UpdateNode {
            old: node,
            new: new_node,
        });
        modified += 1;
    }

    if commands.is_empty() {
        return ToolResult::error("No path nodes found in node_ids");
    }

    for cmd in commands {
        history.execute(cmd, &mut doc);
    }

    let summary = format!(
        "Averaged anchor points on {} node(s){}",
        modified,
        if skipped > 0 {
            format!(" — {} non-path node(s) skipped", skipped)
        } else {
            String::new()
        },
    );
    ToolResult::text(summary).with_data(serde_json::json!({
        "modified": modified,
        "skipped":  skipped,
        "axis":     args.axis.as_deref().unwrap_or("both"),
    }))
}

// ── outline_stroke ─────────────────────────────────────────────────────────────

pub async fn outline_stroke(state: &AppState, args: OutlineStrokeArgs) -> ToolResult {
    use photonic_core::ops::stroke_outline::outline_stroke as do_outline;
    use photonic_core::style::{Fill, FillKind, Stroke};

    if args.node_ids.is_empty() {
        return ToolResult::error("node_ids must not be empty");
    }

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let mut commands = Vec::new();
    let mut outlined_ids = Vec::new();
    let mut original_ids = Vec::new();
    let mut skipped = 0usize;

    for node_id in &args.node_ids {
        let node = match doc.nodes.get(node_id) {
            Some(n) => n.clone(),
            None => {
                skipped += 1;
                continue;
            }
        };

        let pn = match &node.kind {
            SceneNodeKind::Path(p) => p.clone(),
            _ => {
                skipped += 1;
                continue;
            }
        };

        let outline_data = match do_outline(&pn.path_data, &pn.stroke) {
            Ok(d) => d,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };

        let layer_id = node.layer_id;

        // Build outlined path node: fill = stroke color, stroke disabled.
        let mut outline_pn = PathNode::new(outline_data);
        outline_pn.fill = Fill {
            kind: FillKind::Solid(pn.stroke.color),
            opacity: pn.stroke.opacity,
            enabled: true,
        };
        outline_pn.stroke = Stroke::none();

        let outline_node = SceneNode::new(
            &format!("{} outline", node.name),
            layer_id,
            SceneNodeKind::Path(outline_pn),
        );
        let outline_id = outline_node.id;

        // Disable stroke on the original node.
        let mut updated_orig = node.clone();
        if let SceneNodeKind::Path(ref mut op) = updated_orig.kind {
            op.stroke.enabled = false;
        }

        commands.push(Command::Batch(vec![
            Command::AddNode {
                node: outline_node,
                layer_id: Some(layer_id),
            },
            Command::UpdateNode {
                old: node.clone(),
                new: updated_orig,
            },
        ]));

        outlined_ids.push(outline_id.to_string());
        original_ids.push(node_id.to_string());
    }

    if commands.is_empty() {
        return ToolResult::error(
            "No eligible path nodes found — each node must be a path with an enabled stroke",
        );
    }

    let modified = outlined_ids.len();
    let batch = if commands.len() == 1 {
        commands.remove(0)
    } else {
        Command::Batch(commands)
    };
    history.execute(batch, &mut doc);

    let summary = format!(
        "Outlined stroke on {} node(s){}",
        modified,
        if skipped > 0 {
            format!(" — {} node(s) skipped", skipped)
        } else {
            String::new()
        },
    );
    ToolResult::text(summary).with_data(serde_json::json!({
        "outlined_ids": outlined_ids,
        "original_ids": original_ids,
        "skipped":      skipped,
    }))
}

/// Offset (expand or inset) one or more path nodes by a fixed distance.
pub async fn offset_path(state: &AppState, args: OffsetPathArgs) -> ToolResult {
    use kurbo::Join;
    use photonic_core::ops::offset::offset_path as do_offset;

    if args.node_ids.is_empty() {
        return ToolResult::error("node_ids must not be empty");
    }

    let join = match args.join_style.as_deref().unwrap_or("miter") {
        "round" => Join::Round,
        "bevel" => Join::Bevel,
        _ => Join::Miter,
    };
    let create_copy = args.create_copy.unwrap_or(true);

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let mut commands = Vec::new();
    let mut processed = Vec::new();
    let mut skipped = 0usize;

    for node_id in &args.node_ids {
        let node = match doc.nodes.get(node_id) {
            Some(n) => n.clone(),
            None => {
                skipped += 1;
                continue;
            }
        };

        let pn = match &node.kind {
            SceneNodeKind::Path(p) => p.clone(),
            _ => {
                skipped += 1;
                continue;
            }
        };

        let offset_data = match do_offset(&pn.path_data, args.distance, join) {
            Ok(d) => d,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };

        let layer_id = node.layer_id;

        if create_copy {
            let mut new_pn = pn.clone();
            new_pn.path_data = offset_data;
            let new_node = SceneNode::new(
                &format!("{} offset", node.name),
                layer_id,
                SceneNodeKind::Path(new_pn),
            );
            let new_id = new_node.id.to_string();
            commands.push(Command::AddNode {
                node: new_node,
                layer_id: Some(layer_id),
            });
            processed.push(new_id);
        } else {
            let mut new_node = node.clone();
            if let SceneNodeKind::Path(ref mut p) = new_node.kind {
                p.path_data = offset_data;
            }
            commands.push(Command::UpdateNode {
                old: node,
                new: new_node,
            });
            processed.push(node_id.to_string());
        }
    }

    if commands.is_empty() {
        return ToolResult::error("No path nodes found or all offsets failed (path may be too small to inset by this distance)");
    }

    let batch = if commands.len() == 1 {
        commands.remove(0)
    } else {
        Command::Batch(commands)
    };
    history.execute(batch, &mut doc);

    ToolResult::text(format!(
        "Offset {} path(s) by {}{:.1} units{}",
        processed.len(),
        if args.distance >= 0.0 { "+" } else { "" },
        args.distance,
        if skipped > 0 {
            format!(" — {} node(s) skipped", skipped)
        } else {
            String::new()
        },
    ))
    .with_data(serde_json::json!({
        "processed": processed,
        "skipped":   skipped,
    }))
}

// ─── split_into_grid ─────────────────────────────────────────────────────────

/// Divide a path node's bounding box into a rows×cols grid of rectangle nodes.
pub async fn split_into_grid(state: &AppState, args: SplitIntoGridArgs) -> ToolResult {
    if args.rows == 0 {
        return ToolResult::error("rows must be ≥ 1");
    }
    if args.cols == 0 {
        return ToolResult::error("cols must be ≥ 1");
    }

    let mut doc = state.document.lock().await;

    // Read source node.
    let source = match doc.nodes.get(&args.node_id) {
        Some(n) => n.clone(),
        None => return ToolResult::error(format!("node {} not found", args.node_id)),
    };

    // Source must be a path.
    let path_node = match &source.kind {
        SceneNodeKind::Path(p) => p.clone(),
        _ => return ToolResult::error("split_into_grid requires a path node"),
    };

    // Get local bounding box.
    let local_bbox = match path_node.path_data.bounding_box() {
        Some(b) => b,
        None => return ToolResult::error("source path has no computable bounding box"),
    };

    // Apply the source node's transform to the four corners to get world-space bounds.
    let t = &source.transform;
    let corners = [
        t.apply(local_bbox.x0, local_bbox.y0),
        t.apply(local_bbox.x1, local_bbox.y0),
        t.apply(local_bbox.x0, local_bbox.y1),
        t.apply(local_bbox.x1, local_bbox.y1),
    ];
    let min_x = corners.iter().map(|c| c.0).fold(f64::INFINITY, f64::min);
    let min_y = corners.iter().map(|c| c.1).fold(f64::INFINITY, f64::min);
    let max_x = corners
        .iter()
        .map(|c| c.0)
        .fold(f64::NEG_INFINITY, f64::max);
    let max_y = corners
        .iter()
        .map(|c| c.1)
        .fold(f64::NEG_INFINITY, f64::max);
    let total_w = max_x - min_x;
    let total_h = max_y - min_y;

    let gx = args.gutter_x.unwrap_or(0.0).max(0.0);
    let gy = args.gutter_y.unwrap_or(0.0).max(0.0);

    let cell_w = (total_w - gx * (args.cols as f64 - 1.0)) / args.cols as f64;
    let cell_h = (total_h - gy * (args.rows as f64 - 1.0)) / args.rows as f64;

    if cell_w <= 0.0 {
        return ToolResult::error(format!(
            "gutter_x ({gx}) is too large — cells would have non-positive width ({cell_w:.2})"
        ));
    }
    if cell_h <= 0.0 {
        return ToolResult::error(format!(
            "gutter_y ({gy}) is too large — cells would have non-positive height ({cell_h:.2})"
        ));
    }

    let target_layer = args.layer_id.unwrap_or(source.layer_id);
    let keep = args.keep_original.unwrap_or(false);
    let source_name = source.name.clone();

    let mut commands: Vec<Command> = Vec::new();
    let mut created_ids: Vec<uuid::Uuid> = Vec::new();

    for r in 0..args.rows {
        for c in 0..args.cols {
            let x = min_x + c as f64 * (cell_w + gx);
            let y = min_y + r as f64 * (cell_h + gy);

            let pd = PathData::rect(x, y, cell_w, cell_h);
            let mut cell_pn = PathNode::new(pd);
            cell_pn.fill = path_node.fill.clone();
            cell_pn.stroke = path_node.stroke.clone();

            let cell_name = format!("{} {},{}", source_name, r + 1, c + 1);
            let mut cell_node =
                SceneNode::new(&cell_name, target_layer, SceneNodeKind::Path(cell_pn));
            cell_node.opacity = source.opacity;
            cell_node.blend_mode = source.blend_mode;
            cell_node.tags = source.tags.clone();

            created_ids.push(cell_node.id);
            commands.push(Command::AddNode {
                node: cell_node,
                layer_id: Some(target_layer),
            });
        }
    }

    if !keep {
        commands.push(Command::RemoveNode {
            node_id: args.node_id,
        });
    }

    let batch = Command::Batch(commands);
    let mut history = state.history.lock().await;
    history.execute(batch, &mut doc);

    let count = created_ids.len();
    ToolResult::text(format!(
        "Split into {}×{} grid — created {} rectangle{} from \"{}\"{}",
        args.rows,
        args.cols,
        count,
        if count == 1 { "" } else { "s" },
        source_name,
        if keep {
            " (original kept)"
        } else {
            " (original removed)"
        },
    ))
    .with_data(serde_json::json!({
        "created": created_ids,
        "rows": args.rows,
        "cols": args.cols,
        "cell_width":  cell_w,
        "cell_height": cell_h,
    }))
}

// ─── blend_colors ─────────────────────────────────────────────────────────────

/// Distribute fill colors linearly across a set of path nodes.
/// The first and last nodes keep their solid fill colors; intermediate nodes
/// receive interpolated colors at evenly spaced positions along the range.
pub async fn blend_colors(state: &AppState, args: BlendColorsArgs) -> ToolResult {
    use photonic_core::style::FillKind;
    use photonic_core::Color;

    if args.node_ids.len() < 2 {
        return ToolResult::error("blend_colors requires at least 2 node_ids");
    }

    // 1. Collect nodes and validate they are all path nodes, then optionally sort.
    let nodes: Vec<SceneNode> = {
        let doc = state.document.lock().await;

        let mut out: Vec<SceneNode> = Vec::new();
        for &id in &args.node_ids {
            match doc.nodes.get(&id) {
                Some(n) => out.push(n.clone()),
                None => return ToolResult::error(format!("Node {} not found", id)),
            }
        }

        for n in &out {
            if !matches!(n.kind, SceneNodeKind::Path(_)) {
                return ToolResult::error(format!("Node '{}' is not a path node", n.name));
            }
        }

        // Sort by the requested direction.
        if let Some(dir) = &args.direction {
            match dir.as_str() {
                "horizontal" => {
                    out.sort_by(|a, b| {
                        let ax = path_center_x(a);
                        let bx = path_center_x(b);
                        ax.partial_cmp(&bx).unwrap_or(std::cmp::Ordering::Equal)
                    });
                }
                "vertical" => {
                    out.sort_by(|a, b| {
                        let ay = path_center_y(a);
                        let by_ = path_center_y(b);
                        ay.partial_cmp(&by_).unwrap_or(std::cmp::Ordering::Equal)
                    });
                }
                "depth" => {
                    // Build a global z-index from layer order.
                    let mut z_index: std::collections::HashMap<photonic_core::NodeId, usize> =
                        std::collections::HashMap::new();
                    let mut z = 0usize;
                    for layer_id in &doc.layer_order {
                        if let Some(layer) = doc.layers.get(layer_id) {
                            for &nid in &layer.node_ids {
                                z_index.insert(nid, z);
                                z += 1;
                            }
                        }
                    }
                    out.sort_by_key(|n| z_index.get(&n.id).copied().unwrap_or(0));
                }
                other => {
                    return ToolResult::error(format!(
                        "Unknown direction '{}'; use 'horizontal', 'vertical', or 'depth'",
                        other
                    ));
                }
            }
        }

        out
    };

    // 2. Extract solid fill colors from the first and last nodes.
    let start_color = match &nodes[0].kind {
        SceneNodeKind::Path(p) => match &p.fill.kind {
            FillKind::Solid(c) => *c,
            _ => return ToolResult::error("First node must have a solid fill for blending"),
        },
        _ => unreachable!(),
    };
    let end_color = match &nodes[nodes.len() - 1].kind {
        SceneNodeKind::Path(p) => match &p.fill.kind {
            FillKind::Solid(c) => *c,
            _ => return ToolResult::error("Last node must have a solid fill for blending"),
        },
        _ => unreachable!(),
    };

    // 3. Build UpdateNode commands for intermediate nodes only.
    let n = nodes.len();
    let mut commands: Vec<Command> = Vec::new();

    for (i, node) in nodes.iter().enumerate() {
        if i == 0 || i == n - 1 {
            continue; // endpoints keep their own colors
        }
        let t = i as f32 / (n - 1) as f32;
        let blended = Color {
            r: start_color.r + t * (end_color.r - start_color.r),
            g: start_color.g + t * (end_color.g - start_color.g),
            b: start_color.b + t * (end_color.b - start_color.b),
            a: start_color.a + t * (end_color.a - start_color.a),
        };
        let mut new_node = node.clone();
        if let SceneNodeKind::Path(ref mut p) = new_node.kind {
            p.fill.kind = FillKind::Solid(blended);
        }
        commands.push(Command::UpdateNode {
            old: node.clone(),
            new: new_node,
        });
    }

    if commands.is_empty() {
        return ToolResult::text(
            "No intermediate nodes to update (need at least 3 nodes to interpolate).",
        );
    }

    let updated = commands.len();
    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    history.execute(Command::Batch(commands), &mut doc);
    history.schedule_mcp_checkpoint(format!("Blend colors ({} nodes)", n));

    ToolResult::text(format!(
        "Blended colors across {} nodes ({} intermediate node(s) updated).",
        n, updated
    ))
    .with_data(serde_json::json!({
        "start_color": start_color.to_hex(),
        "end_color":   end_color.to_hex(),
        "node_count":  n,
        "updated_count": updated,
    }))
}

// ─── join_paths ───────────────────────────────────────────────────────────────

/// Close or join path nodes.
///
/// * **1 node_id** — appends `ClosePath` to every open subpath in the node.
/// * **2 node_ids** — merges the two paths into one by connecting their nearest
///   open endpoints with a straight line; the result replaces the first node
///   and the second node is deleted.
pub async fn join_paths(state: &AppState, args: JoinPathsArgs) -> ToolResult {
    use photonic_core::ops::join::{close_open_paths, join_two_paths};

    let n = args.node_ids.len();
    if n == 0 || n > 2 {
        return ToolResult::error("node_ids must contain 1 or 2 path node IDs");
    }

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    if n == 1 {
        // ── Close a single path ──────────────────────────────────────────────
        let nid = args.node_ids[0];
        let node = match doc.nodes.get(&nid) {
            Some(n) => n.clone(),
            None => return ToolResult::error(format!("node {} not found", nid)),
        };
        let pn = match &node.kind {
            SceneNodeKind::Path(p) => p.clone(),
            _ => return ToolResult::error("node is not a path node"),
        };

        let new_path = close_open_paths(&pn.path_data);
        let mut new_node = node.clone();
        if let SceneNodeKind::Path(ref mut new_pn) = new_node.kind {
            new_pn.path_data = new_path;
        }
        history.execute(
            Command::UpdateNode {
                old: node,
                new: new_node.clone(),
            },
            &mut doc,
        );

        ToolResult::text("Closed open subpaths.").with_data(serde_json::json!({
            "operation":  "closed",
            "result_id":  new_node.id,
        }))
    } else {
        // ── Join two paths ───────────────────────────────────────────────────
        let id_a = args.node_ids[0];
        let id_b = args.node_ids[1];

        let node_a = match doc.nodes.get(&id_a) {
            Some(n) => n.clone(),
            None => return ToolResult::error(format!("node {} not found", id_a)),
        };
        let node_b = match doc.nodes.get(&id_b) {
            Some(n) => n.clone(),
            None => return ToolResult::error(format!("node {} not found", id_b)),
        };

        let pn_a = match &node_a.kind {
            SceneNodeKind::Path(p) => p.clone(),
            _ => return ToolResult::error(format!("node {} is not a path node", id_a)),
        };
        let pn_b = match &node_b.kind {
            SceneNodeKind::Path(p) => p.clone(),
            _ => return ToolResult::error(format!("node {} is not a path node", id_b)),
        };

        let merged = join_two_paths(&pn_a.path_data, &pn_b.path_data);
        let mut result_node = node_a.clone();
        if let SceneNodeKind::Path(ref mut new_pn) = result_node.kind {
            new_pn.path_data = merged;
        }

        history.execute(
            Command::Batch(vec![
                Command::UpdateNode {
                    old: node_a,
                    new: result_node.clone(),
                },
                Command::RemoveNode { node_id: id_b },
            ]),
            &mut doc,
        );

        ToolResult::text("Joined two paths into one.").with_data(serde_json::json!({
            "operation":  "joined",
            "result_id":  result_node.id,
            "removed_id": id_b,
        }))
    }
}

// ─── pathfinder_crop ─────────────────────────────────────────────────────────

/// Clip all selected paths to the boundary of the frontmost selected node.
///
/// The frontmost node (highest z-order) acts as the crop mask: every other
/// selected path is replaced by `path ∩ frontmost_path`. The frontmost node
/// itself is removed. All transforms are baked into path coordinates before
/// the intersection so that results are correct regardless of node transform.
pub async fn pathfinder_crop(state: &AppState, args: PathfinderCropArgs) -> ToolResult {
    use photonic_core::ops::boolean::{boolean_op, BooleanOp};
    use photonic_core::ops::transform_ops::apply_affine_to_path;

    if args.node_ids.len() < 2 {
        return ToolResult::error("node_ids must contain at least 2 path node IDs");
    }

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    // ── Verify all nodes exist and are paths ─────────────────────────────────
    for nid in &args.node_ids {
        let node = match doc.nodes.get(nid) {
            Some(n) => n,
            None => return ToolResult::error(format!("node {} not found", nid)),
        };
        if !matches!(node.kind, SceneNodeKind::Path(_)) {
            return ToolResult::error(format!("node {} is not a path node", nid));
        }
    }

    // ── Determine z-order and find frontmost ─────────────────────────────────
    let frontmost_id = {
        let mut best_id = args.node_ids[0];
        let mut best_key = node_z_key(&doc, &best_id);
        for nid in &args.node_ids[1..] {
            let key = node_z_key(&doc, nid);
            if key > best_key {
                best_key = key;
                best_id = *nid;
            }
        }
        best_id
    };

    // ── Bake frontmost path ───────────────────────────────────────────────────
    let front_node = doc.nodes[&frontmost_id].clone();
    let front_pn = match &front_node.kind {
        SceneNodeKind::Path(p) => p.clone(),
        _ => unreachable!(),
    };
    let front_path = apply_affine_to_path(&front_pn.path_data, front_node.transform.to_kurbo());

    // ── Build update commands for each back node ──────────────────────────────
    let mut commands: Vec<Command> = Vec::new();
    let mut cropped = 0usize;

    for nid in &args.node_ids {
        if *nid == frontmost_id {
            continue;
        }
        let node = doc.nodes[nid].clone();
        let pn = match &node.kind {
            SceneNodeKind::Path(p) => p.clone(),
            _ => unreachable!(),
        };
        let baked_path = apply_affine_to_path(&pn.path_data, node.transform.to_kurbo());

        let intersected = match boolean_op(&baked_path, &front_path, BooleanOp::Intersect) {
            Ok(p) => p,
            Err(e) => {
                return ToolResult::error(format!("intersection failed for node {}: {}", nid, e))
            }
        };

        let mut new_node = node.clone();
        if let SceneNodeKind::Path(ref mut new_pn) = new_node.kind {
            new_pn.path_data = intersected;
        }
        // Reset transform since path is now in world space.
        new_node.transform = photonic_core::transform::Transform::IDENTITY;
        commands.push(Command::UpdateNode {
            old: node,
            new: new_node,
        });
        cropped += 1;
    }

    // Remove the frontmost (crop mask) last so undo works cleanly.
    commands.push(Command::RemoveNode {
        node_id: frontmost_id,
    });

    history.execute(Command::Batch(commands), &mut doc);

    ToolResult::text(format!(
        "Cropped {} node(s) to the frontmost boundary.",
        cropped
    ))
    .with_data(serde_json::json!({
        "cropped":       cropped,
        "removed_id":    frontmost_id,
    }))
}

// ─── pathfinder_minus_back ────────────────────────────────────────────────────

/// Subtract all back nodes from the frontmost node's path.
///
/// The frontmost node (highest z-order) has the union of all other selected
/// nodes subtracted from its path in sequence. The back nodes are removed.
/// The frontmost node's fill/stroke style is preserved unchanged.
pub async fn pathfinder_minus_back(state: &AppState, args: PathfinderMinusBackArgs) -> ToolResult {
    use photonic_core::ops::boolean::{boolean_op, BooleanOp};
    use photonic_core::ops::transform_ops::apply_affine_to_path;

    if args.node_ids.len() < 2 {
        return ToolResult::error("node_ids must contain at least 2 path node IDs");
    }

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    // ── Verify all nodes exist and are paths ─────────────────────────────────
    for nid in &args.node_ids {
        let node = match doc.nodes.get(nid) {
            Some(n) => n,
            None => return ToolResult::error(format!("node {} not found", nid)),
        };
        if !matches!(node.kind, SceneNodeKind::Path(_)) {
            return ToolResult::error(format!("node {} is not a path node", nid));
        }
    }

    // ── Determine frontmost (highest z-order) ────────────────────────────────
    let frontmost_id = {
        let mut best_id = args.node_ids[0];
        let mut best_key = node_z_key(&doc, &best_id);
        for nid in &args.node_ids[1..] {
            let key = node_z_key(&doc, nid);
            if key > best_key {
                best_key = key;
                best_id = *nid;
            }
        }
        best_id
    };

    // ── Bake frontmost path and subtract each back node ───────────────────────
    let front_node = doc.nodes[&frontmost_id].clone();
    let front_pn = match &front_node.kind {
        SceneNodeKind::Path(p) => p.clone(),
        _ => unreachable!(),
    };
    let mut result_path =
        apply_affine_to_path(&front_pn.path_data, front_node.transform.to_kurbo());

    for nid in &args.node_ids {
        if *nid == frontmost_id {
            continue;
        }
        let node = doc.nodes[nid].clone();
        let pn = match &node.kind {
            SceneNodeKind::Path(p) => p.clone(),
            _ => unreachable!(),
        };
        let baked = apply_affine_to_path(&pn.path_data, node.transform.to_kurbo());
        result_path = match boolean_op(&result_path, &baked, BooleanOp::Subtract) {
            Ok(p) => p,
            Err(e) => {
                return ToolResult::error(format!("subtraction failed for node {}: {}", nid, e))
            }
        };
    }

    // ── Build commands: update front node, remove back nodes ─────────────────
    let mut commands: Vec<Command> = Vec::new();

    let mut new_front = front_node.clone();
    if let SceneNodeKind::Path(ref mut new_pn) = new_front.kind {
        new_pn.path_data = result_path;
    }
    new_front.transform = photonic_core::transform::Transform::IDENTITY;
    commands.push(Command::UpdateNode {
        old: front_node,
        new: new_front,
    });

    let back_count = args.node_ids.len() - 1;
    for nid in &args.node_ids {
        if *nid != frontmost_id {
            commands.push(Command::RemoveNode { node_id: *nid });
        }
    }

    history.execute(Command::Batch(commands), &mut doc);

    ToolResult::text(format!(
        "Subtracted {} back node(s) from frontmost; back nodes removed.",
        back_count
    ))
    .with_data(serde_json::json!({
        "result_node_id": frontmost_id,
        "removed_count":  back_count,
    }))
}

// ─── pathfinder_minus_front ───────────────────────────────────────────────────

/// Subtract the frontmost node's path from every back node.
///
/// The frontmost node (highest z-order) punches a hole in each back node;
/// each back node is updated with `back_path - front_path`. The frontmost
/// node is then removed. Each back node's fill/stroke is preserved.
pub async fn pathfinder_minus_front(
    state: &AppState,
    args: PathfinderMinusFrontArgs,
) -> ToolResult {
    use photonic_core::ops::boolean::{boolean_op, BooleanOp};
    use photonic_core::ops::transform_ops::apply_affine_to_path;

    if args.node_ids.len() < 2 {
        return ToolResult::error("node_ids must contain at least 2 path node IDs");
    }

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    // ── Verify all nodes exist and are paths ─────────────────────────────────
    for nid in &args.node_ids {
        let node = match doc.nodes.get(nid) {
            Some(n) => n,
            None => return ToolResult::error(format!("node {} not found", nid)),
        };
        if !matches!(node.kind, SceneNodeKind::Path(_)) {
            return ToolResult::error(format!("node {} is not a path node", nid));
        }
    }

    // ── Determine frontmost (highest z-order) ────────────────────────────────
    let frontmost_id = {
        let mut best_id = args.node_ids[0];
        let mut best_key = node_z_key(&doc, &best_id);
        for nid in &args.node_ids[1..] {
            let key = node_z_key(&doc, nid);
            if key > best_key {
                best_key = key;
                best_id = *nid;
            }
        }
        best_id
    };

    // ── Bake the frontmost path (the cutter) ─────────────────────────────────
    let front_node = doc.nodes[&frontmost_id].clone();
    let front_pn = match &front_node.kind {
        SceneNodeKind::Path(p) => p.clone(),
        _ => unreachable!(),
    };
    let front_path = apply_affine_to_path(&front_pn.path_data, front_node.transform.to_kurbo());

    // ── Subtract front from each back node ───────────────────────────────────
    let mut commands: Vec<Command> = Vec::new();
    let mut updated = 0usize;

    for nid in &args.node_ids {
        if *nid == frontmost_id {
            continue;
        }
        let node = doc.nodes[nid].clone();
        let pn = match &node.kind {
            SceneNodeKind::Path(p) => p.clone(),
            _ => unreachable!(),
        };
        let baked = apply_affine_to_path(&pn.path_data, node.transform.to_kurbo());
        let result = match boolean_op(&baked, &front_path, BooleanOp::Subtract) {
            Ok(p) => p,
            Err(e) => {
                return ToolResult::error(format!("subtraction failed for node {}: {}", nid, e))
            }
        };
        let mut new_node = node.clone();
        if let SceneNodeKind::Path(ref mut new_pn) = new_node.kind {
            new_pn.path_data = result;
        }
        new_node.transform = photonic_core::transform::Transform::IDENTITY;
        commands.push(Command::UpdateNode {
            old: node,
            new: new_node,
        });
        updated += 1;
    }

    // Remove the frontmost (cutter) last.
    commands.push(Command::RemoveNode {
        node_id: frontmost_id,
    });

    history.execute(Command::Batch(commands), &mut doc);

    ToolResult::text(format!(
        "Subtracted frontmost from {} back node(s); frontmost removed.",
        updated
    ))
    .with_data(serde_json::json!({
        "updated_count": updated,
        "removed_id":    frontmost_id,
    }))
}

// ─── pathfinder_trim ──────────────────────────────────────────────────────────

/// Remove hidden portions of each node by subtracting all paths above it.
///
/// Nodes are processed back-to-front. Each node's path is replaced by
/// `its_path - union(all_paths_above)`. Strokes are disabled on every result
/// node; fills are preserved. No nodes are removed. Single undoable step.
pub async fn pathfinder_trim(state: &AppState, args: PathfinderTrimArgs) -> ToolResult {
    use photonic_core::ops::boolean::{boolean_op, BooleanOp};
    use photonic_core::ops::transform_ops::apply_affine_to_path;

    if args.node_ids.len() < 2 {
        return ToolResult::error("node_ids must contain at least 2 path node IDs");
    }

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    // ── Verify all nodes exist and are paths ─────────────────────────────────
    for nid in &args.node_ids {
        let node = match doc.nodes.get(nid) {
            Some(n) => n,
            None => return ToolResult::error(format!("node {} not found", nid)),
        };
        if !matches!(node.kind, SceneNodeKind::Path(_)) {
            return ToolResult::error(format!("node {} is not a path node", nid));
        }
    }

    // ── Sort nodes back-to-front by z-order ──────────────────────────────────
    let mut sorted_ids = args.node_ids.clone();
    sorted_ids.sort_by_key(|nid| node_z_key(&doc, nid));
    // sorted_ids[0] = backmost, sorted_ids[last] = frontmost

    // ── Bake all paths up front ───────────────────────────────────────────────
    let baked_paths: Vec<(uuid::Uuid, photonic_core::path::PathData)> = sorted_ids
        .iter()
        .map(|nid| {
            let node = &doc.nodes[nid];
            let pn = match &node.kind {
                SceneNodeKind::Path(p) => p,
                _ => unreachable!(),
            };
            (
                *nid,
                apply_affine_to_path(&pn.path_data, node.transform.to_kurbo()),
            )
        })
        .collect();

    // ── For each node (back to front), subtract all nodes above it ────────────
    let mut commands: Vec<Command> = Vec::new();

    for i in 0..sorted_ids.len() {
        let nid = sorted_ids[i];
        let mut trimmed = baked_paths[i].1.clone();

        // Subtract every node above this one (higher index = higher z).
        for j in (i + 1)..sorted_ids.len() {
            trimmed = match boolean_op(&trimmed, &baked_paths[j].1, BooleanOp::Subtract) {
                Ok(p) => p,
                Err(e) => {
                    return ToolResult::error(format!(
                        "trim subtraction failed at step {}: {}",
                        j, e
                    ))
                }
            };
        }

        let node = doc.nodes[&nid].clone();
        let mut new_node = node.clone();
        if let SceneNodeKind::Path(ref mut new_pn) = new_node.kind {
            new_pn.path_data = trimmed;
            new_pn.stroke.enabled = false; // Trim removes strokes (Illustrator behaviour)
        }
        new_node.transform = photonic_core::transform::Transform::IDENTITY;
        commands.push(Command::UpdateNode {
            old: node,
            new: new_node,
        });
    }

    history.execute(Command::Batch(commands), &mut doc);

    ToolResult::text(format!(
        "Trimmed {} node(s); hidden areas removed, strokes disabled.",
        sorted_ids.len()
    ))
    .with_data(serde_json::json!({
        "trimmed_count": sorted_ids.len(),
    }))
}

// ─── pathfinder_outline ───────────────────────────────────────────────────────

/// Convert each selected path from filled to stroked outline.
///
/// For each node: the solid fill color is moved to the stroke; the fill is set
/// to none. If the fill is a gradient, the stroke defaults to black. Existing
/// stroke width is preserved (or defaults to 1.0 if no stroke was set). The
/// path data is unchanged. Single undoable step.
pub async fn pathfinder_outline(state: &AppState, args: PathfinderOutlineArgs) -> ToolResult {
    use photonic_core::style::{Fill, FillKind};

    if args.node_ids.is_empty() {
        return ToolResult::error("node_ids must not be empty");
    }

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let mut commands: Vec<Command> = Vec::new();
    let mut updated = 0usize;

    for nid in &args.node_ids {
        let node = match doc.nodes.get(nid) {
            Some(n) => n.clone(),
            None => return ToolResult::error(format!("node {} not found", nid)),
        };
        let pn = match &node.kind {
            SceneNodeKind::Path(p) => p.clone(),
            _ => continue, // silently skip non-path nodes
        };

        // Determine stroke color from fill.
        let stroke_color = match &pn.fill.kind {
            FillKind::Solid(c) => *c,
            FillKind::Gradient(g) => g
                .stops
                .first()
                .map(|s| s.color)
                .unwrap_or(photonic_core::color::Color::BLACK),
            FillKind::FluidGradient(fg) => fg
                .points
                .first()
                .map(|p| p.color)
                .unwrap_or(photonic_core::color::Color::BLACK),
            FillKind::MeshGradient(_) => photonic_core::color::Color::BLACK,
            FillKind::None => photonic_core::color::Color::BLACK,
        };

        let stroke_width = if pn.stroke.enabled {
            pn.stroke.width
        } else {
            1.0
        };

        let mut new_node = node.clone();
        if let SceneNodeKind::Path(ref mut new_pn) = new_node.kind {
            new_pn.fill = Fill::none();
            new_pn.stroke.color = stroke_color;
            new_pn.stroke.width = stroke_width;
            new_pn.stroke.enabled = true;
        }
        commands.push(Command::UpdateNode {
            old: node,
            new: new_node,
        });
        updated += 1;
    }

    if commands.is_empty() {
        return ToolResult::text("No path nodes found in node_ids; nothing changed.".to_string());
    }

    history.execute(Command::Batch(commands), &mut doc);

    ToolResult::text(format!(
        "Outlined {} node(s); fills removed, strokes set.",
        updated
    ))
    .with_data(serde_json::json!({
        "outlined_count": updated,
    }))
}

// ─── pathfinder_divide ────────────────────────────────────────────────────────

/// Divide two paths at every overlap edge into distinct colored face nodes.
/// Exactly two path node IDs must be provided. Up to three result nodes are
/// created; the originals are removed. Face colors are inherited from the
/// source shape that contained each face. Single undoable step.
pub async fn pathfinder_divide(state: &AppState, args: PathfinderDivideArgs) -> ToolResult {
    use photonic_core::ops::boolean::divide_paths;
    use photonic_core::ops::transform_ops::apply_affine_to_path;

    if args.node_ids.len() != 2 {
        return ToolResult::error("pathfinder_divide requires exactly 2 node IDs (back, front)");
    }

    let back_id = args.node_ids[0];
    let front_id = args.node_ids[1];

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let back_node = match doc.nodes.get(&back_id) {
        Some(n) => n.clone(),
        None => return ToolResult::error(format!("node {} not found", back_id)),
    };
    let front_node = match doc.nodes.get(&front_id) {
        Some(n) => n.clone(),
        None => return ToolResult::error(format!("node {} not found", front_id)),
    };

    let back_pn = match &back_node.kind {
        SceneNodeKind::Path(p) => p.clone(),
        _ => return ToolResult::error("back node is not a path"),
    };
    let front_pn = match &front_node.kind {
        SceneNodeKind::Path(p) => p.clone(),
        _ => return ToolResult::error("front node is not a path"),
    };

    // Bake transforms into path coordinates.
    let back_baked = apply_affine_to_path(&back_pn.path_data, back_node.transform.to_kurbo());
    let front_baked = apply_affine_to_path(&front_pn.path_data, front_node.transform.to_kurbo());

    let faces = divide_paths(&back_baked, &front_baked);
    if faces.is_empty() {
        return ToolResult::error("Divide produced no faces — shapes may not overlap");
    }

    let target_layer = args.layer_id.unwrap_or(back_node.layer_id);
    let source_pns = [&back_pn, &front_pn];
    let source_nodes = [&back_node, &front_node];

    let mut commands: Vec<Command> = Vec::new();
    commands.push(Command::RemoveNode { node_id: back_id });
    commands.push(Command::RemoveNode { node_id: front_id });

    let mut created_ids: Vec<uuid::Uuid> = Vec::new();
    for (i, (path_data, source_idx)) in faces.into_iter().enumerate() {
        let src_pn = source_pns[source_idx];
        let src_node = source_nodes[source_idx];
        let mut new_pn = src_pn.clone();
        new_pn.path_data = path_data;
        let mut new_node = SceneNode::new(
            format!("{} face {}", src_node.name, i + 1),
            target_layer,
            SceneNodeKind::Path(new_pn),
        );
        new_node.opacity = src_node.opacity;
        new_node.blend_mode = src_node.blend_mode;
        new_node.tags = src_node.tags.clone();
        let new_id = new_node.id;
        commands.push(Command::AddNode {
            node: new_node,
            layer_id: Some(target_layer),
        });
        created_ids.push(new_id);
    }

    history.execute(Command::Batch(commands), &mut doc);

    ToolResult::text(format!("Divided into {} face(s).", created_ids.len())).with_data(
        serde_json::json!({
            "face_count": created_ids.len(),
            "created_node_ids": created_ids,
        }),
    )
}

// ─── divide_objects_below ─────────────────────────────────────────────────────

/// Use a path node as a cutting edge to divide all nodes below it in z-order.
/// Each overlapping node beneath the cutter is split into two face nodes:
/// the region inside the cutter and the region outside. The cutter is removed.
/// Non-overlapping nodes below are unchanged. Single undoable step.
pub async fn divide_objects_below(state: &AppState, args: DivideObjectsBelowArgs) -> ToolResult {
    use photonic_core::ops::boolean::{boolean_op, divide_paths, BooleanOp};
    use photonic_core::ops::transform_ops::apply_affine_to_path;

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let cutter_node = match doc.nodes.get(&args.node_id) {
        Some(n) => n.clone(),
        None => return ToolResult::error(format!("node {} not found", args.node_id)),
    };
    let cutter_pn = match &cutter_node.kind {
        SceneNodeKind::Path(p) => p.clone(),
        _ => return ToolResult::error("cutter node must be a path"),
    };
    let cutter_baked = apply_affine_to_path(&cutter_pn.path_data, cutter_node.transform.to_kurbo());

    // Find all path nodes below the cutter in the same layer.
    let (cutter_layer_id, cutter_z) = match doc.node_layer_and_index(&args.node_id) {
        Some(x) => x,
        None => return ToolResult::error("could not determine cutter z-order"),
    };
    let layer = match doc.layers.get(&cutter_layer_id) {
        Some(l) => l.clone(),
        None => return ToolResult::error("cutter layer not found"),
    };

    let below_ids: Vec<uuid::Uuid> = layer.node_ids[..cutter_z].iter().copied().collect();

    let mut commands: Vec<Command> = Vec::new();
    let mut split_count = 0usize;

    for target_id in &below_ids {
        let target_node = match doc.nodes.get(target_id) {
            Some(n) => n.clone(),
            None => continue,
        };
        let target_pn = match &target_node.kind {
            SceneNodeKind::Path(p) => p.clone(),
            _ => continue, // skip non-path nodes
        };
        let target_baked =
            apply_affine_to_path(&target_pn.path_data, target_node.transform.to_kurbo());

        // Skip if no overlap.
        let overlap = boolean_op(&target_baked, &cutter_baked, BooleanOp::Intersect)
            .unwrap_or_else(|_| {
                photonic_core::path::PathData::from_bez_path(&kurbo::BezPath::new())
            });
        if overlap.is_empty() {
            continue;
        }

        let faces = divide_paths(&target_baked, &cutter_baked);
        commands.push(Command::RemoveNode {
            node_id: *target_id,
        });
        for (i, (path_data, _source_idx)) in faces.into_iter().enumerate() {
            let mut new_pn = target_pn.clone();
            new_pn.path_data = path_data;
            let mut new_node = SceneNode::new(
                format!("{} face {}", target_node.name, i + 1),
                cutter_layer_id,
                SceneNodeKind::Path(new_pn),
            );
            new_node.opacity = target_node.opacity;
            new_node.blend_mode = target_node.blend_mode;
            new_node.tags = target_node.tags.clone();
            commands.push(Command::AddNode {
                node: new_node,
                layer_id: Some(cutter_layer_id),
            });
        }
        split_count += 1;
    }

    // Remove the cutter.
    commands.push(Command::RemoveNode {
        node_id: args.node_id,
    });

    if commands.len() == 1 {
        // Only the cutter removal — nothing actually overlapped.
        history.execute(Command::Batch(commands), &mut doc);
        return ToolResult::text(
            "No overlapping objects found below the cutter; cutter removed.".to_string(),
        );
    }

    history.execute(Command::Batch(commands), &mut doc);
    ToolResult::text(format!(
        "Divided {} object(s) below; cutter removed.",
        split_count
    ))
    .with_data(serde_json::json!({ "split_count": split_count }))
}

// ─── pathfinder_merge ────────────────────────────────────────────────────────

/// Trim all selected nodes of overlapping areas, then merge (union) any nodes
/// that share the same solid fill color into a single combined shape.
///
/// Process:
///  1. Sort nodes back-to-front by z-order.
///  2. Each node is trimmed: regions covered by nodes above it are subtracted.
///  3. Trimmed faces are grouped by solid fill color (RGBA, rounded to 2 dp).
///     Non-solid fills each form their own group.
///  4. Each group's paths are unioned into one shape.
///  5. The original nodes are replaced by the merged result nodes.
///  6. Strokes are disabled on all result nodes (Illustrator behaviour).
pub async fn pathfinder_merge(state: &AppState, args: PathfinderMergeArgs) -> ToolResult {
    use photonic_core::ops::boolean::{boolean_op, BooleanOp};
    use photonic_core::ops::transform_ops::apply_affine_to_path;
    use photonic_core::style::FillKind;

    if args.node_ids.len() < 2 {
        return ToolResult::error("node_ids must contain at least 2 path node IDs");
    }

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    // Verify all nodes exist and are paths.
    for nid in &args.node_ids {
        let node = match doc.nodes.get(nid) {
            Some(n) => n,
            None => return ToolResult::error(format!("node {} not found", nid)),
        };
        if !matches!(node.kind, SceneNodeKind::Path(_)) {
            return ToolResult::error(format!("node {} is not a path node", nid));
        }
    }

    // Sort back-to-front by z-order.
    let mut sorted_ids = args.node_ids.clone();
    sorted_ids.sort_by_key(|nid| node_z_key(&doc, nid));

    let target_layer = args
        .layer_id
        .unwrap_or_else(|| doc.nodes[&sorted_ids[0]].layer_id);

    // Bake all paths.
    let baked: Vec<(uuid::Uuid, photonic_core::path::PathData)> = sorted_ids
        .iter()
        .map(|nid| {
            let node = &doc.nodes[nid];
            let pn = match &node.kind {
                SceneNodeKind::Path(p) => p,
                _ => unreachable!(),
            };
            (
                *nid,
                apply_affine_to_path(&pn.path_data, node.transform.to_kurbo()),
            )
        })
        .collect();

    // Trim each node: subtract all nodes above it.
    // trimmed_faces[i] = (nid, trimmed_path, fill_key_string, source_pn clone)
    let mut trimmed_faces: Vec<(uuid::Uuid, photonic_core::path::PathData, String)> = Vec::new();
    for i in 0..baked.len() {
        let (nid, ref path) = baked[i];
        let mut trimmed = path.clone();
        for j in (i + 1)..baked.len() {
            match boolean_op(&trimmed, &baked[j].1, BooleanOp::Subtract) {
                Ok(p) => trimmed = p,
                Err(e) => {
                    return ToolResult::error(format!("merge trim step failed at z {}: {}", j, e))
                }
            }
        }
        // Build a fill group key.
        let fill_key = match &doc.nodes[&nid].kind {
            SceneNodeKind::Path(pn) => match &pn.fill.kind {
                FillKind::Solid(c) => format!("solid:{:.2},{:.2},{:.2},{:.2}", c.r, c.g, c.b, c.a),
                _ => format!("other:{}", nid), // non-solid: unique group
            },
            _ => format!("other:{}", nid),
        };
        trimmed_faces.push((nid, trimmed, fill_key));
    }

    // Group by fill_key, preserving back-to-front order for first representative.
    let mut groups: Vec<(String, Vec<photonic_core::path::PathData>)> = Vec::new();
    let mut key_to_group_idx: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    for (_, trimmed_path, fill_key) in &trimmed_faces {
        if let Some(&idx) = key_to_group_idx.get(fill_key) {
            groups[idx].1.push(trimmed_path.clone());
        } else {
            let idx = groups.len();
            key_to_group_idx.insert(fill_key.clone(), idx);
            groups.push((fill_key.clone(), vec![trimmed_path.clone()]));
        }
    }

    // For each group, union all paths.
    // Representative node (first occurrence back-to-front) donates style.
    let mut commands: Vec<Command> = Vec::new();

    // Remove all originals first.
    for nid in &sorted_ids {
        commands.push(Command::RemoveNode { node_id: *nid });
    }

    let mut created_count = 0usize;
    for (fill_key, paths) in &groups {
        // Union all paths in the group.
        let mut merged = paths[0].clone();
        for path in &paths[1..] {
            match boolean_op(&merged, path, BooleanOp::Union) {
                Ok(p) => merged = p,
                Err(e) => return ToolResult::error(format!("merge union step failed: {}", e)),
            }
        }

        // Find the representative (first sorted_id with this fill_key).
        let rep_id = trimmed_faces
            .iter()
            .find(|(_, _, k)| k == fill_key)
            .map(|(nid, _, _)| *nid)
            .unwrap();
        let rep_node = doc.nodes[&rep_id].clone();
        let rep_pn = match &rep_node.kind {
            SceneNodeKind::Path(p) => p.clone(),
            _ => unreachable!(),
        };

        let mut new_pn = rep_pn.clone();
        new_pn.path_data = merged;
        new_pn.stroke.enabled = false;

        let group_name = if paths.len() > 1 {
            format!("{} merged", rep_node.name)
        } else {
            rep_node.name.clone()
        };
        let mut new_node = SceneNode::new(group_name, target_layer, SceneNodeKind::Path(new_pn));
        new_node.opacity = rep_node.opacity;
        new_node.blend_mode = rep_node.blend_mode;
        commands.push(Command::AddNode {
            node: new_node,
            layer_id: Some(target_layer),
        });
        created_count += 1;
    }

    history.execute(Command::Batch(commands), &mut doc);

    ToolResult::text(format!(
        "Merged {} node(s) into {} result shape(s); strokes disabled.",
        sorted_ids.len(),
        created_count
    ))
    .with_data(serde_json::json!({
        "source_count":  sorted_ids.len(),
        "result_count":  created_count,
    }))
}

// ─── select_same ─────────────────────────────────────────────────────────────

/// Select all document nodes that share a specific attribute with the reference
/// node. Updates the document's active selection and returns the matching IDs.
pub async fn select_same(state: &AppState, args: SelectSameArgs) -> ToolResult {
    let tolerance_f64 = args.tolerance.unwrap_or(0.01);
    let tolerance = tolerance_f64 as f32;
    let include_self = args.include_self.unwrap_or(true);

    let mut doc = state.document.lock().await;

    let ref_node = match doc.nodes.get(&args.node_id) {
        Some(n) => n.clone(),
        None => return ToolResult::error(format!("reference node {} not found", args.node_id)),
    };

    let mut matched: Vec<uuid::Uuid> = Vec::new();

    for (nid, node) in &doc.nodes {
        let is_self = *nid == args.node_id;
        if is_self && !include_self {
            continue;
        }

        let matches = match args.attribute {
            SelectSameAttribute::FillColor => {
                let ref_color = solid_fill_color(&ref_node);
                let cand_color = solid_fill_color(node);
                match (ref_color, cand_color) {
                    (Some(rc), Some(cc)) => color_distance(rc, cc) <= tolerance,
                    (None, None) => true, // both have no solid fill
                    _ => false,
                }
            }
            SelectSameAttribute::StrokeColor => {
                if let (SceneNodeKind::Path(rp), SceneNodeKind::Path(cp)) =
                    (&ref_node.kind, &node.kind)
                {
                    match (rp.stroke.enabled, cp.stroke.enabled) {
                        (true, true) => {
                            color_distance(rp.stroke.color, cp.stroke.color) <= tolerance
                        }
                        (false, false) => true,
                        _ => false,
                    }
                } else {
                    false
                }
            }
            SelectSameAttribute::StrokeWeight => {
                if let (SceneNodeKind::Path(rp), SceneNodeKind::Path(cp)) =
                    (&ref_node.kind, &node.kind)
                {
                    (rp.stroke.width - cp.stroke.width).abs() <= tolerance as f64
                } else {
                    false
                }
            }
            SelectSameAttribute::Opacity => (ref_node.opacity - node.opacity).abs() <= tolerance,
            SelectSameAttribute::BlendMode => ref_node.blend_mode == node.blend_mode,
            SelectSameAttribute::ObjectType => {
                std::mem::discriminant(&ref_node.kind) == std::mem::discriminant(&node.kind)
            }
        };

        if matches {
            matched.push(*nid);
        }
    }

    // Update the document selection.
    doc.selection.clear();
    for nid in &matched {
        doc.selection.add(*nid);
    }

    let attr_label = match args.attribute {
        SelectSameAttribute::FillColor => "fill color",
        SelectSameAttribute::StrokeColor => "stroke color",
        SelectSameAttribute::StrokeWeight => "stroke weight",
        SelectSameAttribute::Opacity => "opacity",
        SelectSameAttribute::BlendMode => "blend mode",
        SelectSameAttribute::ObjectType => "object type",
    };
    let attr_key = match args.attribute {
        SelectSameAttribute::FillColor => "fill_color",
        SelectSameAttribute::StrokeColor => "stroke_color",
        SelectSameAttribute::StrokeWeight => "stroke_weight",
        SelectSameAttribute::Opacity => "opacity",
        SelectSameAttribute::BlendMode => "blend_mode",
        SelectSameAttribute::ObjectType => "object_type",
    };
    let count = matched.len();
    ToolResult::text(format!(
        "Selected {} node(s) with matching {}.",
        count, attr_label
    ))
    .with_data(serde_json::json!({
        "node_ids": matched,
        "count":    count,
        "attribute": attr_key,
    }))
}

/// Extract the solid fill color from a node, or None if it has no solid fill.
fn solid_fill_color(node: &SceneNode) -> Option<photonic_core::color::Color> {
    use photonic_core::style::FillKind;
    if let SceneNodeKind::Path(pn) = &node.kind {
        if pn.fill.enabled {
            if let FillKind::Solid(c) = pn.fill.kind {
                return Some(c);
            }
        }
    }
    None
}

/// Euclidean distance between two RGBA colors in [0,1] space.
fn color_distance(a: photonic_core::color::Color, b: photonic_core::color::Color) -> f32 {
    let dr = a.r - b.r;
    let dg = a.g - b.g;
    let db = a.b - b.b;
    let da = a.a - b.a;
    (dr * dr + dg * dg + db * db + da * da).sqrt()
}

/// Compute a sortable z-order key `(layer_order_index, node_index_in_layer)`.
/// Higher = frontmost.
fn node_z_key(doc: &photonic_core::document::Document, node_id: &uuid::Uuid) -> (usize, usize) {
    if let Some(node) = doc.nodes.get(node_id) {
        let layer_pos = doc
            .layer_order
            .iter()
            .position(|id| *id == node.layer_id)
            .unwrap_or(0);
        let node_pos = doc
            .layers
            .get(&node.layer_id)
            .and_then(|l| l.node_ids.iter().position(|id| id == node_id))
            .unwrap_or(0);
        (layer_pos, node_pos)
    } else {
        (0, 0)
    }
}

/// Returns the horizontal center of a path node's bounding box (local space).
fn path_center_x(node: &SceneNode) -> f32 {
    if let SceneNodeKind::Path(p) = &node.kind {
        if let Some(bb) = p.path_data.bounding_box() {
            return ((bb.x0 + bb.x1) / 2.0) as f32;
        }
    }
    0.0
}

/// Returns the vertical center of a path node's bounding box (local space).
fn path_center_y(node: &SceneNode) -> f32 {
    if let SceneNodeKind::Path(p) = &node.kind {
        if let Some(bb) = p.path_data.bounding_box() {
            return ((bb.y0 + bb.y1) / 2.0) as f32;
        }
    }
    0.0
}

// ─── make_compound_path ───────────────────────────────────────────────────────

/// Combine multiple path nodes into a single compound path.
/// Overlapping subpaths create holes via the even-odd fill rule.
/// The bottommost node (first in document order) keeps its position and donates
/// its fill, stroke, and transform; all other source nodes are removed.
pub async fn make_compound_path(state: &AppState, args: MakeCompoundPathArgs) -> ToolResult {
    if args.node_ids.len() < 2 {
        return ToolResult::error("make_compound_path requires at least 2 node_ids");
    }

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    // Validate: all must be top-level path nodes.
    for &id in &args.node_ids {
        match doc.nodes.get(&id) {
            Some(n) => {
                if !matches!(n.kind, SceneNodeKind::Path(_)) {
                    return ToolResult::error(format!("Node {} is not a path node", id));
                }
            }
            None => return ToolResult::error(format!("Node {} not found", id)),
        }
    }

    // Determine document order among selected nodes (bottommost first).
    let mut ordered_ids: Vec<NodeId> = Vec::new();
    for node in doc.nodes_in_draw_order() {
        if args.node_ids.contains(&node.id) {
            ordered_ids.push(node.id);
        }
    }
    if ordered_ids.len() != args.node_ids.len() {
        return ToolResult::error(
            "One or more nodes not found in draw order (may be inside a group)",
        );
    }

    // The bottommost node is the base: its ID becomes the compound path ID.
    let base_id = ordered_ids[0];
    let base_node = doc.nodes[&base_id].clone();
    // Concatenate all BezPaths, baking each node's world transform.
    let [ba, bb, bc, bd, be, bf] = base_node.transform.matrix;
    let base_det = ba * bd - bb * bc;
    let mut merged = kurbo::BezPath::new();
    for &id in &ordered_ids {
        let node = &doc.nodes[&id];
        let pn = match &node.kind {
            SceneNodeKind::Path(p) => p,
            _ => unreachable!(),
        };
        let [a, b, c, d, e, f] = node.transform.matrix;
        let bez = pn.path_data.to_bez_path();
        for el in bez.elements() {
            use kurbo::PathEl::*;
            // Transform to world coords, then into base node's local space (inverse of base transform).
            let to_local = |wx: f64, wy: f64| -> (f64, f64) {
                // world → base local (inverse affine)
                if base_det.abs() < 1e-12 {
                    return (wx - be, wy - bf);
                }
                let tx = wx - be;
                let ty = wy - bf;
                (
                    (bd * tx - bc * ty) / base_det,
                    (-bb * tx + ba * ty) / base_det,
                )
            };
            let world_pt = |px: f64, py: f64| -> kurbo::Point {
                kurbo::Point::new(a * px + c * py + e, b * px + d * py + f)
            };
            let transformed = match el {
                MoveTo(p) => {
                    let wp = world_pt(p.x, p.y);
                    let lp = to_local(wp.x, wp.y);
                    MoveTo(kurbo::Point::new(lp.0, lp.1))
                }
                LineTo(p) => {
                    let wp = world_pt(p.x, p.y);
                    let lp = to_local(wp.x, wp.y);
                    LineTo(kurbo::Point::new(lp.0, lp.1))
                }
                QuadTo(p1, p2) => {
                    let wp1 = world_pt(p1.x, p1.y);
                    let lp1 = to_local(wp1.x, wp1.y);
                    let wp2 = world_pt(p2.x, p2.y);
                    let lp2 = to_local(wp2.x, wp2.y);
                    QuadTo(
                        kurbo::Point::new(lp1.0, lp1.1),
                        kurbo::Point::new(lp2.0, lp2.1),
                    )
                }
                CurveTo(p1, p2, p3) => {
                    let wp1 = world_pt(p1.x, p1.y);
                    let lp1 = to_local(wp1.x, wp1.y);
                    let wp2 = world_pt(p2.x, p2.y);
                    let lp2 = to_local(wp2.x, wp2.y);
                    let wp3 = world_pt(p3.x, p3.y);
                    let lp3 = to_local(wp3.x, wp3.y);
                    CurveTo(
                        kurbo::Point::new(lp1.0, lp1.1),
                        kurbo::Point::new(lp2.0, lp2.1),
                        kurbo::Point::new(lp3.0, lp3.1),
                    )
                }
                ClosePath => ClosePath,
            };
            merged.push(transformed);
        }
    }

    let compound_name = args
        .name
        .unwrap_or_else(|| format!("{} (compound)", base_node.name));

    // Build the updated base node: merged path + is_compound flag + new name.
    let mut updated_node = base_node.clone();
    updated_node.name = compound_name.clone();
    if let SceneNodeKind::Path(ref mut p) = updated_node.kind {
        p.path_data = PathData::from_bez_path(&merged);
        p.is_compound = true;
    }

    // Batch: UpdateNode for base, RemoveNode for all other sources.
    let mut cmds = vec![Command::UpdateNode {
        old: base_node,
        new: updated_node,
    }];
    for &id in &ordered_ids[1..] {
        cmds.push(Command::RemoveNode { node_id: id });
    }

    history.execute(Command::Batch(cmds), &mut doc);
    history.schedule_mcp_checkpoint(format!("Make compound path '{}'", compound_name));

    ToolResult::text(format!(
        "Combined {} paths into compound path '{}' (id: {}).",
        ordered_ids.len(),
        compound_name,
        base_id
    ))
    .with_data(serde_json::json!({
        "node_id": base_id,
        "source_count": ordered_ids.len(),
    }))
}

// ─── release_compound_path ────────────────────────────────────────────────────

/// Split a compound path back into individual path nodes, one per subpath.
pub async fn release_compound_path(state: &AppState, args: ReleaseCompoundPathArgs) -> ToolResult {
    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let node = match doc.nodes.get(&args.node_id) {
        Some(n) => n.clone(),
        None => return ToolResult::error(format!("Node {} not found", args.node_id)),
    };

    let pn = match &node.kind {
        SceneNodeKind::Path(p) => p.clone(),
        _ => return ToolResult::error("Node is not a path node"),
    };

    // Split BezPath into individual subpaths (each beginning with MoveTo).
    let bez = pn.path_data.to_bez_path();
    let mut subpaths: Vec<kurbo::BezPath> = Vec::new();
    let mut current = kurbo::BezPath::new();

    for el in bez.elements() {
        if matches!(el, kurbo::PathEl::MoveTo(_)) && !current.elements().is_empty() {
            subpaths.push(current);
            current = kurbo::BezPath::new();
        }
        current.push(*el);
    }
    if !current.elements().is_empty() {
        subpaths.push(current);
    }

    if subpaths.len() < 2 {
        // Nothing to release — just clear the compound flag.
        let mut updated = node.clone();
        if let SceneNodeKind::Path(ref mut p) = updated.kind {
            p.is_compound = false;
        }
        history.execute(
            Command::UpdateNode {
                old: node,
                new: updated,
            },
            &mut doc,
        );
        return ToolResult::text(
            "Compound path had only one subpath; compound flag cleared.".to_string(),
        );
    }

    let layer_id = match doc.node_layer_and_index(&args.node_id) {
        Some((lid, _)) => lid,
        None => return ToolResult::error("Node has no layer position"),
    };

    let base_name = node.name.trim_end_matches(" (compound)").to_string();
    let mut new_ids: Vec<NodeId> = vec![args.node_id]; // first subpath reuses base node ID
    let mut cmds: Vec<Command> = Vec::new();

    // Update the compound node in-place to become subpath 0 (keeps layer position).
    let mut updated_base = node.clone();
    updated_base.name = format!("{} 1", base_name);
    if let SceneNodeKind::Path(ref mut p) = updated_base.kind {
        p.path_data = PathData::from_bez_path(&subpaths[0]);
        p.is_compound = false;
    }
    cmds.push(Command::UpdateNode {
        old: node.clone(),
        new: updated_base,
    });

    // Add one new node per remaining subpath.
    for (i, subpath_bez) in subpaths[1..].iter().enumerate() {
        let mut sub_pn = PathNode::new(PathData::from_bez_path(subpath_bez));
        sub_pn.fill = pn.fill.clone();
        sub_pn.stroke = pn.stroke.clone();
        sub_pn.is_compound = false;

        let sub_id = uuid::Uuid::new_v4();
        let sub_node = SceneNode::new(
            format!("{} {}", base_name, i + 2),
            layer_id,
            SceneNodeKind::Path(sub_pn),
        )
        .with_transform(node.transform);
        // Copy opacity/blend_mode manually since SceneNode::new doesn't expose them as builders.
        let mut sub_node = sub_node;
        sub_node.id = sub_id;
        sub_node.opacity = node.opacity;
        sub_node.visible = node.visible;
        sub_node.locked = node.locked;
        sub_node.blend_mode = node.blend_mode;

        new_ids.push(sub_id);
        cmds.push(Command::AddNode {
            node: sub_node,
            layer_id: Some(layer_id),
        });
    }

    history.execute(Command::Batch(cmds), &mut doc);
    history.schedule_mcp_checkpoint(format!("Release compound path '{}'", node.name));

    ToolResult::text(format!(
        "Released '{}' into {} individual path(s).",
        node.name,
        new_ids.len()
    ))
    .with_data(serde_json::json!({
        "node_ids": new_ids,
        "subpath_count": new_ids.len(),
    }))
}

/// Iteratively push nodes apart until none of their bounding boxes overlap.
pub async fn distribute_no_overlap(state: &AppState, args: DistributeNoOverlapArgs) -> ToolResult {
    use kurbo::Shape as _;
    tracing::debug!("tool: distribute_no_overlap");

    let padding = args.padding.unwrap_or(4.0_f64).max(0.0);
    let max_iter = args.max_iterations.unwrap_or(100).min(500);

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    // Resolve node IDs (from args or current selection).
    let ids: Vec<uuid::Uuid> = if args.node_ids.is_empty() {
        doc.selection.node_ids.iter().copied().collect()
    } else {
        args.node_ids
            .iter()
            .filter_map(|s| {
                uuid::Uuid::parse_str(s)
                    .ok()
                    .or_else(|| doc.find_node_by_name(s).map(|n| n.id))
            })
            .collect()
    };

    if ids.len() < 2 {
        return ToolResult::error("need at least 2 nodes to distribute");
    }

    // Cap at 100 nodes to keep O(n²) bounded.
    let ids: Vec<uuid::Uuid> = ids.into_iter().take(100).collect();
    let n = ids.len();

    // Snapshot current translation offsets (dx, dy) accumulated during simulation.
    let mut offsets: Vec<(f64, f64)> = vec![(0.0_f64, 0.0_f64); n];

    // Get node bounding boxes in local space (without transform — we apply transform separately).
    let mut local_bboxes: Vec<(f64, f64, f64, f64)> = ids
        .iter()
        .map(|id| -> (f64, f64, f64, f64) {
            if let Some(node) = doc.nodes.get(id) {
                if let SceneNodeKind::Path(pn) = &node.kind {
                    let bb = pn.path_data.to_bez_path().bounding_box();
                    return (bb.x0, bb.y0, bb.x1, bb.y1);
                }
            }
            (0.0_f64, 0.0_f64, 0.0_f64, 0.0_f64)
        })
        .collect();

    // Include node's existing translation in local_bboxes (world bbox = local_bbox + translate).
    let translates: Vec<(f64, f64)> = ids
        .iter()
        .map(|id| {
            if let Some(node) = doc.nodes.get(id) {
                (node.transform.matrix[4], node.transform.matrix[5])
            } else {
                (0.0, 0.0)
            }
        })
        .collect();

    // Make bboxes world-space.
    for i in 0..n {
        let (tx, ty) = translates[i];
        local_bboxes[i].0 += tx;
        local_bboxes[i].1 += ty;
        local_bboxes[i].2 += tx;
        local_bboxes[i].3 += ty;
    }

    let mut iterations_done = 0usize;

    for _ in 0..max_iter {
        let mut any_overlap = false;

        for i in 0..n {
            for j in (i + 1)..n {
                let (ax0, ay0, ax1, ay1) = (
                    local_bboxes[i].0 + offsets[i].0 - padding / 2.0,
                    local_bboxes[i].1 + offsets[i].1 - padding / 2.0,
                    local_bboxes[i].2 + offsets[i].0 + padding / 2.0,
                    local_bboxes[i].3 + offsets[i].1 + padding / 2.0,
                );
                let (bx0, by0, bx1, by1) = (
                    local_bboxes[j].0 + offsets[j].0 - padding / 2.0,
                    local_bboxes[j].1 + offsets[j].1 - padding / 2.0,
                    local_bboxes[j].2 + offsets[j].0 + padding / 2.0,
                    local_bboxes[j].3 + offsets[j].1 + padding / 2.0,
                );

                let overlap_x: f64 = (ax1.min(bx1) - ax0.max(bx0)).max(0.0);
                let overlap_y: f64 = (ay1.min(by1) - ay0.max(by0)).max(0.0);

                if overlap_x > 0.0 && overlap_y > 0.0 {
                    any_overlap = true;
                    // Push along the axis with smaller overlap.
                    let (push_x, push_y) = if overlap_x < overlap_y {
                        // Push horizontally.
                        let acx = (ax0 + ax1) / 2.0;
                        let bcx = (bx0 + bx1) / 2.0;
                        let dir = if acx <= bcx { -1.0 } else { 1.0 };
                        (dir * overlap_x / 2.0, 0.0)
                    } else {
                        // Push vertically.
                        let acy = (ay0 + ay1) / 2.0;
                        let bcy = (by0 + by1) / 2.0;
                        let dir = if acy <= bcy { -1.0 } else { 1.0 };
                        (0.0, dir * overlap_y / 2.0)
                    };
                    offsets[i].0 += push_x;
                    offsets[i].1 += push_y;
                    offsets[j].0 -= push_x;
                    offsets[j].1 -= push_y;
                }
            }
        }

        iterations_done += 1;
        if !any_overlap {
            break;
        }
    }

    // Apply offsets as UpdateNode commands.
    let mut commands = Vec::new();
    let mut moved = 0usize;
    for (i, id) in ids.iter().enumerate() {
        let (dx, dy): (f64, f64) = offsets[i];
        if dx.abs() > 0.01 || dy.abs() > 0.01 {
            if let Some(node) = doc.nodes.get(id).cloned() {
                let mut new_node = node.clone();
                new_node.transform.matrix[4] += dx;
                new_node.transform.matrix[5] += dy;
                commands.push(Command::UpdateNode {
                    old: node,
                    new: new_node,
                });
                moved += 1;
            }
        }
    }

    if commands.is_empty() {
        return ToolResult::text("No overlapping nodes found — nothing moved".to_string());
    }

    let batch = if commands.len() == 1 {
        commands.remove(0)
    } else {
        Command::Batch(commands)
    };
    history.execute(batch, &mut doc);

    ToolResult::text(format!(
        "Distributed {moved} nodes in {iterations_done} iterations"
    ))
    .with_data(serde_json::json!({
        "moved": moved,
        "iterations": iterations_done,
        "total_nodes": n,
    }))
}

pub async fn snap_to_pixel(state: &AppState, args: SnapToPixelArgs) -> ToolResult {
    if args.node_ids.is_empty() {
        return ToolResult::error("node_ids must not be empty");
    }

    let mut doc = state.document.lock().await;
    let mut commands: Vec<Command> = Vec::new();
    let mut snapped = 0usize;

    for id in &args.node_ids {
        let node = match doc.nodes.get(id) {
            Some(n) => n.clone(),
            None => return ToolResult::error(format!("Node {} not found", id)),
        };
        let mut updated = node.clone();
        // Round translation components to nearest integer.
        updated.transform.matrix[4] = updated.transform.matrix[4].round();
        updated.transform.matrix[5] = updated.transform.matrix[5].round();
        if (node.transform.matrix[4] - updated.transform.matrix[4]).abs() > 1e-9
            || (node.transform.matrix[5] - updated.transform.matrix[5]).abs() > 1e-9
        {
            commands.push(Command::UpdateNode {
                old: node,
                new: updated,
            });
            snapped += 1;
        }
    }

    if commands.is_empty() {
        return ToolResult::text(format!(
            "{} node(s) already on integer coordinates — no changes made",
            args.node_ids.len()
        ));
    }

    let mut history = state.history.lock().await;
    history.execute(Command::Batch(commands), &mut doc);

    ToolResult::text(format!(
        "Snapped {} of {} node(s) to pixel coordinates",
        snapped,
        args.node_ids.len()
    ))
    .with_data(serde_json::json!({ "snapped_count": snapped }))
}

pub async fn distribute_on_path(state: &AppState, args: DistributeOnPathArgs) -> ToolResult {
    if args.node_ids.is_empty() {
        return ToolResult::error("node_ids must not be empty");
    }

    let doc = state.document.lock().await;

    // Resolve the guide path.
    let path_node = match doc.nodes.get(&args.path_node_id) {
        Some(n) => n.clone(),
        None => return ToolResult::error(format!("path_node_id {} not found", args.path_node_id)),
    };
    let path_data = match &path_node.kind {
        SceneNodeKind::Path(p) => p.path_data.clone(),
        _ => return ToolResult::error("path_node_id must reference a path node"),
    };

    // Validate source nodes exist.
    for id in &args.node_ids {
        if !doc.nodes.contains_key(id) {
            return ToolResult::error(format!("node_id {} not found", id));
        }
    }

    let count = args.count.unwrap_or(args.node_ids.len()).max(1);
    let align = args.align_to_path.unwrap_or(false);
    let target_layer = args
        .layer_id
        .or(Some(path_node.layer_id))
        .or(doc.active_layer_id);

    let positions = path_data.sample_positions(count);
    if positions.is_empty() {
        return ToolResult::error("Path has no geometry to distribute along");
    }

    let mut commands: Vec<Command> = Vec::new();
    let mut new_ids: Vec<uuid::Uuid> = Vec::new();

    for (k, (px, py, angle_deg)) in positions.iter().enumerate() {
        // Cycle through source nodes.
        let src_id = args.node_ids[k % args.node_ids.len()];
        let src = doc.nodes[&src_id].clone();

        let mut new_node = src.clone();
        new_node.id = uuid::Uuid::new_v4();
        new_node.name = format!("{} {}", src.name, k + 1);

        // Position: offset to path sample point.
        new_node.transform.matrix[4] = px + src.transform.matrix[4];
        new_node.transform.matrix[5] = py + src.transform.matrix[5];

        // Align to path tangent if requested.
        if align {
            use std::f64::consts::PI;
            let rad = angle_deg * PI / 180.0;
            let (cos_r, sin_r) = (rad.cos(), rad.sin());
            // Build a pure rotation matrix and compose with existing transform.
            let m = &src.transform.matrix;
            // Apply rotation to [m0,m1,m2,m3] (linear part), keep new translation.
            new_node.transform.matrix[0] = m[0] * cos_r + m[2] * sin_r;
            new_node.transform.matrix[1] = m[1] * cos_r + m[3] * sin_r;
            new_node.transform.matrix[2] = -m[0] * sin_r + m[2] * cos_r;
            new_node.transform.matrix[3] = -m[1] * sin_r + m[3] * cos_r;
        }

        new_ids.push(new_node.id);
        commands.push(Command::AddNode {
            node: new_node,
            layer_id: target_layer,
        });
    }

    drop(doc);
    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    history.execute(Command::Batch(commands), &mut doc);

    ToolResult::text(format!(
        "Distributed {} node(s) across {} position(s) along path '{}'",
        new_ids.len(),
        positions.len(),
        path_node.name
    ))
    .with_data(serde_json::json!({ "node_ids": new_ids, "count": positions.len() }))
}

pub async fn recolor_artwork(state: &AppState, args: RecolorArtworkArgs) -> ToolResult {
    use photonic_core::color::Color;
    use photonic_core::style::FillKind;

    if args.palette.is_empty() {
        return ToolResult::error("palette must contain at least one color");
    }

    // Parse palette.
    let mut palette: Vec<[f32; 4]> = Vec::with_capacity(args.palette.len());
    for hex in &args.palette {
        match Color::from_hex(hex) {
            Some(c) => palette.push([c.r, c.g, c.b, c.a]),
            None => return ToolResult::error(format!("Invalid palette color: '{}'", hex)),
        }
    }

    let mut doc = state.document.lock().await;

    // Determine which nodes to process.
    let ids: Vec<NodeId> = if args.node_ids.is_empty() {
        doc.nodes.keys().cloned().collect()
    } else {
        for id in &args.node_ids {
            if !doc.nodes.contains_key(id) {
                return ToolResult::error(format!("Node {} not found", id));
            }
        }
        args.node_ids.clone()
    };

    // Helper: Euclidean RGB distance.
    fn color_dist(a: [f32; 4], b: [f32; 4]) -> f32 {
        let dr = a[0] - b[0];
        let dg = a[1] - b[1];
        let db = a[2] - b[2];
        dr * dr + dg * dg + db * db
    }
    fn nearest(c: [f32; 4], palette: &[[f32; 4]]) -> [f32; 4] {
        *palette
            .iter()
            .min_by(|a, b| {
                color_dist(c, **a)
                    .partial_cmp(&color_dist(c, **b))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap()
    }

    let mut commands: Vec<Command> = Vec::new();
    let mut recolored = 0usize;

    for id in &ids {
        let node = match doc.nodes.get(id) {
            Some(n) => n.clone(),
            None => continue,
        };
        let pn = match &node.kind {
            SceneNodeKind::Path(p) => p.clone(),
            _ => continue,
        };
        if !pn.fill.enabled {
            continue;
        }
        let orig = match &pn.fill.kind {
            FillKind::Solid(c) => [c.r, c.g, c.b, c.a],
            _ => continue, // Only remap solid fills.
        };
        let target = nearest(orig, &palette);
        if (orig[0] - target[0]).abs() < 1e-6
            && (orig[1] - target[1]).abs() < 1e-6
            && (orig[2] - target[2]).abs() < 1e-6
        {
            continue; // Already that color.
        }
        let mut new_node = node.clone();
        if let SceneNodeKind::Path(ref mut p) = new_node.kind {
            p.fill.kind = FillKind::Solid(Color {
                r: target[0],
                g: target[1],
                b: target[2],
                a: target[3],
            });
        }
        commands.push(Command::UpdateNode {
            old: node,
            new: new_node,
        });
        recolored += 1;
    }

    if commands.is_empty() {
        return ToolResult::text(
            "No fills were remapped — all colors already in palette or no solid fills found",
        );
    }

    let mut history = state.history.lock().await;
    history.execute(Command::Batch(commands), &mut doc);

    ToolResult::text(format!(
        "Recolored {} node(s) to nearest palette colors",
        recolored
    ))
    .with_data(serde_json::json!({ "recolored_count": recolored }))
}

// ─── Guide tools ─────────────────────────────────────────────────────────────

/// Add a ruler guide (horizontal or vertical) at the specified document-unit position.
pub async fn add_guide(state: &AppState, args: AddGuideArgs) -> ToolResult {
    let orientation = match args.orientation.to_lowercase().as_str() {
        "horizontal" => GuideOrientation::Horizontal,
        "vertical" => GuideOrientation::Vertical,
        other => {
            return ToolResult::error(format!(
                "Unknown orientation {:?}; expected \"horizontal\" or \"vertical\"",
                other
            ))
        }
    };

    let mut doc = state.document.lock().await;
    let old_guides = doc.guides.clone();

    let mut guide = Guide::new(orientation, args.position);
    if let Some(c) = args.color {
        guide.color = Some(c);
    }
    let guide_id = guide.id;
    let mut new_guides = old_guides.clone();
    new_guides.push(guide);

    let mut history = state.history.lock().await;
    history.execute(
        Command::SetGuides {
            old: old_guides,
            new: new_guides,
        },
        &mut doc,
    );

    ToolResult::text(format!(
        "Added {} guide at {:.2}",
        args.orientation, args.position
    ))
    .with_data(serde_json::json!({ "guide_id": guide_id }))
}

/// Remove a guide by its UUID.
pub async fn remove_guide(state: &AppState, args: RemoveGuideArgs) -> ToolResult {
    let mut doc = state.document.lock().await;
    let old_guides = doc.guides.clone();

    if !old_guides.iter().any(|g| g.id == args.guide_id) {
        return ToolResult::error(format!("Guide {} not found", args.guide_id));
    }

    let locked = old_guides
        .iter()
        .find(|g| g.id == args.guide_id)
        .map(|g| g.locked)
        .unwrap_or(false);
    if locked {
        return ToolResult::error(format!(
            "Guide {} is locked and cannot be removed",
            args.guide_id
        ));
    }

    let new_guides: Vec<_> = old_guides
        .iter()
        .filter(|g| g.id != args.guide_id)
        .cloned()
        .collect();

    let mut history = state.history.lock().await;
    history.execute(
        Command::SetGuides {
            old: old_guides,
            new: new_guides,
        },
        &mut doc,
    );

    ToolResult::text(format!("Removed guide {}", args.guide_id))
}

/// List all guides in the document.
pub async fn list_guides(state: &AppState, _args: ListGuidesArgs) -> ToolResult {
    let doc = state.document.lock().await;
    let guides: Vec<_> = doc
        .guides
        .iter()
        .map(|g| {
            serde_json::json!({
                "id": g.id,
                "orientation": match g.orientation {
                    GuideOrientation::Horizontal => "horizontal",
                    GuideOrientation::Vertical   => "vertical",
                },
                "position": g.position,
                "locked": g.locked,
                "color": g.color,
            })
        })
        .collect();
    let count = guides.len();
    ToolResult::text(format!("{} guide(s) in document", count))
        .with_data(serde_json::json!({ "guides": guides }))
}

/// Remove all unlocked guides from the document.
pub async fn clear_guides(state: &AppState, _args: ClearGuidesArgs) -> ToolResult {
    let mut doc = state.document.lock().await;
    let old_guides = doc.guides.clone();
    // Keep locked guides; remove everything else.
    let new_guides: Vec<_> = old_guides.iter().filter(|g| g.locked).cloned().collect();
    let removed = old_guides.len() - new_guides.len();

    if removed == 0 {
        return ToolResult::text("No unlocked guides to clear");
    }

    let mut history = state.history.lock().await;
    history.execute(
        Command::SetGuides {
            old: old_guides,
            new: new_guides,
        },
        &mut doc,
    );

    ToolResult::text(format!("Cleared {} guide(s)", removed))
        .with_data(serde_json::json!({ "removed_count": removed }))
}

/// Cut a path node at the point on it nearest to `(canvas_x, canvas_y)`,
/// producing two new open path nodes with the same style as the original.
pub async fn scissors_cut(state: &AppState, args: ScissorsCutArgs) -> ToolResult {
    let mut doc = state.document.lock().await;

    let node = match doc.nodes.get(&args.node_id) {
        Some(n) => n.clone(),
        None => return ToolResult::error(format!("Node {} not found", args.node_id)),
    };

    let pn = match &node.kind {
        SceneNodeKind::Path(pn) => pn.clone(),
        _ => return ToolResult::error("scissors_cut only works on path nodes"),
    };

    if pn.path_data.is_empty() {
        return ToolResult::error("Path has no segments to cut");
    }

    // Transform the canvas point into the node's local coordinate space.
    let inv = node.transform.to_kurbo().inverse();
    let local_pt = inv * kurbo::Point::new(args.canvas_x, args.canvas_y);
    let (lx, ly) = (local_pt.x, local_pt.y);

    let (path_before, path_after) =
        match pn.path_data.split_at_point(lx, ly) {
            Some(pair) => pair,
            None => return ToolResult::error(
                "Could not split path — cut point may be at an endpoint or the path is degenerate",
            ),
        };

    let layer_id = node.layer_id;

    // Build two new nodes inheriting the original's style.
    let mut node_before = SceneNode::new(
        format!("{} (1/2)", node.name),
        layer_id,
        SceneNodeKind::Path(PathNode {
            path_data: path_before,
            ..pn.clone()
        }),
    );
    node_before.transform = node.transform.clone();
    node_before.opacity = node.opacity;
    node_before.blend_mode = node.blend_mode;

    let mut node_after = SceneNode::new(
        format!("{} (2/2)", node.name),
        layer_id,
        SceneNodeKind::Path(PathNode {
            path_data: path_after,
            ..pn.clone()
        }),
    );
    node_after.transform = node.transform.clone();
    node_after.opacity = node.opacity;
    node_after.blend_mode = node.blend_mode;

    let id_before = node_before.id;
    let id_after = node_after.id;

    let mut history = state.history.lock().await;
    history.execute(
        Command::Batch(vec![
            Command::RemoveNode {
                node_id: args.node_id,
            },
            Command::AddNode {
                node: node_before,
                layer_id: Some(layer_id),
            },
            Command::AddNode {
                node: node_after,
                layer_id: Some(layer_id),
            },
        ]),
        &mut doc,
    );

    ToolResult::text(format!("Cut path into 2 open paths")).with_data(serde_json::json!({
        "node_before_id": id_before,
        "node_after_id": id_after,
    }))
}

// ─── magic_wand_select ────────────────────────────────────────────────────────

/// Find the topmost visible node at (canvas_x, canvas_y) and select all nodes
/// that share the specified attribute with it.
pub async fn magic_wand_select(state: &AppState, args: MagicWandSelectArgs) -> ToolResult {
    let tolerance_f64 = args.tolerance.unwrap_or(0.01);
    let tolerance = tolerance_f64 as f32;
    let (cx, cy) = (args.canvas_x, args.canvas_y);

    let mut doc = state.document.lock().await;

    // ── 1. Hit-test: topmost visible unlocked node whose world AABB contains the point ─
    // Nodes are iterated front-to-back (reversed draw order) to pick the topmost.
    let ref_node_id: Option<photonic_core::node::NodeId> = {
        let ordered: Vec<_> = doc.nodes_in_draw_order().into_iter().rev().collect();
        let mut found = None;
        for node in ordered {
            if !node.visible || node.locked {
                continue;
            }
            let (bx0, by0, bx1, by1) = match node_world_aabb(&node) {
                Some(b) => b,
                None => continue,
            };
            if cx >= bx0 && cx <= bx1 && cy >= by0 && cy <= by1 {
                found = Some(node.id);
                break;
            }
        }
        found
    };

    let ref_id = match ref_node_id {
        Some(id) => id,
        None => return ToolResult::error("No node found at the specified canvas coordinates"),
    };

    let ref_node = doc.nodes.get(&ref_id).cloned().unwrap();

    // ── 2. Select all nodes matching the reference attribute ─────────────────
    let mut matched: Vec<photonic_core::node::NodeId> = Vec::new();
    for (nid, node) in &doc.nodes {
        let matches = match args.attribute {
            SelectSameAttribute::FillColor => {
                let ref_color = solid_fill_color(&ref_node);
                let cand_color = solid_fill_color(node);
                match (ref_color, cand_color) {
                    (Some(rc), Some(cc)) => color_distance(rc, cc) <= tolerance,
                    (None, None) => true,
                    _ => false,
                }
            }
            SelectSameAttribute::StrokeColor => {
                if let (SceneNodeKind::Path(rp), SceneNodeKind::Path(cp)) =
                    (&ref_node.kind, &node.kind)
                {
                    match (rp.stroke.enabled, cp.stroke.enabled) {
                        (true, true) => {
                            color_distance(rp.stroke.color, cp.stroke.color) <= tolerance
                        }
                        (false, false) => true,
                        _ => false,
                    }
                } else {
                    false
                }
            }
            SelectSameAttribute::StrokeWeight => {
                if let (SceneNodeKind::Path(rp), SceneNodeKind::Path(cp)) =
                    (&ref_node.kind, &node.kind)
                {
                    (rp.stroke.width - cp.stroke.width).abs() <= tolerance as f64
                } else {
                    false
                }
            }
            SelectSameAttribute::Opacity => (ref_node.opacity - node.opacity).abs() <= tolerance,
            SelectSameAttribute::BlendMode => ref_node.blend_mode == node.blend_mode,
            SelectSameAttribute::ObjectType => {
                std::mem::discriminant(&ref_node.kind) == std::mem::discriminant(&node.kind)
            }
        };
        if matches {
            matched.push(*nid);
        }
    }

    doc.selection.clear();
    for nid in &matched {
        doc.selection.add(*nid);
    }

    let attr_label = match args.attribute {
        SelectSameAttribute::FillColor => "fill color",
        SelectSameAttribute::StrokeColor => "stroke color",
        SelectSameAttribute::StrokeWeight => "stroke weight",
        SelectSameAttribute::Opacity => "opacity",
        SelectSameAttribute::BlendMode => "blend mode",
        SelectSameAttribute::ObjectType => "object type",
    };
    let count = matched.len();
    ToolResult::text(format!(
        "Clicked node: {}. Selected {} node(s) with matching {}.",
        ref_node.name, count, attr_label
    ))
    .with_data(serde_json::json!({
        "clicked_node_id": ref_id,
        "node_ids": matched,
        "count": count,
        "attribute": attr_label,
    }))
}

/// Compute the world-space axis-aligned bounding box of a node using its
/// transform and path bounding box (or a text fallback of 1×1 at origin).
fn node_world_aabb(node: &SceneNode) -> Option<(f64, f64, f64, f64)> {
    let (lx0, ly0, lx1, ly1) = match &node.kind {
        SceneNodeKind::Path(pn) => {
            let r = pn.path_data.bounding_box()?;
            (r.x0, r.y0, r.x1, r.y1)
        }
        SceneNodeKind::Text(_) => (0.0, 0.0, 1.0, 1.0),
        SceneNodeKind::Group(_) => (0.0, 0.0, 1.0, 1.0),
    };
    // Transform all four corners of the local AABB and compute the world AABB.
    let fwd = node.transform.to_kurbo();
    let corners = [
        fwd * kurbo::Point::new(lx0, ly0),
        fwd * kurbo::Point::new(lx1, ly0),
        fwd * kurbo::Point::new(lx0, ly1),
        fwd * kurbo::Point::new(lx1, ly1),
    ];
    let wx0 = corners.iter().map(|p| p.x).fold(f64::MAX, f64::min);
    let wy0 = corners.iter().map(|p| p.y).fold(f64::MAX, f64::min);
    let wx1 = corners.iter().map(|p| p.x).fold(f64::MIN, f64::max);
    let wy1 = corners.iter().map(|p| p.y).fold(f64::MIN, f64::max);
    Some((wx0, wy0, wx1, wy1))
}

// ─── convert_anchor_points ────────────────────────────────────────────────────

/// Convert all cubic anchor points in selected path nodes to smooth or corner joins.
pub async fn convert_anchor_points(state: &AppState, args: ConvertAnchorPointsArgs) -> ToolResult {
    if args.node_ids.is_empty() {
        return ToolResult::error("node_ids must not be empty");
    }

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let mut cmds: Vec<Command> = Vec::new();
    let mut skipped = 0usize;
    let mut converted = 0usize;

    for &nid in &args.node_ids {
        let node = match doc.nodes.get(&nid) {
            Some(n) => n.clone(),
            None => {
                skipped += 1;
                continue;
            }
        };
        let pn = match &node.kind {
            SceneNodeKind::Path(p) => p.clone(),
            _ => {
                skipped += 1;
                continue;
            }
        };

        let new_path = match args.mode {
            ConvertAnchorMode::Smooth => pn.path_data.convert_to_smooth(),
            ConvertAnchorMode::Corner => pn.path_data.convert_to_corner(),
        };

        let mut new_node = node.clone();
        if let SceneNodeKind::Path(ref mut np) = new_node.kind {
            np.path_data = new_path;
        }
        cmds.push(Command::UpdateNode {
            old: node,
            new: new_node,
        });
        converted += 1;
    }

    if cmds.is_empty() {
        return ToolResult::error("No path nodes found in node_ids");
    }

    let cmd = if cmds.len() == 1 {
        cmds.remove(0)
    } else {
        Command::Batch(cmds)
    };
    history.execute(cmd, &mut doc);

    let mode_label = match args.mode {
        ConvertAnchorMode::Smooth => "smooth",
        ConvertAnchorMode::Corner => "corner",
    };
    ToolResult::text(format!(
        "Converted {} node(s) to {} anchors ({} skipped).",
        converted, mode_label, skipped
    ))
    .with_data(serde_json::json!({
        "converted": converted,
        "skipped": skipped,
        "mode": mode_label,
    }))
}

// ─── lasso_select ─────────────────────────────────────────────────────────────

/// Select nodes whose bounding-box centroid (or any corner) lies inside the
/// given canvas-space polygon.
pub async fn lasso_select(state: &AppState, args: LassoSelectArgs) -> ToolResult {
    if args.points.len() < 3 {
        return ToolResult::error(
            "lasso_select requires at least 3 points to form a closed polygon",
        );
    }

    let mut doc = state.document.lock().await;

    let poly: Vec<[f64; 2]> = args.points.clone();
    let mut selected_ids: Vec<photonic_core::node::NodeId> = Vec::new();

    for node in doc.nodes_in_draw_order() {
        if !node.visible {
            continue;
        }
        let (wx0, wy0, wx1, wy1) = match node_world_aabb(node) {
            Some(b) => b,
            None => continue,
        };

        let inside = if args.centroid_mode {
            // Check if the AABB centroid is inside the polygon.
            let cx = (wx0 + wx1) / 2.0;
            let cy = (wy0 + wy1) / 2.0;
            point_in_polygon(cx, cy, &poly)
        } else {
            // Check if any AABB corner is inside the polygon.
            let corners = [(wx0, wy0), (wx1, wy0), (wx0, wy1), (wx1, wy1)];
            corners.iter().any(|&(x, y)| point_in_polygon(x, y, &poly))
        };

        if inside {
            selected_ids.push(node.id);
        }
    }

    if !args.additive {
        doc.selection.clear();
    }
    for nid in &selected_ids {
        doc.selection.add(*nid);
    }

    let count = selected_ids.len();
    ToolResult::text(format!("Lasso selected {} node(s).", count)).with_data(serde_json::json!({
        "node_ids": selected_ids,
        "count": count,
    }))
}

// ─── select_by_kind ──────────────────────────────────────────────────────────

/// Select all nodes whose kind matches the specified filter.
pub async fn select_by_kind(state: &AppState, args: SelectByKindArgs) -> ToolResult {
    let mut doc = state.document.lock().await;

    let active_layer = doc.active_layer_id;

    let matching: Vec<NodeId> = doc
        .nodes
        .iter()
        .filter(|(_, node)| match &args.kind {
            ObjectKindFilter::Path => matches!(node.kind, SceneNodeKind::Path(_)),
            ObjectKindFilter::Text => matches!(node.kind, SceneNodeKind::Text(_)),
            ObjectKindFilter::Group => matches!(node.kind, SceneNodeKind::Group(_)),
            ObjectKindFilter::SameLayer => active_layer
                .map(|lid| node.layer_id == lid)
                .unwrap_or(false),
        })
        .map(|(id, _)| *id)
        .collect();

    if !args.additive {
        doc.selection.clear();
    }
    let count = matching.len();
    for nid in &matching {
        doc.selection.add(*nid);
    }

    ToolResult::text(format!(
        "Selected {} {} node(s)",
        count,
        format!("{:?}", args.kind).to_lowercase()
    ))
    .with_data(serde_json::json!({
        "selected_count": count,
        "node_ids": matching,
    }))
}

// ─── create_freehand_path ────────────────────────────────────────────────────

/// Create a freehand polyline path from an ordered list of canvas-space points.
pub async fn create_freehand_path(state: &AppState, args: CreateFreehandPathArgs) -> ToolResult {
    if args.points.len() < 2 {
        return ToolResult::error("create_freehand_path requires at least 2 points");
    }

    // Build SVG path string.
    let first = args.points[0];
    let mut svg = format!("M {:.4} {:.4}", first[0], first[1]);
    for pt in &args.points[1..] {
        svg.push_str(&format!(" L {:.4} {:.4}", pt[0], pt[1]));
    }
    let path_data = match PathData::from_svg(&svg) {
        Ok(p) => p,
        Err(e) => return ToolResult::error(format!("Failed to build path: {}", e)),
    };

    let mut path_node = PathNode::new(path_data);
    if let Err(e) = apply_style(&mut path_node, args.fill, args.stroke) {
        return ToolResult::error(e);
    }

    let name = args.name.unwrap_or_else(|| "Pencil".to_string());
    let mut doc = state.document.lock().await;
    let node_id;
    {
        let node = SceneNode::new(&name, uuid::Uuid::nil(), SceneNodeKind::Path(path_node));
        node_id = node.id;
        let cmd = Command::AddNode {
            node,
            layer_id: None,
        };
        let mut history = state.history.lock().await;
        history.execute(cmd, &mut doc);
    }

    ToolResult::text(format!(
        "Created freehand path '{}' ({} points, id: {})",
        name,
        args.points.len(),
        node_id
    ))
    .with_data(serde_json::json!({ "node_id": node_id }))
}

// ─── Isolation Mode ──────────────────────────────────────────────────────────

/// Select all children of the group — the MCP-observable effect of entering Isolation Mode.
pub async fn enter_isolation_mode(state: &AppState, args: EnterIsolationModeArgs) -> ToolResult {
    let mut doc = state.document.lock().await;
    let children = match doc.nodes.get(&args.group_id) {
        Some(node) => {
            if let SceneNodeKind::Group(g) = &node.kind {
                if g.children.is_empty() {
                    return ToolResult::text(format!("Group {} has no children", args.group_id));
                }
                g.children.clone()
            } else {
                return ToolResult::error(format!("Node {} is not a group", args.group_id));
            }
        }
        None => return ToolResult::error(format!("No node found with id {}", args.group_id)),
    };

    doc.selection.clear();
    for cid in &children {
        doc.selection.add(*cid);
    }

    ToolResult::text(format!(
        "Entered isolation mode for group {} — {} child node(s) selected",
        args.group_id,
        children.len()
    ))
    .with_data(serde_json::json!({
        "group_id": args.group_id,
        "child_count": children.len(),
        "children": children,
    }))
}

/// Exit Isolation Mode — clears the current selection.
pub async fn exit_isolation_mode(state: &AppState, _args: ExitIsolationModeArgs) -> ToolResult {
    let mut doc = state.document.lock().await;
    doc.selection.clear();
    ToolResult::text("Exited isolation mode. Selection cleared.")
}

// ─── select_inside_group ─────────────────────────────────────────────────────

/// Replace the selection with the direct children of the specified group node.
pub async fn select_inside_group(state: &AppState, args: SelectInsideGroupArgs) -> ToolResult {
    let mut doc = state.document.lock().await;
    let group_id = args.group_id;

    let children = match doc.nodes.get(&group_id) {
        Some(node) => {
            if let SceneNodeKind::Group(g) = &node.kind {
                g.children.clone()
            } else {
                return ToolResult::error(format!(
                    "Node {} is not a group (kind: {:?})",
                    group_id,
                    std::mem::discriminant(&node.kind)
                ));
            }
        }
        None => return ToolResult::error(format!("No node found with id {}", group_id)),
    };

    if children.is_empty() {
        return ToolResult::text(format!("Group {} has no children", group_id));
    }

    if !args.additive {
        doc.selection.clear();
    }
    for cid in &children {
        doc.selection.add(*cid);
    }

    ToolResult::text(format!(
        "Selected {} child node(s) inside group {}",
        children.len(),
        group_id
    ))
    .with_data(serde_json::json!({
        "group_id": group_id,
        "selected_count": children.len(),
        "selected_ids": children,
    }))
}

// ─── get_recent_colors ───────────────────────────────────────────────────────

pub async fn get_recent_colors(state: &AppState, _args: GetRecentColorsArgs) -> ToolResult {
    let doc = state.document.lock().await;
    let colors: Vec<serde_json::Value> = doc
        .recent_colors
        .iter()
        .map(|c| serde_json::json!({ "r": c.r, "g": c.g, "b": c.b, "a": c.a }))
        .collect();
    ToolResult::text(format!("{} recent color(s)", colors.len())).with_data(serde_json::json!({
        "count": colors.len(),
        "colors": colors,
    }))
}

/// Ray-casting point-in-polygon test (Jordan curve theorem).
/// Returns true when `(px, py)` is strictly inside the polygon.
fn point_in_polygon(px: f64, py: f64, poly: &[[f64; 2]]) -> bool {
    let n = poly.len();
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let xi = poly[i][0];
        let yi = poly[i][1];
        let xj = poly[j][0];
        let yj = poly[j][1];
        if ((yi > py) != (yj > py)) && (px < (xj - xi) * (py - yi) / (yj - yi) + xi) {
            inside = !inside;
        }
        j = i;
    }
    inside
}

// ─── smooth_path ─────────────────────────────────────────────────────────────

/// Smooth path nodes using Chaikin's corner-cutting algorithm.
pub async fn smooth_path(state: &AppState, args: SmoothPathArgs) -> ToolResult {
    let mut doc = state.document.lock().await;
    let factor = args.factor.clamp(0.0, 0.5);
    let iterations = args.iterations.min(8);

    let ids: Vec<NodeId> = if args.node_ids.is_empty() {
        doc.selection.ids().copied().collect()
    } else {
        args.node_ids
    };

    if ids.is_empty() {
        return ToolResult::text("No nodes specified or selected.");
    }

    let mut cmds = Vec::new();
    let mut smoothed = 0usize;
    for id in &ids {
        if let Some(node) = doc.nodes.get(id) {
            if let SceneNodeKind::Path(pn) = &node.kind {
                let new_path = pn.path_data.smooth(factor, iterations);
                let mut new_node = node.clone();
                if let SceneNodeKind::Path(ref mut new_pn) = new_node.kind {
                    new_pn.path_data = new_path;
                }
                cmds.push(Command::UpdateNode {
                    old: node.clone(),
                    new: new_node,
                });
                smoothed += 1;
            }
        }
    }

    if cmds.is_empty() {
        return ToolResult::text("No path nodes found in the specified IDs.");
    }

    let batch = if cmds.len() == 1 {
        cmds.remove(0)
    } else {
        Command::Batch(cmds)
    };
    let mut history = state.history.lock().await;
    history.execute(batch, &mut doc);

    ToolResult::text(format!(
        "Smoothed {} path node(s) with factor={:.2}, iterations={}.",
        smoothed, factor, iterations
    ))
}

// ─── noise_deform ─────────────────────────────────────────────────────────────

/// Displace every anchor point and control point in selected paths using
/// a smooth sinusoidal field, producing organic wave-like deformation.
pub async fn noise_deform(state: &AppState, args: NoiseDeformArgs) -> ToolResult {
    tracing::debug!("tool: noise_deform");

    if args.node_ids.is_empty() {
        return ToolResult::error("node_ids must not be empty");
    }

    let amplitude = args.amplitude.unwrap_or(8.0);
    let frequency = args.frequency.unwrap_or(0.05);
    let seed = args.seed.unwrap_or(0.0);
    let axis = args.axis.as_deref().unwrap_or("both");

    let deform_x = axis == "both" || axis == "x";
    let deform_y = axis == "both" || axis == "y";

    // Displace a single point using two-octave sinusoidal noise.
    let displace = |pt: kurbo::Point| -> kurbo::Point {
        let dx = if deform_x {
            amplitude * (pt.y * frequency + seed).sin()
                + (amplitude * 0.5) * (pt.y * frequency * 2.1 + seed * 1.3).sin()
        } else {
            0.0
        };
        let dy = if deform_y {
            amplitude * (pt.x * frequency + seed + std::f64::consts::FRAC_PI_2).sin()
                + (amplitude * 0.5) * (pt.x * frequency * 2.1 + seed * 1.7).sin()
        } else {
            0.0
        };
        kurbo::Point::new(pt.x + dx, pt.y + dy)
    };

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    let mut commands = Vec::new();
    let mut modified = 0usize;
    let mut skipped = 0usize;

    for node_id_str in &args.node_ids {
        let nid = match uuid::Uuid::parse_str(node_id_str) {
            Ok(id) => id,
            Err(_) => match doc.find_node_by_name(node_id_str) {
                Some(n) => n.id,
                None => {
                    skipped += 1;
                    continue;
                }
            },
        };
        let node = match doc.nodes.get(&nid) {
            Some(n) => n.clone(),
            None => {
                skipped += 1;
                continue;
            }
        };
        let pn = match &node.kind {
            SceneNodeKind::Path(pn) => pn,
            _ => {
                skipped += 1;
                continue;
            }
        };

        let bez = pn.path_data.to_bez_path();

        let new_els: Vec<kurbo::PathEl> = bez
            .iter()
            .map(|el| match el {
                kurbo::PathEl::MoveTo(p) => kurbo::PathEl::MoveTo(displace(p)),
                kurbo::PathEl::LineTo(p) => kurbo::PathEl::LineTo(displace(p)),
                kurbo::PathEl::QuadTo(p1, p2) => kurbo::PathEl::QuadTo(displace(p1), displace(p2)),
                kurbo::PathEl::CurveTo(p1, p2, p3) => {
                    kurbo::PathEl::CurveTo(displace(p1), displace(p2), displace(p3))
                }
                kurbo::PathEl::ClosePath => kurbo::PathEl::ClosePath,
            })
            .collect();

        let new_bez = kurbo::BezPath::from_vec(new_els);
        let mut new_node = node.clone();
        if let SceneNodeKind::Path(ref mut new_pn) = new_node.kind {
            new_pn.path_data = PathData::from_bez_path(&new_bez);
        }
        commands.push(Command::UpdateNode {
            old: node,
            new: new_node,
        });
        modified += 1;
    }

    if commands.is_empty() {
        return ToolResult::error("No path nodes found in node_ids");
    }

    for cmd in commands {
        history.execute(cmd, &mut doc);
    }

    ToolResult::text(format!(
        "Noise-deformed {} path node(s) (amplitude={:.1}, frequency={:.4}, axis={}, seed={:.2}). Skipped: {}.",
        modified, amplitude, frequency, axis, seed, skipped
    ))
}

// ─── mirror_copy ──────────────────────────────────────────────────────────────

/// Duplicate selected nodes and flip each copy across its own bounding-box
/// center, producing mirrored twins that can be repositioned independently.
pub async fn mirror_copy(state: &AppState, args: MirrorCopyArgs) -> ToolResult {
    tracing::debug!("tool: mirror_copy");
    use kurbo::Shape as _;

    let flip_h = args.axis.as_deref().unwrap_or("horizontal") != "vertical";
    // flip_h = true  → flip left/right (mirror across vertical axis)
    // flip_h = false → flip top/bottom (mirror across horizontal axis)

    // Collect source node IDs.
    let src_ids: Vec<NodeId> = {
        let doc = state.document.lock().await;
        if args.node_ids.is_empty() {
            doc.selection.node_ids.iter().copied().collect()
        } else {
            args.node_ids
                .iter()
                .filter_map(|s| {
                    uuid::Uuid::parse_str(s)
                        .ok()
                        .or_else(|| doc.find_node_by_name(s).map(|n| n.id))
                })
                .collect()
        }
    };

    if src_ids.is_empty() {
        return ToolResult::error("No nodes specified and nothing selected");
    }

    // Build clones using the existing subtree helper.
    let mut all_commands: Vec<Command> = Vec::new();
    let mut new_root_ids: Vec<uuid::Uuid> = Vec::new();

    for src_id in &src_ids {
        let (cloned_nodes, layer_id) = {
            let doc = state.document.lock().await;
            let layer = doc
                .nodes
                .get(src_id)
                .map(|n| n.layer_id)
                .unwrap_or_else(uuid::Uuid::nil);
            let nodes = clone_subtree(&doc, *src_id, layer, 0.0, 0.0);
            (nodes, layer)
        };

        if cloned_nodes.is_empty() {
            continue;
        }

        // Flip the root node's geometry.
        let mut modified = cloned_nodes;
        {
            let root = &mut modified[0];
            // Build a friendly name
            root.name = if root.name.is_empty() {
                "mirror".to_string()
            } else {
                format!("{} mirror", root.name)
            };

            match &mut root.kind {
                SceneNodeKind::Path(pn) => {
                    let bez = pn.path_data.to_bez_path();
                    let bbox = bez.bounding_box();
                    let cx = bbox.x0 + bbox.width() / 2.0;
                    let cy = bbox.y0 + bbox.height() / 2.0;

                    let flip_pt = |p: kurbo::Point| {
                        kurbo::Point::new(
                            if flip_h { 2.0 * cx - p.x } else { p.x },
                            if !flip_h { 2.0 * cy - p.y } else { p.y },
                        )
                    };

                    let mut new_bez = kurbo::BezPath::new();
                    for el in bez.elements() {
                        match *el {
                            kurbo::PathEl::MoveTo(p) => new_bez.move_to(flip_pt(p)),
                            kurbo::PathEl::LineTo(p) => new_bez.line_to(flip_pt(p)),
                            kurbo::PathEl::CurveTo(c1, c2, p) => {
                                new_bez.curve_to(flip_pt(c1), flip_pt(c2), flip_pt(p))
                            }
                            kurbo::PathEl::QuadTo(c, p) => new_bez.quad_to(flip_pt(c), flip_pt(p)),
                            kurbo::PathEl::ClosePath => new_bez.close_path(),
                        }
                    }
                    pn.path_data = PathData::from_bez_path(&new_bez);
                }
                SceneNodeKind::Text(_) | SceneNodeKind::Group(_) => {
                    if flip_h {
                        root.transform.matrix[0] *= -1.0;
                        root.transform.matrix[2] *= -1.0;
                    } else {
                        root.transform.matrix[1] *= -1.0;
                        root.transform.matrix[3] *= -1.0;
                    }
                }
            }

            new_root_ids.push(root.id);
        }

        for node in modified {
            all_commands.push(Command::AddNode {
                layer_id: Some(layer_id),
                node,
            });
        }
    }

    if all_commands.is_empty() {
        return ToolResult::error("No nodes found to mirror");
    }

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    let batch = if all_commands.len() == 1 {
        all_commands.remove(0)
    } else {
        Command::Batch(all_commands)
    };
    history.execute(batch, &mut doc);

    ToolResult::text(format!(
        "Created {} mirrored cop{} ({}). New node IDs: {}",
        new_root_ids.len(),
        if new_root_ids.len() == 1 { "y" } else { "ies" },
        if flip_h { "horizontally" } else { "vertically" },
        new_root_ids
            .iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    ))
    .with_data(serde_json::json!({
        "node_ids": new_root_ids.iter().map(|id| id.to_string()).collect::<Vec<_>>(),
        "axis": if flip_h { "horizontal" } else { "vertical" },
    }))
}

// ─── pin_object_guides ────────────────────────────────────────────────────────

/// Create persistent guide lines at the edges and/or center of selected nodes,
/// making key alignments permanent reference markers visible during editing.
pub async fn pin_object_guides(state: &AppState, args: PinObjectGuidesArgs) -> ToolResult {
    tracing::debug!("tool: pin_object_guides");
    use kurbo::Shape as _;

    // Parse requested edges.
    let edge_spec = args.edges.as_deref().unwrap_or("all");
    let all = edge_spec == "all";
    let edges = edge_spec == "edges"; // top + bottom + left + right only
    let center = edge_spec == "center"; // center_h + center_v only
    let want_top = all || edges || edge_spec.contains("top");
    let want_bottom = all || edges || edge_spec.contains("bottom");
    let want_left = all || edges || edge_spec.contains("left");
    let want_right = all || edges || edge_spec.contains("right");
    let want_center_h = all || center || edge_spec.contains("center_h");
    let want_center_v = all || center || edge_spec.contains("center_v");

    let mut doc = state.document.lock().await;

    let node_ids: Vec<NodeId> = if args.node_ids.is_empty() {
        doc.selection.node_ids.iter().copied().collect()
    } else {
        args.node_ids
            .iter()
            .filter_map(|s| {
                uuid::Uuid::parse_str(s)
                    .ok()
                    .or_else(|| doc.find_node_by_name(s).map(|n| n.id))
            })
            .collect()
    };

    if node_ids.is_empty() {
        return ToolResult::error("No nodes specified and nothing selected");
    }

    let tolerance = 0.5_f64; // deduplicate guides within 0.5 px

    // Helper: add guide only if no guide at this position+orientation exists.
    let mut new_guides: Vec<Guide> = Vec::new();

    let add_h = |pos: f64, new_guides: &mut Vec<Guide>, doc_guides: &[Guide]| {
        let exists = doc_guides.iter().chain(new_guides.iter()).any(|g| {
            g.orientation == GuideOrientation::Horizontal && (g.position - pos).abs() < tolerance
        });
        if !exists {
            new_guides.push(Guide::new(GuideOrientation::Horizontal, pos));
        }
    };

    let add_v = |pos: f64, new_guides: &mut Vec<Guide>, doc_guides: &[Guide]| {
        let exists = doc_guides.iter().chain(new_guides.iter()).any(|g| {
            g.orientation == GuideOrientation::Vertical && (g.position - pos).abs() < tolerance
        });
        if !exists {
            new_guides.push(Guide::new(GuideOrientation::Vertical, pos));
        }
    };

    for nid in &node_ids {
        if let Some(node) = doc.nodes.get(nid) {
            let tx = node.transform.matrix[4];
            let ty = node.transform.matrix[5];

            let (x0, y0, x1, y1) = match &node.kind {
                SceneNodeKind::Path(pn) => {
                    let bez = pn.path_data.to_bez_path();
                    let bb = bez.bounding_box();
                    (bb.x0 + tx, bb.y0 + ty, bb.x1 + tx, bb.y1 + ty)
                }
                _ => continue,
            };

            if want_top {
                add_h(y0, &mut new_guides, &doc.guides);
            }
            if want_bottom {
                add_h(y1, &mut new_guides, &doc.guides);
            }
            if want_center_h {
                add_h((y0 + y1) / 2.0, &mut new_guides, &doc.guides);
            }
            if want_left {
                add_v(x0, &mut new_guides, &doc.guides);
            }
            if want_right {
                add_v(x1, &mut new_guides, &doc.guides);
            }
            if want_center_v {
                add_v((x0 + x1) / 2.0, &mut new_guides, &doc.guides);
            }
        }
    }

    let added = new_guides.len();
    doc.guides.extend(new_guides);

    if added == 0 {
        ToolResult::text("No new guides added — all positions already have existing guides.")
    } else {
        ToolResult::text(format!(
            "Pinned {} guide(s) from {} node(s).",
            added,
            node_ids.len()
        ))
        .with_data(serde_json::json!({ "guides_added": added }))
    }
}

// ─── reverse_node_order ───────────────────────────────────────────────────────

/// Reverse the stacking order of children within each selected group node.
/// Useful to flip front-to-back ordering of blend results or any grouped artwork.
pub async fn reverse_node_order(state: &AppState, args: ReverseNodeOrderArgs) -> ToolResult {
    tracing::debug!("tool: reverse_node_order");

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let node_ids: Vec<NodeId> = if args.node_ids.is_empty() {
        doc.selection.node_ids.iter().copied().collect()
    } else {
        args.node_ids
            .iter()
            .filter_map(|s| {
                uuid::Uuid::parse_str(s)
                    .ok()
                    .or_else(|| doc.find_node_by_name(s).map(|n| n.id))
            })
            .collect()
    };

    if node_ids.is_empty() {
        return ToolResult::error("No nodes specified and nothing selected");
    }

    let mut reversed = 0usize;
    let mut skipped = 0usize;
    let mut commands = Vec::new();

    for nid in &node_ids {
        let node = match doc.nodes.get(nid) {
            Some(n) => n.clone(),
            None => {
                skipped += 1;
                continue;
            }
        };
        match &node.kind {
            SceneNodeKind::Group(g) if g.children.len() > 1 => {
                let mut new_node = node.clone();
                if let SceneNodeKind::Group(ref mut ng) = new_node.kind {
                    ng.children.reverse();
                }
                commands.push(Command::UpdateNode {
                    old: node,
                    new: new_node,
                });
                reversed += 1;
            }
            SceneNodeKind::Group(_) => {
                skipped += 1;
            } // 0 or 1 children — no-op
            _ => {
                skipped += 1;
            }
        }
    }

    if commands.is_empty() {
        return ToolResult::error("No group nodes with 2+ children found in the specified IDs");
    }

    let batch = if commands.len() == 1 {
        commands.remove(0)
    } else {
        Command::Batch(commands)
    };
    history.execute(batch, &mut doc);

    ToolResult::text(format!(
        "Reversed child order in {} group node(s). Skipped: {}.",
        reversed, skipped
    ))
}

// ─── prompt history ───────────────────────────────────────────────────────────

/// Record an AI prompt on a node's prompt_history field for provenance tracking.
pub async fn set_node_prompt(state: &AppState, args: SetNodePromptArgs) -> ToolResult {
    tracing::debug!("tool: set_node_prompt");

    if args.prompt.trim().is_empty() {
        return ToolResult::error("prompt must not be empty");
    }

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let nid = match uuid::Uuid::parse_str(&args.node_id) {
        Ok(id) => id,
        Err(_) => match doc.find_node_by_name(&args.node_id) {
            Some(n) => n.id,
            None => return ToolResult::error(format!("Node '{}' not found", args.node_id)),
        },
    };

    let node = match doc.nodes.get(&nid) {
        Some(n) => n.clone(),
        None => return ToolResult::error(format!("Node {} not found", nid)),
    };

    let mut new_node = node.clone();
    let mode = args.mode.as_deref().unwrap_or("append");
    match mode {
        "replace" => {
            new_node.prompt_history = vec![args.prompt.clone()];
        }
        "prepend" => {
            new_node.prompt_history.insert(0, args.prompt.clone());
        }
        _ => {
            // "append" and anything else
            new_node.prompt_history.push(args.prompt.clone());
        }
    }

    let entry_count = new_node.prompt_history.len();
    history.execute(
        Command::UpdateNode {
            old: node,
            new: new_node,
        },
        &mut doc,
    );

    ToolResult::text(format!(
        "Recorded prompt on node '{}' ({} mode). History length: {}.",
        args.node_id, mode, entry_count
    ))
}

/// Return the full prompt history for a node.
pub async fn get_node_prompts(state: &AppState, args: GetNodePromptsArgs) -> ToolResult {
    tracing::debug!("tool: get_node_prompts");

    let doc = state.document.lock().await;
    let nid = match uuid::Uuid::parse_str(&args.node_id) {
        Ok(id) => id,
        Err(_) => match doc.find_node_by_name(&args.node_id) {
            Some(n) => n.id,
            None => return ToolResult::error(format!("Node '{}' not found", args.node_id)),
        },
    };

    let node = match doc.nodes.get(&nid) {
        Some(n) => n,
        None => return ToolResult::error(format!("Node {} not found", nid)),
    };

    if node.prompt_history.is_empty() {
        return ToolResult::text(format!("Node '{}' has no prompt history.", node.name));
    }

    let prompts: Vec<serde_json::Value> = node
        .prompt_history
        .iter()
        .enumerate()
        .map(|(i, p)| serde_json::json!({ "index": i, "prompt": p }))
        .collect();

    ToolResult::text(format!(
        "Node '{}' has {} prompt(s) in history.",
        node.name,
        prompts.len()
    ))
    .with_data(serde_json::json!({
        "node_id": nid.to_string(),
        "node_name": node.name,
        "prompts": prompts,
    }))
}

// ─── Select Similar ───────────────────────────────────────────────────────────

/// Select all nodes in the document whose visual attributes match those of the
/// reference node(s). Implements Illustrator's "Select > Same > …" and
/// "Global Edit" behaviour.
pub async fn select_similar(state: &AppState, args: SelectSimilarArgs) -> ToolResult {
    tracing::debug!("tool: select_similar");
    use photonic_core::style::FillKind;

    let mut doc = state.document.lock().await;

    // Resolve reference IDs.
    let ref_ids: Vec<NodeId> = if args.node_ids.is_empty() {
        doc.selection.node_ids.iter().copied().collect()
    } else {
        args.node_ids
            .iter()
            .filter_map(|s| {
                uuid::Uuid::parse_str(s)
                    .ok()
                    .or_else(|| doc.nodes.values().find(|n| n.name == *s).map(|n| n.id))
            })
            .collect()
    };

    if ref_ids.is_empty() {
        return ToolResult::error("No reference nodes — pass node_ids or make a selection first");
    }

    let match_by = args.match_by.as_deref().unwrap_or("fill_color");
    let tol = args.tolerance.unwrap_or(5) as i32;

    // Color tolerance as f32 fraction (tol is 0-255 scale → convert to 0-1 scale).
    let tol_f = tol as f32 / 255.0;

    // Collect attributes from reference nodes.
    let mut ref_fill_colors: Vec<[f32; 3]> = Vec::new();
    let mut ref_stroke_colors: Vec<[f32; 3]> = Vec::new();
    let mut ref_stroke_widths: Vec<f64> = Vec::new();
    let mut ref_opacities: Vec<f32> = Vec::new();
    let mut ref_kinds: Vec<&'static str> = Vec::new();

    for rid in &ref_ids {
        if let Some(node) = doc.nodes.get(rid) {
            ref_opacities.push(node.opacity);
            match &node.kind {
                SceneNodeKind::Path(p) => {
                    ref_kinds.push("path");
                    if p.fill.enabled {
                        if let FillKind::Solid(c) = &p.fill.kind {
                            ref_fill_colors.push([c.r, c.g, c.b]);
                        }
                    }
                    if p.stroke.enabled {
                        ref_stroke_colors.push([
                            p.stroke.color.r,
                            p.stroke.color.g,
                            p.stroke.color.b,
                        ]);
                        ref_stroke_widths.push(p.stroke.width);
                    }
                }
                SceneNodeKind::Text(t) => {
                    ref_kinds.push("text");
                    if t.fill.enabled {
                        if let FillKind::Solid(c) = &t.fill.kind {
                            ref_fill_colors.push([c.r, c.g, c.b]);
                        }
                    }
                    if t.stroke.enabled {
                        ref_stroke_colors.push([
                            t.stroke.color.r,
                            t.stroke.color.g,
                            t.stroke.color.b,
                        ]);
                        ref_stroke_widths.push(t.stroke.width);
                    }
                }
                SceneNodeKind::Group(_) => {
                    ref_kinds.push("group");
                }
            }
        }
    }

    // Helper closures.
    let color_matches = |a: [f32; 3], ref_colors: &[[f32; 3]]| -> bool {
        ref_colors.iter().any(|rc| {
            (a[0] - rc[0]).abs() <= tol_f
                && (a[1] - rc[1]).abs() <= tol_f
                && (a[2] - rc[2]).abs() <= tol_f
        })
    };

    let criteria: Vec<&str> = match_by.split(',').map(|s| s.trim()).collect();

    // Collect all matching node IDs.
    let all_ids: Vec<NodeId> = doc.nodes.keys().copied().collect();
    let mut matched: Vec<NodeId> = Vec::new();

    for nid in &all_ids {
        if ref_ids.contains(nid) {
            continue;
        } // skip the reference itself
        let node = match doc.nodes.get(nid) {
            Some(n) => n,
            None => continue,
        };

        let mut node_matches = true;
        for criterion in &criteria {
            let ok = match *criterion {
                "fill_color" => match &node.kind {
                    SceneNodeKind::Path(p) => {
                        if p.fill.enabled {
                            if let FillKind::Solid(c) = &p.fill.kind {
                                color_matches([c.r, c.g, c.b], &ref_fill_colors)
                            } else {
                                false
                            }
                        } else {
                            ref_fill_colors.is_empty()
                        }
                    }
                    SceneNodeKind::Text(t) => {
                        if t.fill.enabled {
                            if let FillKind::Solid(c) = &t.fill.kind {
                                color_matches([c.r, c.g, c.b], &ref_fill_colors)
                            } else {
                                false
                            }
                        } else {
                            ref_fill_colors.is_empty()
                        }
                    }
                    SceneNodeKind::Group(_) => false,
                },
                "stroke_color" => match &node.kind {
                    SceneNodeKind::Path(p) => {
                        if p.stroke.enabled {
                            color_matches(
                                [p.stroke.color.r, p.stroke.color.g, p.stroke.color.b],
                                &ref_stroke_colors,
                            )
                        } else {
                            false
                        }
                    }
                    SceneNodeKind::Text(t) => {
                        if t.stroke.enabled {
                            color_matches(
                                [t.stroke.color.r, t.stroke.color.g, t.stroke.color.b],
                                &ref_stroke_colors,
                            )
                        } else {
                            false
                        }
                    }
                    SceneNodeKind::Group(_) => false,
                },
                "stroke_width" => match &node.kind {
                    SceneNodeKind::Path(p) => {
                        if p.stroke.enabled {
                            ref_stroke_widths
                                .iter()
                                .any(|&rw| (p.stroke.width - rw).abs() < 0.01)
                        } else {
                            false
                        }
                    }
                    SceneNodeKind::Text(t) => {
                        if t.stroke.enabled {
                            ref_stroke_widths
                                .iter()
                                .any(|&rw| (t.stroke.width - rw).abs() < 0.01)
                        } else {
                            false
                        }
                    }
                    SceneNodeKind::Group(_) => false,
                },
                "kind" => {
                    let k = match &node.kind {
                        SceneNodeKind::Path(_) => "path",
                        SceneNodeKind::Text(_) => "text",
                        SceneNodeKind::Group(_) => "group",
                    };
                    ref_kinds.contains(&k)
                }
                "opacity" => ref_opacities
                    .iter()
                    .any(|&ro| (node.opacity - ro).abs() < 0.01_f32),
                "tags" => {
                    // Match if any ref node shares at least one tag with this node.
                    let node_tags: std::collections::HashSet<_> = node.tags.iter().collect();
                    ref_ids.iter().any(|rid| {
                        if let Some(rn) = doc.nodes.get(rid) {
                            rn.tags.iter().any(|t| node_tags.contains(t))
                        } else {
                            false
                        }
                    })
                }
                _ => true, // unknown criterion — ignore
            };
            if !ok {
                node_matches = false;
                break;
            }
        }

        if node_matches {
            matched.push(*nid);
        }
    }

    // Apply selection.
    if args.additive {
        for nid in &matched {
            doc.selection.node_ids.insert(*nid);
        }
        for nid in &ref_ids {
            doc.selection.node_ids.insert(*nid);
        }
    } else {
        doc.selection.node_ids.clear();
        for nid in matched.iter().chain(ref_ids.iter()) {
            doc.selection.node_ids.insert(*nid);
        }
    }

    let total = doc.selection.node_ids.len();

    ToolResult::text(format!(
        "Selected {total} node(s) matching {match_by} (tolerance={tol})"
    ))
    .with_data(serde_json::json!({
        "matched_count": matched.len(),
        "total_selected": total,
        "match_by": match_by,
        "tolerance": tol,
        "node_ids": doc.selection.node_ids.iter().map(|id| id.to_string()).collect::<Vec<_>>(),
    }))
}

// ─── Asset Export ─────────────────────────────────────────────────────────────

/// Tag a node for inclusion in batch asset exports.  Passing an empty `name`
/// removes the tag entirely.
pub async fn tag_node_for_export(state: &AppState, args: TagNodeForExportArgs) -> ToolResult {
    tracing::debug!("tool: tag_node_for_export");
    use photonic_core::history::Command;
    use photonic_core::AssetExportSpec;

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let nid = uuid::Uuid::parse_str(&args.node_id).ok().or_else(|| {
        doc.nodes
            .values()
            .find(|n| n.name == args.node_id)
            .map(|n| n.id)
    });

    let nid = match nid {
        Some(id) => id,
        None => return ToolResult::error(format!("Node not found: {}", args.node_id)),
    };

    let node = match doc.nodes.get(&nid).cloned() {
        Some(n) => n,
        None => return ToolResult::error(format!("Node not found: {}", args.node_id)),
    };

    let mut new_node = node.clone();
    if args.name.trim().is_empty() {
        new_node.export_spec = None;
        history.execute(
            Command::UpdateNode {
                old: node,
                new: new_node,
            },
            &mut doc,
        );
        return ToolResult::text(format!("Removed export tag from node '{}'.", args.node_id));
    }

    let format = args.format.as_deref().unwrap_or("svg").to_lowercase();
    if !matches!(format.as_str(), "svg" | "png" | "jpeg" | "jpg" | "webp") {
        return ToolResult::error(format!(
            "Unsupported format '{}'. Use svg, png, jpeg, or webp.",
            format
        ));
    }

    let scales = if args.scales.is_empty() {
        vec![1.0]
    } else {
        args.scales.clone()
    };

    new_node.export_spec = Some(AssetExportSpec {
        name: args.name.trim().to_string(),
        format: format.clone(),
        scales: scales.clone(),
    });

    history.execute(
        Command::UpdateNode {
            old: node,
            new: new_node,
        },
        &mut doc,
    );

    ToolResult::text(format!(
        "Tagged node '{}' for export as '{}' ({}, {} scale(s)).",
        args.node_id,
        args.name.trim(),
        format,
        scales.len()
    ))
    .with_data(serde_json::json!({
        "node_id": nid.to_string(),
        "asset_name": args.name.trim(),
        "format": format,
        "scales": scales,
    }))
}

/// Export all nodes tagged with `tag_node_for_export`.  Returns a JSON array
/// of export results, one entry per (node × scale) combination.
pub async fn export_tagged_assets(state: &AppState, args: ExportTaggedAssetsArgs) -> ToolResult {
    tracing::debug!("tool: export_tagged_assets");

    let doc = state.document.lock().await;

    let tagged: Vec<_> = doc
        .nodes
        .values()
        .filter(|n| {
            n.export_spec.is_some()
                && args
                    .filter
                    .as_deref()
                    .map(|f| n.export_spec.as_ref().unwrap().name.contains(f))
                    .unwrap_or(true)
        })
        .collect();

    if tagged.is_empty() {
        return ToolResult::text("No nodes tagged for export. Use tag_node_for_export first.");
    }

    let mut results: Vec<serde_json::Value> = Vec::new();

    for node in &tagged {
        let spec = node.export_spec.as_ref().unwrap();
        match spec.format.as_str() {
            "svg" => {
                let svg = photonic_core::export::export_nodes_as_svg(&doc, &[node.id]);
                results.push(serde_json::json!({
                    "asset_name": spec.name,
                    "node_id": node.id.to_string(),
                    "node_name": node.name,
                    "format": "svg",
                    "scale": 1.0,
                    "filename": format!("{}.svg", spec.name),
                    "svg": svg,
                    "bytes": svg.len(),
                }));
            }
            _ => {
                // For raster formats, record intent (actual raster requires render thread).
                for &scale in &spec.scales {
                    let suffix = if (scale - 1.0).abs() < 0.001 {
                        String::new()
                    } else {
                        format!("@{}x", scale as u32)
                    };
                    results.push(serde_json::json!({
                        "asset_name": spec.name,
                        "node_id": node.id.to_string(),
                        "node_name": node.name,
                        "format": spec.format,
                        "scale": scale,
                        "filename": format!("{}{}.{}", spec.name, suffix, spec.format),
                        "note": "Raster export requires render thread — use export_raster MCP tool with the returned node_id",
                    }));
                }
            }
        }
    }

    ToolResult::text(format!(
        "Exported {} asset(s) from {} tagged node(s).",
        results.len(),
        tagged.len()
    ))
    .with_data(serde_json::json!({
        "asset_count": results.len(),
        "tagged_node_count": tagged.len(),
        "assets": results,
    }))
}

// ─── Character Styles ─────────────────────────────────────────────────────────

/// Save (or update) a named character style in the document.
pub async fn create_character_style(
    state: &AppState,
    args: CreateCharacterStyleArgs,
) -> ToolResult {
    tracing::debug!("tool: create_character_style");
    use photonic_core::{style::FillKind, CharacterStyle};

    if args.name.trim().is_empty() {
        return ToolResult::error("Style name must not be empty");
    }

    let mut doc = state.document.lock().await;

    // Optionally capture attributes from a source text node.
    let mut style = CharacterStyle {
        name: args.name.trim().to_string(),
        font_family: args.font_family.clone(),
        font_size: args.font_size,
        font_weight: args.font_weight,
        fill_hex: args.fill_hex.clone(),
        letter_spacing: args.letter_spacing,
        line_height: args.line_height,
    };

    if let Some(src_id_str) = &args.source_node_id {
        let src_id = uuid::Uuid::parse_str(src_id_str).ok().or_else(|| {
            doc.nodes
                .values()
                .find(|n| n.name == *src_id_str)
                .map(|n| n.id)
        });
        if let Some(sid) = src_id {
            if let Some(node) = doc.nodes.get(&sid) {
                if let photonic_core::SceneNodeKind::Text(t) = &node.kind {
                    // Capture from node; explicit args override.
                    if style.font_family.is_none() {
                        style.font_family = Some(t.font_family.clone());
                    }
                    if style.font_size.is_none() {
                        style.font_size = Some(t.font_size);
                    }
                    if style.font_weight.is_none() {
                        style.font_weight = Some(t.font_weight);
                    }
                    if style.letter_spacing.is_none() {
                        style.letter_spacing = Some(t.letter_spacing);
                    }
                    if style.line_height.is_none() {
                        style.line_height = Some(t.line_height);
                    }
                    if style.fill_hex.is_none() {
                        if t.fill.enabled {
                            if let FillKind::Solid(c) = &t.fill.kind {
                                style.fill_hex = Some(c.to_hex());
                            }
                        }
                    }
                }
            }
        }
    }

    // Replace existing or append.
    let action = if let Some(existing) = doc
        .character_styles
        .iter_mut()
        .find(|s| s.name == style.name)
    {
        *existing = style.clone();
        "Updated"
    } else {
        doc.character_styles.push(style.clone());
        "Created"
    };

    ToolResult::text(format!("{action} character style '{}'.", style.name)).with_data(
        serde_json::json!({
            "name": style.name,
            "font_family": style.font_family,
            "font_size": style.font_size,
            "font_weight": style.font_weight,
            "fill_hex": style.fill_hex,
            "letter_spacing": style.letter_spacing,
            "line_height": style.line_height,
        }),
    )
}

/// List all character styles saved in the document.
pub async fn list_character_styles(state: &AppState) -> ToolResult {
    tracing::debug!("tool: list_character_styles");
    let doc = state.document.lock().await;
    if doc.character_styles.is_empty() {
        return ToolResult::text("No character styles defined.");
    }
    let styles: Vec<_> = doc
        .character_styles
        .iter()
        .map(|s| {
            serde_json::json!({
                "name": s.name,
                "font_family": s.font_family,
                "font_size": s.font_size,
                "font_weight": s.font_weight,
                "fill_hex": s.fill_hex,
                "letter_spacing": s.letter_spacing,
                "line_height": s.line_height,
            })
        })
        .collect();
    ToolResult::text(format!("{} character style(s).", styles.len()))
        .with_data(serde_json::json!({ "character_styles": styles }))
}

/// Apply a named character style to one or more text nodes.
pub async fn apply_character_style(state: &AppState, args: ApplyCharacterStyleArgs) -> ToolResult {
    tracing::debug!("tool: apply_character_style");
    use photonic_core::color::Color;
    use photonic_core::history::Command;
    use photonic_core::style::Fill;

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    // Look up style.
    let style = match doc
        .character_styles
        .iter()
        .find(|s| s.name == args.style_name)
        .cloned()
    {
        Some(s) => s,
        None => {
            return ToolResult::error(format!("Character style '{}' not found.", args.style_name))
        }
    };

    // Resolve target nodes.
    let target_ids: Vec<photonic_core::NodeId> = if args.node_ids.is_empty() {
        doc.selection.node_ids.iter().copied().collect()
    } else {
        args.node_ids
            .iter()
            .filter_map(|s| {
                uuid::Uuid::parse_str(s)
                    .ok()
                    .or_else(|| doc.nodes.values().find(|n| n.name == *s).map(|n| n.id))
            })
            .collect()
    };

    if target_ids.is_empty() {
        return ToolResult::error("No target nodes specified and no active selection.");
    }

    let mut applied = 0usize;
    let mut commands = Vec::new();

    for nid in &target_ids {
        if let Some(node) = doc.nodes.get(nid).cloned() {
            if let photonic_core::SceneNodeKind::Text(_) = &node.kind {
                let mut new_node = node.clone();
                if let photonic_core::SceneNodeKind::Text(ref mut t) = new_node.kind {
                    if let Some(ff) = &style.font_family {
                        t.font_family = ff.clone();
                    }
                    if let Some(fs) = style.font_size {
                        t.font_size = fs;
                    }
                    if let Some(fw) = style.font_weight {
                        t.font_weight = fw;
                    }
                    if let Some(ls) = style.letter_spacing {
                        t.letter_spacing = ls;
                    }
                    if let Some(lh) = style.line_height {
                        t.line_height = lh;
                    }
                    if let Some(hex) = &style.fill_hex {
                        if let Some(color) = Color::from_hex(hex) {
                            t.fill = Fill::solid(color);
                        }
                    }
                }
                commands.push(Command::UpdateNode {
                    old: node,
                    new: new_node,
                });
                applied += 1;
            }
        }
    }

    if commands.is_empty() {
        return ToolResult::error("No text nodes found in the target set.");
    }

    let batch = if commands.len() == 1 {
        commands.remove(0)
    } else {
        Command::Batch(commands)
    };
    history.execute(batch, &mut doc);

    ToolResult::text(format!(
        "Applied character style '{}' to {applied} text node(s).",
        style.name
    ))
    .with_data(serde_json::json!({
        "style_name": style.name,
        "nodes_updated": applied,
    }))
}

/// Delete a named character style from the document.
pub async fn delete_character_style(
    state: &AppState,
    args: DeleteCharacterStyleArgs,
) -> ToolResult {
    tracing::debug!("tool: delete_character_style");
    let mut doc = state.document.lock().await;
    let before = doc.character_styles.len();
    doc.character_styles.retain(|s| s.name != args.name);
    if doc.character_styles.len() < before {
        ToolResult::text(format!("Deleted character style '{}'.", args.name))
    } else {
        ToolResult::error(format!("No character style named '{}' found.", args.name))
    }
}

// ─── Paragraph Styles ─────────────────────────────────────────────────────────

/// Save (or update) a named paragraph style.
pub async fn create_paragraph_style(
    state: &AppState,
    args: CreateParagraphStyleArgs,
) -> ToolResult {
    tracing::debug!("tool: create_paragraph_style");
    use photonic_core::ParagraphStyle;

    if args.name.trim().is_empty() {
        return ToolResult::error("Style name must not be empty");
    }

    let mut doc = state.document.lock().await;

    let mut style = ParagraphStyle {
        name: args.name.trim().to_string(),
        align: args.align.clone(),
        line_height: args.line_height,
        letter_spacing: args.letter_spacing,
        font_size: args.font_size,
        font_family: args.font_family.clone(),
    };

    // Optionally capture from a source text node.
    if let Some(src_str) = &args.source_node_id {
        let src_id = uuid::Uuid::parse_str(src_str).ok().or_else(|| {
            doc.nodes
                .values()
                .find(|n| n.name == *src_str)
                .map(|n| n.id)
        });
        if let Some(sid) = src_id {
            if let Some(node) = doc.nodes.get(&sid) {
                if let photonic_core::SceneNodeKind::Text(t) = &node.kind {
                    use photonic_core::node::TextAlign;
                    if style.align.is_none() {
                        style.align = Some(match t.align {
                            TextAlign::Left => "left".to_string(),
                            TextAlign::Center => "center".to_string(),
                            TextAlign::Right => "right".to_string(),
                        });
                    }
                    if style.line_height.is_none() {
                        style.line_height = Some(t.line_height);
                    }
                    if style.letter_spacing.is_none() {
                        style.letter_spacing = Some(t.letter_spacing);
                    }
                    if style.font_size.is_none() {
                        style.font_size = Some(t.font_size);
                    }
                    if style.font_family.is_none() {
                        style.font_family = Some(t.font_family.clone());
                    }
                }
            }
        }
    }

    let action = if let Some(existing) = doc
        .paragraph_styles
        .iter_mut()
        .find(|s| s.name == style.name)
    {
        *existing = style.clone();
        "Updated"
    } else {
        doc.paragraph_styles.push(style.clone());
        "Created"
    };

    ToolResult::text(format!("{action} paragraph style '{}'.", style.name)).with_data(
        serde_json::json!({
            "name": style.name,
            "align": style.align,
            "line_height": style.line_height,
            "letter_spacing": style.letter_spacing,
            "font_size": style.font_size,
            "font_family": style.font_family,
        }),
    )
}

/// List all paragraph styles saved in the document.
pub async fn list_paragraph_styles(state: &AppState) -> ToolResult {
    tracing::debug!("tool: list_paragraph_styles");
    let doc = state.document.lock().await;
    if doc.paragraph_styles.is_empty() {
        return ToolResult::text("No paragraph styles defined.");
    }
    let styles: Vec<_> = doc
        .paragraph_styles
        .iter()
        .map(|s| {
            serde_json::json!({
                "name": s.name,
                "align": s.align,
                "line_height": s.line_height,
                "letter_spacing": s.letter_spacing,
                "font_size": s.font_size,
                "font_family": s.font_family,
            })
        })
        .collect();
    ToolResult::text(format!("{} paragraph style(s).", styles.len()))
        .with_data(serde_json::json!({ "paragraph_styles": styles }))
}

/// Apply a named paragraph style to one or more text nodes.
pub async fn apply_paragraph_style(state: &AppState, args: ApplyParagraphStyleArgs) -> ToolResult {
    tracing::debug!("tool: apply_paragraph_style");
    use photonic_core::history::Command;
    use photonic_core::node::TextAlign;

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let style = match doc
        .paragraph_styles
        .iter()
        .find(|s| s.name == args.style_name)
        .cloned()
    {
        Some(s) => s,
        None => {
            return ToolResult::error(format!("Paragraph style '{}' not found.", args.style_name))
        }
    };

    let target_ids: Vec<photonic_core::NodeId> = if args.node_ids.is_empty() {
        doc.selection.node_ids.iter().copied().collect()
    } else {
        args.node_ids
            .iter()
            .filter_map(|s| {
                uuid::Uuid::parse_str(s)
                    .ok()
                    .or_else(|| doc.nodes.values().find(|n| n.name == *s).map(|n| n.id))
            })
            .collect()
    };

    if target_ids.is_empty() {
        return ToolResult::error("No target nodes specified and no active selection.");
    }

    let mut applied = 0usize;
    let mut commands = Vec::new();

    for nid in &target_ids {
        if let Some(node) = doc.nodes.get(nid).cloned() {
            if let photonic_core::SceneNodeKind::Text(_) = &node.kind {
                let mut new_node = node.clone();
                if let photonic_core::SceneNodeKind::Text(ref mut t) = new_node.kind {
                    if let Some(align_str) = &style.align {
                        t.align = match align_str.as_str() {
                            "center" => TextAlign::Center,
                            "right" => TextAlign::Right,
                            _ => TextAlign::Left,
                        };
                    }
                    if let Some(lh) = style.line_height {
                        t.line_height = lh;
                    }
                    if let Some(ls) = style.letter_spacing {
                        t.letter_spacing = ls;
                    }
                    if let Some(fs) = style.font_size {
                        t.font_size = fs;
                    }
                    if let Some(ff) = &style.font_family {
                        t.font_family = ff.clone();
                    }
                }
                commands.push(Command::UpdateNode {
                    old: node,
                    new: new_node,
                });
                applied += 1;
            }
        }
    }

    if commands.is_empty() {
        return ToolResult::error("No text nodes found in the target set.");
    }

    let batch = if commands.len() == 1 {
        commands.remove(0)
    } else {
        Command::Batch(commands)
    };
    history.execute(batch, &mut doc);

    ToolResult::text(format!(
        "Applied paragraph style '{}' to {applied} text node(s).",
        style.name
    ))
    .with_data(serde_json::json!({
        "style_name": style.name,
        "nodes_updated": applied,
    }))
}

/// Delete a named paragraph style from the document.
pub async fn delete_paragraph_style(
    state: &AppState,
    args: DeleteParagraphStyleArgs,
) -> ToolResult {
    tracing::debug!("tool: delete_paragraph_style");
    let mut doc = state.document.lock().await;
    let before = doc.paragraph_styles.len();
    doc.paragraph_styles.retain(|s| s.name != args.name);
    if doc.paragraph_styles.len() < before {
        ToolResult::text(format!("Deleted paragraph style '{}'.", args.name))
    } else {
        ToolResult::error(format!("No paragraph style named '{}' found.", args.name))
    }
}

// ─── Clipping Mask ────────────────────────────────────────────────────────────

/// Make a clipping mask from a group node.
/// The topmost child (last in `children`) becomes the clip path for all other children.
pub async fn make_clipping_mask(state: &AppState, args: MakeClippingMaskArgs) -> ToolResult {
    tracing::debug!("tool: make_clipping_mask");
    let mut doc = state.document.lock().await;

    // Resolve node ID
    let group_id = {
        let id = args.group_id.trim();
        if let Ok(uuid) = uuid::Uuid::parse_str(id) {
            uuid
        } else {
            match doc.nodes.values().find(|n| n.name == id) {
                Some(n) => n.id,
                None => return ToolResult::error(format!("Node '{}' not found.", args.group_id)),
            }
        }
    };

    let node = match doc.nodes.get(&group_id) {
        Some(n) => n.clone(),
        None => return ToolResult::error(format!("Node '{}' not found.", args.group_id)),
    };

    let group = match &node.kind {
        SceneNodeKind::Group(g) => g.clone(),
        _ => return ToolResult::error(format!("Node '{}' is not a group.", args.group_id)),
    };

    if group.children.len() < 2 {
        return ToolResult::error(
            "Group must have at least 2 children: one clip path and one or more masked objects.",
        );
    }

    // Topmost child (last in children list) is the clip path
    let clip_id = *group.children.last().unwrap();

    let mut new_node = node.clone();
    if let SceneNodeKind::Group(ref mut g) = new_node.kind {
        g.clip_node_id = Some(clip_id);
    }

    let clip_name = doc
        .nodes
        .get(&clip_id)
        .map(|n| n.name.clone())
        .unwrap_or_else(|| clip_id.to_string());
    let mut history = state.history.lock().await;
    history.execute(
        Command::UpdateNode {
            old: node,
            new: new_node,
        },
        &mut doc,
    );
    drop(history);

    ToolResult::text(format!(
        "Clipping mask created on group '{}' using '{}' as clip path.",
        args.group_id, clip_name
    ))
    .with_data(serde_json::json!({ "group_id": group_id, "clip_node_id": clip_id }))
}

/// Release the clipping mask from a group node, restoring all children as normal objects.
pub async fn release_clipping_mask(state: &AppState, args: ReleaseClippingMaskArgs) -> ToolResult {
    tracing::debug!("tool: release_clipping_mask");
    let mut doc = state.document.lock().await;

    let group_id = {
        let id = args.group_id.trim();
        if let Ok(uuid) = uuid::Uuid::parse_str(id) {
            uuid
        } else {
            match doc.nodes.values().find(|n| n.name == id) {
                Some(n) => n.id,
                None => return ToolResult::error(format!("Node '{}' not found.", args.group_id)),
            }
        }
    };

    let node = match doc.nodes.get(&group_id) {
        Some(n) => n.clone(),
        None => return ToolResult::error(format!("Node '{}' not found.", args.group_id)),
    };

    let had_mask = match &node.kind {
        SceneNodeKind::Group(g) => g.clip_node_id.is_some(),
        _ => return ToolResult::error(format!("Node '{}' is not a group.", args.group_id)),
    };

    if !had_mask {
        return ToolResult::error(format!(
            "Group '{}' does not have a clipping mask.",
            args.group_id
        ));
    }

    let mut new_node = node.clone();
    if let SceneNodeKind::Group(ref mut g) = new_node.kind {
        g.clip_node_id = None;
    }

    let mut history = state.history.lock().await;
    history.execute(
        Command::UpdateNode {
            old: node,
            new: new_node,
        },
        &mut doc,
    );
    drop(history);

    ToolResult::text(format!(
        "Released clipping mask from group '{}'.",
        args.group_id
    ))
    .with_data(serde_json::json!({ "group_id": group_id }))
}

// ─── Type on a Path ───────────────────────────────────────────────────────────

/// Place text along a path spine (Type on a Path).
pub async fn set_text_path(state: &AppState, args: SetTextPathArgs) -> ToolResult {
    tracing::debug!("tool: set_text_path");
    let mut doc = state.document.lock().await;

    let resolve = |id: &str| -> Option<NodeId> {
        uuid::Uuid::parse_str(id)
            .ok()
            .or_else(|| doc.find_node_by_name(id).map(|n| n.id))
    };

    let text_id = match resolve(&args.text_node_id) {
        Some(id) => id,
        None => return ToolResult::error(format!("Text node '{}' not found.", args.text_node_id)),
    };
    let path_id = match resolve(&args.path_node_id) {
        Some(id) => id,
        None => return ToolResult::error(format!("Path node '{}' not found.", args.path_node_id)),
    };

    let text_node = match doc.nodes.get(&text_id) {
        Some(n) => n.clone(),
        None => return ToolResult::error(format!("Text node '{}' not found.", args.text_node_id)),
    };

    // Verify target is actually a text node.
    if !matches!(text_node.kind, SceneNodeKind::Text(_)) {
        return ToolResult::error(format!("Node '{}' is not a text node.", args.text_node_id));
    }

    if text_id == path_id {
        return ToolResult::error("Text node and path node must be different nodes.");
    }

    // Verify spine is a path node.
    match doc.nodes.get(&path_id) {
        Some(n) if matches!(n.kind, SceneNodeKind::Path(_)) => {}
        Some(_) => {
            return ToolResult::error(format!("Node '{}' is not a path node.", args.path_node_id))
        }
        None => return ToolResult::error(format!("Path node '{}' not found.", args.path_node_id)),
    }

    let mut new_node = text_node.clone();
    if let SceneNodeKind::Text(ref mut tn) = new_node.kind {
        tn.path_spine_id = Some(path_id);
        tn.path_offset = args.offset;
    }

    let mut history = state.history.lock().await;
    history.execute(
        Command::UpdateNode {
            old: text_node,
            new: new_node,
        },
        &mut doc,
    );
    drop(history);

    ToolResult::text(format!(
        "Text node '{}' now follows path '{}'  (offset: {:.1}).",
        args.text_node_id, args.path_node_id, args.offset
    ))
    .with_data(serde_json::json!({
        "text_node_id": text_id,
        "path_node_id": path_id,
        "offset": args.offset,
    }))
}

/// Remove the path spine from a text node (revert to normal positioned text).
pub async fn clear_text_path(state: &AppState, args: ClearTextPathArgs) -> ToolResult {
    tracing::debug!("tool: clear_text_path");
    let mut doc = state.document.lock().await;

    let text_id = uuid::Uuid::parse_str(&args.text_node_id)
        .ok()
        .or_else(|| doc.find_node_by_name(&args.text_node_id).map(|n| n.id));
    let text_id = match text_id {
        Some(id) => id,
        None => return ToolResult::error(format!("Text node '{}' not found.", args.text_node_id)),
    };

    let text_node = match doc.nodes.get(&text_id) {
        Some(n) => n.clone(),
        None => return ToolResult::error(format!("Text node '{}' not found.", args.text_node_id)),
    };

    if !matches!(text_node.kind, SceneNodeKind::Text(_)) {
        return ToolResult::error(format!("Node '{}' is not a text node.", args.text_node_id));
    }

    let had_spine =
        matches!(&text_node.kind, SceneNodeKind::Text(tn) if tn.path_spine_id.is_some());
    if !had_spine {
        return ToolResult::error(format!(
            "Text node '{}' is not on a path.",
            args.text_node_id
        ));
    }

    let mut new_node = text_node.clone();
    if let SceneNodeKind::Text(ref mut tn) = new_node.kind {
        tn.path_spine_id = None;
        tn.path_offset = 0.0;
    }

    let mut history = state.history.lock().await;
    history.execute(
        Command::UpdateNode {
            old: text_node,
            new: new_node,
        },
        &mut doc,
    );
    drop(history);

    ToolResult::text(format!(
        "Removed path spine from text node '{}'.",
        args.text_node_id
    ))
    .with_data(serde_json::json!({ "text_node_id": text_id }))
}

// ─── Text Direction ────────────────────────────────────────────────────────────

/// Set the text layout direction of a text node (horizontal or vertical).
pub async fn set_text_direction(state: &AppState, args: SetTextDirectionArgs) -> ToolResult {
    tracing::debug!("tool: set_text_direction");
    let mut doc = state.document.lock().await;

    let node_id = uuid::Uuid::parse_str(&args.node_id)
        .ok()
        .or_else(|| doc.find_node_by_name(&args.node_id).map(|n| n.id));
    let node_id = match node_id {
        Some(id) => id,
        None => return ToolResult::error(format!("Node '{}' not found.", args.node_id)),
    };

    let node = match doc.nodes.get(&node_id) {
        Some(n) => n.clone(),
        None => return ToolResult::error(format!("Node '{}' not found.", args.node_id)),
    };

    if !matches!(node.kind, SceneNodeKind::Text(_)) {
        return ToolResult::error(format!("Node '{}' is not a text node.", args.node_id));
    }

    let mut new_node = node.clone();
    if let SceneNodeKind::Text(ref mut tn) = new_node.kind {
        tn.vertical = args.vertical;
    }

    let mut history = state.history.lock().await;
    history.execute(
        Command::UpdateNode {
            old: node,
            new: new_node,
        },
        &mut doc,
    );
    drop(history);

    let dir = if args.vertical {
        "vertical"
    } else {
        "horizontal"
    };
    ToolResult::text(format!(
        "Text node '{}' set to {} layout.",
        args.node_id, dir
    ))
    .with_data(serde_json::json!({ "node_id": node_id, "vertical": args.vertical }))
}

// ─── Area Type ────────────────────────────────────────────────────────────────

/// Flow a text node inside a closed path area (Area Type).
pub async fn set_text_area(state: &AppState, args: SetTextAreaArgs) -> ToolResult {
    tracing::debug!("tool: set_text_area");
    let mut doc = state.document.lock().await;

    let resolve = |id: &str| -> Option<NodeId> {
        uuid::Uuid::parse_str(id)
            .ok()
            .or_else(|| doc.find_node_by_name(id).map(|n| n.id))
    };

    let text_id = match resolve(&args.text_node_id) {
        Some(id) => id,
        None => return ToolResult::error(format!("Text node '{}' not found.", args.text_node_id)),
    };
    let area_id = match resolve(&args.area_path_id) {
        Some(id) => id,
        None => return ToolResult::error(format!("Area path '{}' not found.", args.area_path_id)),
    };
    if text_id == area_id {
        return ToolResult::error("Text node and area path must be different nodes.");
    }

    let text_node = match doc.nodes.get(&text_id) {
        Some(n) => n.clone(),
        None => return ToolResult::error(format!("Text node '{}' not found.", args.text_node_id)),
    };
    if !matches!(text_node.kind, SceneNodeKind::Text(_)) {
        return ToolResult::error(format!("Node '{}' is not a text node.", args.text_node_id));
    }

    match doc.nodes.get(&area_id) {
        Some(n) if matches!(n.kind, SceneNodeKind::Path(_)) => {}
        Some(_) => {
            return ToolResult::error(format!("Node '{}' is not a path node.", args.area_path_id))
        }
        None => return ToolResult::error(format!("Area path '{}' not found.", args.area_path_id)),
    }

    let mut new_node = text_node.clone();
    if let SceneNodeKind::Text(ref mut tn) = new_node.kind {
        tn.area_path_id = Some(area_id);
    }

    let area_name = doc
        .nodes
        .get(&area_id)
        .map(|n| n.name.clone())
        .unwrap_or_else(|| area_id.to_string());
    let mut history = state.history.lock().await;
    history.execute(
        Command::UpdateNode {
            old: text_node,
            new: new_node,
        },
        &mut doc,
    );
    drop(history);

    ToolResult::text(format!(
        "Text node '{}' now flows inside area '{}'.",
        args.text_node_id, area_name
    ))
    .with_data(serde_json::json!({ "text_node_id": text_id, "area_path_id": area_id }))
}

/// Remove the area boundary from a text node (revert to normal point text).
pub async fn clear_text_area(state: &AppState, args: ClearTextAreaArgs) -> ToolResult {
    tracing::debug!("tool: clear_text_area");
    let mut doc = state.document.lock().await;

    let text_id = uuid::Uuid::parse_str(&args.text_node_id)
        .ok()
        .or_else(|| doc.find_node_by_name(&args.text_node_id).map(|n| n.id));
    let text_id = match text_id {
        Some(id) => id,
        None => return ToolResult::error(format!("Text node '{}' not found.", args.text_node_id)),
    };

    let text_node = match doc.nodes.get(&text_id) {
        Some(n) => n.clone(),
        None => return ToolResult::error(format!("Text node '{}' not found.", args.text_node_id)),
    };

    if !matches!(text_node.kind, SceneNodeKind::Text(_)) {
        return ToolResult::error(format!("Node '{}' is not a text node.", args.text_node_id));
    }

    let had_area = matches!(&text_node.kind, SceneNodeKind::Text(tn) if tn.area_path_id.is_some());
    if !had_area {
        return ToolResult::error(format!(
            "Text node '{}' does not have an area path.",
            args.text_node_id
        ));
    }

    let mut new_node = text_node.clone();
    if let SceneNodeKind::Text(ref mut tn) = new_node.kind {
        tn.area_path_id = None;
    }

    let mut history = state.history.lock().await;
    history.execute(
        Command::UpdateNode {
            old: text_node,
            new: new_node,
        },
        &mut doc,
    );
    drop(history);

    ToolResult::text(format!(
        "Removed area boundary from text node '{}'.",
        args.text_node_id
    ))
    .with_data(serde_json::json!({ "text_node_id": text_id }))
}

// ─── Text Frame Threading ─────────────────────────────────────────────────────

/// Link two text nodes as a threaded text chain (overflow from `from` flows into `to`).
pub async fn link_text_frames(state: &AppState, args: LinkTextFramesArgs) -> ToolResult {
    tracing::debug!("tool: link_text_frames");
    let mut doc = state.document.lock().await;

    let from_id = uuid::Uuid::parse_str(&args.from_id)
        .ok()
        .or_else(|| doc.find_node_by_name(&args.from_id).map(|n| n.id));
    let from_id = match from_id {
        Some(id) => id,
        None => return ToolResult::error(format!("Node '{}' not found.", args.from_id)),
    };

    let to_id = uuid::Uuid::parse_str(&args.to_id)
        .ok()
        .or_else(|| doc.find_node_by_name(&args.to_id).map(|n| n.id));
    let to_id = match to_id {
        Some(id) => id,
        None => return ToolResult::error(format!("Node '{}' not found.", args.to_id)),
    };

    if from_id == to_id {
        return ToolResult::error("A text frame cannot be linked to itself.");
    }

    // Validate both are text nodes.
    let from_node = match doc.nodes.get(&from_id) {
        Some(n) if matches!(n.kind, SceneNodeKind::Text(_)) => n.clone(),
        Some(_) => {
            return ToolResult::error(format!("Node '{}' is not a text node.", args.from_id))
        }
        None => return ToolResult::error(format!("Node '{}' not found.", args.from_id)),
    };
    let to_node = match doc.nodes.get(&to_id) {
        Some(n) if matches!(n.kind, SceneNodeKind::Text(_)) => n.clone(),
        Some(_) => return ToolResult::error(format!("Node '{}' is not a text node.", args.to_id)),
        None => return ToolResult::error(format!("Node '{}' not found.", args.to_id)),
    };

    let mut new_from = from_node.clone();
    let mut new_to = to_node.clone();
    if let SceneNodeKind::Text(ref mut tn) = new_from.kind {
        tn.next_frame = Some(to_id);
    }
    if let SceneNodeKind::Text(ref mut tn) = new_to.kind {
        tn.prev_frame = Some(from_id);
    }

    let mut history = state.history.lock().await;
    history.execute(
        Command::Batch(vec![
            Command::UpdateNode {
                old: from_node,
                new: new_from,
            },
            Command::UpdateNode {
                old: to_node,
                new: new_to,
            },
        ]),
        &mut doc,
    );
    drop(history);

    ToolResult::text(format!(
        "Linked text frames: '{}' → '{}'.",
        args.from_id, args.to_id
    ))
    .with_data(serde_json::json!({ "from_id": from_id.to_string(), "to_id": to_id.to_string() }))
}

/// Unlink a text node from its thread chain, updating adjacent frame links.
pub async fn unlink_text_frames(state: &AppState, args: UnlinkTextFramesArgs) -> ToolResult {
    tracing::debug!("tool: unlink_text_frames");
    let mut doc = state.document.lock().await;

    let node_id = uuid::Uuid::parse_str(&args.node_id)
        .ok()
        .or_else(|| doc.find_node_by_name(&args.node_id).map(|n| n.id));
    let node_id = match node_id {
        Some(id) => id,
        None => return ToolResult::error(format!("Node '{}' not found.", args.node_id)),
    };

    let node = match doc.nodes.get(&node_id) {
        Some(n) if matches!(n.kind, SceneNodeKind::Text(_)) => n.clone(),
        Some(_) => {
            return ToolResult::error(format!("Node '{}' is not a text node.", args.node_id))
        }
        None => return ToolResult::error(format!("Node '{}' not found.", args.node_id)),
    };

    let (prev_id, next_id) = match &node.kind {
        SceneNodeKind::Text(tn) => (tn.prev_frame, tn.next_frame),
        _ => (None, None),
    };

    if prev_id.is_none() && next_id.is_none() {
        return ToolResult::error(format!(
            "Node '{}' is not part of a text thread.",
            args.node_id
        ));
    }

    let mut commands: Vec<Command> = Vec::new();

    // Update this node.
    let mut new_node = node.clone();
    if let SceneNodeKind::Text(ref mut tn) = new_node.kind {
        tn.next_frame = None;
        tn.prev_frame = None;
    }
    commands.push(Command::UpdateNode {
        old: node,
        new: new_node,
    });

    // Clear next_frame link from prev node.
    if let Some(pid) = prev_id {
        if let Some(prev) = doc.nodes.get(&pid).cloned() {
            let mut new_prev = prev.clone();
            if let SceneNodeKind::Text(ref mut tn) = new_prev.kind {
                tn.next_frame = None;
            }
            commands.push(Command::UpdateNode {
                old: prev,
                new: new_prev,
            });
        }
    }

    // Clear prev_frame link from next node.
    if let Some(nid) = next_id {
        if let Some(next) = doc.nodes.get(&nid).cloned() {
            let mut new_next = next.clone();
            if let SceneNodeKind::Text(ref mut tn) = new_next.kind {
                tn.prev_frame = None;
            }
            commands.push(Command::UpdateNode {
                old: next,
                new: new_next,
            });
        }
    }

    let mut history = state.history.lock().await;
    history.execute(Command::Batch(commands), &mut doc);
    drop(history);

    ToolResult::text(format!("Unlinked text frame '{}'.", args.node_id))
        .with_data(serde_json::json!({ "node_id": node_id.to_string() }))
}

// ─── Text Variable Binding ────────────────────────────────────────────────────

/// Bind a text node to a document variable so apply_variables replaces its content.
pub async fn bind_text_variable(state: &AppState, args: BindTextVariableArgs) -> ToolResult {
    tracing::debug!("tool: bind_text_variable");
    let mut doc = state.document.lock().await;

    let node_id = uuid::Uuid::parse_str(&args.node_id)
        .ok()
        .or_else(|| doc.find_node_by_name(&args.node_id).map(|n| n.id));
    let node_id = match node_id {
        Some(id) => id,
        None => return ToolResult::error(format!("Node '{}' not found.", args.node_id)),
    };

    // Verify the variable exists.
    if !doc.variables.iter().any(|v| v.name == args.variable_name) {
        return ToolResult::error(format!(
            "Variable '{}' not found. Use define_variable first.",
            args.variable_name
        ));
    }

    let node = match doc.nodes.get(&node_id) {
        Some(n) => n.clone(),
        None => return ToolResult::error(format!("Node '{}' not found.", args.node_id)),
    };
    if !matches!(node.kind, SceneNodeKind::Text(_)) {
        return ToolResult::error(format!("Node '{}' is not a text node.", args.node_id));
    }

    let mut new_node = node.clone();
    if let SceneNodeKind::Text(ref mut tn) = new_node.kind {
        tn.variable_binding = Some(args.variable_name.clone());
    }

    let mut history = state.history.lock().await;
    history.execute(
        Command::UpdateNode {
            old: node,
            new: new_node,
        },
        &mut doc,
    );
    drop(history);

    ToolResult::text(format!(
        "Text node '{}' bound to variable '{}'.",
        args.node_id, args.variable_name
    ))
    .with_data(serde_json::json!({ "node_id": node_id, "variable_name": args.variable_name }))
}

/// Remove the variable binding from a text node.
pub async fn unbind_text_variable(state: &AppState, args: UnbindTextVariableArgs) -> ToolResult {
    tracing::debug!("tool: unbind_text_variable");
    let mut doc = state.document.lock().await;

    let node_id = uuid::Uuid::parse_str(&args.node_id)
        .ok()
        .or_else(|| doc.find_node_by_name(&args.node_id).map(|n| n.id));
    let node_id = match node_id {
        Some(id) => id,
        None => return ToolResult::error(format!("Node '{}' not found.", args.node_id)),
    };

    let node = match doc.nodes.get(&node_id) {
        Some(n) => n.clone(),
        None => return ToolResult::error(format!("Node '{}' not found.", args.node_id)),
    };

    if !matches!(node.kind, SceneNodeKind::Text(_)) {
        return ToolResult::error(format!("Node '{}' is not a text node.", args.node_id));
    }

    let had_binding =
        matches!(&node.kind, SceneNodeKind::Text(tn) if tn.variable_binding.is_some());
    if !had_binding {
        return ToolResult::error(format!(
            "Text node '{}' does not have a variable binding.",
            args.node_id
        ));
    }

    let mut new_node = node.clone();
    if let SceneNodeKind::Text(ref mut tn) = new_node.kind {
        tn.variable_binding = None;
    }

    let mut history = state.history.lock().await;
    history.execute(
        Command::UpdateNode {
            old: node,
            new: new_node,
        },
        &mut doc,
    );
    drop(history);

    ToolResult::text(format!(
        "Removed variable binding from text node '{}'.",
        args.node_id
    ))
    .with_data(serde_json::json!({ "node_id": node_id }))
}

/// Set the font style (normal / italic / oblique) on a text node.
pub async fn set_font_style(state: &AppState, args: SetFontStyleArgs) -> ToolResult {
    tracing::debug!("tool: set_font_style");
    let mut doc = state.document.lock().await;

    let node_id = uuid::Uuid::parse_str(&args.node_id)
        .ok()
        .or_else(|| doc.find_node_by_name(&args.node_id).map(|n| n.id));
    let node_id = match node_id {
        Some(id) => id,
        None => return ToolResult::error(format!("Node '{}' not found.", args.node_id)),
    };
    let node = match doc.nodes.get(&node_id) {
        Some(n) => n.clone(),
        None => return ToolResult::error(format!("Node '{}' not found.", args.node_id)),
    };
    if !matches!(node.kind, SceneNodeKind::Text(_)) {
        return ToolResult::error(format!("Node '{}' is not a text node.", args.node_id));
    }
    let font_style = match args.style.to_lowercase().as_str() {
        "italic" => FontStyle::Italic,
        "oblique" => FontStyle::Oblique,
        _ => FontStyle::Normal,
    };
    let mut new_node = node.clone();
    if let SceneNodeKind::Text(ref mut tn) = new_node.kind {
        tn.font_style = font_style;
    }
    let mut history = state.history.lock().await;
    history.execute(
        Command::UpdateNode {
            old: node,
            new: new_node,
        },
        &mut doc,
    );
    drop(history);
    ToolResult::text(format!(
        "Set font style to '{}' on node '{}'.",
        args.style, args.node_id
    ))
    .with_data(serde_json::json!({ "node_id": node_id, "font_style": args.style }))
}

/// Set the font weight (100–900) on a text node.
pub async fn set_font_weight(state: &AppState, args: SetFontWeightArgs) -> ToolResult {
    tracing::debug!("tool: set_font_weight");
    let mut doc = state.document.lock().await;

    let node_id = uuid::Uuid::parse_str(&args.node_id)
        .ok()
        .or_else(|| doc.find_node_by_name(&args.node_id).map(|n| n.id));
    let node_id = match node_id {
        Some(id) => id,
        None => return ToolResult::error(format!("Node '{}' not found.", args.node_id)),
    };
    let node = match doc.nodes.get(&node_id) {
        Some(n) => n.clone(),
        None => return ToolResult::error(format!("Node '{}' not found.", args.node_id)),
    };
    if !matches!(node.kind, SceneNodeKind::Text(_)) {
        return ToolResult::error(format!("Node '{}' is not a text node.", args.node_id));
    }
    let weight = args.weight.clamp(100, 900);
    let mut new_node = node.clone();
    if let SceneNodeKind::Text(ref mut tn) = new_node.kind {
        tn.font_weight = weight;
    }
    let mut history = state.history.lock().await;
    history.execute(
        Command::UpdateNode {
            old: node,
            new: new_node,
        },
        &mut doc,
    );
    drop(history);
    ToolResult::text(format!(
        "Set font weight to {} on node '{}'.",
        weight, args.node_id
    ))
    .with_data(serde_json::json!({ "node_id": node_id, "font_weight": weight }))
}

/// Flatten transparency — bake node opacity and fill/stroke opacity into color
/// alpha values, then set all opacity fields to 1.0 for print-ready output.
pub async fn flatten_transparency(state: &AppState, args: FlattenTransparencyArgs) -> ToolResult {
    tracing::debug!("tool: flatten_transparency");
    use photonic_core::style::{Fill, FillKind, Stroke};

    let mut doc = state.document.lock().await;

    // Collect target node IDs
    let target_ids: Vec<NodeId> = if args.node_ids.is_empty() {
        doc.nodes.keys().cloned().collect()
    } else {
        args.node_ids
            .iter()
            .filter_map(|id_str| {
                uuid::Uuid::parse_str(id_str)
                    .ok()
                    .or_else(|| doc.find_node_by_name(id_str).map(|n| n.id))
            })
            .collect()
    };

    /// Premultiply a fill's own opacity and the node's opacity into color alphas.
    fn bake_fill(fill: &Fill, node_opacity: f32) -> Fill {
        let combined = (fill.opacity as f32) * node_opacity;
        let kind = match &fill.kind {
            FillKind::Solid(c) => FillKind::Solid(photonic_core::color::Color {
                r: c.r,
                g: c.g,
                b: c.b,
                a: c.a * combined,
            }),
            FillKind::Gradient(g) => {
                let mut g2 = g.clone();
                for stop in g2.stops.iter_mut() {
                    stop.color.a *= combined;
                }
                FillKind::Gradient(g2)
            }
            other => other.clone(),
        };
        Fill {
            kind,
            opacity: 1.0,
            enabled: fill.enabled,
        }
    }

    fn bake_stroke(stroke: &Stroke, node_opacity: f32) -> Stroke {
        let combined = node_opacity;
        let mut s = stroke.clone();
        s.color.a *= combined;
        s.opacity = 1.0;
        s
    }

    let mut commands = Vec::new();
    let mut processed = 0usize;

    for nid in target_ids {
        let node = match doc.nodes.get(&nid) {
            Some(n)
                if n.opacity < 1.0 - f32::EPSILON
                    || matches!(n.kind, SceneNodeKind::Path(ref pn) if pn.fill.opacity < 1.0 - f32::EPSILON) =>
            {
                n.clone()
            }
            Some(n) if matches!(n.kind, SceneNodeKind::Text(ref tn) if tn.fill.opacity < 1.0 - f32::EPSILON) => {
                n.clone()
            }
            _ => continue,
        };

        let node_opacity = node.opacity;
        let mut new_node = node.clone();
        new_node.opacity = 1.0;

        match &mut new_node.kind {
            SceneNodeKind::Path(pn) => {
                pn.fill = bake_fill(&pn.fill, node_opacity);
                pn.stroke = bake_stroke(&pn.stroke, node_opacity);
            }
            SceneNodeKind::Text(tn) => {
                tn.fill = bake_fill(&tn.fill, node_opacity);
            }
            SceneNodeKind::Group(_) => {
                // Group opacity baking is skipped — children are processed individually
            }
        }

        commands.push(Command::UpdateNode {
            old: node,
            new: new_node,
        });
        processed += 1;
    }

    if commands.is_empty() {
        return ToolResult::text("No nodes with transparency found — nothing to flatten.")
            .with_data(serde_json::json!({ "processed": 0 }));
    }

    let mut history = state.history.lock().await;
    history.execute(Command::Batch(commands), &mut doc);
    drop(history);

    ToolResult::text(format!("Flattened transparency on {} node(s).", processed))
        .with_data(serde_json::json!({ "processed": processed }))
}

pub async fn apply_flex_layout(state: &AppState, args: ApplyFlexLayoutArgs) -> ToolResult {
    let mut doc = state.document.lock().await;

    // Resolve group node
    let uid = match uuid::Uuid::parse_str(&args.group_id)
        .ok()
        .or_else(|| doc.find_node_by_name(&args.group_id).map(|n| n.id))
    {
        Some(id) => id,
        None => return ToolResult::error(format!("Node '{}' not found.", args.group_id)),
    };

    let group_node = match doc.nodes.get(&uid) {
        Some(n) => n.clone(),
        None => return ToolResult::error(format!("Node '{}' not found.", args.group_id)),
    };

    let child_ids = match &group_node.kind {
        SceneNodeKind::Group(g) => g.children.clone(),
        _ => return ToolResult::error("Target node is not a group."),
    };

    if child_ids.is_empty() {
        return ToolResult::text("Group has no children — nothing to layout.")
            .with_data(serde_json::json!({ "arranged": 0 }));
    }

    let direction = args.direction.as_deref().unwrap_or("row");
    let gap = args.gap.unwrap_or(8.0);
    let align = args.align.as_deref().unwrap_or("center");
    let padding = args.padding.unwrap_or(0.0);

    // Collect children with their bounding boxes
    struct ChildInfo {
        id: NodeId,
        tx: f64,
        ty: f64,
        w: f64,
        h: f64,
    }

    let mut children: Vec<ChildInfo> = Vec::new();
    for cid in &child_ids {
        let child = match doc.nodes.get(cid) {
            Some(n) => n.clone(),
            None => continue,
        };
        let (w, h) = match &child.kind {
            SceneNodeKind::Path(pn) => {
                if let Some(bb) = pn.path_data.bounding_box() {
                    (bb.width().abs().max(1.0), bb.height().abs().max(1.0))
                } else {
                    (60.0, 30.0)
                }
            }
            SceneNodeKind::Text(_) | SceneNodeKind::Group(_) => (60.0, 30.0),
        };
        let tx = child.transform.matrix[4];
        let ty = child.transform.matrix[5];
        children.push(ChildInfo {
            id: *cid,
            tx,
            ty,
            w,
            h,
        });
    }

    if children.is_empty() {
        return ToolResult::text("No accessible children found.")
            .with_data(serde_json::json!({ "arranged": 0 }));
    }

    // Sort by position along main axis
    match direction {
        "column" => {
            children.sort_by(|a, b| a.ty.partial_cmp(&b.ty).unwrap_or(std::cmp::Ordering::Equal))
        }
        _ => children.sort_by(|a, b| a.tx.partial_cmp(&b.tx).unwrap_or(std::cmp::Ordering::Equal)),
    }

    // Compute cross-axis extent for alignment
    let cross_max: f64 = match direction {
        "column" => children.iter().map(|c| c.w).fold(0.0_f64, f64::max),
        _ => children.iter().map(|c| c.h).fold(0.0_f64, f64::max),
    };

    let mut cursor = padding;
    let mut commands: Vec<Command> = Vec::new();

    for child in &children {
        let cross_size = match direction {
            "column" => child.w,
            _ => child.h,
        };

        let cross_offset = match align {
            "start" => padding,
            "end" => padding + cross_max - cross_size,
            _ => padding + (cross_max - cross_size) / 2.0, // center
        };

        let (new_tx, new_ty) = match direction {
            "column" => (cross_offset, cursor),
            _ => (cursor, cross_offset),
        };

        let main_size = match direction {
            "column" => child.h,
            _ => child.w,
        };
        cursor += main_size + gap;

        let old = doc.nodes.get(&child.id).unwrap().clone();
        let mut new_node = old.clone();
        new_node.transform.matrix[4] = new_tx;
        new_node.transform.matrix[5] = new_ty;
        commands.push(Command::UpdateNode { old, new: new_node });
    }

    let arranged = commands.len();
    let mut history = state.history.lock().await;
    history.execute(Command::Batch(commands), &mut doc);
    drop(history);

    ToolResult::text(format!(
        "Applied {} flex layout to {} children (gap={}, align={}, padding={}).",
        direction, arranged, gap, align, padding
    ))
    .with_data(serde_json::json!({
        "group_id": uid.to_string(),
        "direction": direction,
        "gap": gap,
        "align": align,
        "padding": padding,
        "arranged": arranged,
    }))
}

pub async fn undo_node(state: &AppState, args: UndoNodeArgs) -> ToolResult {
    let mut doc = state.document.lock().await;

    let uid = match uuid::Uuid::parse_str(&args.node_id)
        .ok()
        .or_else(|| doc.find_node_by_name(&args.node_id).map(|n| n.id))
    {
        Some(id) => id,
        None => return ToolResult::error(format!("Node '{}' not found.", args.node_id)),
    };

    if !doc.nodes.contains_key(&uid) {
        return ToolResult::error(format!("Node '{}' not found.", args.node_id));
    }

    let steps = args.steps.unwrap_or(1).max(1);
    let mut history = state.history.lock().await;

    match history.revert_node_steps(uid, steps, &mut doc) {
        Some(actual) => ToolResult::text(format!(
            "Reverted node '{}' by {} history step(s).",
            args.node_id, actual
        ))
        .with_data(serde_json::json!({
            "node_id": uid.to_string(),
            "steps_reverted": actual,
        })),
        None => ToolResult::text(format!(
            "Node '{}' has no edits in history — nothing to revert.",
            args.node_id
        ))
        .with_data(serde_json::json!({ "node_id": uid.to_string(), "steps_reverted": 0 })),
    }
}

/// Stack all children of a group at the same anchor point (z-stack).
/// Every child is repositioned so that its specified alignment anchor
/// aligns with the union bounding box of all children.
pub async fn apply_stack_layout(state: &AppState, args: ApplyStackLayoutArgs) -> ToolResult {
    tracing::debug!("tool: apply_stack_layout group={}", args.group_id);
    let mut doc = state.document.lock().await;

    let uid = match uuid::Uuid::parse_str(&args.group_id)
        .ok()
        .or_else(|| doc.find_node_by_name(&args.group_id).map(|n| n.id))
    {
        Some(id) => id,
        None => return ToolResult::error(format!("Node '{}' not found.", args.group_id)),
    };

    let group_node = match doc.nodes.get(&uid) {
        Some(n) => n.clone(),
        None => return ToolResult::error(format!("Node '{}' not found.", args.group_id)),
    };

    let child_ids = match &group_node.kind {
        SceneNodeKind::Group(g) => g.children.clone(),
        _ => return ToolResult::error("Target node is not a group."),
    };

    if child_ids.is_empty() {
        return ToolResult::text("Group has no children — nothing to stack.")
            .with_data(serde_json::json!({ "stacked": 0 }));
    }

    let align_h = args.align_h.as_deref().unwrap_or("center");
    let align_v = args.align_v.as_deref().unwrap_or("center");

    // Collect each child's current position and dimensions.
    let mut children: Vec<(NodeId, f64, f64, f64, f64)> = Vec::new(); // (id, tx, ty, w, h)
    let mut min_x = f64::MAX;
    let mut min_y = f64::MAX;
    let mut max_x = f64::MIN;
    let mut max_y = f64::MIN;

    for cid in &child_ids {
        let child = match doc.nodes.get(cid) {
            Some(n) => n.clone(),
            None => continue,
        };
        let (w, h) = match &child.kind {
            SceneNodeKind::Path(pn) => {
                if let Some(bb) = pn.path_data.bounding_box() {
                    (bb.width().abs().max(1.0), bb.height().abs().max(1.0))
                } else {
                    (60.0, 30.0)
                }
            }
            SceneNodeKind::Text(_) | SceneNodeKind::Group(_) => (60.0, 30.0),
        };
        let tx = child.transform.matrix[4];
        let ty = child.transform.matrix[5];
        min_x = min_x.min(tx);
        min_y = min_y.min(ty);
        max_x = max_x.max(tx + w);
        max_y = max_y.max(ty + h);
        children.push((*cid, tx, ty, w, h));
    }

    if children.is_empty() {
        return ToolResult::text("No accessible children found.")
            .with_data(serde_json::json!({ "stacked": 0 }));
    }

    // Union bounding box of all children.
    let union_x = min_x;
    let union_y = min_y;
    let union_w = (max_x - min_x).max(1.0);
    let union_h = (max_y - min_y).max(1.0);

    let mut history = state.history.lock().await;
    let count = children.len();

    for (cid, _tx, _ty, w, h) in &children {
        let new_tx = match align_h {
            "left" => union_x,
            "right" => union_x + union_w - w,
            _ => union_x + (union_w - w) / 2.0, // center
        };
        let new_ty = match align_v {
            "top" => union_y,
            "bottom" => union_y + union_h - h,
            _ => union_y + (union_h - h) / 2.0, // center
        };

        let child = match doc.nodes.get(cid) {
            Some(n) => n.clone(),
            None => continue,
        };
        let mut new_child = child.clone();
        new_child.transform.matrix[4] = new_tx;
        new_child.transform.matrix[5] = new_ty;
        history.execute(
            Command::UpdateNode {
                old: child,
                new: new_child,
            },
            &mut doc,
        );
    }

    ToolResult::text(format!(
        "Stacked {} children in '{}' (align_h={}, align_v={}).",
        count, args.group_id, align_h, align_v
    ))
    .with_data(serde_json::json!({
        "group_id": uid.to_string(),
        "stacked": count,
        "align_h": align_h,
        "align_v": align_v,
    }))
}

pub async fn apply_grid_layout(state: &AppState, args: ApplyGridLayoutArgs) -> ToolResult {
    let mut doc = state.document.lock().await;

    let uid = match uuid::Uuid::parse_str(&args.group_id)
        .ok()
        .or_else(|| doc.find_node_by_name(&args.group_id).map(|n| n.id))
    {
        Some(id) => id,
        None => return ToolResult::error(format!("Node '{}' not found.", args.group_id)),
    };

    let child_ids = match doc.nodes.get(&uid) {
        Some(n) => match &n.kind {
            SceneNodeKind::Group(g) => g.children.clone(),
            _ => return ToolResult::error("Target node is not a group."),
        },
        None => return ToolResult::error(format!("Node '{}' not found.", args.group_id)),
    };

    if child_ids.is_empty() {
        return ToolResult::text("Group has no children — nothing to layout.")
            .with_data(serde_json::json!({ "arranged": 0 }));
    }

    let cols = args.columns.unwrap_or(3).max(1);
    let gap_x = args.gap_x.unwrap_or(8.0);
    let gap_y = args.gap_y.unwrap_or(8.0);
    let padding = args.padding.unwrap_or(0.0);

    // Collect children with bounding sizes
    struct ChildInfo {
        id: NodeId,
        w: f64,
        h: f64,
    }
    let mut children: Vec<ChildInfo> = Vec::new();
    for cid in &child_ids {
        let child = match doc.nodes.get(cid) {
            Some(n) => n.clone(),
            None => continue,
        };
        let (w, h) = match &child.kind {
            SceneNodeKind::Path(pn) => {
                if let Some(bb) = pn.path_data.bounding_box() {
                    (bb.width().abs().max(1.0), bb.height().abs().max(1.0))
                } else {
                    (60.0, 30.0)
                }
            }
            _ => (60.0, 30.0),
        };
        children.push(ChildInfo { id: *cid, w, h });
    }

    // Compute column widths and row heights
    let n = children.len();
    let rows = (n + cols - 1) / cols;

    let col_width: f64 = children.iter().map(|c| c.w).fold(0.0_f64, f64::max);
    let row_height: f64 = children.iter().map(|c| c.h).fold(0.0_f64, f64::max);

    let mut commands: Vec<Command> = Vec::new();
    for (i, child) in children.iter().enumerate() {
        let col = i % cols;
        let row = i / cols;
        let new_tx = padding + col as f64 * (col_width + gap_x);
        let new_ty = padding + row as f64 * (row_height + gap_y);

        if let Some(old) = doc.nodes.get(&child.id) {
            let mut new_node = old.clone();
            new_node.transform.matrix[4] = new_tx;
            new_node.transform.matrix[5] = new_ty;
            commands.push(Command::UpdateNode {
                old: old.clone(),
                new: new_node,
            });
        }
    }

    let arranged = commands.len();
    let mut history = state.history.lock().await;
    history.execute(Command::Batch(commands), &mut doc);
    drop(history);

    ToolResult::text(format!(
        "Applied grid layout to {} children ({} cols × {} rows, gap={}×{}).",
        arranged, cols, rows, gap_x, gap_y
    ))
    .with_data(serde_json::json!({
        "group_id": uid.to_string(),
        "columns": cols,
        "rows": rows,
        "gap_x": gap_x,
        "gap_y": gap_y,
        "arranged": arranged,
    }))
}

/// Set (or add/remove) OpenType feature tags on a text node.
pub async fn set_opentype_features(state: &AppState, args: SetOpenTypeFeaturesArgs) -> ToolResult {
    tracing::debug!("tool: set_opentype_features");
    let mut doc = state.document.lock().await;

    let node_id = uuid::Uuid::parse_str(&args.node_id)
        .ok()
        .or_else(|| doc.find_node_by_name(&args.node_id).map(|n| n.id));
    let node_id = match node_id {
        Some(id) => id,
        None => return ToolResult::error(format!("Node '{}' not found.", args.node_id)),
    };

    let node = match doc.nodes.get(&node_id) {
        Some(n) if matches!(n.kind, SceneNodeKind::Text(_)) => n.clone(),
        Some(_) => {
            return ToolResult::error(format!("Node '{}' is not a text node.", args.node_id))
        }
        None => return ToolResult::error(format!("Node '{}' not found.", args.node_id)),
    };

    let mut new_node = node.clone();
    if let SceneNodeKind::Text(ref mut tn) = new_node.kind {
        let mode = if args.mode.is_empty() {
            "set"
        } else {
            args.mode.as_str()
        };
        match mode {
            "add" => {
                for f in &args.features {
                    if !tn.opentype_features.contains(f) {
                        tn.opentype_features.push(f.clone());
                    }
                }
            }
            "remove" => {
                tn.opentype_features.retain(|f| !args.features.contains(f));
            }
            _ => {
                // "set" is default
                tn.opentype_features = args.features.clone();
            }
        }
    }

    let features_after = match &new_node.kind {
        SceneNodeKind::Text(tn) => tn.opentype_features.clone(),
        _ => vec![],
    };

    let mut history = state.history.lock().await;
    history.execute(
        Command::UpdateNode {
            old: node,
            new: new_node,
        },
        &mut doc,
    );

    ToolResult::text(format!(
        "OpenType features on '{}' updated ({} active).",
        args.node_id,
        features_after.len()
    ))
    .with_data(serde_json::json!({ "node_id": node_id.to_string(), "features": features_after }))
}

/// Return the active OpenType feature tags on a text node.
pub async fn get_opentype_features(state: &AppState, args: GetOpenTypeFeaturesArgs) -> ToolResult {
    tracing::debug!("tool: get_opentype_features");
    let doc = state.document.lock().await;

    let node_id = uuid::Uuid::parse_str(&args.node_id)
        .ok()
        .or_else(|| doc.find_node_by_name(&args.node_id).map(|n| n.id));
    let node_id = match node_id {
        Some(id) => id,
        None => return ToolResult::error(format!("Node '{}' not found.", args.node_id)),
    };

    match doc.nodes.get(&node_id) {
        Some(n) => match &n.kind {
            SceneNodeKind::Text(tn) => {
                ToolResult::text(format!(
                    "Node '{}' has {} OpenType feature(s): {}",
                    args.node_id, tn.opentype_features.len(),
                    if tn.opentype_features.is_empty() { "(none — using font defaults)".to_string() }
                    else { tn.opentype_features.join(", ") }
                ))
                .with_data(serde_json::json!({ "node_id": node_id.to_string(), "features": tn.opentype_features }))
            }
            _ => ToolResult::error(format!("Node '{}' is not a text node.", args.node_id)),
        },
        None => ToolResult::error(format!("Node '{}' not found.", args.node_id)),
    }
}

/// Set the text decoration (underline, line-through, overline, or none) on a text node.
pub async fn set_text_decoration(state: &AppState, args: SetTextDecorationArgs) -> ToolResult {
    tracing::debug!("tool: set_text_decoration");

    let decoration = match args.decoration.to_lowercase().as_str() {
        "" | "none" => String::new(),
        "underline" | "line-through" | "overline" => args.decoration.to_lowercase(),
        other => {
            return ToolResult::error(format!(
                "Unknown decoration '{}'. Valid values: none, underline, line-through, overline.",
                other
            ))
        }
    };

    let mut doc = state.document.lock().await;

    let node_id = uuid::Uuid::parse_str(&args.node_id)
        .ok()
        .or_else(|| doc.find_node_by_name(&args.node_id).map(|n| n.id));
    let node_id = match node_id {
        Some(id) => id,
        None => return ToolResult::error(format!("Node '{}' not found.", args.node_id)),
    };

    let node = match doc.nodes.get(&node_id) {
        Some(n) if matches!(n.kind, SceneNodeKind::Text(_)) => n.clone(),
        Some(_) => {
            return ToolResult::error(format!("Node '{}' is not a text node.", args.node_id))
        }
        None => return ToolResult::error(format!("Node '{}' not found.", args.node_id)),
    };

    let mut new_node = node.clone();
    if let SceneNodeKind::Text(ref mut tn) = new_node.kind {
        tn.text_decoration = decoration.clone();
    }

    let mut history = state.history.lock().await;
    history.execute(
        Command::UpdateNode {
            old: node,
            new: new_node,
        },
        &mut doc,
    );

    ToolResult::text(format!(
        "Text decoration on '{}' set to '{}'.",
        args.node_id,
        if decoration.is_empty() {
            "none"
        } else {
            &decoration
        }
    ))
    .with_data(serde_json::json!({ "node_id": node_id.to_string(), "decoration": decoration }))
}

/// Set paragraph-level text options: spacing before/after paragraphs and first-line indent.
pub async fn set_paragraph_options(state: &AppState, args: SetParagraphOptionsArgs) -> ToolResult {
    tracing::debug!("tool: set_paragraph_options");
    let mut doc = state.document.lock().await;

    let node_id = uuid::Uuid::parse_str(&args.node_id)
        .ok()
        .or_else(|| doc.find_node_by_name(&args.node_id).map(|n| n.id));
    let node_id = match node_id {
        Some(id) => id,
        None => return ToolResult::error(format!("Node '{}' not found.", args.node_id)),
    };

    let node = match doc.nodes.get(&node_id) {
        Some(n) if matches!(n.kind, SceneNodeKind::Text(_)) => n.clone(),
        Some(_) => {
            return ToolResult::error(format!("Node '{}' is not a text node.", args.node_id))
        }
        None => return ToolResult::error(format!("Node '{}' not found.", args.node_id)),
    };

    let mut new_node = node.clone();
    if let SceneNodeKind::Text(ref mut tn) = new_node.kind {
        if let Some(v) = args.spacing_before {
            tn.paragraph_spacing_before = v;
        }
        if let Some(v) = args.spacing_after {
            tn.paragraph_spacing_after = v;
        }
        if let Some(v) = args.indent {
            tn.text_indent = v;
        }
    }

    let (sb, sa, ti) = match &new_node.kind {
        SceneNodeKind::Text(tn) => (
            tn.paragraph_spacing_before,
            tn.paragraph_spacing_after,
            tn.text_indent,
        ),
        _ => (0.0, 0.0, 0.0),
    };

    let mut history = state.history.lock().await;
    history.execute(
        Command::UpdateNode {
            old: node,
            new: new_node,
        },
        &mut doc,
    );

    ToolResult::text(format!(
        "Paragraph options on '{}': spacing_before={:.1}, spacing_after={:.1}, indent={:.1}.",
        args.node_id, sb, sa, ti
    ))
    .with_data(serde_json::json!({
        "node_id": node_id.to_string(),
        "spacing_before": sb, "spacing_after": sa, "indent": ti
    }))
}

/// Set explicit tab stop positions on a text node.
pub async fn set_tab_stops(state: &AppState, args: SetTabStopsArgs) -> ToolResult {
    tracing::debug!("tool: set_tab_stops");
    let mut doc = state.document.lock().await;

    let node_id = uuid::Uuid::parse_str(&args.node_id)
        .ok()
        .or_else(|| doc.find_node_by_name(&args.node_id).map(|n| n.id));
    let node_id = match node_id {
        Some(id) => id,
        None => return ToolResult::error(format!("Node '{}' not found.", args.node_id)),
    };

    let node = match doc.nodes.get(&node_id) {
        Some(n) if matches!(n.kind, SceneNodeKind::Text(_)) => n.clone(),
        Some(_) => {
            return ToolResult::error(format!("Node '{}' is not a text node.", args.node_id))
        }
        None => return ToolResult::error(format!("Node '{}' not found.", args.node_id)),
    };

    if args.stops.is_empty() {
        return ToolResult::error(
            "stops must contain at least one position. Use clear_tab_stops to reset to defaults.",
        );
    }

    let mut sorted_stops = args.stops.clone();
    sorted_stops.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let mut new_node = node.clone();
    if let SceneNodeKind::Text(ref mut tn) = new_node.kind {
        tn.tab_stops = sorted_stops.clone();
    }

    let mut history = state.history.lock().await;
    history.execute(
        Command::UpdateNode {
            old: node,
            new: new_node,
        },
        &mut doc,
    );

    ToolResult::text(format!(
        "Set {} tab stop(s) on '{}': {:?}",
        sorted_stops.len(),
        args.node_id,
        sorted_stops
    ))
    .with_data(serde_json::json!({
        "node_id": node_id.to_string(),
        "tab_stops": sorted_stops,
    }))
}

/// Clear custom tab stops on a text node, restoring default tab spacing.
pub async fn clear_tab_stops(state: &AppState, args: ClearTabStopsArgs) -> ToolResult {
    tracing::debug!("tool: clear_tab_stops");
    let mut doc = state.document.lock().await;

    let node_id = uuid::Uuid::parse_str(&args.node_id)
        .ok()
        .or_else(|| doc.find_node_by_name(&args.node_id).map(|n| n.id));
    let node_id = match node_id {
        Some(id) => id,
        None => return ToolResult::error(format!("Node '{}' not found.", args.node_id)),
    };

    let node = match doc.nodes.get(&node_id) {
        Some(n) if matches!(n.kind, SceneNodeKind::Text(_)) => n.clone(),
        Some(_) => {
            return ToolResult::error(format!("Node '{}' is not a text node.", args.node_id))
        }
        None => return ToolResult::error(format!("Node '{}' not found.", args.node_id)),
    };

    let mut new_node = node.clone();
    if let SceneNodeKind::Text(ref mut tn) = new_node.kind {
        tn.tab_stops.clear();
    }

    let mut history = state.history.lock().await;
    history.execute(
        Command::UpdateNode {
            old: node,
            new: new_node,
        },
        &mut doc,
    );

    ToolResult::text(format!(
        "Cleared tab stops on '{}'. Default tab spacing restored.",
        args.node_id
    ))
    .with_data(serde_json::json!({ "node_id": node_id.to_string(), "tab_stops": [] }))
}

/// Assign a path node as the blend spine for a group node.
pub async fn set_blend_spine(state: &AppState, args: SetBlendSpineArgs) -> ToolResult {
    tracing::debug!("tool: set_blend_spine");
    let mut doc = state.document.lock().await;

    let group_id = uuid::Uuid::parse_str(&args.group_id)
        .ok()
        .or_else(|| doc.find_node_by_name(&args.group_id).map(|n| n.id));
    let group_id = match group_id {
        Some(id) => id,
        None => return ToolResult::error(format!("Group '{}' not found.", args.group_id)),
    };

    let path_id = uuid::Uuid::parse_str(&args.path_id)
        .ok()
        .or_else(|| doc.find_node_by_name(&args.path_id).map(|n| n.id));
    let path_id = match path_id {
        Some(id) => id,
        None => return ToolResult::error(format!("Path '{}' not found.", args.path_id)),
    };

    let group_node = match doc.nodes.get(&group_id) {
        Some(n) if matches!(n.kind, SceneNodeKind::Group(_)) => n.clone(),
        Some(_) => return ToolResult::error(format!("Node '{}' is not a group.", args.group_id)),
        None => return ToolResult::error(format!("Group '{}' not found.", args.group_id)),
    };

    // Validate path node exists and is a path
    match doc.nodes.get(&path_id) {
        Some(n) if matches!(n.kind, SceneNodeKind::Path(_)) => {}
        Some(_) => {
            return ToolResult::error(format!("Node '{}' is not a path node.", args.path_id))
        }
        None => return ToolResult::error(format!("Path '{}' not found.", args.path_id)),
    }

    let mut new_group = group_node.clone();
    if let SceneNodeKind::Group(ref mut gn) = new_group.kind {
        gn.blend_spine_id = Some(path_id);
    }

    let mut history = state.history.lock().await;
    history.execute(
        Command::UpdateNode {
            old: group_node,
            new: new_group,
        },
        &mut doc,
    );

    ToolResult::text(format!(
        "Blend spine of group '{}' set to path '{}'.",
        args.group_id, args.path_id
    ))
    .with_data(serde_json::json!({
        "group_id": group_id.to_string(),
        "path_id": path_id.to_string()
    }))
}

/// Clear the blend spine assignment from a group node.
pub async fn clear_blend_spine(state: &AppState, args: ClearBlendSpineArgs) -> ToolResult {
    tracing::debug!("tool: clear_blend_spine");
    let mut doc = state.document.lock().await;

    let group_id = uuid::Uuid::parse_str(&args.group_id)
        .ok()
        .or_else(|| doc.find_node_by_name(&args.group_id).map(|n| n.id));
    let group_id = match group_id {
        Some(id) => id,
        None => return ToolResult::error(format!("Group '{}' not found.", args.group_id)),
    };

    let group_node = match doc.nodes.get(&group_id) {
        Some(n) if matches!(n.kind, SceneNodeKind::Group(_)) => n.clone(),
        Some(_) => return ToolResult::error(format!("Node '{}' is not a group.", args.group_id)),
        None => return ToolResult::error(format!("Group '{}' not found.", args.group_id)),
    };

    if let SceneNodeKind::Group(ref gn) = group_node.kind {
        if gn.blend_spine_id.is_none() {
            return ToolResult::text(format!(
                "Group '{}' has no blend spine assigned.",
                args.group_id
            ));
        }
    }

    let mut new_group = group_node.clone();
    if let SceneNodeKind::Group(ref mut gn) = new_group.kind {
        gn.blend_spine_id = None;
    }

    let mut history = state.history.lock().await;
    history.execute(
        Command::UpdateNode {
            old: group_node,
            new: new_group,
        },
        &mut doc,
    );

    ToolResult::text(format!(
        "Blend spine cleared from group '{}'.",
        args.group_id
    ))
    .with_data(serde_json::json!({ "group_id": group_id.to_string() }))
}

/// Reverse the direction of the blend spine path in a group node.
pub async fn reverse_blend_spine(state: &AppState, args: ReverseBlendSpineArgs) -> ToolResult {
    tracing::debug!("tool: reverse_blend_spine");
    let mut doc = state.document.lock().await;

    let group_id = uuid::Uuid::parse_str(&args.group_id)
        .ok()
        .or_else(|| doc.find_node_by_name(&args.group_id).map(|n| n.id));
    let group_id = match group_id {
        Some(id) => id,
        None => return ToolResult::error(format!("Group '{}' not found.", args.group_id)),
    };

    // Resolve the spine ID from the group
    let spine_id = match doc.nodes.get(&group_id) {
        Some(n) => match &n.kind {
            SceneNodeKind::Group(gn) => match gn.blend_spine_id {
                Some(sid) => sid,
                None => {
                    return ToolResult::error(format!(
                        "Group '{}' has no blend spine assigned.",
                        args.group_id
                    ))
                }
            },
            _ => return ToolResult::error(format!("Node '{}' is not a group.", args.group_id)),
        },
        None => return ToolResult::error(format!("Group '{}' not found.", args.group_id)),
    };

    let spine_node = match doc.nodes.get(&spine_id) {
        Some(n) if matches!(n.kind, SceneNodeKind::Path(_)) => n.clone(),
        Some(_) => return ToolResult::error("Blend spine node is not a path."),
        None => return ToolResult::error("Blend spine node not found in document."),
    };

    let mut new_spine = spine_node.clone();
    if let SceneNodeKind::Path(ref mut pn) = new_spine.kind {
        pn.path_data = pn.path_data.reverse();
    }

    let mut history = state.history.lock().await;
    history.execute(
        Command::UpdateNode {
            old: spine_node,
            new: new_spine,
        },
        &mut doc,
    );

    ToolResult::text(format!(
        "Blend spine of group '{}' reversed.",
        args.group_id
    ))
    .with_data(serde_json::json!({
        "group_id": group_id.to_string(),
        "spine_id": spine_id.to_string()
    }))
}

/// Expand a blend group into individual discrete objects at the parent layer.
/// Semantically equivalent to Illustrator's Object > Blend > Expand.
pub async fn expand_blend(state: &AppState, args: ExpandBlendArgs) -> ToolResult {
    tracing::debug!("tool: expand_blend");
    let mut doc = state.document.lock().await;

    let group_id_str = args.group_id.clone();
    let group_id = uuid::Uuid::parse_str(&group_id_str)
        .ok()
        .or_else(|| doc.find_node_by_name(&group_id_str).map(|n| n.id));
    let group_id = match group_id {
        Some(id) => id,
        None => return ToolResult::error(format!("Group '{}' not found.", group_id_str)),
    };

    let group_node = match doc.nodes.get(&group_id) {
        Some(n) if matches!(n.kind, SceneNodeKind::Group(_)) => n.clone(),
        Some(_) => return ToolResult::error(format!("Node '{}' is not a group.", group_id_str)),
        None => return ToolResult::error(format!("Group '{}' not found.", group_id_str)),
    };

    let children = match &group_node.kind {
        SceneNodeKind::Group(g) => g.children.clone(),
        _ => unreachable!(),
    };

    let child_count = children.len();

    let (layer_id, group_index) = match doc.node_layer_and_index(&group_id) {
        Some(v) => v,
        None => return ToolResult::error("Blend group has no layer position."),
    };

    let cmd = Command::UngroupNodes {
        group: group_node,
        layer_id,
        group_index,
        children,
    };
    let mut history = state.history.lock().await;
    history.execute(cmd, &mut doc);

    ToolResult::text(format!(
        "Expanded blend group '{}' into {} individual object(s).",
        group_id_str, child_count
    ))
    .with_data(serde_json::json!({
        "group_id": group_id.to_string(),
        "child_count": child_count
    }))
}

/// Set per-instance fill and/or stroke color overrides on a symbol instance node.
pub async fn set_symbol_override(state: &AppState, args: SetSymbolOverrideArgs) -> ToolResult {
    tracing::debug!("tool: set_symbol_override");
    let mut doc = state.document.lock().await;

    let node_id = uuid::Uuid::parse_str(&args.node_id)
        .ok()
        .or_else(|| doc.find_node_by_name(&args.node_id).map(|n| n.id));
    let node_id = match node_id {
        Some(id) => id,
        None => return ToolResult::error(format!("Node '{}' not found.", args.node_id)),
    };

    let node = match doc.nodes.get(&node_id) {
        Some(n) => n.clone(),
        None => return ToolResult::error(format!("Node '{}' not found.", args.node_id)),
    };

    if node.symbol_ref.is_none() {
        return ToolResult::error(format!("Node '{}' is not a symbol instance.", args.node_id));
    }

    let mut new_node = node.clone();
    if let Some(hex) = args.fill_hex {
        new_node.symbol_fill_override = Some(hex);
    }
    if let Some(hex) = args.stroke_hex {
        new_node.symbol_stroke_override = Some(hex);
    }

    let fill_out = new_node.symbol_fill_override.clone();
    let stroke_out = new_node.symbol_stroke_override.clone();

    let mut history = state.history.lock().await;
    history.execute(
        Command::UpdateNode {
            old: node,
            new: new_node,
        },
        &mut doc,
    );

    ToolResult::text(format!(
        "Symbol overrides set on '{}': fill={:?}, stroke={:?}.",
        args.node_id, fill_out, stroke_out
    ))
    .with_data(serde_json::json!({
        "node_id": node_id.to_string(),
        "fill_override": fill_out,
        "stroke_override": stroke_out
    }))
}

/// Clear all per-instance color overrides on a symbol instance node.
pub async fn clear_symbol_overrides(
    state: &AppState,
    args: ClearSymbolOverridesArgs,
) -> ToolResult {
    tracing::debug!("tool: clear_symbol_overrides");
    let mut doc = state.document.lock().await;

    let node_id = uuid::Uuid::parse_str(&args.node_id)
        .ok()
        .or_else(|| doc.find_node_by_name(&args.node_id).map(|n| n.id));
    let node_id = match node_id {
        Some(id) => id,
        None => return ToolResult::error(format!("Node '{}' not found.", args.node_id)),
    };

    let node = match doc.nodes.get(&node_id) {
        Some(n) => n.clone(),
        None => return ToolResult::error(format!("Node '{}' not found.", args.node_id)),
    };

    if node.symbol_ref.is_none() {
        return ToolResult::error(format!("Node '{}' is not a symbol instance.", args.node_id));
    }

    if node.symbol_fill_override.is_none() && node.symbol_stroke_override.is_none() {
        return ToolResult::text(format!("Node '{}' has no symbol overrides.", args.node_id));
    }

    let mut new_node = node.clone();
    new_node.symbol_fill_override = None;
    new_node.symbol_stroke_override = None;

    let mut history = state.history.lock().await;
    history.execute(
        Command::UpdateNode {
            old: node,
            new: new_node,
        },
        &mut doc,
    );

    ToolResult::text(format!("Symbol overrides cleared on '{}'.", args.node_id))
        .with_data(serde_json::json!({ "node_id": node_id.to_string() }))
}

/// Create N evenly-spaced rotational copies of a node around a center point.
pub async fn rotate_copies(state: &AppState, args: RotateCopiesArgs) -> ToolResult {
    tracing::debug!("tool: rotate_copies count={}", args.count);
    use photonic_core::transform::Transform;

    if args.count < 2 {
        return ToolResult::error("count must be at least 2.");
    }

    let mut doc = state.document.lock().await;

    let src_id = match uuid::Uuid::parse_str(&args.node_id)
        .ok()
        .or_else(|| doc.find_node_by_name(&args.node_id).map(|n| n.id))
    {
        Some(id) => id,
        None => return ToolResult::error(format!("Node '{}' not found.", args.node_id)),
    };

    let src_node = match doc.nodes.get(&src_id) {
        Some(n) => n.clone(),
        None => return ToolResult::error(format!("Node '{}' not found.", args.node_id)),
    };

    let layer_id = src_node.layer_id;

    // Determine rotation center
    let (cx, cy) = if let (Some(cx), Some(cy)) = (args.cx, args.cy) {
        (cx, cy)
    } else if let Some(lb) = src_node.local_bounds() {
        let (x0, y0) = src_node.transform.apply(lb.x0, lb.y0);
        let (x1, y1) = src_node.transform.apply(lb.x1, lb.y1);
        ((x0 + x1) / 2.0, (y0 + y1) / 2.0)
    } else {
        src_node.transform.apply(0.0, 0.0)
    };

    let angle_step = std::f64::consts::TAU / args.count as f64;
    let mut cmds: Vec<Command> = Vec::new();
    let mut copy_ids: Vec<NodeId> = vec![src_id];

    // Create count-1 copies
    for i in 1..args.count {
        let angle = angle_step * i as f64;
        let rot = Transform::rotate_around(angle, cx, cy);
        let mut copy = src_node.clone();
        copy.id = uuid::Uuid::new_v4();
        copy.name = format!("{} copy {}", src_node.name, i);
        // Compose: rot applied after existing transform
        copy.transform = src_node.transform.then(&rot);
        // Fix translation: rotate the world-space position
        let (orig_tx, orig_ty) = (src_node.transform.matrix[4], src_node.transform.matrix[5]);
        let (rot_tx, rot_ty) = rot.apply(orig_tx, orig_ty);
        copy.transform.matrix[4] = rot_tx;
        copy.transform.matrix[5] = rot_ty;
        copy_ids.push(copy.id);
        cmds.push(Command::AddNode {
            node: copy,
            layer_id: Some(layer_id),
        });
    }

    // Optionally wrap in a group
    if args.group {
        // First add all copies, then group them with the original
        let all_ids = copy_ids.clone();
        let mut history = state.history.lock().await;
        for cmd in cmds {
            history.execute(cmd, &mut doc);
        }
        // Group: create a group with all ids
        let group_node = photonic_core::node::SceneNode::new(
            format!("{} ×{}", src_node.name, args.count),
            layer_id,
            SceneNodeKind::Group(GroupNode {
                children: all_ids.clone(),
                ..Default::default()
            }),
        );
        let group_id = group_node.id;
        let cmd = Command::GroupNodes {
            group: group_node,
            children: all_ids,
            layer_id,
            insert_index: 0,
        };
        history.execute(cmd, &mut doc);
        ToolResult::text(format!(
            "Created {} rotational copies grouped as one node.",
            args.count - 1
        ))
        .with_data(serde_json::json!({ "group_id": group_id.to_string(), "count": args.count }))
    } else {
        let mut history = state.history.lock().await;
        let batch = Command::Batch(cmds);
        history.execute(batch, &mut doc);
        ToolResult::text(format!(
            "Created {} rotational copies of '{}'.",
            args.count - 1,
            src_node.name
        ))
        .with_data(serde_json::json!({
            "source_id": src_id.to_string(),
            "copy_ids": copy_ids.iter().skip(1).map(|id| id.to_string()).collect::<Vec<_>>(),
            "count": args.count,
            "center": [cx, cy],
        }))
    }
}

pub async fn copy_appearance(state: &AppState, args: CopyAppearanceArgs) -> ToolResult {
    if args.target_ids.is_empty() {
        return ToolResult::text("No target nodes specified.");
    }
    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    // Resolve source node
    let src_id = {
        let id_res = uuid::Uuid::parse_str(&args.source_id);
        if let Ok(uuid) = id_res {
            if doc.nodes.contains_key(&uuid) {
                uuid
            } else {
                return ToolResult::text(format!("Source node '{}' not found.", args.source_id));
            }
        } else {
            match doc.nodes.values().find(|n| n.name == args.source_id) {
                Some(n) => n.id,
                None => {
                    return ToolResult::text(format!("Source node '{}' not found.", args.source_id))
                }
            }
        }
    };

    let (src_fill, src_stroke, src_opacity) = {
        let src = &doc.nodes[&src_id];
        let fill = if let SceneNodeKind::Path(ref p) = src.kind {
            Some(p.fill.clone())
        } else {
            None
        };
        let stroke = if let SceneNodeKind::Path(ref p) = src.kind {
            Some(p.stroke.clone())
        } else {
            None
        };
        (fill, stroke, src.opacity)
    };

    let mut cmds: Vec<Command> = Vec::new();
    let mut updated = 0usize;

    for tid_str in &args.target_ids {
        let tid = if let Ok(uuid) = uuid::Uuid::parse_str(tid_str) {
            if doc.nodes.contains_key(&uuid) {
                uuid
            } else {
                continue;
            }
        } else {
            match doc.nodes.values().find(|n| n.name == *tid_str) {
                Some(n) => n.id,
                None => continue,
            }
        };

        if tid == src_id {
            continue;
        }
        let mut new_node = doc.nodes[&tid].clone();
        let old_node = new_node.clone();

        if args.copy_opacity {
            new_node.opacity = src_opacity;
        }
        if let SceneNodeKind::Path(ref mut p) = new_node.kind {
            if args.copy_fill {
                if let Some(ref f) = src_fill {
                    p.fill = f.clone();
                }
            }
            if args.copy_stroke {
                if let Some(ref s) = src_stroke {
                    p.stroke = s.clone();
                }
            }
        }
        cmds.push(Command::UpdateNode {
            old: old_node,
            new: new_node,
        });
        updated += 1;
    }

    if cmds.is_empty() {
        return ToolResult::text("No valid target nodes found.");
    }

    let batch = if cmds.len() == 1 {
        cmds.remove(0)
    } else {
        Command::Batch(cmds)
    };
    history.execute(batch, &mut doc);
    ToolResult::text(format!(
        "Copied appearance from '{}' to {} node(s).",
        args.source_id, updated
    ))
    .with_data(serde_json::json!({ "updated": updated }))
}
