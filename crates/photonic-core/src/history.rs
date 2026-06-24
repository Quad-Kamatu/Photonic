use crate::{
    document::{Document, Guide},
    layer::{Layer, LayerId},
    node::{NodeId, SceneNode},
};
use uuid::Uuid;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        layer::Layer,
        node::{PathNode, SceneNodeKind},
        path::PathData,
    };

    fn make_doc() -> Document {
        Document::new("test", 100.0, 100.0)
    }

    fn make_node(doc: &Document) -> SceneNode {
        let layer_id = doc.active_layer_id.unwrap();
        SceneNode::new(
            "rect",
            layer_id,
            SceneNodeKind::Path(PathNode::new(PathData::rect(0.0, 0.0, 10.0, 10.0))),
        )
    }

    // ── AddNode ──────────────────────────────────────────────────────────────

    #[test]
    fn execute_adds_node() {
        let mut doc = make_doc();
        let mut history = CommandHistory::new(200);
        let node = make_node(&doc);
        let node_id = node.id;
        let layer_id = node.layer_id;

        history.execute(
            Command::AddNode {
                node,
                layer_id: None,
            },
            &mut doc,
        );

        assert!(
            doc.nodes.contains_key(&node_id),
            "node missing from doc.nodes"
        );
        let layer = doc.layers.get(&layer_id).unwrap();
        assert!(
            layer.node_ids.contains(&node_id),
            "node missing from layer.node_ids"
        );
        assert_eq!(history.undo_depth(), 1);
        assert!(!history.can_redo());
    }

    #[test]
    fn undo_removes_node() {
        let mut doc = make_doc();
        let mut history = CommandHistory::new(200);
        let node = make_node(&doc);
        let node_id = node.id;
        let layer_id = node.layer_id;

        history.execute(
            Command::AddNode {
                node,
                layer_id: None,
            },
            &mut doc,
        );
        let undone = history.undo(&mut doc);

        assert!(undone);
        assert!(!doc.nodes.contains_key(&node_id));
        let layer = doc.layers.get(&layer_id).unwrap();
        assert!(!layer.node_ids.contains(&node_id));
        assert_eq!(history.undo_depth(), 0);
        assert!(history.can_redo());
    }

    #[test]
    fn redo_readds_node() {
        let mut doc = make_doc();
        let mut history = CommandHistory::new(200);
        let node = make_node(&doc);
        let node_id = node.id;

        history.execute(
            Command::AddNode {
                node,
                layer_id: None,
            },
            &mut doc,
        );
        history.undo(&mut doc);
        let redone = history.redo(&mut doc);

        assert!(redone);
        assert!(doc.nodes.contains_key(&node_id));
        assert_eq!(history.undo_depth(), 1);
        assert!(!history.can_redo());
    }

    #[test]
    fn redo_cleared_on_new_command() {
        let mut doc = make_doc();
        let mut history = CommandHistory::new(200);
        let node = make_node(&doc);
        history.execute(
            Command::AddNode {
                node,
                layer_id: None,
            },
            &mut doc,
        );
        history.undo(&mut doc);
        assert!(history.can_redo());

        // New command clears redo stack
        let node2 = make_node(&doc);
        history.execute(
            Command::AddNode {
                node: node2,
                layer_id: None,
            },
            &mut doc,
        );
        assert!(!history.can_redo());
    }

    // ── UpdateNode ────────────────────────────────────────────────────────────

    #[test]
    fn update_node_undo_redo() {
        let mut doc = make_doc();
        let mut history = CommandHistory::new(200);
        let node = make_node(&doc);
        let node_id = node.id;
        history.execute(
            Command::AddNode {
                node: node.clone(),
                layer_id: None,
            },
            &mut doc,
        );

        let mut updated = node.clone();
        updated.name = "circle".to_string();
        history.execute(
            Command::UpdateNode {
                old: node.clone(),
                new: updated.clone(),
            },
            &mut doc,
        );
        assert_eq!(doc.nodes[&node_id].name, "circle");

        history.undo(&mut doc);
        assert_eq!(doc.nodes[&node_id].name, "rect");

        history.redo(&mut doc);
        assert_eq!(doc.nodes[&node_id].name, "circle");
    }

    // ── AddLayer / RemoveLayer ────────────────────────────────────────────────

    #[test]
    fn add_layer_undo_redo() {
        let mut doc = make_doc();
        let mut history = CommandHistory::new(200);
        let layer = Layer::new("layer2");
        let layer_id = layer.id;
        let initial_len = doc.layer_order.len();

        history.execute(Command::AddLayer { layer }, &mut doc);
        assert_eq!(doc.layer_order.len(), initial_len + 1);
        assert!(doc.layers.contains_key(&layer_id));

        history.undo(&mut doc);
        assert_eq!(doc.layer_order.len(), initial_len);
        assert!(!doc.layers.contains_key(&layer_id));

        history.redo(&mut doc);
        assert!(doc.layers.contains_key(&layer_id));
    }

    // ── ReorderLayers ─────────────────────────────────────────────────────────

    #[test]
    fn reorder_layers_undo_redo() {
        let mut doc = make_doc();
        let mut history = CommandHistory::new(200);

        // Add a second layer so we can reorder
        let layer2 = Layer::new("layer2");
        let layer2_id = layer2.id;
        history.execute(Command::AddLayer { layer: layer2 }, &mut doc);

        let original_order = doc.layer_order.clone();
        let new_order: Vec<_> = original_order.iter().cloned().rev().collect();
        history.execute(
            Command::ReorderLayers {
                old_order: original_order.clone(),
                new_order: new_order.clone(),
            },
            &mut doc,
        );
        assert_eq!(doc.layer_order, new_order);

        history.undo(&mut doc);
        assert_eq!(doc.layer_order, original_order);

        history.redo(&mut doc);
        assert_eq!(doc.layer_order, new_order);
        let _ = layer2_id; // suppress unused warning
    }

    // ── SetActiveLayer ────────────────────────────────────────────────────────

    #[test]
    fn set_active_layer_undo_redo() {
        let mut doc = make_doc();
        let mut history = CommandHistory::new(200);
        let layer2 = Layer::new("layer2");
        let layer2_id = layer2.id;
        history.execute(Command::AddLayer { layer: layer2 }, &mut doc);

        let old_active = doc.active_layer_id;
        history.execute(
            Command::SetActiveLayer {
                old_id: old_active,
                new_id: Some(layer2_id),
            },
            &mut doc,
        );
        assert_eq!(doc.active_layer_id, Some(layer2_id));

        history.undo(&mut doc);
        assert_eq!(doc.active_layer_id, old_active);

        history.redo(&mut doc);
        assert_eq!(doc.active_layer_id, Some(layer2_id));
    }

    // ── ReorderNode ───────────────────────────────────────────────────────────

    #[test]
    fn reorder_node_undo_redo() {
        let mut doc = make_doc();
        let mut history = CommandHistory::new(200);
        let layer_id = doc.active_layer_id.unwrap();

        let node_a = make_node(&doc);
        let node_b = make_node(&doc);
        let node_a_id = node_a.id;
        let node_b_id = node_b.id;
        history.execute(
            Command::AddNode {
                node: node_a,
                layer_id: None,
            },
            &mut doc,
        );
        history.execute(
            Command::AddNode {
                node: node_b,
                layer_id: None,
            },
            &mut doc,
        );

        // Initial order: [a, b] (index 0 and 1)
        assert_eq!(doc.layers[&layer_id].node_ids[0], node_a_id);
        assert_eq!(doc.layers[&layer_id].node_ids[1], node_b_id);

        // Move node_a (index 0) to index 1
        history.execute(
            Command::ReorderNode {
                layer_id,
                node_id: node_a_id,
                old_index: 0,
                new_index: 1,
            },
            &mut doc,
        );
        assert_eq!(doc.layers[&layer_id].node_ids[0], node_b_id);
        assert_eq!(doc.layers[&layer_id].node_ids[1], node_a_id);

        history.undo(&mut doc);
        assert_eq!(doc.layers[&layer_id].node_ids[0], node_a_id);
        assert_eq!(doc.layers[&layer_id].node_ids[1], node_b_id);
    }

    // ── GroupNodes / UngroupNodes ─────────────────────────────────────────────

    #[test]
    fn group_nodes_undo() {
        use crate::node::GroupNode;

        let mut doc = make_doc();
        let mut history = CommandHistory::new(200);
        let layer_id = doc.active_layer_id.unwrap();

        let node_a = make_node(&doc);
        let node_b = make_node(&doc);
        let node_a_id = node_a.id;
        let node_b_id = node_b.id;
        history.execute(
            Command::AddNode {
                node: node_a,
                layer_id: None,
            },
            &mut doc,
        );
        history.execute(
            Command::AddNode {
                node: node_b,
                layer_id: None,
            },
            &mut doc,
        );

        let mut group = SceneNode::new("group", layer_id, SceneNodeKind::Group(GroupNode::new()));
        let group_id = group.id;
        if let SceneNodeKind::Group(ref mut g) = group.kind {
            g.children = vec![node_a_id, node_b_id];
        }

        history.execute(
            Command::GroupNodes {
                group,
                layer_id,
                insert_index: 0,
                children: vec![node_a_id, node_b_id],
            },
            &mut doc,
        );

        // After grouping: only the group node is in layer.node_ids
        let layer = &doc.layers[&layer_id];
        assert!(layer.node_ids.contains(&group_id));
        assert!(!layer.node_ids.contains(&node_a_id));
        assert!(!layer.node_ids.contains(&node_b_id));
        assert!(doc.nodes.contains_key(&group_id));

        history.undo(&mut doc);

        // After undo: group gone, children restored in layer.node_ids
        let layer = &doc.layers[&layer_id];
        assert!(!layer.node_ids.contains(&group_id));
        assert!(layer.node_ids.contains(&node_a_id));
        assert!(layer.node_ids.contains(&node_b_id));
        assert!(!doc.nodes.contains_key(&group_id));
    }

    // ── Batch ─────────────────────────────────────────────────────────────────

    #[test]
    fn batch_undo() {
        let mut doc = make_doc();
        let mut history = CommandHistory::new(200);

        let node_a = make_node(&doc);
        let node_b = make_node(&doc);
        let node_a_id = node_a.id;
        let node_b_id = node_b.id;

        history.execute(
            Command::Batch(vec![
                Command::AddNode {
                    node: node_a,
                    layer_id: None,
                },
                Command::AddNode {
                    node: node_b,
                    layer_id: None,
                },
            ]),
            &mut doc,
        );
        assert!(doc.nodes.contains_key(&node_a_id));
        assert!(doc.nodes.contains_key(&node_b_id));
        assert_eq!(history.undo_depth(), 1);

        history.undo(&mut doc);
        assert!(!doc.nodes.contains_key(&node_a_id));
        assert!(!doc.nodes.contains_key(&node_b_id));
        assert_eq!(history.undo_depth(), 0);
    }

    // ── max_depth ─────────────────────────────────────────────────────────────

    #[test]
    fn max_depth_respected() {
        let max = 5;
        let mut doc = make_doc();
        let mut history = CommandHistory::new(max);

        for _ in 0..(max + 3) {
            let node = make_node(&doc);
            history.execute(
                Command::AddNode {
                    node,
                    layer_id: None,
                },
                &mut doc,
            );
        }
        assert_eq!(history.undo_depth(), max);
    }

    // ── can_undo / can_redo ───────────────────────────────────────────────────

    #[test]
    fn can_undo_can_redo_states() {
        let mut doc = make_doc();
        let mut history = CommandHistory::new(200);

        assert!(!history.can_undo());
        assert!(!history.can_redo());

        let node = make_node(&doc);
        history.execute(
            Command::AddNode {
                node,
                layer_id: None,
            },
            &mut doc,
        );
        assert!(history.can_undo());
        assert!(!history.can_redo());

        history.undo(&mut doc);
        assert!(!history.can_undo());
        assert!(history.can_redo());

        history.redo(&mut doc);
        assert!(history.can_undo());
        assert!(!history.can_redo());
    }

    // ── Checkpoints ───────────────────────────────────────────────────────────

    #[test]
    fn checkpoint_create_list_restore() {
        let mut doc = make_doc();
        let mut history = CommandHistory::new(200);

        // Execute a command so undo stack is non-empty
        let node = make_node(&doc);
        let node_id = node.id;
        history.execute(
            Command::AddNode {
                node,
                layer_id: None,
            },
            &mut doc,
        );
        assert_eq!(history.undo_depth(), 1);

        // Create checkpoint with node present
        let cp_id = history.create_checkpoint("after add".to_string(), &doc);
        let infos = history.list_checkpoints();
        assert_eq!(infos.len(), 1);
        assert_eq!(infos[0].id, cp_id);
        assert_eq!(infos[0].name, "after add");

        // Execute another command to dirty the state
        let node2 = make_node(&doc);
        history.execute(
            Command::AddNode {
                node: node2,
                layer_id: None,
            },
            &mut doc,
        );
        assert_eq!(history.undo_depth(), 2);

        // Restore to checkpoint — undo/redo cleared, document back to snapshot
        let restored = history.restore_checkpoint(cp_id).unwrap();
        assert!(restored.nodes.contains_key(&node_id));
        assert_eq!(history.undo_depth(), 0);
        assert!(!history.can_redo());
    }
}

