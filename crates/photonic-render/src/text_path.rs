//! Text-on-path layout: shape a text run, extract each glyph's outline, and
//! place + orient it along a spine path.
//!
//! glyphon (cosmic-text 0.12 / glyphon 0.6) can only draw axis-aligned glyph
//! quads, so it cannot rotate glyphs to follow a curve. Instead we extract glyph
//! outlines with `ttf-parser` and emit them as ordinary vector fills, which the
//! existing fill pipeline already renders with arbitrary transforms.

use glyphon::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, Style as GlyphonStyle, Weight};
use kurbo::{Affine, BezPath};
use photonic_core::node::{FontStyle, TextAlign};
use photonic_core::path::PathData;
use ttf_parser::{GlyphId, OutlineBuilder};

/// Parameters describing the text run to lay out along a path. All sizes are in
/// document units (the spine is in the same space).
pub struct TextOnPathParams<'a> {
    pub content: &'a str,
    pub font_family: &'a str,
    pub font_size: f64,
    pub font_weight: u16,
    pub font_style: FontStyle,
    pub line_height: f64,
    pub letter_spacing: f64,
    pub align: TextAlign,
    pub path_offset: f64,
}

/// Accumulates a `ttf-parser` glyph outline (font units, Y-up) into a kurbo
/// `BezPath`. The caller applies the font-unit → document-space affine.
#[derive(Default)]
struct BezOutlineBuilder {
    path: BezPath,
}

impl OutlineBuilder for BezOutlineBuilder {
    fn move_to(&mut self, x: f32, y: f32) {
        self.path.move_to((x as f64, y as f64));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.path.line_to((x as f64, y as f64));
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.path
            .quad_to((x1 as f64, y1 as f64), (x as f64, y as f64));
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.path.curve_to(
            (x1 as f64, y1 as f64),
            (x2 as f64, y2 as f64),
            (x as f64, y as f64),
        );
    }
    fn close(&mut self) {
        self.path.close_path();
    }
}

/// Shape `params.content`, then place each glyph's outline along `spine`,
/// returning one document-space `PathData` per rendered glyph (in run order).
///
/// Glyphs whose centre falls off the path, or whose font/outline can't be
/// resolved (e.g. spaces, bitmap-only fonts), are skipped.
pub fn layout_text_on_path(
    font_system: &mut FontSystem,
    params: &TextOnPathParams,
    spine: &PathData,
) -> Vec<PathData> {
    let font_size = params.font_size.max(0.01) as f32;
    let line_height = font_size * params.line_height.max(0.1) as f32;

    let mut buf = Buffer::new(font_system, Metrics::new(font_size, line_height));
    buf.set_size(font_system, None, None);
    let glyph_style = match params.font_style {
        FontStyle::Italic => GlyphonStyle::Italic,
        FontStyle::Oblique => GlyphonStyle::Oblique,
        FontStyle::Normal => GlyphonStyle::Normal,
    };
    let attrs = Attrs::new()
        .family(Family::Name(params.font_family))
        .weight(Weight(params.font_weight))
        .style(glyph_style);
    buf.set_text(font_system, params.content, attrs, Shaping::Advanced);
    buf.shape_until_scroll(font_system, false);

    // Flatten the (single-line) run into placed glyphs with their along-line x.
    struct Shaped {
        font_id: glyphon::cosmic_text::fontdb::ID,
        glyph_id: u16,
        x: f64,
        w: f64,
        size: f64,
    }
    let mut shaped: Vec<Shaped> = Vec::new();
    for run in buf.layout_runs() {
        for g in run.glyphs.iter() {
            shaped.push(Shaped {
                font_id: g.font_id,
                glyph_id: g.glyph_id,
                x: g.x as f64,
                w: g.w as f64,
                size: g.font_size as f64,
            });
        }
    }
    if shaped.is_empty() {
        return Vec::new();
    }

    // Run width along the baseline (used for centre/right alignment).
    let total = shaped.iter().map(|s| s.x + s.w).fold(0.0_f64, f64::max);
    let spine_len = spine.arc_length();
    let start = match params.align {
        TextAlign::Left => params.path_offset,
        TextAlign::Center => params.path_offset + (spine_len - total) / 2.0,
        TextAlign::Right => params.path_offset + (spine_len - total),
    };

    let mut out = Vec::with_capacity(shaped.len());
    for (i, s) in shaped.iter().enumerate() {
        // Centre of this glyph along the baseline → arc length on the spine.
        let s_mid = start + s.x + params.letter_spacing * i as f64 + s.w / 2.0;
        let Some((cx, cy, angle)) = spine.sample_at_arc_length(s_mid) else {
            continue;
        };

        let Some(font) = font_system.get_font(s.font_id) else {
            continue;
        };
        let face = font.rustybuzz(); // derefs to ttf_parser::Face
        let units = face.units_per_em() as f64;
        if units <= 0.0 {
            continue;
        }
        let mut builder = BezOutlineBuilder::default();
        if face
            .outline_glyph(GlyphId(s.glyph_id), &mut builder)
            .is_none()
        {
            continue; // no outline (whitespace, bitmap glyph, …)
        }

        // font units (Y-up) → document space (Y-down), centred on the sample
        // point and rotated to the path tangent:
        //   world = T(cx,cy) · R(angle) · T(-w/2, 0) · S(scale, -scale) · local
        let scale = s.size / units;
        let affine = Affine::translate((cx, cy))
            * Affine::rotate(angle)
            * Affine::translate((-s.w / 2.0, 0.0))
            * Affine::scale_non_uniform(scale, -scale);
        let mut glyph = builder.path;
        glyph.apply_affine(affine);
        out.push(PathData::from_bez_path(&glyph));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_content_yields_no_glyphs() {
        let mut fs = FontSystem::new();
        let spine = PathData::line(0.0, 0.0, 200.0, 0.0);
        let params = TextOnPathParams {
            content: "",
            font_family: "sans-serif",
            font_size: 24.0,
            font_weight: 400,
            font_style: FontStyle::Normal,
            line_height: 1.2,
            letter_spacing: 0.0,
            align: TextAlign::Left,
            path_offset: 0.0,
        };
        assert!(layout_text_on_path(&mut fs, &params, &spine).is_empty());
    }

    #[test]
    fn glyphs_are_placed_along_the_spine() {
        // Skips cleanly on CI images with no usable fonts.
        let mut fs = FontSystem::new();
        let spine = PathData::line(0.0, 0.0, 400.0, 0.0);
        let params = TextOnPathParams {
            content: "AVA",
            font_family: "sans-serif",
            font_size: 32.0,
            font_weight: 400,
            font_style: FontStyle::Normal,
            line_height: 1.2,
            letter_spacing: 0.0,
            align: TextAlign::Left,
            path_offset: 0.0,
        };
        let glyphs = layout_text_on_path(&mut fs, &params, &spine);
        if glyphs.is_empty() {
            eprintln!("no system font available — skipping placement assertions");
            return;
        }
        // Each glyph outline should have a bounding box near the (horizontal) spine.
        for g in &glyphs {
            let bb = g.bounding_box().expect("glyph has geometry");
            assert!(bb.width() > 0.0 && bb.height() > 0.0);
            // Baseline runs along y=0; glyphs sit within a reasonable band.
            assert!(bb.min_y() > -64.0 && bb.max_y() < 32.0, "bbox={bb:?}");
        }
    }
}
