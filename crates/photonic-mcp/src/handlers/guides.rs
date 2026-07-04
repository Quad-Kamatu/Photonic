use crate::protocol::*;
use crate::server::AppState;
use kurbo;
use photonic_core::{
    document::{Guide, GuideOrientation},
    history::Command,
    node::{GroupNode, NodeId, PathNode, SceneNode, SceneNodeKind},
    path::PathData,
    transform::Transform,
};

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
        history.execute_discrete(
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
    history.execute_discrete(
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
    history.execute_discrete(
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
    history.execute_discrete(
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
    history.execute_discrete(
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
    history.execute_discrete(
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
    history.execute_discrete(
        Command::SetGuides {
            old: old_guides,
            new: new_guides,
        },
        &mut doc,
    );

    ToolResult::text(format!("Cleared {} guide(s)", removed))
        .with_data(serde_json::json!({ "removed_count": removed }))
}
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
