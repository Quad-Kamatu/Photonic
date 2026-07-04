use crate::{
    document::{Document, Guide, WidthProfile},
    layer::{Layer, LayerId},
    node::{NodeId, SceneNode},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

mod branches;
mod checkpoints;
mod coalescing;
mod stacks;
mod tree;

pub use tree::{HistoryGraphNode, HistoryTree};

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

    // ── #191: Ctrl+Z must undo GUI delete (delete now recorded) ──────────────
    //
    // The two GUI delete entry points (command palette / Delete key `edit.delete`
    // and the Select-tool Delete/Backspace handler) used to mutate the doc
    // directly via `doc.remove_node`, bypassing `CommandHistory` — so `undo()`
    // returned `false` with nothing to revert. Both now emit a
    // `Command::Batch(vec![Command::RemoveNode { .. }])` through `history.execute`.
    // These tests lock that exact code path (single- and multi-node selection).

    #[test]
    fn gui_delete_single_node_batch_undo_restores_node() {
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

        // Exactly what the GUI delete paths now emit for a single selection.
        history.execute(
            Command::Batch(vec![Command::RemoveNode { node_id }]),
            &mut doc,
        );
        assert!(!doc.nodes.contains_key(&node_id), "node not deleted");
        assert!(!doc.layers[&layer_id].node_ids.contains(&node_id));

        // Ctrl+Z: undo must report success and bring the node back into its layer.
        let undone = history.undo(&mut doc);
        assert!(undone, "Ctrl+Z of a GUI delete no-oped (#191)");
        assert!(
            doc.nodes.contains_key(&node_id),
            "node not restored on undo (#191)"
        );
        assert_eq!(doc.nodes[&node_id].layer_id, layer_id);
        assert!(
            doc.layers[&layer_id].node_ids.contains(&node_id),
            "node not restored into its original layer (#191)"
        );
    }

    #[test]
    fn gui_delete_multi_node_batch_undo_restores_all() {
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

        // Multi-select delete: one Batch of bare RemoveNode, exactly like the GUI.
        history.execute(
            Command::Batch(vec![
                Command::RemoveNode { node_id: node_a_id },
                Command::RemoveNode { node_id: node_b_id },
            ]),
            &mut doc,
        );
        assert!(!doc.nodes.contains_key(&node_a_id));
        assert!(!doc.nodes.contains_key(&node_b_id));

        // A single Ctrl+Z restores the whole multi-select delete as one step.
        let undone = history.undo(&mut doc);
        assert!(undone, "Ctrl+Z of a multi-select GUI delete no-oped (#191)");
        assert!(doc.nodes.contains_key(&node_a_id));
        assert!(doc.nodes.contains_key(&node_b_id));
        assert!(
            doc.layers[&layer_id].node_ids.contains(&node_a_id),
            "node A not restored into its original layer (#191)"
        );
        assert!(
            doc.layers[&layer_id].node_ids.contains(&node_b_id),
            "node B not restored into its original layer (#191)"
        );
    }

    // ── #182: gesture coalescing (one drag → one undo step) ──────────────────
    //
    // Continuous edits (the fill/stroke RGBA color picker, #180) used to call
    // `history.execute(UpdateNode { .. })` on every pointer tick, so one drag
    // became dozens of undo steps. The GUI now opens a coalescing gesture while
    // the pointer is down; mergeable same-target commands fold into one anchor
    // entry. These tests lock that behavior at the `CommandHistory` layer.

    /// Streamed `UpdateNode`s for the same node during one gesture collapse to a
    /// single undo step, and one undo restores the pre-gesture state.
    #[test]
    fn coalesce_streamed_updates_into_one_step() {
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
        assert_eq!(history.undo_depth(), 1, "baseline: AddNode is one step");

        // Simulate a drag: many UpdateNode ticks inside one open gesture.
        history.begin_coalescing();
        let mut prev = node.clone();
        for i in 1..=20u32 {
            let mut next = prev.clone();
            next.name = format!("frame-{i}");
            history.execute(
                Command::UpdateNode {
                    old: prev.clone(),
                    new: next.clone(),
                },
                &mut doc,
            );
            prev = next;
        }
        history.end_coalescing();

        // The whole gesture is exactly one step on top of the AddNode.
        assert_eq!(
            history.undo_depth(),
            2,
            "20 streamed updates should coalesce into a single undo step"
        );
        assert_eq!(doc.nodes[&node_id].name, "frame-20");

        // One undo restores the pre-gesture state (the original node name).
        assert!(history.undo(&mut doc));
        assert_eq!(
            doc.nodes[&node_id].name, "rect",
            "single undo must restore the state from before the gesture"
        );
        // And redo re-applies the whole gesture in one step.
        assert!(history.redo(&mut doc));
        assert_eq!(doc.nodes[&node_id].name, "frame-20");
    }

    /// #182 fix round 1: an external (MCP/REPL/script) edit routed through
    /// `execute_discrete` must land as its OWN undo step even while a GUI pointer
    /// gesture is open on the shared history, and must not be folded into — nor
    /// swallow a later tick of — that gesture's anchor. The GUI and MCP server
    /// share one `Arc<Mutex<CommandHistory>>`, so this concurrency is realistic.
    #[test]
    fn execute_discrete_does_not_fold_into_open_gesture() {
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
        let base = history.undo_depth();

        // GUI opens a gesture and streams two ticks → one coalesced anchor step.
        history.begin_coalescing();
        let mut prev = node.clone();
        for i in 1..=2u32 {
            let mut next = prev.clone();
            next.name = format!("gui-{i}");
            history.execute(
                Command::UpdateNode {
                    old: prev.clone(),
                    new: next.clone(),
                },
                &mut doc,
            );
            prev = next;
        }
        assert_eq!(
            history.undo_depth(),
            base + 1,
            "the GUI gesture so far is a single coalesced anchor step"
        );

        // An external caller (simulated MCP tool) edits the SAME node mid-gesture.
        let mut ext = prev.clone();
        ext.name = "mcp-edit".into();
        history.execute_discrete(
            Command::UpdateNode {
                old: prev.clone(),
                new: ext.clone(),
            },
            &mut doc,
        );
        prev = ext;
        assert_eq!(
            history.undo_depth(),
            base + 2,
            "an external execute_discrete must push a SEPARATE undo step, not fold \
             into the open GUI gesture"
        );
        assert!(
            history.is_coalescing(),
            "execute_discrete must leave the GUI gesture open"
        );

        // The next GUI tick must NOT merge into the external command (its anchor is
        // no longer undo_stack.last()); it re-anchors as a fresh step.
        let mut resume = prev.clone();
        resume.name = "gui-3".into();
        history.execute(
            Command::UpdateNode {
                old: prev.clone(),
                new: resume.clone(),
            },
            &mut doc,
        );
        assert_eq!(
            history.undo_depth(),
            base + 3,
            "a GUI tick after an interleaved external edit must re-anchor, not fold \
             into the external command's step"
        );

        history.end_coalescing();

        // One undo peels off only the external edit — proving the AI edit and the
        // user's drag are independent, granular steps.
        assert!(history.undo(&mut doc));
        assert_eq!(doc.nodes[&node_id].name, "mcp-edit");
    }

    /// Core invariant the #183 GUI fix *relies on* — NOT the fix itself.
    ///
    /// A `Command::Batch` of `UpdateNode`s (the shape a completed move records)
    /// pushed while a coalescing gesture is open — with a primed same-target
    /// anchor, the adversarial fold case — must land as exactly ONE undo step,
    /// and a single `undo()` must restore every node's pre-move transform. This
    /// holds for both `execute` and `execute_discrete` because `Command::Batch`
    /// is never a mergeable `coalesce` target.
    ///
    /// NOTE (scope, per adversarial review): this locks pre-existing core
    /// behavior and would pass identically with the GUI #183 fix reverted. It
    /// does NOT exercise `PhotonicApp::finalize_move` or the release-time
    /// fallback branch — those are the actual #183 change and are covered by the
    /// pure predicate test `photonic-gui` `move_fallback_tests::*` plus manual
    /// GUI confirmation. Keep this as a core guardrail, not the #183 regression
    /// contract.
    #[test]
    fn batch_never_coalesces_into_open_gesture() {
        for discrete in [false, true] {
            let mut doc = make_doc();
            let mut history = CommandHistory::new(200);

            // Two nodes, moved together as a multi-selection.
            let a = make_node(&doc);
            let b = make_node(&doc);
            let a_id = a.id;
            let b_id = b.id;
            history.execute(
                Command::AddNode {
                    node: a,
                    layer_id: None,
                },
                &mut doc,
            );
            history.execute(
                Command::AddNode {
                    node: b,
                    layer_id: None,
                },
                &mut doc,
            );
            let base = history.undo_depth();

            // What the GUI snapshots at drag start: full pre-move nodes.
            let a_old = doc.nodes[&a_id].clone();
            let b_old = doc.nodes[&b_id].clone();
            let a0 = a_old.transform.matrix;
            let b0 = b_old.transform.matrix;

            // A pointer gesture is open (as it is on release, before
            // `end_coalescing`), and a prior same-target tick (e.g. a color
            // swatch drag on node A) has already armed the coalesce anchor — the
            // adversarial case where a naive push could fold the move into it.
            history.begin_coalescing();
            let mut primed = doc.nodes[&a_id].clone();
            primed.name = "primed".into();
            history.execute(
                Command::UpdateNode {
                    old: doc.nodes[&a_id].clone(),
                    new: primed,
                },
                &mut doc,
            );
            assert!(
                history.is_coalescing(),
                "gesture must still be open when the move is finalized"
            );
            let after_prime = history.undo_depth();

            // Perform the move on the doc (mirrors the GUI dragging both nodes),
            // then build the release-time Batch of UpdateNodes.
            for id in [a_id, b_id] {
                let n = doc.nodes.get_mut(&id).unwrap();
                n.transform.matrix[4] += 25.0;
                n.transform.matrix[5] += 40.0;
            }
            let move_batch = Command::Batch(vec![
                Command::UpdateNode {
                    old: a_old,
                    new: doc.nodes[&a_id].clone(),
                },
                Command::UpdateNode {
                    old: b_old,
                    new: doc.nodes[&b_id].clone(),
                },
            ]);

            if discrete {
                history.execute_discrete(move_batch, &mut doc);
            } else {
                history.execute(move_batch, &mut doc);
            }
            history.end_coalescing();

            // The move Batch is exactly one new step — never folded into the
            // primed anchor.
            assert_eq!(
                history.undo_depth(),
                after_prime + 1,
                "move Batch must record exactly one undo step (discrete={discrete})"
            );

            // One undo restores BOTH nodes' pre-move transforms in a single step.
            assert!(
                history.undo(&mut doc),
                "single Ctrl+Z must undo the move (discrete={discrete})"
            );
            assert_eq!(
                doc.nodes[&a_id].transform.matrix, a0,
                "undo must restore node A's pre-move transform (discrete={discrete})"
            );
            assert_eq!(
                doc.nodes[&b_id].transform.matrix, b0,
                "undo must restore node B's pre-move transform (discrete={discrete})"
            );
            // Exactly one step was peeled: the nodes still exist (the Add steps
            // below the move were not touched).
            assert!(
                history.undo_depth() >= base,
                "undo removed more than the single move step (discrete={discrete})"
            );
            assert!(doc.nodes.contains_key(&a_id) && doc.nodes.contains_key(&b_id));
        }
    }

    /// Two separate gestures produce two independent undo steps.
    #[test]
    fn coalesce_two_gestures_two_steps() {
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

        // Gesture 1.
        history.begin_coalescing();
        for i in 1..=5u32 {
            let mut next = doc.nodes[&node_id].clone();
            next.name = format!("g1-{i}");
            history.execute(
                Command::UpdateNode {
                    old: doc.nodes[&node_id].clone(),
                    new: next,
                },
                &mut doc,
            );
        }
        history.end_coalescing();

        // Gesture 2.
        history.begin_coalescing();
        for i in 1..=5u32 {
            let mut next = doc.nodes[&node_id].clone();
            next.name = format!("g2-{i}");
            history.execute(
                Command::UpdateNode {
                    old: doc.nodes[&node_id].clone(),
                    new: next,
                },
                &mut doc,
            );
        }
        history.end_coalescing();

        // AddNode + gesture1 + gesture2 = 3 steps.
        assert_eq!(
            history.undo_depth(),
            3,
            "two gestures must record two distinct undo steps"
        );
    }

    /// Updates to *different* nodes never merge, even inside one gesture.
    #[test]
    fn coalesce_different_node_ids_do_not_merge() {
        let mut doc = make_doc();
        let mut history = CommandHistory::new(200);
        let node_a = make_node(&doc);
        let node_b = make_node(&doc);
        let a_id = node_a.id;
        let b_id = node_b.id;
        history.execute(
            Command::AddNode {
                node: node_a.clone(),
                layer_id: None,
            },
            &mut doc,
        );
        history.execute(
            Command::AddNode {
                node: node_b.clone(),
                layer_id: None,
            },
            &mut doc,
        );
        let base_depth = history.undo_depth();

        history.begin_coalescing();
        // Update A, then B, then A again — A/B alternate so neither B-vs-A nor
        // A-vs-B ever merges; each is its own step.
        let mut a_new = doc.nodes[&a_id].clone();
        a_new.name = "a1".into();
        history.execute(
            Command::UpdateNode {
                old: doc.nodes[&a_id].clone(),
                new: a_new,
            },
            &mut doc,
        );
        let mut b_new = doc.nodes[&b_id].clone();
        b_new.name = "b1".into();
        history.execute(
            Command::UpdateNode {
                old: doc.nodes[&b_id].clone(),
                new: b_new,
            },
            &mut doc,
        );
        let mut a_new2 = doc.nodes[&a_id].clone();
        a_new2.name = "a2".into();
        history.execute(
            Command::UpdateNode {
                old: doc.nodes[&a_id].clone(),
                new: a_new2,
            },
            &mut doc,
        );
        history.end_coalescing();

        assert_eq!(
            history.undo_depth(),
            base_depth + 3,
            "edits to different nodes must not coalesce"
        );
    }

    /// Consecutive same-node updates DO merge even when interleaved with the
    /// anchor rule: an edit that follows a mergeable anchor folds, but the anchor
    /// only exists within the gesture that pushed it.
    #[test]
    fn coalesce_only_within_the_same_gesture() {
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

        // A pre-gesture UpdateNode pushes a normal step.
        let mut pre = doc.nodes[&node_id].clone();
        pre.name = "pre".into();
        history.execute(
            Command::UpdateNode {
                old: doc.nodes[&node_id].clone(),
                new: pre,
            },
            &mut doc,
        );
        let depth_before = history.undo_depth();

        // Opening a gesture must NOT fold the very first edit into the leftover
        // pre-gesture step — coalesce_started is false until this gesture pushes
        // its own anchor.
        history.begin_coalescing();
        let mut g1 = doc.nodes[&node_id].clone();
        g1.name = "g-1".into();
        history.execute(
            Command::UpdateNode {
                old: doc.nodes[&node_id].clone(),
                new: g1,
            },
            &mut doc,
        );
        assert_eq!(
            history.undo_depth(),
            depth_before + 1,
            "first edit of a gesture must push a fresh anchor, not fold into a prior step"
        );
        // Subsequent edits in the same gesture fold into that anchor.
        let mut g2 = doc.nodes[&node_id].clone();
        g2.name = "g-2".into();
        history.execute(
            Command::UpdateNode {
                old: doc.nodes[&node_id].clone(),
                new: g2,
            },
            &mut doc,
        );
        assert_eq!(
            history.undo_depth(),
            depth_before + 1,
            "later edits of the same gesture must fold into the anchor"
        );
        history.end_coalescing();
    }

    /// `Command::coalesce` merges same-target value-replace commands and refuses
    /// everything else.
    #[test]
    fn command_coalesce_merge_matrix() {
        let doc = make_doc();
        let n1 = make_node(&doc);
        let mut n1b = n1.clone();
        n1b.name = "b".into();
        let mut n1c = n1.clone();
        n1c.name = "c".into();

        // Same node id → merges, keeping first `old` and last `new`.
        let last = Command::UpdateNode {
            old: n1.clone(),
            new: n1b.clone(),
        };
        let next = Command::UpdateNode {
            old: n1b.clone(),
            new: n1c.clone(),
        };
        match Command::coalesce(&last, &next) {
            Some(Command::UpdateNode { old, new }) => {
                assert_eq!(old.name, "rect");
                assert_eq!(new.name, "c");
            }
            other => panic!("expected merged UpdateNode, got {other:?}"),
        }

        // Different node ids → no merge.
        let n2 = make_node(&doc);
        let mut n2b = n2.clone();
        n2b.name = "z".into();
        let other = Command::UpdateNode {
            old: n2.clone(),
            new: n2b,
        };
        assert!(Command::coalesce(&last, &other).is_none());

        // SetWidthProfiles merges.
        let w = Command::SetWidthProfiles {
            old: vec![],
            new: vec![],
        };
        assert!(matches!(
            Command::coalesce(&w, &w),
            Some(Command::SetWidthProfiles { .. })
        ));

        // ResizeCanvas merges, keeping first old dims + last new dims.
        let r1 = Command::ResizeCanvas {
            old_width: 10.0,
            old_height: 10.0,
            new_width: 20.0,
            new_height: 20.0,
        };
        let r2 = Command::ResizeCanvas {
            old_width: 20.0,
            old_height: 20.0,
            new_width: 30.0,
            new_height: 30.0,
        };
        match Command::coalesce(&r1, &r2) {
            Some(Command::ResizeCanvas {
                old_width,
                new_width,
                ..
            }) => {
                assert_eq!(old_width, 10.0);
                assert_eq!(new_width, 30.0);
            }
            other => panic!("expected merged ResizeCanvas, got {other:?}"),
        }

        // Non-value-replace command → never merges.
        let add = Command::AddNode {
            node: n1.clone(),
            layer_id: None,
        };
        assert!(Command::coalesce(&add, &add).is_none());
        // Mismatched variants → no merge.
        assert!(Command::coalesce(&last, &w).is_none());
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
                history.current_command(),
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
                history.current_command(),
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
        match history.current_command() {
            Some(Command::Batch(cmds)) => assert!(
                matches!(cmds.as_slice(), [Command::RemoveNodeFull { node }] if node.id == n2_id),
                "RemoveNode inside Batch was not hydrated"
            ),
            other => panic!("expected Batch on undo stack, got {other:?}"),
        }
    }

    // ── Edit tree / branching ─────────────────────────────────────────────────

    #[test]
    fn editing_after_undo_forks_a_branch_and_keeps_the_old_future() {
        let mut doc = make_doc();
        let mut h = CommandHistory::new(200);

        let a = make_node(&doc);
        let a_id = a.id;
        h.execute(Command::AddNode { node: a, layer_id: None }, &mut doc);
        let b = make_node(&doc);
        let b_id = b.id;
        h.execute(Command::AddNode { node: b, layer_id: None }, &mut doc);

        // Undo B, then make a different edit C — this must FORK (keep B), not
        // discard it the way a flat redo stack would.
        assert!(h.undo(&mut doc));
        assert!(!doc.nodes.contains_key(&b_id));
        let c = make_node(&doc);
        let c_id = c.id;
        h.execute(Command::AddNode { node: c, layer_id: None }, &mut doc);

        // Tree now: root → A → { B (undone), C (current) }.
        let graph = h.history_graph();
        assert_eq!(graph.len(), 4, "root + A + B + C expected");
        assert!(!h.can_redo(), "C is a leaf");

        // Locate the sibling (B) branch and jump across to it.
        let cur = graph.iter().find(|n| n.is_current).unwrap();
        let parent_id = cur.parent.unwrap();
        let parent = graph.iter().find(|n| n.id == parent_id).unwrap();
        let b_node = parent.children.iter().copied().find(|&x| x != cur.id).unwrap();

        assert!(h.jump_to_node(b_node, &mut doc), "jump to B branch");
        assert!(doc.nodes.contains_key(&b_id), "B restored after cross-branch jump");
        assert!(!doc.nodes.contains_key(&c_id), "C removed after jump to B branch");
        assert!(doc.nodes.contains_key(&a_id), "shared ancestor A still present");

        // Jump back to C and confirm the other branch swaps back in.
        assert!(h.jump_to_node(cur.id, &mut doc), "jump back to C branch");
        assert!(doc.nodes.contains_key(&c_id));
        assert!(!doc.nodes.contains_key(&b_id));
    }

    #[test]
    fn branching_survives_snapshot_round_trip() {
        let mut doc = make_doc();
        let mut h = CommandHistory::new(200);
        h.execute(Command::AddNode { node: make_node(&doc), layer_id: None }, &mut doc);
        h.execute(Command::AddNode { node: make_node(&doc), layer_id: None }, &mut doc);
        h.undo(&mut doc);
        h.execute(Command::AddNode { node: make_node(&doc), layer_id: None }, &mut doc);
        // 3 edits made across two branches → 4 tree nodes (root + 3).
        assert_eq!(h.history_graph().len(), 4);

        let json = serde_json::to_string(&h.snapshot_state()).unwrap();
        let restored: HistorySnapshot = serde_json::from_str(&json).unwrap();
        let mut fresh = CommandHistory::new(200);
        fresh.restore_state(restored);
        // The whole tree (both branches) survives, not just the linear path.
        assert_eq!(fresh.history_graph().len(), 4, "branch lost across save/load");
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

    /// Fold a freshly-issued command `new` into the current gesture's anchor
    /// command `last`, producing a single merged command that keeps `last`'s
    /// before-state and adopts `new`'s after-state. Returns `None` when the two
    /// commands are not the same-target value-replace kind (in which case the
    /// caller pushes `new` as its own undo step).
    ///
    /// Only same-target "replace the whole value" commands merge, so that a
    /// continuous drag (fill/stroke color picker, a streamed slider, …) collapses
    /// to one undo step spanning the whole gesture while distinct edits stay
    /// separate:
    ///
    /// - `UpdateNode` merges only with another `UpdateNode` targeting the **same**
    ///   node id (`new.id`); the merged `old` is the anchor's `old` (the state
    ///   before the gesture began) and the merged `new` is the incoming `new`.
    /// - `SetWidthProfiles`, `SetGuides`, `SetArtboards`, `ResizeCanvas`: whole-
    ///   document value replacements — keep `old` from the anchor, `new` from the
    ///   incoming.
    ///
    /// Everything else (adds, removes, reorders, grouping, layer moves, batches,
    /// mismatched variants, different node ids) returns `None`.
    pub fn coalesce(last: &Command, new: &Command) -> Option<Command> {
        match (last, new) {
            (
                Command::UpdateNode { old, new: last_new },
                Command::UpdateNode {
                    new: incoming_new, ..
                },
            ) if last_new.id == incoming_new.id => Some(Command::UpdateNode {
                old: old.clone(),
                new: incoming_new.clone(),
            }),
            (Command::SetWidthProfiles { old, .. }, Command::SetWidthProfiles { new, .. }) => {
                Some(Command::SetWidthProfiles {
                    old: old.clone(),
                    new: new.clone(),
                })
            }
            (Command::SetGuides { old, .. }, Command::SetGuides { new, .. }) => {
                Some(Command::SetGuides {
                    old: old.clone(),
                    new: new.clone(),
                })
            }
            (Command::SetArtboards { old, .. }, Command::SetArtboards { new, .. }) => {
                Some(Command::SetArtboards {
                    old: old.clone(),
                    new: new.clone(),
                })
            }
            (
                Command::ResizeCanvas {
                    old_width,
                    old_height,
                    ..
                },
                Command::ResizeCanvas {
                    new_width,
                    new_height,
                    ..
                },
            ) => Some(Command::ResizeCanvas {
                old_width: *old_width,
                old_height: *old_height,
                new_width: *new_width,
                new_height: *new_height,
            }),
            _ => None,
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
    /// Legacy flat undo history (path root→current, oldest→newest). Still written
    /// so older Photonic builds can open new files, and read to reconstruct a
    /// linear tree from files written before the edit-tree existed.
    #[serde(default)]
    pub undo_stack: Vec<Command>,
    /// Legacy flat redo history (primary future chain), same compatibility role.
    #[serde(default)]
    pub redo_stack: Vec<Command>,
    #[serde(default)]
    pub checkpoints: Vec<Checkpoint>,
    #[serde(default)]
    pub branches: std::collections::HashMap<String, Document>,
    /// Full edit tree (the source of truth when present). Absent in files written
    /// before branching history; those fall back to `undo_stack`/`redo_stack`.
    #[serde(default)]
    pub tree: Option<HistoryTree>,
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
/// One node in the [`CommandHistory`] edit tree. Represents the document state
/// reached by applying `command` to the parent's state (the root has no command
/// and is the oldest retained state). Undo moves toward the root; redo follows
/// `primary_child`. Undo-then-edit adds a *sibling* child (a new branch) rather
/// than discarding the old future — that is the undo-tree behaviour.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistNode {
    pub id: u64,
    #[serde(default)]
    pub parent: Option<u64>,
    /// Command transforming the parent's state into this node's. `None` at the root.
    #[serde(default)]
    pub command: Option<Command>,
    /// Child node ids in creation order — each a divergent future edit.
    #[serde(default)]
    pub children: Vec<u64>,
    /// Which child redo prefers (the most recently created or visited).
    #[serde(default)]
    pub primary_child: Option<u64>,
}

#[derive(Debug)]
pub struct CommandHistory {
    /// Edit tree keyed by node id; always contains the root (id of `root`).
    /// Replaces the old flat undo/redo stacks — the path root→`current` is the
    /// undo history and `current`'s primary-child chain is the redo history, but
    /// diverging branches are retained so undo-then-edit no longer loses the
    /// redo path.
    nodes: std::collections::HashMap<u64, HistNode>,
    /// Root (oldest retained) node id.
    root: u64,
    /// Node the document currently reflects (HEAD).
    current: u64,
    /// Next node id to allocate (monotonic; also the creation-order key).
    next_id: u64,
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
    /// A pointer gesture is open (set by [`begin_coalescing`]): mergeable
    /// same-target edits streamed through [`execute`] fold into the current
    /// gesture's anchor undo entry instead of pushing a new step, so one
    /// continuous drag becomes a single undo step (#182).
    coalescing: bool,
    /// Set once the first command of the current gesture has been pushed, so the
    /// anchor entry (`undo_stack.last()`) is only ever merged into within the
    /// gesture that created it — never into a step left over from before the
    /// gesture began.
    coalesce_started: bool,
}

impl Default for CommandHistory {
    fn default() -> Self {
        Self::new(200)
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
