# Spell check (in-document, with squiggle UI) (#34) — Design Proposal

> Status: design scaffold (not an implementation).

## Summary

No spell-checking infrastructure exists anywhere in the codebase. The existing grammar-rule
system (`GrammarRule`, `ActionSet` in `photonic-core/src/document.rs`) provides a conceptual
precedent for rule-based text constraints, but spell check is a distinct, dictionary-driven
feature. The goal is in-canvas squiggle underlining of misspellings, a right-click suggestion
menu, and an MCP `check_spelling` tool.

## Scope

**In scope**
- Dictionary-based spell check per `TextNode` with per-language dictionaries (hunspell `.aff`/
  `.dic` format).
- Underline misspelled word ranges in the canvas (decorative only — stripped from SVG/PDF export).
- Suggestions and "Add to dictionary" (user personal dictionary stored in preferences).
- MCP: `check_spelling(node_id)` returning word ranges + suggestions.
- Language selection per document and per text node.

**Out of scope**
- Grammar / style checking (distinct from spelling).
- Autocorrect / autocomplete.
- Inline rich-text cursor (spell ranges are computed offsets into the flat `TextNode.content`
  string, not into styled runs).

## Proposed approach

1. **Dictionary integration**: add `zspell` or `symspell` as a workspace dependency (both are
   pure-Rust hunspell-compatible; `zspell` is the more mature). Dictionaries ship as embedded
   assets or are loaded from a user data directory. Start with `en_US` bundled.

2. **Model** (`crates/photonic-core/src/node.rs`): add to `TextNode`:
   ```rust
   pub spell_language: Option<String>,   // e.g. "en_US"; None = inherit document default
   pub spell_check_enabled: bool,        // default true
   ```
   No misspelling data is stored in the document; it is computed at runtime and discarded on
   save.

3. **Spell engine** (new file `crates/photonic-core/src/spell.rs`):
   - `SpellError { byte_start: usize, byte_end: usize, suggestions: Vec<String> }`
   - `fn check_text(content: &str, lang: &str, personal: &[String]) -> Vec<SpellError>`
   - Called lazily when a text node's content changes; result cached in a
     `HashMap<NodeId, Vec<SpellError>>` held by the document state (not persisted).

4. **Renderer** (`crates/photonic-render/src/renderer.rs`): after the glyphon text layout pass,
   convert byte-offset `SpellError` ranges to screen-space rectangles using glyphon's glyph-run
   geometry (available after `shape_until_scroll`). Draw a wavy red underline for each range as
   a thin tessellated path using the existing wgpu fill pipeline.

5. **GUI** (`crates/photonic-gui/src/app.rs` / panels):
   - Pass spell errors down to the canvas draw call.
   - Right-click on a text node opens context menu with "Spelling suggestions" sub-menu;
     selecting a suggestion emits `PanelAction::UpdateNode` with the corrected content.
   - "Add to dictionary" appends to user preferences stored in
     `crates/photonic-gui/src/preferences.rs`.
   - Document-level language selector in Document Properties panel.

6. **MCP** (`crates/photonic-mcp/src/handlers/nodes.rs`): new handler `check_spelling(node_id,
   lang?)` returns a JSON array of `{word, start, end, suggestions}`.

## Affected modules

- `crates/photonic-core/src/node.rs` — `TextNode.spell_language`, `TextNode.spell_check_enabled`
- `crates/photonic-core/src/spell.rs` — new: `SpellError`, `check_text`
- `crates/photonic-core/src/document.rs` — `Document.spell_language: Option<String>` (default lang)
- `crates/photonic-render/src/renderer.rs` — squiggle underline rendering after text layout
- `crates/photonic-gui/src/app.rs` — pass spell errors to canvas; context menu
- `crates/photonic-gui/src/preferences.rs` — personal dictionary storage
- `crates/photonic-mcp/src/handlers/nodes.rs` — `check_spelling` tool
- `Cargo.toml` (workspace) — add `zspell` (or equivalent) dependency

## Risks & open questions

- **Dictionary bundling vs. system**: bundling `en_US` is ~2 MB; system hunspell dicts vary by
  distro. Decide policy: always bundle EN, load system dicts for other languages.
- **Byte-offset to glyph-position mapping**: glyphon 0.6 glyph-run API needs auditing to
  confirm per-glyph bounding-box access after layout; this is the riskiest rendering step.
- **Re-check latency**: spell-checking on every keystroke is fine for short nodes; throttle
  with a 300 ms debounce for large text areas.
- **Export exclusion**: squiggle must not appear in SVG/PDF/PNG exports. Guard all underline
  draw calls with a `!is_export` flag threaded through the render pipeline.
- **Grammar rules vs. spell**: the existing `GrammarRule` system (`document.rs:331`) operates
  on design constraints, not text content — no reuse possible; treat spell as independent.

## Acceptance criteria

- [ ] Misspelled words in a text node show a red squiggle underline in the canvas.
- [ ] Squiggle is absent from SVG, PNG, and (future) PDF exports.
- [ ] Right-click on a misspelled word shows ≥1 suggestion for common errors ("teh" → "the").
- [ ] "Add to dictionary" removes the squiggle for that word permanently in the session/prefs.
- [ ] MCP `check_spelling` returns the correct byte-range and suggestions for a known misspelling.
- [ ] Language can be set per-document and overridden per text node.

## Effort estimate

**M** — the dictionary integration and MCP tool are straightforward; the glyph-position-to-screen
mapping for squiggles is medium complexity; the GUI context menu wiring is standard egui work.
