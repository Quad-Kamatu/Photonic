use crate::protocol::{
    CopyNodesToClipboardArgs, GetClipboardHistoryArgs, PasteFromHistoryArgs, ToolResult,
};
use crate::server::AppState;
use photonic_core::{
    history::Command,
    node::{NodeId, SceneNode, SceneNodeKind},
    transform::Transform,
};
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex as StdMutex;

// ─── Clipboard data model ─────────────────────────────────────────────────────

/// A single entry in the clipboard ring.
pub struct ClipboardEntry {
    /// Monotonic ID (for stable references in tool output).
    pub id: u64,
    /// Human-readable label set at copy time.
    pub label: String,
    /// UTC timestamp string (seconds since UNIX epoch, formatted as ISO 8601-ish).
    pub created_at: String,
    /// IDs of the top-level roots that were copied (in order).
    pub root_ids: Vec<NodeId>,
    /// All nodes in every copied subtree (roots + all descendants).
    pub nodes: HashMap<NodeId, SceneNode>,
}

/// Session-scoped clipboard ring (not persisted across restarts).
pub struct ClipboardRing {
    entries: VecDeque<ClipboardEntry>,
    max_size: usize,
    next_id: u64,
}

impl ClipboardRing {
    pub fn new(max_size: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            max_size,
            next_id: 1,
        }
    }

    /// Push a new entry to the front (most recent).  Evicts tail when over capacity.
    pub fn push(&mut self, entry: ClipboardEntry) {
        self.entries.push_front(entry);
        if self.entries.len() > self.max_size {
            self.entries.pop_back();
        }
    }

    /// Fetch the next monotonic ID and advance the counter.
    pub fn next_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
}

// ─── Public constructor (used in server.rs) ───────────────────────────────────

pub fn new_clipboard_ring() -> StdMutex<ClipboardRing> {
    StdMutex::new(ClipboardRing::new(20))
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Collect a node subtree (root + all descendants) from the document into a HashMap.
/// Returns (root_id, all_nodes).
fn collect_subtree(
    doc: &photonic_core::document::Document,
    root_id: NodeId,
) -> HashMap<NodeId, SceneNode> {
    let mut out: HashMap<NodeId, SceneNode> = HashMap::new();
    let mut stack = vec![root_id];
    while let Some(id) = stack.pop() {
        if let Some(node) = doc.nodes.get(&id) {
            if let SceneNodeKind::Group(ref g) = node.kind {
                for child_id in &g.children {
                    stack.push(*child_id);
                }
            }
            out.insert(id, node.clone());
        }
    }
    out
}

/// Clone a subtree from a clipboard snapshot into a Vec<SceneNode> suitable for
/// AddNode commands.  All node IDs are remapped to fresh UUIDs.
/// `(dx, dy)` is composed onto each root's existing transform.
/// Returns `(new_root_id, all_cloned_nodes)`.
fn remap_subtree(
    snapshot: &HashMap<NodeId, SceneNode>,
    root_id: NodeId,
    target_layer: NodeId,
    dx: f64,
    dy: f64,
) -> (NodeId, Vec<SceneNode>) {
    // Collect DFS visit order.
    let mut visit_order: Vec<NodeId> = Vec::new();
    let mut stack = vec![root_id];
    while let Some(id) = stack.pop() {
        if let Some(node) = snapshot.get(&id) {
            visit_order.push(id);
            if let SceneNodeKind::Group(ref g) = node.kind {
                for child_id in g.children.iter().rev() {
                    stack.push(*child_id);
                }
            }
        }
    }

    // Build old→new ID mapping.
    let id_map: HashMap<NodeId, NodeId> = visit_order
        .iter()
        .map(|old| (*old, uuid::Uuid::new_v4()))
        .collect();

    let new_root_id = id_map[&root_id];
    let mut result: Vec<SceneNode> = Vec::with_capacity(visit_order.len());

    for (idx, old_id) in visit_order.iter().enumerate() {
        if let Some(src) = snapshot.get(old_id) {
            let mut cloned = src.clone();
            cloned.id = id_map[old_id];

            if idx == 0 {
                // Root: assign target layer and apply offset.
                cloned.layer_id = target_layer;
                cloned.transform = cloned.transform.then(&Transform::translate(dx, dy));
            }

            // Remap group children.
            if let SceneNodeKind::Group(ref mut g) = cloned.kind {
                g.children = g.children.iter().map(|cid| id_map[cid]).collect();
            }

            result.push(cloned);
        }
    }

    (new_root_id, result)
}

// ─── Timestamp helper ─────────────────────────────────────────────────────────

fn now_iso() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Format as YYYY-MM-DDTHH:MM:SSZ (UTC, no sub-second precision needed).
    let s = secs;
    let sec = s % 60;
    let min = (s / 60) % 60;
    let hour = (s / 3600) % 24;
    let days = s / 86400; // days since 1970-01-01
                          // Gregorian calendar reconstruction.
    let (year, month, day) = days_to_ymd(days);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, month, day, hour, min, sec
    )
}

