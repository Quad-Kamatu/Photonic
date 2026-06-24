---
name: photonic-shapes
description: Use when deciding how to construct a specific part of a vector illustration in Photonic — choosing shape types, when to boolean vs overlap, how to build organic forms, eyes, wings, legs, and complex outlines.
---

# Photonic Shapes

## Overview

A decision guide for translating visual forms into Photonic primitives and paths.

**Core principle:** Primitives for structural scaffolding. `create_path` whenever a shape has character — asymmetry, a pointed tip, a swept curve, an organic contour. Default toward paths, not away from them.

---

## The Primitive Bias — Resist It

**The failure mode:** Using an ellipse or polygon for every shape because it is the easiest tool call. The result looks like clip-art — blobs and triangles instead of a real illustration.

**The rule:** Use a primitive only if **all** of these are true:
1. The shape is geometrically regular (truly circular, truly rectangular, truly equilateral).
2. The shape has no meaningful asymmetry.
3. The shape has no pointed tip, tapered end, or swept curvature.
4. The shape would look correct if you scaled a circle or rectangle to fit.

If any condition fails → use `create_path`.

**Examples that fail the primitive test:**
- A bird wing → not an ellipse (one end swept back, trailing edge curves differently than leading)
- A tail feather → not an ellipse (tapers to a point at tip)
- A beak → not a triangle (the sides are curved, not straight, and tip tapers)
- A teardrop / raindrop → not an ellipse (pointed at one end)
- A leaf → not an ellipse (pointed at both ends with different curvature on each side)
- A flowing body contour → not an ellipse (one side is flat/muscular, the other convex)
- An ear → not an ellipse (base is flat, top is rounded, inner curve differs from outer)

---

## Tool Selection

| Form | Tool | Notes |
|---|---|---|
| True circle, true ellipse, round pupil/highlight | `create_shape` type=`ellipse` | Eyes, highlights, sun, pure circles only |
| Rectangle, straight leg, flat background | `create_shape` type=`rectangle` | Structural scaffolding, not final shapes |
| Regular polygon (equilateral) | `create_shape` type=`polygon` | Stars, hex tiles — not beaks or organic forms |
| Any organic outline with character | `create_path` with SVG cubic bezier data | Wings, tails, beaks, body contours, feathers |
| Any shape with explicit vertices | `build_shape_from_points` | When exact corner coordinates matter |

**Decision rule:**
```
Is the shape a pure geometric form (circle, square, regular polygon)?
  → Yes AND it requires no asymmetry or taper: use create_shape.
  → No, or it has any organic quality: use create_path with cubic bezier data.
  → Faceted/angular with explicit corners: use build_shape_from_points.
```

---

## SVG Path Recipes

These are the shapes Claude most often replaces with inadequate primitives. Use these as starting templates and scale to fit.

All coordinates assume a shape centered at origin (0,0) — translate after placement.

### Teardrop (pointing up — feather, raindrop, leaf tip)
```
M 0,-60 C 35,-60 60,-20 60,20 C 60,50 35,70 0,70 C -35,70 -60,50 -60,20 C -60,-20 -35,-60 0,-60 Z
```
Adjust: increase the top control points (±35 → ±50) to make it rounder; decrease to sharpen the tip.

### Pointed leaf (sharp at both ends)
```
M 0,-70 C 30,-40 50,0 50,20 C 50,50 30,65 0,70 C -30,65 -50,50 -50,20 C -50,0 -30,-40 0,-70 Z
```

### Bird wing (swept — wider at front, tapering to back)
```
M -80,0 C -60,-60 20,-80 80,-40 C 120,-20 130,10 100,30 C 70,50 20,60 -20,50 C -60,40 -100,20 -80,0 Z
```
This gives a swept wing: wide leading edge (left), tapered trailing tip (right). Mirror with `reflect_horizontal` for the other wing.

### Curved beak (upper mandible, pointing right)
```
M -30,-10 C -10,-30 30,-25 50,-5 C 60,5 50,20 30,15 C 10,10 -10,5 -30,-10 Z
```
Lower mandible: same but flipped vertically and slightly smaller.

### Organic body contour (bird/character body — not a blob ellipse)
```
M 0,-80 C 50,-75 90,-40 100,0 C 110,40 90,80 50,100 C 20,115 -20,115 -50,100 C -90,80 -110,40 -100,0 C -90,-40 -50,-75 0,-80 Z
```
This is asymmetric — right side (positive x) extends further than left, giving mass/direction. Adjust control points to shift weight left or right.

