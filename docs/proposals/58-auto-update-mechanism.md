# Auto-update mechanism (#58) — Design Proposal

> Status: design scaffold (not an implementation).

## Summary

Photonic ships binaries but has no mechanism to detect, download, or apply new releases.
Users must manually re-download and replace the binary, which slows delivery of bug fixes
and security patches. This proposal adds a background update checker and an opt-in
self-updater backed by GitHub Releases.

## Scope

**In:**
- Background version check against the GitHub Releases API (`https://api.github.com/repos/Quad-Kamatu/Photonic/releases/latest`).
- UI notification ("A new version is available — download?") in the toolbar or a status bar chip.
- Opt-in download + checksum verification before applying (using the existing `reqwest` dependency in `crates/photonic-app/Cargo.toml`).
- Settings toggle for auto-check (wired into `AppPreferences` in `crates/photonic-gui/src/preferences.rs`).
- "Check now" menu action.

**Out:**
- Delta/binary-patch updates (full binary replacement only for v1).
- Automatic silent updates without user consent.
- Flatpak / package-manager–driven updates (those channels self-update).

## Proposed approach

1. **Add `self_update` or manual HTTP crate**: The `self_update` crate (or equivalent hand-rolled code using the already-present `reqwest = "0.12"` in `crates/photonic-app/Cargo.toml`) fetches `/releases/latest` and compares `tag_name` against `env!("CARGO_PKG_VERSION")`.

2. **Background task**: Spawn a `tokio::task::spawn` in `crates/photonic-app/src/main.rs` after GUI init, so it does not block startup. Result is sent back via an `mpsc` channel already used for MCP→GUI communication.

3. **Persist user preference**: Add `check_for_updates: bool` (default `true`) to `AppPreferences` in `crates/photonic-gui/src/preferences.rs`. Persist in the existing `preferences.json`.

4. **UI notification**: In `crates/photonic-gui/src/app.rs`, if a pending update version is set, render a dismissible chip in the top bar. Clicking opens an egui modal that shows release notes excerpt and a download button.

5. **Download + verify**: Download the platform-appropriate binary asset, verify SHA256 against the release's `*.sha256` file, write to a temp path, then ask the OS to replace the running binary on next restart (on Linux: `rename(2)` to overwrite in place; on Windows: use a batch rename helper).

6. **Signature**: Stretch goal — GPG or minisign detached signature verification before applying. Requires adding a public key to the binary.

## Affected modules

- `crates/photonic-app/src/main.rs` — spawn background check task
- `crates/photonic-app/Cargo.toml` — add `self_update` or rely on existing `reqwest`
- `crates/photonic-gui/src/preferences.rs` — `AppPreferences::check_for_updates: bool`
- `crates/photonic-gui/src/app.rs` — update notification UI, "Check now" menu item
- `docs/architecture.md` — note the update subsystem

## Risks & open questions

- **Antivirus false positives**: Self-replacing binaries on Windows are commonly flagged; code signing is near-mandatory there.
- **Sandboxed installs**: Flatpak/Snap restrict network + filesystem access; this flow would need to be gated out on those distributions (detect via `FLATPAK_ID` env).
- **Release pipeline prerequisite**: This is blocked on a working release CI that publishes signed binaries; the current `ci.yml` only builds, not releases.
- **Rollback**: No rollback mechanism is proposed for v1; discuss for v2.
- Open Q: Should the updater restart the app automatically, or only on next launch?

## Acceptance criteria

- [ ] On startup (if `check_for_updates = true`), the app silently queries GitHub Releases without blocking the UI.
- [ ] When a newer version exists, a non-intrusive notification appears.
- [ ] User can trigger a download, checksum is verified before any write.
- [ ] The setting can be disabled; "Check now" works regardless.
- [ ] No crash or UI hang if the network is unavailable.

## Effort estimate

**M** — HTTP call + UI notification is small; safe binary replace + Windows handling + edge-case error UX is the majority of work.
