use egui::{style::WidgetVisuals, Color32, Rounding, Shadow, Stroke};

/// Build the "Deep Violet" dark theme.
pub fn build_dark_theme() -> egui::Visuals {
    let bg_base = Color32::from_rgb(7, 7, 11); // #07070B window fill
    let bg_panel = Color32::from_rgb(12, 12, 21); // #0C0C15 panel fill
    let bg_elevated = Color32::from_rgb(19, 19, 31); // #13131F hover / input bg
    let bg_widget = Color32::from_rgb(26, 26, 40); // #1A1A28 text inputs
    let border = Color32::from_rgb(30, 30, 50); // #1E1E32 panel/widget border
    let border_focus = Color32::from_rgb(110, 86, 207); // #6E56CF focused border
    let accent = Color32::from_rgb(110, 86, 207); // #6E56CF electric violet
    let accent_dim = Color32::from_rgb(61, 48, 128); // #3D3080 accent at ~40%
    let accent_light = Color32::from_rgb(144, 119, 224); // #9077E0 hover / glow
    let text_primary = Color32::from_rgb(232, 232, 242); // #E8E8F2
    let text_muted = Color32::from_rgb(122, 122, 154); // #7A7A9A secondary labels

    let _ = accent_light;

    let rounding = Rounding::same(3.0);
    let mut v = egui::Visuals::dark();

    v.window_fill = bg_base;
    v.panel_fill = bg_panel;
    v.faint_bg_color = bg_elevated;
    v.extreme_bg_color = bg_widget;
    v.code_bg_color = bg_elevated;

    v.override_text_color = Some(text_primary);

    v.window_rounding = Rounding::same(4.0);
    v.window_stroke = Stroke::new(1.0, border);
    v.window_shadow = Shadow::NONE;
    v.popup_shadow = Shadow::NONE;
    v.menu_rounding = Rounding::same(4.0);

    v.selection.bg_fill = accent_dim;
    v.selection.stroke = Stroke::new(1.0, accent);

    v.hyperlink_color = Color32::from_rgb(144, 119, 224);
    v.warn_fg_color = Color32::from_rgb(251, 191, 36);
    v.error_fg_color = Color32::from_rgb(248, 113, 113);

    v.widgets.noninteractive = WidgetVisuals {
        bg_fill: bg_panel,
        weak_bg_fill: bg_elevated,
        bg_stroke: Stroke::new(1.0, border),
        rounding,
        fg_stroke: Stroke::new(1.0, text_muted),
        expansion: 0.0,
    };
    v.widgets.inactive = WidgetVisuals {
        bg_fill: bg_elevated,
        weak_bg_fill: bg_panel,
        bg_stroke: Stroke::new(1.0, border),
        rounding,
        fg_stroke: Stroke::new(1.0, text_primary),
        expansion: 0.0,
    };
    v.widgets.hovered = WidgetVisuals {
        bg_fill: bg_widget,
        weak_bg_fill: bg_elevated,
        bg_stroke: Stroke::new(1.0, border_focus),
        rounding,
        fg_stroke: Stroke::new(1.5, text_primary),
        expansion: 1.0,
    };
    v.widgets.active = WidgetVisuals {
        bg_fill: accent_dim,
        weak_bg_fill: bg_elevated,
        bg_stroke: Stroke::new(1.0, accent),
        rounding,
        fg_stroke: Stroke::new(2.0, Color32::WHITE),
        expansion: 1.0,
    };
    v.widgets.open = WidgetVisuals {
        bg_fill: bg_elevated,
        weak_bg_fill: bg_panel,
        bg_stroke: Stroke::new(1.0, border),
        rounding,
        fg_stroke: Stroke::new(1.5, text_primary),
        expansion: 0.0,
    };

    v
}

/// Build the "Soft Lavender" light theme — much lighter purple companion.
pub fn build_light_theme() -> egui::Visuals {
    let bg_base = Color32::from_rgb(250, 249, 255); // #FAF9FF near-white with violet tint
    let bg_panel = Color32::from_rgb(243, 240, 255); // #F3F0FF soft lavender panels
    let bg_elevated = Color32::from_rgb(234, 228, 255); // #EAE4FF elevated surfaces
    let bg_widget = Color32::from_rgb(255, 255, 255); // #FFFFFF inputs
    let border = Color32::from_rgb(210, 200, 240); // #D2C8F0 soft violet border
    let border_focus = Color32::from_rgb(110, 86, 207); // #6E56CF accent border (same violet)
    let accent = Color32::from_rgb(110, 86, 207); // #6E56CF electric violet
    let accent_dim = Color32::from_rgb(210, 198, 245); // #D2C6F5 light selection fill
    let text_primary = Color32::from_rgb(25, 20, 60); // #19143C near-black with violet
    let text_muted = Color32::from_rgb(110, 100, 150); // #6E6496 muted labels

    let rounding = Rounding::same(3.0);
    let mut v = egui::Visuals::light();

    v.window_fill = bg_base;
    v.panel_fill = bg_panel;
    v.faint_bg_color = bg_elevated;
    v.extreme_bg_color = bg_widget;
    v.code_bg_color = bg_elevated;

    v.override_text_color = Some(text_primary);

    v.window_rounding = Rounding::same(4.0);
    v.window_stroke = Stroke::new(1.0, border);
    v.window_shadow = Shadow::NONE;
    v.popup_shadow = Shadow::NONE;
    v.menu_rounding = Rounding::same(4.0);

    v.selection.bg_fill = accent_dim;
    v.selection.stroke = Stroke::new(1.0, accent);

    v.hyperlink_color = Color32::from_rgb(110, 86, 207);
    v.warn_fg_color = Color32::from_rgb(180, 120, 0);
    v.error_fg_color = Color32::from_rgb(200, 60, 60);

    v.widgets.noninteractive = WidgetVisuals {
        bg_fill: bg_panel,
        weak_bg_fill: bg_elevated,
        bg_stroke: Stroke::new(1.0, border),
        rounding,
        fg_stroke: Stroke::new(1.0, text_muted),
        expansion: 0.0,
    };
    v.widgets.inactive = WidgetVisuals {
        bg_fill: bg_elevated,
        weak_bg_fill: bg_panel,
        bg_stroke: Stroke::new(1.0, border),
        rounding,
        fg_stroke: Stroke::new(1.0, text_primary),
        expansion: 0.0,
    };
    v.widgets.hovered = WidgetVisuals {
        bg_fill: bg_widget,
        weak_bg_fill: bg_elevated,
        bg_stroke: Stroke::new(1.0, border_focus),
        rounding,
        fg_stroke: Stroke::new(1.5, text_primary),
        expansion: 1.0,
    };
    v.widgets.active = WidgetVisuals {
        bg_fill: accent_dim,
        weak_bg_fill: bg_elevated,
        bg_stroke: Stroke::new(1.0, accent),
        rounding,
        fg_stroke: Stroke::new(2.0, Color32::from_rgb(25, 20, 60)),
        expansion: 1.0,
    };
    v.widgets.open = WidgetVisuals {
        bg_fill: bg_elevated,
        weak_bg_fill: bg_panel,
        bg_stroke: Stroke::new(1.0, border),
        rounding,
        fg_stroke: Stroke::new(1.5, text_primary),
        expansion: 0.0,
    };

    v
}
