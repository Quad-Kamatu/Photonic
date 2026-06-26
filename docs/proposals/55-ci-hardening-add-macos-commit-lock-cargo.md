# CI Hardening: Add macOS, Commit + Lock Cargo.lock, Deny Clippy Warnings (#55) — Design Proposal

> Status: design scaffold (not an implementation).

## Summary

`.github/workflows/ci.yml` currently builds and tests on `ubuntu-latest` and
`windows-latest` only, with three known gaps:

1. **No macOS runner** despite the app targeting all three desktop platforms.
2. **`--locked` is intentionally omitted** (comment in `ci.yml` line ~43) because
   `Cargo.lock` is listed in `.gitignore` (line 2). Builds use whatever `cargo` resolves
   at run time, meaning dependency upgrades can silently break CI.
3. **Clippy runs without `-D warnings`** (`ci.yml` lint job, last step). The comment
   notes ~50 warnings in backlog. `cargo-deny` exists (`deny.toml`) but is not wired
   into CI.

## Scope

**In**
- Add `macos-latest` to the `build-test` matrix.
- Remove `Cargo.lock` from `.gitignore`, commit it, re-add `--locked` to all `cargo
  build` / `cargo test` invocations in CI.
- Fix the ~50 outstanding clippy warnings, then add `-D warnings` to the clippy step.
- Wire `cargo deny check` into the lint job.
- Cache and fail-fast tuning.

**Out**
- ARM64 macOS runner (`macos-14`) — can be added later; `macos-latest` (x86) is the
  baseline.
- MSRV pinning (separate issue).
- Code coverage (issue #53).
- Visual regression (issue #54).

## Proposed Approach

### Step 1 — Commit Cargo.lock

```bash
# One-time local action
sed -i '/^Cargo\.lock$/d' .gitignore
git add Cargo.lock
git commit -m "chore: track Cargo.lock for reproducible builds"
```

After this, all CI `cargo` invocations should add `--locked`:

```yaml
- name: Build (workspace)
  run: cargo build --workspace --locked

- name: Test (workspace)
  run: cargo test --workspace --locked
```

### Step 2 — Add macOS to the matrix

```yaml
# ci.yml, jobs.build-test.strategy.matrix
os: [ubuntu-latest, windows-latest, macos-latest]
```

macOS does not need the Linux `apt-get` block. The `if: runner.os == 'Linux'`
guard on the system-dep step already handles this correctly. Verify that the
wgpu/winit/egui stack builds on macOS without additional brew deps (metal backend
is used; no X11 packages needed).

### Step 3 — Clear clippy backlog + enforce `-D warnings`

Enumerate current warnings:

```bash
cargo clippy --workspace --all-targets 2>&1 | grep "^warning" | sort | uniq -c | sort -rn
```

Common categories to expect in a GPU/egui codebase: dead_code, unused_variables,
clippy::too_many_arguments, clippy::single_match, clippy::needless_return. Address
these before the flag change. Silence genuinely intentional warnings with
`#[allow(clippy::...)]` at the site (with a comment explaining why) rather than
workspace-level `[lints]` suppression.

Once clean, update the clippy step in `ci.yml`:

```yaml
- name: clippy
  run: cargo clippy --workspace --all-targets -- -D warnings
```

### Step 4 — Wire `cargo deny` into CI

`deny.toml` is already maintained. Add to the lint job:

```yaml
- name: cargo deny
  uses: EmbarkStudios/cargo-deny-action@v2
  with:
    command: check licenses advisories
```

This catches new dependencies with non-permissive licenses (the `deny.toml` allow-list
covers MIT/Apache-2.0/BSD etc.) and any known security advisories.

### Step 5 — Cache + fail-fast tuning

The existing `Swatinem/rust-cache@v2` step is sufficient. Set `fail-fast: false` on the
matrix (already set in `build-test`) so a macOS failure doesn't abort Linux/Windows runs.
Consider splitting lint into its own fast-path job that runs before `build-test` to give
early signal on formatting issues.

## Affected Modules

- `.github/workflows/ci.yml` — matrix, `--locked`, `-D warnings`, `cargo deny` step
- `.gitignore` — remove `Cargo.lock` entry (line 2)
- `Cargo.lock` — commit to repository
- Scattered `#[allow(...)]` additions across all five crates where warnings are
  intentional

## Risks & Open Questions

- **macOS build time**: `macos-latest` runners are slower and more expensive than Linux.
  If build time is a concern, gate macOS on `push` to `main` only (not every PR), using
  `if: github.event_name == 'push'` on that matrix row.
- **wgpu on macOS CI**: `HeadlessRenderer` tests (issue #54) would need Metal; for basic
  `cargo build` + `cargo test` (unit tests only), no GPU is required. Confirm no test
  calls into wgpu without a `#[cfg(not(ci))]` guard.
- **Clippy version drift**: clippy lints change between Rust releases. Pinning the Rust
  toolchain version in `dtolnay/rust-toolchain@stable` or adding a `rust-toolchain.toml`
  prevents surprise lint additions on minor Rust updates from breaking CI.
- **`--locked` on a new machine**: any contributor who doesn't `git pull` the lock file
  before adding a dep will get an error. Document in `CONTRIBUTING.md` that `cargo add`
  should be followed by committing the updated `Cargo.lock`.

## Acceptance Criteria

- [ ] `Cargo.lock` is committed and `--locked` is used in CI build and test steps.
- [ ] CI matrix includes `macos-latest`; all three OS jobs pass.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` passes with zero warnings.
- [ ] `cargo deny check licenses advisories` passes in CI.
- [ ] `cargo fmt --all --check` continues to pass (already enforced).

## Effort Estimate

**M** — macOS matrix and `--locked` are mechanical. Clearing the clippy backlog is the
variable-cost item; 50 warnings could be an afternoon or a week depending on complexity.
