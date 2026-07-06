# Layer Options — Professional Parity Plan (Photoshop + Illustrator)

Goal: bring Photonic's **layer options** to full professional parity with **both**
Adobe Photoshop and Adobe Illustrator. Scope is the whole surface — layer
compositing, layer styles, advanced blending, masks, layer types (incl. smart
objects), and the Illustrator organizational flags — sequenced so the lowest-risk,
highest-leverage work lands first.

Sequencing decision: **keystone + panel parity first**, then outward.
Scope decision: **everything, including smart objects.**

---

## 0. Keystone finding (why this is the linchpin)

`Document::nodes_in_draw_order()` (`crates/photonic-core/src/document.rs:1155`)
renders by **flattening every visible layer into one flat node list** — the layer
boundary dissolves at composite time. Consequences:

- Layers today are purely **organizational**: stacking order + visibility +
  template-dim. Nothing else about a layer affects the pixels.
- `Layer.opacity` and `Layer.blend_mode` (`layer.rs:8`) are **dead fields** — they
  serialize but nothing reads them.
- There is no layer mask, layer effect, layer isolation, or knockout, because
  there is no per-layer compositing pass to hang them on.

Both Photoshop and Illustrator treat a layer as a **compositing unit**: render the
layer's contents to an isolated buffer, then apply `opacity → blend → mask →
effects` onto the canvas. **Group nodes in Photonic already composite this way**
(`crates/photonic-render/src/compositor.rs:416`; `GroupNode` in `node.rs` carries
`clip_children`, `clip_node_id`, `live_boolean`, etc.). So the keystone is:

> **Composite each layer as an implicit group.**

Nearly every parity feature below (layer opacity, blend, mask, effects, isolation,
knockout, blend-if) is blocked on this one change, and it *reuses the existing
group-compositing path* rather than inventing a new one.

### Adjustment-layer subtlety (must preserve)
Adjustment layers are non-destructive and sample the **composite beneath them**
across the whole stack (`raster/adjust.rs`, `RasterNode.adjustment` at
`node.rs:481`). Per-layer isolation must **not** break this: an adjustment layer
still reads the accumulated canvas below it, not just its own layer buffer. P0
keeps the current "apply to composite beneath" semantics.

---

## 1. Current-state inventory (condensed)

Legend: ✅ have · ◐ partial · ✗ missing. Anchors are `file:line` at time of writing
(verify before editing).

| Area | State |
|---|---|
| Layer struct (`layer.rs:8`) | id, name, visible, locked, **opacity**(dead), **blend_mode**(dead), color tag, is_template, node_ids |
| Blend modes (`layer.rs:49`) | ✅ 16 · ✗ ~11 (Linear Dodge/Add, Subtract, Divide, Linear Burn, Vivid/Linear/Pin Light, Hard Mix, Darker/Lighter Color, Dissolve) |
| Draw path (`document.rs:1155`) | flattens layers → one list (honors layer.visible only) |
| Node compositing (`compositor.rs:416`) | ✅ per-node mask, blend, adjustment |
| Node effects (`node.rs:240`) | ✅ drop_shadow, outer_glow, inner_glow, gaussian_glow, object_blur, feather · ✗ inner shadow, bevel/emboss, satin, overlays, stroke-fx |
| Adjustments (`raster/adjust.rs:919`) | ✅ 19 kinds, non-destructive |
| Masks | ✅ raster mask per-node (`node.rs:472`) · ✗ vector mask · ✗ mask enable/disable, density, feather, unlink |
| Clipping | ◐ group-scoped (`node.rs:571`) · ✗ per-layer "clip to layer below" |
| Locking | ◐ single `bool` (layer + node) · ✗ transparency/pixels/position/all |
| Layer types | ✗ fill layers · ◐ smart objects (symbols only, frozen at `node.rs:257`) |
| Illustrator flags | ✅ template · ✗ print/non-print, outline/preview per-layer, dim-% · ◐ release-to-layers (MCP only), target/appearance dots |
| GUI panel (`panels/layers_panel.rs`) | ✅ create/delete/rename/reorder/reparent/merge/flatten/color-tag/adjustment-tray/clip-mask/collect/sublayer · ✗ opacity slider, blend dropdown, duplicate button, per-object eye/lock, reverse, search, isolate, mask enable/disable, locate |
| MCP (`schema_gen.rs`) | ✅ create/delete/duplicate/update/merge/flatten/reorder/move/collect/release/active + set_opacity/blend/visibility/locked (node-level), masks, clip mask · ✗ layer opacity/blend in `update_layer`, reverse, effects beyond drop shadow |

---

## 2. Phased plan

Each phase lists: **data** (core model), **render** (compositor/export), **GUI**
(panel/editor), **MCP** (tools), **tests**, **risk**, **size** (S/M/L/XL). File
format: any new serialized field ships with `#[serde(default)]` + a
`CURRENT_FORMAT_VERSION` bump (`document.rs`) and back-compat migration.

