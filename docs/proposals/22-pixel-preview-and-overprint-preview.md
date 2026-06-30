# Pixel Preview and Overprint Preview View Modes (#22)

> Status: **implemented** (pre-deploy/6-29-26-improvements). The standard
> Multiply overprint approximation and a nearest-sampled export-resolution Pixel
> Preview both ship; ICC-correct CMYK overprint and the other items under
> "Remaining work" are deferred.

## What this PR implements

- **`ExportOptions.overprint_preview`** (`crates/photonic-render/src/headless.rs`)
  — when set, the headless `build_geometry` collects the canonicalised `#RRGGBB`
  hexes of every `overprint`-flagged `SpotColor` in `document.spot_colors` and
  forces any node whose `FillKind::Solid` fill matches one to composite with
  `BlendMode::Multiply` instead of its own (knockout) blend mode. Multiply is an
  already-supported separable blend, so no new pipeline was needed.
- **`HeadlessRenderer::render_rgba_with_opts(doc, w, h, &opts) -> (Vec<u8>, u32,
  u32)`** — `render_png_with_opts` was refactored to build the raw RGBA buffer
  via this method and only PNG-encode at the end, so the GUI preview overlay gets
  the export bytes without a PNG encode/decode round trip.
- **Two view toggles on `PhotonicApp`** — `pixel_preview` and
  `overprint_preview` (default false, not persisted), mutually exclusive with
  each other and with `outline_mode`. Toggling is centralised in
  `toggle_outline_mode` / `toggle_pixel_preview` / `toggle_overprint_preview`,
  which clear the other two.
- **Preview overlay** (`PhotonicApp::paint_preview_overlay`) — renders the active
  artboard (or the full document when there are no artboards) at its native
  export pixel size through a lazily-created GUI-held `HeadlessRenderer`, uploads
  the bytes as a `TextureOptions::NEAREST` egui texture, and paints it over the
  artboard's screen rect. The render is content-hashed (FNV-1a over the
  serialised document + active mode + target pixel size) into a `PreviewTexCache`
  so it only re-runs on change. The live raster-node overlay is skipped while a
  preview is active to avoid double compositing.
- **End-to-end wiring** mirroring `view.outline_mode`: command defs
  `view.pixel_preview` (Ctrl+Alt+Y) and `view.overprint_preview` (Ctrl+Shift+Y)
  in `commands.rs` (plus a new `KeyBinding::ctrl_alt` constructor),
  command-center handlers, `SearchAction::PixelPreview` /
  `OverprintPreview` global-search entries, View-menu checkboxes, and keybinding
  dispatch in `tool_handlers.rs`.
- **Test**: `headless::blend_tests::overprint_preview_multiplies_matching_spot_ink`
  renders a top solid fill (Normal blend) whose hex matches an overprint spot
  ink over a backdrop and asserts the overlap multiplies (passes on the GPU here).

## Remaining work (deferred)

- Accurate CMYK / ICC-correct overprint compositing — the issue itself notes
  this **depends on M5 color-management work**. This pass ships the standard
  Multiply approximation only.
- Overprint for gradient / pattern / mesh fills and for strokes (solid fills
  only this pass).
- Overprint in the **CPU compositor** raster-document path
  (`compositor.rs::composite_document`). The preview's headless path still routes
  raster-containing documents through `composite_document`, which does not apply
  the overprint override, so Overprint Preview is a no-op for documents that
  contain raster layers (Pixel Preview still works for them).
- MCP tool exposure of the toggles (no MCP surface was touched, so
  `docs/mcp-api.md` is unchanged).
- A configurable preview PPI spinner — v1 uses the artboard's native pixel size
  (document units already encode px); a PPI selector can follow.

## Summary

Add two non-destructive **View menu** toggles that change how the canvas is
*displayed* without mutating the document:

- **Pixel Preview** — renders the active artboard at its output pixel size
  (document-units = px) through the existing headless/export render path and
  displays the result as a **nearest-neighbour** textured quad over the canvas.
  Because it shows the exact rasterised bytes the exporter would write, the user
  sees true aliasing and pixel-grid snapping, especially when zoomed past 100%.
- **Overprint Preview** — re-renders the same way but forces nodes whose solid
  fill matches an `overprint`-flagged `SpotColor` to composite with **Multiply**
  against the backdrop (the standard overprint simulation) instead of knocking
  out (Normal). This makes the `overprint` flag on `SpotColor`
  (`crates/photonic-core/src/document.rs:445-454`) visible for the first time.

Both modes reuse the proven offscreen render path (`HeadlessRenderer` /
`render_png_with_opts`) and the existing overlay-painting pattern already used
by Outline Mode and raster nodes in `app/mod.rs:2470-2524`, rather than touching
the live windowed GPU pipeline. They are mutually exclusive with each other and
with Outline Mode.

