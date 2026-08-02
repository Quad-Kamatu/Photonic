use super::*;

/// Draw the horizontal toolbar (logo, doc name, zoom — no tool buttons).
pub fn draw_toolbar(
    ui: &mut Ui,
    doc_name: &str,
    zoom: f64,
    file_status: Option<&str>,
    logo: Option<&egui::TextureHandle>,
) {
    ui.horizontal(|ui| {
        // Show the actual Photonic logo when loaded; fall back to a glyph.
        if let Some(tex) = logo {
            ui.add(
                egui::Image::new(egui::load::SizedTexture::new(
                    tex.id(),
                    egui::vec2(18.0, 18.0),
                ))
                .maintain_aspect_ratio(true),
            );
            ui.label(
                RichText::new("Photonic")
                    .strong()
                    .color(Color32::from_rgb(110, 86, 207)),
            );
        } else {
            ui.label(
                RichText::new(format!("{} Photonic", ph::HEXAGON))
                    .strong()
                    .color(Color32::from_rgb(110, 86, 207)),
            );
        }
        ui.separator();
        ui.label(doc_name);

        // Right-aligned cluster: zoom readout, then (optionally) the file-status
        // message. Both are rendered in the *same* right-to-left layout so they
        // stack next to each other instead of overlapping in the top-right.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // Quit menu — explicit, discoverable shutdown (the window's close
            // button works too; this also offers killing every instance).
            ui.menu_button(format!("Quit {}", ph::CARET_DOWN), |ui| {
                if ui.button("Quit Photonic").clicked() {
                    ui.close_menu();
                    crate::quit::quit_self();
                }
                ui.separator();
                if ui
                    .button("Quit ALL Photonic processes")
                    .on_hover_text(
                        "Closes every Photonic window and any headless MCP server. \
                         Unsaved changes in other windows will be lost.",
                    )
                    .clicked()
                {
                    ui.close_menu();
                    crate::quit::quit_all();
                }
            });
            ui.separator();
            ui.label(format!("{:.0}%", zoom * 100.0));
            ui.label("Zoom:");
            if let Some(status) = file_status {
                ui.separator();
                ui.label(RichText::new(status).weak().italics());
            }
        });
    });
}