/// A reversible command that can be applied to a Document.
/// Each variant carries enough data to undo itself.
#[derive(Debug, Clone)]
pub enum Command {
    /// Add a new node to the document.
    AddNode {
        node: SceneNode,
        layer_id: Option<LayerId>,
    },
    /// Remove an existing node.
    RemoveNode { node_id: NodeId },
    /// Replace a node (used for any property update — stores old node for undo).
    UpdateNode { old: SceneNode, new: SceneNode },
    /// Add a layer.
    AddLayer { layer: Layer },
    /// Remove a layer.
    RemoveLayer { layer_id: LayerId },
    /// Reorder layers.
    ReorderLayers {
        old_order: Vec<LayerId>,
        new_order: Vec<LayerId>,
    },
    /// Change active layer.
    SetActiveLayer {
        old_id: Option<LayerId>,
        new_id: Option<LayerId>,
    },
    /// Batch multiple commands as one undo step.
    Batch(Vec<Command>),

    /// Move a node to a different z-position within its layer.
    /// `old_index` is stored for undo (swap old/new to reverse).
    ReorderNode {
        layer_id: LayerId,
        node_id: NodeId,
        old_index: usize,
        new_index: usize,
    },

    /// Promote a set of nodes into a new group, removing them from
    /// `layer.node_ids` and inserting the group in their place.
    /// Children remain in `doc.nodes`; only their layer membership changes.
    GroupNodes {
        /// The fully constructed group SceneNode (kind: Group).
        group: SceneNode,
        /// The layer the group is inserted into.
        layer_id: LayerId,
        /// Index at which the group is inserted in layer.node_ids
        /// (position of the bottom-most child before grouping).
        insert_index: usize,
        /// Children in bottom-to-top order.
        children: Vec<NodeId>,
    },