## Scope

### In

- `pixel_preview: bool` and `overprint_preview: bool` view state on
  `PhotonicApp` (default false; not persisted), mirroring `outline_mode`.
- A `View` menu / command-palette / global-search entry and keybinding for each,
  mirroring `view.outline_mode` wiring.
- A cached, content-hashed preview render (FNV-1a over the doc + mode + ppi,
  reusing the `raster_texture` cache pattern) displayed as a nearest-neighbour
  egui texture covering the current artboard rect. Re-renders only when the hash
  changes.
- Overprint simulation in the **GPU headless build_geometry path**
  (`headless.rs`): a new `ExportOptions.overprint_preview` flag that, when set,
  overrides the per-node blend segment to `BlendMode::Multiply` for nodes whose
  `FillKind::Solid` color (`style.rs:63-77`) hex-matches an overprint-flagged
  spot color in `document.spot_colors`.
- A `render_rgba_with_opts(doc, w, h, &opts) -> (Vec<u8>, u32, u32)` helper on
  `HeadlessRenderer` so the GUI gets raw RGBA without a PNG encode/decode round
  trip (currently `render_png_with_opts` only returns encoded PNG bytes).

### Out (deferred)

- Accurate CMYK / ICC-correct overprint compositing — the issue itself notes
  this **depends on M5 color-management work**. This pass ships the standard
  Multiply approximation only.
- Overprint for gradient / pattern / mesh fills and for strokes (solid fills
  only this pass).
- Overprint in the **CPU compositor** raster-document path
  (`compositor.rs::composite_document`) — pixel/overprint preview will fall back
  to the normal composite for documents containing raster layers this pass.
- MCP tool exposure of the toggles.
- A configurable preview PPI spinner — v1 uses the artboard's native pixel size
  (document units already encode px); a PPI selector can follow.

## Approach

### Rendering plumbing (`photonic-render`)

1. Add `overprint_preview: bool` (default false) to `ExportOptions`
   (`headless.rs:46-72`).
2. In the free `build_geometry` used by the headless path, when
   `overprint_preview` is set, compute the set of overprint ink hex values from
   `document.spot_colors` (filter `overprint == true`), and for each node whose
   `FillKind::Solid` resolves to a matching hex, push its index range with
   `BlendMode::Multiply` instead of `node.blend_mode` (the segment list is built
   around `raw_segments.push((node.blend_mode, …))` at `renderer.rs:1457` for
   the windowed path; the headless `build_geometry` has the analogous site).
   Multiply is already a supported separable blend (`SEPARABLE_BLEND_MODES`,
   `pipeline.rs:23-28`), so no new pipeline is needed.
3. Add `render_rgba_with_opts` returning raw premultiplied-or-straight RGBA
   bytes (the same buffer `render_png_with_opts` builds before PNG encoding at
   `headless.rs:258-259`), to avoid a PNG round trip in the GUI.

### GUI integration (`photonic-gui`)

4. Add `pixel_preview` / `overprint_preview` fields to `PhotonicApp`
   (`app/mod.rs:492` neighbourhood) and a small `PreviewTexCache`
   `{ hash, handle }` like `RasterTexCache`.
5. In the central-panel render block (`app/mod.rs:2470`), when either mode is
   active: build `ExportOptions` (`region` = current artboard bounds,
   `overprint_preview` = the flag), render via the GUI-held `HeadlessRenderer`
   (or a lazily-created one) at the artboard pixel size, upload as an egui
   texture with `TextureOptions::NEAREST`, and paint it over the artboard rect
   (replacing the live view for that region, the way Outline Mode covers it).
   Skip the live raster-node overlay while a preview is active to avoid double
   compositing.
6. Make the three view modes mutually exclusive: turning one on clears the other
   two.
7. Register commands `view.pixel_preview` / `view.overprint_preview` in
   `commands.rs:272` style, handle them in `command_center.rs:102`, add
   `SearchAction` variants + entries in `global_search.rs`, and a checkbox in the
   View menu (`app/mod.rs:1885` where the Outline Mode checkbox lives), plus
   keybinding dispatch in `tool_handlers.rs:117`.

### Acceptance mapping

- *Pixel Preview shows aliasing/snapping as it would export* → satisfied by
  nearest-neighbour display of the actual export-resolution render.
- *Overprint Preview changes the composite for overprinting spot inks* →
  satisfied by the Multiply override on overprint-ink solid fills.

## Build / verify

After each edit, `cargo build --release` must pass. Verify by creating a doc
with two overlapping shapes, defining a spot color matching the top shape's fill
with `overprint = true`, then toggling Overprint Preview (top shape should
multiply into the lower one) and Pixel Preview (zoom past 100% to see crisp
export pixels).
