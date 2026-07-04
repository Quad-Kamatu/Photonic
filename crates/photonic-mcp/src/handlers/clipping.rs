use crate::protocol::*;
use crate::server::AppState;
use kurbo;
use photonic_core::{
    history::Command,
    node::{NodeId, PathNode, SceneNode, SceneNodeKind},
    path::PathData,
};

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
    history.execute_discrete(
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
    history.execute_discrete(
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

    history.execute_discrete(Command::Batch(cmds), &mut doc);
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
        history.execute_discrete(
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

    history.execute_discrete(Command::Batch(cmds), &mut doc);
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
