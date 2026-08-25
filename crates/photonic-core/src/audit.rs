use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::VecDeque;

/// Maximum number of entries retained by the in-memory audit ring.
pub const AUDIT_MAX_ENTRIES: usize = 1000;
/// Maximum compact serialized size of all retained audit entries.
pub const AUDIT_MAX_BYTES: usize = 1024 * 1024;
/// Maximum compact serialized size of one retained argument summary.
pub const AUDIT_MAX_ARGS_BYTES: usize = 4 * 1024;
/// Maximum size of one formatted audit list/export response.
pub const AUDIT_EXPORT_MAX_BYTES: usize = 256 * 1024;

const MAX_SUMMARY_DEPTH: usize = 8;
const MAX_SUMMARY_ITEMS: usize = 16;
const MAX_SUMMARY_NODES: usize = 128;
const MAX_SUMMARY_STRING_BYTES: usize = 256;
const MAX_SUMMARY_KEY_BYTES: usize = 128;
const MAX_TIMESTAMP_BYTES: usize = 64;
const MAX_TOOL_NAME_BYTES: usize = 128;
const MAX_RESULT_SUMMARY_BYTES: usize = 512;
const TRUNCATED_MARKER: &str = "[truncated]";

/// A single recorded MCP tool call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    /// Sequential entry ID (1-based, monotonically increasing).
    pub id: u64,
    /// ISO 8601 UTC timestamp of when the call started.
    pub timestamp: String,
    /// The MCP tool name (e.g. `"create_shape"`).
    pub tool_name: String,
    /// Bounded structural summary of the arguments passed to the tool.
    pub args: Value,
    /// First 200 characters of the result text, or `"error: <msg>"` on failure.
    pub result_summary: String,
    /// Wall-clock duration of the tool call in milliseconds.
    pub duration_ms: u64,
    /// `true` when the tool returned an MCP error result.
    pub is_error: bool,
}

/// In-memory ring buffer of recent MCP tool calls.
///
/// Stored in `AppState` and shared (via `Arc<StdMutex<AuditLog>>`) between the
/// MCP server and the GUI Audit panel. In-memory only — not persisted across
/// server restarts.
pub struct AuditLog {
    entries: VecDeque<AuditEntry>,
    next_id: u64,
    max_entries: usize,
    max_bytes: usize,
    stored_bytes: usize,
}

impl AuditLog {
    pub fn new() -> Self {
        Self::with_limits(AUDIT_MAX_ENTRIES, AUDIT_MAX_BYTES)
    }

    /// Create an audit log with explicit entry-count and serialized-byte limits.
    /// A ring always retains at least one entry when its byte budget allows it.
    pub fn with_limits(max_entries: usize, max_bytes: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            next_id: 1,
            max_entries: max_entries.max(1),
            max_bytes,
            stored_bytes: 0,
        }
    }

    /// Record a new entry. Assigns `entry.id`, bounds its stored fields, and
    /// trims the oldest entries if either retention limit is exceeded.
    pub fn record(&mut self, mut entry: AuditEntry) {
        entry.id = self.next_id;
        self.next_id += 1;

        // AuditLog is the storage boundary and is also shared with non-MCP
        // callers, so do not rely on dispatch_tool to sanitize an entry first.
        entry.timestamp = truncate_text(&entry.timestamp, MAX_TIMESTAMP_BYTES);
        entry.tool_name = truncate_text(&entry.tool_name, MAX_TOOL_NAME_BYTES);
        entry.args = summarize_audit_args(&entry.args);
        entry.result_summary = truncate_text(&entry.result_summary, MAX_RESULT_SUMMARY_BYTES);

        let entry_bytes = serialized_size(&entry);
        if entry_bytes > self.max_bytes {
            return;
        }

        while (self.entries.len() >= self.max_entries
            || self.stored_bytes.saturating_add(entry_bytes) > self.max_bytes)
            && !self.entries.is_empty()
        {
            self.remove_oldest();
        }

        // The fixed metadata can exceed a caller-supplied test/configuration
        // budget. Dropping the entry is safer than violating that budget.
        if self.stored_bytes.saturating_add(entry_bytes) > self.max_bytes {
            return;
        }

        self.stored_bytes += entry_bytes;
        self.entries.push_back(entry);
    }

    /// Borrow all stored entries (oldest first).
    pub fn entries(&self) -> &VecDeque<AuditEntry> {
        &self.entries
    }

    /// Return up to `limit` most-recent entries (newest first).
    pub fn recent(&self, limit: usize) -> Vec<&AuditEntry> {
        self.entries.iter().rev().take(limit).collect()
    }

    /// Total number of entries recorded since the server started (includes
    /// entries that have been evicted from the buffer).
    pub fn total_recorded(&self) -> u64 {
        self.next_id - 1
    }

    /// Compact serialized bytes currently retained by the audit ring.
    pub fn stored_bytes(&self) -> usize {
        self.stored_bytes
    }

    /// Compact serialized-byte capacity of this audit ring.
    pub fn max_bytes(&self) -> usize {
        self.max_bytes
    }

    fn remove_oldest(&mut self) {
        if let Some(entry) = self.entries.pop_front() {
            self.stored_bytes = self.stored_bytes.saturating_sub(serialized_size(&entry));
        }
    }
}

