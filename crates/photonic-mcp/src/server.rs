use crate::dispatch;
use crate::handlers;
use crate::handlers::clipboard::ClipboardRing;
use crate::protocol::*;
pub use crate::schema_gen::tool_list;
use axum::{
    extract::{DefaultBodyLimit, Request, State},
    http::{
        header::{self, HeaderName, HeaderValue},
        request::Parts as RequestParts,
        HeaderMap, Method, StatusCode, Uri,
    },
    middleware::{self, Next},
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
use tower_http::cors::{AllowOrigin, CorsLayer};
use tracing::info;

/// Header used to authenticate requests when `McpServerConfig::secret` is set.
pub const MCP_SECRET_HEADER: &str = "x-mcp-secret";

/// Maximum encoded JSON-RPC request size accepted by the MCP endpoint.
const MCP_REQUEST_BODY_LIMIT: usize = 2 * 1024 * 1024;

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
        // Authenticate the request metadata before any body extractor runs.
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            require_authentication,
        ))
        .layer(DefaultBodyLimit::max(MCP_REQUEST_BODY_LIMIT))
        .layer(
            CorsLayer::new()
                .allow_origin(AllowOrigin::predicate(is_loopback_origin))
                .allow_methods([Method::POST])
                .allow_headers([
                    header::CONTENT_TYPE,
                    HeaderName::from_static(MCP_SECRET_HEADER),
                ]),
        )
        .with_state(state)
}

async fn require_authentication(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    if !is_authorized(request.headers(), state.config.secret.as_deref()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    next.run(request).await
}

fn is_loopback_origin(origin: &HeaderValue, _parts: &RequestParts) -> bool {
    let Ok(origin) = origin.to_str() else {
        return false;
    };
    let Ok(uri) = origin.parse::<Uri>() else {
        return false;
    };

    matches!(uri.scheme_str(), Some("http" | "https"))
        && (uri.path().is_empty() || uri.path() == "/")
        && uri.query().is_none()
        && uri.host().is_some_and(|host| {
            host.eq_ignore_ascii_case("localhost")
                || host == "127.0.0.1"
                || host == "[::1]"
                || host == "::1"
        })
}

fn is_authorized(headers: &HeaderMap, secret: Option<&str>) -> bool {
    let Some(secret) = secret else {
        return true;
    };

    headers
        .get(MCP_SECRET_HEADER)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|provided| bool::from(secret.as_bytes().ct_eq(provided.as_bytes())))
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

    if req.id.is_none() {
        return StatusCode::NO_CONTENT.into_response();
    }

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
        body::{to_bytes, Body},
        http::Request,
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

    fn save_request(path: &std::path::Path, secret: Option<&str>) -> axum::http::Request<Body> {
        let mut builder = axum::http::Request::builder()
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
                        "name": "save_document",
                        "arguments": { "path": path }
                    }
                })
                .to_string(),
            ))
            .unwrap()
    }

    #[tokio::test]
    async fn configured_secret_rejects_missing_and_incorrect_headers_before_dispatch() {
        let base = std::env::temp_dir().join(format!("photonic-mcp-auth-{}", uuid::Uuid::new_v4()));
        let path = base.join("missing.photon");
        let state = test_state(Some("correct-secret"));
        let app = build_router(state);

        let response = app
            .clone()
            .oneshot(save_request(&path, None))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert!(!path.exists(), "unauthenticated request must not dispatch");

        let response = app
            .clone()
            .oneshot(save_request(&path, Some("wrong-secret")))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert!(!path.exists(), "incorrect secret must not dispatch");

        let response = app
            .oneshot(save_request(&path, Some("correct-secret")))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(path.exists(), "authenticated request should dispatch");

        std::fs::remove_dir_all(base).unwrap();
    }

    #[tokio::test]
    async fn unauthenticated_oversized_body_is_rejected_before_body_extraction() {
        let app = build_router(test_state(Some("correct-secret")));
        let oversized_body = vec![b' '; MCP_REQUEST_BODY_LIMIT + 1];
        let request = axum::http::Request::builder()
            .method(Method::POST)
            .uri("/mcp")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(oversized_body))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn authenticated_oversized_body_is_capped() {
        let app = build_router(test_state(Some("correct-secret")));
        let oversized_body = vec![b' '; MCP_REQUEST_BODY_LIMIT + 1];
        let request = axum::http::Request::builder()
            .method(Method::POST)
            .uri("/mcp")
            .header(header::CONTENT_TYPE, "application/json")
            .header(MCP_SECRET_HEADER, "correct-secret")
            .body(Body::from(oversized_body))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn no_secret_preserves_local_development_behavior() {
        let base =
            std::env::temp_dir().join(format!("photonic-mcp-no-auth-{}", uuid::Uuid::new_v4()));
        let path = base.join("allowed.photon");
        let app = build_router(test_state(None));

        let response = app.oneshot(save_request(&path, None)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(path.exists());

        std::fs::remove_dir_all(base).unwrap();
    }

    async fn post_json(request: Value) -> (StatusCode, Vec<u8>) {
        let response = build_router(test_state(None))
            .oneshot(
                Request::post("/mcp")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(request.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, body.to_vec())
    }

    #[tokio::test]
    async fn initialized_notification_returns_no_content() {
        let (status, body) = post_json(json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        }))
        .await;

        assert_eq!(status, StatusCode::NO_CONTENT);
        assert!(body.is_empty());
    }

    #[tokio::test]
    async fn notification_errors_emit_no_response_bytes() {
        let (status, body) = post_json(json!({
            "jsonrpc": "2.0",
            "method": "notifications/cancelled"
        }))
        .await;

        assert_eq!(status, StatusCode::NO_CONTENT);
        assert!(body.is_empty());
    }

    #[tokio::test]
    async fn request_returns_id_matched_json_rpc_response() {
        let (status, body) = post_json(json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "initialize"
        }))
        .await;

        assert_eq!(status, StatusCode::OK);
        let response: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(response["jsonrpc"], "2.0");
        assert_eq!(response["id"], 7);
        assert!(response["result"].is_object());
        assert!(response.get("error").is_none());
    }

    #[tokio::test]
    async fn cors_preflight_is_limited_to_loopback_origins_and_mcp_headers() {
        let app = build_router(test_state(None));
        let request = |origin: &'static str| {
            axum::http::Request::builder()
                .method(Method::OPTIONS)
                .uri("/mcp")
                .header(header::ORIGIN, origin)
                .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
                .header(
                    header::ACCESS_CONTROL_REQUEST_HEADERS,
                    "content-type, x-mcp-secret",
                )
                .body(Body::empty())
                .unwrap()
        };

        let response = app
            .clone()
            .oneshot(request("http://127.0.0.1:3000"))
            .await
            .unwrap();
        assert!(response.status().is_success());
        assert_eq!(
            response
                .headers()
                .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .and_then(|value| value.to_str().ok()),
            Some("http://127.0.0.1:3000")
        );
        assert!(response
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_HEADERS)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.contains("x-mcp-secret")));

        let response = app.oneshot(request("https://evil.example")).await.unwrap();
        assert!(!response
            .headers()
            .contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN));
    }
}
