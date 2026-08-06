# PDF Import (Multi-Page, Vector + Raster) (#40) — Design Proposal

> Status: design scaffold (not an implementation).

## Summary

No PDF import exists in the codebase. The current import surface is limited to `import_svg` in `crates/photonic-core/src/import.rs`. PDF is the most common cross-tool interchange format; adding it unlocks editing legacy assets and receiving client-supplied artwork. The feature depends on M3 milestones: `ImageNode` (for embedded rasters) and multi-artboard support (each PDF page maps to a Photonic artboard / `Page`). The `Page` struct already exists in `document.rs`, so the artboard host is ready.

## Scope (in / out)

**In:**
- Parse a PDF byte stream into a `Document`: one `Page` per PDF page, with per-page `width`/`height` taken from MediaBox.
- Vector content: PDF path operators (`m l c v y h S s f F b`) → `PathData`; fill and stroke properties → `Fill` / `Stroke`.
- Text: PDF text operators → `TextNode` where possible; fall back to outlined `PathNode` when the embedded font is unavailable.
- Embedded raster images: decode to `ImageNode` (M3 dependency; if unavailable, emit a placeholder `PathNode` bounding rect).
- Clipping paths: PDF clip operators (`W`, `W*`) → `GroupNode` with a clip mask (needs clip-mask support on `GroupNode`).
- Layer structure (PDF Optional Content Groups) → Photonic `Layer` objects.
- Multi-page: each page becomes its own `Page` in `doc.pages`; the first page is the active artboard.
- MCP tool: `import_pdf(bytes_base64, options)` returning the new `Document` JSON.

**Out:**
- PDF forms, JS, annotations, encryption, XFA — out of scope.
- Perfect font rendering when the embedded font is not installed — fall back to outlined paths.
- PDF export (separate issue).
- AI-native format (PostScript extension layer) — covered by #41.

## Proposed Approach

1. **Dependency**: Add `lopdf` (pure-Rust PDF parser; MIT, well-maintained) to `photonic-core/Cargo.toml`. It handles cross-reference tables, page tree traversal, and content stream decoding without native deps.

2. **New module** `crates/photonic-core/src/import_pdf.rs`:
   - `pub fn import_pdf(bytes: &[u8]) -> Result<Document, ImportError>` — top-level entry point.
   - `struct PdfContext` — holds `lopdf::Document`, current page transform stack, a color space map, and an id counter.
   - `fn process_page(ctx: &mut PdfContext, page_dict: &lopdf::Dictionary) -> Page` — creates a new `Page`, runs the content stream interpreter, returns the page with its `NodeId` list.
   - `fn interpret_content_stream(ctx: &mut PdfContext, ops: &[lopdf::content::Operation]) -> Vec<SceneNode>` — state machine over PDF operators. Key operator groups:
     - Path construction (`m l c v y h re`) → accumulate into `PathData::from_bez_path`.
     - Painting (`S s f F b B`) → emit `PathNode` with resolved `Fill`/`Stroke`.
     - Text (`BT ET Tf Td TD Tm T* Tj TJ '`)  → emit `TextNode` with font/size.
     - Images (`Do`) → resolve XObject, decode, emit `ImageNode` (or placeholder).
     - Graphics state (`q Q cm w J j M d gs`) → push/pop `Transform` and style state.

3. **Color space mapping**: Map PDF `DeviceRGB`, `DeviceCMYK`, `DeviceGray`, `ICCBased` to `Color` in `color.rs`. CMYK conversion via the standard formula.

4. **Page to artboard**: After all pages are processed, set `doc.pages` and `doc.active_page`. Page dimensions from MediaBox translated from PDF units (1/72 inch) to document units (px at 96 dpi: multiply by 96/72).

5. **`ImportError` extension**: Add `ImportError::PdfParse(String)` variant to the existing enum in `import.rs`.

6. **Tests**: Unit-test with a minimal hand-constructed PDF byte string covering: single path, text, multi-page.

## Affected Modules

- `crates/photonic-core/src/import_pdf.rs` — new file (all PDF import logic)
- `crates/photonic-core/src/import.rs` — expose `import_pdf` alongside `import_svg`; extend `ImportError`
- `crates/photonic-core/src/lib.rs` — re-export `import_pdf`
- `crates/photonic-core/Cargo.toml` — add `lopdf = "0.32"`
- `crates/photonic-mcp/src/server.rs` — add `import_pdf` tool handler
- `crates/photonic-mcp/src/protocol.rs` — `ImportPdfArgs` struct

## Risks & Open Questions

- **`lopdf` vs. `pdf-extract` vs. `pdfium-render`**: `lopdf` is pure Rust and gives raw operator access (needed for accurate path/text extraction). `pdf-extract` is higher-level but text-only. `pdfium-render` wraps PDFium (C++) and handles more edge cases but adds a native dep. Start with `lopdf`; escalate to pdfium only if coverage is insufficient.
- **Font handling**: PDF embeds fonts as streams; matching them to system fonts or outlining them requires a font subsetting library. Proposed fallback: extract the glyph outlines from embedded Type1/CFF/TrueType subsets using the `ttf-parser` crate (already available via `glyphon` in the render crate).
- **ImageNode missing**: If M3 ships after this issue, embedded raster XObjects must be deferred. Emit a gray-fill placeholder `PathNode` with a `TODO` tag so the user can see the bounding box.
- **Coordinate system**: PDF origin is bottom-left; Photonic is top-left. Apply a Y-flip transform per page.
- **Clip masks on `GroupNode`**: Not yet in the model. For clipped paths, fall back to using `BooleanOp::Intersect` at import time (destructive but correct).

## Acceptance Criteria

- [ ] A simple single-page vector PDF (paths, fills, strokes, basic text) imports as a `Document` with one `Page` and editable `PathNode`/`TextNode` objects.
- [ ] A multi-page PDF produces one `Page` per PDF page; all pages are present in `doc.pages`.
- [ ] Embedded raster images produce `ImageNode` objects (or labeled placeholders) at correct positions.
- [ ] Import does not panic on malformed or encrypted PDFs (returns `ImportError`).
- [ ] MCP `import_pdf` tool is exercisable via the server.

## Effort Estimate

**L** — PDF content stream interpretation is complex state-machine work. Font handling and raster image decoding add further scope. Multi-page + artboard mapping is straightforward given the existing `Page` struct.