    /// Dissolve a group, re-inserting its children into the layer at the
    /// group's former position. The full group SceneNode is stored so the
    /// inverse (re-grouping) can reconstruct it without querying the document.
    UngroupNodes {
        /// Full group node — stored for undo reconstruction.
        group: SceneNode,
        /// The layer the group belonged to.
        layer_id: LayerId,
        /// The z-index the group occupied in layer.node_ids.
        group_index: usize,
        /// Children in bottom-to-top order.
        children: Vec<NodeId>,
    },

    /// Remove a layer, storing the full Layer struct so the inverse
    /// (`AddLayer`) can be computed without a document lookup.
    /// Use this instead of `RemoveLayer` when the command appears inside
    /// a `Batch` and the layer may already be absent from the document
    /// at undo-inverse-computation time.
    RemoveLayerFull { layer: Layer },

    /// Update mutable layer metadata (name, visible, locked, color).
    /// Stores old and new values so the inverse is self-contained.
    UpdateLayer {
        layer_id: LayerId,
        old_name: String,
        new_name: String,
        old_visible: bool,
        new_visible: bool,
        old_locked: bool,
        new_locked: bool,
        old_color: Option<[f32; 4]>,
        new_color: Option<[f32; 4]>,
        old_is_template: bool,
        new_is_template: bool,
    },

