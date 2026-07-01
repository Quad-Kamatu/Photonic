# Direct Select Tool Icon Redesign (#161)

> Status: **implemented**. Small, self-contained GUI polish — one-line icon swap.
>
> ## What this PR implements
> - Swapped the `Tool::DirectSelect` arm of `Tool::icon()` in
>   `crates/photonic-gui/src/tools/mod.rs:75` from `ph::BEZIER_CURVE` to
>   `ph::VECTOR_TWO`. `Tool::icon()` is the single source of truth for the glyph,
>   so the change propagates to every render surface (hotbar, tools panel,
>   global search, active-tool status readouts) at once.
> - `VECTOR_TWO` verified present in `egui-phosphor 0.7.3` regular set
>   (`variants/regular.rs`, glyph `\u{EE64}`) and unused by any other tool — no
>   collision or missing-font risk.
> - `label()` ("Direct Select") and `description()` ("Edit individual anchor
>   points") left unchanged; they already read well.
>
> Verification: `cargo build --release` OK, `cargo test -p photonic-gui` OK
> (0 tests, none for this crate), `cargo check --workspace` OK. Warnings present
> are pre-existing and unrelated.
>
> ## Remaining work
> None — the issue is a single-glyph swap and is fully addressed. A future,
> larger effort could introduce a custom hollow-arrow (Illustrator-convention)
> icon, but that requires an additional font variant / asset pipeline and is
> intentionally out of scope here.

## Summary

The Direct Select tool's toolbar glyph is `ph::BEZIER_CURVE`
(`crates/photonic-gui/src/tools/mod.rs:75`). It reads poorly and is too close
in spirit to the other path tools (Pen = `ph::PEN_NIB`, and the generic
"curve" reading gives no hint that this tool edits *individual anchor points*).
It also fails the issue's core requirement: it must read clearly **and** be
visually distinct from the Selection tool (`Tool::Select` = `ph::CURSOR`, a
solid pointer arrow).

`Tool::icon()` is the single source of truth for the glyph everywhere it is
rendered — the hotbar (`hotbar.rs:113`), the tools panel / grouped tool
buttons (`panels/mod.rs`), global search (`global_search.rs:110`), and the
active-tool status readouts (`app/mod.rs`). Changing the one `match` arm
updates every surface at once. No other tool currently uses the candidate
glyphs.

## Approach

Swap the `Tool::DirectSelect` arm in `Tool::icon()` from `ph::BEZIER_CURVE`
to **`ph::VECTOR_TWO`**. Phosphor's `VECTOR_TWO` depicts a bezier path segment
with visible **anchor points and control handles** — precisely what the Direct
Select tool manipulates. It is:

- **Clear** — the anchor-point dots directly communicate "edit points on a path".
- **Distinct from Select** — `ph::CURSOR` is a plain solid arrow pointer;
  `VECTOR_TWO` is an anchored path segment. No confusion between the two
  selection tools.
- **Distinct from neighbours** — differs from Pen (`PEN_NIB`) and from the old
  `BEZIER_CURVE`, so the drawing/selection cluster in the hotbar stays legible.

`ph::VECTOR_TWO` is confirmed present in the linked `egui-phosphor 0.7.3`
regular set (`variants/regular.rs`) and is unused elsewhere, so there is no
glyph-collision or missing-font risk (egui_phosphor bundles the full font).

This is deliberately the minimal change. The tool's `label()` ("Direct Select")
and `description()` ("Edit individual anchor points") already read well and are
left as-is.

## Scope

**In**
- `crates/photonic-gui/src/tools/mod.rs` — change the `Tool::DirectSelect` arm
  of `icon()` to `ph::VECTOR_TWO`.
- `cargo build --release` must succeed; visually confirm the new glyph renders
  in the hotbar and tools panel.

**Out**
- No changes to tool behaviour, hit-testing, or keyboard shortcut.
- No custom/vector-drawn icon or new asset pipeline — this stays within the
  bundled Phosphor set.
- No re-icon of the Selection tool or any other tool.

## Alternatives considered

- **`ph::VECTOR_THREE`** — same family, three anchors; slightly busier at small
  sizes. `VECTOR_TWO` is cleaner at toolbar scale.
- **Hollow-arrow (Illustrator convention)** — the familiar "white arrow" for
  Direct Select. Rejected: the loaded regular Phosphor set has no clean
  hollow-cursor variant, and pulling in the `fill`/outline variant fonts is out
  of proportion for a p3 polish.
- **`ph::PATH`** — a bare curved line with no anchor dots; reads as "curve", the
  same weakness as the current glyph.
