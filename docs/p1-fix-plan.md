# P1 Bug Fix Plan

Root-cause investigation completed 2026-06-26. Line numbers below are **verified against the current source tree**. The open GitHub issues cite line numbers that are stale by approximately 21 lines — do not use the issue line numbers when navigating the code; use the ones in this document.

---

## Overview

| # | Title | Crate(s) | Severity | Effort |
|---|-------|----------|----------|--------|
| [#11](#11--move-doesnt-record-history) | Move doesn't record history | photonic-gui | P1 — data loss | S |
| [#5](#5--resize-doesnt-record-history) | Resize doesn't record history | photonic-gui | P1 — data loss | S |
| [#3](#3--hit-test-uses-bounding-box-not-geometry) | Hit-test uses bbox, not geometry | photonic-gui, photonic-core | P1 — correctness | M |
| [#2](#2--multi-select-flip--rotate-bugs) | Multi-select flip / rotate bugs | photonic-gui | P1 — correctness | M |
| [#8](#8--eyedropper-opens-os-picker--samples-wrong-pixel) | Eyedropper opens OS picker / wrong pixel | photonic-gui, photonic-mcp | P1 — UX breakage | L |

**Recommended fix sequence:** #11 → #5 (share a single commit, see [Shared Work](#shared-work-11--5)) → #3 → #2 → #8.

Rationale: #11 and #5 are the highest-impact data-loss bugs, share the same code block, and carry zero risk (no logic change, only adds the missing `history.execute` call). #3 is a self-contained correctness fix with no observable regressions. #2 requires a targeted caller fix plus handler extension. #8 is the largest change (removes an external dependency, replaces the sampling strategy) and benefits from #3 being in place first (shares the winding-number traversal).

---

## #11 — Move doesn't record history

**Root cause.** `photonic-gui/src/app.rs:9457–9468` — the move-drag loop mutates `node.transform.matrix[4]` and `matrix[5]` directly per frame. `*doc_modified = true` at line 9466 is not a history entry. The drag-release block at lines 9472–9473 sets `self.moving = false` and nothing else. There is no pre-move snapshot field; `point_drag_origin` (app.rs:348) belongs to the pen/point tool and is unrelated.

**Pattern to mirror.** Two existing patterns in `app.rs` do this correctly:

- Arrow-key nudge (~2000–2041): clones each selected node as `old`, builds a translated `new`, creates `Command::UpdateNode{old,new}` per node, collects into `Command::Batch`, calls `history.execute`.
- Bezier point-drag (~9900–9963): snapshots the full `SceneNode` at drag start, commits `Command::UpdateNode` on release. This is the exact shape needed for whole-node moves.

**Fix.**

1. Add field to `App` state:
   ```rust
   move_origins: Vec<(NodeId, SceneNode)>,
   ```
2. Populate when `self.moving` becomes `true` — there are two entry points, both in `app.rs`:
   - Line ~9318 (primary pointer down on a selected node)
   - Line ~9373 (pointer down starting a new selection + move)

   At each: `self.move_origins = doc.selection.ids().filter_map(|id| doc.nodes.get(id).map(|n| (id, n.clone()))).collect();`

3. On `drag_stopped` (app.rs:9472), before clearing `self.moving`:
   ```rust
   if !self.move_origins.is_empty() {
       let cmds: Vec<Command> = self.move_origins.drain(..)
           .filter_map(|(id, old)| {
               let new = doc.nodes.get(id)?.clone();
               if new.transform.matrix == old.transform.matrix { return None; }
               Some(Command::UpdateNode { old, new })
           })
           .collect();
       if !cmds.is_empty() {
           history.execute(Command::Batch(cmds), doc);
       }
   }
   self.moving = false;
   ```

Live per-frame mutation is unchanged — this only adds the commit on release.

**Edge cases.**

| Case | Handling |
|------|----------|
| Multi-select | Natural — all `selection.ids()` are snapshotted |
| Moving a group | Group node transform is the only thing that changes; `UpdateNode` on it is complete |
| Zero-distance click | `matrix == old.matrix` guard prevents a spurious history entry |
| `history.execute` re-applying `new` | Reads `doc.nodes[id].clone()` which already is `new`; idempotent no-op |

**Test strategy.**

- **Unit test (photonic-core/src/history.rs):** existing tests `update_node_undo_redo` (~139) and `batch_undo` (~363) already cover the `Command::UpdateNode` / `Command::Batch` round-trip. No new unit test needed for the history machinery itself.
- **GUI integration:** move a node, undo (Ctrl+Z), verify node returns to origin position. Repeat with multi-select. These are manual verification steps; the drag lifecycle is not unit-testable without the egui harness.

**Blast radius / risk.** Change is additive — two new field reads at drag start, one `history.execute` call at drag end. No existing code path is altered.

**Acceptance criteria.**
- Moving a node then pressing Ctrl+Z returns it to the pre-move position.
- Moving a multi-select group then pressing Ctrl+Z returns all nodes.
- Ctrl+Y re-applies the move.
- A click-without-drag does not produce a history entry.

---

## #5 — Resize doesn't record history

**Root cause.** Direct transform mutation during the resize-drag loop with no history commit on release:

- Multi-node resize: `app.rs:9419–9422` — `node.transform = Transform{matrix: orig_xf}.then(&t_scale)`
- Single non-text: `app.rs:9449` — `node.transform = t_orig.then(&t_scale)`
- Single text: `app.rs:9433` and `9442` — matrix translate + `text.font_size = (orig_fs * scale).max(1.0)`

Drag release at `app.rs:9472–9478` clears `self.resizing`, `resize_origin_*`, and `resize_multi_origins` — no `history.execute`.

**Key nuance.** Start snapshots already exist and are simply discarded:

| Field | Location | Content |
|-------|----------|---------|
| `resize_origin_transform: Option<[f64;6]>` | app.rs:331 | Set at drag start ~9295–9298; single-node pre-resize matrix |
| `resize_origin_font_size: Option<f64>` | app.rs:333 | Set at drag start ~9299–9308; pre-resize font size |
| `resize_multi_origins: Vec<(NodeId,[f64;6])>` | app.rs:336 | Set at drag start ~9286–9289; all selected nodes' pre-resize matrices |

`resize_origin_bounds` is the geometry anchor for scale math only — it is not an undo snapshot and should not be used as one.

**Fix.** On `drag_stopped` (app.rs:9472), before clearing the origin fields:

```rust
// single node
if let (Some(orig_matrix), Some(id)) = (self.resize_origin_transform, self.resizing) {
    if let Some(node) = doc.nodes.get(id) {
        let mut old = node.clone();
        old.transform.matrix = orig_matrix;
        if let (SceneNodeKind::Text(ref mut t), Some(orig_fs)) =
            (&mut old.kind, self.resize_origin_font_size)
        {
            t.font_size = orig_fs;
        }
        let new = node.clone();
        if new.transform.matrix != orig_matrix {
            history.execute(Command::UpdateNode { old, new }, doc);
        }
    }
}

// multi-node
if !self.resize_multi_origins.is_empty() {
    let cmds: Vec<Command> = self.resize_multi_origins.iter()
        .filter_map(|(id, orig_matrix)| {
            let new = doc.nodes.get(*id)?.clone();
            if new.transform.matrix == *orig_matrix { return None; }
            let mut old = new.clone();
            old.transform.matrix = *orig_matrix;
            Some(Command::UpdateNode { old, new })
        })
        .collect();
    if !cmds.is_empty() {
        history.execute(Command::Batch(cmds), doc);
    }
}
```

**Edge cases.**

| Case | Handling |
|------|----------|
| Single text node | Restore both `transform.matrix` and `font_size` from origin fields |
| Multi-select text | Multi path uses matrix scaling only; matrix-only undo is correct (font_size not mutated in multi path) |
| Groups | Same path as other nodes; transform matrix fully captures the resize |
| Click without drag | `matrix == orig_matrix` epsilon guard prevents spurious entries |

**Test strategy.**

- **Unit test (photonic-core):** `Command::UpdateNode` round-trip is already proven by `update_node_undo_redo`. The reconstruction logic above can be tested with a trivial `#[test]` in `history.rs` that exercises `Command::Batch` with two `UpdateNode` commands.
- **GUI (manual):** resize a path, undo → verify original size; resize a text node, undo → verify font size and bounds restore; resize multi-select, undo → all nodes restore.

**Blast radius / risk.** Additive change in the same `drag_stopped` block. Origin fields are already computed and populated; this change only adds the `history.execute` call before they are cleared.

**Acceptance criteria.**
- Resizing a node then Ctrl+Z restores original size and position.
- Resizing a text node restores font size and bounding box.
- Resizing a multi-select restores all nodes.
- A click-without-drag on a handle does not produce a history entry.

---

## Shared Work — #11 + #5

Both bugs live in the same `drag_stopped` block (`app.rs:9472+`) and follow an identical pattern: **snapshot at drag start → live mutation → no commit on stop**. They should be fixed in a single commit with a single PR.

Suggested structure for that commit:

1. Add `move_origins` field to `App` (for #11).
2. Populate `move_origins` at the two move-start sites.
3. In the single `drag_stopped` block: emit move history (if `self.moving`), emit resize history (if `self.resizing.is_some()` or `!resize_multi_origins.is_empty()`), then clear all origin fields and flags as before.

No new function needed — the two paths are short enough to read inline.

---

## #3 — Hit-test uses bounding box, not geometry

**Root cause.** `fn hit_test` at `app.rs:12480–12492` performs pure AABB containment via `text_aware_canvas_bounds` (`app.rs:10869–10891`). The doc comment says "whose bounding box contains" — this is the intended behavior, but it produces false positives for concave and open paths (e.g. a donut, a star, an L-shape).

**Capability already in tree.** No new dependencies required:

| Existing piece | Location |
|----------------|----------|
| `kurbo 0.11.3` with `BezPath::winding(pt)` | `Cargo.toml` (already dep of photonic-core) |
| `PathData::to_bez_path()` | `photonic-core/src/path.rs` |
| `PathNode::is_compound` (fill rule flag) | `node.rs:252`; used by renderer at `renderer.rs:883` |
| `Transform::to_kurbo() -> kurbo::Affine` | already exists |
| `kurbo::Affine::inverse()` | kurbo stdlib |

**Fix.**

Keep bbox as a cheap pre-filter. On bbox hit, map the cursor into node-local space and apply winding:

```rust
// In hit_test, after bbox pre-filter passes:
if let SceneNodeKind::Path(ref p) = node.kind {
    let bez = p.path_data.to_bez_path();
    let affine = node.transform.to_kurbo();
    // Guard: skip winding test if transform is singular
    if affine.determinant().abs() < 1e-10 { /* keep bbox result */ } else {
        let local = affine.inverse() * kurbo::Point::new(cx, cy);
        let inside = if p.is_compound {
            bez.winding(local) % 2 != 0   // even-odd
        } else {
            bez.winding(local) != 0        // non-zero
        };
        if !inside { return None; }
    }
}
// Text: keep bbox (no PathData)
// Group: dead arm — nodes_in_draw_order flattens to leaves (document.rs:808–813)
```

**Edge cases.**

| Case | Handling |
|------|----------|
| Stroke-only / open paths | `winding == 0` for all interior points → node becomes unclickable. Short-term: fall back to bbox when `fill` is `None`/`Transparent`. Long-term: stroke-proximity test (out of scope for this fix). |
| Singular transform (zero-scale node) | Guard on `determinant < 1e-10`; fall back to bbox result. |
| Group nodes | Handled by document.rs leaf-flattening before `hit_test` is called; no winding needed here. |
| Group child transform composition | Leaf transforms already compose with group transforms before being passed to `hit_test`. Flag as a risk to verify manually. |

**Callers.** 11 call sites in `app.rs` (lines 1877, 2388, 2641, 9214, 9322, 9525, 9905, 9989, 10093, 10112, plus the definition). All consume `Option<NodeId>`; the function contract is unchanged — only correctness improves.

**Test strategy.**

- **Unit test (photonic-core or photonic-gui tests):** construct a `PathData` for a concave polygon (e.g. star or L-shape), assert that a point in the concavity returns `None` from `hit_test` and a point inside the solid region returns `Some`. This is fully testable without egui.
- **Manual:** click inside the "hole" of a compound path (donut) — should not select. Click the solid ring — should select.

**Blast radius / risk.** Low. The function contract is unchanged. The pre-filter bbox check ensures no performance regression for non-intersecting nodes. The only behavioral change is that some previously selectable regions become non-selectable (the correct outcome).

**Acceptance criteria.**
- Clicking inside the visual hole of a donut/compound path does not select it.
- Clicking the solid stroke area of a star or L-shape selects it.
- Clicking empty canvas area in the bbox of a concave path does not select it.
- Stroke-only open paths remain clickable (via bbox fallback).

---

## #2 — Multi-select flip / rotate bugs

**Corrected premise.** The issue originally described "N separate bounding boxes" for multi-select. The overlay already draws a single unified bbox with handles (`app.rs:9565–9583`) alongside intentional per-node outlines (`app.rs:9549–9563`). This is Figma/Affinity style behavior, not a bug. `selection_canvas_bounds` at `app.rs:10895–10919` already computes the correct union. The per-node outline question is a UX/design call outside this plan.

**Confirmed bugs.**

**Bug A — Flip/Rotate only affects primary node.**
- File: `photonic-gui/src/panels/mod.rs:1970–1983`
- The Flip button handler constructs `PanelAction::FlipNodes { node_ids: vec![nid], ... }` using only the primary `nid`. The enclosing guard is `if let (Some(nid), SceneNodeKind::Path) = (selected_id, &node.kind)` — so the button is hidden entirely for non-Path primaries.
- `selected_ids: &[NodeId]` is already in scope (passed at `app.rs:1412`).
- Panel Rotate (`panels/mod.rs:~1346`) has the same caller-only-primary bug.
- The `FlipNodes` handler at `app.rs:5304–5352` correctly iterates `node_ids` — the bug is entirely in the caller.

**Bug B — Flip pivots about each node's own center, not the selection center.**
- `app.rs:5315–5316`: each node is flipped about its own bbox center.
- `app.rs:9102–9169` (keyboard flip): same per-own-center issue.
- `align` (`app.rs:4864–4870`) already computes a union bbox correctly — mirror that pattern.

**Bug C — Flip handler silently skips non-Path nodes.**
- `app.rs:5304`: handler matches only `SceneNodeKind::Path`; Group and Text nodes are silently ignored.
- Fix: handle flip via transform matrix for all kinds (a horizontal flip is `matrix[0] = -matrix[0]`; vertical is `matrix[3] = -matrix[3]`, with appropriate translate to preserve the pivot).

**Fix sequence.**

1. **Caller fix** (`panels/mod.rs:1971,1981`): replace `vec![nid]` with `selected_ids.to_vec()`. Remove the `SceneNodeKind::Path` guard from the button-visibility check so the Flip button appears for mixed selections. Same fix for Rotate.
2. **Handler pivot fix** (`app.rs:5315–5316`): compute the union bbox of all `node_ids` using the same approach as `selection_canvas_bounds`; use its center as the pivot for all flips.
3. **Handler kind fix** (`app.rs:5304`): replace the `SceneNodeKind::Path`-only match arm with a general transform-matrix flip applied to all node kinds.

Note: `selected_id: Option<NodeId>` (~50 valid single-primary uses for the Properties panel) requires no refactor.

**Test strategy.**

- **Unit test (photonic-core):** `Command::Batch` + `Command::UpdateNode` round-trip for a flip operation is testable without egui. Construct two nodes at known positions, apply the flip transform to both about the shared center, verify matrix values, apply inverse, verify originals.
- **Manual:** select two Path nodes, flip horizontal → both flip about shared midpoint; select Path + Text, flip → both flip; undo → both restore.

**Blast radius / risk.** Caller change is two lines. Handler changes are localized to `app.rs:5304–5352`. No selection model changes. Keyboard flip path (`9102–9169`) needs the same pivot fix — do it in the same commit.

**Acceptance criteria.**
- Flip/Rotate with multiple nodes selected affects all selected nodes, not only the primary.
- All selected nodes flip about the shared selection bounding box center.
- Flip works for Path, Text, and Group nodes.
- Undo restores all flipped nodes.

---

## #8 — Eyedropper opens OS picker / samples wrong pixel

**Root cause — two issues.**

1. `capture_screen` at `app.rs:85–100` uses the `screenshots` crate (`Screen::from_point(...).capture()`). On KDE/Wayland this routes through libwayshot → XDG ScreenShot portal → Spectacle permission prompt. If dismissed, capture errors and the picker silently no-ops.
2. The snapshot is taken once at activation (`app.rs:4774–4779`), then sampled using `window_logical_pos` from winit `outer_position()` (`photonic-app/src/main.rs:374–378`), which is unreliable on Wayland. A scale mismatch between libwayshot's `display_info.scale` and winit's logical coordinates further corrupts the sampled pixel. Sampling loop: `app.rs:8701–8771`; coordinate math: `8726–8727`; `EyedropperCapture::sample_logical` formula: `app.rs:38–54`.

**Dependency.** `screenshots = "0.8"` in `crates/photonic-gui/Cargo.toml:25` — only one user in the repo (`use screenshots::Screen;` in `app.rs`). The crate is flagged `future-incompat` by the current Rust toolchain.

**Reusable path already exists.** `sample_color_at` in `photonic-mcp/src/handlers/nodes.rs:5613–5668` walks layers/nodes top-down, uses `bez.winding(pt) != 0` to find the topmost path under a canvas point, and reads `FillKind::Solid`. This is exactly the document-sampling logic the eyedropper needs.

**Fix.**

Replace the screen-grab approach with in-canvas document sampling:

1. Remove the `screenshots` dependency from `photonic-gui/Cargo.toml`. Remove `capture_screen`, `EyedropperCapture`, and the activation snapshot path (`app.rs:85–100`, `4774–4779`).
2. On each hover frame while the eyedropper is active, translate the egui cursor position to canvas coordinates via the live `CanvasView`/`PhotonicRenderer.view` (the same transform used by hit-test callers).
3. Walk `doc.layer_order` top-to-bottom; for each visible node in draw order, test with `bez.winding(local_pt) != 0`; take the fill color of the first match. (Mirror the logic in `nodes.rs:5613–5668`.)
4. Display the sampled color immediately in the picker UI — no activation snapshot, no release event needed.

**Scope note.** This fix covers "pick a color from the canvas document." True "pick anywhere on screen including outside the app window" (across other apps) would require the XDG ScreenShot portal with explicit user opt-in — that is a separate future feature. All current eyedropper call sites (fill/stroke/glow pickers) sample canvas content, so this covers all current uses.

**On X11.** The `screenshots` path used xcb and was reliable. After this fix X11 and Wayland both use the in-canvas path; the behavior is now identical on both. There is no regression.

**Optional future.** A cached `HeadlessRenderer` (`headless.rs:138–272`) could sample pixel-accurate gradient/overlay blends. Per-hover wgpu spin-up is too slow; defer until a persistent headless render thread design is in place.

**Test strategy.**

- **Unit test (photonic-core):** the winding-based node-under-point lookup is already tested by MCP handler tests; reuse or reference those. Write a test in photonic-core that places two overlapping paths with known colors and asserts the correct top-node color is returned at the overlap point.
- **Manual:** activate eyedropper, hover over a filled path — picker shows that path's fill color with no OS permission prompt; hover over canvas background — picker shows canvas background color; hover over a stroke-only path — falls back gracefully (transparent / no color).

**Blast radius / risk.** Medium. Removes `screenshots` + libwayshot + xcb from the dep tree (build time improves). The `EyedropperCapture` struct and the snapshot path in `app.rs:85–100` are deleted. The sampling loop in `app.rs:8701–8771` is replaced with a document-walk call. No other crate is affected. Risk: gradient/overlay fills will show the flat fill color, not the rendered pixel — acceptable for P1; document this as a known limitation.

**Acceptance criteria.**
- Activating the eyedropper does not open Spectacle or any OS permission prompt.
- Hovering over a solid-fill path updates the color picker to that fill color.
- Hovering over the canvas background returns the canvas background color.
- The sampled color is correct (not a scale-shifted neighbor pixel).
- `screenshots` does not appear in `cargo tree` output after the fix.

---

## Sequencing and Effort

| Order | Issue | Estimated effort | Dependency |
|-------|-------|-----------------|------------|
| 1 | #11 Move history | S — ~1 h | None |
| 2 | #5 Resize history | S — ~1 h | Same commit as #11 |
| 3 | #3 Hit-test geometry | M — ~3 h | None (but #8 benefits from this being in first) |
| 4 | #2 Flip/Rotate multi-select | M — ~3 h | None |
| 5 | #8 Eyedropper in-canvas | L — ~6 h | Benefits from #3's winding path |

Total estimated: ~14 h engineering + ~2 h manual verification across all five.

**Commit strategy.** One PR per issue except #11+#5 which share a single PR. All PRs are independent after #3 is merged (no shared files with #8 or #2 after the history fix is done).
