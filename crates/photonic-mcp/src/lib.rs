#![recursion_limit = "1024"]
// `pub` (29 §3 / CAP-019): the acceptance-story harness in
// `crates/photonic-app/tests/` must drive the real tool-call path — the
// same `dispatch_tool` entry that carries document-snapshot/undo/audit-log
// side effects — from outside this crate. `dispatch_tool_inner` stays
// `pub(crate)`.
pub mod dispatch;

pub mod auth;
pub mod handlers;
pub mod protocol;
pub mod schema_gen;
pub mod server;
pub mod stdio;

pub use handlers::doc_export::register_export_gpu;
pub use server::{McpServer, McpServerConfig};
