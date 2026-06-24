use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

/// A single recorded MCP tool call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    /// Sequential entry ID (1-based, monotonically increasing).
    pub id: u64,
    /// ISO 8601 UTC timestamp of when the call started.
    pub timestamp: String,
    /// The MCP tool name (e.g. `"create_shape"`).
    pub tool_name: String,
    /// Full arguments passed to the tool.
    pub args: serde_json::Value,
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
}

impl AuditLog {
    pub fn new() -> Self {
        Self {
            entries: VecDeque::new(),
            next_id: 1,
            max_entries: 1000,
        }
    }

    /// Record a new entry. Assigns `entry.id` and trims the oldest entry if
    /// the buffer exceeds `max_entries`.
    pub fn record(&mut self, mut entry: AuditEntry) {
        entry.id = self.next_id;
        self.next_id += 1;
        self.entries.push_back(entry);
        if self.entries.len() > self.max_entries {
            self.entries.pop_front();
        }
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
}

impl Default for AuditLog {
    fn default() -> Self {
        Self::new()
    }
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
