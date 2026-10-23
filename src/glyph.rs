use eframe::egui::{Color32, Painter, Rect, Vec2};

/// Follow felis's fitted_bitmap policy: only an excessive advance triggers
/// fitting, and fitting rasterizes a smaller font instead of stretching a mesh.
pub fn paint_text(
    painter: &Painter,
    text: &str,
    mut font: eframe::egui::FontId,
    cell: Rect,
    color: Color32,
    bold: bool,
) {
    let mut galley = painter.layout_no_wrap(text.to_owned(), font.clone(), color);
    let ppp = painter.ctx().pixels_per_point();
    let bold_offset = if bold { 0.4 } else { 0.0 };
    let mut pos = cell.min;
    // Like felis, tolerate a physical pixel of advance rounding. Ink overhang
    // alone does not shrink an otherwise normal-width letter.
    if galley.size().x > cell.width() + 1.0 / ppp && galley.mesh_bounds.is_positive() {
        let ink = galley.mesh_bounds;
        let ratio = ((cell.width() - bold_offset).max(0.0) / ink.width())
            .min(cell.height() / ink.height())
            .min(1.0);
        font.size = (font.size * ppp * ratio).floor().max(1.0) / ppp;
        galley = painter.layout_no_wrap(text.to_owned(), font, color);
        // Center the actual rasterized ink, excluding its advance and bearings.
        pos = cell.center()
            - galley.mesh_bounds.center().to_vec2()
            - Vec2::new(bold_offset / 2.0, 0.0);
        pos.x = (pos.x * ppp).round() / ppp;
        pos.y = (pos.y * ppp).round() / ppp;
    }
    painter.galley(pos, galley.clone(), color);
    if bold {
        painter.galley(pos + Vec2::new(bold_offset, 0.0), galley, color);
    }
}

