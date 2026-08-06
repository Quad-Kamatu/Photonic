# [EPIC] Appearance Stack — Non-Destructive Fills, Strokes, and Live Effects (#24) — Design Proposal

> Status: design scaffold (not an implementation).

## Summary

Today a `PathNode` / `TextNode` carries exactly one `Fill` and one `Stroke` field, and
`SceneNode` has three fixed glow slots (`outer_glow`, `inner_glow`, `gaussian_glow`).
There is no way to stack multiple fills/strokes, reorder them, or attach editable
non-destructive effects. This epic replaces the single-fill/stroke model with an ordered
appearance stack that unblocks a large class of professional features documented in
`docs/illustrator-feature-gaps.md` §9 and §30.

## Scope

**In (this epic; sub-issues to be filed):**
- Core model: `AppearanceEntry` enum + `Vec<AppearanceEntry>` on `SceneNode`; migration
  from current single-fill/stroke fields.
- Renderer: composite the stack bottom-to-top per object.
- Appearance panel UI: add / remove / reorder / toggle entries.
- Convert existing destructive effect MCP tools to optional live-effect entries.
- "Expand Appearance" command: bake the stack to flat geometry.
- SVG export of stacked appearance.

**Out:**
- Full warps, distort effects, pathfinder-as-effect (later sub-issues).
- Paragraph style stacking (text-specific; separate).

## Proposed Approach

### 1. Core Model (`crates/photonic-core/src/node.rs`)

Replace `PathNode::fill: Fill`, `PathNode::stroke: Stroke` and `SceneNode::outer_glow` /
`inner_glow` / `gaussian_glow` with:

```rust
pub struct AppearanceStack {
    pub entries: Vec<AppearanceEntry>,
}

pub enum AppearanceEntry {
    Fill(FillEntry),
    Stroke(StrokeEntry),
    Effect(EffectEntry),
}

pub struct FillEntry   { pub fill: Fill, pub enabled: bool, pub blend_mode: BlendMode }
pub struct StrokeEntry { pub stroke: Stroke, pub enabled: bool }
pub struct EffectEntry { pub effect: LiveEffect, pub enabled: bool }

pub enum LiveEffect {
    DropShadow  { dx: f64, dy: f64, blur: f64, color: Color, opacity: f32 },
    GaussianBlur { radius: f64 },
    Feather      { radius: f64 },
    OuterGlow   { color: Color, opacity: f32, size: f64 },
    InnerGlow   { color: Color, opacity: f32, size: f64 },
    // extensible: RoundCorners, Roughen, ZigZag, … as sub-issues
}
```

Move `AppearanceStack` onto `SceneNode` directly (not per-variant), so groups, paths,
and text all share the same structure.

### 2. Schema Migration

- Add a `schema_version: u32` field to `Document` (or a top-level wrapper).
- A migration function reads the old single `fill`/`stroke`/glow fields and constructs an
  `AppearanceStack` with one `FillEntry` + one `StrokeEntry` + up to three `EffectEntry`s.
- Old fields removed from `SceneNode` variants; `#[serde(alias)]` or a versioned enum
  can handle deserialization of v1 documents.
- `photonic-core` exposes `migrate_v1_to_v2(doc: DocumentV1) -> Document`.

### 3. Renderer (`crates/photonic-render/src/renderer.rs`)