    /// Move a top-level node from one layer to another.
    /// All fields are stored so the inverse is fully self-contained.
    MoveNodeToLayer {
        node_id: NodeId,
        old_layer_id: LayerId,
        new_layer_id: LayerId,
        /// Node's z-index in `old_layer` before the move (stored for undo).
        old_index: usize,
        /// Desired z-index in `new_layer` after the move (clamped on apply).
        new_index: usize,
    },

    /// Replace the entire guide list. Stores old and new for self-contained undo.
    SetGuides { old: Vec<Guide>, new: Vec<Guide> },

    /// Resize the document canvas.
    ResizeCanvas {
        old_width: f64,
        old_height: f64,
        new_width: f64,
        new_height: f64,
    },
}

impl Command {
    /// Return a short human-readable description of this command.
    pub fn description(&self) -> String {
        match self {
            Command::AddNode { node, .. } => format!("Add {}", node.name),
            Command::RemoveNode { .. } => "Remove node".to_string(),
            Command::UpdateNode { new, .. } => format!("Update {}", new.name),
            Command::AddLayer { layer } => format!("Add layer \"{}\"", layer.name),
            Command::RemoveLayer { .. } => "Remove layer".to_string(),
            Command::ReorderLayers { .. } => "Reorder layers".to_string(),
            Command::SetActiveLayer { .. } => "Change active layer".to_string(),
            Command::ReorderNode { .. } => "Reorder node".to_string(),
            Command::GroupNodes { group, .. } => format!("Group → {}", group.name),
            Command::UngroupNodes { group, .. } => format!("Ungroup {}", group.name),
            Command::RemoveLayerFull { layer } => format!("Remove layer \"{}\"", layer.name),
            Command::UpdateLayer { new_name, .. } => format!("Update layer \"{}\"", new_name),
            Command::MoveNodeToLayer { .. } => "Move node to layer".to_string(),
            Command::SetGuides { .. } => "Update guides".to_string(),
            Command::ResizeCanvas {
                new_width,
                new_height,
                ..
            } => format!("Resize canvas to {new_width}×{new_height}"),
            Command::Batch(cmds) => {
                // Use the name of the first AddNode result, falling back to
                // the description of the first command in the batch.
                cmds.iter()
                    .find_map(|c| {
                        if let Command::AddNode { node, .. } = c {
                            Some(format!("Create {}", node.name))
                        } else {
                            None
                        }
                    })
                    .unwrap_or_else(|| {
                        cmds.first()
                            .map(|c| c.description())
                            .unwrap_or_else(|| "Batch".to_string())
                    })
            }
        }
    }

