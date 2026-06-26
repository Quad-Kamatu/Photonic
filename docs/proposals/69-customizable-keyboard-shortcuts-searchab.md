# Customizable Keyboard Shortcuts + Searchable Command Palette (#69) — Design Proposal

> Status: design scaffold (not an implementation).

## Summary

Keyboard shortcuts are currently hardcoded in the Select tool handler (`app.rs:9117–9217`) and scattered throughout the event loop with no central registry. This issue introduces a `CommandRegistry` mapping every action to a stable id and a default shortcut, a user-editable `KeyMap` persisted alongside `AppPreferences`, and a fuzzy command palette (Ctrl+K) that can find and execute any registered command by name.

## Scope

**In**
- `CommandRegistry`: static list of all commands (id: `&'static str`, label, default shortcut, handler reference or `PanelAction` equivalent)
- `KeyMap`: `HashMap<CommandId, KeyBinding>` loaded from config and merged over the defaults; serialized to the same prefs file as `AppPreferences`
- Key conflict detection on edit (warn when a binding shadows another)
- Searchable command palette overlay: Ctrl/Cmd+K opens a floating egui modal; typing fuzzy-filters all commands; Enter runs the highlighted one
- Keyboard Shortcuts settings page (list + inline remap)
- Import / export keymap as JSON
- MCP / AI operations surfaced in the palette (if they have a corresponding `PanelAction`)

**Out**
- Per-tool-mode shortcut scoping (all bindings are global for M1)
- Mouse-gesture or chord (multi-key sequence) bindings
- Platform-specific (macOS Cmd vs Linux Ctrl) auto-swap (use a `Modifier::Primary` abstraction, but full platform handling is deferred)

## Proposed Approach

1. **`CommandId` and `CommandRegistry`** — new file `crates/photonic-gui/src/commands.rs`:
   ```rust
   pub type CommandId = &'static str;
   pub struct CommandDef { pub id: CommandId, pub label: &'static str, pub default: Option<KeyBinding> }
   pub static REGISTRY: &[CommandDef] = &[ ... ];
   ```
   Enumerate every distinct action currently in `app.rs` (undo, redo, group, ungroup, z-order, delete, duplicate, align, etc.) plus tool activations.

2. **`KeyBinding`** type: `{ key: egui::Key, modifiers: egui::Modifiers }`. Serializes as `"ctrl+shift+g"` string for the prefs file.

3. **`KeyMap`** in `AppPreferences` (`preferences.rs`): add `pub keymap: HashMap<String, KeyBinding>` (keyed by `CommandId`). `Default` omits it (all defaults apply). `AppPreferences::resolve_binding(id) -> Option<KeyBinding>` checks the user map first, then the registry default.

4. **Dispatch refactor in `app.rs`**: replace the inline `ui.input(|i| i.key_pressed(egui::Key::G) && i.modifiers.ctrl)` checks with `self.action_just_pressed(ui, "group")` calls that consult the `KeyMap`. This is a mechanical but large refactor — do it incrementally: wrap existing hardcoded checks in a helper first, then migrate one block at a time.

5. **Command palette** — new `crates/photonic-gui/src/command_palette.rs`:
   - `CommandPalette { open: bool, query: String, filtered: Vec<&'static CommandDef>, selected_idx: usize }`
   - Added to `App` as `pub palette: CommandPalette`
   - Opened by a global Ctrl+K check (before `viewport_kb` guard — the palette itself is the focus)
   - Rendered as an `egui::Window` (modal-style, centered, dimmed backdrop via `egui::Area`)
   - Fuzzy match: simple `query.chars()` subsequence check over `label` strings; ranked by match quality
   - Execute: call `self.dispatch_command(id, doc, history)` on Enter

6. **Keyboard Shortcuts settings page**: a new panel tab or settings modal listing `REGISTRY` entries with an inline `egui::TextEdit` for each binding. On edit, validate (no conflict) and update `self.prefs.keymap`.

7. **Persist**: `AppPreferences` already serializes to disk via `serde`; `KeyMap` round-trips as a JSON object automatically.

## Affected Modules

- `crates/photonic-gui/src/commands.rs` — new: `CommandDef`, `CommandId`, `REGISTRY`, `KeyBinding`
- `crates/photonic-gui/src/command_palette.rs` — new: `CommandPalette`, fuzzy filter, render
- `crates/photonic-gui/src/preferences.rs` — `AppPreferences`: add `keymap: HashMap<String, KeyBinding>`
- `crates/photonic-gui/src/app.rs` — `App`: add `palette: CommandPalette`; `action_just_pressed()` helper; incremental migration of all hardcoded key checks; global Ctrl+K handler
- `crates/photonic-gui/src/panels/` — settings panel: Keyboard Shortcuts tab

## Risks & Open Questions

- **Scale of refactor**: there are >20 hardcoded key checks spread across multiple tool handler methods in `app.rs`. Full migration is large. Suggest shipping the palette and new-binding infrastructure first, then migrating existing checks incrementally behind the helper without breaking them.
- **`viewport_kb` guard** (`app.rs`: `fn viewport_kb`): currently suppresses all shortcuts when any text widget has focus. The command palette needs to intercept Ctrl+K before that guard applies, but its own text field must also not trigger tool shortcuts.
- **`PanelAction` coverage**: some operations fire via `PanelAction` enum and are not in the viewport key handler. These need a dispatch path in `dispatch_command()` to be reachable from the palette.
- **Conflict with egui default bindings**: egui intercepts some keys (e.g. Tab, Escape) internally. Test that remapping them does not create conflicts.

## Acceptance Criteria

- [ ] Every previously hardcoded shortcut is registered in `REGISTRY` with its default binding
- [ ] User can remap a shortcut in the settings page and it persists across sessions
- [ ] Conflict warning appears when a binding shadows another command
- [ ] Ctrl+K opens the palette; typing filters commands fuzzy; Enter executes the selected command
- [ ] Keymap can be exported to and imported from a JSON file
- [ ] All existing shortcuts continue to work by default after the migration

## Effort Estimate

**XL** — the registry + palette UI is M, but the full incremental refactor of hardcoded key handling across `app.rs` is large and risk-prone.
