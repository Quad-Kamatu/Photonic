# General-Purpose Live Effects Rendering (#18) — Design Proposal

> Status: design scaffold (not an implementation).

## Summary

The renderer has three hardwired glow slots per `SceneNode` — `outer_glow: GlowEffect`,
`inner_glow: GlowEffect`, and `gaussian_glow: GaussianGlow` (`node.rs:154-158`) with a
dedicated two-pass blur pipeline (`renderer.rs:509-649`, `pipeline.rs` blur pipelines).
Drop shadow, gaussian blur as a standalone effect, and feather are not in the render
path. This proposal extends the existing blur infrastructure to cover the missing effects
without yet building the full appearance stack (issue #24).

## Scope

**In:**
- **Drop Shadow** (dx, dy, blur radius, color, opacity) rendered as an offset blurred
  silhouette composited beneath the object.
- **Gaussian Blur** as a standalone per-node effect (blur the node's fill, not just a
  glow halo).
- **Feather** (soft-fade the object's alpha boundary by a given radius).
- Effects apply to `PathNode` and `GroupNode`; `TextNode` is a stretch goal.
- SVG export via `<filter>` / `<feDropShadow>` / `<feGaussianBlur>`.

**Out:**
- Full appearance stack reordering / multiple stacked effects (issue #24).
- Inner shadow, bevel/emboss, pattern overlay (later).

## Proposed Approach

1. **Extend the node model** — add optional fields to `SceneNode` parallel to the
   existing glow slots:
   ```rust
   pub drop_shadow: DropShadow,   // new struct in node.rs
   pub object_blur: ObjectBlur,   // sigma-only, no color offset
   pub feather: Feather,          // radius, fades alpha edge
   ```
   Keep the same `#[serde(skip_serializing_if = "disabled")]` pattern as `GlowEffect`.

2. **Drop Shadow** — reuse the `GaussianGlowJob` / two-pass blur machinery in `renderer.rs`.
   Before drawing the node, render its silhouette to `glow_tex_a` translated by
   `(dx, dy)` in screen space, blur it (H then V pass), composite beneath the node.
   Tint with shadow color × opacity. Add `DropShadowJob` analogous to `GaussianGlowJob`.

3. **Gaussian Blur (object-level)** — render node fill geometry to `glow_tex_a`,
   H-blur to `glow_tex_b`, V-blur back to `glow_tex_a`, then composite `glow_tex_a`
   onto the surface in place of the normal fill draw. The existing
   `blur_pipeline_h` / `blur_pipeline_v` + `BlurParams` uniform are already wired; only
   a new dispatch path is needed.

4. **Feather** — implemented in the existing `fill_pipeline` as a screen-space alpha
   gradient: render the node to `glow_tex_a`, run a narrow V+H blur at the feather
   radius, use the blurred result as an alpha mask when compositing.

5. **`build_geometry` snapshot** — extend `NodeSnapshot` (`renderer.rs:703`) to include
   the new effect fields; emit `DropShadowJob`, `ObjectBlurJob`, `FeatherJob` into
   per-frame queues analogous to `pending_gaussian_glows`.

6. **MCP handlers** — add `set_drop_shadow`, `set_object_blur`, `set_feather` in
   `crates/photonic-mcp/src/handlers/nodes.rs` (or a new `effects.rs`).

7. **SVG export** — in `crates/photonic-core/src/export.rs`, emit a `<defs><filter>`
   block with `<feDropShadow>` or `<feGaussianBlur>` and reference it via `filter=`.

## Affected Modules

- `crates/photonic-core/src/node.rs` — new effect structs (`DropShadow`, `ObjectBlur`, `Feather`)
- `crates/photonic-render/src/renderer.rs` — `NodeSnapshot`, new job queues, render passes
- `crates/photonic-render/src/pipeline.rs` — `BlurParams`, blur pipeline reuse
- `crates/photonic-core/src/export.rs` — SVG `<filter>` definitions
- `crates/photonic-mcp/src/handlers/nodes.rs` — new MCP tool handlers

## Risks & Open Questions

- **Texture lifetime:** `glow_tex_a`/`glow_tex_b` are currently sized to the surface;
  they must be large enough for shadow offsets that push geometry off-canvas.
- **Draw order:** drop shadow must composite *under* the node fill; the current frame
  loop renders geometry then glow passes. Ordering must be specified clearly.
- **Object blur vs. group blur:** blurring a group means blurring the composite of its
  children; the group must render to an offscreen texture first (see also issue #17
  group isolation).
- **Transition to appearance stack (#24):** these slots should map 1:1 to appearance
  stack entries when #24 lands. Design migration now to avoid churn.

## Acceptance Criteria

- [ ] Drop shadow renders on-canvas and in headless export with correct dx/dy/blur/color.
- [ ] Gaussian blur visually blurs the object fill at the given sigma.
- [ ] Feather fades the object boundary smoothly.
- [ ] SVG export emits a valid `<filter>` reference; renders correctly in a browser.
- [ ] Effect parameters update live without requiring a document reload.
- [ ] No regression on existing `outer_glow` / `inner_glow` / `gaussian_glow` rendering.

## Effort Estimate

**M** — drop shadow is the heaviest; object blur and feather reuse the same machinery.