### Ear (animal/character — flat base, curved top)
```
M -30,0 C -30,-50 -10,-80 0,-80 C 10,-80 30,-50 30,0 Z
```
Position at the top of the head, rotate slightly outward.

### Flowing tail feather (one feather, tapers to point)
```
M 0,0 C 20,-30 30,-70 20,-120 C 15,-140 5,-150 0,-155 C -5,-150 -15,-140 -20,-120 C -30,-70 -20,-30 0,0 Z
```
Rotate each feather separately around the tail base pivot using `apply_transform`.

### Flame / spike / spine
```
M 0,0 C -20,-20 -15,-60 0,-80 C 15,-60 20,-20 0,0 Z
```

---

## Organic Forms (Characters, Animals, Birds)

### Bodies and Heads

Heads: true ellipses are acceptable for simple round heads. For any character with a chin, snout, or brow — use a path.

Bodies: **never use a plain ellipse for a body.** A body has weight distribution — it is wider at the shoulders or haunches, tapers differently top vs bottom. Use the organic body contour path recipe above, modified to shift mass to the correct region.

```
head  → ellipse only if perfectly round (owl, penguin chick); path for elongated/beaked heads
body  → create_path using body contour recipe; adjust control points for species
```

**When to boolean union vs overlap:**

| Scenario | Strategy |
|---|---|
| Head and body are the same color, seam should disappear | Draw both as paths, then `boolean_operation` union → one clean silhouette |
| Head and body are different colors, visible separation is fine | Overlap ellipses, z-order body behind head |
| Body has a belly of a different color | Overlay a smaller ellipse on the front, no boolean needed |

After union, the result is a single path node. Name it `body_silhouette` and reorder to the correct z-position.

### Belly / Chest Patches

Overlay a smaller, lighter ellipse on the body — no boolean. It should sit on top of the body silhouette in z-order.

```
body_silhouette (base color, e.g. #5BA4D4)
belly_patch     (lighter color, e.g. #F5F0E8) — smaller ellipse, centered-low on body
```

---

## Eye Anatomy (Standard 4-Layer Recipe)

Build eyes from four stacked shapes, back-to-front:

```
Layer 1 (back):  sclera     → white ellipse (e.g. 60×60)
Layer 2:         iris        → colored circle, slightly smaller than sclera (e.g. 44×44)
Layer 3:         pupil       → dark circle, centered within iris (e.g. 28×28)
Layer 4 (front): highlight   → tiny white ellipse, offset to upper-right of pupil (e.g. 10×8)
```

Group all four as `head_eye_group` (or `head_eye_left_group` / `head_eye_right_group` for paired eyes).

**Eye mask / shadow band:** If the subject has a dark band across the eye area (like a blue jay), add a dark-filled ellipse or path behind the eye layers and in front of the head base. Name it `head_eye_mask`.

---

## Beak Construction

**Do not use polygon triangles for beaks.** Real beaks have curved edges — a polygon produces a harsh geometric triangle that breaks the organic feel of the character.

Use `create_path` with the curved beak recipe from the SVG Path Recipes section:

```
beak_upper → create_path with curved beak SVG (see recipe), filled with beak color
beak_lower → create_path, same recipe but scaled ~70% and flipped vertically, lower mandible color
```

Adjust the path control points: for a hooked beak (raptor), pull the tip control point further; for a stout beak (finch), widen the mid-section; for a long thin beak (heron), stretch along x-axis.

Open beaks: two separate paths with a gap between them. Draw the gap as the background color showing through — no boolean needed.

---

## Wing and Feather Patterns

### Base Wing Shape

**Do not use a plain ellipse for a wing.** Wings are swept — wider at the base near the body, tapering toward the tip, with a different curvature on the leading edge vs trailing edge. An ellipse produces a symmetrical blob.

Use `create_path` with the bird wing recipe from the SVG Path Recipes section. Adjust the control points to match the species:
- Short rounded wings (robin, sparrow): pull the tip inward, widen the mid-section
- Long swept wings (swallow, swift): elongate along x-axis, sharpen the trailing tip
- Broad wings (hawk, owl): increase the height, flatten the top edge

### Wing Stripes

Long thin ellipses overlaid on the wing base, z-ordered above it:

```
wing_left_stripe_1 → ellipse (width=200, height=14), dark color
wing_left_stripe_2 → ellipse (width=180, height=14), offset 20px down from stripe_1
```

