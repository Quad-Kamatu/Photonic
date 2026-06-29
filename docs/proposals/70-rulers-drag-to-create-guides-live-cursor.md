# Rulers: Drag-to-Create Guides + Live Cursor Measurement (#70) — Design Proposal

> Status: implemented (MVP). See "What this PR implements" / "Remaining work" below.

## What this PR implements

Interactive rulers, wired to the existing `Guide` / `Command::SetGuides` data model.

- **Drag-to-create guides** — dragging out of the top ruler strip creates a
  horizontal guide at the release Y; dragging out of the left strip creates a
  vertical guide at the release X. The drag is gated strictly to the 18px ruler
  rects (`ui.interact` on dedicated sub-rects), so it never competes with
  canvas pan/select. A live position label (in the current unit) floats next to
  the pointer, and a preview line previews the guide across the canvas. Releasing
  back over the ruler cancels (no guide created).
- **Guide editing** — each non-locked, non-angled guide gets a ~4px grab zone
  across the canvas. Dragging it moves the guide live; releasing commits one
  `Command::SetGuides { old, new }`. Dragging a guide back onto its ruler strip
  deletes it. Double-clicking a guide opens a small `egui::Window` with a
  `DragValue` (in the current unit) to set the exact position. Every create /
  move / delete / exact-edit is a single undoable step.
- **Live cursor readout** — a marker line + numeric label is drawn on each ruler
  at the pointer's canvas X (top ruler) and Y (left ruler), in the current unit.
- **Document units** — new `photonic_core::units` module with
  `DocumentUnit { Px, Mm, In, Pt }` and `to_px` / `from_px(value, unit, dpi)`
  helpers (round-trip unit-tested at 72/96/150/300 dpi). `AppPreferences`
  gains `document_units` (default `Px`, `#[serde(default)]`, persisted). A
  click-to-cycle unit selector lives in the ruler corner box. All ruler tick
  labels (horizontal and vertical) and the cursor readout honor the unit at a
  fixed 96 dpi.

**Files changed / created**

- `crates/photonic-core/src/units.rs` — **new**: `DocumentUnit` + conversions + tests
- `crates/photonic-core/src/lib.rs` — register `units` module + re-exports
- `crates/photonic-gui/src/preferences.rs` — add `document_units` field + default
- `crates/photonic-gui/src/app/rulers.rs` — **new**: `GuideEditPopup`,
  `handle_ruler_interaction`, `format_ruler_value`, drag-label + popup helpers
- `crates/photonic-gui/src/app/mod.rs` — declare `mod rulers`; new state fields
  (`ruler_drag`, `ruler_drag_pos`, `guide_dragging`, `guide_drag_old`,
  `guide_edit_popup`) + `Default` init; unit-aware ruler tick labels (added
  vertical labels); call into `handle_ruler_interaction` from the canvas pass

## Remaining work

- **Selection extent markers** on the rulers (min/max of the selection bbox) —
  listed in the original acceptance criteria but deferred; not part of the core
  drag/measure MVP.
- **Document-level DPI** — conversions use a fixed 96 dpi constant
  (`rulers::RULER_DPI`). A per-document `dpi` field (and a Document Settings UI)
  would make mm/in/pt physically accurate for print/export; deferred.
- **Bleed / safe-area margin guides, guide locking/layers, diagonal guides** —
  explicitly out of scope (M3).
- Numeric position fields elsewhere in the app (Properties panel, etc.) still
  display pixels; only ruler labels + the readout + the guide popup respect the
  unit so far.

---

## Summary

Rulers render today (`app.rs:1672–1740`) as static tick strips with no interactivity. The `Guide` / `GuideOrientation` types exist in `photonic-core` (`document.rs:15–50`) and `Command::SetGuides` provides undo support (`history.rs`). This issue wires them together: drag from a ruler to create a guide, double-click a guide to set its exact position, drag back to delete, and add a live cursor/selection position readout on both rulers. Document units (px/mm/in/pt) apply everywhere.

## Scope

**In**
- Drag from horizontal ruler → create a horizontal `Guide`; drag from vertical ruler → vertical `Guide`
- Live guide position label during drag
- Double-click an existing guide → inline position input (small egui popup)
- Drag a guide back over the ruler strip to delete it
- Cursor tick on each ruler tracks `pointer.hover_pos()` in canvas coordinates
- Selection extent markers on the ruler (min/max extent of the current selection bbox)
- Document units: `px | mm | in | pt`; a unit selector in the View menu or ruler corner; all numeric fields and ruler labels respect the chosen unit
- `AppPreferences::document_units` field; unit conversion helpers

**Out**
- Bleed / safe-area margin guides (M3)
- Guide locking or guide layers
- Diagonal guides

## Proposed Approach

