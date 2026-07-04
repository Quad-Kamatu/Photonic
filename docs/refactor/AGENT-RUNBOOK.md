# Agent Runbook — Work Packages, Model Assignments & Prompts

Operational companion to the [master plan](README.md). This is what you hand the
fleet. It contains: the full work-package (WP) table with model tier per package,
the dispatch order, the copy-paste prompt templates for each tier, and the
non-negotiable per-PR guardrails.

---

## 1. Master work-package table

Tier legend: **Codex** = judgment/architecture · **Hermes-DS** = Hermes running
Deepseek V4 (big long-context mechanical moves) · **Hermes-Q** = Hermes running
Qwen 3.7 (many small mechanical files).

| WP | Description | Tier | Model | Deps | ~Diff | Parallel |
|---|---|---|---|---|---:|---|
| **0** | Baseline: freeze main, golden snapshot, refactor CI job | Codex | — | — | small | 1 |
| **1A** | Lift `nodes.rs` shared helpers → `handlers/shared/` | Codex | — | 0 | small | 1 |
| **1D** | `server.rs` dispatch → per-domain `dispatch/*.rs` | Hermes | Hermes-DS | 0 | med | 1 |
| **1C** | `server.rs` `tool_list()` → `schema_gen.rs` | Hermes | Hermes-DS | 0 | large-move | 1 |
| **1E** | `protocol.rs` 296 Args → `protocol/args/*.rs` | Hermes | Hermes-DS | 0 | large-move | ×4 |
| **1B-1…9** | Split `nodes.rs` → 9 domain handler files | Hermes | DS+Q (see 01) | 1A,1D | med each | ×9 |
| **1F** | `#[register_tool]` macro (Layer B, stretch) | Codex | — | 1A–1E | large | 1 |
| **2A** | Extract independent panels/editors | Hermes | Hermes-Q | 0 | med | seq* |
| **2B** | Per-drawer `Ctx` facades (+`DrawerSection` trait) | Codex | — | 2A | med | 1 |
| **2C** | Migrate 43 drawer sections behind facades | Hermes | Hermes-Q | 2B | med | ×6 |
| **3A** | `draw()` drain-actions → `ui_actions.rs` (6.6k) | Hermes | Hermes-DS | 0 | large-move | seq* |
| **3B** | `ui_drawers`/`ui_dialogs`/`canvas_overlays`/`artboard_ui` | Hermes | Hermes-Q | 0 | med | seq* |
| **3C** | `FrameState`/`CanvasInput`/`DrawPhase`; split render/input | Codex | — | 3A,3B | large | 1 |
| **3D** | `canvas_input.rs` + `tool_dispatch.rs` | Hermes | Hermes-Q | 3C | med | seq* |
| **4A** | `CommandHistory` methods → `history/*.rs` | Hermes | Hermes-Q | 0 | med | 1 |
| **4B** | `Command` trait + serde-on-trait-objects + 2 exemplars | Codex | — | 4A | large | 1 |
| **4C** | Migrate 17 command variants behind trait | Hermes | Hermes-Q | 4B | med | ×4 |
| **5A** | Extract 7 render passes (+texture-builder scaffold) | Codex+Hermes | Hermes-Q | 0 | med | seq* |
| **5B** | Refactor `build_geometry` by node kind | Codex | — | 5A | large | 1 |
| **6A–C** | Optional core tidy (document/export/import) | Hermes | Hermes-Q | — | med | seq* |

`seq*` = all sub-WPs touch **one** file, so run them **sequentially** (one agent,
one-PR-at-a-time) to avoid self-conflict — they do NOT fan out like 1B/1E/2C/4C,
which each land in distinct new files.

### Rough tier split of total mass
- **Hermes (~85% of the LOC moved):** 1B, 1C, 1D, 1E, 2A, 2C, 3A, 3B, 3D, 4A,
  4C, 5A, 6*. Almost all of the ~63k god-module lines.
- **Codex (~15%, the load-bearing 15%):** 0, 1A, 2B, 3C, 4B, 5B, (+1F stretch).
  Every seam-cut, every new abstraction, every interleaved-concern split.

---

## 2. Dispatch order (what to launch when)

1. **WP-0** (Codex) — nothing starts until the golden snapshot + refactor CI job
   exist and `main` is frozen on the target files.
2. Launch the **three independent tracks in parallel:**
   - **MCP:** 1A (Codex) → then 1D, then fan out 1B ×9; 1C & 1E run anytime.
   - **GUI-app:** 3A → 3B (sequential Hermes) → 3C (Codex) → 3D.
   - **GUI-panels:** 2A (sequential Hermes) → 2B (Codex) → 2C ×6.
   - **core/render:** 4A → 4B (Codex) → 4C ×4;  5A → 5B (Codex).
3. **Per-wave Codex rebase/integration pass** reconciles branches + re-runs the
   golden snapshot before the next fan-out.
4. **Stretch Layer-B** (1F macro) only after the mechanical MCP split ships and
   proves stable.

---

## 3. Guardrails — identical for every PR, every tier

