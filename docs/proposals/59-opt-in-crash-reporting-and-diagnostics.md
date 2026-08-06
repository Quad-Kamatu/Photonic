# Opt-in crash reporting and diagnostics (#59) — Design Proposal

> Status: design scaffold (not an implementation).

## Summary

Photonic already writes a local panic log (`crates/photonic-app/src/main.rs:60–79`):
a `std::panic::set_hook` captures the panic info and appends it to `photonic.log` via
`tracing_appender`. However that log stays on the user's machine, making field crashes
invisible. This proposal adds opt-in remote crash reporting plus in-app helpers for
log access.

## Scope

**In:**
- Extend the existing panic hook (lines 60–79 of `main.rs`) to, with user consent, POST a crash report to a configurable endpoint.
- Consent dialog on first launch (off by default); stored in `AppPreferences`.
- Capture: panic message + location, backtrace, OS/arch, app version, and a redacted scene summary (node count, not content).
- "Open log folder" button in Help menu.
- "Report a bug" link that pre-fills a GitHub issue template.

**Out:**
- Full minidump / core-file capture (out of scope for Rust panic-hook approach).
- Any PII collection beyond what the user explicitly adds.
- Sentry SDK (licensing/cost concern; prefer a self-hosted or lightweight alternative).

## Proposed approach

1. **Consent flag**: Add `crash_reporting_enabled: bool` (default `false`) to `AppPreferences` in `crates/photonic-gui/src/preferences.rs`. Show a first-run dialog in `crates/photonic-gui/src/welcome.rs` (already exists) or `app.rs`.

2. **Extend panic hook** in `crates/photonic-app/src/main.rs` (after line 79): if `crash_reporting_enabled` (read from a `std::sync::OnceLock<bool>` set at startup), serialize a compact JSON report and send it via `reqwest::blocking::Client` (already in `Cargo.toml`) to the reporting endpoint. Block in the hook is acceptable since the process is dying.

3. **Crash report schema**:
   ```json
   { "version": "0.1.0", "os": "linux/x86_64", "panic": "...", "location": "...",
     "backtrace": "...", "scene_nodes": 42, "timestamp": "..." }
   ```

4. **Reporting backend**: Initially, file a GitHub Issue via the GitHub Issues API (no server needed). Longer-term, a small self-hosted endpoint (Rust / `axum`) that deduplicates by panic location and appends to a log.

5. **"Open log folder"**: In `crates/photonic-gui/src/app.rs` Help menu, `std::process::Command::new("xdg-open").arg(log_dir)` on Linux, `explorer` on Windows, `open` on macOS.

6. **"Report a bug"**: Construct a GitHub new-issue URL with pre-filled body containing version + OS; `open::that(url)` via the `open` crate.

## Affected modules

- `crates/photonic-app/src/main.rs` — extend panic hook (lines 60–79), set `OnceLock` consent flag at startup
- `crates/photonic-gui/src/preferences.rs` — `AppPreferences::crash_reporting_enabled: bool`
- `crates/photonic-gui/src/welcome.rs` — consent prompt on first run
- `crates/photonic-gui/src/app.rs` — Help menu items ("Open log folder", "Report a bug")
- `crates/photonic-app/Cargo.toml` — add `open` crate; `reqwest` already present

## Risks & open questions

- **Privacy**: Backtrace may contain user file paths. Must sanitize or let users review before sending.
- **Blocking in panic hook**: `reqwest::blocking` in a panic hook is generally OK since the process is dying, but it will hang if the network is slow or down — add a timeout (e.g. 2 s).
- **Wayland**: `xdg-open` should work for log folder; test on GNOME + KDE.
- Open Q: Should we use a GitHub App token for issue filing, or a dedicated lightweight ingest endpoint?
- Open Q: Where should the self-hosted endpoint live (Dokploy server, existing infra)?

## Acceptance criteria

- [ ] Crash reporting is **off by default**; user must explicitly opt in.
- [ ] A consent dialog clearly explains what is collected.
- [ ] On a test panic, with consent on: a crash report is delivered to the endpoint with version + OS + backtrace.
- [ ] "Open log folder" opens the directory containing `photonic.log`.
- [ ] "Report a bug" opens a pre-filled GitHub issue in the browser.
- [ ] Disabling consent in Settings stops all transmissions.

## Effort estimate

**S** — The hook already exists; this is plumbing + a small UI. The biggest variable is deciding on and standing up the ingest endpoint.
