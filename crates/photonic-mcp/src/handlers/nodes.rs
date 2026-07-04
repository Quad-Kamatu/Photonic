use crate::handlers::shared::{
    paths::{
        apply_pucker_bloat, apply_roughen, apply_round_corners, apply_twirl, apply_zig_zag,
    },
    random::xorshift64,
    styling::{apply_stroke_paint, apply_style},
};
pub use crate::handlers::selection::{
    deselect_all, find_nodes, find_replace_style, find_replace_text, get_selection, lasso_select,
    magic_wand_select, select_all, select_by_kind, select_inside_group, select_same, select_similar,
    set_selection,
};
pub use crate::handlers::transform::{
    align_nodes, apply_flex_layout, apply_grid_layout, apply_stack_layout, apply_transform,
    center_on_canvas, create_array, distribute_no_overlap, distribute_on_path, duplicate_nodes,
    fit_to_canvas, flatten_group, flip_nodes, layout_nodes, mirror_copy, reorder_node,
    reverse_node_order, rotate_copies, scatter_copies, snap_to_pixel, split_into_grid,
    transform_copies,
};
pub use crate::handlers::guides::{
    add_dimension_line, add_guide, clear_guides, list_guides, pin_object_guides, remove_guide,
};
pub use crate::handlers::clipping::{
    make_clipping_mask, make_compound_path, release_clipping_mask, release_compound_path,
};
pub use crate::handlers::pathfinder::{
    boolean_operation, divide_objects_below, pathfinder_crop, pathfinder_divide,
    pathfinder_merge, pathfinder_minus_back, pathfinder_minus_front, pathfinder_outline,
    pathfinder_trim,
};
pub use crate::handlers::charts::{
    create_bar_chart, create_line_chart, create_pie_chart, create_radar_chart,
    create_scatter_plot, create_stacked_bar_chart,
};
use crate::protocol::{
    AddAnchorPointsArgs, AddDropShadowArgs, AdjustColorsArgs,
    AlignAnchor, AlignNodesArgs, AlignOperation, ApplyCharacterStyleArgs, ApplyFlexLayoutArgs,
    ApplyGridLayoutArgs, ApplyParagraphStyleArgs, ApplyStackLayoutArgs, ApplyTransformArgs,
    ArrayMode, AutoNameNodesArgs, AverageAnchorPointsArgs, BindTextVariableArgs, BlendColorsArgs,
    BlendObjectsArgs, BuildShapeFromPointsArgs, CenterOnCanvasArgs,
    CheckStyleContinuityArgs, CleanUpArgs, ClearBlendSpineArgs,
    ClearSymbolOverridesArgs, ClearTabStopsArgs, ClearTextAreaArgs, ClearTextPathArgs,
    ConvertAnchorMode, ConvertAnchorPointsArgs, ConvertToGrayscaleArgs, CopyAppearanceArgs,
    CreateArrayArgs, CreateArrowShapeArgs, CreateCharacterStyleArgs,
    CreateCrossArgs, CreateCurvaturePathArgs, CreateDonutArgs, CreateFlareArgs,
    CreateFreehandPathArgs, CreateGearArgs, CreateGridArgs, CreateHeartArgs,
    CreateParagraphStyleArgs, CreateParametricShapeArgs, CreatePathArgs,
    CreatePolarGridArgs, CreateShapeArgs,
    CreateSpeechBubbleArgs, CreateSpiralArgs, CreateSunburstArgs,
    CreateTextArgs, CreateTruchetTilingArgs, CreateWavePatternArgs, CrossAxisAlign,
    CrystallizePathArgs, DeleteAnchorPointArgs, DeleteCharacterStyleArgs, DeleteNodeArgs,
    DeleteParagraphStyleArgs, DeselectAllArgs, DistributeNoOverlapArgs, DistributeOnPathArgs,
    DuplicateNodesArgs, EnterIsolationModeArgs, ExitIsolationModeArgs,
    ExpandBlendArgs, ExportTaggedAssetsArgs, FindNodesArgs, FindReplaceStyleArgs,
    FindReplaceTextArgs, FitToCanvasArgs, FlattenGroupArgs, FlattenTransparencyArgs, FlipNodesArgs,
    GetCssPreviewArgs, GetNodeArgs, GetNodePromptsArgs, GetOpenTypeFeaturesArgs,
    GetRecentColorsArgs, GroupNodesArgs, HatchFillArgs, InspectNodeArgs, InvertColorsArgs,
    JoinPathsArgs, LassoSelectArgs, LayoutMode, LayoutNodesArgs, LinkTextFramesArgs,
    MagicWandSelectArgs,
    MeasureDistanceArgs, MeasurePathArgs, MeasureTarget, MirrorCopyArgs, MoveToLayerArgs,
    NoiseDeformArgs, ObjectKindFilter, OffsetPathArgs, OutlineStrokeArgs, ParametricShapeType,
    PointOnPathArgs, PuckerBloatArgs, RandomizeColorsArgs, RecolorArtworkArgs,
    RemoveStyleArgs,
    ReorderNodeArgs, ReorderOperation, ReverseBlendSpineArgs, ReverseNodeOrderArgs,
    ReversePathDirectionArgs, RotateCopiesArgs, RoughenPathArgs, RoundCornersArgs,
    SampleColorAtArgs, ScallopPathArgs, ScatterCopiesArgs, ScissorsCutArgs, SelectAllArgs,
    SelectByKindArgs, SelectInsideGroupArgs, SelectSameArgs, SelectSameAttribute,
    SelectSimilarArgs, SetBlendModeArgs, SetBlendSpineArgs, SetCharacterMetricsArgs,
    SetFontStyleArgs, SetFontWeightArgs, SetLockedArgs, SetNodePromptArgs, SetOpacityArgs,
    SetPaintArgs,
    SetOpenTypeFeaturesArgs, SetParagraphOptionsArgs, SetSelectionArgs, SetSymbolOverrideArgs,
    SetTabStopsArgs, SetTextAreaArgs, SetTextDecorationArgs, SetTextDirectionArgs, SetTextPathArgs,
    SetVisibilityArgs, ShapeType, SimplifyPathArgs, SmoothPathArgs, SnapToPixelArgs,
    SplitIntoGridArgs, StippleFillArgs, StyleTransferArgs, SwapFillStrokeArgs,
    TagNodeForExportArgs, TagNodesArgs, ToolResult, TransformCopiesArgs, TwirlPathArgs,
    UnbindTextVariableArgs, UndoNodeArgs, UngroupNodesArgs, UnlinkTextFramesArgs, UpdateNodeArgs,
    WarpEnvelopeArgs, ZigZagPathArgs,
};
use crate::server::AppState;
use kurbo;
use photonic_core::{
    history::Command,
    layer::BlendMode,
    node::{FontStyle, GroupNode, NodeId, PathNode, SceneNode, SceneNodeKind, TextNode},
    path::PathData,
    transform::Transform,
};