### P0 — Layer as a compositing unit *(keystone)* — size **M**
- **render:** in the compositor, composite each visible layer into its own buffer
  then blend onto the accumulator with `layer.opacity` + `layer.blend_mode`; reuse
  the group-compositing routine (treat a layer as a top-level implicit group).
  **Fast path:** if a layer has `opacity==1 && blend==Normal && no mask/fx`,
  flatten inline as today (no extra buffer) — avoids per-layer buffer cost.
  Preserve adjustment-layer "sample composite beneath" semantics.
- **export:** SVG/PDF/raster must honor it too — SVG wraps a layer's nodes in
  `<g opacity mix-blend-mode>`; raster/PDF composite per-layer. (`core/src/export`)
- **data:** none (fields already exist + serialize).
- **GUI/MCP:** none yet (P1 surfaces the controls).
- **tests:** golden render — a half-opacity Multiply layer over a base matches the
  group-composited equivalent; fast-path vs isolated-path pixel-identical when
  opacity=1/Normal.
- **risk:** perf (extra buffers) → mitigated by fast path; adjustment-layer
  ordering → covered by a regression test.

### P1 — Panel parity (surface what exists) — size **M**
- **GUI (`layers_panel.rs`):** per-layer **opacity slider** + **blend-mode
  dropdown** (now live via P0); **duplicate-layer** button; **per-object eye/lock**
  icons on object rows (node `visible`/`locked` already exist); **active-layer
  target indicator**; **reverse order**; **select-all-on-layer**; **locate object**
  (expand+scroll to selection); **search/filter** box; **mask enable/disable**.
- **data:** add `Mask.enabled: bool` (`#[serde(default = true)]`) for enable/disable.
- **MCP:** extend `update_layer` to accept `opacity` + `blend_mode`; add
  `reverse_layers`, `select_layer_contents`. Regenerate `docs/mcp-api.md`.
- **tests:** update_layer opacity/blend round-trip; reverse-order correctness.
- **risk:** low (mostly wiring).

### P2 — Blend modes to full set — size **S–M**
- **data:** extend `BlendMode` (`layer.rs:49`) with the ~11 missing modes.
- **render:** implement each in the CPU compositor blend fn + GPU shader; map to
  SVG `mix-blend-mode` where one exists, else rasterize-on-export (document the
  non-representable modes).
- **MCP:** widen the `set_blend_mode` enum + schema; regenerate docs.
- **tests:** per-mode 2-pixel blend math table.

### P3 — Locking granularity — size **S**
- **data:** replace single `locked: bool` with lock flags (`lock_all`,
  `lock_transparency`, `lock_pixels`, `lock_position`) on `Layer` and `SceneNode`;
  migrate old `locked → lock_all`.
- **enforcement:** move/transform/paint/anchor tools honor the specific locks.
- **GUI:** lock sub-toggles in the row/menu (PS-style lock cluster).
- **tests:** each lock blocks exactly its operation and nothing else.

### P4 — Layer Styles (FX) engine — size **XL** (the big visual lever)
- **data:** an ordered, non-destructive effect stack `effects: Vec<LayerEffect>` on
  node **and** layer, each with its own blend+opacity+enabled. Variants: DropShadow,
  InnerShadow, OuterGlow, InnerGlow, Bevel&Emboss (style/depth/size/soften/
  angle/altitude/gloss/highlight/shadow), Satin, ColorOverlay, GradientOverlay,
  PatternOverlay, Stroke (position/size/fill). Document-level **global light**
  (angle/altitude). Migrate existing `drop_shadow`/glows into the stack (back-compat
  shim so old files still render).
