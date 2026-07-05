use super::*;


/// Draw a compact fill editor for a path node's fill.
/// Returns `Some(new_fill)` if the user changed anything.
/// Sets `*dropper` to the chosen slot when the eyedropper button is clicked.
pub(crate) fn draw_fill_editor(ui: &mut Ui, fill: &Fill, dropper: &mut Option<FillColorSlot>) -> Option<Fill> {
    use photonic_core::style::FillKind;

    let current_type = match &fill.kind {
        FillKind::None | FillKind::Solid(_) => FillType::Solid,
        FillKind::Gradient(g) => match g.kind {
            GradientKind::Linear => FillType::Linear,
            GradientKind::Radial => FillType::Radial,
        },
        FillKind::FluidGradient(_) => FillType::Fluid,
        FillKind::MeshGradient(_) => FillType::Mesh,
        FillKind::Pattern(_) => FillType::Pattern,
    };

    let mut chosen_type = current_type;
    let mut changed = false;

    // Fill type selector
    ui.horizontal(|ui| {
        for (label, t) in [
            ("Solid", FillType::Solid),
            ("Linear", FillType::Linear),
            ("Radial", FillType::Radial),
            ("Fluid", FillType::Fluid),
            ("Mesh", FillType::Mesh),
            ("Pattern", FillType::Pattern),
        ] {
            if ui.selectable_label(chosen_type == t, label).clicked() {
                chosen_type = t;
                changed = true;
            }
        }
    });

    // If type changed, build a default fill for the new type
    if changed && chosen_type != current_type {
        // Inherit the first colour from the current fill where possible
        let base_color = match &fill.kind {
            FillKind::Solid(c) => *c,
            FillKind::Gradient(g) => g.stops.first().map(|s| s.color).unwrap_or(Color::BLACK),
            FillKind::FluidGradient(fg) => {
                fg.points.first().map(|p| p.color).unwrap_or(Color::BLACK)
            }
            FillKind::MeshGradient(mg) => {
                mg.vertices.first().map(|v| v.color).unwrap_or(Color::BLACK)
            }
            FillKind::Pattern(_) => Color::BLACK,
            FillKind::None => Color::BLACK,
        };
        let white = Color {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 1.0,
        };
        let new_fill = match chosen_type {
            FillType::Solid => Fill::solid(base_color),
            FillType::Linear => Fill::gradient(Gradient::linear(
                0.0,
                0.0,
                200.0,
                0.0,
                vec![
                    GradientStop::new(0.0, base_color),
                    GradientStop::new(1.0, white),
                ],
            )),
            FillType::Radial => Fill::gradient(Gradient::radial(
                100.0,
                100.0,
                100.0,
                vec![
                    GradientStop::new(0.0, base_color),
                    GradientStop::new(1.0, white),
                ],
            )),
            FillType::Fluid => Fill::fluid_gradient(FluidGradient::new(vec![
                FluidGradientPoint::new(50.0, 50.0, base_color),
                FluidGradientPoint::new(150.0, 50.0, white),
                FluidGradientPoint::new(
                    100.0,
                    150.0,
                    Color {
                        r: 1.0,
                        g: 0.5,
                        b: 0.0,
                        a: 1.0,
                    },
                ),
            ])),
            FillType::Mesh => {
                let r = Color {
                    r: 1.0,
                    g: 0.2,
                    b: 0.2,
                    a: 1.0,
                };
                let g = Color {
                    r: 0.2,
                    g: 1.0,
                    b: 0.2,
                    a: 1.0,
                };
                let b = Color {
                    r: 0.2,
                    g: 0.2,
                    b: 1.0,
                    a: 1.0,
                };
                Fill::mesh_gradient(MeshGradient::new(
                    2,
                    2,
                    vec![
                        MeshGradientVertex::new(0.0, 0.0, base_color),
                        MeshGradientVertex::new(200.0, 0.0, r),
                        MeshGradientVertex::new(0.0, 200.0, g),
                        MeshGradientVertex::new(200.0, 200.0, b),
                    ],
                ))
            }
            FillType::Pattern => Fill::pattern(PatternFill::new(default_checker_tile())),
        };
        return Some(new_fill);
    }

    // ── Type-specific controls ────────────────────────────────────────────
    match &fill.kind {
        FillKind::None => {
            ui.label(RichText::new("(no fill)").weak().small());
        }
        FillKind::Solid(col) => {
            // `Color` is stored as gamma-encoded sRGB, so drive the shared
            // sRGBA picker — which interprets bytes as gamma sRGB — rather than
            // the linear `Rgba` picker (issue #185).
            let mut new_col = *col;
            let mut changed = false;
            ui.horizontal(|ui| {
                if ColorPopup::swatch_color(ui, &mut new_col).changed() {
                    changed = true;
                }
                if eyedropper_btn(ui) {
                    *dropper = Some(FillColorSlot::Solid);
                }
            });
            if changed {
                return Some(Fill::solid(new_col));
            }
        }

        FillKind::Gradient(g) => {
            let mut new_g = g.clone();
            let mut grad_changed = false;

            // Coordinate inputs
            match g.kind {
                GradientKind::Linear => {
                    if g.coords.len() >= 4 {
                        ui.label(RichText::new("Start / End").small().weak());
                        ui.horizontal(|ui| {
                            let mut x0 = g.coords[0] as f32;
                            let mut y0 = g.coords[1] as f32;
                            if ui
                                .add(egui::DragValue::new(&mut x0).prefix("x0: ").speed(1.0))
                                .changed()
                            {
                                new_g.coords[0] = x0 as f64;
                                grad_changed = true;
                            }
                            if ui
                                .add(egui::DragValue::new(&mut y0).prefix("y0: ").speed(1.0))
                                .changed()
                            {
                                new_g.coords[1] = y0 as f64;
                                grad_changed = true;
                            }
                        });
                        ui.horizontal(|ui| {
                            let mut x1 = g.coords[2] as f32;
                            let mut y1 = g.coords[3] as f32;
                            if ui
                                .add(egui::DragValue::new(&mut x1).prefix("x1: ").speed(1.0))
                                .changed()
                            {
                                new_g.coords[2] = x1 as f64;
                                grad_changed = true;
                            }
                            if ui
                                .add(egui::DragValue::new(&mut y1).prefix("y1: ").speed(1.0))
                                .changed()
                            {
                                new_g.coords[3] = y1 as f64;
                                grad_changed = true;
                            }
                        });
                    }
                }
                GradientKind::Radial => {
                    if g.coords.len() >= 5 {
                        ui.label(RichText::new("Center / Radius").small().weak());
                        ui.horizontal(|ui| {
                            let mut cx = g.coords[0] as f32;
                            let mut cy = g.coords[1] as f32;
                            if ui
                                .add(egui::DragValue::new(&mut cx).prefix("cx: ").speed(1.0))
                                .changed()
                            {
                                new_g.coords[0] = cx as f64;
                                new_g.coords[2] = cx as f64;
                                grad_changed = true;
                            }
                            if ui
                                .add(egui::DragValue::new(&mut cy).prefix("cy: ").speed(1.0))
                                .changed()
                            {
                                new_g.coords[1] = cy as f64;
                                new_g.coords[3] = cy as f64;
                                grad_changed = true;
                            }
                        });
                        let mut r = g.coords[4] as f32;
                        if ui
                            .add(
                                egui::DragValue::new(&mut r)
                                    .prefix("r: ")
                                    .speed(1.0)
                                    .range(1.0..=10000.0),
                            )
                            .changed()
                        {
                            new_g.coords[4] = r as f64;
                            grad_changed = true;
                        }
                    }
                }
            }

            // Stop editor
            ui.label(RichText::new("Stops").small().weak());
            let mut stop_changed = false;
            let mut remove_idx: Option<usize> = None;
            let stop_count = new_g.stops.len();
            for i in 0..stop_count {
                let mut off = new_g.stops[i].offset;
                let can_remove = stop_count > 2;
                ui.horizontal(|ui| {
                    // Gamma-sRGB `Color` → shared sRGBA picker (issue #185).
                    if ColorPopup::swatch_color(ui, &mut new_g.stops[i].color).changed() {
                        stop_changed = true;
                    }
                    if eyedropper_btn(ui) {
                        *dropper = Some(FillColorSlot::GradientStop(i));
                    }
                    if ui
                        .add(egui::DragValue::new(&mut off).speed(0.01).range(0.0..=1.0))
                        .changed()
                    {
                        stop_changed = true;
                    }
                    if can_remove && ui.small_button(ph::X).clicked() {
                        remove_idx = Some(i);
                    }
                });
                if stop_changed {
                    new_g.stops[i].offset = off;
                }
            }
            if let Some(idx) = remove_idx {
                new_g.stops.remove(idx);
                stop_changed = true;
            }
            if ui.small_button("+ Stop").clicked() {
                let off = new_g
                    .stops
                    .last()
                    .map(|s| (s.offset + 1.0) / 2.0)
                    .unwrap_or(1.0);
                new_g.stops.push(GradientStop::new(off, Color::WHITE));
                stop_changed = true;
            }

            if grad_changed || stop_changed {
                let mut new_fill = fill.clone();
                new_fill.kind = photonic_core::style::FillKind::Gradient(new_g);
                return Some(new_fill);
            }
        }

        FillKind::FluidGradient(fg) => {
            let mut new_fg = fg.clone();
            let mut fg_changed = false;
            let mut remove_idx: Option<usize> = None;

            ui.label(RichText::new("Control Points").small().weak());
            let pt_count = new_fg.points.len();
            for i in 0..pt_count {
                let mut x = new_fg.points[i].x as f32;
                let mut y = new_fg.points[i].y as f32;
                let can_remove = pt_count > 1;
                let mut pt_changed = false;
                ui.horizontal(|ui| {
                    // Gamma-sRGB `Color` → shared sRGBA picker (issue #185).
                    if ColorPopup::swatch_color(ui, &mut new_fg.points[i].color).changed() {
                        pt_changed = true;
                    }
                    if eyedropper_btn(ui) {
                        *dropper = Some(FillColorSlot::FluidPoint(i));
                    }
                    if ui
                        .add(egui::DragValue::new(&mut x).prefix("x:").speed(1.0))
                        .changed()
                    {
                        pt_changed = true;
                    }
                    if ui
                        .add(egui::DragValue::new(&mut y).prefix("y:").speed(1.0))
                        .changed()
                    {
                        pt_changed = true;
                    }
                    if can_remove && ui.small_button(ph::X).clicked() {
                        remove_idx = Some(i);
                    }
                });
                if pt_changed {
                    new_fg.points[i].x = x as f64;
                    new_fg.points[i].y = y as f64;
                    fg_changed = true;
                }
            }
            if let Some(idx) = remove_idx {
                new_fg.points.remove(idx);
                fg_changed = true;
            }
            if ui.small_button("+ Point").clicked() {
                new_fg
                    .points
                    .push(FluidGradientPoint::new(100.0, 100.0, Color::WHITE));
                fg_changed = true;
            }
            ui.horizontal(|ui| {
                ui.label(RichText::new("Power:").small());
                let mut p = new_fg.power;
                if ui
                    .add(egui::DragValue::new(&mut p).speed(0.1).range(0.5..=8.0))
                    .changed()
                {
                    new_fg.power = p;
                    fg_changed = true;
                }
            });

            if fg_changed {
                let mut new_fill = fill.clone();
                new_fill.kind = photonic_core::style::FillKind::FluidGradient(new_fg);
                return Some(new_fill);
            }
        }

        FillKind::MeshGradient(mg) => {
            let mut new_mg = mg.clone();
            let mut mg_changed = false;
            let mut mesh_drop_idx: Option<usize> = None;

            ui.label(
                RichText::new(format!("Grid {}×{}", mg.rows, mg.cols))
                    .small()
                    .weak(),
            );
            egui::ScrollArea::vertical()
                .id_salt("mesh_grad_scroll")
                .max_height(180.0)
                .show(ui, |ui| {
                    for row in 0..new_mg.rows {
                        ui.horizontal(|ui| {
                            for col in 0..new_mg.cols {
                                let idx = (row * new_mg.cols + col) as usize;
                                if let Some(v) = new_mg.vertices.get_mut(idx) {
                                    // Gamma-sRGB `Color` → shared sRGBA picker (issue #185).
                                    if ColorPopup::swatch_color(ui, &mut v.color)
                                        .on_hover_text(format!("({},{})", row, col))
                                        .changed()
                                    {
                                        mg_changed = true;
                                    }
                                    if eyedropper_btn(ui) {
                                        mesh_drop_idx = Some(idx);
                                    }
                                }
                            }
                        });
                    }
                });
            if let Some(idx) = mesh_drop_idx {
                *dropper = Some(FillColorSlot::MeshVertex(idx));
            }

            // Grid resize buttons
            ui.horizontal(|ui| {
                if new_mg.rows < 8 && ui.small_button("+ Row").clicked() {
                    let new_row: Vec<MeshGradientVertex> = (0..new_mg.cols)
                        .map(|c| {
                            let x = new_mg
                                .vertices
                                .get(((new_mg.rows - 1) * new_mg.cols + c) as usize)
                                .map(|v| v.x)
                                .unwrap_or(0.0);
                            let prev_y = new_mg
                                .vertices
                                .get(((new_mg.rows - 1) * new_mg.cols + c) as usize)
                                .map(|v| v.y)
                                .unwrap_or(0.0);
                            MeshGradientVertex::new(x, prev_y + 50.0, Color::WHITE)
                        })
                        .collect();
                    new_mg.rows += 1;
                    new_mg.vertices.extend(new_row);
                    mg_changed = true;
                }
                if new_mg.cols < 8 && ui.small_button("+ Col").clicked() {
                    let old_cols = new_mg.cols;
                    new_mg.cols += 1;
                    // Insert a new vertex at the end of each row
                    let mut new_verts: Vec<MeshGradientVertex> = Vec::new();
                    for r in 0..new_mg.rows {
                        for c in 0..old_cols {
                            let v = new_mg.vertices[(r * old_cols + c) as usize].clone();
                            new_verts.push(v);
                        }
                        let prev_x = new_mg.vertices[((r + 1) * old_cols - 1) as usize].x;
                        let prev_y = new_mg.vertices[((r + 1) * old_cols - 1) as usize].y;
                        new_verts.push(MeshGradientVertex::new(
                            prev_x + 50.0,
                            prev_y,
                            Color::WHITE,
                        ));
                    }
                    new_mg.vertices = new_verts;
                    mg_changed = true;
                }
            });

            if mg_changed {
                let mut new_fill = fill.clone();
                new_fill.kind = photonic_core::style::FillKind::MeshGradient(new_mg);
                return Some(new_fill);
            }
        }

        FillKind::Pattern(p) => {
            use photonic_core::style::PatternTileType;
            let mut new_p = p.clone();
            let mut pat_changed = false;

            ui.label(
                RichText::new(format!("Tile {}×{}px", p.tile.width, p.tile.height))
                    .small()
                    .weak(),
            );

            // Tile layout.
            ui.horizontal(|ui| {
                ui.label(RichText::new("Layout").small().weak());
                for (label, t) in [
                    ("Grid", PatternTileType::Grid),
                    ("Brick", PatternTileType::BrickByRow),
                    ("Brick↕", PatternTileType::BrickByColumn),
                    ("Hex", PatternTileType::Hex),
                ] {
                    if ui.selectable_label(new_p.tile_type == t, label).clicked() {
                        new_p.tile_type = t;
                        pat_changed = true;
                    }
                }
            });

            // Scale.
            let mut scale = new_p.scale as f32;
            if ui
                .add(
                    egui::Slider::new(&mut scale, 0.05..=8.0)
                        .text("Scale")
                        .logarithmic(true),
                )
                .changed()
            {
                new_p.scale = scale as f64;
                pat_changed = true;
            }

            // Rotation (degrees in UI, radians in model).
            let mut rot_deg = new_p.rotation.to_degrees() as f32;
            if ui
                .add(egui::Slider::new(&mut rot_deg, -180.0..=180.0).text("Rotate°"))
                .changed()
            {
                new_p.rotation = (rot_deg as f64).to_radians();
                pat_changed = true;
            }

            // Offset.
            ui.horizontal(|ui| {
                let mut ox = new_p.offset[0] as f32;
                let mut oy = new_p.offset[1] as f32;
                if ui
                    .add(egui::DragValue::new(&mut ox).prefix("x: ").speed(1.0))
                    .changed()
                {
                    new_p.offset[0] = ox as f64;
                    pat_changed = true;
                }
                if ui
                    .add(egui::DragValue::new(&mut oy).prefix("y: ").speed(1.0))
                    .changed()
                {
                    new_p.offset[1] = oy as f64;
                    pat_changed = true;
                }
            });

            // Spacing (gutter).
            let mut spacing = new_p.spacing as f32;
            if ui
                .add(egui::Slider::new(&mut spacing, 0.0..=64.0).text("Spacing"))
                .changed()
            {
                new_p.spacing = spacing as f64;
                pat_changed = true;
            }

            if pat_changed {
                let mut new_fill = fill.clone();
                new_fill.kind = photonic_core::style::FillKind::Pattern(new_p);
                return Some(new_fill);
            }
        }
    }

    None
}


