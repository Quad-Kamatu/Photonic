# Drawer-Based UI Redesign — "Balanced" Model (#154)

> Status: **proposed**. Forward-looking spec to be built in phases. The recent
> Navigator extraction (`crates/photonic-gui/src/panels/navigator.rs`) was,
> in effect, Phase 1 step 0.

## Problem

The left **Properties panel** (`panels::draw_properties_panel`,
`crates/photonic-gui/src/panels/mod.rs`) packs **~55 `CollapsingHeader`
sections** — Transform, Fill, Stroke, Effects, Pathfinder, Recolor, Grammar,
Actions, Edit History, Branches, Swatches, Symbols, Variables, … — into a single
scrolling column, most gated by active tool / selection and reached through a
property search box. Every control is present, but discovery is poor and the
panel reads as cluttered.

The goal is to strike a balance between **fine-grained control** (pro-tool depth)
and **clarity** (Canva-like approachability) — *without sacrificing either*.

## Core insight

Clarity here is not won by **removing** controls; it is won by **grouping** them
and **disclosing progressively**. The panel feels cluttered because all 55
sections live in one undifferentiated scroll — not because 55 controls is too
many. The fix is taxonomy + addressability.

## One taxonomy, three surfaces

A single grouping of the controls feeds three coordinated surfaces, split by
**interaction speed** (not by feature):

| Surface | Role | Latency | Draws from |
|---|---|---|---|
| **Contextual top bar** | The 3–5 hottest *properties* for the current selection, always visible | zero clicks | top of **Inspector** |
| **Radial wheel** (right-click, at cursor) | Fast *verbs* done constantly — Group, Duplicate, Boolean, Align, z-order, Clip, Delete | one gesture, no travel | quick actions from **Modify / Arrange** |
| **Left-rail drawers** | Full depth — every parameter and library | open + dwell | all 8 groups |

Principle: **verbs at the cursor, properties in the bar, depth in the drawers.**

This is how both goals are met at once: a power user rarely opens a drawer —
they right-click for verbs and use the top bar for properties, staying on the
canvas. Drawers exist for *dwelling* (browsing swatches, managing branches), not
for routine edits.

## The 8 groups (mapping today's ~55 sections)

