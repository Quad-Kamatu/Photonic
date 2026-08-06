#![recursion_limit = "2048"]
pub mod auth;
pub mod dispatch;

pub mod handlers;
pub mod protocol;
pub mod schema_gen;
pub mod server;
pub mod catalog;
pub mod path_guard;
pub mod stdio;

pub use handlers::doc_export::register_export_gpu;
pub use server::{McpServer, McpServerConfig};
