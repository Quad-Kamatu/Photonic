# Photonic God-Module Refactor — Master Plan

**Status:** Planning (no implementation yet)
**Owner:** Joseph
**Created:** 2026-07-04

This directory is the plan for breaking Photonic's largest "god modules" into
predictable, plug-in-shaped components, and for delegating that work across a
fleet of coding agents at two capability tiers.

> **Read order:** this file (strategy + rubric + sequencing) → the per-module
> specs (`01`–`06`) → [`AGENT-RUNBOOK.md`](AGENT-RUNBOOK.md) (the work-package
> table, model assignments, and agent prompt templates).

---

## 1. The problem, in numbers

Photonic is a ~110k-LOC, 7-crate Cargo workspace. The LOC is pathologically
concentrated: **8 files hold ~63k lines (57% of the tree)**, and two single
files exceed 14k lines each.

| File | Lines | Crate | Character |
|---|---:|---|---|
| `gui/src/app/mod.rs` | 14,946 | gui | `PhotonicApp` struct (85 fields) + an 11,179-line `draw()` |
| `mcp/src/handlers/nodes.rs` | 17,719 | mcp | 179 tool handlers (79% of all MCP tools) |
| `mcp/src/server.rs` | 6,791 | mcp | Dispatch table (1,883 L) + tool schema list (4,641 L) |
| `gui/src/panels/mod.rs` | 8,199 | gui | 43 UI drawer sections behind one 60-ref context |
| `mcp/src/handlers/document.rs` | 5,782 | mcp | 99 document/asset handlers (already domain-grouped) |
| `mcp/src/protocol.rs` | 4,780 | mcp | 296 near-identical `*Args` deserialize structs |
| `core/src/history.rs` | 2,789 | core | 19-variant `Command` enum + `CommandHistory` |
| `render/src/renderer.rs` | 2,259 | render | `build_geometry` (840 L) + isolated render passes |

These are not evenly "bad." Structural analysis (see per-module docs) shows most
of the mass is **mechanical, repetitive, and behavior-preserving to split** —
which is exactly what makes tiered delegation possible.

### Non-god modules (leave mostly alone)
`core/src/document.rs` (1,830), `export.rs` (1,523), and `import.rs` (1,330) are
large but **cohesive and well-layered**. They get optional low-priority splits
only (see [`06`](06-core-cohesive-modules.md)); they are not part of the critical
path.

---

## 2. Goals & non-goals

**Goals**
- G1. No single Rust file over **~1,500 lines** when done (soft cap; some
  generated-schema and match files may exceed by exception).
- G2. Each extracted unit is **cohesive** (one responsibility) and its blast
  radius is knowable without reading the whole crate.
- G3. Where a god module is really "N of the same thing" (MCP tools, panel
  sections, undo commands, render passes), convert it into a **plug-in
  component** behind a trait/registry so future items are added in one place,
  not three.
- G4. **Zero behavior change.** The refactor is provably behavior-preserving:
  the app renders and the 300+ MCP tools respond identically.

**Non-goals**
- No feature work, no bug-fixing (except incidental, and only if flagged).
- No dependency upgrades, no rustfmt/clippy config changes.
- No public API / file-format / `.photon` schema changes.
- Not chasing the cohesive core modules (§ non-god above).

---

## 3. Strategy: two layers, in this order

Every god module is attacked in the same two-layer sequence. The layer boundary
**is** the model-tier boundary.

### Layer A — Mechanical decomposition (make it navigable)
Move code, unchanged, into cohesive sibling modules. In Rust this is cheap and
safe because **`impl` blocks can be split across files** while retaining full
access to private struct internals — the codebase *already* does this (the
`app/` directory has 10 such `impl PhotonicApp` submodules). Layer A is pure
code motion + `use`/`mod` wiring. It is CI-verifiable to the byte and needs no
design judgment → **Hermes tier**.

### Layer B — Plug-in architecture (make it extensible)
Introduce the abstraction that turns "N copies of a pattern" into "N plug-ins
behind a trait/registry":
- MCP tools → a `Tool` registry (a `#[register_tool]` proc-macro or `inventory`
  linkme-style registration) so a new tool is one file, not edits in
  `protocol.rs` + `server.rs` dispatch + handler.
- Undo → a `Command` trait (one impl per command) replacing the 19-arm
  `apply`/`inverse`/`coalesce` matches.
- Panels → a `DrawerSection` trait replacing the 60-ref `PropPanelCtx` +
  central dispatch.
- Renderer → a `RenderPass` trait for the isolated passes.

Layer B invents abstractions and reshapes control flow → **Codex tier**.

### The orchestration pattern that ties them together

> **Codex cuts the seam; Hermes fills it.**

For each god module, a **single Codex agent** lands the *scaffolding* PR first:
it extracts shared helpers into a `util`/`shared` module, defines the trait or
registry, and migrates **one or two exemplar** components through it. Then a
**fleet of Hermes agents** migrates the remaining N components in parallel,
each following the exemplar mechanically. This de-risks the cheap tier: Hermes
never has to invent the pattern, only repeat it, and each Hermes PR is a small,
independently CI-checkable diff.