    /// Apply this command to the document, mutating it.
    pub fn apply(&self, doc: &mut Document) {
        match self {
            Command::AddNode { node, layer_id } => {
                doc.add_node(node.clone(), *layer_id);
            }
            Command::RemoveNode { node_id } => {
                doc.remove_node(node_id);
            }
            Command::UpdateNode { new, .. } => {
                if let Some(n) = doc.nodes.get_mut(&new.id) {
                    *n = new.clone();
                }
            }
            Command::AddLayer { layer } => {
                doc.add_layer(layer.clone());
            }
            Command::RemoveLayer { layer_id } => {
                doc.remove_layer(layer_id);
            }
            Command::ReorderLayers { new_order, .. } => {
                doc.layer_order = new_order.clone();
            }
            Command::SetActiveLayer { new_id, .. } => {
                doc.active_layer_id = *new_id;
            }
            Command::Batch(cmds) => {
                for cmd in cmds {
                    cmd.apply(doc);
                }
            }

            Command::ReorderNode {
                layer_id,
                node_id,
                new_index,
                ..
            } => {
                if let Some(layer) = doc.layers.get_mut(layer_id) {
                    if let Some(pos) = layer.node_ids.iter().position(|id| id == node_id) {
                        layer.node_ids.remove(pos);
                        let clamped = (*new_index).min(layer.node_ids.len());
                        layer.node_ids.insert(clamped, *node_id);
                    }
                }
            }

            Command::GroupNodes {
                group,
                layer_id,
                insert_index,
                children,
            } => {
                if let Some(layer) = doc.layers.get_mut(layer_id) {
                    layer.node_ids.retain(|id| !children.contains(id));
                    let clamped = (*insert_index).min(layer.node_ids.len());
                    layer.node_ids.insert(clamped, group.id);
                }
                doc.nodes.insert(group.id, group.clone());
            }

            Command::UngroupNodes {
                group,
                layer_id,
                children,
                ..
            } => {
                doc.nodes.remove(&group.id);
                if let Some(layer) = doc.layers.get_mut(layer_id) {
                    if let Some(pos) = layer.node_ids.iter().position(|id| *id == group.id) {
                        layer.node_ids.remove(pos);
                        for (i, child_id) in children.iter().enumerate() {
                            layer.node_ids.insert(pos + i, *child_id);
                        }
                    }
                }
            }

            Command::RemoveLayerFull { layer } => {
                doc.remove_layer(&layer.id);
            }

            Command::UpdateLayer {
                layer_id,
                new_name,
                new_visible,
                new_locked,
                new_color,
                new_is_template,
                ..
            } => {
                if let Some(layer) = doc.layers.get_mut(layer_id) {
                    layer.name = new_name.clone();
                    layer.visible = *new_visible;
                    layer.locked = *new_locked;
                    layer.color = *new_color;
                    layer.is_template = *new_is_template;
                }
            }

            Command::MoveNodeToLayer {
                node_id,
                old_layer_id,
                new_layer_id,
                new_index,
                ..
            } => {
                if let Some(layer) = doc.layers.get_mut(old_layer_id) {
                    layer.node_ids.retain(|id| id != node_id);
                }
                if let Some(node) = doc.nodes.get_mut(node_id) {
                    node.layer_id = *new_layer_id;
                }
                if let Some(layer) = doc.layers.get_mut(new_layer_id) {
                    let clamped = (*new_index).min(layer.node_ids.len());
                    layer.node_ids.insert(clamped, *node_id);
                }
            }

            Command::SetGuides { new, .. } => {
                doc.guides = new.clone();
            }

            Command::ResizeCanvas {
                new_width,
                new_height,
                ..
            } => {
                doc.width = *new_width;
                doc.height = *new_height;
            }
        }
    }

