# Changelog

All notable changes to Photonic are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

This file is the single source of truth for release notes: `scripts/release.sh`
rolls the `[Unreleased]` section into a versioned heading at release time, the
release CI uses that section as the GitHub release body, and the running app
embeds this file to show a "What's New" popup after an update.

## [Unreleased]

## [0.2.0] - 2026-07-04

### Added

- **Gradient (and pattern) strokes** (#201). Strokes are no longer limited to a
  single solid colour — a `Stroke` can now carry any paint (linear/radial
  gradient, pattern), rendered on-canvas, in headless/raster export, and in SVG
  (`stroke="url(#…)"`). Line icons with a gradient outline are now a first-class
  paint instead of an outline-stroke→delete→refill workaround. Set one via the
  new `set_paint` tool (`target: "stroke"`) or a `stroke.paint` object.
- **`set_paint` MCP tool** (#202): apply one paint to many nodes in a single
  undoable call, each re-fit to its own bounding box. Gradients accept
  `units: "bbox"` with 0–1 coordinates, so one relative gradient (e.g. left→right
  blue→purple) styles a whole icon set with zero per-node coordinate maths.
- **`export_icon_set` MCP tool + "Export Icon Set…" command** (#203): batch-export
  tagged groups (or every top-level group) to normalised, uniform-square `.svg`
  files in one step — no external post-pass to make an icon set render at a
  consistent scale.
- **`preview_selection` MCP tool** (#204): render the selection at target display
  sizes over light AND dark backgrounds as one contact-sheet PNG, to judge
  small-size legibility and on-surface contrast without leaving Photonic.
- **`import_design_tokens` MCP tool + "Import tokens…" swatch action** (#207):
  register named brand swatches from a CSS / JSON / Style-Dictionary tokens file
  (the counterpart to `export_design_tokens`). Paints can then reference brand
  colour by name, and re-importing re-themes everything at once.
- **Icon keyline grid + snap-to-pixel** (#208): a View toggle overlays the classic
  Material/Apple icon keyline template (square, circle, portrait & landscape safe
  areas) on the artboard, and "Snap to Pixel" lands drawing/moving on crisp
  integer coordinates.

### Changed

- **Under the hood: a big internal cleanup, with no change to how Photonic
  works.** The app's largest source files were reorganised into smaller, focused
  modules across the editor, the panels, the renderer, and the built-in tool
  server. This is purely a maintainability refactor — every feature, tool, and
  keyboard shortcut behaves exactly as before, and your documents are
  unaffected. It just makes Photonic faster to improve from here. If you do spot
  anything behaving differently after updating, please report it.
- **Selection SVG export is compact and consistent** (#205, #206, #203).
  `export_selection_as_svg` now rounds coordinates and path data to a `precision`
  (default 4 — no more 15-decimal path bloat), deduplicates byte-identical
  gradient/pattern defs into a single shared `<defs>` entry (across fills *and*
  strokes), and takes `normalize: "square"` to frame each icon in a uniform
  centred square viewBox.

### Fixed

- **Eyedropper now samples raster images** (and transformed shapes). In-canvas
  colour sampling only ever hit vector paths, so clicking the eyedropper on an
  imported image sampled nothing; it also hit-tested shape geometry against the
  raw canvas point, so moved/scaled/rotated shapes sampled the wrong spot.
  Sampling now maps the click into each node's local space, reads the pixel of a
  raster layer (honouring its layer mask, falling through transparent pixels to
  whatever is beneath), and keeps gradient colours matched to the on-screen
  render. Applies to both the GUI eyedropper and the `sample_color_at` MCP tool.

### Added

- **Rotate objects from the canvas.** Hover just outside a corner handle with
  the Select tool — a rotate affordance appears — then drag to rotate the
  selection in place about its centre (Illustrator/Photoshop-style). Works for
  single or multiple objects; hold Shift to snap to 15° increments; one undo
  step. (Precise numeric rotation in the Transform inspector still works too.)
- Image import in the GUI (previously raster layers could only be placed via
  the MCP `place_image` tool):
  - **File → Place Image…** imports a PNG/JPEG/WebP/BMP/GIF/TIFF as a raster
    layer, centred on the artboard, selected, and undoable.
  - **File → Open…** and the welcome screen's Browse now accept image files
    too: from the editor the photo is placed into the current document; from
    the welcome screen it opens as a fresh artboard sized to the photo.
  - Image files can be **dragged & dropped** onto the window on X11, Windows,
    and macOS. (Not on Wayland yet — winit's Wayland backend has no
    drag-and-drop support.)
- **Crop to Artboard** on raster layers (Inspector → Raster Layer): trims the
  image — and its layer mask, in lockstep — to the bounds of the artboard the
  image is on (the one it overlaps most, in the spatial multi-artboard model),
  discarding pixels outside while keeping the surviving pixels exactly where
  they were on canvas. Destructive but undoable; rotated images are rejected
  rather than silently resampled.
- **Group selection with the Select tool**: clicking any member of a group now
  selects every object in its outermost group (Illustrator behavior), so the
  whole group moves/edits as a unit; Shift+click toggles the whole group in and
  out of the selection. Alt+click still grabs just the clicked member, and
  double-click still enters isolation mode for editing inside the group.

### Fixed

- **Shape Builder now merges across layers, with feedback.** Like the boolean
  combine below, it silently did nothing unless every touched shape lived in the
  same layer. It now folds all dragged-over path shapes regardless of layer,
  reports the result (or why nothing merged) in the status bar, and hints when a
  drag only caught one shape.
- **Union / boolean combine now actually merges shapes.** The Union, Subtract,
  Intersect, and Exclude operations silently did nothing unless *exactly two*
  path objects in the *same layer* were selected, and gave no feedback when
  they didn't run. They now combine **any number** of selected path shapes,
  **across layers**, folding bottom-to-top (Union merges all, Subtract removes
  the upper shapes from the bottom one, etc.), and report what happened in the
  status bar — including why nothing merged ("Select 2 or more path shapes",
  "produced an empty shape").
- **Strokes no longer scale with the object.** Scaling a shape (drag its
  bounding-box handles, or any transform with a scale) kept its stroke a fixed
  width instead of thickening/thinning it — a stroke is an absolute property,
  not geometry (Illustrator's "Scale Strokes & Effects" off). Applied uniformly
  across the live canvas, raster/PNG export, and SVG/PDF export, so all four
  agree. Rotation and translation are unaffected; view zoom still scales
  everything as before.
- Opening a photo from the welcome screen now also sizes the **artboard** to
  the photo (previously only the document dimensions changed, leaving the
  visible artboard at its default size — which also made Crop to Artboard trim
  against the wrong rectangle).
- Raster Masking on imported images (Inspector → Raster Masking, shown when a
  raster layer is selected): pick a color on the canvas and hide every pixel
  within an adjustable fuzziness of it — globally (Color Range) or only the
  connected region under the click (Contiguous / magic-wand) — with a live
  preview and Apply/Cancel. Non-destructive: the pixels are hidden via the
  layer mask, never erased, and the edit is a single undo step.
- One-click **Remove Background** on raster layers: a small local matting
  model (U²-Net-p, Apache-2.0) detects the subject fully on-device via ONNX
  Runtime and applies it as a non-destructive foreground layer mask. The
  ~5 MB model downloads once to the Photonic cache, then works offline. Also
  exposed as the `remove_background` MCP tool, and a Clear Layer Mask button
  reveals the layer again at any time.
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