- **render:** an effects compositor honoring Photoshop's fixed effect z-order
  (drop shadow → pattern/gradient/color overlay → satin → stroke → inner
  shadow/glow → bevel → outer glow), applied at node and (via P0) layer level.
- **GUI:** a Layer Style editor (PS-style dialog), an `fx` badge on rows, and
  copy/paste/clear/scale-effects.
- **MCP:** `add_layer_effect` / `set_layer_effects` / `clear_layer_effects` (+ keep
  `add_drop_shadow` as sugar). Regenerate docs.
- **export:** SVG filter approximations where possible, else rasterize-the-effect
  on export; note fidelity limits.
- **tests:** golden renders per effect; migration test (old drop_shadow → stack).
- **risk:** large surface; land effect-by-effect behind the shared stack.

### P5 — Advanced compositing — size **L**
- **fill-opacity** separate from layer/object opacity (fill-opacity affects
  fill+interior, not effects — the PS distinction).
- **knockout** (shallow/deep) and **blend-clipped-layers-as-group**.
- **Blend-If** sliders (this-layer / underlying, per channel, split handles).
- render plumbing in the layer/group compositor; data fields + serde defaults.
- **tests:** fill-opacity vs opacity divergence with an effect present; blend-if
  threshold math.

### P6 — Masks & layer types — size **XL** (contains the largest single piece)
- **Vector mask:** attach a path mask to a node/layer; render clips by path
  (distinct from raster mask). GUI add/edit/enable/disable/unlink.
- **Clip to layer below:** per-node/per-layer PS-style clipping (clip to the
  composited item beneath), distinct from group clipping.
- **Fill layers:** a layer/node kind holding a `Fill` (solid/gradient/pattern)
  filling the layer bounds/mask, non-destructive.
- **Smart objects** *(largest; may itself be multi-step)*: a `SmartObject` node
  kind wrapping an **embedded or linked** source (a sub-document or placed file)
  under a **non-destructive transform**, with a **smart-filter** stack (adjustments/
  filters applied live). Double-click edits the source; edits re-render instances.
  Consider unifying with the existing symbol system (`node.rs:257`). Linked-file
  reload + embed/convert. This is the deepest architectural item — plan it as its
  own sub-plan when reached.
- **tests:** vector-mask clip golden; clip-to-below vs group-clip; smart-object
  non-destructive transform + smart-filter re-render.

### P7 — Illustrator organizational flags — size **M**
- **print/non-print** layer (export honors); **outline/preview** per-layer (render
  mode); **dim-images %** (template dim slider, generalize current template dim);
  **release-to-layers** button in the panel (tool exists in MCP); **target/
  appearance dots** on rows (indicate an object/layer carries appearance/fx);
  **layer color** drives the on-canvas selection/bbox highlight.
- mostly GUI + small render/export flags.

---

## 3. Cross-cutting concerns

- **File format:** P1/P3/P4/P5/P6 add serialized fields → bump
  `CURRENT_FORMAT_VERSION` (`document.rs`) with `#[serde(default)]` + migration;
  keep loading old `.photon` files (see the history/format back-compat precedent).
- **Export fidelity:** SVG/PDF can't express every PS mode/effect. Rule: map to a
  native SVG feature when one exists, else **rasterize that element on export** and
  log what was approximated (no silent fidelity loss).
- **Undo/history:** every new op commits through `Command::UpdateNode` (or a new
  command) as a single undoable step, matching existing tools.
- **MCP:** each phase that adds capability adds/extends tools and **regenerates
  `docs/mcp-api.md`** (`cargo run -p photonic-mcp --bin dump_tools | python3
  tools/gen-mcp-docs.py > docs/mcp-api.md`).
- **Model reconciliation:** Photonic stays closer to Illustrator (Layer = container,
  appearance on objects/groups) while P0 makes a Layer *also* a compositing unit —
  giving the Photoshop behaviors without abandoning the object-appearance model.

## 4. Suggested delivery order

P0 → P1 (ship together: keystone + visible controls) → P2 → P3 → P4 (effect-by-
effect) → P5 → P6 (fill layers/vector masks first, smart objects last as its own
sub-plan) → P7. Each phase is independently shippable and testable; none blocks a
release.