Group with wing base: `wing_left_group`

### Wing Dots / Spots

Small ellipses (12×10 or similar), placed manually along the stripe line:

```
wing_left_dot_1 through wing_left_dot_6 → white ellipses, evenly spaced
```

No `duplicate_node` tool yet — place each dot manually. Plan positions in the composition brief to avoid uneven spacing.

---

## Tail Feathers

**Do not use thin ellipses for feathers.** Ellipses are symmetric — real feathers taper to a point at the tip. Use the flowing tail feather path recipe from the SVG Path Recipes section.

3–5 feather paths, fanned out from the tail base. Draw from center feather outward:

```
tail_feather_3  (center, topmost in z-order) → create_path with feather recipe
tail_feather_2 / tail_feather_4  (flanking, slightly behind)
tail_feather_1 / tail_feather_5  (outermost, furthest back)
```

Use `apply_transform` with rotation around the tail base pivot to fan the feathers. Each feather rotates around the same `cx`/`cy` pivot point. Group all as `tail_group`.

---

## Legs and Feet

Legs: two thin tall rectangles.

```
leg_left  → rectangle (width=8, height=60), positioned center-bottom of body
leg_right → rectangle (width=8, height=60), offset slightly right
```

Toes: 3 short thin rectangles per foot, branching from the base of each leg. Rotate outward.

```
leg_left_toe_1 → rectangle (width=6, height=22), rotated -30°
leg_left_toe_2 → rectangle (width=6, height=22), no rotation (straight forward)
leg_left_toe_3 → rectangle (width=6, height=22), rotated +30°
```

Group: `leg_left_group` = [leg_left, leg_left_toe_1, leg_left_toe_2, leg_left_toe_3]

---

## Crest / Crown Feathers

3–5 teardrop-shaped paths, positioned at the top of the head, pointing upward at slight angles.

Use `create_path` with a simple teardrop SVG path. Rotate each slightly differently. Group as `head_crest_group`.

---

## Fill Types

Every shape creation tool (`create_shape`, `create_path`, `build_shape_from_points`) and `update_node` accept a `fill` object. Five types are available:

### Solid
```json
{"type": "solid", "color": "#5BA4D4"}
```
Default for most shapes.

### Linear Gradient
```json
{"type": "gradient", "gradient_type": "linear", "colors": ["#1a1a2e", "#4a90d9"], "coords": [x0, y0, x1, y1]}
```
`coords` are two document-space points: `[start_x, start_y, end_x, end_y]`.
For a top-to-bottom gradient on a 400×300 shape at (100, 50): `[100, 50, 100, 350]`.
`colors` supports 2+ hex stops (evenly distributed). For custom offsets, use the longer form with `offsets: [0.0, 0.5, 1.0]`.

### Radial Gradient
```json
{"type": "gradient", "gradient_type": "radial", "colors": ["#ffffff", "#5BA4D4"], "coords": [cx, cy, r]}
```
`coords` are `[center_x, center_y, radius]` in document space.
Inner color is `colors[0]`, outer color is `colors[last]`.
Use for eye irises, glowing highlights, spherical objects.

### Fluid Gradient
```json
{"type": "fluid_gradient", "points": [{"x": 100, "y": 50, "color": "#ff6b6b"}, {"x": 300, "y": 200, "color": "#4ecdc4"}, {"x": 500, "y": 80, "color": "#ffe66d"}], "power": 2.0}
```
Colors blended via inverse-distance weighting from freely-placed control points. Any number of points, each with an absolute document-space `x`/`y` and a `color`.
`power` controls blend sharpness (2.0 = smooth, 4.0+ = harder transitions).
Use for: atmospheric backgrounds, soft multi-color skies, organic color washes.

### Mesh Gradient
```json
{"type": "mesh_gradient", "rows": 2, "cols": 2, "vertices": [{"x": 0, "y": 0, "color": "#ff0000"}, {"x": 200, "y": 0, "color": "#00ff00"}, {"x": 0, "y": 200, "color": "#0000ff"}, {"x": 200, "y": 200, "color": "#ffff00"}]}
```
A `rows × cols` grid of colored vertices. Vertices must be supplied in row-major order (top-left → top-right → next row...).
Total vertex count must equal `rows × cols`.
Use for: complex color transitions across large flat areas, gradient backgrounds with corner-anchored colors.

### No Fill
```json
{"type": "none"}
```
Transparent — stroke only.

### When to Use Which

