//! Deep-cloning of node subtrees for copy/paste and duplicate.
//!
//! Copying "a group of paths or images" means copying a *subtree*: the group
//! root plus every descendant it references by id. A shallow `SceneNode::clone`
//! is not enough — the clone would still reference the **original** children by
//! id, so pasting it produces a group that shares (and corrupts) the source's
//! children, and pasting into another document produces an empty group because
//! those ids don't exist there.
//!
//! [`clone_subtree`] fixes this by assigning a fresh id to every node in the
//! subtree and remapping *every* intra-subtree id reference — group children,
//! clip and blend-spine references, threaded text frames, type-on-path and
//! area-type spines. References that point *outside* the subtree (e.g. a symbol
//! master) are deliberately left untouched.

use crate::node::{NodeId, SceneNode, SceneNodeKind};
use crate::{layer::LayerId, transform::Transform};
use std::collections::HashMap;

/// Deep-clone the subtree rooted at `root_id` from a node snapshot.
///
/// Returns the cloned nodes **root-first** (callers use `result[0].id` as the
/// new root). Every node gets a fresh id, is reparented to `target_layer`, and
/// receives the `(dx, dy)` paste offset. Returns an empty vec if `root_id` is
/// not present in `nodes`.
///
/// The `nodes` map may be a live `Document::nodes` or a detached clipboard
/// snapshot — cloning never touches the source, so cross-document paste works
/// even after the source document is closed.
pub fn clone_subtree(
    nodes: &HashMap<NodeId, SceneNode>,
    root_id: NodeId,
    target_layer: LayerId,
    dx: f64,
    dy: f64,
) -> Vec<SceneNode> {
    // Collect the subtree in draw order (root first, children in order).
    let mut order: Vec<NodeId> = Vec::new();
    let mut stack = vec![root_id];
    while let Some(id) = stack.pop() {
        if let Some(node) = nodes.get(&id) {
            order.push(id);
            if let SceneNodeKind::Group(g) = &node.kind {
                // Push in reverse so children pop in their stored order.
                for c in g.children.iter().rev() {
                    stack.push(*c);
                }
            }
        }
    }
    if order.is_empty() {
        return Vec::new();
    }

    // Fresh id for every node in the subtree.
    let id_map: HashMap<NodeId, NodeId> = order
        .iter()
        .map(|old| (*old, uuid::Uuid::new_v4()))
        .collect();
    // Remap an id if it belongs to the subtree; leave external references intact.
    let remap = |id: NodeId| id_map.get(&id).copied().unwrap_or(id);

    let mut out = Vec::with_capacity(order.len());
    for old_id in &order {
        let Some(src) = nodes.get(old_id) else {
            continue;
        };
        let mut node = src.clone();
        node.id = id_map[old_id];
        // The whole subtree belongs to the target layer now. Descendants are
        // reachable via their parent group's `children`, but keeping their
        // `layer_id` pointed at a (possibly foreign) source layer would leave a
        // dangling reference after a cross-document paste.
        node.layer_id = target_layer;
        // Renderers flatten groups to their leaves, so each cloned descendant
        // needs its own geometry offset. User-space gradients are
        // document-anchored and follow the same translation.
        if dx != 0.0 || dy != 0.0 {
            node.transform.matrix[4] += dx;
            node.transform.matrix[5] += dy;
            node.transform_user_space_gradients(&Transform::translate(dx, dy));
        }

        match &mut node.kind {
            SceneNodeKind::Group(g) => {
                g.children = g.children.iter().map(|c| remap(*c)).collect();
                g.clip_node_id = g.clip_node_id.map(remap);
                g.blend_spine_id = g.blend_spine_id.map(remap);
            }
            SceneNodeKind::Text(t) => {
                t.path_spine_id = t.path_spine_id.map(remap);
                t.area_path_id = t.area_path_id.map(remap);
                t.next_frame = t.next_frame.map(remap);
                t.prev_frame = t.prev_frame.map(remap);
            }
            _ => {}
        }
        // A symbol instance whose master was *also* copied should point at the
        // copied master; an instance of an un-copied master keeps its link.
        if let Some(sr) = node.symbol_ref {
            node.symbol_ref = Some(remap(sr));
        }

        out.push(node);
    }
    out
}

