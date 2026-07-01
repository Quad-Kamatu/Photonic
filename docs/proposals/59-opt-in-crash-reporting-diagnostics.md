# Opt-in Crash Reporting and Diagnostics (#59)

> **Status: implemented.** This PR delivers the full design below end-to-end.
> The sections after this status block are the original design notes, kept for
> context.

## What this PR implements

- **`crates/photonic-core/src/diagnostics.rs`** (new): the shared, dependency-light
  subsystem.
  - `crash_dir()` — the single source of truth for the Photonic config directory
    (APPDATA → XDG_CONFIG_HOME → HOME/.config, joined with `Photonic`).
    `welcome::config_dir` now delegates to it (the duplicated resolver was removed).
  - `reports_dir()` → `<config>/crash-reports/`.
  - `CrashReport { version, timestamp, os, arch, panic_message, location, backtrace }`
    (serde) with `capture(&PanicHookInfo, &Backtrace)`, `write()` →
    `crash-reports/crash-<unix-millis>.json`, and `issue_title()` / `issue_body()`
    builders. Capture records only the app version (`CARGO_PKG_VERSION`), a UTC
    timestamp, `OS`/`ARCH`, the panic payload string, the `&Location`, and the
    formatted backtrace — it never reads document content, file paths, project
    data, or environment variables.
  - `pending_reports()` / `load_report()` / `clear_report()` for the GUI.
  - Unit tests for the UTC conversion, serde round-trip, and issue-title/body.
  - Re-exported from `lib.rs`: `pub use diagnostics::{crash_dir, CrashReport};`.
- **`crates/photonic-app/src/main.rs`**: the existing panic hook keeps its
  `[PANIC]` log line, then captures a backtrace with
  `std::backtrace::Backtrace::force_capture()`, builds a `CrashReport`, and
  `write()`s it. Local capture is unconditional; only sending is gated.
- **`crates/photonic-gui/src/preferences.rs`**: new
  `crash_reporting_consent: Option<bool>` (`#[serde(default)]`, `None` = never
  asked, off by default).
- **`crates/photonic-gui/src/update.rs`**: `REPO_OWNER` / `REPO_NAME` made `pub`
  so the reporting path reuses the same serverless GitHub model as updates.
- **`crates/photonic-gui/src/app/mod.rs`**:
  - `"Privacy & Diagnostics"` added to `EDIT_OPTIONS` with a new `Some(5)` settings
    branch: the consent checkbox, a plain-language collected/excluded disclosure,
    an **Open log folder** button (`explorer`/`open`/`xdg-open`), a **Report a bug**
    button (blank pre-filled issue via `ctx.open_url`), and inline controls to
    report/dismiss any pending reports.
  - On startup, after the update/What's-New checks, a once-per-launch scan via
    `pending_reports()`. If reports exist: consent `None` → a one-time consent
    modal; `Some(true)` → a Report/Dismiss banner; `Some(false)` → silent. Filed
    or dismissed reports are deleted so they are not re-offered.
  - URL helpers: `percent_encode`, `issue_new_base`, `blank_issue_url`,
    `issue_url_for_report` (body bounded to keep the URL within browser limits).
- **`CHANGELOG.md`**: `[Unreleased]` entry added.

Verification: `cargo build --release`, `cargo test -p photonic-core`
(293 lib tests incl. the new diagnostics tests), `cargo test -p photonic-gui`,
`cargo test -p photonic-app`, and `cargo check --workspace` all pass.

## Remaining work

Carried over from the original "Out" scope — intentionally not in this PR:

- Native crash / minidump capture (segfaults, OOM, GPU driver aborts) via
  crashpad/minidump-writer/breakpad. This pass covers Rust **panics** only.
- A hosted ingestion backend / Sentry DSN. The model here is serverless
  (user-reviewed GitHub issue); the HTTP-POST extension point is not wired.
- Symbolication of release backtraces (depends on shipping debug symbols /
  a symbol server) — deferred with minidumps.
- Automatic background upload — every send remains user-initiated by design.

## Summary

Today a panic hook in `crates/photonic-app/src/main.rs` appends `[PANIC] {info}` to
`photonic.log`, but crashes are never surfaced or reportable — field failures are
invisible and hard to reproduce. This proposal adds:

