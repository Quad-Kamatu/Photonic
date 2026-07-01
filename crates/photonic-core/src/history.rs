use crate::{
    document::{Document, Guide, WidthProfile},
    layer::{Layer, LayerId},
    node::{NodeId, SceneNode},
};
use serde::{Deserialize, Serialize};
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

    // ── Persistence: snapshot / restore round-trips ──────────────────────────

    fn push_n_nodes(history: &mut CommandHistory, doc: &mut Document, n: usize) {
        for _ in 0..n {
            let node = make_node(doc);
            history.execute(
                Command::AddNode {
                    node,
                    layer_id: None,
                },
                doc,
            );
        }
    }

    #[test]
    fn snapshot_restore_round_trips_undo_stack() {
        let mut doc = make_doc();
        let mut history = CommandHistory::new(200);
        push_n_nodes(&mut history, &mut doc, 3);
        let cp = history.create_checkpoint("cp".into(), &doc);
        assert_eq!(history.undo_depth(), 3);

        let snap = history.snapshot_state();
        // Serialize → deserialize (proves Command + Checkpoint are serde-safe).
        let json = serde_json::to_string(&snap).unwrap();
        let restored: HistorySnapshot = serde_json::from_str(&json).unwrap();

        let mut fresh = CommandHistory::new(200);
        fresh.restore_state(restored);
        assert_eq!(fresh.undo_depth(), 3);
        assert_eq!(fresh.list_checkpoints().len(), 1);
        assert_eq!(fresh.list_checkpoints()[0].id, cp);
        // Restored history is still functional: undo unwinds a real command.
        assert!(fresh.undo(&mut doc));
        assert_eq!(fresh.undo_depth(), 2);
    }

    #[test]
    fn set_limits_trims_to_step_ceiling() {
        let mut doc = make_doc();
        let mut history = CommandHistory::new(200);
        push_n_nodes(&mut history, &mut doc, 10);
        assert_eq!(history.undo_depth(), 10);

        history.set_limits(4, None);
        assert_eq!(history.undo_depth(), 4, "step ceiling not enforced");
        // A warning should have latched on the trim.
        assert!(history.take_limit_warning().is_some());
        // Drained once.
        assert!(history.take_limit_warning().is_none());
    }

    #[test]
    fn size_cap_trims_until_within_budget() {
        let mut doc = make_doc();
        let mut history = CommandHistory::new(100_000);
        push_n_nodes(&mut history, &mut doc, 30);
        let full = history.history_byte_size();
        assert!(full > 0);

        // Budget that only fits a fraction of the history forces trimming.
        let budget = full / 3;
        history.set_limits(100_000, Some(budget));
        assert!(
            history.history_byte_size() <= budget || history.undo_depth() <= 5,
            "size cap did not bring history within budget (or down to the floor)"
        );
        assert!(history.undo_depth() < 30, "nothing was trimmed");
    }

    #[test]
    fn checkpoint_snapshot_content_survives_round_trip() {
        let mut doc = make_doc();
        let mut history = CommandHistory::new(200);
        push_n_nodes(&mut history, &mut doc, 2);
        let node_ct = doc.nodes.len();
        let cp = history.create_checkpoint("cp".into(), &doc);

        let json = serde_json::to_string(&history.snapshot_state()).unwrap();
        let restored: HistorySnapshot = serde_json::from_str(&json).unwrap();
        let mut fresh = CommandHistory::new(200);
        fresh.restore_state(restored);

        let snap_doc = fresh
            .restore_checkpoint(cp)
            .expect("checkpoint must be restorable after round-trip");
        assert_eq!(
            snap_doc.nodes.len(),
            node_ct,
            "checkpoint snapshot lost its document content across serialization"
        );
    }

    #[test]
    fn size_cap_never_trims_named_checkpoints() {
        let mut doc = make_doc();
        let mut history = CommandHistory::new(100_000);
        push_n_nodes(&mut history, &mut doc, 4);
        history.create_checkpoint("keep".into(), &doc);
        let full = history.history_byte_size();

        // A budget far below a single checkpoint forces maximal trimming.
        history.set_limits(100_000, Some(full / 4));
        // Undo steps may be trimmed, but the named checkpoint is preserved …
        assert_eq!(
            history.list_checkpoints().len(),
            1,
            "size cap must never auto-delete a named checkpoint"
        );
        // … and because the un-trimmable checkpoint dominates, an honest
        // over-budget warning is raised.
        assert!(history.take_limit_warning().is_some());
    }

    #[test]
    fn reset_clears_all_persistent_state() {
        let mut doc = make_doc();
        let mut history = CommandHistory::new(200);
        push_n_nodes(&mut history, &mut doc, 3);
        history.create_checkpoint("cp".into(), &doc);
        history.reset();
        assert_eq!(history.undo_depth(), 0);
        assert!(history.list_checkpoints().is_empty());
        assert!(!history.can_undo());
    }

    // ── RemoveNode / RemoveLayer deletion undo (#153) ────────────────────────
    //
    // Regression: `RemoveNode`/`RemoveLayer` computed their inverse by reading
    // the entity out of the current document, but `undo()` runs `inverse()`
    // *after* `apply()` has already deleted it — so the lookup returned `None`
    // and undo silently no-oped. `execute` now hydrates bare deletes into their
    // self-contained `*Full` form (while the entity still exists) so the pushed
    // undo entry is always invertible.

    #[test]
    fn delete_node_undo_redo_round_trip() {
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

        // Delete via the *bare* RemoveNode — this is what all ~40 call sites emit.
        history.execute(Command::RemoveNode { node_id }, &mut doc);
        assert!(!doc.nodes.contains_key(&node_id), "node not deleted");
        assert!(!doc.layers[&layer_id].node_ids.contains(&node_id));

        // Undo must actually restore the node (previously a silent no-op).
        let undone = history.undo(&mut doc);
        assert!(undone, "undo of node deletion no-oped (#153)");
        assert!(
            doc.nodes.contains_key(&node_id),
            "node not restored on undo"
        );
        assert!(
            doc.layers[&layer_id].node_ids.contains(&node_id),
            "node not restored into its original layer"
        );
        // Secondary bug: restored node must keep its ORIGINAL layer, not the
        // active layer.
        assert_eq!(doc.nodes[&node_id].layer_id, layer_id);

        // Redo must delete it again.
        let redone = history.redo(&mut doc);
        assert!(redone, "redo of node deletion failed");
        assert!(!doc.nodes.contains_key(&node_id));
    }

    #[test]
    fn delete_node_into_non_active_layer_restores_original_layer() {
        // Reproduces the secondary defect: the old inverse used
        // `layer_id: None`, re-homing the undeleted node to the *active* layer.
        let mut doc = make_doc();
        let mut history = CommandHistory::new(200);
        let original_layer = doc.active_layer_id.unwrap();

        let node = make_node(&doc);
        let node_id = node.id;
        history.execute(
            Command::AddNode {
                node,
                layer_id: None,
            },
            &mut doc,
        );

        // Add a second layer and make IT active, so "active" != node's layer.
        let layer2 = Layer::new("layer2");
        let layer2_id = layer2.id;
        history.execute(Command::AddLayer { layer: layer2 }, &mut doc);
        history.execute(
            Command::SetActiveLayer {
                old_id: Some(original_layer),
                new_id: Some(layer2_id),
            },
            &mut doc,
        );
        assert_eq!(doc.active_layer_id, Some(layer2_id));

        history.execute(Command::RemoveNode { node_id }, &mut doc);
        assert!(history.undo(&mut doc));

        assert_eq!(
            doc.nodes[&node_id].layer_id, original_layer,
            "restored node re-homed to active layer instead of original"
        );
        assert!(doc.layers[&original_layer].node_ids.contains(&node_id));
        assert!(!doc.layers[&layer2_id].node_ids.contains(&node_id));
    }

    #[test]
    fn delete_layer_undo_redo_round_trip() {
        let mut doc = make_doc();
        let mut history = CommandHistory::new(200);
        let layer = Layer::new("layer2");
        let layer_id = layer.id;
        history.execute(Command::AddLayer { layer }, &mut doc);
        assert!(doc.layers.contains_key(&layer_id));

        history.execute(Command::RemoveLayer { layer_id }, &mut doc);
        assert!(!doc.layers.contains_key(&layer_id), "layer not deleted");

        let undone = history.undo(&mut doc);
        assert!(undone, "undo of layer deletion no-oped (#153)");
        assert!(
            doc.layers.contains_key(&layer_id),
            "layer not restored on undo"
        );

        let redone = history.redo(&mut doc);
        assert!(redone, "redo of layer deletion failed");
        assert!(!doc.layers.contains_key(&layer_id));
    }

    #[test]
    fn delete_node_in_batch_undo_redo_round_trip() {
        let mut doc = make_doc();
        let mut history = CommandHistory::new(200);
        let node_a = make_node(&doc);
        let node_b = make_node(&doc);
        let node_a_id = node_a.id;
        let node_b_id = node_b.id;
        let layer_id = node_a.layer_id;
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

        // Delete both nodes in a single batch of bare RemoveNode commands.
        history.execute(
            Command::Batch(vec![
                Command::RemoveNode { node_id: node_a_id },
                Command::RemoveNode { node_id: node_b_id },
            ]),
            &mut doc,
        );
        assert!(!doc.nodes.contains_key(&node_a_id));
        assert!(!doc.nodes.contains_key(&node_b_id));

        // Previously the batch inverse propagated the None and no-oped.
        let undone = history.undo(&mut doc);
        assert!(undone, "undo of batched node deletion no-oped (#153)");
        assert!(doc.nodes.contains_key(&node_a_id));
        assert!(doc.nodes.contains_key(&node_b_id));
        assert!(doc.layers[&layer_id].node_ids.contains(&node_a_id));
        assert!(doc.layers[&layer_id].node_ids.contains(&node_b_id));

        let redone = history.redo(&mut doc);
        assert!(redone, "redo of batched node deletion failed");
        assert!(!doc.nodes.contains_key(&node_a_id));
        assert!(!doc.nodes.contains_key(&node_b_id));
    }

    #[test]
    fn execute_hydrates_bare_deletes_into_self_contained_forms() {
        let mut doc = make_doc();
        let mut history = CommandHistory::new(200);

        // Node delete → pushed entry must be RemoveNodeFull.
        let node = make_node(&doc);
        let node_id = node.id;
        history.execute(
            Command::AddNode {
                node,
                layer_id: None,
            },
            &mut doc,
        );
        history.execute(Command::RemoveNode { node_id }, &mut doc);
        assert!(
            matches!(
                history.undo_stack.last(),
                Some(Command::RemoveNodeFull { node }) if node.id == node_id
            ),
            "RemoveNode was not hydrated into RemoveNodeFull on the undo stack"
        );

        // Layer delete → pushed entry must be RemoveLayerFull.
        let layer = Layer::new("layer2");
        let layer_id = layer.id;
        history.execute(Command::AddLayer { layer }, &mut doc);
        history.execute(Command::RemoveLayer { layer_id }, &mut doc);
        assert!(
            matches!(
                history.undo_stack.last(),
                Some(Command::RemoveLayerFull { layer }) if layer.id == layer_id
            ),
            "RemoveLayer was not hydrated into RemoveLayerFull on the undo stack"
        );

        // Batch delete → each element hydrated recursively.
        let n2 = make_node(&doc);
        let n2_id = n2.id;
        history.execute(
            Command::AddNode {
                node: n2,
                layer_id: None,
            },
            &mut doc,
        );
        history.execute(
            Command::Batch(vec![Command::RemoveNode { node_id: n2_id }]),
            &mut doc,
        );
        match history.undo_stack.last() {
            Some(Command::Batch(cmds)) => assert!(
                matches!(cmds.as_slice(), [Command::RemoveNodeFull { node }] if node.id == n2_id),
                "RemoveNode inside Batch was not hydrated"
            ),
            other => panic!("expected Batch on undo stack, got {other:?}"),
        }
    }
}

