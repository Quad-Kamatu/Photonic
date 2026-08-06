# Pixel Preview and Overprint Preview View Modes (#22) — Design Proposal

> Status: design scaffold (not an implementation).

## Summary

There is currently no way to preview rasterization at a target output PPI (Pixel Preview)
or to simulate overprint compositing for spot inks (Overprint Preview). The `SpotColor`
struct (`document.rs:358-379`) carries an `overprint: bool` flag that is stored but never
visualized. Both are non-destructive view toggles standard in professional vector editors.

## Scope

**In:**
- **Pixel Preview:** render at the document's output PPI, snap all geometry to the
  device pixel grid at that resolution, and display the rasterized result (showing
  aliasing and sub-pixel snapping as they will appear in export).
- **Overprint Preview:** for fills using spot colors flagged `overprint = true`, simulate
  overprint compositing (multiply ink layers) instead of the default knock-out behavior.
- Both are view-mode toggles, not permanent document changes.
- GUI: View menu items + toggle state in `crates/photonic-gui/src/app.rs`.

**Out:**
- Full CMYK/ICC color-profile simulation (depends on M5 color-management work).
- Soft-proofing for arbitrary output profiles (M5).
- Spot color to CMYK conversion accuracy (M5).

## Proposed Approach

### Pixel Preview

1. **View mode enum** — add to `crates/photonic-gui/src/app.rs` (or a shared state
   struct):
   ```rust
   pub enum ViewMode { Normal, PixelPreview { ppi: f64 }, OverprintPreview }
   ```
   Store as `view_mode: ViewMode` on the app state.

2. **Renderer flag** — expose `pixel_preview: Option<f64>` (the target PPI) on
   `PhotonicRenderer` (or pass it per-frame via `CanvasView`).

3. **Implementation:** when `pixel_preview` is set, in `build_geometry` / `update()`:
   - Compute the document-to-pixel scale factor: `scale = ppi / 72.0` (assuming 72 dpi
     document units).
   - Snap all document-coordinate vertices to the nearest pixel boundary at that scale
     before tessellation: `snap(v) = (v * scale).round() / scale`.
   - Upload the snapped geometry; the resulting render shows the rasterized grid.
   - Alternatively (simpler): render normally to an offscreen texture at the target
     pixel dimensions, then display the texture scaled up to fill the viewport with
     nearest-neighbor sampling. This avoids per-vertex snapping and correctly shows
     anti-aliasing artifacts.

4. **GUI:** add "Pixel Preview" to the View menu in `app.rs`; show the active PPI in the
   status bar. A PPI picker (72/96/150/300) is sufficient for MVP.

### Overprint Preview

1. **Identify overprinting nodes:** during `build_geometry`, check if a node's fill
   color matches any `SpotColor` with `overprint = true` in `doc.spot_colors`
   (`Document::spot_colors` — verify field name in `document.rs`). Tag these nodes in
   `NodeSnapshot`.

2. **Composite mode:** for overprinting nodes, replace the normal `PREMULTIPLIED_ALPHA_BLENDING`
   with a multiply blend: `src_color * dst_color` (equivalent to `BlendMode::Multiply`
   per issue #17). This simulates ink-on-ink overprint without knock-out.

3. **View flag:** only activate overprint compositing when `ViewMode::OverprintPreview`
   is active; Normal mode always uses default knock-out.

4. **GUI:** "Overprint Preview" toggle in the View menu; show a status bar indicator
   when active.

### Shared

- Both modes are view-only; they do not mutate the document or affect headless/SVG export.
- The modes should be mutually exclusive (selecting one clears the other).

## Affected Modules

- `crates/photonic-gui/src/app.rs` — `ViewMode` enum, View menu items, status bar
- `crates/photonic-render/src/renderer.rs` — `pixel_preview` flag, vertex snapping or
  offscreen rasterize-and-display path; overprint blend mode per node
- `crates/photonic-render/src/pipeline.rs` — multiply blend variant (shared with #17)
- `crates/photonic-core/src/document.rs` — `SpotColor::overprint` (already present)

## Risks & Open Questions

- **Pixel Preview accuracy:** snapping vertices changes tessellation topology (triangles
  may collapse); the offscreen-texture approach is safer but requires a resize to match
  the chosen PPI × document dimensions, which can be large.
- **Overprint color accuracy:** true overprint simulation requires CMYK separation; RGB
  multiply is only an approximation. Must be documented as approximate until M5.
- **SpotColor matching:** fills currently use `Color` (RGBA floats), not a reference to
  a `SpotColor` ID. Matching by hex value is fragile; ideally fills reference a spot
  color ID directly. This may require a `FillKind::SpotColor(Uuid)` variant (see #24 /
  #20 for precedent).
- **Mutually exclusive modes:** the UI must prevent PixelPreview + OverprintPreview being
  active simultaneously.

## Acceptance Criteria

- [ ] Pixel Preview shows aliasing/snapping as it would appear in a raster export at the
      chosen PPI; zooming in makes the pixel grid visible.
- [ ] Overprint Preview changes the composite for nodes whose fill matches a
      `SpotColor { overprint: true }`; non-overprint colors are unaffected.
- [ ] Both modes are non-destructive: toggling them off restores the normal render.
- [ ] Normal headless export and SVG export are unaffected by the view mode state.

## Effort Estimate

**S** (Overprint Preview alone) to **M** (both modes + PPI picker UI + offscreen rasterize path).
