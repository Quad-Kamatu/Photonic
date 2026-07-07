# 12 — Agent Execution Plan: Model Tiers & Parallelism

How AI implementation agents build this module: which model tier each role runs on (Fable → Opus → Sonnet), what runs in parallel, what is strictly sequential, and the coordination rules. Governed by the workspace `claude-subagent-protocol` (spawn-once + SendMessage reuse, sonnet-first, permission-before-opus).

---

## 1. Model-tier policy

Tier by **decision density**, not by phase prestige. Escalate only where a wrong decision is expensive to unwind.

| Tier | Use for | Rationale |
|---|---|---|
| **Fable** (main session) | Orchestration; frame-graph IR final design sign-off; cross-doc consistency reviews; phase-gate reviews; anything touching ≥3 crates at once; unblocking stuck agents | Highest judgment per token; keeps global context; never used for bulk code emission |
| **Opus** (permission-gated per protocol) | Architecturally hard, self-contained builds: P1 renderer rework (dirty tracking + persistent buffers + COMPOSITE_SHADER), engine core (graph compile/eval, decode scheduler, A/V clock), audio DSP correctness, color-math kernels | Deep single-domain reasoning; mistakes here are structural |
| **Sonnet** (default) | Well-specified implementation against these docs: timeline data model + edit ops (01 is prescriptive), panels/UI, MCP tool wiring (mechanical 4-touch-point pattern), caption/provider adapters, presets, tests, docs/Features.md updates, golden-corpus authoring | Spec docs carry the design; execution is bounded; cheapest correct tier |

Standing rules:
- **Sonnet-first**: every task starts Sonnet unless this plan marks it Opus/Fable. An agent that detects it is out of depth reports back rather than improvising; orchestrator re-issues at higher tier.
- **Opus requires explicit user permission** at spawn time (protocol requirement). Batch permission per phase, not per agent.
- **Review inversion**: code written at tier N is reviewed at tier ≥ N. Phase-gate reviews (architect + verifier) run at Fable-in-session or Opus.

## 2. Role catalog

| Role | Tier | Cardinality | Responsibility |
|---|---|---|---|
| Orchestrator | Fable | 1 (main session) | Wave dispatch, integration, merge order, conflict arbitration |
| Engine builder | Opus | 1 per engine work-package | photonic-video internals (02) |
| Render builder | Opus | 1 | P1 + video texture path (03) |
| Core-model builder | Sonnet | 1 | core::timeline types, ops, commands, migration (01) |
| UI builders | Sonnet | 2–3 parallel | Timeline panel, monitor/transport, adaptive panels (04); later color page (07 UI), node editor (08 UI), mixer UI (09 UI) |
| MCP builder | Sonnet | 1 | handlers/video.rs + schema/args/dispatch + doc regen (10) |
| Domain builders | Sonnet | 1 per domain | Captions/providers (06), presets/import-export surface (05), grading ops (07 math may escalate Opus), audio DSP (09 DSP is Opus), node catalog (08) |
| QA (arcwright-qa) | Sonnet | 1, reused via SendMessage | Runs test suites, MCP story scripts, perf harness per phase |
| Verifier (arcwright-verifier) | Sonnet | 1, reused | Goal-backward check at each phase gate (L1 exists → L4 real data flows) |
| Architect reviewer (arcwright-architect) | Opus | 1 (2 during P3 and P8) | Design-conformance review of each wave's diff against these docs. P3/P8 run concurrent Opus builder waves — a single reviewer would queue them; spawn a second architect instance for those phases (or serialize the Opus waves if permission for a second is declined) |

## 3. Hard sequencing constraints (cannot parallelize)

Dependency spine — each item blocks everything after it:

0. **CommandHistory::revision extension** (bump on execute/undo/redo + public accessor + `changes_since` introspection, 03 §2.1) → P1 tessellation cache AND P3 engine snapshotting both consume it. Lives in `photonic-core/src/history/` (a §4 choke-point file) but must land in **P1**, before the P2 Core-model builder touches history.
0b. **`ir.rs` signature stub (design-only)** — the `IrOp` enum + `FrameGraph` type signatures from 02 §2, landed as a compile-checked stub before P1's render-builder wave: 03 §3.4's texture pool and Tier-B conversion pass are keyed by IR-shaped contracts, and without the stub the P1 render builder guesses at them. Authored by the orchestrator or architect (design tier), reviewed against 02 verbatim; no evaluator body, types only.
1. **01 data model types + migration** → everything (all crates import these types).
2. **02 frame-graph IR definition** (ir.rs signatures, not full evaluator) → engine eval, 07 GradeOps, 08 catalog lowering, 03 texture pool contract.
3. **P1 renderer rework** → any real-time playback work (P3+ GPU eval). Vector editing golden-corpus must pass BEFORE P1 merges (00 §7 risk 1).
4. **Engine facade (EngineCmd/EngineFrame API, 02 §1)** → timeline UI playback wiring (04), MCP playback/export tools (10).
5. **timeline/ops.rs edit ops** → both GUI interactions (04) and MCP tools (10) — parity by construction requires ops land first.
6. **Mixer core (P3: gain/pan/mute/solo)** → EQ/comp/automation/ducking (P8) — DSP nodes plug into an existing graph.
7. **Per-clip composition splice (02 §2 step 3) proven by tests** → fusion UI (P7/P8) — UI must not drive engine design.
8. **08 authors review 02's IR before P3 engine code starts** (explicit gate, 00 §7 risk 2).

## 4. Safe parallelism (and its boundaries)

Parallel-safe because **file-disjoint** (worktree isolation optional but recommended for same-crate work):

| Wave-mates | Why safe |
|---|---|
| Core-model builder (photonic-core/src/timeline/) ∥ Render builder (photonic-render/) | Different crates; IR types stubbed via 02 signatures |
| Engine decode/media (photonic-video/src/{decode,media}/) ∥ Engine graph (graph/) ∥ Engine audio (audio/) | Sibling modules, interfaces pinned by 02 §1 |
| Timeline panel (gui/src/app/timeline/) ∥ Media pool panel ∥ Monitor/transport | Separate new module files; shared PhotonicApp fields negotiated up front by orchestrator (single small PR adding fields first — the "fields PR" pattern) |
| MCP builder ∥ any GUI builder | Different crates; both call the same core ops |
| Captions (06) ∥ Grading (07) ∥ Audio DSP (09) ∥ Export presets (05) | Disjoint engine submodules + disjoint panels, all downstream of the spine |
| Test/golden-corpus authoring ∥ everything | tests/ + tools/ only |

NOT parallel-safe (serialize or pre-split):
- Two agents touching `history/mod.rs`, `schema_gen.rs`, `dispatch.rs`, `panels/mod.rs` enums, or `app/mod.rs` struct fields — these are **append-choke-point files**. Rule: orchestrator lands a skeleton PR (enum variants, empty match arms, struct fields) first; wave agents then fill disjoint bodies.
- Anything editing `document.rs` / `migration.rs` concurrently.
- Two agents inside the same engine submodule.

## 5. Per-phase wave plan

Notation: `[..]` = one parallel wave; `→` = barrier (previous wave merged + reviewed).