/// A reversible command that can be applied to a Document.
/// Each variant carries enough data to undo itself.
#[derive(Debug, Clone, Serialize, Deserialize)]
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

    /// Remove a node, storing the full `SceneNode` so the inverse (`AddNode`)
    /// can be computed without a document lookup. Mirrors `RemoveLayerFull`.
    ///
    /// Bare `RemoveNode { node_id }` computes its inverse by reading the node
    /// out of the current document, but `undo()` runs `inverse()` *after*
    /// `apply()` has already deleted the node, so the lookup returns `None`
    /// and undo silently no-ops. `hydrate` rewrites `RemoveNode` into this
    /// self-contained form at `execute` time (while the node still exists),
    /// so the pushed undo entry — and the persisted `.photon` history — is
    /// always invertible.
    RemoveNodeFull { node: SceneNode },

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

    /// Replace the entire artboard list (move/resize/rename/add/remove of
    /// artboards). Stores old and new for self-contained undo.
    SetArtboards {
        old: Vec<crate::Artboard>,
        new: Vec<crate::Artboard>,
    },

    /// Replace the entire variable-width profile list (used by the Width tool
    /// when editing a profile's samples on canvas). Profiles are small, so the
    /// whole list is snapshotted for self-contained undo.
    SetWidthProfiles {
        old: Vec<WidthProfile>,
        new: Vec<WidthProfile>,
    },

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
            Command::RemoveNodeFull { node } => format!("Remove {}", node.name),
            Command::UpdateLayer { new_name, .. } => format!("Update layer \"{}\"", new_name),
            Command::MoveNodeToLayer { .. } => "Move node to layer".to_string(),
            Command::SetGuides { .. } => "Update guides".to_string(),
            Command::SetArtboards { .. } => "Update artboards".to_string(),
            Command::SetWidthProfiles { .. } => "Edit width profile".to_string(),
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

    /// Normalize deletion commands into their self-contained `*Full` forms
    /// **while the target entity still exists** in `doc`.
    ///
    /// This is called once at the single choke point [`History::execute`],
    /// immediately before `apply`, so the command pushed onto the undo stack
    /// (and later persisted into the `.photon` history) always carries the full
    /// payload needed to invert itself. Without this, a bare
    /// `RemoveNode`/`RemoveLayer` would try to read the entity out of the
    /// document during `undo()` — but `apply()` has already deleted it, so the
    /// lookup returns `None` and undo silently no-ops.
    ///
    /// Rewrites performed:
    /// - `RemoveNode { node_id }`   → `RemoveNodeFull { node }`  (if present)
    /// - `RemoveLayer { layer_id }` → `RemoveLayerFull { layer }` (if present)
    /// - `Batch(cmds)`              → recurse into each element
    ///
    /// If the entity is already absent the command is returned unchanged
    /// (its `apply` is then a harmless no-op). All other variants pass through.
    pub fn hydrate(self, doc: &Document) -> Command {
        match self {
            Command::RemoveNode { node_id } => match doc.nodes.get(&node_id) {
                Some(node) => Command::RemoveNodeFull { node: node.clone() },
                None => Command::RemoveNode { node_id },
            },
            Command::RemoveLayer { layer_id } => match doc.layers.get(&layer_id) {
                Some(layer) => Command::RemoveLayerFull {
                    layer: layer.clone(),
                },
                None => Command::RemoveLayer { layer_id },
            },
            Command::Batch(cmds) => {
                Command::Batch(cmds.into_iter().map(|c| c.hydrate(doc)).collect())
            }
            other => other,
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

            Command::RemoveNodeFull { node } => {
                doc.remove_node(&node.id);
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

            Command::SetArtboards { new, .. } => {
                doc.artboards = new.clone();
                if doc
                    .active_artboard
                    .map_or(true, |id| !doc.artboards.iter().any(|a| a.id == id))
                {
                    doc.active_artboard = doc.artboards.first().map(|a| a.id);
                }
            }

            Command::SetWidthProfiles { new, .. } => {
                doc.width_profiles = new.clone();
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

            // Self-contained inverse: restore the node into its *original*
            // layer (not the active layer — that was the secondary bug in the
            // bare `RemoveNode` inverse, which passed `layer_id: None`).
            Command::RemoveNodeFull { node } => Some(Command::AddNode {
                node: node.clone(),
                layer_id: Some(node.layer_id),
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

            Command::SetArtboards { old, new } => Some(Command::SetArtboards {
                old: new.clone(),
                new: old.clone(),
            }),

            Command::SetWidthProfiles { old, new } => Some(Command::SetWidthProfiles {
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    pub id: Uuid,
    pub name: String,
    /// Unix timestamp (seconds since epoch) when the checkpoint was created.
    pub created_at: u64,
    /// Full document snapshot for restoration.
    snapshot: Document,
}

/// A serializable point-in-time copy of a [`CommandHistory`]'s persistent
/// state: the undo/redo stacks, named checkpoints, and named branches. The
/// transient parts of `CommandHistory` (debounce timers, the in-memory
/// `revision` counter, and the configured limits) are intentionally excluded —
/// they are runtime state, not project data.
///
/// This is what travels inside a `.photon` file so a project's full edit
/// history survives save → close → reopen and file transfer.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HistorySnapshot {
    #[serde(default)]
    pub undo_stack: Vec<Command>,
    #[serde(default)]
    pub redo_stack: Vec<Command>,
    #[serde(default)]
    pub checkpoints: Vec<Checkpoint>,
    #[serde(default)]
    pub branches: std::collections::HashMap<String, Document>,
}

impl HistorySnapshot {
    /// Bring nested documents (branch states and checkpoint snapshots) up to the
    /// load-time invariants the rest of the app relies on — currently, that every
    /// document has at least one artboard (`ensure_default_artboard`). The
    /// top-level document is normalized by [`Document::from_value`] on load, but
    /// the documents embedded in history bypass that path, so they are fixed up
    /// here after deserialization. Commands' embedded nodes need no such fixup.
    pub fn normalize_nested(&mut self) {
        for doc in self.branches.values_mut() {
            doc.ensure_default_artboard();
        }
        for cp in self.checkpoints.iter_mut() {
            cp.snapshot.ensure_default_artboard();
        }
    }
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
    /// Hard ceiling on retained undo steps. Always enforced (cheaply) on every
    /// `execute`, independent of the optional size cap below, so memory stays
    /// bounded even in size-limited mode.
    max_depth: usize,
    /// Optional cap on the *serialized* size of the persistent history (the
    /// `.photon` history payload, in bytes). `None` = no size cap. Enforced
    /// out of the hot path via [`enforce_size`] because measuring it requires
    /// serializing the history.
    size_limit_bytes: Option<u64>,
    /// Rising-edge latch for the user-facing "history limit reached" warning.
    /// Set true once when trimming begins; reset when history falls back under
    /// the soft threshold so the warning can fire again on the next breach.
    warned_at_limit: bool,
    /// A one-shot warning message for the GUI to surface, produced the first
    /// time the limit forces oldest steps to be dropped. Drained via
    /// [`take_limit_warning`].
    pending_warning: Option<String>,
    /// Named snapshots (git-style commits). Most recent is last.
    checkpoints: Vec<Checkpoint>,
    /// Named document branches — forks of the document state by name.
    branches: std::collections::HashMap<String, Document>,
    /// Debounce timer for GUI-triggered checkpoints (30 s timeout).
    gui_debounce: DebounceCheckpoint,
    /// Debounce timer for MCP-triggered checkpoints (60 s timeout).
    mcp_debounce: DebounceCheckpoint,
    /// Monotonically-incrementing content revision, bumped on every mutation that
    /// changes the document (execute / undo / redo / checkpoint or branch restore).
    /// Lets viewers (e.g. the GUI Pixel/Overprint Preview cache) detect content
    /// changes cheaply without re-serializing the whole document each frame.
    /// Never reset, so it cannot collide across document replacements.
    revision: u64,
}

/// Serialized byte length of a single history entry (a `Command` or
/// `Checkpoint`), used for incremental size accounting in
/// [`CommandHistory::enforce_size`].
fn entry_byte_size<T: Serialize>(v: &T) -> u64 {
    serde_json::to_vec(v).map(|b| b.len() as u64).unwrap_or(0)
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
            size_limit_bytes: None,
            warned_at_limit: false,
            pending_warning: None,
            checkpoints: vec![],
            branches: std::collections::HashMap::new(),
            gui_debounce: DebounceCheckpoint::new(30),
            mcp_debounce: DebounceCheckpoint::new(60),
            revision: 0,
        }
    }

    // ── Configurable history limits ──────────────────────────────────────────

    /// Soft floor on undo steps the size cap trims down to: while over budget we
    /// keep at least this many recent undo steps before falling back to trimming
    /// the redo stack. As an absolute last resort (redo empty, still over) undo
    /// may be taken below this, down to a single step. Named checkpoints and
    /// branches are deliberate user artifacts and are NEVER auto-trimmed.
    const MIN_RETAINED_STEPS: usize = 5;

    /// Set the retention limits and immediately re-enforce them.
    ///
    /// `max_steps` is the hard step ceiling (always >= 1). `size_bytes` is the
    /// optional cap on the serialized history payload. Cheap and idempotent when
    /// the limits are unchanged, so callers may invoke it every frame.
    pub fn set_limits(&mut self, max_steps: usize, size_bytes: Option<u64>) {
        let max_steps = max_steps.max(1);
        if self.max_depth == max_steps && self.size_limit_bytes == size_bytes {
            return;
        }
        self.max_depth = max_steps;
        self.size_limit_bytes = size_bytes;
        self.enforce_steps();
        self.enforce_size();
    }

    /// The configured step ceiling.
    pub fn max_depth(&self) -> usize {
        self.max_depth
    }

    /// The configured size cap in bytes, if any.
    pub fn size_limit_bytes(&self) -> Option<u64> {
        self.size_limit_bytes
    }

    /// Serialized size, in bytes, of the persistent history payload — exactly
    /// what gets written into the `.photon` file. This is the "history size"
    /// the size cap constrains (the document is measured separately).
    pub fn history_byte_size(&self) -> u64 {
        serde_json::to_vec(&self.snapshot_state())
            .map(|v| v.len() as u64)
            .unwrap_or(0)
    }

    /// Drop oldest undo steps until within the step ceiling. Cheap — no
    /// serialization. Latches a warning on the first step actually dropped.
    fn enforce_steps(&mut self) {
        let mut dropped = false;
        while self.undo_stack.len() > self.max_depth {
            self.undo_stack.remove(0);
            dropped = true;
        }
        // Recovered comfortably under the ceiling → re-arm the warning latch.
        if self.undo_stack.len() * 10 < self.max_depth * 9 {
            self.warned_at_limit = false;
        }
        if dropped {
            self.latch_warning(
                "Project history reached its maximum step count — the oldest \
                 undo steps are being discarded. Raise the limit in \
                 Edit ▸ Behavior ▸ Project History.",
            );
        }
    }

    /// Enforce the optional size cap by trimming the linear undo/redo history
    /// until the serialized payload is within budget. Named checkpoints and
    /// branches are user artifacts and are never auto-deleted — if they alone
    /// exceed the budget, a distinct warning is raised instead. No-op when no
    /// size cap is configured. Returns true if it dropped any step.
    ///
    /// Measures the whole history once, then trims against a running byte
    /// estimate (each removed entry's own serialized size), so the cost is
    /// O(history size) rather than O(entries · history size). One exact
    /// re-measure at the end drives the warning + re-arm decisions.
    pub fn enforce_size(&mut self) -> bool {
        let Some(limit) = self.size_limit_bytes else {
            return false;
        };

        let mut est = self.history_byte_size();
        let mut dropped = false;
        while est > limit {
            // `+1` approximates the JSON array separator per element.
            if self.undo_stack.len() > Self::MIN_RETAINED_STEPS {
                est = est.saturating_sub(entry_byte_size(&self.undo_stack[0]).saturating_add(1));
                self.undo_stack.remove(0);
            } else if !self.redo_stack.is_empty() {
                est = est.saturating_sub(entry_byte_size(&self.redo_stack[0]).saturating_add(1));
                self.redo_stack.remove(0);
            } else if self.undo_stack.len() > 1 {
                est = est.saturating_sub(entry_byte_size(&self.undo_stack[0]).saturating_add(1));
                self.undo_stack.remove(0);
            } else {
                // Only a single undo step plus un-trimmable checkpoints/branches
                // remain. Stop rather than wipe the last step.
                break;
            }
            dropped = true;
        }

        // Exact size now drives the (accurate) warning and the re-arm latch.
        let actual = self.history_byte_size();
        if actual > limit {
            self.latch_warning(
                "Project history exceeds its size limit because of saved \
                 checkpoints or branches — delete some, or raise the limit in \
                 Edit ▸ Behavior ▸ Project History.",
            );
        } else if dropped {
            self.latch_warning(
                "Project history reached its size limit — the oldest undo steps \
                 are being discarded to make room. Raise the limit in \
                 Edit ▸ Behavior ▸ Project History.",
            );
        }
        if actual * 10 < limit * 9 {
            self.warned_at_limit = false;
        }
        dropped
    }

    /// Set the one-shot warning on the rising edge only (so it fires once per
    /// breach, not on every trimmed step), with a context-specific message.
    fn latch_warning(&mut self, msg: &str) {
        if !self.warned_at_limit {
            self.warned_at_limit = true;
            self.pending_warning = Some(msg.to_string());
        }
    }

    /// Take the pending limit warning, if any, for the GUI to display once.
    pub fn take_limit_warning(&mut self) -> Option<String> {
        self.pending_warning.take()
    }

    // ── Persistence (save/restore the full history with the document) ─────────

    /// Capture the persistent history (undo/redo/checkpoints/branches) for
    /// serialization into a `.photon` file. Clones; does not mutate self.
    pub fn snapshot_state(&self) -> HistorySnapshot {
        HistorySnapshot {
            undo_stack: self.undo_stack.clone(),
            redo_stack: self.redo_stack.clone(),
            checkpoints: self.checkpoints.clone(),
            branches: self.branches.clone(),
        }
    }

    /// Replace the persistent history with a restored snapshot (on file open),
    /// then re-enforce the current limits. Configured limits, debounce timers,
    /// and the revision counter are preserved. Bumps `revision` so revision-
    /// keyed caches refresh.
    pub fn restore_state(&mut self, s: HistorySnapshot) {
        self.undo_stack = s.undo_stack;
        self.redo_stack = s.redo_stack;
        self.checkpoints = s.checkpoints;
        self.branches = s.branches;
        self.warned_at_limit = false;
        self.pending_warning = None;
        self.revision = self.revision.wrapping_add(1);
        self.enforce_steps();
        self.enforce_size();
    }

    /// Clear all persistent history (undo/redo/checkpoints/branches) while
    /// keeping the configured limits. Used when opening a document that carries
    /// no embedded history, or on New, so a previous project's history can't
    /// bleed into the freshly loaded one. Bumps `revision`.
    pub fn reset(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.checkpoints.clear();
        self.branches.clear();
        self.warned_at_limit = false;
        self.pending_warning = None;
        self.revision = self.revision.wrapping_add(1);
    }

    /// Apply a command and push it onto the undo stack.
    /// Schedules a debounced checkpoint — the snapshot is written after 30 s of
    /// inactivity via [`tick_checkpoint`], so burst operations (e.g. drag) do
    /// not produce a checkpoint on every frame.
    pub fn execute(&mut self, cmd: Command, doc: &mut Document) {
        // Normalize deletion commands into their self-contained `*Full` forms
        // while the target entity still exists, so the pushed undo entry (and
        // the persisted `.photon` history) is always invertible. See
        // [`Command::hydrate`].
        let cmd = cmd.hydrate(doc);
        let desc = cmd.description();
        cmd.apply(doc);
        reevaluate_constraints(doc);
        self.undo_stack.push(cmd);
        self.redo_stack.clear();
        // Enforce the step ceiling on the hot path (cheap). The optional size
        // cap is enforced separately via `enforce_size` (off the hot path,
        // since it must serialize the history to measure it).
        self.enforce_steps();
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
                reevaluate_constraints(doc);
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
            reevaluate_constraints(doc);
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

/// Re-evaluate live property constraints after a mutation. Errors (cycles,
/// parse failures, unsupported targets) are intentionally swallowed here so the
/// document stays usable and constrained properties keep their last valid
/// values; the MCP layer surfaces errors explicitly when a constraint is created.
fn reevaluate_constraints(doc: &mut Document) {
    if !doc.constraints.is_empty() {
        let _ = crate::ops::constraints::evaluate_constraints(doc);
    }
}
