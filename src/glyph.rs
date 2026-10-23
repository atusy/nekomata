use eframe::egui::{Color32, Painter, Rect, Vec2};

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
