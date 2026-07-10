# tools/

Standalone, stdlib-only Python scripts. Each file in this directory is
self-contained — no shared local module, no `requirements.txt` — matching
the "one script, one job" convention started by `gen-mcp-docs.py`.

## Docs / fixtures generators

- **`gen-mcp-docs.py`** — regenerates `docs/mcp-api.md` from the live tool
  list (`cargo run -p photonic-mcp --bin dump_tools | python3
  tools/gen-mcp-docs.py > docs/mcp-api.md`). No server needed — reads the
  tool manifest straight from `photonic_mcp::server::tool_list()`.
- **`gen-test-fixtures.py`** — regenerates the video-editor test-media
  corpus under `crates/photonic-video/tests/fixtures/`. ffmpeg-dependent,
  run locally when a fixture needs to change; CI consumes the committed
  output rather than regenerating it. See that directory's own `README.md`.

## Acceptance-story MCP scripts

- **`as1_arrange_cut.py`** — AS-1 "Social clip" story, arrange + cut slice
  (create sequence, import fixtures, insert/split/move/trim/ripple,
  `render_frame_at` spot-checks, export).
- **`as2_proxy_edit.py`** — AS-2 "Short film" story, proxy-edit slice
  (import a heavy fixture, `generate_proxies`, toggle `set_proxy_mode`,
  edit).

Both are scripts, not `cargo test`s: they drive a **running** Photonic MCP
server over HTTP JSON-RPC (`POST /mcp`), the same protocol a real MCP client
(e.g. Claude) speaks. `docs/specs/video-editor/11-testing-phasing.md` §3.4
calls for one such script per acceptance story (AS-1/2/3), expanded
incrementally as each phase lands more of that story; these two cover the
slice available as of P3 (see each script's own module docstring for
exactly what it does and doesn't cover yet).

### Running them

```sh
# 1. Launch a headless Photonic MCP server (no GUI window) in one terminal:
cargo run -p photonic-app -- --headless
# (add --mcp-port <N> to use a non-default port; default is 7842)

# 2. Run a story script against it from another terminal:
python3 tools/as1_arrange_cut.py
python3 tools/as2_proxy_edit.py

# Point at a non-default port/host:
python3 tools/as1_arrange_cut.py --url http://127.0.0.1:7842/mcp
# or:
PHOTONIC_MCP_URL=http://127.0.0.1:7842/mcp python3 tools/as1_arrange_cut.py
```

A running GUI instance also serves the same `/mcp` endpoint, so pointing
`--url` at one works too — headless is just the lighter way to get a server
up for scripted runs.

### Output and exit codes

Each script prints one `[PASS]` / `[FAIL]` / `[SKIP]` line per step and a
final `N/M passed, S skipped, F failed` summary. Exit code is `0` iff
nothing failed (skips don't count as failures — they're reserved for
environment gaps the script can't control, e.g. no GPU adapter for the
engine-backed tools, or no `ffmpeg` on `PATH` for export). A nonzero exit
means at least one step genuinely failed, which is what makes these
CI-runnable once a GPU + ffmpeg runner is wired up (currently manual /
local-only — see `11-testing-phasing.md` §3.4).

### Fixtures

Both scripts import from the committed corpus at
`crates/photonic-video/tests/fixtures/`. If a fixture is missing, regenerate
the whole corpus with `python3 tools/gen-test-fixtures.py` (needs `ffmpeg`
on `PATH`). No 4K fixture exists in that corpus (everything is kept tiny to
stay inside its size budget) — `as2_proxy_edit.py` uses the corpus's
largest/longest real asset as a stand-in; see that script's module
docstring for why that's still a meaningful test of the proxy-edit slice.