1. **Local structured crash capture** (always on, no privacy concern — it is already
   writing locally): the panic hook writes a self-contained JSON crash report
   (panic message + backtrace + non-sensitive context) to a `crash-reports/`
   folder, in addition to the existing log line.
2. **Opt-in reporting** (off by default): on next launch the GUI detects pending
   crash reports and, only with explicit consent, offers to file them. Consent is a
   one-time dialog explaining exactly what is collected and what is excluded.
3. **UI helpers**: a "Privacy & Diagnostics" settings tab with the consent toggle,
   plus **Open log folder** and **Report a bug** buttons.

Design choice: rather than embedding the full Sentry SDK (heavy dependency, requires
a hosted DSN/server), reporting reuses the project's existing GitHub-centric model —
the "report a bug" path opens a pre-filled GitHub issue (panic summary + backtrace +
environment) in the browser via `ctx.open_url`. This keeps the feature serverless,
consistent with `update.rs` (GitHub Releases) and privacy-reviewable (the user sees
the exact text before anything leaves the machine). An optional HTTP POST endpoint is
left as a small, clearly-marked extension point.

## Scope

### In

- New `crates/photonic-core/src/diagnostics.rs` (shared, no GUI/network deps):
  - `crash_dir()` / config-dir resolution shared with `welcome::config_dir`
    (APPDATA → XDG_CONFIG_HOME → HOME/.config, joined with `Photonic`).
  - `CrashReport { version, timestamp, os, arch, panic_message, location, backtrace }`
    (serde) with `capture(panic_info, backtrace) -> CrashReport` and `write()` →
    `crash-reports/crash-<timestamp>.json`.
  - `pending_reports()` / `clear_report(path)` helpers for the GUI to enumerate and
    dismiss reports.
  - Re-export from `lib.rs` (`pub use diagnostics::{CrashReport, crash_dir};`).
- `crates/photonic-app/src/main.rs` panic hook (lines 61–75): after the existing
  log write, build a `CrashReport` with `std::backtrace::Backtrace::force_capture()`
  and `diagnostics::write()` it. Force `RUST_BACKTRACE=1` capture inside the hook so
  a backtrace is always present. Local capture is **unconditional** — only *sending*
  is gated.
- `crates/photonic-gui/src/preferences.rs`: add
  `crash_reporting_consent: Option<bool>` (`None` = never asked, `Some(false)` =
  declined, `Some(true)` = allowed; `#[serde(default)]`, off by default). Refactor
  `config_dir()` to delegate to `photonic_core::crash_dir`'s helper (single source of
  truth).
- `crates/photonic-gui/src/app/mod.rs`:
  - Add `"Privacy & Diagnostics"` to `EDIT_OPTIONS` (line 136) and a new `Some(5)`
    settings branch mirroring the existing `Some(3)` "Behavior" pattern (~line 2466):
    consent checkbox with a plain-language description of collected/excluded fields;
    **Open log folder** button (reveal the config dir via `open`/explorer/xdg-open or
    `ctx.open_url("file://…")`); **Report a bug** button that opens a pre-filled
    GitHub issue URL.
  - On startup (near the existing update-check block ~line 1854), if
    `diagnostics::pending_reports()` is non-empty: if consent is `None`, show a
    one-time consent dialog; if `Some(true)`, surface a banner offering "Report"
    (open pre-filled issue) / "Dismiss" (clear the file).
- Pre-filled GitHub issue URL builder (reuses `REPO_OWNER`/`REPO_NAME` constants
  already in `update.rs`): title = panic summary, body = backtrace + env, URL-encoded.
- `CHANGELOG.md` `[Unreleased]` entry.

### Out

- **Native crash / minidump capture** (segfaults, OOM, GPU driver aborts via
  `crashpad`/`minidump-writer`/breakpad). That is a large native-toolchain effort per
  platform; this pass covers Rust **panics** (the common, in-process failure) only.
  Noted as the main follow-up.
- A hosted ingestion backend / Sentry DSN. The HTTP-POST path is scaffolded behind
  the consent flag but no server is stood up here.
- Symbolication of release backtraces (depends on shipping debug symbols / a symbol
  server) — deferred with minidumps.
- Automatic background upload without user action — every send is user-initiated in
  this pass to keep the privacy story simple.

## Approach

