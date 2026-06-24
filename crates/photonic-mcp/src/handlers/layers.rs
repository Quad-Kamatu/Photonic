use crate::protocol::{
    CollectInNewLayerArgs, CreateLayerArgs, FlattenArtworkArgs, MergeLayersArgs,
    ReleaseToLayersArgs, ToolResult, UpdateLayerArgs,
};
use crate::server::AppState;
use photonic_core::{
    history::Command,
    layer::{Layer, LayerId},
    node::NodeId,
};

pub async fn create_layer(state: &AppState, args: CreateLayerArgs) -> ToolResult {
    let layer = Layer::new(&args.name);
    let layer_id = layer.id;

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    history.execute(Command::AddLayer { layer }, &mut doc);

    // Optionally reposition the layer.
    // position 0 = top of stack (highest z-order); position 1 = just below top; etc.
    if let Some(pos) = args.position {
        let current_pos = doc.layer_order.iter().position(|id| id == &layer_id);
        if let Some(cur) = current_pos {
            let old_order = doc.layer_order.clone();
            doc.layer_order.remove(cur);
            // After removal, len() is the new length. Inserting at len() = top.
            let insert_at = doc.layer_order.len().saturating_sub(pos);
            doc.layer_order.insert(insert_at, layer_id);
            let cmd = Command::ReorderLayers {
                old_order,
                new_order: doc.layer_order.clone(),
            };
            history.execute(cmd, &mut doc);
        }
    }

    ToolResult::text(format!("Created layer '{}' (id: {})", args.name, layer_id))
        .with_data(serde_json::json!({ "layer_id": layer_id }))
}

pub async fn collect_in_new_layer(state: &AppState, args: CollectInNewLayerArgs) -> ToolResult {
    if args.node_ids.is_empty() {
        return ToolResult::error("node_ids must not be empty");
    }

    let layer_name = args.name.unwrap_or_else(|| "Collected Layer".to_string());

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    // Resolve every node_id to its top-level ancestor (deduplicated, order preserved)
    let mut resolved: Vec<NodeId> = Vec::new();
    for &raw_id in &args.node_ids {
        match doc.top_level_ancestor(raw_id) {
            Some(tid) if !resolved.contains(&tid) => resolved.push(tid),
            Some(_) => {} // duplicate after resolution — skip
            None => return ToolResult::error(format!("Node {} not found", raw_id)),
        }
    }

    // Collect (old_layer_id, old_index) for each resolved node
    let mut moves: Vec<(NodeId, photonic_core::layer::LayerId, usize)> = Vec::new();
    for &nid in &resolved {
        match doc.node_layer_and_index(&nid) {
            Some((old_layer_id, old_index)) => moves.push((nid, old_layer_id, old_index)),
            None => return ToolResult::error(format!("Node {} has no layer position", nid)),
        }
    }

    // Create the new layer
    let new_layer = Layer::new(&layer_name);
    let new_layer_id = new_layer.id;

    // Batch: AddLayer + one MoveNodeToLayer per node
    let mut cmds = vec![Command::AddLayer { layer: new_layer }];
    for (i, (node_id, old_layer_id, old_index)) in moves.iter().enumerate() {
        cmds.push(Command::MoveNodeToLayer {
            node_id: *node_id,
            old_layer_id: *old_layer_id,
            new_layer_id,
            old_index: *old_index,
            new_index: i,
        });
    }
    history.execute(Command::Batch(cmds), &mut doc);

    // Optionally reposition the new layer.
    // position 0 = top of stack (highest z-order); position 1 = just below top; etc.
    if let Some(pos) = args.position {
        if let Some(cur) = doc.layer_order.iter().position(|id| id == &new_layer_id) {
            let old_order = doc.layer_order.clone();
            doc.layer_order.remove(cur);
            let insert_at = doc.layer_order.len().saturating_sub(pos);
            doc.layer_order.insert(insert_at, new_layer_id);
            history.execute(
                Command::ReorderLayers {
                    old_order,
                    new_order: doc.layer_order.clone(),
                },
                &mut doc,
            );
        }
    }

    ToolResult::text(format!(
        "Moved {} node(s) into new layer '{}' (id: {})",
        moves.len(),
        layer_name,
        new_layer_id
    ))
    .with_data(serde_json::json!({
        "layer_id": new_layer_id,
        "moved_count": moves.len()
    }))
}

// ─── release_to_layers ───────────────────────────────────────────────────────

