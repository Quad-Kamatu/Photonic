# Print output: print dialog, crop/registration marks, and separations (#37) — Design Proposal

> Status: design scaffold (not an implementation).

## Summary

`Document` already stores `bleed_mm: f64` and `slug_mm: f64` (`photonic-core/src/document.rs`
lines 505–511) and `spot_colors: Vec<SpotColor>` (line 490). There is no print pipeline:
no print dialog, no marks rendering, and no separations output. This issue builds the print
output layer on top of the existing bleed/slug/spot-color model, gated on #36 (CMYK) and #38
(PDF export) being available.

## Scope

**In scope**
- Print export dialog: media size, orientation, bleed on/off, marks (crop, registration, color
  bars), scale to fit.
- Marks rendering: crop marks at artboard corners, registration marks (crosshair + circle),
  color bars (CMYK + spot ink swatches).
- Separations output: one plate per ink (C, M, Y, K + each spot color), exported as individual
  single-channel images or as a multi-page PDF with each page being one plate.
- Separations preview in the GUI ("View > Separations Preview" overlay per channel).
- MCP tool `export_print(options)` returning file path(s).

**Out of scope**
- Direct printing to system printers (PostScript/IPP); output is always to file.
- Trapping / ink limit / total ink coverage calculations (deferred).
- Imposition / n-up layouts.

## Proposed approach

1. **Print export options** (new struct in `crates/photonic-core/src/export.rs`):
   ```rust
   pub struct PrintExportOptions {
       pub media_width_mm: f64,
       pub media_height_mm: f64,
       pub include_bleed: bool,       // uses Document.bleed_mm
       pub include_slug: bool,        // uses Document.slug_mm
       pub crop_marks: bool,
       pub registration_marks: bool,
       pub color_bars: bool,
       pub separations: bool,         // one plate per ink vs. composite
       pub output_path: PathBuf,
   }
   ```

2. **Marks as scene-graph overlays**: rather than baking marks into the renderer, generate marks
   as temporary `SceneNode` (path) objects assembled in a `print_marks_layer(doc, opts) ->
   Vec<SceneNode>` function. These nodes are injected into export calls but never persisted to
   the document. This lets the existing SVG and (future) PDF exporters render them without
   special-casing. Mark geometry:
   - **Crop marks**: 4 corner L-shapes at bleed + slug offset from artboard corners, 0.25 pt
     stroke, black.
   - **Registration marks**: crosshair + circle centered at top/bottom/left/right midpoints of
     the slug area.
   - **Color bars**: a row of 10×5 mm filled rectangles for 100% C, M, Y, K, and each spot ink.

3. **Separations** (`crates/photonic-core/src/export.rs`):
   - For CMYK separations: for each channel C/M/Y/K, render the document with all fills/strokes
     converted to their ink percentage on that channel only, output in grayscale. Requires #36
     (CMYK color mode) for correct separation values.
   - For spot inks: render only nodes whose fill/stroke matches the spot color name; all other
     artwork is suppressed.
   - Output: series of PNG files (one per plate) or a multi-page PDF (one page per plate) via
     the PDF exporter (#38).
   - Entry point: `fn export_separations(doc: &Document, opts: &PrintExportOptions) ->
     Result<Vec<PathBuf>>`.

4. **Separations preview** (`crates/photonic-render/src/renderer.rs`): add a
   `SeparationPreviewChannel` enum (C | M | Y | K | Spot(String) | Composite) to `ExportOptions`
   (or a new `PreviewOptions`). When set, the renderer applies a per-pixel channel extraction
   to the final composite before display. This is a view-only mode, no document change.

5. **Print dialog** (`crates/photonic-gui/src/app.rs`): new `PrintDialog` struct alongside the
   existing `ExportDialog`. Accessible from File > Print / Export for Print. Fields mirror
   `PrintExportOptions`; media size has a preset dropdown (A4, Letter, US Legal, custom).
   "Export Separations" checkbox toggles separations mode. Preview thumbnail shows bleed +
   marks placement.

6. **MCP** (`crates/photonic-mcp/src/handlers/document.rs`): `export_print(options_json)`.

## Affected modules

- `crates/photonic-core/src/export.rs` — `PrintExportOptions`, `print_marks_layer`,
  `export_separations`, `export_print`
- `crates/photonic-core/src/document.rs` — no new fields; uses existing `bleed_mm`, `slug_mm`,
  `spot_colors`
- `crates/photonic-render/src/renderer.rs` — `SeparationPreviewChannel` in preview path
- `crates/photonic-gui/src/app.rs` — `PrintDialog`; View > Separations Preview
- `crates/photonic-mcp/src/handlers/document.rs` — `export_print`

## Risks & open questions

- **Hard dependency on #36 and #38**: separations require CMYK color values (#36), and
  multi-plate PDF requires PDF export (#38). The composite-with-marks output (crop marks on a
  single PDF page) can ship independently, but separations cannot. Sequence accordingly.
- **Spot color separation accuracy**: without full CMYK mode (#36), spot color separation is
  approximate (only nodes directly assigned the spot ink are included; overprint interactions
  are not simulated). Document this limitation clearly.
- **Mark geometry in SVG export**: SVG has no concept of "print bleed area"; marks will render
  outside the SVG viewBox. Ensure `export_svg` with print options sets the viewBox to include
  the slug area.
- **Screen vs. print resolution for raster separations**: 300 dpi minimum for print; the
  renderer's wgpu surface is screen-resolution by default. The headless render path
  (`photonic-render/src/headless.rs`) must accept a target DPI for print-resolution output.
- **Registration mark centering**: marks must be at the geometric center of each side of the
  bleed+slug area, not the artboard — verify the coordinate math carefully.

## Acceptance criteria

- [ ] Print export dialog opens from File menu; accepts media size, marks toggles.
- [ ] Exported output includes crop marks at artboard corners at the correct bleed offset.
- [ ] Registration marks appear at the midpoint of each side within the slug area.
- [ ] Color bar row shows 100% patches for C, M, Y, K, plus any document spot colors.
- [ ] Separations export produces one grayscale image per ink channel.
- [ ] Separations preview in the GUI correctly isolates each ink channel visually.
- [ ] Bleed area (the extra `bleed_mm` on each side) is included in the output bounds.
- [ ] MCP `export_print` returns paths to all generated files.

## Effort estimate

**L** — marks geometry and separations logic are medium-complexity; the main cost is the
dependency chain (#36 CMYK + #38 PDF) and ensuring the headless renderer hits print DPI.
Composite-with-marks output (no separations) alone is **M**.