/// Deep-clone several sibling subtrees, filtering out any root that is already a
/// descendant of another root in `roots` (so selecting both a group and one of
/// its children never double-pastes the child). Returns `(new_root_ids, nodes)`
/// where `nodes` is every cloned node across all subtrees, root-first per tree.
pub fn clone_subtrees(
    nodes: &HashMap<NodeId, SceneNode>,
    roots: &[NodeId],
    target_layer: LayerId,
    dx: f64,
    dy: f64,
) -> (Vec<NodeId>, Vec<SceneNode>) {
    // Drop roots contained within another selected root's subtree.
    let mut descendants: std::collections::HashSet<NodeId> = std::collections::HashSet::new();
    for r in roots {
        collect_descendants(nodes, *r, &mut descendants);
    }
    let mut new_roots = Vec::new();
    let mut all_nodes = Vec::new();
    let mut seen: std::collections::HashSet<NodeId> = std::collections::HashSet::new();
    for r in roots {
        if descendants.contains(r) || !nodes.contains_key(r) || !seen.insert(*r) {
            continue;
        }
        let cloned = clone_subtree(nodes, *r, target_layer, dx, dy);
        if let Some(first) = cloned.first() {
            new_roots.push(first.id);
        }
        all_nodes.extend(cloned);
    }
    (new_roots, all_nodes)
}

/// Insert the descendants (excluding the root itself) of `root_id` into `out`.
fn collect_descendants(
    nodes: &HashMap<NodeId, SceneNode>,
    root_id: NodeId,
    out: &mut std::collections::HashSet<NodeId>,
) {
    if let Some(SceneNode {
        kind: SceneNodeKind::Group(g),
        ..
    }) = nodes.get(&root_id)
    {
        for c in &g.children {
            out.insert(*c);
            collect_descendants(nodes, *c, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::{GroupNode, PathNode};
    use crate::path::PathData;

    fn path_node(layer: LayerId) -> SceneNode {
        SceneNode::new(
            "p",
            layer,
            SceneNodeKind::Path(PathNode::new(PathData::rect(0.0, 0.0, 10.0, 10.0))),
        )
    }

    #[test]
    fn clones_group_with_fresh_ids_and_remapped_children() {
        let layer = uuid::Uuid::new_v4();
        let target = uuid::Uuid::new_v4();
        let c1 = path_node(layer);
        let c2 = path_node(layer);
        let (id1, id2) = (c1.id, c2.id);
        let group = SceneNode::new(
            "g",
            layer,
            SceneNodeKind::Group(GroupNode {
                children: vec![id1, id2],
                clip_children: false,
                clip_node_id: Some(id2),
                blend_spine_id: None,
                live_boolean: None,
            }),
        );
        let group_id = group.id;
        let mut map = HashMap::new();
        for n in [group, c1, c2] {
            map.insert(n.id, n);
        }

        let out = clone_subtree(&map, group_id, target, 5.0, 7.0);
        assert_eq!(out.len(), 3, "root + two children");

        // Every id is fresh.
        for n in &out {
            assert!(!map.contains_key(&n.id), "id was not remapped");
            assert_eq!(n.layer_id, target, "reparented to target layer");
        }

        // The cloned group references the cloned children (not the originals),
        // and its clip reference tracks the remap too.
        let SceneNodeKind::Group(g) = &out[0].kind else {
            panic!("root should be a group");
        };
        let cloned_child_ids: Vec<NodeId> = out[1..].iter().map(|n| n.id).collect();
        assert_eq!(g.children, cloned_child_ids);
        assert!(!g.children.contains(&id1) && !g.children.contains(&id2));
        assert_eq!(g.clip_node_id, Some(cloned_child_ids[1]));

        // Root offset applied.
        assert_eq!(out[0].transform.matrix[4], 5.0);
        assert_eq!(out[0].transform.matrix[5], 7.0);
    }

    #[test]
    fn clone_subtrees_drops_nested_selection() {
        let layer = uuid::Uuid::new_v4();
        let child = path_node(layer);
        let child_id = child.id;
        let group = SceneNode::new(
            "g",
            layer,
            SceneNodeKind::Group(GroupNode {
                children: vec![child_id],
                clip_children: false,
                clip_node_id: None,
                blend_spine_id: None,
                live_boolean: None,
            }),
        );
        let group_id = group.id;
        let mut map = HashMap::new();
        for n in [group, child] {
            map.insert(n.id, n);
        }
        // Select both the group and its child — the child must not paste twice.
        let (roots, nodes) = clone_subtrees(&map, &[group_id, child_id], layer, 0.0, 0.0);
        assert_eq!(roots.len(), 1, "nested child dropped as a root");
        assert_eq!(nodes.len(), 2, "group + its one child, once");
    }
}
