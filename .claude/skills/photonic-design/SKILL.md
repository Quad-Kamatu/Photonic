---
name: photonic-design
description: Use when deciding color palettes, placing subjects on canvas, choosing visual emphasis, or evaluating why a Photonic illustration lacks visual quality.
---

# Photonic Design Principles

## Overview

Design quality is separate from technical correctness. A shape can be on the right layer, correctly named, and still look wrong. This skill encodes the judgment rules that make illustrations visually compelling.

**Core principle:** Every decision — color, size, placement, stroke — must serve a hierarchy. Decide what the viewer should look at first, and make every other choice support that.

---

## 1. Color Theory & Palette

### Harmony Types

| Harmony | Construction | When to Use |
|---|---|---|
| Analogous | 3 adjacent hues (≤90° arc) | Peaceful, natural subjects (nature, landscapes) |
| Complementary | Opposite hues (180° apart) | High-contrast focal points, bold energy |
| Split-complementary | 1 hue + 2 hues flanking its complement | Vibrant but not harsh — good for characters |
| Triadic | 3 hues 120° apart | Playful, diverse illustration sets |
| Tetradic | 4 hues at 90° apart | Complex multi-element scenes; keep one hue dominant |
| Monochromatic | 1 hue at varying value/saturation | Elegant, minimal, cohesive moods |

### 60-30-10 Rule

Assign colors to surface areas before drawing anything:
```
60% — dominant color (backgrounds, large fills, primary surfaces)
30% — secondary color (supporting shapes, secondary elements)
10% — accent color (focal details, highlights, call-out elements)
```
Violating this ratio creates visual chaos. The accent must be the rarest color.

### Value and Saturation Rules

- **Value contrast** (lightness difference) is more important than hue difference for readability. Two shapes of similar value will merge visually regardless of their hue.
- A focal element should differ from its background by at least 30% in value.
- Do not pair two fully saturated colors of similar value — reduce one to 60–70% saturation.
- **Warm colors advance; cool colors recede.** Use this for depth: warm foreground, cool background.
- **Grayscale test:** The value structure must carry the hierarchy independently of hue. Convert all fills to grayscale — if the composition is unreadable or the focal element doesn't stand out, fix the value distribution before adding color back.
- **Shadows are never pure black.** Shadow color = base fill with lower value + slightly higher saturation + hue shifted cooler (toward blue/purple). Pure black shadows look flat and amateurish.
- **Highlights are never pure white.** Shift hue slightly toward warm yellow for warm light sources, toward cyan for cool light sources.
- One consistent implied light source direction per illustration. Classic convention: upper-left. Never mix light directions.

### Palette Size Limits

| Subject Complexity | Max Named Colors |
|---|---|
| Icon / simple shape | 3–5 |
| Character (single) | 5–8 |
| Scene with background | 8–12 |

More than 12 named colors signals palette breakdown — consolidate before drawing.

---

## 2. Composition & Visual Balance

### Placement Rules

**Rule of thirds:** Divide canvas into 9 equal zones. Place the primary subject at or near an intersection point (25%, 75% horizontally × 33%, 67% vertically). Dead center is static — use it only for deliberately symmetrical, formal subjects.

**Optical center:** The eye perceives the center of a canvas as being approximately 10% above the mathematical midpoint. Placing a subject at the mathematical center looks slightly low. Aim for the optical center (roughly 45% of canvas height) for a "feels centered" placement.

**Golden ratio (~1.618):** When sizing two related elements, let the larger be 1.618× the smaller. Applies to: head-to-body ratio, subject-to-background ratio, primary-to-secondary element size. For character proportions: 7–8 head heights = realistic adult; 5–6 = standard stylized; 3–4 = cartoon/chibi. Taller = elegant/heroic; shorter = cute/innocent.

**Rule of odd numbers:** Groups of 3 or 5 are more dynamic than 2 or 4.

### Compositional Paths

Guide the eye through the canvas using implied paths:
- **C / S curve**: Eye sweeps across and back — landscapes, flowing organic scenes
- **Z / F pattern**: Top-left → top-right → diagonal down — left-to-right reading audiences
- **V / triangle**: Directs toward center-bottom apex — signals stability and hierarchy
- **Lead lines**: Any continuous element (path edge, shadow, outstretched limb, tail) pointing toward the focal point. Lines that reach an edge stop the eye; lines pointing inward are correct.

**Tangent line trap:** Two unrelated shapes that just barely touch at their edges create an uncomfortable visual coincidence. Either overlap them deliberately or separate them clearly — never let them just kiss.

### Balance Types