    /// Compute the inverse command (for undo).
    /// Returns None if the inverse cannot be computed without document state.
    pub fn inverse(&self, doc: &Document) -> Option<Command> {
        match self {
            Command::AddNode { node, .. } => Some(Command::RemoveNode { node_id: node.id }),
            Command::RemoveNode { node_id } => {
                let node = doc.nodes.get(node_id)?.clone();
                Some(Command::AddNode {
                    node,
                    layer_id: None,
                })
            }
            Command::UpdateNode { old, new } => Some(Command::UpdateNode {
                old: new.clone(),
                new: old.clone(),
            }),
            Command::AddLayer { layer } => Some(Command::RemoveLayer { layer_id: layer.id }),
            Command::RemoveLayer { layer_id } => {
                let layer = doc.layers.get(layer_id)?.clone();
                Some(Command::AddLayer { layer })
            }
            Command::ReorderLayers {
                old_order,
                new_order,
            } => Some(Command::ReorderLayers {
                old_order: new_order.clone(),
                new_order: old_order.clone(),
            }),
            Command::SetActiveLayer { old_id, new_id } => Some(Command::SetActiveLayer {
                old_id: *new_id,
                new_id: *old_id,
            }),
            Command::Batch(cmds) => {
                // Inverse of a batch is the reversed batch of inverses
                let mut inv_cmds = vec![];
                for cmd in cmds.iter().rev() {
                    inv_cmds.push(cmd.inverse(doc)?);
                }
                Some(Command::Batch(inv_cmds))
            }

            Command::ReorderNode {
                layer_id,
                node_id,
                old_index,
                new_index,
            } => Some(Command::ReorderNode {
                layer_id: *layer_id,
                node_id: *node_id,
                old_index: *new_index,
                new_index: *old_index,
            }),

            Command::GroupNodes {
                group,
                layer_id,
                insert_index,
                children,
            } => Some(Command::UngroupNodes {
                group: group.clone(),
                layer_id: *layer_id,
                group_index: *insert_index,
                children: children.clone(),
            }),

            Command::UngroupNodes {
                group,
                layer_id,
                group_index,
                children,
            } => Some(Command::GroupNodes {
                group: group.clone(),
                layer_id: *layer_id,
                insert_index: *group_index,
                children: children.clone(),
            }),

            Command::RemoveLayerFull { layer } => Some(Command::AddLayer {
                layer: layer.clone(),
            }),

            Command::UpdateLayer {
                layer_id,
                old_name,
                new_name,
                old_visible,
                new_visible,
                old_locked,
                new_locked,
                old_color,
                new_color,
                old_is_template,
                new_is_template,
            } => Some(Command::UpdateLayer {
                layer_id: *layer_id,
                old_name: new_name.clone(),
                new_name: old_name.clone(),
                old_visible: *new_visible,
                new_visible: *old_visible,
                old_locked: *new_locked,
                new_locked: *old_locked,
                old_color: *new_color,
                new_color: *old_color,
                old_is_template: *new_is_template,
                new_is_template: *old_is_template,
            }),

            Command::MoveNodeToLayer {
                node_id,
                old_layer_id,
                new_layer_id,
                old_index,
                new_index,
            } => Some(Command::MoveNodeToLayer {
                node_id: *node_id,
                old_layer_id: *new_layer_id,
                new_layer_id: *old_layer_id,
                old_index: *new_index,
                new_index: *old_index,
            }),

            Command::SetGuides { old, new } => Some(Command::SetGuides {
                old: new.clone(),
                new: old.clone(),
            }),

            Command::ResizeCanvas {
                old_width,
                old_height,
                new_width,
                new_height,
            } => Some(Command::ResizeCanvas {
                old_width: *new_width,
                old_height: *new_height,
                new_width: *old_width,
                new_height: *old_height,
            }),
        }
    }
}

/// A named snapshot of the document at a point in time (like a git commit).
#[derive(Debug)]
pub struct Checkpoint {
    pub id: Uuid,
    pub name: String,
    /// Unix timestamp (seconds since epoch) when the checkpoint was created.
    pub created_at: u64,
    /// Full document snapshot for restoration.
    snapshot: Document,
}

/// Public summary of a checkpoint (no snapshot data).
#[derive(Debug, Clone)]
pub struct CheckpointInfo {
    pub id: Uuid,
    pub name: String,
    pub created_at: u64,
}

/// Reusable debounce timer for auto-checkpointing.
/// Call `schedule` on each mutation, `tick` on each poll interval.
#[derive(Debug)]
struct DebounceCheckpoint {
    pending_desc: Option<String>,
    last_at: Option<std::time::Instant>,
    timeout_secs: u64,
}

impl DebounceCheckpoint {
    fn new(timeout_secs: u64) -> Self {
        Self {
            pending_desc: None,
            last_at: None,
            timeout_secs,
        }
    }

    /// Record a pending description and reset the debounce window.
    fn schedule(&mut self, desc: impl Into<String>) {
        self.pending_desc = Some(desc.into());
        self.last_at = Some(std::time::Instant::now());
    }

