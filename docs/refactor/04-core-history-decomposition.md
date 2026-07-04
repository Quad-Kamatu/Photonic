# 04 — `core/history.rs` Decomposition

**File:** `core/src/history.rs` (2,789). **Track:** core (independent).

Undo/redo. Two concerns: the **`Command` enum** (19 variants, each carrying
old+new values for invertibility) and the **`CommandHistory`** state machine.
No UI coupling; only reads/writes `Document`. This is the cleanest candidate in
the tree to become a **true plug-in architecture** (Layer B).

## Structure
| Entity | Lines | Notes |
|---|---|---|
| `Command` enum (19 variants) | 1,482–1,619 | AddNode/RemoveNode/UpdateNode/…/`Batch(Vec<Command>)` |
| `description()` | 1,621–1,665 | undo-menu labels |
| `hydrate()` | 1,667–1,702 | reconstruct for undo |
| `apply()` | 1,704–1,854 | **151-line 19-arm match** |
| `coalesce()` | 1,856–1,924 | gesture merge |
| `inverse()` | 1,926–2,084 | **159-line 19-arm match** |
| `Checkpoint`/snapshots | 2,087–2,150 | named save points |
| `CommandHistory` struct + methods | 2,185–2,789 | undo/redo stacks, debounce, checkpoints, branches |

---

## WP-4A — Extract `CommandHistory` methods  ·  **Hermes (Qwen 3.7)**  ·  no deps
The state-machine methods (lines 2,246–2,789, ~544 L) are logically separable
and independent of the `Command` internals. Split into:
- `history/stacks.rs` — undo/redo stacks, depth/size limiting.
- `history/checkpoints.rs` — checkpoint create/name/serialize.
- `history/branches.rs` — branch fork/merge/restore.
- `history/coalescing.rs` — debounce timers + `coalesce_started` flags.

Pure code motion behind `impl CommandHistory` blocks → Hermes. **Do this first**;
it's independent of the harder trait work and immediately halves the file.

---

## WP-4B — Introduce a `Command` trait  ·  **Codex**  ·  Layer B, gate for 4C
Replace the two 19-arm matches (`apply`/`inverse`, plus `coalesce`/`description`)
with a trait:
```rust
trait Command: Serialize + DeserializeOwned {
    fn apply(&self, doc: &mut Document) -> Result<()>;
    fn inverse(&self) -> Box<dyn Command>;
    fn coalesce(&mut self, next: &Self) -> bool { false }
    fn description(&self) -> String;
}
```
One impl per command → adding an undoable op becomes a **one-file plug-in**
instead of editing four matches. Codex must solve the real problems here:
- **Serde on trait objects** — the enum is `Serialize/Deserialize` and persists
  into `.photon` files; a `Box<dyn Command>` needs tagged (de)serialization
  (e.g. `typetag`, or a registry keyed by a variant tag) **without changing the
  on-disk format** (format compatibility is a hard constraint — see
  `docs/format-versions.md`).
- **`Batch(Vec<Command>)`** — recursive; the trait must compose.
- Migrate **2 exemplar variants** (one simple: `UpdateNode`; one structural:
  `GroupNodes`) through the trait in this PR as the template.

**Why Codex:** trait design + serde-on-trait-objects + preserving the persisted
format is squarely rubric C1. This is the module's only hard piece.

---

## WP-4C — Migrate remaining 17 variants behind the trait  ·  **Hermes (Qwen 3.7) ×4**  ·  needs 4B
Following the 4B exemplars, one agent per cluster:
- `commands/node.rs` — AddNode, RemoveNode, UpdateNode, RemoveNodeFull.
- `commands/layer.rs` — AddLayer, RemoveLayer, ReorderLayers, SetActiveLayer,
  UpdateLayer, MoveNodeToLayer.
- `commands/structure.rs` — GroupNodes, UngroupNodes, SetGuides, SetArtboards,
  SetWidthProfiles, ResizeCanvas.
- `commands/batch.rs` — the recursive `Batch` composition.

Each variant's `apply`/`inverse`/`coalesce`/`description` bodies already exist in
the matches; migrating them into per-command `impl Command` blocks is mechanical
once the trait + serde plumbing (4B) exists → Hermes.

**Critical acceptance for the whole module:** a **round-trip test per variant**
(apply → inverse restores the document) **and** a `.photon` save/load test
proving the persisted history format is unchanged. These must be green before
4C fans out.

## Summary
| WP | Tier | Model | Deps |
|---|---|---|---|
| 4A CommandHistory methods | Hermes | Qwen 3.7 | — |
| 4B Command trait + serde + 2 exemplars | Codex | — | (after 4A optional) |
| 4C migrate 17 variants | Hermes | Qwen 3.7 ×4 | 4B |

End state: `history.rs` → thin core + `history/{stacks,checkpoints,branches,
coalescing}.rs` + `commands/*.rs`, with undo commands as serde-safe plug-ins.
This is the recommended **reference implementation** for the "true plug-in"
success criterion (§9 of the master plan) — smaller and lower-risk than the MCP
macro.
