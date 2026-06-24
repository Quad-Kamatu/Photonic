# Photonic

A vector graphics editor built in Rust with a native GPU-accelerated GUI and an integrated MCP server for AI-assisted design via Claude.

## Features

- **Native GUI** using egui and wgpu (GPU-accelerated, cross-platform)
- **MCP server** — Claude can create and edit vector art through a JSON-RPC API
- **Full vector toolset** — shapes, bezier paths, boolean operations, gradients, transforms
- **Undo/redo history** with named checkpoints for time-travel
- **SVG / PNG / JPEG export**
- **Lua scripting** for automation

## Quick Start

### Building

```sh
cargo build --release
```

### Running (GUI mode)

```sh
cargo run --release
# or open a saved document:
cargo run --release -- path/to/file.photonic
```

### Running the MCP server (headless)

```sh
cargo run --release -- mcp --port 7842
```

The MCP server listens on `http://localhost:7842` and accepts JSON-RPC 2.0 requests.

### Lua REPL

```sh
cargo run --release -- repl
```

## Project Layout

```
photonic/
├── crates/
│   ├── photonic-core/     # Data structures & business logic
│   ├── photonic-render/   # GPU rendering (wgpu + lyon)
│   ├── photonic-gui/      # egui GUI
│   ├── photonic-mcp/      # MCP server & JSON-RPC handlers
│   └── photonic-app/      # Binary entry point
├── docs/
│   ├── architecture.md    # Crate design and internals
│   ├── mcp-api.md         # MCP tool reference
│   └── file-format.md     # .photonic file format
└── ROADMAP.md             # Planned features
```

## Documentation

| Document | Contents |
|---|---|
| [docs/architecture.md](docs/architecture.md) | Crate breakdown, data model, concurrency model |
| [docs/mcp-api.md](docs/mcp-api.md) | Every MCP tool with parameters and examples |
| [docs/file-format.md](docs/file-format.md) | `.photonic` JSON schema reference |

## Crates at a Glance

| Crate | Role |
|---|---|
| `photonic-core` | `Document`, `SceneNode`, `Transform`, `Fill`, `CommandHistory` |
| `photonic-render` | `PhotonicRenderer` (wgpu), `HeadlessRenderer`, tessellator |
| `photonic-gui` | `PhotonicApp`, panels, tool implementations |
| `photonic-mcp` | JSON-RPC server, 20+ handler functions |
| `photonic-app` | `main()`, logging, mode dispatch |

## Key Technologies

| Area | Library |
|---|---|
| GPU rendering | wgpu |
| Path tessellation | lyon |
| Bezier geometry | kurbo |
| GUI | egui + egui-wgpu |
| Async runtime / HTTP | tokio + axum |
| Boolean path ops | geo |
| Serialization | serde_json |
| Scripting | mlua (Lua 5.4) |