1. **Capture (core).** Add `diagnostics.rs` to `photonic-core` so both `photonic-app`
   (panic hook) and `photonic-gui` (enumerate/clear) depend on one implementation and
   one directory resolver. `CrashReport::capture` records only non-sensitive context:
   app version (`CARGO_PKG_VERSION`), UTC timestamp, `std::env::consts::OS`/`ARCH`,
   the panic payload string, `&panic::Location`, and the formatted backtrace.
   Explicitly **excluded**: document content, file paths/names, project data, env
   vars. `write()` serializes to `crash-reports/crash-<ts>.json` under the config dir.

2. **Hook wiring (app).** In `main.rs`, keep the existing `[PANIC]` log line, then add
   the structured write. The hook is registered before the GUI starts, so it cannot
   read live app state — that is fine; the report is static crash facts. Sending is
   deferred to the next GUI launch where prefs are available.

3. **Consent + reporting (gui).** Default `crash_reporting_consent = None` → nothing
   is ever sent until the user answers the one-time dialog. The dialog (and the
   settings tab) spell out exactly what a report contains and link the privacy note.
   "Report a bug" assembles a GitHub `new issue` URL with a pre-filled, URL-encoded
   body the user can review and edit in-browser before submitting — no silent upload.
   "Open log folder" reveals the config dir. After a report is filed or dismissed the
   JSON is removed so it is not re-offered.

4. **Build gate.** `cargo build --release` must pass; no new heavy dependencies (uses
   `std::backtrace`, existing `serde`/`serde_json`, and egui's `open_url`). House rule
   followed.

## Acceptance criteria mapping

- *With consent, a crash produces an actionable report; disabled by default* → local
  JSON report is always written; with `Some(true)` consent the GUI offers a pre-filled
  GitHub issue (panic message + backtrace + OS/version). `None`/`Some(false)` ⇒ nothing
  leaves the machine.

## Fix round 1 — adversarial review remediations (2026-06-30)

Three major findings addressed in the working tree (`pre-deploy/6-30-improvements`):

1. **Report destroyed all-but-newest pending reports.** The consent-accept path and
   the launch banner's "Report" both opened an issue for `pending.last()` only, then
   looped `clear_report` over **every** pending report — silently deleting unreported
   crashes (e.g. a startup crash-loop). Fix: those paths now clear **only** the single
   report actually filed (`last()`); remaining reports stay on disk and are re-offered
   next launch. Delete-all is reserved exclusively for the explicit "Dismiss" action.
   This also fixed a related defect: the consent "Not now" choice previously deleted
   every report — it now leaves them on disk (consistent with the `Some(false)` arm and
   the settings page). All three file paths (consent dialog, banner, settings
   "Report latest…") now share identical clear semantics.

2. **Prefill URL could exceed GitHub's length limit.** `issue_url_for_report` bounded
   the *raw* body at 6000 bytes, but percent-encoding expands release backtraces ~3x
   (spaces/slashes/colons/newlines → `%XX`), so the encoded URL routinely hit
   ~10–16 KB and tripped GitHub's HTTP 414. Fix: the bound is now on the **final
   encoded URL** (`MAX_URL = 7000`), trimming the raw body on char boundaries until the
   whole encoded URL fits, then re-closing the backtrace code fence with a trim note.
   Added `crash_report_url_tests` regression tests (huge all-`%XX` backtrace stays
   under budget; small report is left untrimmed).

3. **"Open log folder" button misdirected on Linux/macOS.** `photonic.log` is written
   to the binary dir when `APPDATA` is unset (Linux/macOS), but the button opens
   `crash_dir()` (`~/.config/Photonic/...`), which holds the crash reports but **not**
   the log. Took the minimal-correct relabel option: button is now
   "Open crash-report folder" with hover text "Reveal the folder holding crash reports
   (in your Photonic config folder)", and the CHANGELOG no longer claims reports sit
   "next to the log".

### Deferred

- **Co-locating `photonic.log` with the crash reports** (routing logging through
  `crash_dir()` so the log and reports really do share one directory) is deferred — it
  changes logging behavior in `photonic-app/main.rs` and is out of scope for #59. The
  round-1 fix corrects the user-facing wording/target instead; unifying the two base
  directories remains the cleaner long-term follow-up.
