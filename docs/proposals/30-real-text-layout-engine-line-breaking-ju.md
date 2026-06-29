# Real text layout engine: line breaking, justification, and area-type reflow (#30) — Design Proposal

> Status: design scaffold (not an implementation).

## Summary

`TextNode` (node.rs lines 311-369) has a rich data model: `align`, `line_height`,
`letter_spacing`, `area_path_id`, `next_frame`/`prev_frame` threading, `text_indent`,
`tab_stops`, `paragraph_spacing_before`/`after`. None of these are honoured by the
renderer, which delegates to glyphon's basic `Buffer` layout at lines 402-507 of
`renderer.rs`. Glyphon can lay out a single block to a fixed width, but area-type
reflow, frame threading, and justification are not implemented. This proposal adds a
paragraph layout layer between `TextNode` and glyphon.

## Scope

**In:**
- Line breaking to a measure (frame width or `area_path_id` boundary width at each
  scanline) using glyphon's existing `cosmic-text` shaper as the glyph source.
- Justification modes: honour `TextAlign::Left/Center/Right` (already stored) and add
  `TextAlign::Justified` — expand word/letter spacing to fill the line measure.
- Paragraph spacing: apply `paragraph_spacing_before`/`after` between `\n`-delimited
  paragraphs.
- First-line indent: offset the first line of each paragraph by `text_indent`.
- Tab stops: replace `\t` characters with advance to the next `tab_stops` position (or
  default every 4 em).
- Area type: when `area_path_id` is set, fit text inside the referenced closed-path
  boundary; compute the scanline width at each baseline using the path's horizontal
  extent at that y.
- Frame threading: when text overflows its area, push the remainder into the
  `next_frame` `TextNode`; when the area grows, pull back from `next_frame`.

**Out:**
- Knuth-Plass optimal line breaking (nice to have, greedy first).
- Hanging punctuation (stretch goal).
- Hyphenation (separate feature, gap doc §6 note).
- Right-to-left / bidi text (requires cosmic-text bidi, separate issue).
- Vertical text layout (`TextNode.vertical = true`) — separate issue.

## Proposed approach

1. **Layout module** (new `photonic-core/src/text_layout.rs` or
   `photonic-render/src/text_layout.rs`):
   - `pub struct LayoutFrame { pub lines: Vec<LayoutLine> }` where `LayoutLine` holds
     glyphs, their x-offsets, baseline y.
   - `pub fn layout_text(node: &TextNode, measure: f64, area_shape: Option<&PathData>)
     -> LayoutFrame`:
     (a) split `content` into paragraphs on `\n`,
     (b) for each paragraph, use `cosmic-text`'s `Buffer` with explicit `line_width` to
         break runs into lines (greedy: accept glyphs until line overflows),
     (c) apply `text_indent` to first line of paragraph,
     (d) apply `paragraph_spacing_before`/`after` as extra baseline gaps,
     (e) apply `tab_stops` advance,
     (f) for `TextAlign::Justified`, redistribute slack across word spaces on full
         (non-last) lines.
   - When `area_path_id` is set, compute `measure` at each baseline y via the path's
     horizontal chord width at that y (using `kurbo::BezPath` intersection helpers in
     `photonic-core/src/path.rs`).

2. **Renderer integration** (`photonic-render/src/renderer.rs`, `render_text_pass`
   lines 406-507):
   - Replace the current `Buffer`-per-node construction with a call to `layout_text`.
   - Feed the resulting `LayoutLine` glyph positions directly to glyphon's `TextArea`
     (which accepts pre-positioned glyph IDs via `custom_glyphs` or a per-line
     `Buffer`).
   - Alternatively: call `layout_text` to get final line extents, then construct one
     glyphon `Buffer` per line with the correct `line_width` set to enforce the measure.

3. **Frame threading**:
   - After `layout_text` returns, if there is overflow (text that did not fit the area),
     and `next_frame` is set: truncate `LayoutFrame` at the fitting glyph, write the
     overflow content to the `TextNode` referenced by `next_frame` (as a derived
     mutation, not a user Command, similar to constraint evaluation).
   - This derived write must not be undoable independently — it is a consequence of
     the source node's content.

4. **`measure_text` API** (`renderer.rs` line 314): update to use `layout_text` for
   accurate multi-line measurements (currently returns a single-line glyphon estimate).

5. **SVG export** (`photonic-core/src/export.rs`): emit `<text>` with `<tspan>`s per
   line, honouring `x` offset for indent/justification; area type remains as positioned
   `<tspan>` elements (SVG does not support area text natively).

## Affected modules (real paths)

- `crates/photonic-core/src/text_layout.rs` (new) — paragraph layout engine
- `crates/photonic-render/src/renderer.rs` — `render_text_pass`, `measure_text`
- `crates/photonic-render/src/headless.rs` — capture text pass
- `crates/photonic-core/src/export.rs` — `<tspan>` SVG emission
- `crates/photonic-core/src/node.rs` — `TextAlign` enum: add `Justified` variant
- `crates/photonic-core/src/path.rs` — horizontal chord helper for area-type scanlines

## Risks & open questions

- **glyphon API coupling**: `render_text_pass` constructs glyphon `Buffer` objects
  directly. The new layout layer must either wrap or extend glyphon's `Buffer` rather than
  replace it, since glyphon handles rasterisation (not just shaping).
- **Area-type scanline width**: computing horizontal chord width at each baseline via
  `kurbo` path intersection is O(segments × lines) — may be slow for complex paths.
  Cache the chord-width table per area path per frame.
- **Threading derived writes**: mutating `next_frame` content as a derived (non-command)
  write risks diverging from undo state if the user manually edits the overflow frame.
  Consider treating overflow frame content as read-only in the GUI when it has a
  `prev_frame` set.
- **`TextAlign::Justified`**: adding a new variant to the existing `TextAlign` enum is a
  model change that requires a `format_version` migration guard (old files that
  deserialise this will fail if the variant is unknown).

## Acceptance criteria

- [ ] A text node with a frame width breaks lines at word boundaries and the result is
      stable across canvas/headless/SVG render paths.
- [ ] Left, center, right, and justified alignment all render correctly.
- [ ] `paragraph_spacing_before`/`after` and `text_indent` are visibly applied.
- [ ] Area type: text wraps inside the boundary of the referenced closed path.
- [ ] When area text overflows, the surplus flows into the `next_frame` node; resizing
      the area boundary pulls it back.
- [ ] `measure_text` returns accurate multi-line dimensions.

## Effort estimate

**XL** — The paragraph layout engine is substantial; area-type scanline computation and
frame threading add further complexity; glyphon integration must not break existing
single-line rendering.
