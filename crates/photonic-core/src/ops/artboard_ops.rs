//! Undoable artboard duplication and movement planners.

use crate::{
    document::{ArtboardId, Document},
    export::node_world_bbox,
    history::Command,
    layer::LayerId,
    node::{NodeId, SceneNodeKind},
    ops::cloning::clone_subtrees,
    transform::Transform,
};
use std::collections::HashSet;
use uuid::Uuid;

/// Root nodes (top-level layer nodes) whose largest-overlap owner is `artboard_id`.
///
/// A root is owned by the artboard containing the greatest portion of its
/// world-space bounding box. Text uses its glyph-derived local bounds (via
/// `node_world_bbox`) rather than being treated as a geometry-less node.
pub fn owned_root_nodes(doc: &Document, artboard_id: ArtboardId) -> Vec<(LayerId, NodeId)> {
    let mut owned = Vec::new();

    for layer_id in &doc.layer_order {
        let Some(layer) = doc.layers.get(layer_id) else {
            continue;
        };
        for root_id in &layer.node_ids {
            let Some(node) = doc.nodes.get(root_id) else {
                continue;
            };
            let Some(bbox) = node_world_bbox(node, doc) else {
                continue;
            };
            let Some(owner) = doc.artboard_for_rect(bbox.x0, bbox.y0, bbox.x1, bbox.y1) else {
                continue;
            };
            if owner.id == artboard_id {
                owned.push((*layer_id, *root_id));
            }
        }
    }

    owned
}

/// Return a root and every descendant it moves in world space, once each.
fn subtree_node_ids(doc: &Document, root_id: NodeId) -> Vec<NodeId> {
    let mut ids = Vec::new();
    let mut stack = vec![root_id];
    let mut seen = HashSet::new();
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        let Some(node) = doc.nodes.get(&id) else {
            continue;
        };
        ids.push(id);
        if let SceneNodeKind::Group(group) = &node.kind {
            stack.extend(group.children.iter().rev().copied());
        }
    }
    ids
}

/// Build one undoable command that duplicates an artboard and all root subtrees it owns.
pub fn plan_duplicate_artboard(
    doc: &Document,
    artboard_id: ArtboardId,
    dx: f64,
    dy: f64,
    new_name: Option<String>,
) -> Option<(Command, ArtboardId)> {
    let original = doc
        .artboards
        .iter()
        .find(|artboard| artboard.id == artboard_id)?;
    let mut new_artboards = doc.artboards.clone();
    let mut new_artboard = original.clone();
    new_artboard.id = Uuid::new_v4();
    new_artboard.name = new_name.unwrap_or_else(|| format!("{} copy", original.name));
    new_artboard.x += dx;
    new_artboard.y += dy;
    let new_artboard_id = new_artboard.id;
    new_artboards.push(new_artboard);

    let owned = owned_root_nodes(doc, artboard_id);
    let mut commands = vec![Command::SetArtboards {
        old: doc.artboards.clone(),
        new: new_artboards,
    }];

    for layer_id in &doc.layer_order {
        let roots: Vec<NodeId> = owned
            .iter()
            .filter_map(|(owned_layer_id, root_id)| {
                (*owned_layer_id == *layer_id).then_some(*root_id)
            })
            .collect();
        if roots.is_empty() {
            continue;
        }

        // `clone_subtrees` offsets each subtree ROOT by (dx, dy) — descendants keep
        // their own transforms and follow the root, because a node's world position
        // is `parent_world * node.transform` (see `export::…` world composition).
        // Translating every node would double-count the offset once per ancestor and
        // scatter grouped content, so we translate roots only.
        let (new_roots, nodes) = clone_subtrees(&doc.nodes, &roots, *layer_id, dx, dy);
        if !new_roots.is_empty() {
            commands.push(Command::AddSubtree {
                layer_id: *layer_id,
                roots: new_roots,
                nodes,
            });
        }
    }

    Some((Command::Batch(commands), new_artboard_id))
}