/// #202: apply one paint to many nodes in a single undoable call, each re-fit to
/// its own bounding box (bbox-relative gradients). Reuses the `fill` paint shape.
pub async fn set_paint(state: &AppState, args: SetPaintArgs) -> ToolResult {
    use photonic_core::history::Command;

    if args.node_ids.is_empty() {
        return ToolResult::error("node_ids must not be empty.");
    }
    let target = args
        .target
        .as_deref()
        .unwrap_or("fill")
        .to_ascii_lowercase();
    if target != "fill" && target != "stroke" {
        return ToolResult::error(format!(
            "Invalid target '{target}' (expected \"fill\" or \"stroke\")."
        ));
    }

    let mut doc = state.document.lock().await;
    let mut commands = Vec::new();
    let mut applied = 0usize;
    let mut skipped: Vec<String> = Vec::new();

    for id_str in &args.node_ids {
        let nid = uuid::Uuid::parse_str(id_str)
            .ok()
            .or_else(|| doc.find_node_by_name(id_str).map(|n| n.id));
        let Some(nid) = nid else {
            skipped.push(id_str.clone());
            continue;
        };
        let Some(node) = doc.nodes.get(&nid) else {
            skipped.push(id_str.clone());
            continue;
        };

        // World-space bounding box (x, y, w, h) for bbox-relative resolution.
        let bbox = node
            .local_bounds()
            .map(|lb| {
                let (x0, y0) = node.transform.apply(lb.x0, lb.y0);
                let (x1, y1) = node.transform.apply(lb.x1, lb.y1);
                (
                    x0.min(x1),
                    y0.min(y1),
                    (x1 - x0).abs().max(1e-6),
                    (y1 - y0).abs().max(1e-6),
                )
            })
            .unwrap_or((0.0, 0.0, 1.0, 1.0));

        let fill = match args.paint.resolved_for_bbox(bbox).to_fill() {
            Ok(f) => f,
            Err(e) => return ToolResult::error(e),
        };

        let mut new_node = node.clone();
        let ok = match &mut new_node.kind {
            SceneNodeKind::Path(pn) => {
                if target == "fill" {
                    pn.fill = fill;
                } else {
                    apply_stroke_paint(&mut pn.stroke, &fill);
                }
                true
            }
            SceneNodeKind::Text(tn) => {
                if target == "fill" {
                    tn.fill = fill;
                } else {
                    apply_stroke_paint(&mut tn.stroke, &fill);
                }
                true
            }
            _ => false,
        };
        if ok {
            commands.push(Command::UpdateNode {
                old: node.clone(),
                new: new_node,
            });
            applied += 1;
        } else {
            skipped.push(id_str.clone());
        }
    }

    if commands.is_empty() {
        return ToolResult::error(format!(
            "No paintable nodes found ({} skipped).",
            skipped.len()
        ));
    }

    let mut history = state.history.lock().await;
    for cmd in commands {
        history.execute_discrete(cmd, &mut doc);
    }
    drop(history);

    ToolResult::text(format!(
        "Applied {target} paint to {applied} node(s){}.",
        if skipped.is_empty() {
            String::new()
        } else {
            format!(" ({} skipped)", skipped.len())
        }
    ))
    .with_data(serde_json::json!({
        "applied_count": applied,
        "target": target,
        "skipped": skipped,
    }))
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
    let has_explicit_fill = args.fill.is_some();
    if let Err(e) = apply_style(&mut path_node, args.fill, args.stroke) {
        return ToolResult::error(e);
    }
    // `color` is a convenience shorthand for a solid fill. An explicit `fill`
    // takes priority; a bad hex string is reported rather than silently ignored.
    if !has_explicit_fill {
        if let Some(hex) = &args.color {
            match photonic_core::color::Color::from_hex(hex) {
                Some(c) => path_node.fill = photonic_core::style::Fill::solid(c),
                None => {
                    return ToolResult::error(format!("Invalid color '{hex}' (expected #rrggbb)"))
                }
            }
        }
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
    history.execute_discrete(cmd.clone(), &mut doc);

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
    history.execute_discrete(cmd, &mut doc);

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
    history.execute_discrete(cmd, &mut doc);

    ToolResult::text(format!(
        "Created smooth curve through {} points (id: {})",
        pts.len(),
        node_id
    ))
    .with_data(serde_json::json!({ "node_id": node_id, "point_count": pts.len() }))
}

/// Convert a sequence of points to a smooth cubic bezier path using Catmull-Rom interpolation.
/// The tension parameter is fixed at 0 (uniform Catmull-Rom = smooth interpolation).
pub(crate) fn catmull_rom_to_bezier(points: &[kurbo::Point], closed: bool) -> kurbo::BezPath {
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
        history.execute_discrete(
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
        history.execute_discrete(
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
        history.execute_discrete(
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
    history.execute_discrete(
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
    history.execute_discrete(cmd, &mut doc);

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
    history.execute_discrete(cmd, &mut doc);

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
    history.execute_discrete(cmd, &mut doc);

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
    history.execute_discrete(cmd, &mut doc);

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
        // raster: no vector fill/stroke/text properties to update
        SceneNodeKind::Raster(_) => {}
    }

    // Write phase: acquire both locks, execute synchronously, release both.
    let cmd = Command::UpdateNode {
        old: old_node,
        new: new_node,
    };
    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    history.execute_discrete(cmd, &mut doc);

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
    history.execute_discrete(cmd, &mut doc);
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
    history.execute_discrete(cmd, &mut doc);

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
    history.execute_discrete(cmd, &mut doc);

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
    history.execute_discrete(cmd, &mut doc);

    ToolResult::text(format!(
        "Ungrouped {} into {} child node(s)",
        args.group_id, child_count
    ))
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
    history.execute_discrete(cmd, &mut doc);

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
pub(crate) fn style_prop_enabled(properties: &Option<Vec<String>>, prop: &str) -> bool {
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
        history.execute_discrete(cmd, &mut doc);
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


// ─── find_replace_text ───────────────────────────────────────────────────────


// ─── layout_nodes ────────────────────────────────────────────────────────────


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
                    // raster: no anchors/fill/stroke to aggregate
                    SceneNodeKind::Raster(_) => {}
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
                "baseline_shift": text_node.baseline_shift,
                "script_position": text_node.script_position.as_str(),
            });

            ToolResult::text(format!(
                "inspect_node '{}': text, {} char(s), {} line(s), font '{}'",
                name, char_count, line_count, text_node.font_family
            ))
            .with_data(data)
        }

        // raster: pixel layer — no vector geometry, fill, or stroke
        SceneNodeKind::Raster(_) => {
            let world_bounds = world_aabb_of(&node).map(aabb_to_json);

            let data = serde_json::json!({
                "id": id_str,
                "name": name,
                "type": "raster",
                "world_bounds": world_bounds,
            });

            ToolResult::text(format!("inspect_node '{}': raster (pixel layer)", name))
                .with_data(data)
        }
    }
}

// ─── auto_name_nodes ──────────────────────────────────────────────────────────

/// Returns true if `name` looks like an auto-generated default (should be renamed).
pub(crate) fn is_generic_name(name: &str) -> bool {
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
pub(crate) fn color_label(r: f32, g: f32, b: f32) -> &'static str {
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
pub(crate) fn generate_name(node: &SceneNode) -> String {
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
                    FillKind::Pattern(_) => "pattern".to_string(),
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
        // raster: pixel layer — no fill/geometry to describe
        SceneNodeKind::Raster(_) => "raster".to_string(),
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
    history.execute_discrete(batch, &mut doc);

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
            FillKind::Pattern(p) => {
                use base64::Engine;
                let png = p.tile.to_png();
                let b64 = base64::engine::general_purpose::STANDARD.encode(png);
                let size = (p.tile.width.max(1) as f64 + p.spacing.max(0.0)) * p.scale.max(0.001);
                lines.push(format!(
                    "background-image: url(data:image/png;base64,{b64});"
                ));
                lines.push("background-repeat: repeat;".to_string());
                lines.push(format!("background-size: {:.1}px;", size));
                notes.push(
                    "Pattern fill exported as a repeating CSS background image (grid layout); brick/hex staggers are approximated."
                        .to_string(),
                );
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
        // raster: no vector fill or stroke
        SceneNodeKind::Raster(_) => {
            notes.push(
                "Raster nodes have no fill or stroke — CSS shown covers size and positioning only."
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
        SceneNodeKind::Raster(_) => "raster",
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
            // raster: no vector fill/stroke/font — opacity analysis only
            SceneNodeKind::Raster(_) => {
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
        history.execute_discrete(cmd, &mut doc);
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
    history.execute_discrete(
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
        history.execute_discrete(cmd, &mut doc);
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
        history.execute_discrete(cmd, &mut doc);
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
pub(crate) fn path_centroid(bez: &kurbo::BezPath) -> kurbo::Point {
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
        history.execute_discrete(cmd, &mut doc);
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


/// Subdivide every segment of a BezPath once (insert midpoints).
pub(crate) fn subdivide_bez(bez: &kurbo::BezPath) -> kurbo::BezPath {
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

pub(crate) fn mid(a: kurbo::Point, b: kurbo::Point) -> kurbo::Point {
    kurbo::Point::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0)
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
        history.execute_discrete(cmd, &mut doc);
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
        history.execute_discrete(
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

pub(crate) fn lerp_point(a: kurbo::Point, b: kurbo::Point, t: f64) -> kurbo::Point {
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
    history.execute_discrete(
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
            history.execute_discrete(
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
            history.execute_discrete(
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
    history.execute_discrete(
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
    history.execute_discrete(
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
    history.execute_discrete(
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
            history.execute_discrete(
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
            // Map the canvas point into the node's local space so moved/scaled/
            // rotated nodes hit-test correctly.
            let local = node.transform.to_kurbo().inverse() * pt;
            match &node.kind {
                SceneNodeKind::Path(pn) => {
                    let bez = pn.path_data.to_bez_path();
                    if bez.winding(local) != 0 {
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
                SceneNodeKind::Raster(rn) if !rn.is_adjustment_layer() => {
                    if local.x >= 0.0
                        && local.y >= 0.0
                        && local.x < rn.image.width as f64
                        && local.y < rn.image.height as f64
                    {
                        let rgba = rn.image.pixel(local.x as u32, local.y as u32);
                        let cov = rn
                            .mask
                            .as_ref()
                            .map(|m| m.coverage(local.x as u32, local.y as u32))
                            .unwrap_or(1.0);
                        // Skip transparent/masked pixels so sampling falls through.
                        if (rgba[3] as f32 / 255.0) * cov * node.opacity > 0.0 {
                            let hex = format!("#{:02X}{:02X}{:02X}", rgba[0], rgba[1], rgba[2]);
                            return ToolResult::text(format!(
                                "Sampled '{}': color={} (raster pixel)",
                                node.name, hex
                            ))
                            .with_data(serde_json::json!({
                                "node_id": nid,
                                "node_name": node.name,
                                "fill_color": hex,
                                "stroke_color": null,
                                "opacity": node.opacity,
                            }));
                        }
                    }
                }
                _ => {}
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

        history.execute_discrete(
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
        history.execute_discrete(
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
        history.execute_discrete(
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
    history.execute_discrete(
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
        history.execute_discrete(
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
        history.execute_discrete(
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
        history.execute_discrete(
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
        history.execute_discrete(
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

        history.execute_discrete(
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

        history.execute_discrete(
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
    history.execute_discrete(
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
    history.execute_discrete(
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
    history.execute_discrete(
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
pub(crate) fn reverse_bez(els: &[kurbo::PathEl]) -> Vec<kurbo::PathEl> {
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
    history.execute_discrete(
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
    history.execute_discrete(
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
        history.execute_discrete(
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
        history.execute_discrete(
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
            // raster: no vector fill to recolor — offset + opacity only
            SceneNodeKind::Raster(_) => {}
        }

        history.execute_discrete(
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
        history.execute_discrete(cmd, &mut doc);
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
        history.execute_discrete(cmd, &mut doc);
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
pub(crate) fn apply_warp_envelope(
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
        history.execute_discrete(cmd, &mut doc);
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
pub(crate) fn apply_scallop(bez: &kurbo::BezPath, depth: f64, count: usize) -> kurbo::BezPath {
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
pub(crate) fn scallop_segment(
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
        history.execute_discrete(cmd, &mut doc);
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
pub(crate) fn apply_crystallize(bez: &kurbo::BezPath, size: f64, count: usize) -> kurbo::BezPath {
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
pub(crate) fn crystallize_segment(
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

pub(crate) fn solid_fill_of(fill: &photonic_core::style::Fill) -> Option<photonic_core::color::Color> {
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
                // raster: not subject to stray/unpainted/empty-text cleanup
                SceneNodeKind::Raster(_) => {}
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
    history.execute_discrete(cmd, &mut doc);

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
    history.execute_discrete(cmd, &mut doc);

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
                    FillKind::Pattern(p) => {
                        p.tile.map_rgb(|[r, g, b]| [1.0 - r, 1.0 - g, 1.0 - b]);
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
    history.execute_discrete(Command::Batch(commands), &mut doc);
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
                FillKind::Pattern(p) => {
                    p.tile.map_pixels(|[r, g, b, a]| {
                        let c = shift_color(photonic_core::Color {
                            r: r as f32 / 255.0,
                            g: g as f32 / 255.0,
                            b: b as f32 / 255.0,
                            a: a as f32 / 255.0,
                        });
                        [
                            (c.r * 255.0).round().clamp(0.0, 255.0) as u8,
                            (c.g * 255.0).round().clamp(0.0, 255.0) as u8,
                            (c.b * 255.0).round().clamp(0.0, 255.0) as u8,
                            (c.a * 255.0).round().clamp(0.0, 255.0) as u8,
                        ]
                    });
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
    history.execute_discrete(Command::Batch(commands), &mut doc);
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
                    FillKind::Pattern(p) => {
                        p.tile.map_rgb(|rgb| {
                            let l = photonic_core::raster::image::luma(rgb);
                            [l, l, l]
                        });
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
    history.execute_discrete(Command::Batch(commands), &mut doc);
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
        history.execute_discrete(cmd, &mut doc);
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
        history.execute_discrete(cmd, &mut doc);
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
    use photonic_core::ops::stroke_outline::{
        outline_stroke_with_scale as do_outline, transform_uniform_scale,
    };
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

        // Non-scaling stroke: divide width by the object's transform scale so
        // the outline matches the drawn stroke (the outline keeps this transform).
        let obj_scale = transform_uniform_scale(&node.transform.matrix);
        let outline_data = match do_outline(&pn.path_data, &pn.stroke, obj_scale) {
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
    history.execute_discrete(batch, &mut doc);

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
    history.execute_discrete(batch, &mut doc);

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
    history.execute_discrete(Command::Batch(commands), &mut doc);
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
        history.execute_discrete(
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

        history.execute_discrete(
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


// ─── pathfinder_minus_back ────────────────────────────────────────────────────


// ─── pathfinder_minus_front ───────────────────────────────────────────────────


// ─── pathfinder_trim ──────────────────────────────────────────────────────────


// ─── pathfinder_outline ───────────────────────────────────────────────────────


// ─── pathfinder_divide ────────────────────────────────────────────────────────


// ─── divide_objects_below ─────────────────────────────────────────────────────


// ─── pathfinder_merge ────────────────────────────────────────────────────────


// ─── select_same ─────────────────────────────────────────────────────────────


/// Extract the solid fill color from a node, or None if it has no solid fill.
pub(crate) fn solid_fill_color(node: &SceneNode) -> Option<photonic_core::color::Color> {
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
pub(crate) fn color_distance(a: photonic_core::color::Color, b: photonic_core::color::Color) -> f32 {
    let dr = a.r - b.r;
    let dg = a.g - b.g;
    let db = a.b - b.b;
    let da = a.a - b.a;
    (dr * dr + dg * dg + db * db + da * da).sqrt()
}


/// Returns the horizontal center of a path node's bounding box (local space).
pub(crate) fn path_center_x(node: &SceneNode) -> f32 {
    if let SceneNodeKind::Path(p) = &node.kind {
        if let Some(bb) = p.path_data.bounding_box() {
            return ((bb.x0 + bb.x1) / 2.0) as f32;
        }
    }
    0.0
}

/// Returns the vertical center of a path node's bounding box (local space).
pub(crate) fn path_center_y(node: &SceneNode) -> f32 {
    if let SceneNodeKind::Path(p) = &node.kind {
        if let Some(bb) = p.path_data.bounding_box() {
            return ((bb.y0 + bb.y1) / 2.0) as f32;
        }
    }
    0.0
}

// ─── make_compound_path ───────────────────────────────────────────────────────


// ─── release_compound_path ────────────────────────────────────────────────────





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
    history.execute_discrete(Command::Batch(commands), &mut doc);

    ToolResult::text(format!(
        "Recolored {} node(s) to nearest palette colors",
        recolored
    ))
    .with_data(serde_json::json!({ "recolored_count": recolored }))
}

// ─── Guide tools ─────────────────────────────────────────────────────────────





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
    history.execute_discrete(
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


/// Compute the world-space axis-aligned bounding box of a node using its
/// transform and path bounding box (or a text fallback of 1×1 at origin).
pub(crate) fn node_world_aabb(node: &SceneNode) -> Option<(f64, f64, f64, f64)> {
    let (lx0, ly0, lx1, ly1) = match &node.kind {
        SceneNodeKind::Path(pn) => {
            let r = pn.path_data.bounding_box()?;
            (r.x0, r.y0, r.x1, r.y1)
        }
        SceneNodeKind::Text(_) => (0.0, 0.0, 1.0, 1.0),
        SceneNodeKind::Group(_) => (0.0, 0.0, 1.0, 1.0),
        // raster: no path geometry — fallback local AABB
        SceneNodeKind::Raster(_) => (0.0, 0.0, 1.0, 1.0),
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
    history.execute_discrete(cmd, &mut doc);

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


// ─── select_by_kind ──────────────────────────────────────────────────────────


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
        history.execute_discrete(cmd, &mut doc);
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
pub(crate) fn point_in_polygon(px: f64, py: f64, poly: &[[f64; 2]]) -> bool {
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
    history.execute_discrete(batch, &mut doc);

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
        history.execute_discrete(cmd, &mut doc);
    }

    ToolResult::text(format!(
        "Noise-deformed {} path node(s) (amplitude={:.1}, frequency={:.4}, axis={}, seed={:.2}). Skipped: {}.",
        modified, amplitude, frequency, axis, seed, skipped
    ))
}

// ─── mirror_copy ──────────────────────────────────────────────────────────────


// ─── pin_object_guides ────────────────────────────────────────────────────────


// ─── reverse_node_order ───────────────────────────────────────────────────────


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
    history.execute_discrete(
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
        history.execute_discrete(
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

    history.execute_discrete(
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
    history.execute_discrete(batch, &mut doc);

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
    history.execute_discrete(batch, &mut doc);

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
    history.execute_discrete(
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
    history.execute_discrete(
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
    history.execute_discrete(
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
    history.execute_discrete(
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
    history.execute_discrete(
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
    history.execute_discrete(
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
    history.execute_discrete(Command::Batch(commands), &mut doc);
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
    history.execute_discrete(
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
    history.execute_discrete(
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
    history.execute_discrete(
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
    history.execute_discrete(
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
            // raster: no vector fill/stroke to bake
            SceneNodeKind::Raster(_) => {}
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
    history.execute_discrete(Command::Batch(commands), &mut doc);
    drop(history);

    ToolResult::text(format!("Flattened transparency on {} node(s).", processed))
        .with_data(serde_json::json!({ "processed": processed }))
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
    history.execute_discrete(
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
    history.execute_discrete(
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

/// Set advanced node-level character metrics: baseline shift and super/subscript.
pub async fn set_character_metrics(state: &AppState, args: SetCharacterMetricsArgs) -> ToolResult {
    use photonic_core::node::ScriptPosition;
    tracing::debug!("tool: set_character_metrics");

    // Validate script_position up front so a bad value fails before mutating.
    let parsed_script = match args.script_position.as_deref() {
        Some(s) => match ScriptPosition::from_str_opt(s) {
            Some(sp) => Some(sp),
            None => {
                return ToolResult::error(format!(
                    "Unknown script_position '{}'. Valid values: normal, superscript, subscript.",
                    s
                ))
            }
        },
        None => None,
    };

    if args.baseline_shift.is_none() && parsed_script.is_none() {
        return ToolResult::error(
            "Nothing to change: provide baseline_shift and/or script_position.".to_string(),
        );
    }

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
    let (baseline_shift, script_position) = if let SceneNodeKind::Text(ref mut tn) = new_node.kind {
        if let Some(bs) = args.baseline_shift {
            tn.baseline_shift = bs;
        }
        if let Some(sp) = parsed_script {
            tn.script_position = sp;
        }
        (tn.baseline_shift, tn.script_position)
    } else {
        (0.0, ScriptPosition::Normal)
    };

    let mut history = state.history.lock().await;
    history.execute_discrete(
        Command::UpdateNode {
            old: node,
            new: new_node,
        },
        &mut doc,
    );

    ToolResult::text(format!(
        "Character metrics on '{}' set: baseline_shift={}, script_position={}.",
        args.node_id,
        baseline_shift,
        script_position.as_str()
    ))
    .with_data(serde_json::json!({
        "node_id": node_id.to_string(),
        "baseline_shift": baseline_shift,
        "script_position": script_position.as_str(),
    }))
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
    history.execute_discrete(
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
    history.execute_discrete(
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
    history.execute_discrete(
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
    history.execute_discrete(
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
    history.execute_discrete(
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
    history.execute_discrete(
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
    history.execute_discrete(cmd, &mut doc);

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
    history.execute_discrete(
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
    history.execute_discrete(
        Command::UpdateNode {
            old: node,
            new: new_node,
        },
        &mut doc,
    );

    ToolResult::text(format!("Symbol overrides cleared on '{}'.", args.node_id))
        .with_data(serde_json::json!({ "node_id": node_id.to_string() }))
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
    history.execute_discrete(batch, &mut doc);
    ToolResult::text(format!(
        "Copied appearance from '{}' to {} node(s).",
        args.source_id, updated
    ))
    .with_data(serde_json::json!({ "updated": updated }))
}

#[cfg(test)]
mod create_shape_color_tests {
    use super::*;
    use crate::server::{AppState, McpServerConfig};
    use photonic_core::style::FillKind;
    use photonic_core::{AuditLog, Document};
    use serde_json::json;
    use std::sync::{Arc, Mutex as StdMutex};
    use tokio::sync::Mutex;

    fn test_state() -> AppState {
        let (tx, _rx) = std::sync::mpsc::channel();
        AppState {
            document: Arc::new(Mutex::new(Document::new("t", 100.0, 100.0))),
            history: Arc::new(Mutex::new(photonic_core::history::CommandHistory::new(100))),
            capture_tx: Arc::new(StdMutex::new(tx)),
            config: McpServerConfig::default(),
            audit_log: Arc::new(StdMutex::new(AuditLog::new())),
            clipboard_ring: Arc::new(crate::handlers::clipboard::new_clipboard_ring()),
        }
    }

    async fn only_fill(state: &AppState) -> photonic_core::style::Fill {
        let doc = state.document.lock().await;
        let node = doc
            .nodes
            .values()
            .find(|n| matches!(n.kind, SceneNodeKind::Path(_)))
            .expect("a path node");
        match &node.kind {
            SceneNodeKind::Path(p) => p.fill.clone(),
            _ => unreachable!(),
        }
    }

    #[tokio::test]
    async fn color_shorthand_sets_solid_fill() {
        let state = test_state();
        let args = serde_json::from_value(json!({
            "shape_type": "rectangle", "x": 0.0, "y": 0.0, "width": 10.0, "height": 10.0,
            "color": "#2277ff"
        }))
        .unwrap();
        create_shape(&state, args).await;

        let fill = only_fill(&state).await;
        match fill.kind {
            FillKind::Solid(c) => {
                assert!((c.r - 0.133).abs() < 0.02, "r={}", c.r);
                assert!((c.g - 0.467).abs() < 0.02, "g={}", c.g);
                assert!((c.b - 1.0).abs() < 0.02, "b={}", c.b);
            }
            other => panic!("expected solid fill, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn explicit_fill_wins_over_color() {
        let state = test_state();
        let args = serde_json::from_value(json!({
            "shape_type": "rectangle", "x": 0.0, "y": 0.0, "width": 10.0, "height": 10.0,
            "color": "#2277ff",
            "fill": { "type": "solid", "color": "#ff0000" }
        }))
        .unwrap();
        create_shape(&state, args).await;

        let fill = only_fill(&state).await;
        match fill.kind {
            FillKind::Solid(c) => {
                assert!(
                    c.r > 0.9 && c.g < 0.1 && c.b < 0.1,
                    "expected red, got {c:?}"
                );
            }
            other => panic!("expected solid fill, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn invalid_color_is_an_error() {
        let state = test_state();
        let args = serde_json::from_value(json!({
            "shape_type": "rectangle", "x": 0.0, "y": 0.0, "width": 10.0, "height": 10.0,
            "color": "not-a-color"
        }))
        .unwrap();
        let result = create_shape(&state, args).await;
        assert_eq!(result.is_error, Some(true), "invalid color should error");
    }
}

#[cfg(test)]
mod round_corners_tests {
    use super::*;

    fn end_vertices(bez: &kurbo::BezPath) -> Vec<kurbo::Point> {
        bez.elements()
            .iter()
            .filter_map(|el| match el {
                kurbo::PathEl::MoveTo(p) => Some(*p),
                kurbo::PathEl::LineTo(p) => Some(*p),
                kurbo::PathEl::QuadTo(_, p) => Some(*p),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn open_run_corner_rounds_past_half_edge() {
        // Open 3-vertex polyline: (0,0) → (10,0) → (10,10). The interior corner at
        // (10,0) borders two endpoints, so it retreats almost the full edge rather
        // than being capped at L/2 = 5.
        let mut bez = kurbo::BezPath::new();
        bez.move_to((0.0, 0.0));
        bez.line_to((10.0, 0.0));
        bez.line_to((10.0, 10.0));
        let out = apply_round_corners(&bez, 8.0);

        // fillet_start is the LineTo preceding the corner's QuadTo (control = corner).
        let els: Vec<kurbo::PathEl> = out.elements().to_vec();
        let mut fillet_start = None;
        for (i, el) in els.iter().enumerate() {
            if let kurbo::PathEl::QuadTo(ctrl, _) = el {
                assert!(
                    (ctrl.x - 10.0).abs() < 1e-6 && ctrl.y.abs() < 1e-6,
                    "quad control should be the corner (10,0), got {ctrl:?}"
                );
                if let kurbo::PathEl::LineTo(p) = els[i - 1] {
                    fillet_start = Some(p);
                }
            }
        }
        let p = fillet_start.expect("expected a rounded corner preceded by a LineTo");
        // From (10,0) toward (0,0) by r=8 ⇒ x=2, past the midpoint x=5. The old
        // unconditional L/2 clamp would have stopped at x=5.
        assert!(
            p.x < 5.0 - 1e-6,
            "fillet_start x {} should be past the half-edge (5.0)",
            p.x
        );
        assert!(
            (p.x - 2.0).abs() < 1e-6,
            "fillet_start x {} expected 2.0",
            p.x
        );
    }

    #[test]
    fn closed_square_splits_edges_fifty_fifty() {
        // Closed 10×10 square with an oversized radius. Every neighbour is rounded,
        // so each corner stays clamped to L/2 = 5 and its fillet points land on the
        // edge midpoints — adjacent fillets meet but never overlap.
        let mut bez = kurbo::BezPath::new();
        bez.move_to((0.0, 0.0));
        bez.line_to((10.0, 0.0));
        bez.line_to((10.0, 10.0));
        bez.line_to((0.0, 10.0));
        bez.close_path();
        let out = apply_round_corners(&bez, 100.0);

        for p in end_vertices(&out) {
            if p.y.abs() < 1e-6 || (p.y - 10.0).abs() < 1e-6 {
                assert!(
                    (p.x - 5.0).abs() < 1e-6,
                    "fillet point {p:?} on a horizontal edge crosses the midpoint x=5"
                );
            }
            if p.x.abs() < 1e-6 || (p.x - 10.0).abs() < 1e-6 {
                assert!(
                    (p.y - 5.0).abs() < 1e-6,
                    "fillet point {p:?} on a vertical edge crosses the midpoint y=5"
                );
            }
        }
    }
}