**P1** — decomposed into gate-able stories (the pattern every phase follows; stories map 1:1 onto 11 §6's exit-criteria checkboxes):
- **S1** `CommandHistory::revision` extension (spine 0, Sonnet) — bump on execute/undo/redo + accessor + `changes_since`; existing history tests stay green.
- **S1b** `ir.rs` signature stub (spine 0b, design tier) — types only, reviewed against 02.
- **S2** Vector golden-output baseline capture (Sonnet, ∥ S1) — corpus captured on pre-P1 code, blessed, committed.
- **S3** Dirty tracking + persistent GPU buffers (Opus, after S1) — existing compositor/headless suites pass unchanged; S2 corpus byte-identical.
- **S4** COMPOSITE_SHADER wiring + render-to-texture (Opus, after S3) — S2 corpus re-diffed: byte-identical for untouched paths, PSNR ≥45dB for previously-approximated blend modes (03 §2.6).
- **S5** f16 video texture path + pool (Opus, after S1b+S4) — 03 §3 contract consumed, conversion-pass WGSL in the validation table.
- Gate: architect diff review + QA full-suite + golden diff + CI green.
**P2** `[Core-model (Sonnet, Opus review on commands/migration)]` → fields/skeleton PR (orchestrator) → `[Timeline panel ∥ mode switch+monitor shell ∥ MCP timeline-edit tools]` → gate (AS-1 arrange/cut via MCP script).
**P3** IR review gate (08 authors + architect) → `[Engine graph (Opus) ∥ decode/media (Opus) ∥ audio core (Sonnet) ∥ proxy (Sonnet)]` → **interim architect checkpoint on the engine facade** (EngineCmd/EngineFrame/EngineStatus as built, before any consumer code — a wrong facade decision must surface here, not at phase end) → `[playback wiring in GUI ∥ MCP playback/render_frame_at ∥ media pool panel]` → gate (SS-1 subset on proxy; CAP-022 crash-recovery of a timeline project verified per D-12).
**P4** `[Export loop+encoders (Sonnet, Opus review) ∥ preset system+dialog ∥ aspect/reframe UX ∥ transcode tool]` → gate (AS-1 minus captions).
**P5** `[Provider trait+adapters (Sonnet) ∥ caption track UI ∥ TTS flow ∥ subtitle interchange]` → gate (AS-1 complete, provider mock in CI).
**P6** `[Keyframe curve editor ∥ vector-doc animation binding (Opus — touches core+render) ∥ transitions ∥ effect-param animation]` → gate (AS-3 core).
**P7** `[Grade ops GPU kernels (Opus) ∥ color page UI ∥ scopes (Sonnet) ∥ LUT/CDL parsers (Sonnet)]` → gate (AS-2 grade pass).
**P8** `[Node editor UI ∥ node catalog lowering ∥ DSP fx (Opus) ∥ automation lanes UI ∥ ducking preset]` → final gate (AS-2, AS-3 complete; SS-1..3 full).

Every gate = QA suite green + verifier goal-backward pass + architect diff review + CI gates + Features.md/mcp-api.md updated. Fail → findings loop back to the owning builder (SendMessage reuse, no respawn).

## 6. Communication & state rules

- Task registry (TaskCreate/TaskList) is the single work ledger; one task per work-package, `blockedBy` encodes §3 spine.
- Builders report via Agent return values; long-lived roles (QA, verifier, architect) are spawned once and re-engaged via SendMessage.
- Worktree isolation for any wave with ≥2 builders in one crate.
- No agent edits another agent's in-flight files; conflicts escalate to orchestrator, never resolved by force-push.
- Context economy: builders receive the relevant spec doc numbers, not the whole set; 01/02 always included.
- **Story template** (every task-registry entry uses it): goal (one sentence), spec sections (doc §), files owned, DoD (tests that must pass + exit-criteria checkbox it closes), tier, blockers. Prevents scope drift between parallel builders in one wave.
- **Phase-kickoff checklist** (orchestrator, before dispatching a wave): spine dependencies merged; CI green on branch tip; the phase's 11 §6 exit criteria re-read; choke-point skeleton PR landed if the wave needs one; reviewer capacity confirmed (§2 architect note).
- **Batched reviews (process amendment, locked with user 2026-07-07):** reviews are NOT per-story. The orchestrator verifies each story's DoD (tests/golden/clippy) at commit time; architect + QA review happens ONCE per phase gate over the whole phase diff. The single exception is the P3 engine-facade interim checkpoint (§5 P3), which stays because consumer code builds on the facade mid-phase. Story-level review pings are retired.
- **Rollback runbook:** every phase merge is revertable as one commit range; the `video` cargo feature (11 §7) is the runtime kill-switch. Incident procedure: flip feature off → ship → revert range on a fix branch → re-land. Recorded here so mid-incident nobody designs the procedure from scratch.
