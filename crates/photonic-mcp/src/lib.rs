#![recursion_limit = "1024"]
pub mod dispatch;

pub mod handlers;
pub mod protocol;
pub mod schema_gen;
pub mod server;

pub use handlers::doc_export::register_export_gpu;
pub use server::{McpServer, McpServerConfig, MCP_SECRET_HEADER};
