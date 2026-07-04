# 01 — MCP Crate Decomposition

**Files:** `handlers/nodes.rs` (17,719), `server.rs` (6,791),
`protocol.rs` (4,780), `handlers/document.rs` (5,782).
**Track:** independent of the GUI track. **Highest-mass, highest-mechanical
target — the fleet's main workload.**

The MCP crate exposes ~300 image/vector tools to AI clients. It has the single
best property for delegation: **the CI doc-drift gate regenerates
`docs/mcp-api.md` from `server::tool_list()` and fails on any diff**, so a
handler/schema reshuffle is *mechanically proven* to have dropped or altered no
tool. This is why almost the entire MCP track is Hermes-tier.

---

## The handler contract (invariant across all splits)

```rust
pub async fn <tool>(state: &AppState, args: <Tool>Args) -> ToolResult
```

- `AppState` → document mutex, history, audit log, capture channel, clipboard.
- `<Tool>Args` → a `#[derive(Deserialize)]` struct in `protocol.rs`.
- Dispatch: a single giant match in `server.rs` maps `"tool_name"` → deserialize
  args → call handler → wrap in `ToolOutput::{mutating,readonly}`.
- **Adding a tool today touches 3 places:** `protocol.rs` (args struct),
  `server.rs` (dispatch arm + schema), `handlers/*.rs` (fn). This 3-place edit
  is the core pain that Layer B (the optional macro, WP-1F) removes.

---

## WP-1A — Lift shared helpers  ·  **Codex**  ·  scaffold, blocks 1B

`nodes.rs` has 27 private helpers; a few are load-bearing across domains and
**must be lifted first**, before any handler file is split, or the 9 fan-out
PRs will each try to move the same helper and collide.

Create `handlers/shared/` (or `handlers/util/`):
- `shared/styling.rs` — `apply_style` (**16 callers** — the critical one),
  `apply_stroke_paint`.
- `shared/paths.rs` — `apply_affine_to_path` (8 callers) + the path-distortion
  helpers (`apply_zig_zag`, `apply_pucker_bloat`, `apply_roughen`, `apply_twirl`,
  `apply_round_corners`, `scallop`, `crystallize`, `warp_envelope`).
- `shared/ordering.rs` — `node_z_key` (5 callers).
- `shared/cloning.rs` — `clone_subtree` (3 callers).
- `shared/random.rs` — `xorshift64` (3 callers).
- `shared/document.rs` — the `lock → read → batch → single-undo-step` access
  pattern, ideally as `GetNodeExt` / `BatchUpdateExt` extension traits.

**Why Codex:** choosing the helper module boundaries and the extension-trait
shapes is judgment that the whole fan-out inherits (rubric C4). Small PR, big
leverage. Ends with all of `nodes.rs` still compiling and calling the lifted
helpers via `use`.

**Acceptance:** build+test+fmt green; `docs/mcp-api.md` unchanged; diff is
pure motion of helper bodies + `use` edits.

---

## WP-1B — Split `nodes.rs` into 9 domain handler files  ·  **Hermes ×9**  ·  needs 1A

179 handlers → 9 cohesive files. Each Hermes agent owns exactly one target file
and moves only its handlers (bodies unchanged; they now `use` the 1A helpers).

| Sub-WP | New file | Handlers | ~Lines | Seam quality | Suggested model |
|---|---|---:|---:|---|---|
| 1B-1 | `handlers/shapes.rs` | 19 create + 18 path-edit | ~3,500 | 8/10 | Deepseek V4 |
| 1B-2 | `handlers/pathfinder.rs` | 9 boolean/pathfinder | ~2,300 | 7/10 | Deepseek V4 |
| 1B-3 | `handlers/selection.rs` | 13 select/find | ~3,500 | 6/10 (consider sub-splitting `find_replace_*`) | Deepseek V4 |
| 1B-4 | `handlers/charts.rs` | 6 chart | ~1,100 | **10/10** (zero cross-refs) | Qwen 3.7 |
| 1B-5 | `handlers/transform.rs` | 19 transform/arrange | ~2,200 | 7/10 | Qwen 3.7 |
| 1B-6 | `handlers/typography.rs` | 15 text | ~2,000 | 9/10 | Qwen 3.7 |
| 1B-7 | `handlers/utility.rs` | 25 inspect/measure/misc | ~2,500 | 5/10 (heterogeneous) | Qwen 3.7 |
| 1B-8 | `handlers/guides.rs` | 5 guide/dimension | ~400 | **10/10** | Qwen 3.7 |
| 1B-9 | `handlers/clipping.rs` | 2 clip mask | ~120 | **10/10** | Qwen 3.7 |

`handlers/mod.rs` re-exports so external paths (`handlers::nodes::create_shape`)
still resolve — **or** update the dispatch arms in one pass (see 1D chokepoint
note). Start the fan-out with the 10/10-seam files (charts, guides, clipping) as
warm-up validations of the pipeline, then the harder ones.

**Chokepoint:** all 9 re-point dispatch arms in `server.rs`. **Do 1D first** so
each handler split edits a *different* `dispatch/<domain>.rs` file instead of
the one shared match — eliminating the merge conflict.

