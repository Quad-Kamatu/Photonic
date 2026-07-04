use crate::dispatch;
use crate::handlers;
use crate::handlers::clipboard::ClipboardRing;
use crate::protocol::*;
pub use crate::schema_gen::tool_list;
use axum::{
    extract::State,
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use photonic_core::{
    document::Document, history::CommandHistory, AuditLog,
};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::{oneshot, Mutex};
use tower_http::cors::CorsLayer;
use tracing::info;

/// Configuration for the MCP server.
#[derive(Debug, Clone)]
pub struct McpServerConfig {
    pub port: u16,
    pub secret: Option<String>,
}

impl Default for McpServerConfig {
    fn default() -> Self {
        Self {
            port: 7842,
            secret: None,
        }
    }
}

/// Wraps a handler result with its mutation intent so `dispatch_tool` can
/// decide whether to schedule an auto-checkpoint without consulting a
/// separate list.
pub(crate) struct ToolOutput {
    pub(crate) result: ToolResult,
    pub(crate) mutates: bool,
}

impl ToolOutput {
    pub(crate) fn mutating(result: ToolResult) -> Self {
        Self {
            result,
            mutates: true,
        }
    }
    pub(crate) fn readonly(result: ToolResult) -> Self {
        Self {
            result,
            mutates: false,
        }
    }
}

/// Shared application state injected into all axum handlers.
#[derive(Clone)]
pub struct AppState {
    pub document: Arc<Mutex<Document>>,
    pub history: Arc<Mutex<CommandHistory>>,
    /// Sends screenshot requests to the render thread.
    /// Uses std::sync::mpsc so the render thread can poll synchronously.
    pub capture_tx: Arc<StdMutex<std::sync::mpsc::Sender<oneshot::Sender<Vec<u8>>>>>,
    pub config: McpServerConfig,
    /// In-memory audit log of every MCP tool call. Shared with the GUI Audit panel
    /// via a std::sync::Mutex so the sync GUI thread can read it without async.
    pub audit_log: Arc<StdMutex<AuditLog>>,
    /// Session-scoped clipboard ring — stores up to 20 copied node snapshots.
    /// Uses StdMutex so the GUI thread can also read it without async.
    pub clipboard_ring: Arc<StdMutex<ClipboardRing>>,
}

/// The MCP server — wraps axum and owns shared state.
pub struct McpServer {
    pub state: AppState,
    /// Set to `true` once the server is successfully bound and listening.
    pub running: Arc<AtomicBool>,
}

impl McpServer {
    pub fn new(
        document: Arc<Mutex<Document>>,
        history: Arc<Mutex<CommandHistory>>,
        capture_tx: std::sync::mpsc::Sender<oneshot::Sender<Vec<u8>>>,
        config: McpServerConfig,
        running: Arc<AtomicBool>,
        audit_log: Arc<StdMutex<AuditLog>>,
    ) -> Self {
        Self {
            state: AppState {
                document,
                history,
                capture_tx: Arc::new(StdMutex::new(capture_tx)),
                config,
                audit_log,
                clipboard_ring: Arc::new(handlers::clipboard::new_clipboard_ring()),
            },
            running,
        }
    }

    /// Start listening. This blocks the current task.
    pub async fn run(self) -> anyhow::Result<()> {
        let port = self.state.config.port;

        // Background task: flush debounced MCP checkpoint every 10 s.
        // A checkpoint is actually written once 60 s have elapsed with no new
        // mutations — so rapid tool calls coalesce into a single snapshot.
        let bg_state = self.state.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));
            loop {
                interval.tick().await;
                let doc = bg_state.document.lock().await;
                let mut history = bg_state.history.lock().await;
                history.tick_mcp_checkpoint(&doc);
            }
        });

        let router = build_router(self.state);
        let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{}", port)).await?;
        // Mark the server as successfully bound before blocking on serve.
        self.running.store(true, Ordering::Relaxed);
        info!("Photonic MCP server listening on http://127.0.0.1:{}", port);
        axum::serve(listener, router).await?;
        Ok(())
    }
}

fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/mcp", post(handle_mcp))
        .layer(CorsLayer::permissive())
        .with_state(state)
}

/// Main MCP JSON-RPC handler.
async fn handle_mcp(State(state): State<AppState>, Json(req): Json<JsonRpcRequest>) -> Response {
    if req.jsonrpc != "2.0" {
        return Json(JsonRpcResponse::error(
            req.id,
            -32600,
            "Invalid JSON-RPC version",
        ))
        .into_response();
    }

    let result = dispatch(state, &req.method, req.params).await;

    match result {
        Ok(value) => Json(JsonRpcResponse::success(req.id, value)).into_response(),
        Err(msg) => Json(JsonRpcResponse::error(req.id, -32000, msg)).into_response(),
    }
}

async fn dispatch(state: AppState, method: &str, params: Option<Value>) -> Result<Value, String> {
    let params = params.unwrap_or(Value::Object(Default::default()));

    match method {
        // ── MCP lifecycle ─────────────────────────────────────────────────
        "initialize" => {
            let result = InitializeResult {
                protocol_version: "2024-11-05".to_string(),
                capabilities: ServerCapabilities {
                    tools: ToolsCapability {
                        list_changed: false,
                    },
                },
                server_info: ServerInfo {
                    name: "photonic".to_string(),
                    version: env!("CARGO_PKG_VERSION").to_string(),
                },
            };
            Ok(serde_json::to_value(result).unwrap())
        }
        "notifications/initialized" => Ok(json!({ "status": "ok" })),

        // ── Tool list ─────────────────────────────────────────────────────
        "tools/list" => Ok(json!({ "tools": tool_list() })),

        // ── Tool calls ────────────────────────────────────────────────────
        "tools/call" => {
            let tool_name = params
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or("Missing tool name")?
                .to_string();
            let args = params.get("arguments").cloned().unwrap_or(json!({}));

            let tool_result = dispatch::dispatch_tool(&state, &tool_name, args).await?;
            Ok(serde_json::to_value(tool_result).unwrap())
        }

        _ => Err(format!("Unknown method: {}", method)),
    }
}

pub(crate) use crate::dispatch::dispatch_tool_inner;