Every agent PR MUST, before opening:
- [ ] `cargo build --workspace --locked` — green
- [ ] `cargo test --workspace --locked` — green
- [ ] `cargo fmt --all --check` — clean
- [ ] `cargo clippy --workspace --all-targets --locked` — **no new warnings**
- [ ] `cargo run -p photonic-mcp --bin dump_tools | python3 tools/gen-mcp-docs.py`
      → `docs/mcp-api.md` **byte-identical** (MCP WPs especially)
- [ ] Golden snapshot hashes (from WP-0) **unchanged**
- [ ] Diff is **pure code motion** — reviewer can confirm no logic line was
      edited (moved lines match verbatim). Any necessary logic change is a STOP
      → escalate to Codex/human, do not "fix while moving."
- [ ] One WP = one branch = one PR, branched from current `main`.

**Hard prohibitions for Hermes agents:**
- Do NOT change behavior, rename public items, alter signatures, or "improve"
  code while moving it.
- Do NOT touch files outside your WP's declared target list.
- Do NOT invent traits/abstractions — if the code resists a clean move, STOP and
  report; that means it was mis-classified and belongs to Codex.
- Do NOT edit `docs/mcp-api.md` by hand (it's generated).
- Do NOT modify the `.photon` format, tool schemas, or test expectations.

---

## 4. Prompt template — Hermes (mechanical extraction)

> **Task:** Behavior-preserving extraction, WP-`<id>`.
> **Repo:** Photonic (Rust workspace). Branch from `main` as `refactor/wp-<id>`.
>
> **Move** the following code **verbatim** (no logic edits, no renames, no
> reformatting beyond what `cargo fmt` produces) from `<source file>` lines
> `<range>` into new file `<target file>` as an `impl <Type>` block (or free
> functions in the target module). Wire up `mod`/`use`/`pub use` so all existing
> call sites resolve unchanged.
>
> **Constraints:**
> - Every moved line must be identical to the original (diff = delete+add of the
>   same text). If you cannot move it without editing logic, **STOP and report
>   why** — do not modify it.
> - Touch only: `<explicit file list>`. Nothing else.
> - Do not rename, do not change signatures, do not add abstractions.
>
> **Definition of done (all must pass, paste the output):**
> `cargo build --workspace --locked`, `cargo test --workspace --locked`,
> `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked`
> (no new warnings). For MCP WPs also run the `dump_tools` doc-regen and confirm
> `git diff --exit-code docs/mcp-api.md` is clean. Report the golden-snapshot
> hash and confirm it matches the WP-0 baseline.
>
> Open one PR titled `refactor(wp-<id>): <summary>`. Body: list files moved and
> paste the green CI/gate output.

## 5. Prompt template — Codex (seam-cut / abstraction)

> **Task:** Architectural refactor, WP-`<id>` (Layer B / seam-cut).
> **Repo:** Photonic (Rust workspace). Branch `refactor/wp-<id>` from `main`.
>
> **Objective:** `<e.g. Introduce the `Command` trait replacing the 19-arm
> apply/inverse matches, with serde-safe tagged (de)serialization that preserves
> the existing `.photon` on-disk format; migrate two exemplar variants
> (`UpdateNode`, `GroupNodes`) as the template the fleet will follow.>`
>
> **You own the design decisions** — module boundaries, trait shape, naming —
> because the downstream Hermes fan-out (`WP-<deps>`) inherits them. Optimize for
> a pattern that is trivially repeatable by a cheaper model: after your PR,
> migrating each remaining item should be near-mechanical.
>
> **Hard constraints:**
> - **Zero behavior change.** Golden snapshot (WP-0) and `docs/mcp-api.md` must
>   be unchanged. `.photon` format and tool schemas are frozen.
> - Land shared helpers/abstractions such that Hermes agents never need to
>   invent anything — only follow your exemplar.
>
> **Deliverables:** the new abstraction + wiring + 1–2 migrated exemplars + a
> 5-line "how to migrate the rest" note in the PR body for the fleet. All WP-0
> guardrails (§3) green; paste the output.

---

## 6. Escalation & re-routing
- A Hermes agent that hits "can't move without editing logic" → the WP was
  mis-tiered. Re-scope: either Codex lifts the entangled helper first (like 1A),
  or the WP moves to Codex.
- A Codex WP that turns out to be pure motion → downgrade to Hermes, save budget.
- Wrong-tier guesses are **cheap** because the CI + golden-snapshot gate catches
  any breakage before merge. Bias toward launching, not deliberating.

## 7. Model-tier cheat sheet (§4 rubric, condensed)
**Hermes iff:** behavior-preserving ∧ CI-decidable ∧ no new abstraction ∧
localized coupling ∧ repetitive.
**Codex if any:** invents an abstraction ∨ separates interleaved concerns ∨
non-obvious shared-mutable-state ∨ boundary/naming judgment the fleet depends on
∨ cross-cutting sequencing.
**Within Hermes:** Deepseek V4 → big long-context single-block moves (1C, 1E, 3A);
Qwen 3.7 → the many small per-file extractions (everything else).
