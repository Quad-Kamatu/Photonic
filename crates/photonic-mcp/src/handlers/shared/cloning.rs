use photonic_core::node::{SceneNode, SceneNodeKind};

/// Deep-clone a node subtree rooted at `root_id`, remapping all IDs to fresh UUIDs.
///
/// Returns a flat `Vec<SceneNode>` in add-order: root first, then descendants (DFS).
/// The returned root node already has its `layer_id` set to `target_layer`.
/// An incremental translate of `(dx, dy)` is composed onto the root's existing transform.
pub(crate) fn clone_subtree(
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

            // The root's translate moves the complete subtree in world space.
            // Keep document-space gradient fills and stroke paints aligned on
            // every cloned descendant as well.
            if dx != 0.0 || dy != 0.0 {
                cloned.transform_user_space_gradients(&Transform::translate(dx, dy));
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
