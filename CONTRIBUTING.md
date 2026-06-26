# Contributing to Photonic

Thanks for your interest in contributing! This document explains how to get set
up, the conventions we follow, and how changes get merged.

By participating, you agree to abide by our [Code of Conduct](CODE_OF_CONDUCT.md).

## Getting set up

Photonic is a Rust workspace. You need a stable Rust toolchain (install via
[rustup](https://rustup.rs)).

On Linux you also need system libraries for the GPU/GUI stack (winit, wgpu, egui),
GTK file dialogs, and OpenSSL:

```sh
sudo apt-get install -y --no-install-recommends \
  pkg-config libssl-dev libgtk-3-dev \
  libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev \
  libx11-dev libxcursor-dev libxi-dev libxrandr-dev \
  libxcb1-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev
```

macOS and Windows need no extra packages beyond the standard toolchain.

Build and test the whole workspace:

```sh
cargo build --workspace
cargo test --workspace
```

See [README.md](README.md) for how to run the GUI, the MCP server, and the Lua
REPL, and [docs/architecture.md](docs/architecture.md) for the crate layout.

## Before you open a pull request

CI runs on every pull request and must pass before merge. Run these locally first:

```sh
cargo fmt --all --check     # formatting is enforced (blocking in CI)
cargo clippy --workspace --all-targets   # keep new code warning-free
cargo build --workspace
cargo test --workspace
```

- **Formatting** is enforced — run `cargo fmt --all` before committing.
- **Clippy** runs in CI. Warnings are not yet denied (there is an existing
  backlog), but please don't add new ones.

## Dependencies & licensing

Photonic is MIT-licensed, and we only depend on crates under permissive licenses
compatible with MIT redistribution (MIT, Apache-2.0, BSD, ISC, Zlib, Unicode,
etc.). If you add a dependency:

- Make sure its license is on the allow-list in [`deny.toml`](deny.toml).
- Regenerate the third-party notices so the new dependency's license is included:

  ```sh
  cargo install cargo-about --features cli   # one-time
  cargo about generate about.hbs -o THIRD-PARTY-NOTICES.md
  ```

- Optionally run `cargo install cargo-deny --locked && cargo deny check` to
  verify license and advisory compliance.

By submitting a contribution, you agree that your work is licensed under the
project's [MIT License](LICENSE).

## How changes get merged

`main` is protected. To land a change:

1. Create a branch (or fork) and push your work.
2. Open a pull request against `main`.
3. CI must pass and the PR needs **one approving review**. Resolve all review
   conversations before merging.
4. Squash or rebase as appropriate; keep history readable.

## Commit & PR style

- Write clear, imperative commit messages ("Add gradient editor", not "added").
- Keep PRs focused — one logical change per PR is easier to review.
- Reference any related issue (e.g. `Closes #14`).
- Include a short description of what changed and why.

## Reporting bugs & requesting features

Open a [GitHub issue](https://github.com/Quad-Kamatu/Photonic/issues). For bugs,
include steps to reproduce, what you expected, and what happened (plus OS and GPU
if it's a rendering issue).

For **security** issues, do **not** open a public issue — see
[SECURITY.md](SECURITY.md).
