//! Layer & group operations (group, collect-in-layer, release-to-layers,
//! merge-layers) extracted from app::mod. Methods on PhotonicApp.
#![allow(clippy::too_many_arguments)]
use super::*;

impl PhotonicApp {
    /// Group the currently selected nodes. Requires 2+ nodes in selection.
    pub(crate) fn do_group_selected(
        &mut self,
        doc: &mut Document,
        history: &mut CommandHistory,
        doc_modified: &mut bool,
    ) {
        if doc.selection.count() < 2 {
            return;
        }
        let sel_ids: Vec<_> = doc.selection.ids().copied().collect();
        if let Some((layer_id, mut indexed)) = doc.nodes_layer_and_indices(&sel_ids) {
            indexed.sort_by_key(|(_, idx)| *idx);
            let children: Vec<_> = indexed.iter().map(|(id, _)| *id).collect();
            let insert_index = indexed[0].1;
            let group_kind = SceneNodeKind::Group(GroupNode {
                children: children.clone(),
                clip_children: false,
                clip_node_id: None,
                blend_spine_id: None,
            });
            let group = SceneNode::new("Group", layer_id, group_kind);
            let group_id = group.id;
            let cmd = Command::GroupNodes {
                group,
                layer_id,
                insert_index,
                children,
            };
            history.execute(cmd, doc);
            self.selected_id = Some(group_id);
            doc.selection = Selection::single(group_id);
            *doc_modified = true;
        }
    }

    pub(crate) fn do_collect_in_new_layer(
        &mut self,
        node_ids: Vec<NodeId>,
        doc: &mut Document,
        history: &mut CommandHistory,
        doc_modified: &mut bool,
    ) {
        // Fall back to current selection when no explicit ids given
        let raw_ids: Vec<NodeId> = if node_ids.is_empty() {
            doc.selection.ids().copied().collect()
        } else {
            node_ids
        };
        if raw_ids.is_empty() {
            return;
        }

        // Resolve group children to their top-level ancestors (deduplicated)
        let mut resolved: Vec<NodeId> = Vec::new();
        for id in raw_ids {
            if let Some(tid) = doc.top_level_ancestor(id) {
                if !resolved.contains(&tid) {
                    resolved.push(tid);
                }
            }
        }
        if resolved.is_empty() {
            return;
        }

        let new_layer = Layer::new("Collected Layer");
        let new_layer_id = new_layer.id;

        let mut cmds = vec![Command::AddLayer { layer: new_layer }];
        for (i, nid) in resolved.iter().enumerate() {
            if let Some((old_layer_id, old_index)) = doc.node_layer_and_index(nid) {
                cmds.push(Command::MoveNodeToLayer {
                    node_id: *nid,
                    old_layer_id,
                    new_layer_id,
                    old_index,
                    new_index: i,
                });
            }
        }
        history.execute(Command::Batch(cmds), doc);
        *doc_modified = true;
    }

    pub(crate) fn do_release_to_layers(
        &mut self,
        node_ids: Vec<NodeId>,
        doc: &mut Document,
        history: &mut CommandHistory,
        doc_modified: &mut bool,
    ) {
        let raw_ids: Vec<NodeId> = if node_ids.is_empty() {
            doc.selection.ids().copied().collect()
        } else {
            node_ids
        };
        if raw_ids.is_empty() {
            return;
        }

        // Resolve group children to top-level ancestors (deduplicated).
        let mut resolved: Vec<NodeId> = Vec::new();
        for id in raw_ids {
            if let Some(tid) = doc.top_level_ancestor(id) {
                if !resolved.contains(&tid) {
                    resolved.push(tid);
                }
            }
        }
        if resolved.is_empty() {
            return;
        }

        // One new layer per node.
        let mut cmds: Vec<Command> = Vec::new();
        for (seq, nid) in resolved.iter().enumerate() {
            if let Some((old_layer_id, old_index)) = doc.node_layer_and_index(nid) {
                let new_layer = Layer::new(&format!("Layer {}", seq + 1));
                let new_layer_id = new_layer.id;
                cmds.push(Command::AddLayer { layer: new_layer });
                cmds.push(Command::MoveNodeToLayer {
                    node_id: *nid,
                    old_layer_id,
                    new_layer_id,
                    old_index,
                    new_index: 0,
                });
            }
        }
        if !cmds.is_empty() {
            history.execute(Command::Batch(cmds), doc);
            *doc_modified = true;
        }
    }

    pub(crate) fn do_merge_layers(
        &mut self,
        layer_ids: Vec<photonic_core::layer::LayerId>,
        doc: &mut Document,
        history: &mut CommandHistory,
        doc_modified: &mut bool,
    ) {
        if layer_ids.len() < 2 {
            return;
        }
        // Validate
        for lid in &layer_ids {
            if !doc.layers.contains_key(lid) {
                return;
            }
        }

        // Target = first of the selected layers in document order (bottom-most).
        let target_id = match doc.layer_order.iter().find(|id| layer_ids.contains(id)) {
            Some(&id) => id,
            None => return,
        };

        let source_ids: Vec<_> = layer_ids
            .iter()
            .filter(|&&id| id != target_id)
            .copied()
            .collect();

        let mut cmds: Vec<Command> = Vec::new();

        // Process sources in document order.
        let ordered_sources: Vec<_> = doc
            .layer_order
            .iter()
            .filter(|id| source_ids.contains(id))
            .copied()
            .collect();

        let mut new_index_offset = doc.layers[&target_id].node_ids.len();

        for src_id in &ordered_sources {
            let src_layer = doc.layers[src_id].clone();
            for node_id in src_layer.node_ids.clone() {
                if let Some((old_layer_id, old_index)) = doc.node_layer_and_index(&node_id) {
                    cmds.push(Command::MoveNodeToLayer {
                        node_id,
                        old_layer_id,
                        new_layer_id: target_id,
                        old_index,
                        new_index: new_index_offset,
                    });
                    new_index_offset += 1;
                }
            }
            cmds.push(Command::RemoveLayerFull { layer: src_layer });
        }

        if !cmds.is_empty() {
            history.execute(Command::Batch(cmds), doc);
            *doc_modified = true;
        }
    }
}