/// A small two-tone checkerboard tile used as the default when a user switches a
/// fill to `Pattern` in the inspector (gives an immediately visible pattern).
fn default_checker_tile() -> photonic_core::RasterImage {
    let n = 32u32; // tile is 2×2 cells of 16px
    let cell = 16u32;
    let mut img = photonic_core::RasterImage::new(n, n);
    for y in 0..n {
        for x in 0..n {
            let on = ((x / cell) + (y / cell)) % 2 == 0;
            let rgba = if on {
                [40, 40, 48, 255]
            } else {
                [220, 220, 230, 255]
            };
            img.set_pixel(x, y, rgba);
        }
    }
    img
}




/// Draw a compact stroke editor. Returns `Some(new_stroke)` if the user changed anything.
/// Sets `*dropper` to `true` when the eyedropper button is clicked.
pub(crate) fn draw_stroke_editor(ui: &mut Ui, stroke: &Stroke, dropper: &mut bool) -> Option<Stroke> {
    use photonic_core::style::{LineCap, LineJoin, StrokeAlign};

    let mut new_stroke = stroke.clone();
    let mut changed = false;

    // Enable / disable toggle
    let mut enabled = new_stroke.enabled;
    if ui.checkbox(&mut enabled, "Enabled").changed() {
        new_stroke.enabled = enabled;
        changed = true;
    }

    if new_stroke.enabled {
        // Color — gamma-sRGB `Color` → shared sRGBA picker (issue #185).
        ui.horizontal(|ui| {
            if ColorPopup::swatch_color(ui, &mut new_stroke.color).changed() {
                changed = true;
            }
            if eyedropper_btn(ui) {
                *dropper = true;
            }
        });

        // Width
        ui.horizontal(|ui| {
            ui.label("Width");
            let mut w = new_stroke.width as f32;
            if ui
                .add(egui::DragValue::new(&mut w).range(0.0..=500.0).speed(0.5))
                .changed()
            {
                new_stroke.width = w as f64;
                changed = true;
            }
        });

        // Opacity
        ui.horizontal(|ui| {
            ui.label("Opacity");
            let mut op = new_stroke.opacity;
            if ui.add(egui::Slider::new(&mut op, 0.0..=1.0)).changed() {
                new_stroke.opacity = op;
                changed = true;
            }
        });

        // Line cap
        ui.horizontal(|ui| {
            ui.label("Cap");
            for (label, cap) in [
                ("Butt", LineCap::Butt),
                ("Round", LineCap::Round),
                ("Square", LineCap::Square),
            ] {
                if ui
                    .selectable_label(new_stroke.line_cap == cap, label)
                    .clicked()
                {
                    new_stroke.line_cap = cap;
                    changed = true;
                }
            }
        });

        // Line join
        ui.horizontal(|ui| {
            ui.label("Join");
            for (label, join) in [
                ("Miter", LineJoin::Miter),
                ("Round", LineJoin::Round),
                ("Bevel", LineJoin::Bevel),
            ] {
                if ui
                    .selectable_label(new_stroke.line_join == join, label)
                    .clicked()
                {
                    new_stroke.line_join = join;
                    changed = true;
                }
            }
        });

        // Stroke alignment
        ui.horizontal(|ui| {
            ui.label("Align");
            for (label, align) in [
                ("Center", StrokeAlign::Center),
                ("Inside", StrokeAlign::Inside),
                ("Outside", StrokeAlign::Outside),
            ] {
                if ui
                    .selectable_label(new_stroke.align == align, label)
                    .clicked()
                {
                    new_stroke.align = align;
                    changed = true;
                }
            }
        });

        // Dash controls
        let mut dashes_on = !new_stroke.dash_array.is_empty();
        if ui.checkbox(&mut dashes_on, "Dashed").changed() {
            if dashes_on {
                new_stroke.dash_array = vec![8.0, 4.0];
            } else {
                new_stroke.dash_array.clear();
            }
            changed = true;
        }
        if dashes_on {
            // Ensure pairs up to 3 (6 values); pad if needed for the UI.
            while new_stroke.dash_array.len() < 2 {
                new_stroke.dash_array.push(4.0);
            }
            ui.label(RichText::new("Dash / Gap pairs (up to 3):").weak().small());
            let pair_count = (new_stroke.dash_array.len() / 2).max(1).min(3);
            for i in 0..pair_count {
                let dash_idx = i * 2;
                let gap_idx = i * 2 + 1;
                ui.horizontal(|ui| {
                    ui.label(format!("Pair {}:", i + 1));
                    let mut dash_val =
                        new_stroke.dash_array.get(dash_idx).copied().unwrap_or(8.0) as f32;
                    let mut gap_val =
                        new_stroke.dash_array.get(gap_idx).copied().unwrap_or(4.0) as f32;
                    if ui
                        .add(
                            egui::DragValue::new(&mut dash_val)
                                .range(0.5..=500.0)
                                .speed(0.5)
                                .prefix("—"),
                        )
                        .changed()
                    {
                        if new_stroke.dash_array.len() <= dash_idx {
                            new_stroke.dash_array.resize(dash_idx + 1, 0.0);
                        }
                        new_stroke.dash_array[dash_idx] = dash_val as f64;
                        changed = true;
                    }
                    ui.label("·");
                    if ui
                        .add(
                            egui::DragValue::new(&mut gap_val)
                                .range(0.0..=500.0)
                                .speed(0.5),
                        )
                        .changed()
                    {
                        if new_stroke.dash_array.len() <= gap_idx {
                            new_stroke.dash_array.resize(gap_idx + 1, 0.0);
                        }
                        new_stroke.dash_array[gap_idx] = gap_val as f64;
                        changed = true;
                    }
                });
            }
            // Add/remove pair buttons
            ui.horizontal(|ui| {
                if pair_count < 3 {
                    if ui
                        .small_button("+ Pair")
                        .on_hover_text("Add a dash/gap pair")
                        .clicked()
                    {
                        new_stroke.dash_array.extend_from_slice(&[8.0, 4.0]);
                        changed = true;
                    }
                }
                if pair_count > 1 {
                    if ui
                        .small_button("− Pair")
                        .on_hover_text("Remove the last dash/gap pair")
                        .clicked()
                    {
                        new_stroke
                            .dash_array
                            .truncate(new_stroke.dash_array.len().saturating_sub(2));
                        changed = true;
                    }
                }
            });
            // Dash offset
            ui.horizontal(|ui| {
                ui.label("Offset:");
                let mut offset = new_stroke.dash_offset as f32;
                if ui
                    .add(egui::DragValue::new(&mut offset).speed(0.5))
                    .changed()
                {
                    new_stroke.dash_offset = offset as f64;
                    changed = true;
                }
            });
            // Align dashes to corners
            let mut align_corners = new_stroke.dash_corner_alignment;
            if ui.checkbox(&mut align_corners, "Align to corners")
                .on_hover_text("Adjust dash spacing so dashes start and end cleanly at path corners and endpoints")
                .changed()
            {
                new_stroke.dash_corner_alignment = align_corners;
                changed = true;
            }
        }
    }

    // ── Arrowheads ──────────────────────────────────────────────────────────
    {
        use photonic_core::style::ArrowheadStyle;
        let arrow_label = |s: ArrowheadStyle| match s {
            ArrowheadStyle::None => "None",
            ArrowheadStyle::FilledArrow => "Filled",
            ArrowheadStyle::OpenArrow => "Open",
        };
        ui.horizontal(|ui| {
            ui.label("Arrow start");
            egui::ComboBox::new("arrow_start", "")
                .selected_text(arrow_label(new_stroke.arrowhead_start))
                .show_ui(ui, |ui| {
                    for style in [
                        ArrowheadStyle::None,
                        ArrowheadStyle::FilledArrow,
                        ArrowheadStyle::OpenArrow,
                    ] {
                        if ui
                            .selectable_label(
                                new_stroke.arrowhead_start == style,
                                arrow_label(style),
                            )
                            .clicked()
                        {
                            new_stroke.arrowhead_start = style;
                            changed = true;
                        }
                    }
                });
        });
        ui.horizontal(|ui| {
            ui.label("Arrow end");
            egui::ComboBox::new("arrow_end", "")
                .selected_text(arrow_label(new_stroke.arrowhead_end))
                .show_ui(ui, |ui| {
                    for style in [
                        ArrowheadStyle::None,
                        ArrowheadStyle::FilledArrow,
                        ArrowheadStyle::OpenArrow,
                    ] {
                        if ui
                            .selectable_label(new_stroke.arrowhead_end == style, arrow_label(style))
                            .clicked()
                        {
                            new_stroke.arrowhead_end = style;
                            changed = true;
                        }
                    }
                });
        });
    }

    if changed {
        Some(new_stroke)
    } else {
        None
    }
}



