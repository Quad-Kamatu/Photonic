//! Artboard management tools.
//!
//! Artboards are named crop/export rectangles in the document's shared
//! coordinate space (`Document::artboards`). The GUI lets the user draw, move,
//! rename, resize and delete them; these tools expose the same operations to
//! agents. All geometry edits go through a single `SetArtboards` command (the
//! whole list is snapshotted for self-contained undo, mirroring the GUI), so
//! every change is one undoable step.

use crate::protocol::*;
use crate::server::AppState;
use photonic_core::{history::Command, Artboard};
use serde_json::json;

/// Serialize one artboard for tool output.
fn artboard_json(a: &Artboard, active: bool) -> serde_json::Value {
    json!({
        "id": a.id,
        "name": a.name,
        "x": a.x,
        "y": a.y,
        "width": a.width,
        "height": a.height,
        "active": active,
    })
}

/// List every artboard in the document, marking the active one.
pub async fn list_artboards(state: &AppState, _args: ListArtboardsArgs) -> ToolResult {
    tracing::debug!("tool: list_artboards");
    let doc = state.document.lock().await;
    let active = doc.active_artboard;
    let boards: Vec<serde_json::Value> = doc
        .artboards
        .iter()
        .map(|a| artboard_json(a, Some(a.id) == active))
        .collect();
    let n = boards.len();
    ToolResult::text(format!(
        "{} artboard{}:\n{}",
        n,
        if n == 1 { "" } else { "s" },
        serde_json::to_string_pretty(&boards).unwrap_or_default()
    ))
    .with_data(json!({ "artboards": boards, "active_artboard": active }))
}

/// Add a new artboard and make it active. One undoable step.
pub async fn add_artboard(state: &AppState, args: AddArtboardArgs) -> ToolResult {
    tracing::debug!("tool: add_artboard");
    if args.width <= 0.0 || args.height <= 0.0 {
        return ToolResult::error("width and height must be > 0");
    }
    let mut doc = state.document.lock().await;
    let old = doc.artboards.clone();
    let name = args
        .name
        .clone()
        .unwrap_or_else(|| format!("Artboard {}", old.len() + 1));
    let ab = Artboard::new(name.clone(), args.x, args.y, args.width, args.height);
    let new_id = ab.id;
    let mut new = old.clone();
    new.push(ab);

    let mut history = state.history.lock().await;
    history.execute_discrete(Command::SetArtboards { old, new }, &mut doc);
    // `SetArtboards` keeps the previously-active board; make the new one active.
    doc.active_artboard = Some(new_id);

    ToolResult::text(format!(
        "Added artboard '{}' at ({:.1}, {:.1}), {:.0}×{:.0}",
        name, args.x, args.y, args.width, args.height
    ))
    .with_data(json!({ "artboard_id": new_id }))
}

/// Edit an artboard's name/position/size. Only provided fields change.
pub async fn update_artboard(state: &AppState, args: UpdateArtboardArgs) -> ToolResult {
    tracing::debug!("tool: update_artboard");
    if matches!(args.width, Some(w) if w <= 0.0) {
        return ToolResult::error("width must be > 0");
    }
    if matches!(args.height, Some(h) if h <= 0.0) {
        return ToolResult::error("height must be > 0");
    }
    let mut doc = state.document.lock().await;
    let old = doc.artboards.clone();
    let mut new = old.clone();
    let Some(ab) = new.iter_mut().find(|a| a.id == args.artboard_id) else {
        return ToolResult::error(format!("Artboard {} not found", args.artboard_id));
    };
    if let Some(n) = args.name.clone() {
        ab.name = n;
    }
    if let Some(v) = args.x {
        ab.x = v;
    }
    if let Some(v) = args.y {
        ab.y = v;
    }
    if let Some(v) = args.width {
        ab.width = v;
    }
    if let Some(v) = args.height {
        ab.height = v;
    }
    if new == old {
        return ToolResult::text("No changes — artboard already matches");
    }
    let summary = new
        .iter()
        .find(|a| a.id == args.artboard_id)
        .map(|a| {
            format!(
                "'{}' → ({:.1}, {:.1}), {:.0}×{:.0}",
                a.name, a.x, a.y, a.width, a.height
            )
        })
        .unwrap_or_default();

    let mut history = state.history.lock().await;
    history.execute_discrete(Command::SetArtboards { old, new }, &mut doc);

    ToolResult::text(format!("Updated artboard {summary}"))
        .with_data(json!({ "artboard_id": args.artboard_id }))
}

/// Remove an artboard. Refuses to remove the last remaining one.
pub async fn remove_artboard(state: &AppState, args: RemoveArtboardArgs) -> ToolResult {
    tracing::debug!("tool: remove_artboard");
    let mut doc = state.document.lock().await;
    if doc.artboards.len() <= 1 {
        return ToolResult::error("Cannot remove the last artboard — a document needs at least one");
    }
    let old = doc.artboards.clone();
    if !old.iter().any(|a| a.id == args.artboard_id) {
        return ToolResult::error(format!("Artboard {} not found", args.artboard_id));
    }
    let new: Vec<Artboard> = old
        .iter()
        .filter(|a| a.id != args.artboard_id)
        .cloned()
        .collect();

    let mut history = state.history.lock().await;
    history.execute_discrete(Command::SetArtboards { old, new }, &mut doc);
    // `SetArtboards` re-points `active_artboard` to the first board if the
    // removed one was active.

    ToolResult::text(format!("Removed artboard {}", args.artboard_id))
        .with_data(json!({ "removed": args.artboard_id, "active_artboard": doc.active_artboard }))
}

