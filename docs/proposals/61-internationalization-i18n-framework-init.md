# Internationalization (i18n) framework + initial translations (#61) — Design Proposal

> Status: design scaffold (not an implementation).

## Summary

All user-visible strings in `crates/photonic-gui` are hardcoded English. There is no
localization framework, no external string resource, and no locale selection. This blocks
non-English speakers and prevents community-contributed translations. The proposal adopts
Mozilla Fluent (via the `fluent` + `fluent-bundle` crates) as the i18n framework and
establishes the pipeline to externalize GUI strings and ship two or three seed locales.

## Scope

**In:**
- Adopt Fluent as the message catalog format; store `.ftl` files under a new `crates/photonic-gui/i18n/<locale>/` tree.
- A thin `Localizer` singleton in `photonic-gui` that resolves message IDs to strings at runtime, with English fallback.
- Locale selection in `AppPreferences` (stored as a BCP 47 tag, e.g. `"en-US"`); auto-detect from OS locale on first run.
- Seed translations: `en-US` (source), `es-ES`, and `fr-FR` to prove the pipeline.
- Number/unit formatting (decimal separator, unit labels) per locale via the `num-format` crate.
- CI check: lint that no string literal calls `.label("…")` or `.button("…")` without going through `t!()`.

**Out:**
- RTL layout (Bidirectional text requires egui work beyond string substitution; separate epic).
- Canvas text node content — that is user data, not UI chrome.
- MCP tool names/descriptions (internal API, English-only is acceptable).

## Proposed approach

1. **Fluent crates**: Add `fluent = "0.16"` and `fluent-bundle = "0.15"` to `crates/photonic-gui/Cargo.toml`. These are pure-Rust, no C deps.

2. **`Localizer` struct**: In a new file `crates/photonic-gui/src/i18n.rs`, implement:
   ```rust
   pub struct Localizer { bundle: FluentBundle<FluentResource> }
   impl Localizer {
       pub fn load(locale: &str) -> Self { /* reads i18n/<locale>/*.ftl from embedded bytes */ }
       pub fn t(&self, id: &str) -> &str { /* lookup with en-US fallback */ }
   }
   ```
   Bundle the `.ftl` files into the binary via `include_str!` or the `rust-embed` crate.

3. **Macro `t!()`**: Define a macro `t!(ctx, "message-id")` that calls `ctx.localizer.t("message-id")` so call sites are concise.

4. **Migration strategy**: Begin with the most-used panels (tool names in `crates/photonic-gui/src/tools/mod.rs`, panel headers in `crates/photonic-gui/src/panels/mod.rs`, menu items in `crates/photonic-gui/src/app.rs`). Do not attempt a big-bang migration; extract strings panel-by-panel behind feature flag `i18n` initially.

5. **Locale preference**: Add `pub locale: String` (default `"en-US"`) to `AppPreferences` in `crates/photonic-gui/src/preferences.rs`. On first run, detect OS locale via `sys-locale` crate (`sys-locale = "0.3"`). Expose locale picker in Settings.

6. **`.ftl` file layout**:
   ```
   crates/photonic-gui/i18n/
     en-US/
       tools.ftl
       panels.ftl
       menus.ftl
       dialogs.ftl
     es-ES/
       tools.ftl   # seed
     fr-FR/
       tools.ftl   # seed
   ```

7. **CI lint**: A `cargo test` integration test that walks the GUI source for bare string literals passed to egui label/button APIs (via regex) and fails if any are found outside `i18n/` or test files.

## Affected modules

- `crates/photonic-gui/Cargo.toml` — add `fluent`, `fluent-bundle`, `sys-locale`, optionally `rust-embed`
- `crates/photonic-gui/src/i18n.rs` — new `Localizer`, `t!()` macro
- `crates/photonic-gui/src/preferences.rs` — `AppPreferences::locale: String`
- `crates/photonic-gui/src/app.rs` — locale detection at startup, Settings locale picker
- `crates/photonic-gui/src/tools/mod.rs` — migrate `name()` strings to `t!()`
- `crates/photonic-gui/src/panels/mod.rs` — migrate panel headers, labels
- `crates/photonic-gui/i18n/` — new directory (`.ftl` files, not Rust source)

## Risks & open questions

- **egui string lifetime**: egui widgets often accept `&str` with `'static` lifetime. Fluent returns owned `String` or `Cow`. The `t!()` macro may need to return an owned `String` and callers use `.as_str()` — confirm egui 0.29 API.
- **String extraction effort**: The panels file (`panels/mod.rs`) is very large; mechanical extraction is tedious but not technically hard.
- **Plural rules**: Fluent handles plurals natively; however English source strings must be written to use Fluent selector syntax (`.one`, `.other`) from the start — retrofitting later is painful.
- Open Q: Should `.ftl` files be embedded in the binary or loaded from the filesystem (to allow community patches without recompiling)?

## Acceptance criteria

- [ ] Switching locale in Settings reloads UI strings without restart (or with restart — document which).
- [ ] All tool names and panel headers in `en-US` are served from `.ftl` files.
- [ ] `es-ES` and `fr-FR` seeds translate at least the tool names and panel headers.
- [ ] Missing message IDs fall back to `en-US`, never panic.
- [ ] CI lint fails if a new bare English string is added to a widget call site.

## Effort estimate

**L** — Framework setup is small; mass migration of all panel strings and maintaining fallback correctness is the long tail.
