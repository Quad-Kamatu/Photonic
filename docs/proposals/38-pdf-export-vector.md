# PDF export (vector) (#38) — Design Proposal

> Status: design scaffold (not an implementation).

## Summary

`ExportFormat` in `photonic-gui/src/app.rs` (line 140) covers Png, Jpeg, WebP, Gif, Tiff, Ico,
and Svg. `export.rs` provides `export_svg()` and `export_nodes_as_svg()`. `ExportProfile` in
`document.rs` has `new_svg()` and `new_png()` constructors. There is no PDF in any of these.
PDF is a baseline professional deliverable and a prerequisite for print output (#37). This issue
adds a vector PDF writer to the export pipeline.

## Scope

**In scope**
- Vector PDF writer via the `pdf-writer` crate (pure-Rust, no C dep; generates standards-
  conformant PDF 1.7).
- Exported content: filled/stroked paths, linear and radial gradients, clipping paths, groups
  with opacity, solid-color text with embedded/subsetted fonts.
- Bleed-aware page size (adds `bleed_mm` margin if requested — coordinate with #37).
- Multi-artboard / multi-page (one PDF page per artboard — coordinate with M3 artboards issue).
- GUI export dialog: add "PDF" option to `ExportFormat`; show font embedding toggle.
- MCP: `export_pdf(options)`.

**Out of scope**
- Blend modes beyond Normal/Multiply/Screen (PDF transparency group semantics are complex;
  unsupported modes fall back to Normal in MVP).
- Interactive PDF features (form fields, links, JavaScript).
- PDF/X compliance (subset for print pre-press; deferred to a follow-on).
- Mesh gradients and fluid gradients (no direct PDF equivalent; rasterize these sub-objects
  only as a fallback).

## Proposed approach

1. **Dependency**: add `pdf-writer = "0.9"` (or current stable) to `Cargo.toml` (workspace) and
   to `crates/photonic-core/Cargo.toml`. `pdf-writer` is pure Rust, no build scripts.

2. **`export_pdf` function** (`crates/photonic-core/src/export.rs`):
   ```rust
   pub struct PdfExportOptions {
       pub include_bleed: bool,
       pub embed_fonts: bool,       // default true; false = reference only (not recommended)
       pub compress_streams: bool,  // default true (flate)
   }

   pub fn export_pdf(doc: &Document, opts: &PdfExportOptions) -> Vec<u8>
   ```
   Implementation outline:
   - Open a `pdf_writer::Pdf` writer.
   - For each layer (or artboard page), emit a PDF page with `MediaBox` = artboard size +
     bleed (if `include_bleed`). Set `BleedBox` to artboard + bleed, `TrimBox` to artboard.
   - Walk the scene graph (same traversal as `export_svg` in `export.rs`): for each `SceneNode`
     emit PDF content stream operators.
   - **Paths**: translate `photonic-core::path::Path` segments to PDF `m`/`l`/`c`/`h` operators.
     Apply fill (with winding rule) and stroke as `f`/`S`/`B`.
   - **Solid fills**: `rg` / `RG` operators with sRGB values (or `k`/`K` in CMYK mode — requires #36).
   - **Linear/radial gradients**: PDF `Shading` dictionary (Type 2 / Type 3 function-based). Map
     `Gradient` stops (`style.rs`) to a sampled or parametric PDF shading.
   - **Groups + opacity**: PDF transparency groups (`/Group << /Type /Group /S /Transparency >>`);
     `ExtGState` with `CA`/`ca` for fill+stroke alpha.
   - **Clipping paths**: `W n` operator sequence for clipping masks.
   - **Text**: emit PDF `BT`/`ET` blocks with `Tf` (font + size) and `Tj` / `TJ` for text.
     Font subsetting: use `subsetter` crate (or `pdf-writer`'s built-in subset helper) to
     extract only referenced glyphs from the TrueType/OpenType font file. Embed as a
     `CIDFont` / `ToUnicode` stream for Unicode text extraction.
   - **Mesh / fluid gradients**: detect at export time; rasterize to a PNG image stream and
     embed as an `XObject` image. Log a warning.

3. **`ExportFormat`** (`crates/photonic-gui/src/app.rs`): add `Pdf` variant. Add `pdf_embed_fonts:
   bool` and `pdf_include_bleed: bool` to `ExportDialog`. Render PDF-specific options when
   `ExportFormat::Pdf` is selected (below the existing format selector at line 12200).

4. **`ExportProfile`** (`crates/photonic-core/src/document.rs`): add `new_pdf()` constructor
   alongside `new_svg()` and `new_png()`.

5. **Headless export** (`crates/photonic-render/src/headless.rs`): the headless export path
   currently calls into `export_svg` or the raster pipeline. Add a branch for PDF: call
   `export_pdf(doc, opts)` and write the bytes to disk. PDF does not require the wgpu renderer
   (it is a pure-model export like SVG), so no GPU involvement.

6. **MCP** (`crates/photonic-mcp/src/handlers/document.rs`): `export_pdf(path, embed_fonts?,
   include_bleed?)` — analogous to existing SVG export tool.

## Affected modules

- `crates/photonic-core/src/export.rs` — `PdfExportOptions`, `export_pdf`, path/gradient/text
  translation, font subsetting
- `crates/photonic-core/src/document.rs` — `ExportProfile::new_pdf()`
- `crates/photonic-gui/src/app.rs` — `ExportFormat::Pdf`, `ExportDialog` PDF fields
- `crates/photonic-render/src/headless.rs` — PDF branch in headless export
- `crates/photonic-mcp/src/handlers/document.rs` — `export_pdf` tool
- `Cargo.toml` (workspace) + `crates/photonic-core/Cargo.toml` — `pdf-writer`, `subsetter`

## Risks & open questions

- **Font subsetting complexity**: subsetting OTF/TTF requires reading the font file on disk
  (or from `fontdb`) and extracting referenced glyph tables. The `subsetter` crate handles
  TrueType; CFF-based OpenType fonts require `cff-subset` or manual table surgery. Audit before
  committing to subsetting scope.
- **Gradient fidelity**: PDF Type 2 (axial) and Type 3 (radial) shadings map cleanly to
  `Gradient::Linear` and `Gradient::Radial` (`style.rs:219/227`), but multi-stop gradients
  require a stitching function (Type 3 PDF function). Implement and test carefully.
- **Blend modes**: PDF ExtGState blend modes differ slightly from CSS/SVG mix-blend-mode naming.
  Map `photonic-core` blend mode strings to PDF blend mode names; unsupported modes (e.g. Hue,
  Saturation) fall back to Normal with a logged warning.
- **Text position accuracy**: glyphon shapes text on-screen but `export_pdf` runs without the
  GPU renderer. Text positions must be computed from `TextNode` metrics independently (font
  metrics from `fontdb`/`swash`). Test that positions match the on-screen rendering.
- **CMYK color output**: in CMYK document mode (#36), fills should use PDF `k`/`K` operators
  instead of `rg`/`RG`. This is a soft dependency; MVP can export sRGB values even for CMYK
  docs with a warning.
- **File size**: uncompressed PDF streams bloat output; ensure `compress_streams: true` is the
  default and that `pdf-writer`'s `Stream::filter(FlateDecodeFilter)` is applied.

## Acceptance criteria

- [ ] A document with filled paths, linear gradients, and radial gradients exports a valid PDF.
- [ ] Exported PDF renders identically (visually) to the SVG export in a reference viewer.
- [ ] Text is selectable in the exported PDF (ToUnicode map present).
- [ ] Fonts are embedded/subset; no external font dependencies in the output file.
- [ ] Groups with opacity render with correct transparency in the PDF.
- [ ] `ExportFormat::Pdf` appears in the export dialog and produces a `.pdf` file.
- [ ] MCP `export_pdf` writes a valid PDF to the given path.
- [ ] Mesh/fluid gradient fallback: rasterized as an image with a log warning, not a crash.

## Effort estimate

**XL** — vector PDF writing is inherently complex: path translation is M, gradient shading dicts
are M, font subsetting is L on its own, and text-position correctness without the renderer
requires re-implementing layout math. Plan for at least 3–4 focused sprints.