| Scenario | Fill Type |
|---|---|
| Flat body part, feather, plain background | `solid` |
| Sky, water, horizon fade | `linear` gradient |
| Sun, glow, lens flare, eye iris | `radial` gradient |
| Multi-color atmospheric wash, aurora | `fluid` gradient |
| Complex background with distinct corner colors | `mesh` gradient |
| Outline-only shape | `none` |

---

## Transform Quick Reference

`apply_transform` is used after placement to rotate, scale, move, or reflect shapes.

```jsonc
// Rotate a beak polygon 30° clockwise around its own center
{ "operation": "rotate", "node_ids": ["beak_id"], "rotate": { "angle": 30 } }

// Rotate tail feathers around a shared pivot (base of tail)
{ "operation": "rotate", "node_ids": ["tail_feather_2"], "rotate": { "angle": -15, "cx": 512, "cy": 700 } }

// Scale a wing ellipse wider without moving it
{ "operation": "scale", "node_ids": ["wing_left_base"], "scale": { "sx": 1.3, "sy": 1.0 } }

// Flip right wing from left wing
{ "operation": "reflect_horizontal", "node_ids": ["wing_right_base"] }

// Nudge a shape by exact pixels
{ "operation": "translate", "node_ids": ["leg_left"], "translate": { "x": -5, "y": 0 } }
```

Key rule: when fanning tail/crest feathers, pass the shared pivot as `cx`/`cy` — they all rotate around the same base point.

---

## Boolean Operation Guide

| Goal | Operation | Who is target | Who is tool |
|---|---|---|---|
| Merge two segments into one silhouette | `union` | Either shape | Other shape |
| Cut a hole or notch | `subtract` | Base shape (keeps fill) | Cutter (consumed) |
| Keep only the overlapping region | `intersect` | Base shape | Overlay |
| Inverse overlap (ring effect) | `exclude` | Outer shape | Inner shape |

**Pre-flight checklist before any boolean operation:**
1. Both nodes must be path nodes (not primitives from `create_shape`). Verify with `get_node`.
2. Both nodes must be on the same layer.
3. The target node inherits the fill/stroke of the result — choose which shape's style you want.
4. `keep_originals: false` (default) — both inputs are consumed and replaced with the result.
5. After the operation, rename the result node to reflect its new identity (e.g. `body_silhouette`).

---

## SVG Path Data Quick Reference

For `create_path`, the `path_data` field accepts standard SVG path commands:

| Command | Meaning |
|---|---|
| `M x y` | Move to (start new subpath) |
| `L x y` | Line to |
| `C x1 y1 x2 y2 x y` | Cubic bezier to (x,y) with control points (x1,y1) and (x2,y2) |
| `Q x1 y1 x y` | Quadratic bezier to (x,y) with control point (x1,y1) |
| `Z` | Close path |

Teardrop example (pointing upward, tip at top):
```
M 0,-50 C 30,-50 50,-20 50,10 C 50,35 30,50 0,50 C -30,50 -50,35 -50,10 C -50,-20 -30,-50 0,-50 Z
```

---

## Common Mistakes

| Mistake | Fix |
|---|---|
| Using `create_path` for a shape that a primitive covers | Prefer `create_shape` — fewer nodes, easier to edit |
| Boolean-unioning primitive shapes (ellipse, rect) | `boolean_operation` requires path nodes; draw as paths or use `create_path` |
| Forgetting which node is target vs tool in subtract | Target = the shape you want to keep (it inherits fill); tool = the cutter |
| Drawing eyes front-to-back | Always draw sclera first (back), then iris, then pupil, then highlight |
| Placing all dots/stripes by eye with no plan | Define approximate spacing in the composition brief before drawing |
| Overlapping shapes when union was intended | If the seam between two same-color shapes looks wrong, undo and use boolean union instead |
| **Using an ellipse for a wing** | Wing = swept path. Ellipse = blob. Use the bird wing SVG recipe. |
| **Using a polygon triangle for a beak** | Beak = curved path. Triangle = harsh geometric spike. Use the curved beak SVG recipe. |
| **Using thin ellipses for tail feathers** | Feathers taper to a point — ellipses are symmetric blobs. Use the tail feather SVG recipe. |
| **Using an ellipse for any body shape** | Bodies have asymmetric mass distribution. Use the organic body contour path recipe and adjust control points. |
| Answering "yes" to "can a primitive approximate this?" | If the shape has any taper, asymmetry, or swept curvature — the answer is always "no". Use a path. |