1. **Units infrastructure** — new `crates/photonic-core/src/units.rs` (or add to `document.rs`):
   ```rust
   pub enum DocumentUnit { Px, Mm, In, Pt }
   pub fn to_px(v: f64, unit: DocumentUnit, dpi: f64) -> f64 { ... }
   pub fn from_px(v: f64, unit: DocumentUnit, dpi: f64) -> f64 { ... }
   ```
   Add `pub unit: DocumentUnit` and `pub dpi: f64` to `Document` (default px, 96 dpi). Add `pub document_units: DocumentUnit` to `AppPreferences`.

2. **Ruler drag detection** (`app.rs` ruler paint block, ~line 1672`): the ruler strips are already painted as colored rects. Add `egui::Sense::drag()` to both ruler rects. On `drag_started` inside the horizontal ruler rect, set `ruler_drag_active: Some(GuideOrientation::Horizontal)` and record the starting canvas y. On `dragged`, show a floating label with the current canvas position (converted to document units). On `drag_released`, if the pointer is inside the canvas area, push `Command::SetGuides { old: doc.guides.clone(), new: updated_guides }` to history.

3. **Guide rendering** (`app.rs` canvas paint pass): for each `Guide` in `doc.guides`, draw a full-width (H) or full-height (V) dashed line across the canvas in guide color. Add a small grab handle diamond at the ruler-edge end for drag/delete.

4. **Guide interaction**: on `pointer_button_primary_pressed` over a guide's grab handle, begin `guide_dragging: Some(guide_index)`. On `dragged`, move the guide live (mutate in-place; no undo until release). On `drag_released`, commit `Command::SetGuides`. If the pointer is inside the ruler strip on release, delete the guide instead.

5. **Double-click to edit**: on `pointer_button_double_clicked` over a guide, open a small `egui::Window` with a `DragValue` showing the guide position in document units. On confirm, push `Command::SetGuides`.

6. **Cursor readout on rulers** (`app.rs` ruler ticks block): after rendering ticks, draw a colored line at the cursor's canvas-space x (vertical ruler) and y (horizontal ruler), converted to document units, with a small text label.

7. **Selection extent markers**: if `doc.selection` is non-empty and selection bounding box is available, render two additional tick marks (and a gap highlight between them) on each ruler corresponding to the selection's min/max extents.

8. **Unit selector**: add a small egui `ComboBox` in the ruler corner square (the 18×18 intersection box at top-left of the canvas, currently unpainted). Changing the unit updates `self.prefs.document_units` and re-renders all labels.

## Affected Modules

- `crates/photonic-core/src/document.rs` — `Document`: add `unit: DocumentUnit`, `dpi: f64`; `Guide` / `GuideOrientation` already exist
- `crates/photonic-core/src/history.rs` — `Command::SetGuides` already exists
- `crates/photonic-gui/src/preferences.rs` — `AppPreferences`: add `document_units: DocumentUnit`
- `crates/photonic-gui/src/app.rs` — `App`: add `ruler_drag_active`, `guide_dragging`, `guide_edit_popup` state; extend ruler paint block; add guide hit-test + drag; unit selector in ruler corner
- `crates/photonic-core/src/` — new `units.rs` (or extend `document.rs`) for `DocumentUnit` + conversion helpers

## Risks & Open Questions

- **Ruler strips are currently painted with no `Sense`** — adding drag sense to them must not interfere with canvas pan (which also uses primary drag). Gate ruler drag on `response.rect.contains(pointer)` where `response` is the ruler rect specifically, not the full canvas response.
- **Guide hit-test tolerance**: guides are 1px thin lines; hit-testing needs a ~4px grab zone. In screen space the conversion is straightforward.
- **DPI**: document DPI affects mm/in/pt conversion. Default 96 dpi for screen; expose in Document metadata or a Document Settings panel. Must be considered for export accuracy.
- **Ruler label density**: at low zoom, labels will overlap. Use adaptive step logic (same pattern as the existing ruler tick step calc in `app.rs:1704–1740`) to skip labels when they are too close.

## Acceptance Criteria

- [x] Dragging from the horizontal ruler creates a horizontal guide; vertical ruler creates a vertical guide
- [x] A live position label shows during the drag in the current document unit
- [x] Dragging a guide back into the ruler deletes it
- [x] Double-clicking a guide opens an exact-position input; confirming commits it as one undo step
- [x] Cursor position is tracked live on both rulers with a tick mark and label
- [ ] Selection extent is shown as markers on the rulers when nodes are selected (deferred — see Remaining work)
- [x] Unit selector (px/mm/in/pt) changes all ruler labels and numeric fields consistently (ruler labels + readout + guide popup; other panels deferred)

## Effort Estimate

**M** — the data model and undo infrastructure already exist; the work is plumbing the drag interactions, guide rendering, and unit system.