/// Move each node into its own newly created layer (inverse of collect_in_new_layer).
pub async fn release_to_layers(state: &AppState, args: ReleaseToLayersArgs) -> ToolResult {
    if args.node_ids.is_empty() {
        return ToolResult::error("node_ids must not be empty");
    }

    let prefix = args.name_prefix.unwrap_or_else(|| "Layer".to_string());

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    // Resolve every node_id to its top-level ancestor (deduplicated, order preserved).
    let mut resolved: Vec<NodeId> = Vec::new();
    for &raw_id in &args.node_ids {
        match doc.top_level_ancestor(raw_id) {
            Some(tid) if !resolved.contains(&tid) => resolved.push(tid),
            Some(_) => {}
            None => return ToolResult::error(format!("Node {} not found", raw_id)),
        }
    }

    // Gather current layer positions before modifying anything.
    let mut moves: Vec<(NodeId, photonic_core::layer::LayerId, usize)> = Vec::new();
    for &nid in &resolved {
        match doc.node_layer_and_index(&nid) {
            Some((old_layer_id, old_index)) => moves.push((nid, old_layer_id, old_index)),
            None => return ToolResult::error(format!("Node {} has no layer position", nid)),
        }
    }

    // Build one batch: for each node, AddLayer + MoveNodeToLayer.
    let mut cmds: Vec<Command> = Vec::new();
    let mut created_layer_ids: Vec<photonic_core::layer::LayerId> = Vec::new();

    for (seq, (node_id, old_layer_id, old_index)) in moves.iter().enumerate() {
        let layer_name = format!("{} {}", prefix, seq + 1);
        let new_layer = Layer::new(&layer_name);
        let new_layer_id = new_layer.id;
        created_layer_ids.push(new_layer_id);

        cmds.push(Command::AddLayer { layer: new_layer });
        cmds.push(Command::MoveNodeToLayer {
            node_id: *node_id,
            old_layer_id: *old_layer_id,
            new_layer_id,
            old_index: *old_index,
            new_index: 0,
        });
    }

    history.execute(Command::Batch(cmds), &mut doc);
    history.schedule_mcp_checkpoint(format!("Release {} node(s) to layers", resolved.len()));

    ToolResult::text(format!(
        "Released {} node(s) to {} new layer(s).",
        resolved.len(),
        created_layer_ids.len()
    ))
    .with_data(serde_json::json!({
        "layer_ids": created_layer_ids,
        "node_count": resolved.len(),
    }))
}

// ─── merge_layers ─────────────────────────────────────────────────────────────

/// Merge two or more layers into one. All nodes from source layers are moved
/// into the target layer (the first layer among the selected set in document
/// order). Empty source layers are then removed. Single undoable step.
pub async fn merge_layers(state: &AppState, args: MergeLayersArgs) -> ToolResult {
    if args.layer_ids.len() < 2 {
        return ToolResult::error("merge_layers requires at least 2 layer_ids");
    }

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    // Validate all layer IDs
    for lid in &args.layer_ids {
        if !doc.layers.contains_key(lid) {
            return ToolResult::error(format!("Layer {} not found", lid));
        }
    }

    // Determine target layer: the first of the selected layers in document order.
    let target_id: LayerId = doc
        .layer_order
        .iter()
        .find(|id| args.layer_ids.contains(id))
        .copied()
        .expect("validated above");

    let source_ids: Vec<LayerId> = args
        .layer_ids
        .iter()
        .filter(|&&id| id != target_id)
        .copied()
        .collect();

    // Rename target if requested, capturing old name for undo via UpdateNode pattern.
    // We handle rename as a separate prior step so the batch stays clean.
    if let Some(ref new_name) = args.target_name {
        if let Some(layer) = doc.layers.get_mut(&target_id) {
            layer.name = new_name.clone();
        }
    }
    let target_name = doc.layers[&target_id].name.clone();

    // Build a batch:
    // - For each source layer, in document order: MoveNodeToLayer for every node, then RemoveLayerFull.
    let mut cmds: Vec<Command> = Vec::new();
    let mut total_moved = 0usize;

    // Process source layers in document order to keep z-order stable.
    let ordered_sources: Vec<LayerId> = doc
        .layer_order
        .iter()
        .filter(|id| source_ids.contains(id))
        .copied()
        .collect();

    // Track the next insertion index in the target layer (starts after existing nodes).
    let mut next_new_index = doc.layers[&target_id].node_ids.len();

    for src_id in &ordered_sources {
        // Snapshot layer data before any modification (needed for RemoveLayerFull).
        let src_layer = doc.layers[src_id].clone();
        let node_ids_in_src: Vec<NodeId> = src_layer.node_ids.clone();

        // Move each node to the target layer (appended in order).
        for node_id in node_ids_in_src {
            if let Some((old_layer_id, old_index)) = doc.node_layer_and_index(&node_id) {
                cmds.push(Command::MoveNodeToLayer {
                    node_id,
                    old_layer_id,
                    new_layer_id: target_id,
                    old_index,
                    new_index: next_new_index,
                });
                next_new_index += 1;
                total_moved += 1;
            }
        }

        // Remove the now-empty source layer (store full struct for safe undo).
        cmds.push(Command::RemoveLayerFull { layer: src_layer });
    }

    if cmds.is_empty() {
        return ToolResult::text(format!(
            "No nodes to merge; target layer '{}' unchanged.",
            target_name
        ));
    }

    history.execute(Command::Batch(cmds), &mut doc);
    history.schedule_mcp_checkpoint(format!(
        "Merge {} layers into '{}'",
        source_ids.len() + 1,
        target_name
    ));

    ToolResult::text(format!(
        "Merged {} layer(s) into '{}' ({} node(s) moved, {} layer(s) removed).",
        source_ids.len() + 1,
        target_name,
        total_moved,
        source_ids.len()
    ))
    .with_data(serde_json::json!({
        "target_layer_id": target_id,
        "removed_layer_count": source_ids.len(),
        "moved_node_count": total_moved,
    }))
}