    /// Returns `Some(desc)` if the timeout has elapsed and a checkpoint
    /// should be created; clears state so it won't fire again until
    /// `schedule` is called.
    fn tick(&mut self) -> Option<String> {
        let last = self.last_at?;
        if last.elapsed().as_secs() >= self.timeout_secs {
            self.last_at = None;
            Some(
                self.pending_desc
                    .take()
                    .unwrap_or_else(|| "Edit".to_string()),
            )
        } else {
            None
        }
    }
}

/// Maintains a history of commands applied to a Document, enabling undo/redo.
#[derive(Debug)]
pub struct CommandHistory {
    /// Commands that have been applied (undo stack).
    undo_stack: Vec<Command>,
    /// Commands that have been undone (redo stack). Cleared on new command.
    redo_stack: Vec<Command>,
    /// Maximum undo steps to retain.
    max_depth: usize,
    /// Named snapshots (git-style commits). Most recent is last.
    checkpoints: Vec<Checkpoint>,
    /// Named document branches — forks of the document state by name.
    branches: std::collections::HashMap<String, Document>,
    /// Debounce timer for GUI-triggered checkpoints (30 s timeout).
    gui_debounce: DebounceCheckpoint,
    /// Debounce timer for MCP-triggered checkpoints (60 s timeout).
    mcp_debounce: DebounceCheckpoint,
}

impl Default for CommandHistory {
    fn default() -> Self {
        Self::new(200)
    }
}

impl CommandHistory {
    pub fn new(max_depth: usize) -> Self {
        Self {
            undo_stack: vec![],
            redo_stack: vec![],
            max_depth,
            checkpoints: vec![],
            branches: std::collections::HashMap::new(),
            gui_debounce: DebounceCheckpoint::new(30),
            mcp_debounce: DebounceCheckpoint::new(60),
        }
    }

    /// Apply a command and push it onto the undo stack.
    /// Schedules a debounced checkpoint — the snapshot is written after 30 s of
    /// inactivity via [`tick_checkpoint`], so burst operations (e.g. drag) do
    /// not produce a checkpoint on every frame.
    pub fn execute(&mut self, cmd: Command, doc: &mut Document) {
        let desc = cmd.description();
        cmd.apply(doc);
        self.undo_stack.push(cmd);
        self.redo_stack.clear();
        // Trim to max depth
        if self.undo_stack.len() > self.max_depth {
            self.undo_stack.remove(0);
        }
        self.gui_debounce.schedule(desc);
    }

    /// Call once per frame from the render loop.  If a user action was recorded
    /// and 30 seconds have passed with no further actions, flushes the pending
    /// checkpoint.  Safe to call even when no action is pending.
    pub fn tick_checkpoint(&mut self, doc: &Document) {
        if let Some(desc) = self.gui_debounce.tick() {
            self.create_checkpoint(desc, doc);
        }
    }

    /// Called by the MCP server after each successful mutating tool call.
    /// Resets the 60-second debounce window, extending it on rapid sequential calls.
    pub fn schedule_mcp_checkpoint(&mut self, desc: impl Into<String>) {
        self.mcp_debounce.schedule(desc);
    }

    /// Called periodically by the MCP background task (every ~10 s).
    /// Flushes the pending checkpoint once 60 seconds have elapsed since the
    /// last MCP mutation — a true debounce so burst tool calls produce only
    /// one checkpoint.
    pub fn tick_mcp_checkpoint(&mut self, doc: &Document) {
        if let Some(desc) = self.mcp_debounce.tick() {
            self.create_checkpoint(desc, doc);
        }
    }

    /// Undo the last command.
    pub fn undo(&mut self, doc: &mut Document) -> bool {
        if let Some(cmd) = self.undo_stack.pop() {
            if let Some(inv) = cmd.inverse(doc) {
                inv.apply(doc);
                self.redo_stack.push(cmd);
                return true;
            } else {
                // Can't invert — put it back
                self.undo_stack.push(cmd);
            }
        }
        false
    }

