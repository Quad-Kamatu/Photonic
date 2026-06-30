# Changelog

All notable changes to Photonic are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

This file is the single source of truth for release notes: `scripts/release.sh`
rolls the `[Unreleased]` section into a versioned heading at release time, the
release CI uses that section as the GitHub release body, and the running app
embeds this file to show a "What's New" popup after an update.

## [Unreleased]

### Added

- Opt-in crash reporting and diagnostics (#59): when Photonic panics it now
  writes a structured, non-sensitive crash report (app version, UTC time,
  OS/arch, panic message, backtrace) to a `crash-reports/` folder in your
  Photonic config directory — local
  capture is always on. Sending is opt-in: on the next launch a one-time consent
  dialog (or, once enabled, a Report/Dismiss banner) offers to file the crash as
  a pre-filled GitHub issue you review in your browser before submitting. No
  document content, file paths, or environment variables are ever collected, and
  nothing is sent automatically. A new "Privacy & Diagnostics" settings tab adds
  the consent toggle, an Open crash-report folder button, and a Report a bug
  button.
- Auto-check-on-launch update prompt: once per launch Photonic asks GitHub for
  the latest release (no download) and shows a dismissable banner if a newer
  version exists.
- "What's New" popup that appears after updating, summarising changes in the
  versions you skipped (sourced from this changelog).

## [0.1.0] - 2026-06-29

### Added

- First public release of Photonic — a cross-platform vector + raster graphics
  editor built in Rust (egui / wgpu).
- Guided cinematic welcome flow with a live Lightfall shader background, a
  searchable size catalog (~130 presets), advanced New-Canvas options
  (DPI/PPI, bleed, slug, margins, artboard count), and recent-document
  thumbnails.
- Spatial multi-artboard documents: model, rendering, in-editor rename / drag /
  resize with artwork that moves with its board, alignment + equal-distance
  distribution snapping, and per-board export.
- Global command palette with direct + on-device semantic search (bundled
  embedding model, fully local).
- Disk search for `.photon` files across user-picked roots and the OS index.
- Photoshop-grade raster editing subsystem (engine, Raster node, brush/eraser
  tools, MCP tools, export).
- Signed auto-update pipeline: GitHub Releases as host, ed25519-signed archives
  verified before install, single-source semantic versioning.
