# Accessibility: keyboard navigation, screen-reader support, high-contrast/theming (#60) — Design Proposal

> Status: design scaffold (not an implementation).

## Summary

The GUI runs on egui 0.29, which ships optional AccessKit integration for screen readers.
`crates/photonic-gui/src/theme.rs` has `build_dark_theme()` and `build_light_theme()` but
no high-contrast variant and no user-facing theme picker. Keyboard navigation of panels is
unverified. This proposal defines the concrete steps to reach WCAG 2.1 AA compliance for
the application chrome and achieve keyboard + screen-reader operability for core flows.

## Scope

**In:**
- Enable egui's AccessKit feature flag so platform accessibility APIs receive widget metadata.
- High-contrast theme (light background, high-contrast text/borders) alongside the existing dark/light themes.
- Theme picker in preferences/settings, persisted in `AppPreferences`.
- UI-scale preference already exists (`AppPreferences::ui_scale: f32`); expose it in the Settings panel.
- Audit and fix missing `accessible_name`/`aria_label` equivalents on icon-only buttons (egui `.on_hover_text()` doubles as accessible label when AccessKit is enabled).
- Keyboard focus cycling through panels (F6/Shift+F6 is the conventional shortcut); visible focus ring.
- Document keyboard shortcuts (Tab, arrow keys, Enter/Space for interactive elements).

**Out:**
- Canvas drawing operations via screen reader (the canvas is fundamentally a visual surface; this is a separate, multi-year effort).
- RTL layout support (tracked under i18n, issue #61).
- WCAG compliance for third-party egui widgets we do not control.

## Proposed approach

1. **AccessKit**: In `crates/photonic-gui/Cargo.toml`, enable the `egui` feature `"accesskit"` (egui 0.29 supports this). In `crates/photonic-app/src/main.rs`, initialize `egui-winit` with AccessKit enabled via `eframe`/`egui-winit`'s `enable_accesskit` option. This wires the widget tree to AT-SPI (Linux), UIAutomation (Windows), and NSAccessibility (macOS) with no further code changes for standard widgets.

2. **High-contrast theme**: Add `build_high_contrast_theme() -> egui::Visuals` to `crates/photonic-gui/src/theme.rs`. Use `egui::Visuals::light()` as the base; override: white panel backgrounds (#FFFFFF), black text (#000000), accent borders at 3 px minimum, focused widget ring in solid yellow or blue per WCAG 1.4.11.

3. **Theme enum**: Add `pub enum ThemeChoice { Dark, Light, HighContrast }` in `crates/photonic-gui/src/preferences.rs` and add `theme: ThemeChoice` to `AppPreferences`. Apply the chosen visuals in `crates/photonic-gui/src/app.rs` at startup and on settings change.

4. **Settings panel**: Expose `theme`, `ui_scale`, and a new `reduce_motion: bool` toggle in the existing preferences UI in `app.rs`. `reduce_motion` suppresses any animated transitions (none currently, but prevents regressions as animation is added).

5. **Keyboard focus cycle**: In `crates/photonic-gui/src/app.rs`, handle `egui::Key::F6` to shift keyboard focus to the next panel (Layers → Properties → toolbar → canvas). egui's `memory().request_focus(id)` drives this. Ensure each panel's root widget has a stable `egui::Id`.

6. **Focus ring**: egui 0.29 renders a focus ring when `visuals.selection.stroke` is set with sufficient contrast. Verify the dark and high-contrast themes both set this field to a visible color (currently the dark theme may set it to the dim accent).

7. **Icon button labels**: Audit icon-only buttons in `crates/photonic-gui/src/panels/mod.rs` and `app.rs`; ensure each has `.on_hover_text("Descriptive label")` — egui exposes this as the accessible name under AccessKit.

## Affected modules

- `crates/photonic-gui/Cargo.toml` — add `accesskit` feature to `egui`
- `crates/photonic-gui/src/theme.rs` — `build_high_contrast_theme()`
- `crates/photonic-gui/src/preferences.rs` — `ThemeChoice` enum, `AppPreferences::theme`, `::reduce_motion`
- `crates/photonic-app/src/main.rs` — enable AccessKit in egui-winit init
- `crates/photonic-gui/src/app.rs` — theme application, F6 focus cycle, settings panel

## Risks & open questions

- **egui AccessKit maturity**: egui's AccessKit integration is functional but not exhaustive; custom-painted widgets (the canvas viewport) will be opaque to screen readers without manual AccessKit node registration.
- **High-contrast detection**: Windows offers a system-level high-contrast preference; could auto-apply via `winit`'s `Theme` event.
- Open Q: Should `ThemeChoice` follow the OS preference by default, or always start with Dark?
- Open Q: WCAG audit tooling — consider a manual audit checklist vs. automated tool (Accessibility Insights).

## Acceptance criteria

- [ ] AccessKit is compiled in and connected; NVDA/Orca can read panel labels and button names.
- [ ] High-contrast theme renders with ≥4.5:1 contrast ratio on text and UI elements (WCAG 1.4.3).
- [ ] F6 / Shift+F6 cycles keyboard focus through the major panels with a visible ring.
- [ ] Theme and UI scale are changeable in Settings and persist across restarts.
- [ ] No regression on mouse/touch users.

## Effort estimate

**M** — AccessKit flag + high-contrast theme are relatively small; the focus cycle and label audit across the full panel codebase require careful attention.