/// Renders a compact editor for a `GlowEffect`. Returns `Some(updated)` on any change.
/// Sets `*dropper` to `true` when the eyedropper button is clicked.
pub(crate) fn draw_glow_editor(ui: &mut Ui, glow: &GlowEffect, dropper: &mut bool) -> Option<GlowEffect> {
    let mut new_glow = glow.clone();
    let mut changed = false;

    ui.horizontal(|ui| {
        if ui.checkbox(&mut new_glow.enabled, "Enabled").changed() {
            changed = true;
        }
    });

    if new_glow.enabled {
        ui.horizontal(|ui| {
            ui.label("Color");
            // Gamma-sRGB `Color` → shared sRGBA picker (issue #185).
            if ColorPopup::swatch_color(ui, &mut new_glow.color).changed() {
                changed = true;
            }
            if eyedropper_btn(ui) {
                *dropper = true;
            }
        });
        ui.horizontal(|ui| {
            ui.label("Opacity");
            if ui
                .add(egui::Slider::new(&mut new_glow.opacity, 0.0..=1.0))
                .changed()
            {
                changed = true;
            }
        });
        ui.horizontal(|ui| {
            ui.label("Size");
            if ui
                .add(egui::Slider::new(&mut new_glow.size, 1.0..=100.0).suffix("px"))
                .changed()
            {
                changed = true;
            }
        });
        ui.horizontal(|ui| {
            ui.label("Corners");
            let options = [
                (LineJoin::Miter, "Miter"),
                (LineJoin::Round, "Round"),
                (LineJoin::Bevel, "Bevel"),
            ];
            for (variant, label) in options {
                if ui
                    .selectable_label(new_glow.join == variant, label)
                    .clicked()
                {
                    new_glow.join = variant;
                    changed = true;
                }
            }
        });
    }

    if changed {
        Some(new_glow)
    } else {
        None
    }
}