**Acceptance per sub-WP:** build+test+fmt green; `docs/mcp-api.md` unchanged;
diff is pure handler-body motion + `use`.

---

## WP-1C — Extract `tool_list()` → `schema_gen.rs`  ·  **Hermes ×1 (Deepseek V4)**  ·  no deps

`server.rs` lines ~2,150–6,791 (**4,641 lines**) are one flat array of 309 inline
JSON tool schemas with zero logic. Move verbatim to
`crates/photonic-mcp/src/schema_gen.rs`, expose `pub fn tool_list()`, re-call
from `server.rs`. Highest-ROI single move in the whole plan (server.rs drops by
68% from this WP alone).

**Why Hermes/Deepseek:** giant, long-context, zero-judgment block move; ideal
for the big-context cheap model. **Acceptance:** `docs/mcp-api.md` byte-identical
(this gate *directly* verifies the schema list survived intact).

---

## WP-1D — Extract dispatch → `dispatch/` module  ·  **Hermes ×1 (Deepseek V4)**  ·  do before 1B

`server.rs` lines ~261–2,143 (**1,883 lines**, 309 match arms). Two options:

- **Minimal:** move the whole match to `dispatch/mod.rs`, re-call from
  `server.rs`. Server drops to ~1,400 lines.
- **Preferred (unblocks 1B parallelism):** partition arms into per-domain files
  `dispatch/{shapes,pathfinder,selection,charts,transform,typography,utility,
  guides,clipping,document,raster,layers,…}.rs`, each a function returning the
  match arms for its domain. Then each 1B handler-split PR edits *its own*
  dispatch file → no shared-file conflicts.

**Why Hermes:** mechanical arm-grouping; the partition mirrors the 1B file list
(already decided). **Acceptance:** build+test green; `docs/mcp-api.md` unchanged.

---

## WP-1E — Split `protocol.rs` Args structs  ·  **Hermes ×4 (Deepseek V4)**  ·  no deps

Lines ~86–4,767 are **296 `#[derive(Deserialize)]` structs, 98% of the file,
zero logic**. Keep the JSON-RPC envelope + MCP handshake (lines 1–84) and SSE
events (4,770–4,780) in `protocol.rs`; move the Args by domain to mirror the
handler split:

- `protocol/args/nodes.rs` (~179 structs)
- `protocol/args/document.rs` (~99 structs)
- `protocol/args/raster.rs` (~15 structs)
- `protocol/args/misc.rs` (remainder)
- `protocol/mod.rs` re-exports all (`pub use args::*;`) so `crate::protocol::FooArgs` keeps resolving → **no handler edits needed**.

**Why Hermes/Deepseek:** the most purely mechanical WP in the plan — identical
boilerplate, no interdependencies. Four agents, one per domain file, or one
agent sequentially. **Acceptance:** build green (proves every struct still
resolves); `docs/mcp-api.md` unchanged.

---

## WP-1F — (Stretch / Layer B) `#[register_tool]` plug-in registry  ·  **Codex**  ·  optional

The 3-place-edit pain is inherent to the current design. A proc-macro:

```rust
#[register_tool(mutates = true)]
pub async fn create_shape(state: &AppState, args: CreateShapeArgs) -> ToolResult { … }
```

generated at compile time (via `inventory`/`linkme` collection) would emit the
dispatch arm **and** the JSON schema from the handler signature + doc comment —
collapsing `protocol.rs` Args, `server.rs` dispatch, and `schema_gen.rs` into a
single source of truth per tool. This is the true "predictable plug-in
component" end-state for the MCP crate.

**Why Codex:** proc-macro design, schema-from-type derivation, and preserving
the exact `tool_list()` JSON (the doc-drift gate is unforgiving) is real
architecture (rubric C1). **Do it only after 1A–1E ship** — the mechanical
split delivers 90% of the navigability win immediately; the macro is a churn-
reduction investment justified only if tool-add frequency stays high.

**Acceptance:** `docs/mcp-api.md` byte-identical (the macro must reproduce every
existing schema exactly); build+test green.

---

## `document.rs` (5,782) — leave as-is for now

99 handlers, **already cleanly grouped by sub-domain** (color, style, symbol,
workspace…). It's long but navigable and low-priority. If touched, apply the
same pattern as 1B (split by sub-domain into `handlers/document/*.rs`), all
Hermes. Not on the critical path.

---

## MCP track summary

| WP | Tier | Model | Deps | Parallelism |
|---|---|---|---|---|
| 1A lift helpers | Codex | — | — | 1 agent, first |
| 1D dispatch → module | Hermes | Deepseek V4 | — | before 1B |
| 1C schema_gen | Hermes | Deepseek V4 | — | anytime |
| 1E protocol args | Hermes | Deepseek V4 | — | ×4, anytime |
| 1B split nodes.rs | Hermes | Deepseek/Qwen | 1A, 1D | ×9 parallel |
| 1F register_tool macro | Codex | — | 1A–1E | optional |

After this track: `nodes.rs`→9 files (~2k avg), `server.rs`→~1.4k,
`protocol.rs`→~85 lines + args submodules. ~29k lines of god module dissolved,
almost entirely by the cheap tier, under a gate that proves 300+ tools intact.