| Drawer | Sections folded in (current `panels/mod.rs` headers) |
|---|---|
| **Inspector** | Transform, Fill, Stroke, Color Guide, Effects (inner/outer/Gaussian glow), Opacity/Blend, Snap to Pixel, Path/Geometry, Origin (Prompt History), Asset Export, Symbol Override |
| **Modify** | Path Operations, Boolean Operations, Pathfinder, Blend, Blend Colors, Adjust Colors, Recolor, Flatten Transparency, Copy Appearance, Compound Path, Clipping Mask |
| **Arrange** | Alignment, Distribute on Path, Distribute No Overlap, Align to Artboard, Flex Layout, Distances, Dimension Annotations, Construction Lines |
| **Text** | Character Styles, Paragraph Styles, Type on Path, Area Type, OpenType Features, Text Frame Threading |
| **Assets** | Color Swatches, Spot Colors, Gradient Swatches, Graphic Styles, Width Profiles, Symbols (+ Load Library, Sprayer), Patterns (future, #20), Variables |
| **Document** | Print Settings, Artboard Margins, Export Profiles, Workspaces, Event Triggers, Document Grammar, Composition Analysis, Data Visualization, Actions |
| **History** | Edit History + Checkpoints (changelog) + Branches — the home for the new `.photon` persisted history |
| **(docked) Layers / Navigator / AI Chat** | Right-column reference surfaces, kept always-visible |

The grouping is the load-bearing artifact: it is the single source of truth for
all three surfaces, so they never drift apart (a "Modify" verb in the wheel and
the Modify drawer's full controls stay consistent).

## Layout — Balanced model

```
┌───────────────────────────────────────────────────────┐
│ File Edit Tools  [Fill▾ Stroke▾ Opacity]          100% │  ← contextual top bar
├──┬─────────────────────────────────┬──────────────────┤
│⬚ │                                 │ Layers           │
│✏ │                                 │  ▸ Logo          │
│T │            CANVAS               ├──────────────────┤
│◇ │                                 │ Navigator  ▦     │
│▤ │  ◀ Inspector (one drawer open,  ├──────────────────┤
│⚙ │     pinnable to dock)           │ AI Chat   > …    │
│⟲ │                                 │                  │
└──┴─────────────────────────────────┴──────────────────┘
 left rail: tools + 8 drawer icons (Inspector / Modify /
 Arrange / Text / Assets / Document / History) — click to
 open one at a time; pin to keep docked
 right column: always-docked reference (Layers / Nav / AI)
```

- **Left rail**: existing tool buttons, then 8 drawer icons. Clicking opens one
  drawer at a time (clarity); a pin toggle keeps it docked so power users can
  widen back toward an Illustrator-style multi-panel density (control).
- **Contextual top bar**: the hottest properties for the current selection (Fill,
  Stroke, Opacity; Font/Size for text; corner radius for rounded rects). The
  common case needs no drawer at all.
- **Right column** (kept, already mostly built): Layers, Navigator, AI Chat —
  "reference while you work," spatially natural on the right.

## Radial-wheel integration

The right-click **radial wheel** (`crate::radial_wheel`, `WheelAction`) becomes a
first-class surface, not a separate menu. It is the **fast path to Modify/Arrange
verbs**, context-sensitive to the selection (1 vs 2+ nodes, path vs text vs
group). When a verb needs parameters (Pathfinder offset, blend steps, recolor
palette), the wheel action **opens the relevant drawer pre-focused** — the
handoff between the gestural and the dwell surfaces.

The wheel earns its own redesign later, but it **must share this taxonomy** so the
two systems stay consistent. Decisions that the wheel redesign and this redesign
share (and should be locked together):
- The verb set per selection-context (derived from Modify/Arrange).
- The wheel→drawer handoff contract (which verbs open which drawer pre-focused).
- Visual language (icons/labels) shared between rail, wheel, and top bar.

## Motion & animation — clarity, not decoration

**Hard requirement, not polish-later.** Every drawer/rail/top-bar/wheel
transition must be **snappy and purposeful** — motion exists to make spatial
relationships legible (where a surface came from, what just changed), never to
entertain or to delay the result of an action. A laggy or showy UI fails this
redesign even if the layout is right.

Rules:
- **Snappy duration.** ~**120–180 ms** for drawer slide/fade and top-bar
  swaps; long enough to read direction, short enough to feel instant. One shared
  duration token across the whole UI for coherence.
- **Ease-out only.** Entrances decelerate and settle. **No bounce, overshoot, or
  spring wobble** — that reads as distraction.
- **Animate what aids understanding, nothing else.** Yes: drawer **slides from
  its rail icon** (shows origin), top-bar content **cross-fades** on selection
  change, the radial wheel **expands from the cursor**, pin **dock/undock**. No:
  per-frame value scrubs, hover noise, anything decorative.
- **Never block on motion.** The action is instant; the animation is cosmetic and
  runs *after* the state change. A click's result is never gated on a tween
  finishing. Input stays live mid-animation.
- **60 fps budget.** Drive via egui's built-in animation helpers
  (`Context::animate_bool_with_time` / `animate_value_with_time`) with
  `request_repaint` while animating; no per-frame allocation or jank.
- **Reduced-motion respected.** A Behavior pref shortens/disables transitions;
  functionality must be identical with animations off.

Each phase below ships its transitions to this standard — animation is part of
"done," not a follow-up.

## State & persistence

- Drawer state — which is open, pinned set, per-drawer width — persists in
  `AppPreferences` (`crates/photonic-gui/src/preferences.rs`), reusing the
  cross-platform prefs path and save-on-close hooks recently added.
- Workspaces (the existing Workspaces section) can capture/restore a whole
  drawer-and-pin arrangement, generalizing today's `prop_search`-only workspace.
- The current `DrawerKind` + `draw_two_column_menu` infrastructure
  (`app/mod.rs`) powers the top-anchored **File/Edit/Tools** app menus; those are
  distinct from these working-surface drawers and are kept as-is (a top menubar).

## Phased plan

Incremental, each phase shippable and low-risk; avoids a big-bang rewrite of the
11k-line `panels/mod.rs`.

1. **Taxonomy refactor — no visual change.** Split `draw_properties_panel` into 8
   `draw_*_group` functions matching the table above. Pure plumbing; unlocks
   everything. (Navigator extraction was step 0 of this.)
2. **Rail + drawer host, piloted on History.** Introduce the left icon rail and a
   single-drawer host; migrate the History group first (small, self-contained,
   and the home of the recent persistence work). Prove the open/pin/persist
   pattern end-to-end.
3. **Migrate groups into drawers**, one at a time (Inspector, Modify, Arrange,
   Text, Assets, Document), retiring sections from the monolithic panel as they
   move.
4. **Contextual top bar.** Add the always-visible hot-property strip, sourced
   from the Inspector group.
5. **Re-derive the radial wheel** from the taxonomy and wire the wheel→drawer
   pre-focus handoff. (Tracked as the wheel's own redesign; this phase is the
   integration seam.)

Every phase that introduces or moves a surface must ship its transitions to the
**Motion & animation** standard above — snappy, ease-out, clarity-serving — as
part of that phase's definition of done.

## Risks / open questions

- **Top-bar real estate** on narrow windows — needs an overflow/responsive rule.
- **Pin density** — how many drawers may be pinned at once before the canvas is
  starved; default cap + remembered layout.
- **Discoverability of the wheel** — verbs that only live in the wheel must also
  be reachable from a drawer (no wheel-only actions) so nothing is hidden.
- **Migration UX** — existing users know the search-box panel; consider a one-time
  "things moved" hint and keep global search (`Ctrl/Cmd+K`) able to jump straight
  to any control's new drawer.

## Relationship to other work

- **History group** consumes the new `.photon` persisted history (undo timeline +
  checkpoints + branches).
- **Assets group** is where Pattern fills (#20) will surface once landed.
- Builds directly on the recent prefs/persistence and Navigator-extraction work.
