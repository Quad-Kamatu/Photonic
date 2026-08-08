use crate::protocol::{CreateVectorsFromCssArgs, ToolResult};
use crate::server::AppState;
use photonic_core::{
    css_vectors::{compile_css_vectors, CssOrigin, CssVectorNode, CssViewport, CONTRACT_VERSION},
    history::Command,
    node::{GroupNode, PathNode, SceneNode, SceneNodeKind},
};
use uuid::Uuid;

/// Transport adapter for the reusable core compiler.  It deliberately builds
/// the complete subtree before taking document/history locks so parser or
/// lowering errors cannot leave partial artwork behind.
pub async fn create_vectors_from_css(
    state: &AppState,
    args: CreateVectorsFromCssArgs,
) -> ToolResult {
    let (canvas_w, canvas_h, active_layer) = {
        let doc = state.document.lock().await;
        (doc.width, doc.height, doc.active_layer_id)
    };
    let viewport = args
        .viewport
        .as_ref()
        .map(|v| CssViewport {
            width: v.width,
            height: v.height,
        })
        .unwrap_or(CssViewport {
            width: canvas_w,
            height: canvas_h,
        });
    let origin = args
        .origin
        .as_ref()
        .map(|p| CssOrigin { x: p.x, y: p.y })
        .unwrap_or(CssOrigin { x: 0.0, y: 0.0 });
    let plan = match compile_css_vectors(
        &args.css,
        args.selector.as_deref(),
        origin,
        viewport,
        args.strict,
    ) {
        Ok(plan) => plan,
        Err(diagnostics) => return ToolResult::error("CSS conversion rejected").with_data(
            serde_json::json!({ "diagnostics": diagnostics, "contract_version": CONTRACT_VERSION }),
        ),
    };
    let layer_id = args.layer_id.or(active_layer);
    let Some(layer_id) = layer_id else {
        return ToolResult::error("Document has no active layer");
    };
    {
        let doc = state.document.lock().await;
        let Some(layer) = doc.layers.get(&layer_id) else {
            return ToolResult::error("layer_id does not exist");
        };
        if layer.locked {
            return ToolResult::error("destination layer is locked");
        }
    }
    let mut nodes = Vec::new();
    let mut roots = Vec::new();
    for (i, root) in plan.roots.iter().enumerate() {
        let id = lower(root, layer_id, &mut nodes);
        // A caller-supplied root name applies only to a single selected root.
        if i == 0 {
            if let Some(name) = &args.group_name {
                if let Some(n) = nodes.iter_mut().find(|n: &&mut SceneNode| n.id == id) {
                    n.name = name.clone();
                }
            }
        }
        if let Some(n) = nodes.iter_mut().find(|n: &&mut SceneNode| n.id == id) {
            n.prompt_history.push(format!(
                "css-vector-v{} fingerprint={} css={}",
                CONTRACT_VERSION, plan.fingerprint, args.css
            ));
        }
        roots.push(id);
    }
    let created: Vec<Uuid> = nodes.iter().map(|n| n.id).collect();
    let (groups, paths, segments) = counts(&nodes);
    let data = serde_json::json!({
        "root_node_ids": roots, "created_node_ids": created,
        "bounds": {"x":plan.bounds.0,"y":plan.bounds.1,"width":plan.bounds.2,"height":plan.bounds.3},
        "node_counts": {"groups":groups,"paths":paths,"segments":segments},
        "resolved_viewport": {"width":viewport.width,"height":viewport.height},
        "diagnostics": plan.diagnostics, "source_fingerprint": plan.fingerprint,
        "contract_version": CONTRACT_VERSION, "dry_run": args.dry_run,
    });
    if args.dry_run {
        return ToolResult::text("CSS vector conversion plan").with_data(data);
    }
    // AddSubtree is self-contained and is one history edge: undo/redo removes
    // or restores every group/path as a single atomic operation.
    let cmd = Command::AddSubtree {
        layer_id,
        roots,
        nodes,
    };
    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    history.execute_discrete(cmd, &mut doc);
    ToolResult::text("Created editable vectors from CSS").with_data(data)
}

fn lower(input: &CssVectorNode, layer_id: Uuid, output: &mut Vec<SceneNode>) -> Uuid {
    match input {
        CssVectorNode::Path(p) => {
            let mut path = PathNode::new(p.path.clone());
            path.fill = p.fill.clone();
            path.stroke = p.stroke.clone();
            let mut node = SceneNode::new(&p.name, layer_id, SceneNodeKind::Path(path));
            node.opacity = p.opacity;
            // Persist source inspection information without introducing a new
            // opaque scene-graph field; prompts are intentionally non-printing.
            node.tags.push("css-vector-v1".into());
            let id = node.id;
            output.push(node);
            id
        }
        CssVectorNode::Group(g) => {
            let mut group = GroupNode::new();
            for child in &g.children {
                group.children.push(lower(child, layer_id, output));
            }
            let mut node = SceneNode::new(&g.name, layer_id, SceneNodeKind::Group(group));
            node.opacity = g.opacity;
            node.tags.push("css-vector-v1".into());
            node.prompt_history
                .push(format!("css-vector-v1 selector={}", g.provenance));
            let id = node.id;
            output.push(node);
            id
        }
    }
}

fn counts(nodes: &[SceneNode]) -> (usize, usize, usize) {
    let groups = nodes
        .iter()
        .filter(|n| matches!(n.kind, SceneNodeKind::Group(_)))
        .count();
    let paths: Vec<_> = nodes
        .iter()
        .filter_map(|n| {
            if let SceneNodeKind::Path(p) = &n.kind {
                Some(p)
            } else {
                None
            }
        })
        .collect();
    // Segment count is stable and intentionally approximate only at this
    // boundary: it counts exported path elements rather than renderer tessels.
    let segments = paths
        .iter()
        .map(|p| p.path_data.as_svg().matches(char::is_alphabetic).count())
        .sum();
    (groups, paths.len(), segments)
}
