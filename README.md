# Photonic

A hybrid **vector + raster** graphics editor built in Rust with a native
GPU-accelerated GUI and an integrated MCP server for AI-assisted design via Claude.

## Features

- **Native GUI** using egui and wgpu (GPU-accelerated, cross-platform)
- **MCP server** — Claude can create and edit art through a JSON-RPC API
- **Full vector toolset** — shapes, bezier paths, boolean operations, gradients, transforms
- **Photoshop-grade raster editing** — pixel layers with adjustments (levels,
  curves, hue/saturation, color balance…), filters (gaussian/box/motion blur,
  unsharp mask, median, noise, edges…), a brush engine (paint, erase, clone,
  smudge, dodge/burn), selections and non-destructive layer masks, and the 16
  CSS/Photoshop blend modes. See [docs/raster-editing.md](docs/raster-editing.md).
- **Undo/redo history** with named checkpoints for time-travel
- **SVG / PNG / JPEG / WebP export** (raster layers composite into all formats)
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
| [docs/raster-editing.md](docs/raster-editing.md) | Raster (pixel) editing subsystem — model, ops, MCP surface, phasing |
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

## Contributing

Contributions are welcome! See [CONTRIBUTING.md](CONTRIBUTING.md) for how to set
up, the coding conventions, and the pull-request workflow. Please also read our
[Code of Conduct](CODE_OF_CONDUCT.md). To report a security issue, see
[SECURITY.md](SECURITY.md).

## License

Photonic is licensed under the [MIT License](LICENSE).

It bundles third-party open-source components under their own permissive
licenses; their notices are reproduced in
[THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md).
