# SVG Round-Trip Fidelity: Text-on-Path, Area Type, Effects, Blend Modes, Patterns (#39) — Design Proposal

> Status: design scaffold (not an implementation).

## Summary

`export.rs` emits a flat SVG that silently drops several Photonic features that the document model already stores: blend modes (`Layer::blend_mode`, `BlendMode` enum in `layer.rs`), glow effects (`GlowEffect` / `GaussianGlow` in `node.rs:38–65`), text-on-path (`TextNode::on_path` in `node.rs:326`), area type (`TextNode::area_path` in `node.rs:335`), and pattern fills (`FillKind::Pattern` is absent from `style.rs` but needed). Import (`import.rs`) is limited to the same surface. The result is that any round-trip through SVG silently degrades the document, which directly contradicts the "clean, stable SVG" differentiator.

## Scope (in / out)

**In:**
- Export `mix-blend-mode` CSS property for every `Layer` with a non-`Normal` `BlendMode`.
- Export `<filter id="…">` + `feGaussianBlur` / `feFlood` / `feComposite` for `GlowEffect` outer/inner glow; reference from the node element via `filter="url(#…)"`.
- Export `<textPath href="#path-id">` when `TextNode::on_path` is set; emit the referenced path as a `<defs>` element.
- Export area-type text as a `<foreignObject>` or clipped `<text>` block tied to the area path bounding rect (SVG has no native area type; document the fallback).
- Export `<pattern>` blocks for pattern fills once `FillKind::Pattern` exists.
- Export variable-width strokes as outlined closed paths using the existing `ops/stroke_outline` function.
- Import: parse `mix-blend-mode`, `<filter>` (map `feGaussianBlur` back to `GlowEffect`), `<textPath>`, and `<pattern>` back to the live model.
- Round-trip property tests: serialize → re-parse → assert structural equality.
- Add a `photonic-svg-v1` namespace attribute to versioned output.

**Out:**
- Complex SVG filter graphs beyond glow (drop shadow, displacement map) — separate issue.
- Full CSS text layout within `<foreignObject>` (area type approximation only).
- Pattern brush strokes — belongs in #43.

## Proposed Approach

1. **Blend mode export** (`export.rs`): In `emit_node`, detect the owning `Layer::blend_mode`; emit `style="mix-blend-mode: multiply"` (etc.) on the wrapping `<g>`. Map `BlendMode` variants to their CSS names (1-to-1 for all 16 variants already defined in `layer.rs`).

2. **Effect export** (`export.rs`, new helper `emit_filters`): Walk all nodes before the main SVG body; for each `SceneNode` whose `outer_glow.enabled` or `inner_glow.enabled` is true, push a `<filter>` entry into a `<defs>` section. Write `feGaussianBlur` with `stdDeviation = glow.blur_radius`, `feFlood` for `glow.color`, `feComposite` for blending. Assign a stable ID (`filter-{node_id}`). Attach `filter="url(#filter-{node_id})"` to the element.

3. **Text-on-path export** (`export.rs`): When `TextNode::on_path` is `Some(path_node_id)`, look up the referenced `PathNode`, emit it into `<defs>` as `<path id="tp-{id}" d="…"/>`, then wrap the text content in `<textPath href="#tp-{id}"/>`.

4. **Import parser extensions** (`import.rs`): Extend `SvgContext` / `ComputedStyle` to carry `filter` refs and `textPath` refs; map `feGaussianBlur` back to `GlowEffect`; set `TextNode::on_path` when a `<textPath>` element is found.

5. **Versioning marker**: Add a `xmlns:photonic="…"` attribute and `photonic:version="1"` to the root `<svg>` in `SvgExportOptions`. Document the contract in `docs/`.

6. **Tests**: Add `#[test]` cases in `export.rs` and `import.rs` that build a `Document` with each feature, export to string, re-import, and assert structural equality.

## Affected Modules

- `crates/photonic-core/src/export.rs` — main changes (filter/blend/textPath emission)
- `crates/photonic-core/src/import.rs` — extended parsing in `SvgContext`
- `crates/photonic-core/src/node.rs` — read `GlowEffect`, `TextNode::on_path`, `TextNode::area_path`
- `crates/photonic-core/src/layer.rs` — read `BlendMode` during export
- `crates/photonic-core/src/style.rs` — future `FillKind::Pattern` variant (may be parallel work)
- `docs/` — SVG output contract documentation

## Risks & Open Questions

- **Area type fallback**: SVG has no native area-type text. A `<foreignObject>` works in browsers but breaks in many SVG renderers and Illustrator. Need to decide: approximate with `<foreignObject>` and document the limitation, or expand to multi-line `<text>` with explicit `<tspan dy>` lines (lossy but broadly compatible).
- **Filter ID collisions**: UUIDs for node IDs are long; truncate or hash for readability vs. guaranteed uniqueness.
- **Pattern fills**: `FillKind::Pattern` does not yet exist in `style.rs`; this item blocks until that variant is added (could be gated separately).
- **Import completeness**: Arbitrary SVG `<filter>` graphs from other tools cannot round-trip; we should only reconstruct our own emitted filters (detect by `photonic:version` marker).
- **Blend mode on `Layer` vs. node**: Current model stores blend mode on `Layer`, not on individual `SceneNode`. Illustrator stores it per object. Decide whether to promote to per-node before implementing SVG export.

## Acceptance Criteria

- [ ] Document with all 16 `BlendMode` values exports `mix-blend-mode` correctly; re-import restores the mode.
- [ ] Document with outer/inner glow exports `<filter>` blocks; re-import reconstructs `GlowEffect` fields.
- [ ] `TextNode` with `on_path` set exports `<textPath>`; re-import sets `on_path` to the correct path node.
- [ ] Exported SVG root carries `photonic:version="1"` attribute.
- [ ] Round-trip tests pass for all of the above features without data loss (structural equality).
- [ ] No regression on existing basic export tests.

## Effort Estimate

**M** — The model already stores all the relevant data. The work is emit/parse wiring plus test coverage. Pattern fills are a dependency that may extend the scope if done in this issue.
