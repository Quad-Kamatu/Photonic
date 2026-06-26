# [EPIC] Test Coverage: Only 17 Tests Exist, All in photonic-core (#53) — Design Proposal

> Status: design scaffold (not an implementation).

## Summary

The workspace has 17 test functions (grep confirms ~22 `#[test]` markers, some in helper
functions — the issue's "17" counts distinct test cases). All live in three files inside
`photonic-core`: `export.rs`, `import.rs`, and `history.rs`. The other four crates —
`photonic-mcp` (260+ tool handlers), `photonic-render`, `photonic-gui`, and
`photonic-app` — have zero tests. Critical subsystems with no coverage include boolean
path operations (`photonic-core/src/ops/boolean.rs`) and tessellation
(`photonic-render/src/tessellator.rs`). This is a P1 quality risk for a tool shipping
cross-platform binaries.

## Scope

**In**
- MCP handler integration tests (spin up the server or call handlers directly) for all
  major tool categories: nodes, layers, document, transforms, clipboard, annotations.
- Boolean/path-ops unit tests: union, subtract, intersect, exclude, divide, offset,
  outline, join, simplify with geometric assertions.
- `photonic-core` model property tests: undo/redo invariants across all `Command`
  variants; dual-index invariant (TD-002 per issue).
- Export golden tests: SVG string snapshots (`insta`) + headless PNG hash for
  representative documents.
- Coverage reporting wired into CI.

**Out**
- GUI rendering correctness tests (covered by issue #54 — visual regression harness).
- End-to-end UI interaction tests (out of scope for this issue).

## Proposed Approach

### 1. MCP handler tests (`crates/photonic-mcp/tests/`)

Create an integration test binary `crates/photonic-mcp/tests/handlers.rs`.  
Use `AppState::new_default()` (or whatever the server initialisation path is in
`photonic-mcp/src/server.rs`) to build state without a network socket.  
Call handler functions from `photonic-mcp/src/handlers/` directly — they are `pub async fn`
and take `&AppState`. No HTTP roundtrip needed for unit-style handler tests; add a
separate `tests/e2e_server.rs` that binds a local port for true JSON-RPC tests.  
Organise tests by tool category matching `server.rs` dispatch blocks.

### 2. Boolean ops tests (`crates/photonic-core/src/ops/boolean.rs`)

Add a `#[cfg(test)]` module at the bottom of `boolean.rs`. Tests should:
- Construct simple `PathData` rectangles/circles.
- Call `boolean_op(a, b, BooleanOp::Union)` etc. and assert area / point containment.
- Test degenerate cases: disjoint shapes, coincident edges, empty result.

### 3. Core model property tests

Extend `crates/photonic-core/src/history.rs` test module with:
- Round-trip for each `Command` variant: execute → undo → assert state equals pre-execute.
- Redo after undo restores post-execute state.
- Capacity limit: exceed `CommandHistory::new(200)` depth, assert oldest is evicted.
- Dual-index invariant: after any mutation, `document.nodes` and layer order are consistent.

### 4. Export snapshot tests

Add `crates/photonic-core/tests/export_snapshots.rs` using the `insta` crate for SVG
string snapshots. One fixture per feature: solid fill, gradient, mesh gradient, stroke,
group, text, blend mode. `UPDATE_SNAPSHOTS=1` refreshes committed `.snap` files.

For PNG hashes: call `HeadlessRenderer` from `photonic-render/src/headless.rs`, render
each fixture at 256×256, SHA-256 the raw bytes, compare to committed hashes in a text
fixture. (Hashes are fragile across GPU drivers; consider pixel-exact comparison only on
Linux CI runner and skip on Windows/macOS or use tolerance — see #54 for the full
harness.)

### 5. Coverage reporting

Add `cargo-llvm-cov` (preferred over tarpaulin for speed on this GPU-heavy workspace) to
CI:

```yaml
# in .github/workflows/ci.yml, new step after tests:
- name: Coverage (llvm-cov)
  run: |
    rustup component add llvm-tools-preview
    cargo install cargo-llvm-cov --locked
    cargo llvm-cov --workspace --lcov --output-path lcov.info
- uses: codecov/codecov-action@v4
  with:
    files: lcov.info
```

Exclude `photonic-gui` and `photonic-app` from coverage (no headless GUI runner) via
`.cargo/config.toml` or `#[cfg(not(tarpaulin))]` guards where needed.

## Affected Modules

- `crates/photonic-core/src/ops/boolean.rs` — new test module
- `crates/photonic-core/src/history.rs` — extend existing test module
- `crates/photonic-core/src/export.rs` — existing snapshot tests, extend
- `crates/photonic-core/tests/` — new integration test directory
- `crates/photonic-mcp/tests/` — new directory: `handlers.rs`, `e2e_server.rs`
- `crates/photonic-render/` — used by export snapshot tests via `headless.rs`
- `.github/workflows/ci.yml` — coverage step

## Risks & Open Questions

- `photonic-mcp` handler tests need `AppState` which may hold GPU or async resources.
  Check if `AppState::new_default()` is cheap to construct in a test context; if not,
  a test-mode factory or mock `DocumentStore` may be needed.
- The `geo` crate's `BooleanOps` (used in `ops/boolean.rs`) has known edge-case panics
  on degenerate geometry. Tests should catch panics with `std::panic::catch_unwind`.
- Headless PNG rendering in CI requires a GPU or software rasterizer. The existing CI
  Linux runner may need `mesa-vulkan-drivers` (or `wgpu`'s `dx12`/`metal` backend on
  Windows/macOS). Defer headless PNG tests to a separate job or skip via env flag.
- Adding `cargo-llvm-cov` increases CI time; consider running coverage only on `main`
  pushes, not every PR.

## Acceptance Criteria

- [ ] Every crate has at least one `#[cfg(test)]` module with meaningful tests.
- [ ] All `BooleanOp` variants have passing geometric assertion tests.
- [ ] `CommandHistory` undo/redo invariants are tested for at least 5 `Command` variants.
- [ ] SVG snapshot tests pass with `insta` and committed `.snap` files.
- [ ] Coverage percentage is reported per PR (Codecov or similar).
- [ ] CI does not regress on existing 17 tests.

## Effort Estimate

**XL** — this is an epic; each sub-item is M on its own. Expected to be broken into
child issues and worked across multiple milestones.
