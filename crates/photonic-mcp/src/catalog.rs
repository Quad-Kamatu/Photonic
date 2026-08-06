//! Tool catalog helpers for Pattern B search + execute.

use crate::schema_gen::tool_list;
use serde_json::{json, Value};
use std::sync::OnceLock;

/// Promote high-frequency tools so compact listings stay useful.
pub fn promoted_tool_names() -> &'static [&'static str] {
    &[
        "undo",
        "redo",
        "screenshot",
        "get_document_info",
        "get_document_state",
        "list_artboards",
        "create_shape",
        "set_fill",
        "save_document",
        "create_sequence",
        "list_sequences",
        "import_media",
        "insert_clip",
        "split_clip",
        "export_sequence",
    ]
}

/// All tools from the registry as JSON values (cached for the process).
pub fn all_tools() -> Vec<Value> {
    static CACHE: OnceLock<Vec<Value>> = OnceLock::new();
    CACHE
        .get_or_init(|| match tool_list() {
            Value::Array(a) => a,
            other => vec![other],
        })
        .clone()
}

fn tool_name(t: &Value) -> Option<&str> {
    t.get("name").and_then(|n| n.as_str())
}

fn meta_tool(name: &str) -> Value {
    let description = match name {
        "search_actions" => "Search the MCP tool catalog by keywords (name/description).",
        _ => "Run a tool by name with a params object.",
    };
    json!({
        "name": name,
        "description": description,
        "inputSchema": { "type": "object", "properties": {} }
    })
}

/// Compact list: promoted tools that exist in this build, plus search/execute.
pub fn compact_tool_list() -> Value {
    let all = all_tools();
    let mut out = Vec::new();
    for name in ["search_actions", "execute_action"] {
        if let Some(t) = all.iter().find(|t| tool_name(t) == Some(name)) {
            out.push(t.clone());
        } else {
            out.push(meta_tool(name));
        }
    }
    for name in promoted_tool_names() {
        if let Some(t) = all.iter().find(|t| tool_name(t) == Some(name)) {
            if !out.iter().any(|x| tool_name(x) == Some(name)) {
                out.push(t.clone());
            }
        }
    }
    json!(out)
}

/// Search tools by keyword tokens against name + description.
pub fn search_actions(query: &str, limit: usize) -> Value {
    let q = query.to_ascii_lowercase();
    let tokens: Vec<&str> = q.split_whitespace().filter(|t| !t.is_empty()).collect();
    let mut scored: Vec<(i32, Value)> = Vec::new();
    for t in all_tools() {
        let name = tool_name(&t).unwrap_or("").to_ascii_lowercase();
        let desc = t
            .get("description")
            .and_then(|d| d.as_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if name == "search_actions" || name == "execute_action" {
            continue;
        }
        let mut score = 0i32;
        if tokens.is_empty() {
            continue;
        }
        for tok in &tokens {
            if name == *tok {
                score += 100;
            } else if name.contains(tok) {
                score += 40;
            }
            if desc.contains(tok) {
                score += 10;
            }
        }
        if score > 0 {
            let summary = json!({
                "name": t.get("name"),
                "description": t.get("description"),
                "inputSchema": summarize_schema(t.get("inputSchema")),
                "score": score,
            });
            scored.push((score, summary));
        }
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0));
    let limit = limit.clamp(1, 50);
    let hits: Vec<Value> = scored.into_iter().take(limit).map(|(_, v)| v).collect();
    json!({ "actions": hits, "count": hits.len(), "query": query })
}

fn summarize_schema(schema: Option<&Value>) -> Value {
    let Some(schema) = schema else {
        return json!({});
    };
    let props = schema.get("properties").cloned().unwrap_or(json!({}));
    let required = schema.get("required").cloned().unwrap_or(json!([]));
    let mut slim = serde_json::Map::new();
    if let Some(obj) = props.as_object() {
        for (k, v) in obj {
            slim.insert(
                k.clone(),
                json!({
                    "type": v.get("type"),
                    "description": v.get("description"),
                }),
            );
        }
    }
    json!({ "type": "object", "properties": slim, "required": required })
}