/// Make an artboard the active one (soft view state — not an undo step).
pub async fn set_active_artboard(state: &AppState, args: SetActiveArtboardArgs) -> ToolResult {
    tracing::debug!("tool: set_active_artboard");
    let mut doc = state.document.lock().await;
    if !doc.artboards.iter().any(|a| a.id == args.artboard_id) {
        return ToolResult::error(format!("Artboard {} not found", args.artboard_id));
    }
    doc.active_artboard = Some(args.artboard_id);
    ToolResult::text(format!("Active artboard set to {}", args.artboard_id))
        .with_data(json!({ "active_artboard": args.artboard_id }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::{AppState, McpServerConfig};
    use photonic_core::{AuditLog, Document};
    use std::sync::{Arc, Mutex as StdMutex};
    use tokio::sync::Mutex;

    fn test_state() -> AppState {
        let (tx, _rx) = std::sync::mpsc::channel();
        AppState {
            document: Arc::new(Mutex::new(Document::new("t", 200.0, 100.0))),
            history: Arc::new(Mutex::new(photonic_core::history::CommandHistory::new(100))),
            capture_tx: Arc::new(StdMutex::new(tx)),
            config: McpServerConfig::default(),
            audit_log: Arc::new(StdMutex::new(AuditLog::new())),
            clipboard_ring: Arc::new(crate::handlers::clipboard::new_clipboard_ring()),
        }
    }

    async fn count(state: &AppState) -> usize {
        state.document.lock().await.artboards.len()
    }

    #[tokio::test]
    async fn add_update_remove_roundtrip() {
        let state = test_state();
        let start = count(&state).await; // Document::new seeds one artboard

        // Add
        let r = add_artboard(
            &state,
            AddArtboardArgs {
                name: Some("Frame B".into()),
                x: 300.0,
                y: 0.0,
                width: 400.0,
                height: 300.0,
            },
        )
        .await;
        assert_ne!(r.is_error, Some(true), "add should succeed");
        assert_eq!(count(&state).await, start + 1);
        let new_id = {
            let doc = state.document.lock().await;
            // The newly added board is active and last.
            let ab = doc.artboards.last().unwrap();
            assert_eq!(ab.name, "Frame B");
            assert_eq!(doc.active_artboard, Some(ab.id));
            ab.id
        };

        // Update name + size
        let r = update_artboard(
            &state,
            UpdateArtboardArgs {
                artboard_id: new_id,
                name: Some("Frame B2".into()),
                x: None,
                y: None,
                width: Some(500.0),
                height: None,
            },
        )
        .await;
        assert_ne!(r.is_error, Some(true));
        {
            let doc = state.document.lock().await;
            let ab = doc.artboards.iter().find(|a| a.id == new_id).unwrap();
            assert_eq!(ab.name, "Frame B2");
            assert_eq!(ab.width, 500.0);
            assert_eq!(ab.height, 300.0, "height unchanged");
        }

        // Undo the update, then undo the add — proves both are undoable steps.
        {
            let mut doc = state.document.lock().await;
            let mut h = state.history.lock().await;
            assert!(h.undo(&mut doc)); // undo update
            assert_eq!(
                doc.artboards.iter().find(|a| a.id == new_id).unwrap().name,
                "Frame B"
            );
            assert!(h.undo(&mut doc)); // undo add
            assert_eq!(doc.artboards.len(), start);
        }
        // Redo both.
        {
            let mut doc = state.document.lock().await;
            let mut h = state.history.lock().await;
            assert!(h.redo(&mut doc));
            assert!(h.redo(&mut doc));
            assert_eq!(doc.artboards.len(), start + 1);
        }

        // Remove it.
        let r = remove_artboard(&state, RemoveArtboardArgs { artboard_id: new_id }).await;
        assert_ne!(r.is_error, Some(true));
        assert_eq!(count(&state).await, start);
    }

    #[tokio::test]
    async fn cannot_remove_last_artboard() {
        let state = test_state();
        let id = state.document.lock().await.artboards[0].id;
        let r = remove_artboard(&state, RemoveArtboardArgs { artboard_id: id }).await;
        assert_eq!(r.is_error, Some(true), "removing the last artboard must error");
        assert_eq!(count(&state).await, 1);
    }

    #[tokio::test]
    async fn rejects_nonpositive_size_and_unknown_id() {
        let state = test_state();
        let bad = add_artboard(
            &state,
            AddArtboardArgs { name: None, x: 0.0, y: 0.0, width: 0.0, height: 10.0 },
        )
        .await;
        assert_eq!(bad.is_error, Some(true));

        let missing = update_artboard(
            &state,
            UpdateArtboardArgs {
                artboard_id: uuid::Uuid::new_v4(),
                name: Some("x".into()),
                x: None,
                y: None,
                width: None,
                height: None,
            },
        )
        .await;
        assert_eq!(missing.is_error, Some(true));
    }
}