/// Build one undoable command that moves an artboard and all root subtrees it owns.
pub fn plan_move_artboard(
    doc: &Document,
    artboard_id: ArtboardId,
    dx: f64,
    dy: f64,
) -> Option<Command> {
    if dx == 0.0 && dy == 0.0 {
        return None;
    }

    if !doc
        .artboards
        .iter()
        .any(|artboard| artboard.id == artboard_id)
    {
        return None;
    }

    let mut new_artboards = doc.artboards.clone();
    let artboard = new_artboards
        .iter_mut()
        .find(|artboard| artboard.id == artboard_id)
        .expect("artboard existence was checked above");
    artboard.x += dx;
    artboard.y += dy;

    let mut commands = vec![Command::SetArtboards {
        old: doc.artboards.clone(),
        new: new_artboards,
    }];
    let gradient_translate = Transform::translate(dx, dy);
    // Renderers flatten groups to their leaves, so each subtree node needs its
    // own geometry offset. User-space gradient coordinates are document-anchored
    // and must follow the same translation.
    for (_, root_id) in owned_root_nodes(doc, artboard_id) {
        for node_id in subtree_node_ids(doc, root_id) {
            let Some(old) = doc.nodes.get(&node_id) else {
                continue;
            };
            let mut new = old.clone();
            new.transform_user_space_gradients(&gradient_translate);
            new.transform.matrix[4] += dx;
            new.transform.matrix[5] += dy;
            commands.push(Command::UpdateNode {
                old: old.clone(),
                new,
            });
        }
    }

    Some(Command::Batch(commands))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        color::Color,
        document::Artboard,
        history::CommandHistory,
        node::{GroupNode, PathNode, SceneNode, SceneNodeKind, TextNode},
        path::PathData,
        style::{Fill, FillKind, Gradient, GradientStop},
        transform::Transform,
    };

    fn two_artboard_document() -> (Document, ArtboardId, ArtboardId, NodeId, NodeId) {
        let mut doc = Document::new("test", 100.0, 100.0);
        let artboard_a = doc.artboards[0].id;
        let artboard_b = doc.add_artboard(Artboard::new("Board B", 200.0, 0.0, 100.0, 100.0));
        let layer_id = doc.active_layer_id.expect("default layer");

        let node_a = SceneNode::new(
            "A path",
            layer_id,
            SceneNodeKind::Path(PathNode::new(PathData::rect(10.0, 10.0, 20.0, 20.0))),
        );
        let node_b = SceneNode::new(
            "B path",
            layer_id,
            SceneNodeKind::Path(PathNode::new(PathData::rect(210.0, 10.0, 20.0, 20.0))),
        );
        let node_a_id = node_a.id;
        let node_b_id = node_b.id;
        doc.nodes.insert(node_a_id, node_a);
        doc.nodes.insert(node_b_id, node_b);
        doc.layers
            .get_mut(&layer_id)
            .expect("default layer")
            .node_ids
            .extend([node_a_id, node_b_id]);

        (doc, artboard_a, artboard_b, node_a_id, node_b_id)
    }

    fn grouped_document() -> (Document, ArtboardId, NodeId, NodeId) {
        let mut doc = Document::new("test", 100.0, 100.0);
        let artboard_id = doc.artboards[0].id;
        let layer_id = doc.active_layer_id.expect("default layer");
        let child = SceneNode::new(
            "child",
            layer_id,
            SceneNodeKind::Path(PathNode::new(PathData::rect(10.0, 10.0, 20.0, 20.0))),
        );
        let child_id = child.id;
        let group = SceneNode::new(
            "group",
            layer_id,
            SceneNodeKind::Group(GroupNode {
                children: vec![child_id],
                clip_children: false,
                clip_node_id: None,
                blend_spine_id: None,
                live_boolean: None,
            }),
        );
        let group_id = group.id;
        doc.nodes.insert(child_id, child);
        doc.nodes.insert(group_id, group);
        doc.layers
            .get_mut(&layer_id)
            .expect("default layer")
            .node_ids
            .push(group_id);

        (doc, artboard_id, group_id, child_id)
    }

    fn grouped_gradient_document() -> (Document, ArtboardId, NodeId, NodeId) {
        let mut doc = Document::new("test", 100.0, 100.0);
        let artboard_id = doc.artboards[0].id;
        let layer_id = doc.active_layer_id.expect("default layer");
        let gradient = Gradient::linear(
            10.0,
            20.0,
            30.0,
            20.0,
            vec![
                GradientStop::new(0.0, Color::BLACK),
                GradientStop::new(1.0, Color::WHITE),
            ],
        );
        let mut child = SceneNode::new(
            "QR modules",
            layer_id,
            SceneNodeKind::Path(
                PathNode::new(PathData::rect(0.0, 0.0, 20.0, 10.0))
                    .with_fill(Fill::gradient(gradient)),
            ),
        );
        child.transform = Transform::translate(10.0, 20.0);
        let child_id = child.id;
        let group = SceneNode::new(
            "QR code",
            layer_id,
            SceneNodeKind::Group(GroupNode {
                children: vec![child_id],
                clip_children: false,
                clip_node_id: None,
                blend_spine_id: None,
                live_boolean: None,
            }),
        );
        let group_id = group.id;
        doc.nodes.insert(child_id, child);
        doc.nodes.insert(group_id, group);
        doc.layers
            .get_mut(&layer_id)
            .expect("default layer")
            .node_ids
            .push(group_id);

        (doc, artboard_id, group_id, child_id)
    }

    fn text_and_gradient_document() -> (Document, ArtboardId, NodeId, NodeId) {
        let mut doc = Document::new("test", 100.0, 100.0);
        let artboard_id = doc.artboards[0].id;
        let layer_id = doc.active_layer_id.expect("default layer");

        let mut text = SceneNode::new(
            "Name",
            layer_id,
            SceneNodeKind::Text(TextNode::new("Ada Lovelace")),
        );
        text.transform = Transform::translate(10.0, 25.0);
        let text_id = text.id;

        let gradient = Gradient::linear(
            10.0,
            10.0,
            30.0,
            10.0,
            vec![
                GradientStop::new(0.0, Color::BLACK),
                GradientStop::new(1.0, Color::WHITE),
            ],
        );
        let path = SceneNode::new(
            "Gradient card",
            layer_id,
            SceneNodeKind::Path(
                PathNode::new(PathData::rect(10.0, 10.0, 20.0, 10.0))
                    .with_fill(Fill::gradient(gradient)),
            ),
        );
        let path_id = path.id;

        doc.nodes.insert(text_id, text);
        doc.nodes.insert(path_id, path);
        doc.layers
            .get_mut(&layer_id)
            .expect("default layer")
            .node_ids
            .extend([text_id, path_id]);
        (doc, artboard_id, text_id, path_id)
    }

    fn gradient_coords(doc: &Document, node_id: NodeId) -> Vec<f64> {
        let SceneNodeKind::Path(path) = &doc.nodes[&node_id].kind else {
            panic!("expected path")
        };
        let FillKind::Gradient(gradient) = &path.fill.kind else {
            panic!("expected gradient fill")
        };
        gradient.coords.clone()
    }

    #[test]
    fn owned_roots_only_include_their_artboard_content() {
        let (doc, artboard_a, _, node_a_id, _) = two_artboard_document();

        let owned = owned_root_nodes(&doc, artboard_a);

        assert_eq!(owned.len(), 1);
        assert_eq!(owned[0].1, node_a_id);
    }

    #[test]
    fn duplicate_artboard_copies_content_in_one_undo_step() {
        let (mut doc, artboard_a, _, node_a_id, node_b_id) = two_artboard_document();
        let original_bbox = node_world_bbox(&doc.nodes[&node_a_id], &doc).expect("path bbox");
        let (command, new_artboard_id) =
            plan_duplicate_artboard(&doc, artboard_a, 300.0, 0.0, None).expect("known artboard");
        let mut history = CommandHistory::new(200);

        history.execute_discrete(command, &mut doc);

        assert_eq!(doc.artboards.len(), 3);
        let new_artboard = doc
            .artboards
            .iter()
            .find(|artboard| artboard.id == new_artboard_id)
            .expect("duplicated artboard");
        assert_eq!(new_artboard.name, "Artboard 1 copy");
        assert_eq!(new_artboard.rect(), (300.0, 0.0, 400.0, 100.0));
        let cloned_id = doc
            .nodes
            .keys()
            .copied()
            .find(|id| *id != node_a_id && *id != node_b_id)
            .expect("cloned node");
        assert_ne!(cloned_id, node_a_id);
        let cloned_bbox = node_world_bbox(&doc.nodes[&cloned_id], &doc).expect("cloned path bbox");
        assert!(
            cloned_bbox.x0 >= new_artboard.x
                && cloned_bbox.x1 <= new_artboard.x + new_artboard.width
        );
        assert_eq!(
            node_world_bbox(&doc.nodes[&node_a_id], &doc),
            Some(original_bbox)
        );

        assert!(history.undo(&mut doc));
        assert_eq!(doc.artboards.len(), 2);
        assert!(!doc.nodes.contains_key(&cloned_id));
        assert_eq!(history.undo_depth(), 0);
    }

    #[test]
    fn duplicate_artboard_includes_text_and_offsets_user_space_gradient() {
        let (mut doc, artboard_id, text_id, path_id) = text_and_gradient_document();
        let (command, copied_artboard_id) =
            plan_duplicate_artboard(&doc, artboard_id, 0.0, 700.0, None).expect("known board");
        let mut history = CommandHistory::new(200);

        history.execute_discrete(command, &mut doc);

        assert_eq!(history.undo_depth(), 1, "duplicate stays one undo entry");
        assert_eq!(doc.artboards.len(), 2);
        assert_eq!(
            doc.artboards
                .iter()
                .find(|artboard| artboard.id == copied_artboard_id)
                .expect("copy")
                .y,
            700.0
        );
        let copied_text = doc
            .nodes
            .values()
            .find(|node| node.id != text_id && matches!(node.kind, SceneNodeKind::Text(_)))
            .expect("text is owned and cloned");
        assert_eq!(copied_text.transform.matrix[4], 10.0);
        assert_eq!(copied_text.transform.matrix[5], 725.0);
        let copied_path_id = doc
            .nodes
            .keys()
            .copied()
            .find(|id| *id != path_id && matches!(doc.nodes[id].kind, SceneNodeKind::Path(_)))
            .expect("gradient path is cloned");
        assert_eq!(
            gradient_coords(&doc, copied_path_id),
            vec![10.0, 710.0, 30.0, 710.0]
        );
    }

    #[test]
    fn move_artboard_moves_text_and_user_space_gradient() {
        let (mut doc, artboard_id, text_id, path_id) = text_and_gradient_document();
        let command = plan_move_artboard(&doc, artboard_id, 40.0, 700.0).expect("move command");
        let mut history = CommandHistory::new(200);

        history.execute_discrete(command, &mut doc);

        assert_eq!(history.undo_depth(), 1, "move stays one undo entry");
        assert_eq!(doc.nodes[&text_id].transform.matrix[4], 50.0);
        assert_eq!(doc.nodes[&text_id].transform.matrix[5], 725.0);
        assert_eq!(
            gradient_coords(&doc, path_id),
            vec![50.0, 710.0, 70.0, 710.0]
        );
    }

    #[test]
    fn duplicate_artboard_offsets_group_descendant_geometry_and_user_space_gradient() {
        let (mut doc, artboard_id, group_id, child_id) = grouped_gradient_document();
        let (command, _) =
            plan_duplicate_artboard(&doc, artboard_id, 40.0, 700.0, None).expect("known artboard");
        let mut history = CommandHistory::new(200);
        history.execute_discrete(command, &mut doc);

        let cloned_group = doc
            .nodes
            .values()
            .find(|node| node.id != group_id && matches!(node.kind, SceneNodeKind::Group(_)))
            .expect("cloned group");
        let SceneNodeKind::Group(cloned_group_data) = &cloned_group.kind else {
            unreachable!()
        };
        let cloned_child_id = *cloned_group_data.children.first().expect("cloned child");
        assert_ne!(cloned_child_id, child_id, "descendant receives a fresh id");
        assert_eq!(cloned_group.transform.matrix[4], 40.0);
        assert_eq!(cloned_group.transform.matrix[5], 700.0);
        assert_eq!(doc.nodes[&cloned_child_id].transform.matrix[4], 50.0);
        assert_eq!(doc.nodes[&cloned_child_id].transform.matrix[5], 720.0);
        assert_eq!(
            gradient_coords(&doc, cloned_child_id),
            vec![50.0, 720.0, 70.0, 720.0]
        );
    }

    #[test]
    fn move_artboard_moves_only_owned_content_in_one_undo_step() {
        let (mut doc, artboard_a, artboard_b, node_a_id, node_b_id) = two_artboard_document();
        let original_a_bbox = node_world_bbox(&doc.nodes[&node_a_id], &doc).expect("A path bbox");
        let original_b_bbox = node_world_bbox(&doc.nodes[&node_b_id], &doc).expect("B path bbox");
        let command = plan_move_artboard(&doc, artboard_a, 500.0, 0.0).expect("move command");
        let mut history = CommandHistory::new(200);

        history.execute_discrete(command, &mut doc);

        let moved_artboard = doc
            .artboards
            .iter()
            .find(|artboard| artboard.id == artboard_a)
            .expect("artboard A");
        assert_eq!(moved_artboard.rect(), (500.0, 0.0, 600.0, 100.0));
        let moved_a_bbox = node_world_bbox(&doc.nodes[&node_a_id], &doc).expect("moved A bbox");
        assert_eq!(moved_a_bbox.x0, original_a_bbox.x0 + 500.0);
        assert_eq!(
            node_world_bbox(&doc.nodes[&node_b_id], &doc),
            Some(original_b_bbox)
        );
        assert_eq!(
            doc.artboards
                .iter()
                .find(|artboard| artboard.id == artboard_b)
                .expect("artboard B")
                .rect(),
            (200.0, 0.0, 300.0, 100.0)
        );

        assert!(history.undo(&mut doc));
        assert_eq!(
            doc.artboards
                .iter()
                .find(|artboard| artboard.id == artboard_a)
                .expect("artboard A")
                .rect(),
            (0.0, 0.0, 100.0, 100.0)
        );
        assert_eq!(
            node_world_bbox(&doc.nodes[&node_a_id], &doc),
            Some(original_a_bbox)
        );
        assert_eq!(history.undo_depth(), 0);
    }

    #[test]
    fn move_artboard_offsets_group_descendants() {
        let (mut doc, artboard_id, group_id, child_id) = grouped_document();
        let command = plan_move_artboard(&doc, artboard_id, 50.0, 25.0).expect("move command");

        let Command::Batch(commands) = &command else {
            panic!("move should be a batch");
        };
        let updated_ids: Vec<NodeId> = commands
            .iter()
            .filter_map(|command| match command {
                Command::UpdateNode { new, .. } => Some(new.id),
                _ => None,
            })
            .collect();
        assert_eq!(updated_ids, vec![group_id, child_id]);

        let mut history = CommandHistory::new(200);
        history.execute_discrete(command, &mut doc);

        assert_eq!(doc.nodes[&group_id].transform.matrix[4], 50.0);
        assert_eq!(doc.nodes[&group_id].transform.matrix[5], 25.0);
        assert_eq!(doc.nodes[&child_id].transform.matrix[4], 50.0);
        assert_eq!(doc.nodes[&child_id].transform.matrix[5], 25.0);
    }

    #[test]
    fn moving_by_zero_is_not_planned() {
        let (doc, artboard_a, _, _, _) = two_artboard_document();

        assert!(plan_move_artboard(&doc, artboard_a, 0.0, 0.0).is_none());
    }
}