/// Convert days-since-epoch to (year, month, day).
fn days_to_ymd(mut days: u64) -> (u64, u64, u64) {
    // Algorithm: https://howardhinnant.github.io/date_algorithms.html (civil_from_days)
    days += 719468; // shift epoch from 1970-01-01 to 0000-03-01
    let era = days / 146097;
    let doe = days % 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

// ─── MCP tool handlers ────────────────────────────────────────────────────────

/// Copy one or more nodes (with all descendants) into the clipboard ring.
pub async fn copy_nodes_to_clipboard(
    state: &AppState,
    args: CopyNodesToClipboardArgs,
) -> ToolResult {
    if args.node_ids.is_empty() {
        return ToolResult::error("node_ids must not be empty");
    }

    // Read-only snapshot of the document.
    let (root_ids, all_nodes) = {
        let doc = state.document.lock().await;
        let mut root_ids: Vec<NodeId> = Vec::new();
        let mut all_nodes: HashMap<NodeId, SceneNode> = HashMap::new();

        for id in &args.node_ids {
            if !doc.nodes.contains_key(id) {
                return ToolResult::error(format!("Node {} not found", id));
            }
            root_ids.push(*id);
            let subtree = collect_subtree(&doc, *id);
            all_nodes.extend(subtree);
        }
        (root_ids, all_nodes)
    };

    let node_count = root_ids.len();
    let label = args.label.unwrap_or_else(|| {
        format!(
            "{} node{}",
            node_count,
            if node_count == 1 { "" } else { "s" }
        )
    });

    let entry_id = {
        let mut ring = match state.clipboard_ring.lock() {
            Ok(r) => r,
            Err(_) => return ToolResult::error("clipboard lock poisoned"),
        };
        let id = ring.next_id();
        let entry = ClipboardEntry {
            id,
            label: label.clone(),
            created_at: now_iso(),
            root_ids,
            nodes: all_nodes,
        };
        ring.push(entry);
        id
    };

    ToolResult::text(format!(
        "Copied {} node{} to clipboard (entry id={}, index=0, label=\"{}\")",
        node_count,
        if node_count == 1 { "" } else { "s" },
        entry_id,
        label,
    ))
    .with_data(serde_json::json!({
        "id": entry_id,
        "index": 0,
        "label": label,
        "node_count": node_count,
    }))
}

/// List clipboard history entries (summary only — no full node data).
pub async fn get_clipboard_history(state: &AppState, _args: GetClipboardHistoryArgs) -> ToolResult {
    let summaries: Vec<serde_json::Value> = match state.clipboard_ring.lock() {
        Ok(ring) => ring
            .entries
            .iter()
            .enumerate()
            .map(|(idx, e)| {
                serde_json::json!({
                    "index": idx,
                    "id": e.id,
                    "label": e.label,
                    "node_count": e.root_ids.len(),
                    "created_at": e.created_at,
                })
            })
            .collect(),
        Err(_) => return ToolResult::error("clipboard lock poisoned"),
    };

    let count = summaries.len();
    match serde_json::to_string_pretty(&summaries) {
        Ok(json) => ToolResult::text(format!(
            "{} clipboard entr{}:\n{}",
            count,
            if count == 1 { "y" } else { "ies" },
            json
        ))
        .with_data(serde_json::json!({ "entries": summaries })),
        Err(e) => ToolResult::error(format!("serialization error: {e}")),
    }
}

/// Paste nodes from a clipboard history entry into the document.
pub async fn paste_from_history(state: &AppState, args: PasteFromHistoryArgs) -> ToolResult {
    let dx = args.offset_x.unwrap_or(0.0);
    let dy = args.offset_y.unwrap_or(0.0);

    // Snapshot the clipboard entry (take clones so we release the lock quickly).
    let (root_ids, snapshot): (Vec<NodeId>, HashMap<NodeId, SceneNode>) = {
        match state.clipboard_ring.lock() {
            Ok(ring) => match ring.entries.get(args.index) {
                Some(entry) => (entry.root_ids.clone(), entry.nodes.clone()),
                None => {
                    return ToolResult::error(format!(
                        "No clipboard entry at index {} (history has {} entr{})",
                        args.index,
                        ring.entries.len(),
                        if ring.entries.len() == 1 { "y" } else { "ies" },
                    ))
                }
            },
            Err(_) => return ToolResult::error("clipboard lock poisoned"),
        }
    };

    // Determine target layer.
    let target_layer = {
        let doc = state.document.lock().await;
        match args.layer_id {
            Some(id) => {
                if !doc.layers.contains_key(&id) {
                    return ToolResult::error(format!("Layer {} not found", id));
                }
                id
            }
            None => match doc.active_layer_id {
                Some(id) => id,
                None => return ToolResult::error("No active layer — create a layer first"),
            },
        }
    };

    // Remap and build AddNode commands.
    let mut commands: Vec<Command> = Vec::new();
    let mut new_root_ids: Vec<NodeId> = Vec::new();

    for root_id in &root_ids {
        let (new_root_id, cloned_nodes) = remap_subtree(&snapshot, *root_id, target_layer, dx, dy);
        new_root_ids.push(new_root_id);
        for node in cloned_nodes {
            commands.push(Command::AddNode {
                layer_id: Some(node.layer_id),
                node,
            });
        }
    }

    if commands.is_empty() {
        return ToolResult::error("Clipboard entry contained no nodes");
    }

    // Execute as a single undoable batch.
    let cmd = Command::Batch(commands);
    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    history.execute_discrete(cmd, &mut doc);

    let count = new_root_ids.len();
    ToolResult::text(format!(
        "Pasted {} root node{} from clipboard entry at index {}",
        count,
        if count == 1 { "" } else { "s" },
        args.index,
    ))
    .with_data(serde_json::json!({ "pasted_node_ids": new_root_ids }))
}
