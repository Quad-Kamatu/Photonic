# Changelog

All notable changes to Photonic are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

This file is the single source of truth for release notes: `scripts/release.sh`
rolls the `[Unreleased]` section into a versioned heading at release time, the
release CI uses that section as the GitHub release body, and the running app
embeds this file to show a "What's New" popup after an update.

## [Unreleased]

### Added

- **Gradient Overlay layer style (P4, #222).** The gradient-overlay effect now
  renders: a shape's Layer Styles stack can hold a Gradient Overlay that fills it
  with a gradient, composited with its own opacity + blend mode. Photoshop
  semantics — the gradient supplies the stops/kind while `angle` (0° = →, 90° = ↓)
  and `scale` (1.0 = fit the bbox) drive the geometry. Add it and tune
  opacity/angle/scale from the Layer Styles panel. Full fidelity in the export
  (PDF/raster/headless) compositor; live on-canvas GPU preview is a follow-up
  (as with gradient strokes, the live path currently shows only solid effects).

- **Artboard management over MCP.** Five new tools — `list_artboards`,
  `add_artboard`, `update_artboard`, `remove_artboard`, `set_active_artboard` —
  give agents the same artboard CRUD the GUI already had. All geometry edits go
  through a single `SetArtboards` command, so every change is one undoable step.
  A new `export_artboards` tool (and a matching GUI export picker) renders each
  board at its own size × scale.
- **Copy / paste of object subtrees.** Copying a group now deep-clones the whole
  subtree via `clone_subtree`: every node gets a fresh id and all intra-subtree
  references (group children, clip/blend-spine, threaded/area/path text) are
  remapped, so pasting no longer shares or corrupts the source's children.
  Backed by new `AddSubtree`/`RemoveSubtree` commands and an in-process
  `GuiClipboard` that survives switching between open documents (cross-document
  paste works).
- **Ungroup All.** A hotbar action and `plan_ungroup_all` recursively flatten a
  group and all nested groups down to leaves in a single undoable step.

- **Proportional editing over MCP.** A new `proportional_move_anchor` tool lets
  AI and scripted callers do the same Blender-style proportional edit as the
  interactive Proportional Move tool: name a path, the anchor index(es) to move,
  a displacement, and a `spread` radius + falloff `curve`, and neighbouring
  anchors follow along the falloff — committed as one undoable step. It shares the
  exact falloff math with the GUI tool, so both stay in lockstep.

## [0.2.2] - 2026-07-05

### Added

- **Proportional Move tool** — a new Direct Select sub-variant that brings
  Blender-style proportional editing to vector art (something Illustrator and
  Affinity don't have). Drag an anchor and its neighbours follow along a falloff:
  the whole region flexes instead of one point moving rigidly. While you hold the
  node, **scroll** grows or shrinks the falloff *spread* (radius) and
  **Shift+scroll** bends the falloff *curve* from soft to sharp; an on-canvas
  overlay shows the radius, a half-weight ring, and a live graph of the curve.
  Pick it from the Direct Select fly-out in the toolbar, and set the defaults in
  Tool Options.
- **Multi-document tabs.** Open several documents at once and switch between them
  from a tab bar along the bottom of the canvas — no more closing one file to work
  on another. Each tab keeps its own view, selection, and edit history.
- **Autosave and crash recovery.** Your work is saved in the background as you go,
  and if the app or your machine goes down, Photonic offers to restore the
  unsaved document when you reopen it. Closing a document with unsaved changes now
  warns you first, and the title shows how long ago the last save was.
- **All-new colour picker.** Every colour control now opens one industry-grade
  picker: a saturation/value square with hue and alpha bars, hex entry, and
  RGB / HSB / HSL / OKLCH numeric models — plus a perceptual tint ramp, harmony
  suggestions, recent and document swatches, an inline eyedropper, WCAG contrast
  badges, and a colour-blindness preview. Right-click a path → **Fill/Stroke
  Color** opens it directly.
- **Fill-aware fills with a slide-out gradient drawer.** The picker now edits
  solid, gradient, and pattern fills from one place. The gradient drawer has a
  live preview bar with draggable stops, per-stop **midpoint** control
  (Illustrator-style), add / duplicate / delete / reverse / distribute, and an
  **sRGB ↔ OKLab** toggle for perceptual blending with no muddy grey band. Save
  and re-apply gradient swatches.
- **Gradients now track the object.** Linear, radial, fluid, and mesh gradients
  can be scoped to an object's bounding box, so they move and scale with the
  shape instead of being pinned to the artboard, with a **"Rotate with object"**
  toggle so they rotate and shear along with it. **On-canvas gradient handles**
  let you drag endpoints, centre, radius, and gradient points right on the
  artwork while the fill popup is open.
- **Redesigned mesh gradient.** The mesh gradient is now a spreadsheet-style grid
  of coloured cells: click a cell to recolour it, drag the interior grid lines to
  resize cells, add or remove rows and columns, and use a **Blend** slider that
  runs from hard cell edges to a fully smooth blend.
- **Layers panel refresh.** Rows are decluttered — the name itself is the drag
  handle, and Shift-clicking a name multi-selects. Each row (or a right-click)
  opens a compact three-dot menu (rename, add sublayer, show/hide, lock, delete,
  layer-template toggle). Reordering now previews the drop with an insertion bar
  between rows and a drop-inside outline over groups and layer headers, with a
  subtle animated hover highlight. A pinned footer adds New Layer / Sublayer /
  Mask / Adjustment buttons and a live object count.

### Fixed

- **Radial, fluid, and mesh gradients render correctly on the GPU and headless
  renderers.** They previously collapsed to a flat blend of their edge colours
  (only linear gradients looked right); the fill is now subdivided so the true
  gradient shows through on every render path.
- **Radial gradient facet/wedge artifacts are gone.** Large radial fills no longer
  show sharp wedges or creases.
- **Mesh-gradient cell edges are clean.** Hard cell boundaries no longer zig-zag
  into a "static" look on the GPU/headless renderers.
- **Gradient on-canvas handles are easy to grab.** Hit-testing now starts from the
  exact press point with a larger grab radius and hover feedback, so grabbing a
  handle no longer misses and moves the whole object instead.

## [0.2.1] - 2026-07-05

### Added

- **Branching edit history (undo-tree).** Editing after an undo no longer throws
  away your redo path — it forks a new branch, so nothing you've done is ever
  lost. The History panel now shows the full tree as a VS-Code-style commit
  graph, and you can right-click any commit node (#174) to jump to, branch from,
  or act on that point in time.
- **Reimagined workspace UI.** The left drawer and its icon rail are now floating
  rounded cards, the hotbar is a centred content-width pill pinned under the top
  toolbar, and a mirrored right-hand icon rail toggles the Layers, AI Chat, and
  History drawers (replacing the old fixed right panel). The "What's New" popup
  now renders inline markdown and nested bullets.
- **Live boolean / compound shapes** (#25). A group can carry a live boolean
  operator (union, subtract, …): it renders and exports as the single resolved
  path while its operands stay individually editable — edit an operand and the
  boolean updates. Create one from the new **`make_live_boolean`** MCP tool.
- **Branch merging foundation** (#25). A pure 3-way document merge
  (`merge_3way`) that combines non-conflicting changes from two diverged edit-
  tree branches against their common ancestor and reports the rest as
  resolvable conflicts — the groundwork for true branch merges.
- **Layers panel is now a drag-and-drop folder tree** (#169, #210). Drag rows to
  reorder them, drag a node onto a group to reparent it (with a cycle guard and
  undo), expand/collapse groups as folders, and add a new layer straight from
  the Layers sidebar button.
- **Smarter snapping** (#211, #66). Objects now snap to artboard/canvas edges,
  centre, and margins, and to path anchor points, and show equal-spacing
  distribution hints while you drag — so aligning and evenly distributing
  objects by hand just works.
- **Import & Export controls in the Document tab** (#176): reach the common
  bring-in / send-out actions from the Document panel without hunting through
  menus.
- **Click the MCP status indicator for a control modal** (#170): see the tool
  server's state and restart it in place, instead of it being a passive dot.
- **Per-document edit-history budget** (#195, #196, #197). New files get a
  history size cap chosen at creation, raster edits are stored as just the
  changed region rather than whole snapshots (#196), and you get a proactive
  warning as a document approaches its history limit.
- **Larger hotbar icons.** The adaptive hotbar's icons are bumped up a couple of
  points so the row reads clearly at the top of the canvas.

### Changed

- **Under the hood: unified tool architecture** (#190). Canvas tools now share a
  common parent trait with a single mutation chokepoint and lifecycle seam. This
  is an internal cleanup — every tool behaves exactly as before — that makes the
  drawing tools consistent and easier to extend. Please report anything that
  behaves differently after updating.

### Fixed

- **The UI is now centred and clickable on every display** — including HiDPI and
  fractionally-scaled monitors. The opening screen (and in fact the whole editor)
  could render off-centre with clicks landing beside the widgets on any monitor
  whose scale factor wasn't 1.0; a pixels-per-point mismatch in the window layer
  is corrected, with no change on standard-scale displays.
- **Tool-created shapes are now fully undoable** (#190). Creating a shape, text,
  pen, or duplicated object through a canvas tool now always records a proper
  history step, so undo/redo can't skip past it.
- **Raster edit history is bounded by real memory use** (#194), not just its
  serialized size — keeping the in-memory footprint of image edits in check.

## [0.2.0] - 2026-07-04

### Added

- **Gradient (and pattern) strokes** (#201). Strokes are no longer limited to a
  single solid colour — a `Stroke` can now carry any paint (linear/radial
  gradient, pattern), rendered on-canvas, in headless/raster export, and in SVG
  (`stroke="url(#…)"`). Line icons with a gradient outline are now a first-class
  paint instead of an outline-stroke→delete→refill workaround. Set one via the
  new `set_paint` tool (`target: "stroke"`) or a `stroke.paint` object.
- **`set_paint` MCP tool** (#202): apply one paint to many nodes in a single
  undoable call, each re-fit to its own bounding box. Gradients accept
  `units: "bbox"` with 0–1 coordinates, so one relative gradient (e.g. left→right
  blue→purple) styles a whole icon set with zero per-node coordinate maths.
- **`export_icon_set` MCP tool + "Export Icon Set…" command** (#203): batch-export
  tagged groups (or every top-level group) to normalised, uniform-square `.svg`
  files in one step — no external post-pass to make an icon set render at a
  consistent scale.
- **`preview_selection` MCP tool** (#204): render the selection at target display
  sizes over light AND dark backgrounds as one contact-sheet PNG, to judge
  small-size legibility and on-surface contrast without leaving Photonic.
- **`import_design_tokens` MCP tool + "Import tokens…" swatch action** (#207):
  register named brand swatches from a CSS / JSON / Style-Dictionary tokens file
  (the counterpart to `export_design_tokens`). Paints can then reference brand
  colour by name, and re-importing re-themes everything at once.
- **Icon keyline grid + snap-to-pixel** (#208): a View toggle overlays the classic
  Material/Apple icon keyline template (square, circle, portrait & landscape safe
  areas) on the artboard, and "Snap to Pixel" lands drawing/moving on crisp
  integer coordinates.

### Changed

- **Under the hood: a big internal cleanup, with no change to how Photonic
  works.** The app's largest source files were reorganised into smaller, focused
  modules across the editor, the panels, the renderer, and the built-in tool
  server. This is purely a maintainability refactor — every feature, tool, and
  keyboard shortcut behaves exactly as before, and your documents are
  unaffected. It just makes Photonic faster to improve from here. If you do spot
  anything behaving differently after updating, please report it.
- **Selection SVG export is compact and consistent** (#205, #206, #203).
  `export_selection_as_svg` now rounds coordinates and path data to a `precision`
  (default 4 — no more 15-decimal path bloat), deduplicates byte-identical
  gradient/pattern defs into a single shared `<defs>` entry (across fills *and*
  strokes), and takes `normalize: "square"` to frame each icon in a uniform
  centred square viewBox.

### Fixed

- **Eyedropper now samples raster images** (and transformed shapes). In-canvas
  colour sampling only ever hit vector paths, so clicking the eyedropper on an
  imported image sampled nothing; it also hit-tested shape geometry against the
  raw canvas point, so moved/scaled/rotated shapes sampled the wrong spot.
  Sampling now maps the click into each node's local space, reads the pixel of a
  raster layer (honouring its layer mask, falling through transparent pixels to
  whatever is beneath), and keeps gradient colours matched to the on-screen
  render. Applies to both the GUI eyedropper and the `sample_color_at` MCP tool.

### Added

- **Rotate objects from the canvas.** Hover just outside a corner handle with
  the Select tool — a rotate affordance appears — then drag to rotate the
  selection in place about its centre (Illustrator/Photoshop-style). Works for
  single or multiple objects; hold Shift to snap to 15° increments; one undo
  step. (Precise numeric rotation in the Transform inspector still works too.)
- Image import in the GUI (previously raster layers could only be placed via
  the MCP `place_image` tool):
  - **File → Place Image…** imports a PNG/JPEG/WebP/BMP/GIF/TIFF as a raster
    layer, centred on the artboard, selected, and undoable.
  - **File → Open…** and the welcome screen's Browse now accept image files
    too: from the editor the photo is placed into the current document; from
    the welcome screen it opens as a fresh artboard sized to the photo.
  - Image files can be **dragged & dropped** onto the window on X11, Windows,
    and macOS. (Not on Wayland yet — winit's Wayland backend has no
    drag-and-drop support.)
- **Crop to Artboard** on raster layers (Inspector → Raster Layer): trims the
  image — and its layer mask, in lockstep — to the bounds of the artboard the
  image is on (the one it overlaps most, in the spatial multi-artboard model),
  discarding pixels outside while keeping the surviving pixels exactly where
  they were on canvas. Destructive but undoable; rotated images are rejected
  rather than silently resampled.
- **Group selection with the Select tool**: clicking any member of a group now
  selects every object in its outermost group (Illustrator behavior), so the
  whole group moves/edits as a unit; Shift+click toggles the whole group in and
  out of the selection. Alt+click still grabs just the clicked member, and
  double-click still enters isolation mode for editing inside the group.

### Fixed

- **Shape Builder now merges across layers, with feedback.** Like the boolean
  combine below, it silently did nothing unless every touched shape lived in the
  same layer. It now folds all dragged-over path shapes regardless of layer,
  reports the result (or why nothing merged) in the status bar, and hints when a
  drag only caught one shape.
- **Union / boolean combine now actually merges shapes.** The Union, Subtract,
  Intersect, and Exclude operations silently did nothing unless *exactly two*
  path objects in the *same layer* were selected, and gave no feedback when
  they didn't run. They now combine **any number** of selected path shapes,
  **across layers**, folding bottom-to-top (Union merges all, Subtract removes
  the upper shapes from the bottom one, etc.), and report what happened in the
  status bar — including why nothing merged ("Select 2 or more path shapes",
  "produced an empty shape").
- **Strokes no longer scale with the object.** Scaling a shape (drag its
  bounding-box handles, or any transform with a scale) kept its stroke a fixed
  width instead of thickening/thinning it — a stroke is an absolute property,
  not geometry (Illustrator's "Scale Strokes & Effects" off). Applied uniformly
  across the live canvas, raster/PNG export, and SVG/PDF export, so all four
  agree. Rotation and translation are unaffected; view zoom still scales
  everything as before.
- Opening a photo from the welcome screen now also sizes the **artboard** to
  the photo (previously only the document dimensions changed, leaving the
  visible artboard at its default size — which also made Crop to Artboard trim
  against the wrong rectangle).
- Raster Masking on imported images (Inspector → Raster Masking, shown when a
  raster layer is selected): pick a color on the canvas and hide every pixel
  within an adjustable fuzziness of it — globally (Color Range) or only the
  connected region under the click (Contiguous / magic-wand) — with a live
  preview and Apply/Cancel. Non-destructive: the pixels are hidden via the
  layer mask, never erased, and the edit is a single undo step.
- One-click **Remove Background** on raster layers: a small local matting
  model (U²-Net-p, Apache-2.0) detects the subject fully on-device via ONNX
  Runtime and applies it as a non-destructive foreground layer mask. The
  ~5 MB model downloads once to the Photonic cache, then works offline. Also
  exposed as the `remove_background` MCP tool, and a Clear Layer Mask button
  reveals the layer again at any time.
- Opt-in crash reporting and diagnostics (#59): when Photonic panics it now
  writes a structured, non-sensitive crash report (app version, UTC time,
  OS/arch, panic message, backtrace) to a `crash-reports/` folder in your
  Photonic config directory — local
  capture is always on. Sending is opt-in: on the next launch a one-time consent
  dialog (or, once enabled, a Report/Dismiss banner) offers to file the crash as
  a pre-filled GitHub issue you review in your browser before submitting. No
  document content, file paths, or environment variables are ever collected, and
  nothing is sent automatically. A new "Privacy & Diagnostics" settings tab adds
  the consent toggle, an Open crash-report folder button, and a Report a bug
  button.
- Auto-check-on-launch update prompt: once per launch Photonic asks GitHub for
  the latest release (no download) and shows a dismissable banner if a newer
  version exists.
- "What's New" popup that appears after updating, summarising changes in the
  versions you skipped (sourced from this changelog).

## [0.1.0] - 2026-06-29

### Added

- First public release of Photonic — a cross-platform vector + raster graphics
  editor built in Rust (egui / wgpu).
- Guided cinematic welcome flow with a live Lightfall shader background, a
  searchable size catalog (~130 presets), advanced New-Canvas options
  (DPI/PPI, bleed, slug, margins, artboard count), and recent-document
  thumbnails.
- Spatial multi-artboard documents: model, rendering, in-editor rename / drag /
  resize with artwork that moves with its board, alignment + equal-distance
  distribution snapping, and per-board export.
- Global command palette with direct + on-device semantic search (bundled
  embedding model, fully local).
- Disk search for `.photon` files across user-picked roots and the OS index.
- Photoshop-grade raster editing subsystem (engine, Raster node, brush/eraser
  tools, MCP tools, export).
- Signed auto-update pipeline: GitHub Releases as host, ed25519-signed archives
  verified before install, single-source semantic versioning.
