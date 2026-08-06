use crate::dispatch;
use crate::handlers;
use crate::handlers::clipboard::ClipboardRing;
use crate::protocol::*;
pub use crate::schema_gen::tool_list;
use axum::{
    extract::{DefaultBodyLimit, State},
    http::HeaderMap,
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use crate::protocol::envelope::{
    self, Dialect, ProtocolMode, RpcError, DEFAULT_BODY_LIMIT,
};
use photonic_core::{document::Document, history::CommandHistory, AuditLog};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::{oneshot, Mutex};
use tracing::info;

/// Configuration for the MCP server.
#[derive(Debug, Clone)]
pub struct McpServerConfig {
    pub port: u16,
    pub secret: Option<String>,
    pub protocol_mode: crate::protocol::ProtocolMode,
}

impl Default for McpServerConfig {
    fn default() -> Self {
        Self {
            port: 7842,
            secret: None,
            protocol_mode: crate::protocol::ProtocolMode::Dual,
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
    /// Last native document path used by MCP save/save-as.
    pub document_path: Arc<StdMutex<Option<PathBuf>>>,
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
                document_path: Arc::new(StdMutex::new(None)),
                capture_tx: Arc::new(StdMutex::new(capture_tx)),
                config,
                audit_log,
                clipboard_ring: Arc::new(handlers::clipboard::new_clipboard_ring()),
            },
            running,
        }
    }

    /// Share the host application's current native document path with MCP so a
    /// pathless `save_document` has the same save target as an opened file.
    pub fn with_document_path(mut self, document_path: Arc<StdMutex<Option<PathBuf>>>) -> Self {
        self.state.document_path = document_path;
        self
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

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/mcp", post(handle_mcp))
        .layer(DefaultBodyLimit::max(DEFAULT_BODY_LIMIT))
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            require_bearer,
        ))
        .with_state(state)
}


/// Rejects every request that does not present the session token.
async fn require_bearer(
    State(state): State<AppState>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let Some(expected) = state.config.secret.as_deref() else {
        return next.run(req).await;
    };
    let presented = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split_once(' '))
        .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("Bearer"))
        .map(|(_, token)| token);
    match presented {
        Some(token) if crate::auth::token_eq(token, expected) => next.run(req).await,
        _ => axum::http::StatusCode::UNAUTHORIZED.into_response(),
    }
}

/// Main MCP JSON-RPC handler.
async fn handle_mcp(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<JsonRpcRequest>,
) -> Response {
    if req.jsonrpc != "2.0" {
        return Json(JsonRpcResponse::error(
            req.id.clone(),
            crate::protocol::ERR_INVALID_REQUEST,
            "Invalid JSON-RPC version",
        ))
        .into_response();
    }

    let mode = state.config.protocol_mode;
    let params = req.params.clone().unwrap_or_else(|| json!({}));
    let tool_name = if req.method == "tools/call" {
        params.get("name").and_then(|v| v.as_str())
    } else {
        None
    };
    let header_method = headers
        .get("mcp-method")
        .or_else(|| headers.get("Mcp-Method"))
        .and_then(|v| v.to_str().ok());
    let header_name = headers
        .get("mcp-name")
        .or_else(|| headers.get("Mcp-Name"))
        .and_then(|v| v.to_str().ok());

    if let Err(e) =
        envelope::validate_mcp_headers(mode, header_method, header_name, &req.method, tool_name)
    {
        return rpc_err_response(req.id.clone(), e);
    }

    // Lifecycle methods that predate per-request _meta.
    if req.method == "initialize" {
        return match mode {
            ProtocolMode::Dual => Json(JsonRpcResponse::success(
                req.id,
                envelope::legacy_initialize_result(),
            ))
            .into_response(),
            ProtocolMode::Strict => rpc_err_response(
                req.id,
                RpcError::new(
                    crate::protocol::ERR_METHOD_NOT_FOUND,
                    "initialize removed in 2026-07-28; use server/discover",
                )
                .http(400),
            ),
        };
    }
    if req.method == "notifications/initialized" {
        return match mode {
            ProtocolMode::Dual => {
                Json(JsonRpcResponse::success(req.id, json!({ "status": "ok" }))).into_response()
            }
            ProtocolMode::Strict => rpc_err_response(
                req.id,
                RpcError::new(
                    crate::protocol::ERR_METHOD_NOT_FOUND,
                    "notifications/initialized is not used in 2026-07-28",
                )
                .http(400),
            ),
        };
    }

    // discover may be called without prior handshake; still negotiate dialect.
    let dialect = match envelope::negotiate_dialect(mode, &params) {
        Ok(d) => d,
        Err(e) => return rpc_err_response(req.id.clone(), e),
    };

    let result = dispatch_method(state, &req.method, params, dialect).await;
    match result {
        Ok(value) => Json(JsonRpcResponse::success(req.id, value)).into_response(),
        Err(e) => rpc_err_response(req.id, e),
    }
}

fn rpc_err_response(id: Option<Value>, err: RpcError) -> Response {
    let status = err.http_status.unwrap_or(200);
    let body = Json(JsonRpcResponse::from_rpc_error(id, err));
    if status == 200 {
        body.into_response()
    } else {
        (axum::http::StatusCode::from_u16(status).unwrap_or(axum::http::StatusCode::BAD_REQUEST), body)
            .into_response()
    }
}

async fn dispatch_method(
    state: AppState,
    method: &str,
    params: Value,
    dialect: Dialect,
) -> Result<Value, RpcError> {
    match method {
        "server/discover" => Ok(envelope::discover_result(state.config.protocol_mode)),

        "tools/list" => {
            let tools = serde_json::to_value(tool_list()).unwrap_or_else(|_| json!([]));
            // Always wrap with resultType — extra fields are fine for dual/legacy clients.
            let _ = dialect;
            Ok(envelope::tools_list_result(tools))
        }

        "tools/call" => {
            let tool_name = params
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    RpcError::new(crate::protocol::ERR_INVALID_PARAMS, "Missing tool name").http(400)
                })?
                .to_string();
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            let tool_result = dispatch::dispatch_tool(&state, &tool_name, args)
                .await
                .map_err(|msg| RpcError::new(-32000, msg))?;
            let raw = serde_json::to_value(tool_result).unwrap_or(json!({}));
            Ok(envelope::wrap_tool_call_result(raw))
        }

        other => Err(RpcError::new(
            crate::protocol::ERR_METHOD_NOT_FOUND,
            format!("Unknown method: {other}"),
        )),
    }
}

pub(crate) use crate::dispatch::dispatch_tool_inner;