// ─── flatten_artwork ──────────────────────────────────────────────────────────

/// Flatten all layers into one. Equivalent to merge_layers with all layer IDs.
/// The bottom-most layer in document order survives; all others are removed.
pub async fn flatten_artwork(state: &AppState, args: FlattenArtworkArgs) -> ToolResult {
    let mut doc = state.document.lock().await;

    if doc.layer_order.len() < 2 {
        return ToolResult::text(
            "Document already has a single layer; nothing to flatten.".to_string(),
        );
    }

    let mut history = state.history.lock().await;

    let target_id = doc.layer_order[0]; // bottom-most
    let source_ids: Vec<LayerId> = doc.layer_order[1..].to_vec();

    if let Some(ref new_name) = args.target_name {
        if let Some(layer) = doc.layers.get_mut(&target_id) {
            layer.name = new_name.clone();
        }
    }
    let target_name = doc.layers[&target_id].name.clone();

    let mut cmds: Vec<Command> = Vec::new();
    let mut total_moved = 0usize;
    let mut next_new_index = doc.layers[&target_id].node_ids.len();

    for src_id in &source_ids {
        let src_layer = doc.layers[src_id].clone();
        for node_id in src_layer.node_ids.clone() {
            if let Some((old_layer_id, old_index)) = doc.node_layer_and_index(&node_id) {
                cmds.push(Command::MoveNodeToLayer {
                    node_id,
                    old_layer_id,
                    new_layer_id: target_id,
                    old_index,
                    new_index: next_new_index,
                });
                next_new_index += 1;
                total_moved += 1;
            }
        }
        cmds.push(Command::RemoveLayerFull { layer: src_layer });
    }

    if !cmds.is_empty() {
        history.execute(Command::Batch(cmds), &mut doc);
    }
    history.schedule_mcp_checkpoint(format!("Flatten artwork into '{}'", target_name));

    ToolResult::text(format!(
        "Flattened {} layer(s) into '{}' ({} node(s) merged).",
        source_ids.len() + 1,
        target_name,
        total_moved,
    ))
    .with_data(serde_json::json!({
        "target_layer_id": target_id,
        "removed_layer_count": source_ids.len(),
        "moved_node_count": total_moved,
    }))
}

pub async fn set_active_layer(state: &AppState, layer_id: uuid::Uuid) -> ToolResult {
    let mut doc = state.document.lock().await;

    if !doc.layers.contains_key(&layer_id) {
        return ToolResult::error(format!("Layer {} not found", layer_id));
    }

    let old_id = doc.active_layer_id;
    let cmd = Command::SetActiveLayer {
        old_id,
        new_id: Some(layer_id),
    };
    let mut history = state.history.lock().await;
    history.execute(cmd, &mut doc);

    ToolResult::text(format!("Active layer set to {}", layer_id))
}

// ─── update_layer ──────────────────────────────────────────────────────────────

/// Update mutable metadata on a layer: name, visibility, lock state, and color tag.
pub async fn update_layer(state: &AppState, args: UpdateLayerArgs) -> ToolResult {
    let mut doc = state.document.lock().await;

    let layer = match doc.layers.get(&args.layer_id) {
        Some(l) => l.clone(),
        None => return ToolResult::error(format!("Layer {} not found", args.layer_id)),
    };

    let new_name = args.name.clone().unwrap_or_else(|| layer.name.clone());
    let new_visible = args.visible.unwrap_or(layer.visible);
    let new_is_template = args.is_template.unwrap_or(layer.is_template);
    // Template layers are implicitly locked; respect explicit locked arg otherwise.
    let new_locked = args
        .locked
        .unwrap_or(if new_is_template { true } else { layer.locked });
    let new_color = match args.color {
        Some(c) => c, // explicit — use as provided (Some([..]) sets color, None clears it)
        None => layer.color, // omitted — keep existing
    };

    let cmd = photonic_core::history::Command::UpdateLayer {
        layer_id: args.layer_id,
        old_name: layer.name.clone(),
        new_name: new_name.clone(),
        old_visible: layer.visible,
        new_visible,
        old_locked: layer.locked,
        new_locked,
        old_color: layer.color,
        new_color,
        old_is_template: layer.is_template,
        new_is_template,
    };

    let mut history = state.history.lock().await;
    history.execute(cmd, &mut doc);
    history.schedule_mcp_checkpoint(format!("Update layer '{}'", new_name));

    ToolResult::text(format!(
        "Updated layer '{}' (id: {})",
        new_name, args.layer_id
    ))
    .with_data(serde_json::json!({
        "layer_id": args.layer_id,
        "name": new_name,
        "visible": new_visible,
        "locked": new_locked,
        "color": new_color,
        "is_template": new_is_template,
    }))
}
