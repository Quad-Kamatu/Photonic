use crate::dispatch;
use crate::handlers;
use crate::handlers::clipboard::ClipboardRing;
use crate::protocol::*;
pub use crate::schema_gen::tool_list;
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use photonic_core::{document::Document, history::CommandHistory, AuditLog};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use subtle::ConstantTimeEq;
use tokio::sync::{oneshot, Mutex};
use tracing::info;

/// Header used to authenticate requests when `McpServerConfig::secret` is set.
pub const MCP_SECRET_HEADER: &str = "x-mcp-secret";

/// Configuration for the MCP server.
#[derive(Debug, Clone)]
pub struct McpServerConfig {
    pub port: u16,
    /// Shared secret expected in the [`MCP_SECRET_HEADER`] header, when set.
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

fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/mcp", post(handle_mcp))
        .with_state(state)
}

fn is_authorized(headers: &HeaderMap, secret: Option<&str>) -> bool {
    let Some(secret) = secret else {
        return true;
    };

    headers
        .get(MCP_SECRET_HEADER)
        .is_some_and(|provided| bool::from(secret.as_bytes().ct_eq(provided.as_bytes())))
}

/// Main MCP JSON-RPC handler.
async fn handle_mcp(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<JsonRpcRequest>,
) -> Response {
    if !is_authorized(&headers, state.config.secret.as_deref()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handlers::clipboard::new_clipboard_ring;
    use axum::{
        body::Body,
        http::{header, Method, Request},
    };
    use photonic_core::{history::CommandHistory, AuditLog, Document};
    use serde_json::json;
    use std::sync::{mpsc, Arc, Mutex as StdMutex};
    use tokio::sync::Mutex;
    use tower::util::ServiceExt;

    fn test_state(secret: Option<&str>) -> AppState {
        let (tx, _rx) = mpsc::channel();
        AppState {
            document: Arc::new(Mutex::new(Document::new("auth test", 200.0, 100.0))),
            history: Arc::new(Mutex::new(CommandHistory::new(100))),
            document_path: Arc::new(StdMutex::new(None)),
            capture_tx: Arc::new(StdMutex::new(tx)),
            config: McpServerConfig {
                port: 7842,
                secret: secret.map(str::to_owned),
            },
            audit_log: Arc::new(StdMutex::new(AuditLog::new())),
            clipboard_ring: Arc::new(new_clipboard_ring()),
        }
    }

    fn create_layer_request(secret: Option<&str>) -> Request<Body> {
        let mut builder = Request::builder()
            .method(Method::POST)
            .uri("/mcp")
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(secret) = secret {
            builder = builder.header(MCP_SECRET_HEADER, secret);
        }
        builder
            .body(Body::from(
                json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "tools/call",
                    "params": {
                        "name": "create_layer",
                        "arguments": { "name": "authenticated" }
                    }
                })
                .to_string(),
            ))
            .unwrap()
    }

    #[tokio::test]
    async fn configured_secret_rejects_missing_and_incorrect_headers_before_dispatch() {
        let state = test_state(Some("correct-secret"));
        let document = Arc::clone(&state.document);
        let audit_log = Arc::clone(&state.audit_log);
        let initial_layer_count = document.lock().await.layer_order.len();
        let app = build_router(state);

        for provided_secret in [None, Some("wrong-secret")] {
            let response = app
                .clone()
                .oneshot(create_layer_request(provided_secret))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
            assert_eq!(
                document.lock().await.layer_order.len(),
                initial_layer_count,
                "unauthorized request must not mutate the document"
            );
            assert_eq!(
                audit_log.lock().unwrap().total_recorded(),
                0,
                "unauthorized request must not reach dispatch"
            );
        }

        let response = app
            .oneshot(create_layer_request(Some("correct-secret")))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            document.lock().await.layer_order.len(),
            initial_layer_count + 1
        );
        assert_eq!(audit_log.lock().unwrap().total_recorded(), 1);
    }

    #[tokio::test]
    async fn no_secret_preserves_local_development_behavior() {
        let state = test_state(None);
        let document = Arc::clone(&state.document);
        let initial_layer_count = document.lock().await.layer_order.len();
        let app = build_router(state);

        let response = app.oneshot(create_layer_request(None)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            document.lock().await.layer_order.len(),
            initial_layer_count + 1
        );
    }

    #[tokio::test]
    async fn mcp_endpoint_does_not_enable_cross_origin_requests() {
        let app = build_router(test_state(None));
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::OPTIONS)
                    .uri("/mcp")
                    .header(header::ORIGIN, "https://evil.example")
                    .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
                    .header(
                        header::ACCESS_CONTROL_REQUEST_HEADERS,
                        "content-type, x-mcp-secret",
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert!(!response
            .headers()
            .contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN));
    }
}
