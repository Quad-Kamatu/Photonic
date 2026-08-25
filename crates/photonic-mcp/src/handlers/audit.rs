use crate::protocol::{ExportAuditLogArgs, ListAuditLogArgs, ToolResult};
use crate::server::AppState;
use photonic_core::{AuditEntry, AUDIT_EXPORT_MAX_BYTES, AUDIT_MAX_ENTRIES};

const DEFAULT_EXPORT_LIMIT: usize = 100;

/// Serialize an audit page without allowing pretty-printing to expand it past
/// the response budget. Entries are removed from the end, preserving the
/// requested ordering and leaving the caller's offset useful for pagination.
fn serialize_entries_bounded<F>(entries: &mut Vec<AuditEntry>, header: F) -> Result<String, String>
where
    F: Fn(usize, bool) -> String,
{
    let mut byte_limited = false;
    loop {
        let json = serde_json::to_string_pretty(entries)
            .map_err(|e| format!("serialization error: {e}"))?;
        let output = format!("{}\n{}", header(entries.len(), byte_limited), json);
        if output.len() <= AUDIT_EXPORT_MAX_BYTES || entries.is_empty() {
            return Ok(output);
        }
        entries.pop();
        byte_limited = true;
    }
}

/// Return the most-recent N audit entries (default 50) as formatted JSON text.
pub async fn list_audit_log(state: &AppState, args: ListAuditLogArgs) -> ToolResult {
    let limit = args.limit.unwrap_or(50).min(AUDIT_MAX_ENTRIES);
    let (mut entries, total, retained) = match state.audit_log.lock() {
        Ok(log) => {
            let entries: Vec<_> = log.recent(limit).into_iter().cloned().collect();
            let total = log.total_recorded();
            let retained = log.entries().len();
            (entries, total, retained)
        }
        Err(_) => return ToolResult::error("audit log lock poisoned"),
    };

    match serialize_entries_bounded(&mut entries, |returned, byte_limited| {
        let cap_note = if byte_limited {
            "; response byte cap reached"
        } else {
            ""
        };
        format!(
            "Last {} of {} recorded calls (newest first; {} retained{}):",
            returned, total, retained, cap_note
        )
    }) {
        Ok(output) => ToolResult::text(output),
        Err(error) => ToolResult::error(error),
    }
}

/// Export a bounded page of the retained audit log as a JSON array, oldest
/// first. Use `offset` to continue after the returned page.
pub async fn export_audit_log(state: &AppState, args: ExportAuditLogArgs) -> ToolResult {
    let limit = args
        .limit
        .unwrap_or(DEFAULT_EXPORT_LIMIT)
        .min(AUDIT_MAX_ENTRIES);
    let offset = args.offset.unwrap_or(0);
    let (mut entries, total, retained) = match state.audit_log.lock() {
        Ok(log) => {
            let entries: Vec<_> = log
                .entries()
                .iter()
                .skip(offset)
                .take(limit)
                .cloned()
                .collect();
            let total = log.total_recorded();
            let retained = log.entries().len();
            (entries, total, retained)
        }
        Err(_) => return ToolResult::error("audit log lock poisoned"),
    };

    match serialize_entries_bounded(&mut entries, |returned, byte_limited| {
        let next_offset = offset.saturating_add(returned);
        let has_more = next_offset < retained;
        let next = if has_more {
            next_offset.to_string()
        } else {
            "none".to_string()
        };
        let cap_note = if byte_limited {
            "; response byte cap reached"
        } else {
            ""
        };
        format!(
            "// Photonic MCP audit log — {} total recorded calls, {} retained; offset {}, returned {}, next_offset {}, has_more {}{}",
            total, retained, offset, returned, next, has_more, cap_note
        )
    }) {
        Ok(output) => ToolResult::text(output),
        Err(error) => ToolResult::error(error),
    }
}
