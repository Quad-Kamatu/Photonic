# 03 — GUI `panels/mod.rs` Decomposition

**File:** `gui/src/panels/mod.rs` (8,199). **Track:** GUI (can run beside 02).

egui UI. 43 `draw_*` section functions across 6 drawer groups, plus toolbar,
tools, layers, vertex, and fill/stroke/effect editors. **High internal cohesion
(sections don't call each other) but high fan-in coupling** through one context
object.

## The coupling hotspot
```rust
pub(crate) struct PropPanelCtx<'a> {
    doc: &'a Document,
    /* 60+ &mut refs: tool state, UI flags, numeric inputs, color pickers,
       selection flags, geometry params … */
    action: Option<PanelAction>,   // the only output channel back to the app
}
```
All 43 sections take `PropPanelCtx`. `draw_drawer()` (1,675–1,787) is the single
dispatch point; `PanelAction` (90–730) is the only way a section signals the app.
The 60-ref context is what blocks a clean mechanical split of the *drawer*
sections — so those wait for Codex (WP-2B).

---

## Tier-A (Hermes) — sections that DON'T touch `PropPanelCtx`

These are self-contained egui functions; extract each to its own file, verbatim.

### WP-2A — Independent panels & editors  ·  **Hermes (Qwen 3.7) ×several**
| New file | Source lines | ~Lines | Notes |
|---|---|---:|---|
| `panels/toolbar.rs` | 860–925 | ~65 | chrome |
| `panels/tools_panel.rs` | 928–1,072 | ~145 | tool selector + hotbar |
| `panels/layers_panel.rs` | 1,161–1,344 | ~185 | layer tree |
| `panels/vertex_editor.rs` | 1,346–1,595 | ~250 | direct-select UI |
| `panels/fill_editor.rs` | 7,108–7,667 | ~560 | no `PropPanelCtx` |
| `panels/stroke_editor.rs` | 7,672–7,939 | ~270 | no `PropPanelCtx` |
| `panels/effect_editors.rs` | 7,944–8,199 | ~256 | glow + gaussian glow |
| `panels/assets/*.rs` | 5,545–6,951 | ~1,400 | swatch/library **list** UI, minimal logic |
| `panels/history_drawer.rs` | 6,465–6,780 | ~315 | list views + branch switch |

Fill/stroke/effect editors and assets/history are the highest-value Hermes moves
(large, low-logic). **All target the same `panels/mod.rs` → serialize or single-
agent-sequential** to avoid self-conflict (like the app track).

---

## Tier-B (Codex) — break `PropPanelCtx`, then hand back to Hermes

### WP-2B — Per-drawer `Ctx` facades  ·  **Codex**  ·  gate for 2C
The 60-ref mega-context is the real refactor. Codex replaces it with a facade
per drawer group, each carrying only the refs *that group's* sections need, over
a shared read-only `&Document`:
- `inspector::Ctx`, `modify::Ctx`, `arrange::Ctx`, `assets::Ctx`,
  `document::Ctx`, `history::Ctx`.
- Optionally formalize a `DrawerSection` trait (`fn draw(&mut self, ui, ctx) ->
  Option<PanelAction>`) so each section becomes a **plug-in** and `draw_drawer()`
  iterates a registry instead of hardcoding the section list (Layer B end-state,
  mirrors the MCP `Tool` registry idea).

**Why Codex:** designing the split of a 60-ref god-context into cohesive
per-group facades — and deciding what's shared vs. group-local — is rubric
C1+C3. Migrate **one exemplar section per group** in this PR.

### WP-2C — Move the 43 drawer sections behind the facades  ·  **Hermes (Qwen 3.7) ×6**  ·  needs 2B
One agent per drawer group, following the Codex exemplar:
| Group | Sections | Source lines | Complexity |
|---|---:|---|---|
| `panels/inspector/` | 5 | 1,789–6,879 (navigator, selected_node, tool options, symbol overrides, text-var binding) | med logic |
| `panels/modify/` | 11 | 4,300–4,899 (boolean/blend/pathfinder/distribute…) | med-high |
| `panels/arrange/` | 7 | 4,900–5,225 (align/distribute/dimensions) | med |
| `panels/assets/` | 8 | 5,545–6,951 | **low (mostly 2A already)** |
| `panels/document/` | 9 | 5,255–6,775 (grammar/export profiles/actions/triggers) | high (biz logic) |
| `panels/history/` | 2 | 6,465–6,780 | low |

Once behind a small group-local `Ctx`, each section is a mechanical move → Hermes.

## Summary
| WP | Tier | Model | Deps |
|---|---|---|---|
| 2A independent panels/editors | Hermes | Qwen 3.7 | — |
| 2B per-drawer Ctx facades (+optional DrawerSection trait) | Codex | — | after 2A |
| 2C migrate 43 sections | Hermes | Qwen 3.7 ×6 | 2B |

End state: `panels/mod.rs` → thin dispatcher + `{toolbar,tools,layers,vertex}` +
`{fill,stroke,effect}_editor` + 6 drawer-group subdirs. If the `DrawerSection`
trait lands, adding a panel section becomes a one-file plug-in.