impl Default for AuditLog {
    fn default() -> Self {
        Self::new()
    }
}

/// Build a useful but bounded summary for audit storage.
///
/// The source value is traversed only up to fixed depth, item, and node
/// limits, so this remains bounded even when called with a very large JSON
/// value. Large strings are shortened and omitted collection members are
/// represented by a visible marker.
pub fn summarize_audit_args(value: &Value) -> Value {
    let mut remaining_nodes = MAX_SUMMARY_NODES;
    let summary = summarize_value(value, 0, &mut remaining_nodes);

    match serde_json::to_vec(&summary) {
        Ok(encoded) if encoded.len() <= AUDIT_MAX_ARGS_BYTES => summary,
        _ => Value::String(TRUNCATED_MARKER.to_string()),
    }
}

fn summarize_value(value: &Value, depth: usize, remaining_nodes: &mut usize) -> Value {
    if *remaining_nodes == 0 {
        return Value::String(TRUNCATED_MARKER.to_string());
    }
    *remaining_nodes -= 1;

    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) => value.clone(),
        Value::String(text) => Value::String(truncate_text(text, MAX_SUMMARY_STRING_BYTES)),
        Value::Array(items) => {
            if depth >= MAX_SUMMARY_DEPTH {
                return Value::String(TRUNCATED_MARKER.to_string());
            }

            let mut summary = Vec::with_capacity(items.len().min(MAX_SUMMARY_ITEMS) + 1);
            for item in items.iter().take(MAX_SUMMARY_ITEMS) {
                summary.push(summarize_value(item, depth + 1, remaining_nodes));
            }
            if items.len() > MAX_SUMMARY_ITEMS || *remaining_nodes == 0 {
                summary.push(Value::String(TRUNCATED_MARKER.to_string()));
            }
            Value::Array(summary)
        }
        Value::Object(object) => {
            if depth >= MAX_SUMMARY_DEPTH {
                return Value::String(TRUNCATED_MARKER.to_string());
            }

            let mut summary = Map::new();
            for (key, item) in object.iter().take(MAX_SUMMARY_ITEMS) {
                summary.insert(
                    truncate_text(key, MAX_SUMMARY_KEY_BYTES),
                    summarize_value(item, depth + 1, remaining_nodes),
                );
            }
            if object.len() > MAX_SUMMARY_ITEMS || *remaining_nodes == 0 {
                summary.insert(
                    "_truncated".to_string(),
                    Value::String(TRUNCATED_MARKER.to_string()),
                );
            }
            Value::Object(summary)
        }
    }
}