    /// Redo the last undone command.
    pub fn redo(&mut self, doc: &mut Document) -> bool {
        if let Some(cmd) = self.redo_stack.pop() {
            cmd.apply(doc);
            self.undo_stack.push(cmd);
            true
        } else {
            false
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    pub fn undo_depth(&self) -> usize {
        self.undo_stack.len()
    }

    pub fn redo_depth(&self) -> usize {
        self.redo_stack.len()
    }

    // ── Checkpoints (git-style commits) ──────────────────────────────────

    /// Save a named snapshot of the document. Returns the new checkpoint ID.
    /// Keeps at most 50 checkpoints; oldest are dropped when the limit is reached.
    pub fn create_checkpoint(&mut self, name: String, doc: &Document) -> Uuid {
        let id = Uuid::new_v4();
        let created_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.checkpoints.push(Checkpoint {
            id,
            name,
            created_at,
            snapshot: doc.clone(),
        });
        const MAX_CHECKPOINTS: usize = 50;
        if self.checkpoints.len() > MAX_CHECKPOINTS {
            self.checkpoints.remove(0);
        }
        id
    }

    /// Return summary info for all checkpoints, oldest first.
    pub fn list_checkpoints(&self) -> Vec<CheckpointInfo> {
        self.checkpoints
            .iter()
            .map(|c| CheckpointInfo {
                id: c.id,
                name: c.name.clone(),
                created_at: c.created_at,
            })
            .collect()
    }

    /// Restore the document to a saved checkpoint. Clears undo/redo stacks.
    /// Returns the snapshot to replace the live document, or `None` if not found.
    pub fn restore_checkpoint(&mut self, id: Uuid) -> Option<Document> {
        let snapshot = self
            .checkpoints
            .iter()
            .find(|c| c.id == id)?
            .snapshot
            .clone();
        self.undo_stack.clear();
        self.redo_stack.clear();
        Some(snapshot)
    }

    /// Return a clone of the document snapshot at `id` without touching
    /// undo/redo stacks. Use this for read-only operations like diffing.
    pub fn get_checkpoint_snapshot(&self, id: Uuid) -> Option<Document> {
        self.checkpoints
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.snapshot.clone())
    }

    // ── Named branches ────────────────────────────────────────────────────

    /// Save the current document state as a named branch.
    /// If a branch with the same name already exists it is overwritten.
    pub fn branch_create(&mut self, name: String, doc: &Document) {
        self.branches.insert(name, doc.clone());
    }

    /// Return a sorted list of all branch names.
    pub fn branch_list(&self) -> Vec<String> {
        let mut names: Vec<String> = self.branches.keys().cloned().collect();
        names.sort();
        names
    }

    /// Restore the document to a named branch snapshot.
    /// Clears undo/redo stacks. Returns `None` if the branch doesn't exist.
    pub fn branch_switch(&mut self, name: &str) -> Option<Document> {
        let snapshot = self.branches.get(name)?.clone();
        self.undo_stack.clear();
        self.redo_stack.clear();
        Some(snapshot)
    }

    /// Delete a named branch. Returns `true` if it existed.
    pub fn branch_delete(&mut self, name: &str) -> bool {
        self.branches.remove(name).is_some()
    }

    /// Return the most recent `limit` undo stack entries as `(step_index, description)` pairs,
    /// newest first. `step_index` is 1-based (1 = most recent).
    pub fn history_entries(&self, limit: usize) -> Vec<(usize, String)> {
        self.undo_stack
            .iter()
            .rev()
            .take(limit)
            .enumerate()
            .map(|(i, cmd)| (i + 1, cmd.description()))
            .collect()
    }

    /// Revert a specific node to its state `steps` mutations ago (without
    /// touching any other nodes). Scans the undo stack backwards; counts any
    /// `UpdateNode` or `Batch` command that contained an update to `node_id`.
    ///
    /// Applies the reverted state as a new undoable `UpdateNode` command so the
    /// revert itself can be undone.
    ///
    /// Returns `Some(actual_steps)` — the number of node-specific history
    /// entries that were scanned — or `None` if the node isn't in the document
    /// or has no history.
    pub fn revert_node_steps(
        &mut self,
        node_id: NodeId,
        steps: usize,
        doc: &mut Document,
    ) -> Option<usize> {
        let current = doc.nodes.get(&node_id)?.clone();
        let steps = steps.max(1);

        // Collect UpdateNode commands that touched this node, newest first.
        let mut hits: Vec<SceneNode> = Vec::new(); // each hit's `old` (pre-mutation state)
        for cmd in self.undo_stack.iter().rev() {
            collect_node_olds(cmd, node_id, &mut hits);
            if hits.len() >= steps {
                break;
            }
        }

        if hits.is_empty() {
            return None;
        }

        // The furthest-back `old` is the last element collected.
        let target_state = hits.last().unwrap().clone();
        let actual = hits.len();

        // Apply as a new undoable command.
        self.execute(
            Command::UpdateNode {
                old: current,
                new: target_state,
            },
            doc,
        );

        Some(actual)
    }
}

/// Recursively collect the `old` side of any `UpdateNode` command in `cmd`
/// that touches `node_id`, appending to `out`.
fn collect_node_olds(cmd: &Command, node_id: NodeId, out: &mut Vec<SceneNode>) {
    match cmd {
        Command::UpdateNode { old, new } if new.id == node_id => {
            out.push(old.clone());
        }
        Command::Batch(cmds) => {
            for c in cmds {
                collect_node_olds(c, node_id, out);
            }
        }
        _ => {}
    }
}