---

## 4. Model-assignment rubric

Two tiers of coding agent are available:

- **Codex agents** — high-capability, judgment-bearing. Reserved for
  architecture, seam-cutting, and interleaved-concern separation.
- **Hermes agents** running **Qwen 3.7** or **Deepseek V4** — fast, cheap,
  reliable at repetitive, well-specified, verifiable transforms.

Assign a work package to **Hermes** iff **ALL** hold:
1. **Behavior-preserving** — pure code motion / re-export, no logic edits.
2. **CI-decidable** — success is fully provable by `cargo build`/`test`/`fmt`
   + the MCP-doc-drift check (§6). No human judgment needed to know it worked.
3. **No new abstractions** — it slots into a structure Codex has already
   defined (or a trivially obvious module boundary).
4. **Localized coupling** — the extracted block is self-contained, or its only
   escape hatches are helpers Codex has already lifted into `shared`.
5. **Repetitive** — ideally it's one of N near-identical items.

Assign to **Codex** if **ANY** hold:
1. It must **invent an abstraction** (trait, registry, proc-macro, `Ctx` facade).
2. It must **separate interleaved concerns** (e.g. render vs. input in the same
   loop; doc+history mutation that must stay locked together).
3. It touches **shared mutable state** in non-obvious ways.
4. It requires **judgment about module/API boundaries** or naming that the rest
   of the fleet will depend on.
5. It's a **cross-cutting sequencing decision** (what to extract first so the
   rest becomes mechanical).

**Deepseek V4 vs Qwen 3.7 within the Hermes tier:** route the *largest,
most-repetitive, lowest-coupling* jobs (schema-struct splitting, dispatch-arm
regrouping, wholesale block moves) to whichever your harness benchmarks faster;
prefer **Deepseek V4** for the big long-context moves (the 4,641-line schema
list, the 6,596-line drain-actions block) and **Qwen 3.7** for the many small
per-section/per-handler files. The per-WP table in the runbook gives a default;
swap freely — the CI gate makes a wrong guess cheap to catch.

---

## 5. Global sequencing (waves)

Dependency-ordered. Waves may overlap across crates (the MCP and GUI tracks are
independent), but within a module Codex's scaffold PR **must** land before its
Hermes fan-out.

```
WAVE 0  Baseline & guardrails      (Codex, 1 agent)   — see §6
        └─ snapshot golden outputs, land the refactor CI job, freeze main

WAVE 1  MCP crate                  ── independent track ──────────────
  1a Codex: lift shared helpers in nodes.rs (apply_style, affine, z_key…)
  1b Hermes ×9: split nodes.rs → domain handler files       (needs 1a)
  1c Hermes ×1: extract server.rs tool_list() → schema_gen.rs (no deps)
  1d Hermes ×1: extract server.rs dispatch → dispatch/mod.rs
  1e Hermes ×4: split protocol.rs Args structs by domain
  1f Codex: (optional/stretch) #[register_tool] macro → collapses 1c–1e

WAVE 2  GUI panels                 ── independent track ──────────────
  2a Hermes ×N: extract self-contained panels (toolbar, tools, layers,
                vertex, fill/stroke/effect editors, assets, history)
  2b Codex: design per-drawer Ctx facades to break the 60-ref PropPanelCtx
  2c Hermes ×4: move Inspector/Modify/Arrange/Document sections behind 2b

WAVE 3  GUI app struct             (depends on nothing; can run beside 2)
  3a Hermes: extract drain-actions match → ui_actions.rs (6.6k L, wholesale)
  3b Hermes: extract ui_drawers, ui_dialogs, canvas_overlays, artboard_ui
  3c Codex: FrameState/CanvasInput/DrawPhase abstractions; split render/input
  3d Hermes: extract canvas_input, tool_dispatch behind 3c

WAVE 4  core/history & render      ── independent track ──────────────
  4a Hermes: extract CommandHistory methods → stacks/checkpoints/branches
  4b Codex: introduce Command trait; migrate 2 exemplar variants
  4c Hermes ×4: migrate remaining Command variant clusters behind 4b
  4d Hermes: extract renderer passes (text/glow/effects/scene/frame/capture/camera)
  4e Codex: refactor build_geometry (interleaved node-kind branches)

WAVE 5  Optional core tidy          (Hermes, low priority)
        document.rs / export.rs / import.rs submodule splits — see 06
```

Critical path is **Wave 1** (largest mass) and **Wave 3c/4e** (the only hard
Codex-judgment pieces). Everything else parallelizes.

---

## 6. Verification protocol (the safety net that makes this delegable)

The whole plan rests on the fact that **the CI harness can prove a
behavior-preserving refactor is correct without a human.** Every PR — Codex or
Hermes — must pass the existing gates:

- `cargo build --workspace --locked`
- `cargo test --workspace --locked` (47 test files today)
- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets --locked` (must not add warnings)
- **MCP doc-drift gate:** CI regenerates `docs/mcp-api.md` from
  `server::tool_list()` and fails on any diff. This is the killer feature for
  the MCP track — it *mechanically proves* no tool was dropped, renamed, or had
  its schema altered during a handler/schema split.
- `cargo deny check`

**Wave 0 adds two refactor-specific guardrails (Codex):**
1. **Golden-output snapshot.** A scripted MCP session that exercises a broad
   set of tools and hashes the resulting `.photon` + rendered PNG, plus a GUI
   headless-render snapshot. Captured on frozen `main` *before* any refactor
   PR. Every refactor PR must reproduce the identical hashes. This catches
   behavior drift that unit tests miss.
2. **`.mailmap`/ownership note + a "refactor freeze"** on the target files so
   feature work doesn't collide mid-wave.

**Per-PR acceptance checklist** (in `AGENT-RUNBOOK.md`) is identical for both
tiers; the only difference is that Codex PRs additionally require human review
of the *new abstraction's* shape, while Hermes PRs are auto-mergeable once green
+ diff-is-pure-motion is confirmed by a Codex/human spot-check.

---

## 7. Branching, PR size & conflict avoidance

- Branch from a **clean `main`** (today's tree has uncommitted work on
  `feat/curve-fit-and-fullscreen-startup` — land or shelve that first).
- **One work package = one branch = one PR.** Keep Hermes PRs under ~800 lines
  of *net* diff where possible (moves show as add+delete; that's fine).
- **Conflict avoidance:** the fan-out waves are partitioned so no two
  concurrent Hermes agents touch the same file. The one shared chokepoint is
  `server.rs`'s dispatch match (every MCP handler split re-points arms there) —
  so **1b, 1c, 1d are serialized on `server.rs`**, or better, 1d lands the
  dispatch into its own `dispatch/` module *first* so the 9 handler-split PRs
  each touch a different `dispatch/*.rs` file. This is called out in doc `01`.
- Each wave ends with a Codex "integration/rebase" pass that reconciles the
  parallel branches and re-runs the golden snapshot.

---

## 8. Risk register

| # | Risk | Likelihood | Mitigation |
|---|---|---|---|
| R1 | Hermes silently drops/edits logic during a "move" | Med | Golden snapshot + MCP doc-drift gate + Codex diff-is-pure-motion spot-check |
| R2 | `server.rs` dispatch becomes a merge chokepoint | High | Land `dispatch/` module split (1d) before the 9 handler splits; partition arms into per-domain dispatch files |
| R3 | `apply_style` (16 callers) mis-lifted breaks all shape/text creation | Med | Codex owns the helper-lift PR (1a); dedicated tests before fan-out |
| R4 | `PropPanelCtx` / `draw()` interleaving resists clean extraction | Med | Codex-only (2b, 3c); explicitly Tier-B, not handed to Hermes |
| R5 | Parallel branches drift from `main` and rot | Med | Short waves, per-wave Codex rebase pass, refactor freeze on target files |
| R6 | Register-tool macro (1f/plug-in) over-engineers | Low | It's optional/stretch; ship mechanical split first, macro only if churn justifies |
| R7 | Two-tier fleet cost/coordination overhead | Med | Runbook gives copy-paste prompts + per-WP CI gate; wrong tier assignment is cheap to catch and re-route |

---

## 9. Success criteria

- [ ] No non-generated Rust file > ~1,500 lines (exceptions logged).
- [ ] `nodes.rs`, `server.rs`, `protocol.rs`, `app/mod.rs`, `panels/mod.rs`,
      `history.rs`, `renderer.rs` all decomposed per their specs.
- [ ] At least one god module (MCP tools **or** undo commands) converted to a
      true plug-in registry (Layer B) as the reference implementation.
- [ ] Golden snapshot identical from first refactor PR to last.
- [ ] `docs/mcp-api.md` byte-identical throughout (proves 300+ tools intact).
- [ ] `docs/architecture.md` updated to describe the new module layout.

---

## 10. Document index

| Doc | Scope | Dominant tier |
|---|---|---|
| [`01-mcp-decomposition.md`](01-mcp-decomposition.md) | nodes / server / protocol / document handlers | Hermes (Codex scaffold + optional macro) |
| [`02-gui-app-decomposition.md`](02-gui-app-decomposition.md) | `PhotonicApp` + `draw()` | Mixed (Hermes moves, Codex phases) |
| [`03-gui-panels-decomposition.md`](03-gui-panels-decomposition.md) | 43 drawer sections | Mixed (Codex `Ctx`, Hermes sections) |
| [`04-core-history-decomposition.md`](04-core-history-decomposition.md) | `Command` enum → trait, `CommandHistory` | Codex trait + Hermes migration |
| [`05-render-decomposition.md`](05-render-decomposition.md) | renderer passes + `build_geometry` | Hermes passes, Codex geometry |
| [`06-core-cohesive-modules.md`](06-core-cohesive-modules.md) | document / export / import (optional) | Hermes |
| [`AGENT-RUNBOOK.md`](AGENT-RUNBOOK.md) | Work-package table, assignments, agent prompts, guardrails | — |
