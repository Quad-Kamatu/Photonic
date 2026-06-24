//! Stdio → HTTP MCP proxy.
//!
//! Reads newline-delimited JSON-RPC messages from stdin and forwards each
//! to the running Photonic MCP server, writing the response back to stdout.
//! This lets the `claude` CLI connect to the already-running MCP server via
//! the stdio transport used in `--mcp-config`.

use anyhow::Result;
use std::io::{BufRead, BufReader, Write};

pub fn run(host_port: &str) -> Result<()> {
    let url = format!("http://{host_port}/mcp");

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?;

    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let reader = BufReader::new(stdin.lock());
    let mut out = stdout.lock();

    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        // Notifications (no "id" field) must not receive a response back on stdout.
        // We still forward them to the server so it can update state, but we discard
        // the HTTP response instead of writing it to Claude Code's stdin.
        let is_notification = serde_json::from_str::<serde_json::Value>(&line)
            .map(|v| !v.as_object().map_or(false, |o| o.contains_key("id")))
            .unwrap_or(false);

        let body = match client
            .post(&url)
            .header("content-type", "application/json")
            .body(line)
            .send()
        {
            Ok(resp) => resp.text().unwrap_or_else(|e| {
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "error": { "code": -32603, "message": format!("Read error: {e}") }
                })
                .to_string()
            }),
            Err(e) => serde_json::json!({
                "jsonrpc": "2.0",
                "error": { "code": -32603, "message": format!("Proxy error: {e}") }
            })
            .to_string(),
        };

        if !is_notification {
            writeln!(out, "{body}")?;
            out.flush()?;
        }
    }

    Ok(())
}