| Type | When to Use | Example |
|---|---|---|
| Symmetrical | Formal, stable, logo-like subjects | Centered face, emblem, badge |
| Asymmetric | Natural, dynamic, energetic subjects | Character in action, off-center subject |
| Radial | Decorative, mandala-like, star burst | Sun rays, circular badge, floral |

**Visual weight rules:**
- Darker = heavier
- Larger = heavier
- Isolated = attracts more attention than grouped
- Saturated = heavier than desaturated
- Warm = heavier than cool (visually advances)

Balance by opposing weight: a small dark shape on the right can balance a large light shape on the left.

### Negative Space

Negative space is not empty — it is active. Test: reduce canvas background to a single tone and check whether the main subject has a clear, readable silhouette. If the silhouette is ambiguous, the shapes are not distinct enough.

**Silhouette rule:** Cover all fills with gray. If you cannot identify the subject from the shape alone, the design has a readability problem. Fix with stronger shape contrast, not more detail.

---

## 3. Shape Language & Figure-Ground

Every shape carries implicit emotion. Choose shapes that match the subject's character:

| Shape | Feeling | Use For |
|---|---|---|
| Circle / ellipse | Friendly, safe, approachable, soft | Characters, cute creatures, highlights |
| Square / rectangle | Stable, trustworthy, structured | Backgrounds, technical elements, structural parts |
| Triangle (point up) | Dynamic, energy, aspiration | Pointed features, crowns, spines, fire |
| Triangle (point down) | Unstable, danger, tension | Fangs, claws, threat elements |
| Diagonal lines | Movement, speed, aggression | Limbs in motion, lightning, motion paths |
| Curves | Flow, nature, comfort | Hair, water, organic bodies |

**Mix consciously:** The dominant shape type determines the emotional read of the whole character. A character built primarily from circles but with sharp angular brows reads "friendly but aggressive". Match intent: approachable characters → rounded everywhere; threatening ones → angular features on a rounded base.

### Figure-Ground

Three states:
- **Stable (default):** Clear figure on neutral ground. Subject has higher contrast, sharper edges, more detail than background. Always use this unless deliberately designing otherwise.
- **Reversible (deliberate):** Figure and ground compete equally — the eye alternates. Only attempt this intentionally.
- **Ambiguous (accident):** Elements fuse into ground because they share similar value or color. This is a failure state. Fix with value contrast.

**To ensure stable figure-ground:** Subject must be higher contrast, more detailed, and have sharper edges than anything behind it. Light figure on dark ground, or dark on light — never similar values.

---

## 4. Visual Hierarchy

The viewer's eye must be guided. Hierarchy is set by contrast, not decoration.

**Reading order:** The eye scans illustrations in this priority: **size → contrast → color → shape → position → detail**. If your focal element is not the largest, it must win on contrast. If it doesn't win on contrast, it must win on color (most saturated or warmest). Plan which lever you are pulling.

**The three-tier rule:**

| Tier | Purpose | How to Achieve |
|---|---|---|
| Primary (1 element) | The first thing the viewer sees | Highest contrast, largest, or most saturated |
| Secondary (2–4 elements) | Supporting context | Moderate contrast, medium scale |
| Tertiary (rest) | Environment, texture | Low contrast, similar to background |

Use no more than 3 distinct size tiers. Size ratio between adjacent tiers must be at least 1.5:1 to read as intentional — closer ratios look accidentally similar, not deliberately differentiated.

**Never have two elements at equal contrast competing for primary position.** If the eye cannot decide where to look first, the composition fails.

**Size hierarchy:** The subject should occupy 40–60% of the canvas height for a portrait illustration. Background elements should not exceed 80% of subject size.

---

## 5. Depth and Layering

Depth creates the illusion of three-dimensional space on a flat canvas.

| Depth Cue | Foreground | Background |
|---|---|---|
| Size | Larger | Smaller |
| Overlap | In front | Behind |
| Value contrast | Higher | Lower |
| Saturation | Higher | Lower, more muted |
| Color temperature | Warmer | Cooler |
| Detail density | More detail | Simpler, less detailed |

**Atmospheric perspective (for scenes):** Foreground = 100% saturation/contrast. Midground = 60–70%. Background = 30–50%. Background hue shifts toward the atmospheric color of the scene (blue/grey outdoors, warm tint at sunset). Background edges are softer than foreground edges.

**Shadow rule:** Cast shadows anchor elements to a surface. Without a shadow or contact point, floating objects look ungrounded. Add a small dark ellipse under floating subjects.

---

## 6. Style Consistency

Pick one style and hold it throughout the entire illustration.

### Style Rules