/// Renders a compact editor for a `GaussianGlow`. Returns `Some(updated)` on any change.
/// Sets `*dropper` to `true` when the eyedropper button is clicked.
pub(crate) fn draw_gaussian_glow_editor(
    ui: &mut Ui,
    glow: &GaussianGlow,
    dropper: &mut bool,
) -> Option<GaussianGlow> {
    let mut new_glow = glow.clone();
    let mut changed = false;

    ui.horizontal(|ui| {
        if ui.checkbox(&mut new_glow.enabled, "Enabled").changed() {
            changed = true;
        }
    });

    if new_glow.enabled {
        ui.horizontal(|ui| {
            ui.label("Color");
            // Gamma-sRGB `Color` → shared sRGBA picker (issue #185).
            if ColorPopup::swatch_color(ui, &mut new_glow.color).changed() {
                changed = true;
            }
            if eyedropper_btn(ui) {
                *dropper = true;
            }
        });
        ui.horizontal(|ui| {
            ui.label("Opacity");
            if ui
                .add(egui::Slider::new(&mut new_glow.opacity, 0.0..=1.0))
                .changed()
            {
                changed = true;
            }
        });
        ui.horizontal(|ui| {
            ui.label("Radius");
            if ui
                .add(egui::Slider::new(&mut new_glow.radius, 1.0..=200.0).suffix("px"))
                .changed()
            {
                changed = true;
            }
        });
    }

    if changed {
        Some(new_glow)
    } else {
        None
    }
}

