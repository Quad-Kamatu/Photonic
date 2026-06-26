# Documentation accuracy: auto-generate MCP API reference; refresh architecture docs (#62)

> Status: **implemented.**

## What this PR implements

- **Auto-generated MCP reference.** `server::tool_list()` is now `pub`, and a new
  `photonic-mcp` binary `dump_tools` prints the canonical tool schema as JSON
  without standing up a server. `tools/gen-mcp-docs.py` turns that JSON into
  `docs/mcp-api.md` — one section per tool with a parameter table (type,
  required, description). Output is deterministic (tools and params sorted).
- **Regenerated `docs/mcp-api.md`**: now covers all **283** tools (was ~100,
  heavily drifted). Regenerate with:
  ```sh
  cargo run -p photonic-mcp --bin dump_tools | python3 tools/gen-mcp-docs.py > docs/mcp-api.md
  ```
- **CI drift gate**: the lint job regenerates the file and runs
  `git diff --exit-code docs/mcp-api.md`, so the reference can never silently
  drift from the code again.
- **`docs/architecture.md`**: handler-module table corrected to all nine modules
  under `crates/photonic-mcp/src/handlers/` (was five), and pointed at the
  generated reference.
- **`ROADMAP.md`**: marked historical with a banner pointing to the generated
  `docs/mcp-api.md` as the authoritative tool list (content retained for
  rationale/history rather than deleted).

Verified locally: `dump_tools` runs, the generator is deterministic (two runs
byte-identical), the committed doc matches a fresh regeneration, the workflow
YAML parses, and `photonic-mcp` builds.

> Note: `tool_list()` contains two duplicate registrations
> (`make_compound_path`, `release_compound_path`); the generator faithfully
> reflects the source. De-duplicating the manifest is a small separate cleanup.

---

> Original design scaffold follows.

## Summary

Three documentation artifacts have drifted significantly from the code:

- `docs/mcp-api.md` documents ~100 tools; `crates/photonic-mcp/src/server.rs` exposes ~260+
  (the `tool_list()` function at line 1980 has 380 `"name"` occurrences, reflecting the full schema).
- `docs/architecture.md` describes `SceneNodeKind` as Path/Group/Text only (which is currently
  correct in `crates/photonic-core/src/node.rs:238`), but lists only "20+ handler functions"
  and a handler-module table that no longer matches the nine modules now under
  `crates/photonic-mcp/src/handlers/` (annotations, audit, canvas, clipboard, color_guide,
  document, layers, nodes, transforms).
- `ROADMAP.md` lists Phase A–E items, most of which are done; it predates milestones M1–M8.

The proposal: make `mcp-api.md` auto-generated so it cannot drift; update `architecture.md`
to current reality; replace `ROADMAP.md` with a pointer to GitHub milestones.

## Scope

**In:**
- A Rust binary or build script that reads `tool_list()` output (or a JSON dump) and emits `docs/mcp-api.md`.
- CI gate: generated file is checked in; CI regenerates it and fails if the diff is non-empty.
- `docs/architecture.md` update: correct handler table, `SceneNodeKind` variants, handler module list, crate dependency graph.
- `ROADMAP.md` retirement: replace body with "See GitHub milestones M1–M8" link.

**Out:**
- Generating docs for `photonic-core` internals (use `cargo doc` for that).
- Automatically updating `architecture.md` (it remains a hand-authored document; only regeneration of `mcp-api.md` is automated).

## Proposed approach

1. **Dump tool schema to JSON**: Add a `--dump-tools` CLI flag to `photonic-app` (or a small standalone binary in `crates/photonic-mcp`) that calls `tool_list()` in `crates/photonic-mcp/src/server.rs` (already returns a `serde_json::Value`) and prints it to stdout. Example:
   ```
   cargo run -p photonic-app -- --dump-tools > /tmp/tools.json
   ```

2. **Generator script** (`tools/gen-mcp-docs.py` or a Rust binary `tools/gen-docs`): reads `tools.json`, emits a Markdown table per tool (name, description, required params, optional params). Template:
   ```markdown
   ## `create_shape`
   | Param | Type | Required | Description |
   | ... |
   ```

3. **CI integration**: In `.github/workflows/ci.yml`, add a step after build:
   ```yaml
   - name: Regenerate MCP API docs
     run: cargo run -p photonic-app -- --dump-tools | python3 tools/gen-mcp-docs.py > docs/mcp-api.md
   - name: Check docs are up to date
     run: git diff --exit-code docs/mcp-api.md
   ```

4. **`architecture.md` manual update** (one-time, hand-authored):
   - Correct handler table to: `annotations`, `audit`, `canvas`, `clipboard`, `color_guide`, `document`, `layers`, `nodes`, `transforms` (matching `crates/photonic-mcp/src/handlers/`).
   - `SceneNodeKind`: confirm current variants (Path, Group, Text from `node.rs:238`); note that appearance, effects, symbols etc. are properties of nodes, not separate node kinds.
   - Add crate dependency diagram: `photonic-app` → `photonic-gui` + `photonic-mcp`; both → `photonic-core`; `photonic-render` ← `photonic-core`.
   - Replace "20+ handler functions" with actual count.

5. **`ROADMAP.md`**: Replace content with a brief note: "The original Phase A–E roadmap is complete. For current and planned work, see [GitHub Milestones](https://github.com/Quad-Kamatu/Photonic/milestones)."

## Affected modules

- `crates/photonic-mcp/src/server.rs` — expose `tool_list()` as public if not already; add `--dump-tools` entrypoint
- `crates/photonic-app/src/args.rs` — add `--dump-tools` flag to `Args` struct (Clap)
- `crates/photonic-app/src/main.rs` — handle `--dump-tools` early exit path
- `.github/workflows/ci.yml` — add doc-freshness check step
- `docs/mcp-api.md` — auto-generated output (do not hand-edit after this)
- `docs/architecture.md` — hand-updated (handler table, node kinds, crate graph)
- `ROADMAP.md` — replaced with milestone pointer

## Risks & open questions

- **`tool_list()` currently returns inline JSON**: The function at `server.rs:1980` returns a hardcoded `serde_json::json!{...}` value. If tool descriptions drift from the implementation, the generation only reflects what is in that static JSON, not the actual dispatch table. The real fix is to derive the schema from a typed `ToolDef` registry — but that is a larger refactor; the immediate win is still preventing the doc from lagging the JSON.
- **Generator language**: Python is simpler to write quickly; a Rust binary avoids toolchain dependency in CI. Recommend Python for now (already in CI runner).
- Open Q: Should `docs/mcp-api.md` be grouped by handler module (matching `handlers/`) or alphabetical?

## Acceptance criteria

- [ ] `docs/mcp-api.md` is generated by script and documents all tools exposed by `tool_list()`.
- [ ] CI fails if `mcp-api.md` is stale relative to the tool list.
- [ ] `docs/architecture.md` lists all nine handler modules and reflects the correct `SceneNodeKind` variants.
- [ ] `ROADMAP.md` no longer describes completed Phase A–E work; links to milestones.

## Effort estimate

**S** — Mostly plumbing and writing. The CI integration is the most finicky part.