Walk the appearance stack bottom-to-top per node in `build_geometry`:
- Each `FillEntry` → tessellate fill, composite with the entry's `blend_mode`.
- Each `StrokeEntry` → tessellate stroke, draw above fills.
- Each `EffectEntry` → dispatch the appropriate GPU pass (blur, glow, shadow — see #18).
- Groups: render children to an offscreen texture, then apply the group's stack on that
  texture (required for group-level effects).

### 4. Appearance Panel UI (`crates/photonic-gui/src/panels/`)

Add an `appearance.rs` panel:
- List entries with drag-to-reorder (egui `DragValue` / list).
- Per-entry: enabled checkbox, color swatch, parameters popover.
- `+` button with a dropdown: Add Fill / Add Stroke / Add Effect (submenu).
- Trash icon to remove an entry.
- Wire to MCP tools via the same document mutation path.

### 5. MCP Tools (`crates/photonic-mcp/src/handlers/`)

New tools (in `nodes.rs` or a new `appearance.rs`):
- `add_appearance_fill(node_id, fill, position?)` — appends or inserts a fill entry.
- `add_appearance_stroke(node_id, stroke, position?)`
- `add_appearance_effect(node_id, effect_type, params)`
- `remove_appearance_entry(node_id, index)`
- `reorder_appearance_entry(node_id, from_index, to_index)`
- `set_appearance_entry_enabled(node_id, index, enabled)`
- `expand_appearance(node_id)` — bake to flat paths.

Convert existing destructive MCP tools (`add_drop_shadow`, `add_outer_glow`, etc.) to
call the new `add_appearance_effect` internally, preserving backward compatibility.

### 6. SVG Export (`crates/photonic-core/src/export.rs`)

- Multiple fills: emit `<path>` elements stacked in a `<g>`, each with its own `fill`.
  SVG does not support multiple fills natively; wrapping in a group is the standard
  workaround.
- Effects: emit `<filter>` per effect type (see #18 for `feDropShadow`, `feGaussianBlur`).
- Stacked strokes: each stroke becomes a separate `<path>` with `fill="none"`.

### 7. Expand Appearance (`crates/photonic-core/src/ops/`)

Add `expand_appearance.rs`: for each node with a non-trivial stack, produce a `GroupNode`
whose children are flat `PathNode`s — one per fill, one per expanded stroke outline
(via `ops/stroke_outline.rs`). Effects that cannot be represented as paths are dropped
(or rasterized, as a stretch goal).

## Affected Modules

- `crates/photonic-core/src/node.rs` — `AppearanceStack`, `AppearanceEntry`, `LiveEffect`
- `crates/photonic-core/src/style.rs` — `Fill`, `Stroke` (unchanged structurally; used
  inside entries)
- `crates/photonic-core/src/document.rs` — `schema_version`, migration
- `crates/photonic-render/src/renderer.rs` — stack-walking draw loop
- `crates/photonic-gui/src/panels/` — new `appearance.rs` panel
- `crates/photonic-mcp/src/handlers/` — new appearance MCP tools
- `crates/photonic-core/src/export.rs` — multi-entry SVG export
- `crates/photonic-core/src/ops/expand_appearance.rs` — new ops file

## Risks & Open Questions

- **Migration breakage:** any document written before this change has bare `fill`/`stroke`
  fields on `PathNode`/`TextNode`. The migration must be lossless and well-tested.
- **Renderer complexity:** bottom-to-top stack compositing with blend modes per entry
  depends on issue #17 (blend modes) being complete or in progress.
- **Performance:** multiple fill/stroke passes per node multiplies tessellation cost;
  must combine with dirty-node caching (#21).
- **Group-level effects:** groups need offscreen render targets to apply effects to their
  composite, which is a significant renderer change also needed for #17 group isolation.
- **SVG fidelity:** SVG has no native stacked fills; the multi-`<path>` approach is a
  workaround that loses semantic round-trip fidelity.
- **`symbol_fill_override` / `symbol_stroke_override`:** these fields on `SceneNode`
  apply to the first fill/stroke entry; the semantics need clarification for multi-entry
  stacks.

## Acceptance Criteria

- [ ] A node can have two or more fills stacked; each renders correctly in order.
- [ ] A node can have a fill + stroke + drop shadow as separate stack entries.
- [ ] Entries can be reordered, toggled, and deleted non-destructively.
- [ ] Existing single-fill/stroke documents load correctly after migration (no data loss).
- [ ] "Expand Appearance" produces flat paths matching on-canvas appearance.
- [ ] SVG export with stacked appearance renders correctly in Firefox and Chrome.
- [ ] MCP tools for add/remove/reorder/expand are fully functional.

## Effort Estimate

**XL** — this is an architectural migration touching every layer of the stack; conservative
estimate is 3-4 milestone-weeks including migration testing and renderer rework.
