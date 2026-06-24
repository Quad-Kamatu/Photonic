---
name: photonic-plan
description: Use when asked to create a NEW vector illustration in Photonic from scratch — before calling any MCP tool. Produces a composition brief covering subject decomposition, layer stack, color palette, and grouping strategy. DO NOT use for improvement/edit tasks.
---

# Photonic Plan

## Scope

**This skill is for creating a NEW illustration from scratch only.**

If the user asked you to *improve*, *update*, *refine*, or *edit* an existing design — **do not invoke this skill**. Instead, call `get_document_state` and `screenshot` in parallel to understand what exists, then make targeted changes using `update_node`, `reorder_node`, `create_shape`/`create_path`, and `delete_nodes` as needed.

---

## Overview

Before touching any Photonic MCP tool on a blank canvas, produce a structured composition brief. This prevents color drift, ensures back-to-front draw order, and commits to grouping strategy upfront. The brief is a short document you write in plain text, then reference throughout the drawing session.

**Core principle:** No shapes before a palette. No drawing before a layer stack. No components before a grouping plan.

---

## The Composition Brief

Produce all five sections before the first `create_shape` or `create_path` call.

---

### Section 1 — Subject Decomposition

Break the subject into geometric primitives. For each major body part, identify:
- **Shape type:** ellipse, rectangle, polygon, or custom path
- **Construction strategy:** single primitive / overlapping primitives / boolean union

Examples for a bird:
```
head        → ellipse (large)
body        → ellipse (larger, overlaps head at neck)
head+body   → boolean union after both drawn → one clean silhouette
beak        → polygon (triangle) or two quads
eye         → 4-layer stack: sclera ellipse, iris circle, pupil circle, highlight dot
crest       → 3 teardrop paths grouped
wing        → large ellipse base + long thin ellipses for stripes
tail        → 3-5 thin ellipses or paths, fanned out
legs        → 2 thin rectangles; toes = short thin rectangles branching at base
```

Flag any shapes that should be boolean-unioned:
> **Union candidates:** head + body → after both are drawn, union into `body_silhouette`

---

### Section 2 — Layer Stack

Define 4–6 named layers, ordered back-to-front. Create these layers first via `create_layer`.

Standard stack for character illustrations:

| Layer (back → front) | Contents |
|---|---|
| `background` | Sky, ground, backdrop shapes |
| `base` | Main body silhouettes, large filled areas |
| `overlay` | Wing stripes, body markings, feather details |
| `detail` | Eye anatomy, beak, small features |
| `highlight` | White dots, shine marks, accent shapes |
| `accent` | Outlines, shadow lines, darkest detail elements |

Adjust layer names to fit the subject. Fewer layers for simple subjects (3 is fine).

---

### Section 3 — Color Palette

**REQUIRED SUB-SKILL:** Use `photonic-design` for color harmony rules, the 60-30-10 distribution rule, value contrast requirements, and style consistency guidelines before finalizing any palette.

Commit to 5–8 named colors before drawing anything. Use hex values.

Format:
```
PALETTE
  body_blue:      #5BA4D4                              — main body and wing color (solid)
  body_dark:      #3A7DAF                              — shadow areas, darker wing bands (solid)
  accent_white:   #F0F0F0                              — sclera, belly, highlight dots (solid)
  near_black:     #2C2C2C                              — pupil, dark markings, outlines (solid)
  beak_gray:      #8A8A8A                              — beak and feet (solid)
  belly_cream:    #F5F0E8                              — chest/belly area (solid)
  sky_gradient:   linear [#87CEEB → #FFFFFF], top→bot  — background sky fade
  glow_radial:    radial [#FFFFFF → #5BA4D4], r=150     — eye iris glow
```

**Fill type syntax for palette entries:**
- Solid: `#hexcolor`
- Linear: `linear [#from → #to], direction`
- Radial: `radial [#inner → #outer], r=<radius>`
- Fluid: `fluid [#c1, #c2, #c3], power=2.0`
- Mesh: `mesh [#tl, #tr, #bl, #br], 2×2`

When converting a palette entry to an MCP `fill` argument:
```
solid:   {"type": "solid", "color": "#5BA4D4"}
linear:  {"type": "gradient", "gradient_type": "linear", "colors": ["#87CEEB", "#FFFFFF"], "coords": [cx, top_y, cx, bottom_y]}
radial:  {"type": "gradient", "gradient_type": "radial", "colors": ["#FFFFFF", "#5BA4D4"], "coords": [cx, cy, 150]}
fluid:   {"type": "fluid_gradient", "points": [{"x": x1, "y": y1, "color": "#c1"}, ...], "power": 2.0}
mesh:    {"type": "mesh_gradient", "rows": 2, "cols": 2, "vertices": [{"x": 0, "y": 0, "color": "#tl"}, ...]}
```

Rules:
- Do not introduce new colors mid-session. If a new color is needed, add it to the palette definition first.
- Use the named palette references (not raw hex) when describing shapes in subsequent tool calls — keeps fills consistent.
- Gradient coordinates are in document space — compute them from the shape's bounding box, not relative values.

---

### Section 4 — Grouping Plan

List the groups you will create, and which nodes belong in each.

```
GROUPS
  head_group:       head_base, head_mask, eye_sclera, eye_iris, eye_pupil, eye_highlight, beak_upper, beak_lower, crest_1, crest_2, crest_3
  body_group:       body_silhouette (post-union), belly_ellipse
  wing_group:       wing_base, wing_stripe_1, wing_stripe_2, wing_stripe_3, wing_dot_1..6
  tail_group:       tail_feather_1..5
  legs_group:       leg_left, leg_right, toe_left_1..3, toe_right_1..3
```

Rules:
- Group each component's nodes immediately after completing that component — do not defer grouping to the end of the session.
- Group names match the node naming convention: `{component}_group`.

---

### Section 5 — Canvas Notes

**REQUIRED SUB-SKILL:** Use `photonic-design` for composition placement rules (rule of thirds, visual weight, balance type) when deciding where to position the primary subject.

State the working canvas dimensions and background. If `set_canvas` is available, call it first. Otherwise, note dimensions for consistent coordinate decisions.

```
CANVAS
  width:      1024px
  height:     1024px
  background: #FFFFFF (white) or transparent
  origin:     top-left, Y increases downward
  units:      pixels
```

Note coordinate conventions so placement decisions are consistent throughout the session.

---

## Brief Output Template

```
=== COMPOSITION BRIEF ===

SUBJECT: [what we're drawing]

DECOMPOSITION
  [list each body part with shape type and construction strategy]
  [flag boolean union candidates]

LAYER STACK (back → front)
  background — [contents]
  base       — [contents]
  overlay    — [contents]
  detail     — [contents]
  highlight  — [contents]

PALETTE
  [name]: [hex]  — [note]
  ...

GROUPS
  [group_name]: [node list]
  ...

CANVAS
  [width × height, background, notes]

=== BEGIN DRAWING ===
```

---

## Common Mistakes

| Mistake | Fix |
|---|---|
| Starting with `create_shape` before writing the brief | The brief comes first. Always. No exceptions. |
| Introducing a new color mid-session without updating the palette | Stop, add the color to PALETTE, then use it |
| Planning only one layer | Complex subjects need at minimum base/detail/highlight separation |
| Forgetting to flag union candidates | Ask: does the seam between these shapes need to disappear? If yes, union. |
| Grouping at the end of the session | Group each component as you complete it — this is in the Grouping Plan for a reason |
| Writing a grouping plan but not following it | The plan is the contract. Follow it or update it explicitly. |
