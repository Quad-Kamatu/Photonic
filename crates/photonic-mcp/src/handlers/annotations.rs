use crate::protocol::{AddAnnotationArgs, ListAnnotationsArgs, ResolveAnnotationArgs, ToolResult};
use crate::server::AppState;
use photonic_core::annotation::Annotation;

pub async fn add_annotation(state: &AppState, args: AddAnnotationArgs) -> ToolResult {
    tracing::debug!("tool: add_annotation (node={:?})", args.node_id);

    let text = args.text.trim().to_string();
    if text.is_empty() {
        return ToolResult::error("text must be non-empty");
    }

    // Validate node_id if provided
    if let Some(nid) = args.node_id {
        let doc = state.document.lock().await;
        if doc.get_node(&nid).is_none() {
            return ToolResult::error(format!("Node {} not found in document", nid));
        }
    }

    let ann = Annotation::new(args.node_id, text, args.author);
    let ann_id = ann.id;

    let mut doc = state.document.lock().await;
    doc.add_annotation(ann);

    ToolResult::text(format!("Annotation {} added", ann_id)).with_data(serde_json::json!({
        "annotation_id": ann_id,
    }))
}

pub async fn list_annotations(state: &AppState, args: ListAnnotationsArgs) -> ToolResult {
    tracing::debug!("tool: list_annotations");

    let include_resolved = args.include_resolved.unwrap_or(false);
    let doc = state.document.lock().await;

    let mut annotations: Vec<&photonic_core::annotation::Annotation> = doc
        .annotations
        .values()
        .filter(|a| {
            if !include_resolved && a.resolved {
                return false;
            }
            if let Some(nid) = args.node_id {
                return a.node_id == Some(nid);
            }
            true
        })
        .collect();

    // Sort by created_at ascending (ISO 8601 strings sort lexicographically)
    annotations.sort_by(|a, b| a.created_at.cmp(&b.created_at));

    let items: Vec<serde_json::Value> = annotations
        .iter()
        .map(|a| {
            serde_json::json!({
                "annotation_id": a.id,
                "node_id":       a.node_id,
                "text":          a.text,
                "resolved":      a.resolved,
                "author":        a.author,
                "created_at":    a.created_at,
            })
        })
        .collect();

    let total = items.len();
    ToolResult::text(format!("{} annotation(s)", total)).with_data(serde_json::json!({
        "annotations": items,
        "total": total,
    }))
}

pub async fn resolve_annotation(state: &AppState, args: ResolveAnnotationArgs) -> ToolResult {
    tracing::debug!("tool: resolve_annotation (id={})", args.annotation_id);

    let mut doc = state.document.lock().await;
    if doc.resolve_annotation(&args.annotation_id) {
        ToolResult::text(format!("Annotation {} resolved", args.annotation_id)).with_data(
            serde_json::json!({
                "ok": true,
                "annotation_id": args.annotation_id,
            }),
        )
    } else {
        ToolResult::error(format!("Annotation {} not found", args.annotation_id))
    }
}