**Flat**
- No gradients, no drop shadows, no embossing.
- Depth achieved purely through color, overlap, and scale — no light simulation.
- Max 2 value zones per shape (base + one highlight OR base + one shadow — not both).
- Corner radii: choose one radius value and apply everywhere. Either all sharp (0px) or all rounded consistently. Never mix.
- Fatal mistake: adding one shadow or gradient to an otherwise flat illustration — the exception destroys the style's coherence.

**Outlined / Line-art**
- One stroke weight for primary outlines. Interior detail strokes at 50–60% of primary weight only.
- Stroke color: near-black (90–100% of the darkest palette color), not pure black unless monochromatic.
- Line caps: choose once — round (warmer/friendlier) or square (technical/precise) — apply everywhere.
- Fills: either flat color inside strokes or no fill at all. Never mix filled and unfilled elements without logic.

**Semi-flat / Soft** (current dominant commercial style)
- Gradients allowed but subtle: start and end colors within 10–20% brightness of each other. No rainbow gradients.
- Shadows: soft (blur radius 10–30% of shadow-caster's size), low opacity (15–30%), maximum one shadow layer per object.
- Radial gradients on round forms. 1 highlight dot per curved surface.

**Isometric**
- All surfaces use exact 30° angles from horizontal — no approximation.
- Three visible faces: top (lightest), left (midtone −15 to −20% value), right (darkest −30 to −40% value). Consistent light source across all objects.
- No perspective convergence — parallel lines stay parallel at all scales.
- Stroke optional; when used, same weight on everything.

**Geometric**
- Only straight lines and mathematically regular curves. No freeform organic paths.
- Colors from a strict limited palette. Hard edges, no softening.

**Mixed-style violation:** Using detailed realistic shading on one element while keeping others flat is a style break. All elements must follow the same shading logic with no exceptions.

### Stroke Weight Standards

| Canvas Size | Icon-scale Stroke | Character-scale Stroke |
|---|---|---|
| 24px grid (icon) | 1–1.5px | — |
| 256px | 1.5–2px | 2–3px |
| 512px | 2–3px | 3–5px |
| 1024px | 3–5px | 5–8px |

Rule: stroke weight ≈ 0.5–1% of canvas width. Below this threshold, strokes disappear at normal viewing scale. Above 2%, strokes dominate the fill.

**Consistency rule:** All outlines in a set must use the same weight. Varying stroke weights between shapes in a single illustration creates inconsistency unless intentional (e.g., thin inner details, thick outer silhouette).

---

## 7. Detail Density and Breathing Room

**Less is more in vector illustration.** Each added element competes for attention. Before adding a detail, ask: does this serve the primary focal point?

- Leave 10–15% canvas margin around the main subject (breathing room to edge).
- Do not fill every region with pattern or texture — let the dominant color breathe.
- Highlights: max 1 specular highlight per curved surface. More than 2 highlights on one surface looks cluttered.
- Details visible at intended display size only. Elements smaller than 1% of canvas width disappear at normal scale — omit them.

---

## 8. Common Design Mistakes

| Mistake | Symptom | Fix |
|---|---|---|
| Too many colors | Eye bounces without settling | Apply 60-30-10, consolidate to max 8 named colors |
| Equal-value background and subject | Subject disappears into background | Increase value contrast by 30%+ |
| Centered dead-center composition | Stiff, logo-like feel when dynamic subject intended | Move subject to rule-of-thirds intersection |
| All colors at full saturation | Garish, overwhelming | Desaturate secondary and tertiary colors 20–40% |
| Pure black shadows / pure white highlights | Flat, amateurish lighting | Shadows = shifted hue + higher saturation; highlights = warm or cool tinted |
| Fails grayscale test | Only works in color, hierarchy collapses in grayscale | Fix value distribution so primary element has dominant contrast |
| Tangent lines | Two unrelated shapes barely touching — visual discomfort | Overlap deliberately or separate clearly — never let shapes just kiss |
| Style inconsistency | One shadow/gradient in an otherwise flat illustration | Exceptions destroy style coherence. Apply one style's rules to every element |
| No focal point | Everything looks equally important | Identify primary element, increase its contrast/size |
| Stroke weight inconsistency | Some shapes look bold, others thin | Standardize to one weight across all outlines |
| Floating elements (no ground contact) | Objects look cut-and-pasted | Add shadow ellipse or surface contact shape |
| Over-detailed background | Background competes with subject | Simplify background, reduce saturation 30–50% vs foreground |
| Equal detail density everywhere | No visual rest area; nothing emphasized | Concentrate detail at focal point, diminish toward edges/background |
