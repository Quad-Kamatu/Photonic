# 06 — Cohesive Core Modules (Optional, Low Priority)

**Files:** `core/src/document.rs` (1,830), `export.rs` (1,523),
`import.rs` (1,330). **Track:** core, **Wave 5, all Hermes, all optional.**

Structural analysis found these are **not god modules** — they're large but
cohesive, well-layered, and low-coupling. They are **not on the critical path**
and should only be touched after the real god modules (01–05) are done, if the
~1,500-line soft cap is being enforced strictly. Every split here is pure
mechanical motion → Hermes (Qwen 3.7).

---

## WP-6A — `document.rs` — trim the megastruct  ·  **Hermes**  ·  optional
The `Document` struct has ~40 public fields (it's the global repository for
layers, nodes, swatches, styles, symbols, constraints, grammar, actions,
triggers, workspaces, dimensions, artboards…). The **type definitions** around
it (lines 16–506) are already well-factored per concept and can move out:
- `document/typography.rs` — `CharacterStyle`, `ParagraphStyle`.
- `document/swatches.rs` — `ColorSwatch`, `GradientSwatch`, `SpotColor`.
- `document/assets.rs` — `Symbol`, `Pattern`, `GraphicStyle`, `WidthProfile`.

Keep the `Document` struct + its cohesive `impl` (add/remove/find/query node &
layer ops, ~100 focused methods) in place. Do **not** attempt to shrink the field
count itself — that would ripple through the whole codebase and is out of scope.

## WP-6B — `export.rs` — split emitters  ·  **Hermes**  ·  optional
- `export/svg.rs` — options + `export_svg()` / `export_nodes_as_svg()`.
- `export/svg_emitters.rs` — the ~20 `emit_*()` per-node-kind functions.
- `export/pdf.rs` — `export_pdf()`.

Low coupling (depends only on Document/Fill/Stroke; no mutable state). Optional.

## WP-6C — `import.rs` — split parse phases  ·  **Hermes**  ·  optional
Recursive-descent SVG parser with clean phase boundaries:
- `import/parser.rs` — entry point + element recursion.
- `import/style.rs` — CSS + computed styles.
- `import/paint.rs` — fill/stroke/gradient resolution.
- `import/geometry.rs` — path-data + transform parsing.

---

## Recommendation
Skip Wave 5 unless the size cap is being enforced as a hard gate. The
maintainability payoff is marginal (these files are already navigable), the risk
is near-zero, and the agent-time is better spent finishing 01–05. If done, batch
all three to a single Hermes agent sequentially — they're independent files, no
cross-deps, no self-conflict.
