use crate::protocol::{ExportAuditLogArgs, ListAuditLogArgs, ToolResult};
use crate::server::AppState;

/// Return the most-recent N audit entries (default 50) as formatted JSON text.
pub async fn list_audit_log(state: &AppState, args: ListAuditLogArgs) -> ToolResult {
    let limit = args.limit.unwrap_or(50).min(1000);
    let (entries, total) = match state.audit_log.lock() {
        Ok(log) => {
            let entries: Vec<_> = log.recent(limit).into_iter().cloned().collect();
            let total = log.total_recorded();
            (entries, total)
        }
        Err(_) => return ToolResult::error("audit log lock poisoned"),
    };

    match serde_json::to_string_pretty(&entries) {
        Ok(json) => ToolResult::text(format!(
            "Last {} of {} recorded calls (newest first):\n{}",
            entries.len(),
            total,
            json
        )),
        Err(e) => ToolResult::error(format!("serialization error: {e}")),
    }
}

/// Export the complete audit log as a JSON array (all stored entries, oldest first).
pub async fn export_audit_log(state: &AppState, _args: ExportAuditLogArgs) -> ToolResult {
    let (entries, total) = match state.audit_log.lock() {
        Ok(log) => {
            let entries: Vec<_> = log.entries().iter().cloned().collect();
            let total = log.total_recorded();
            (entries, total)
        }
        Err(_) => return ToolResult::error("audit log lock poisoned"),
    };

    match serde_json::to_string_pretty(&entries) {
        Ok(json) => ToolResult::text(format!(
            "// Photonic MCP audit log — {} total recorded calls, {} in buffer\n{}",
            total,
            entries.len(),
            json
        )),
        Err(e) => ToolResult::error(format!("serialization error: {e}")),
    }
}