pub fn paint_braille(painter: &Painter, ch: char, cell: Rect, color: Color32) -> bool {
    if !('\u{2800}'..='\u{28ff}').contains(&ch) {
        return false;
    }
    let pattern = ch as u32 - 0x2800;
    let dot = cell.size() / Vec2::new(4.0, 8.0);
    // Font coverage and glyph bearings cannot guarantee a continuous Braille canvas.
    for (row, bits) in [[0, 3], [1, 4], [2, 5], [6, 7]].into_iter().enumerate() {
        for (col, bit) in bits.into_iter().enumerate() {
            if pattern & (1 << bit) != 0 {
                let center = cell.min
                    + Vec2::new(
                        (col as f32 * 2.0 + 1.0) * dot.x,
                        (row as f32 * 2.0 + 1.0) * dot.y,
                    );
                painter.rect_filled(Rect::from_center_size(center, dot), 0, color);
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{self, Pos2, Shape, Vec2};

    #[test]
    fn oversized_symbol_is_rasterized_at_a_smaller_size_and_its_ink_is_centered() {
        for ppp in [1.0, 2.0] {
            let ctx = egui::Context::default();
            ctx.set_pixels_per_point(ppp);
            if let Ok(path) = std::env::var("NEKOMATA_FONT") {
                let mut fonts = egui::FontDefinitions::default();
                fonts.font_data.insert(
                    "terminal".into(),
                    egui::FontData::from_owned(std::fs::read(path).unwrap()).into(),
                );
                fonts
                    .families
                    .get_mut(&egui::FontFamily::Monospace)
                    .unwrap()
                    .insert(0, "terminal".into());
                ctx.set_fonts(fonts);
            }
            let mut cell = Rect::NOTHING;
            let output = ctx.run(egui::RawInput::default(), |ctx| {
                let painter = ctx.layer_painter(egui::LayerId::background());
                let font = egui::FontId::monospace(15.0);
                let cw = ctx.fonts_mut(|f| f.glyph_width(&font, 'M'));
                cell = Rect::from_min_size(Pos2::new(10.0, 20.0), Vec2::new(cw, 21.0));
                paint_text(&painter, "①", font, cell, Color32::RED, false);
            });
            let Shape::Text(shape) = &output.shapes[0].shape else {
                panic!("expected text");
            };
            assert!(
                shape.galley.job.sections[0].format.font_id.size < 15.0,
                "must rasterize at a smaller font size, not transform a full-size mesh"
            );
            let ink = shape.galley.mesh_bounds.translate(shape.pos.to_vec2());
            assert!(
                (ink.center() - cell.center()).length() <= 1.0,
                "ink {ink:?} not centered in {cell:?}"
            );
            assert!(ink.width() <= cell.width() + 1.0, "ink exceeds its columns");
            assert!(
                ink.width() >= cell.width() * 0.65,
                "ink unnecessarily small"
            );
            assert!(
                (ink.width() / ink.height() - 1.0).abs() < 0.25,
                "circle lost its proportions"
            );
        }
    }

    #[test]
    fn fitting_preserves_native_layout_for_ascii_and_two_column_text() {
        let ctx = egui::Context::default();
        let mut expected = Vec::new();
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            let painter = ctx.layer_painter(egui::LayerId::background());
            let font = egui::FontId::monospace(15.0);
            let cw = ctx.fonts_mut(|f| f.glyph_width(&font, 'M'));
            for (text, columns) in [("M", 1.0), ("i", 1.0), ("猫", 2.0)] {
                let natural = painter.layout_no_wrap(text.into(), font.clone(), Color32::RED);
                expected.push(natural);
                paint_text(
                    &painter,
                    text,
                    font.clone(),
                    Rect::from_min_size(Pos2::new(10.0, 20.0), Vec2::new(cw * columns, 21.0)),
                    Color32::RED,
                    false,
                );
            }
        });
        for (shape, natural) in output.shapes.iter().zip(expected) {
            let Shape::Text(shape) = &shape.shape else {
                panic!("expected text");
            };
            assert!(std::sync::Arc::ptr_eq(&shape.galley, &natural));
            assert_eq!(shape.pos, Pos2::new(10.0, 20.0));
        }
    }

    #[test]
    fn braille_dots_render_without_a_font_in_unicode_positions() {
        for (ch, x, y) in [
            ('\u{2801}', 12.0, 22.0),
            ('\u{2802}', 12.0, 26.0),
            ('\u{2804}', 12.0, 30.0),
            ('\u{2808}', 16.0, 22.0),
            ('\u{2810}', 16.0, 26.0),
            ('\u{2820}', 16.0, 30.0),
            ('\u{2840}', 12.0, 34.0),
            ('\u{2880}', 16.0, 34.0),
        ] {
            let ctx = egui::Context::default();
            let output = ctx.run(egui::RawInput::default(), |ctx| {
                let painter = ctx.layer_painter(egui::LayerId::background());
                assert!(paint_braille(
                    &painter,
                    ch,
                    Rect::from_min_size(Pos2::new(10.0, 20.0), Vec2::new(8.0, 16.0)),
                    Color32::RED,
                ));
            });
            assert_eq!(output.shapes.len(), 1);
            let Shape::Rect(dot) = &output.shapes[0].shape else {
                panic!("expected a dot, not a font glyph");
            };
            assert_eq!(dot.rect.center(), Pos2::new(x, y));
            assert_eq!(dot.rect.size(), Vec2::splat(2.0));
            assert_eq!(dot.fill, Color32::RED);
        }
    }

    #[test]
    fn every_braille_pattern_keeps_its_dots_inside_scaled_cells() {
        for size in [
            Vec2::new(3.0, 9.0),
            Vec2::new(8.5, 21.0),
            Vec2::new(36.0, 101.0),
        ] {
            let cell = Rect::from_min_size(Pos2::new(10.25, 20.5), size);
            let ctx = egui::Context::default();
            for pattern in 0u32..=255 {
                let output = ctx.run(egui::RawInput::default(), |ctx| {
                    assert!(paint_braille(
                        &ctx.layer_painter(egui::LayerId::background()),
                        char::from_u32(0x2800 + pattern).unwrap(),
                        cell,
                        Color32::RED,
                    ));
                });
                assert_eq!(output.shapes.len(), pattern.count_ones() as usize);
                for shape in output.shapes {
                    let Shape::Rect(dot) = shape.shape else {
                        panic!("expected a dot, not a font glyph");
                    };
                    assert!(cell.contains_rect(dot.rect));
                }
            }
        }
    }

    #[test]
    fn ordinary_text_is_left_for_the_font_renderer() {
        let ctx = egui::Context::default();
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            for ch in ['A', '猫', '\u{27ff}', '\u{2900}'] {
                assert!(!paint_braille(
                    &ctx.layer_painter(egui::LayerId::background()),
                    ch,
                    Rect::from_min_size(Pos2::ZERO, Vec2::new(8.0, 16.0)),
                    Color32::RED,
                ));
            }
        });
        assert!(output.shapes.is_empty());
    }
}