fn truncate_text(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }

    const SUFFIX: &str = "...[truncated]";
    if max_bytes <= SUFFIX.len() {
        let mut end = max_bytes.min(text.len());
        while end > 0 && !text.is_char_boundary(end) {
            end -= 1;
        }
        return text[..end].to_string();
    }

    let mut end = (max_bytes - SUFFIX.len()).min(text.len());
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    let mut result = String::with_capacity(end + SUFFIX.len());
    result.push_str(&text[..end]);
    result.push_str(SUFFIX);
    result
}

fn serialized_size(entry: &AuditEntry) -> usize {
    serde_json::to_vec(entry)
        .map(|encoded| encoded.len())
        .unwrap_or(usize::MAX)
}

// ── Timestamp helper ─────────────────────────────────────────────────────────

/// Current UTC time as an ISO 8601 string. Uses only `std::time` — no external
/// datetime crate required.
pub fn audit_timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => {
            let secs = d.as_secs();
            let (y, mo, day, h, min, s) = epoch_to_ymd(secs);
            format!(
                "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
                y, mo, day, h, min, s
            )
        }
        Err(_) => "1970-01-01T00:00:00Z".to_string(),
    }
}

fn epoch_to_ymd(secs: u64) -> (u32, u32, u32, u32, u32, u32) {
    let s = secs % 60;
    let total_min = secs / 60;
    let min = total_min % 60;
    let total_h = total_min / 60;
    let h = total_h % 24;
    let mut days = total_h / 24;

    let mut year = 1970u32;
    loop {
        let dy = days_in_year(year);
        if days < dy as u64 {
            break;
        }
        days -= dy as u64;
        year += 1;
    }
    let mut month = 1u32;
    loop {
        let dm = days_in_month(year, month);
        if days < dm as u64 {
            break;
        }
        days -= dm as u64;
        month += 1;
    }
    (year, month, days as u32 + 1, h as u32, min as u32, s as u32)
}

fn is_leap(y: u32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}
fn days_in_year(y: u32) -> u32 {
    if is_leap(y) {
        366
    } else {
        365
    }
}
fn days_in_month(y: u32, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap(y) {
                29
            } else {
                28
            }
        }
        _ => 30,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entry(args: Value) -> AuditEntry {
        AuditEntry {
            id: 0,
            timestamp: "2026-01-01T00:00:00Z".to_string(),
            tool_name: "test_tool".to_string(),
            args,
            result_summary: "ok".to_string(),
            duration_ms: 1,
            is_error: false,
        }
    }

    #[test]
    fn argument_summary_bounds_strings_arrays_and_nested_objects() {
        let args = json!({
            "payload": "x".repeat(AUDIT_MAX_ARGS_BYTES * 8),
            "items": (0..64).collect::<Vec<_>>(),
            "nested": {"level": {"level": {"level": {"value": true}}}}
        });

        let summary = summarize_audit_args(&args);
        assert!(serde_json::to_vec(&summary).unwrap().len() <= AUDIT_MAX_ARGS_BYTES);
        assert_ne!(summary, args);
        assert!(summary.to_string().contains(TRUNCATED_MARKER));
    }

    #[test]
    fn record_enforces_count_and_serialized_byte_budgets() {
        let args = json!({"payload": "x".repeat(800)});
        let mut sample = AuditLog::with_limits(3, usize::MAX);
        sample.record(entry(args.clone()));
        let one_entry_bytes = sample.stored_bytes();

        let mut log = AuditLog::with_limits(3, one_entry_bytes * 2 + one_entry_bytes / 2);
        for index in 0..10 {
            log.record(entry(json!({
                "payload": "x".repeat(800),
                "index": index
            })));
        }

        assert!(log.entries().len() <= 2, "byte budget must evict entries");
        assert!(log.entries().front().unwrap().id > 1);
        assert!(log.stored_bytes() <= log.max_bytes());
        let serialized_bytes: usize = log.entries().iter().map(serialized_size).sum();
        assert_eq!(log.stored_bytes(), serialized_bytes);
        assert_eq!(log.total_recorded(), 10);
    }
}
