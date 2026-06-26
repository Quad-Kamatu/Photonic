//! Dump the MCP tool-list manifest as pretty JSON to stdout.
//!
//! Used to regenerate `docs/mcp-api.md` so the reference can never drift from
//! the code:
//!
//! ```sh
//! cargo run -p photonic-mcp --bin dump_tools | python3 tools/gen-mcp-docs.py > docs/mcp-api.md
//! ```

fn main() {
    let tools = photonic_mcp::server::tool_list();
    match serde_json::to_string_pretty(&tools) {
        Ok(s) => println!("{s}"),
        Err(e) => {
            eprintln!("failed to serialize tool list: {e}");
            std::process::exit(1);
        }
    }
}
