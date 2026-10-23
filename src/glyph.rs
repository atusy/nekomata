use eframe::egui::{Color32, Painter, Rect, Vec2};

/// Fit font fallback glyphs to the columns assigned by the terminal, keeping
/// their aspect ratio and texture coordinates intact.
pub fn paint_text(
    painter: &Painter,
    text: &str,
    font: eframe::egui::FontId,
    cell: Rect,
    color: Color32,
    bold: bool,
) {
    let mut galley = painter.layout_no_wrap(text.to_owned(), font, color);
    let bold_offset = if bold {
        0.4_f32.min(cell.width() / 2.0)
    } else {
        0.0
    };
    let left = galley.mesh_bounds.left().min(0.0);
    let right = galley.mesh_bounds.right().max(galley.rect.right());
    let available = (cell.width() - bold_offset).max(0.0);
    let width = right - left;
    if width > 0.0 && (width > available || left < 0.0) {
        let scale = (available / width).min(1.0);
        let fit_x = |x: f32| (x - left) * scale;
        let vertical_offset = if scale < 1.0 {
            ((cell.height() - galley.rect.height() * scale) / 2.0).max(0.0)
        } else {
            0.0
        };
        let fit_rect = |mut rect: Rect| {
            rect.min.x = fit_x(rect.min.x);
            rect.max.x = fit_x(rect.max.x);
            rect.min.y *= scale;
            rect.max.y *= scale;
            rect
        };
        // Clone only overflowing layouts; egui's shared layout cache stays intact.
        let fitted = std::sync::Arc::make_mut(&mut galley);
        fitted.rect = fit_rect(fitted.rect).translate(Vec2::new(0.0, vertical_offset));
        fitted.mesh_bounds =
            fit_rect(fitted.mesh_bounds).translate(Vec2::new(0.0, vertical_offset));
        for placed in &mut fitted.rows {
            placed.pos.x *= scale;
            placed.pos.y = placed.pos.y * scale + vertical_offset;
            let row = std::sync::Arc::make_mut(&mut placed.row);
            row.size *= scale;
            row.visuals.mesh_bounds = fit_rect(row.visuals.mesh_bounds);
            for vertex in &mut row.visuals.mesh.vertices {
                vertex.pos.x = fit_x(vertex.pos.x);
                vertex.pos.y *= scale;
            }
        }
    }
    let painter = painter.with_clip_rect(cell);
    painter.galley(cell.min, galley.clone(), color);
    if bold {
        painter.galley(cell.min + Vec2::new(bold_offset, 0.0), galley, color);
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
    fn oversized_circled_numbers_preserve_their_aspect_ratio() {
        let ctx = egui::Context::default();
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
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            let painter = ctx.layer_painter(egui::LayerId::background());
            let font = egui::FontId::monospace(15.0);
            let cw = ctx.fonts_mut(|fonts| fonts.glyph_width(&font, 'M'));
            for text in ["①", "②", "W"] {
                let natural = painter.layout_no_wrap(text.into(), font.clone(), Color32::RED);
                let cell = Rect::from_min_size(Pos2::ZERO, Vec2::new(cw, 21.0));
                paint_text(&painter, text, font.clone(), cell, Color32::RED, false);
                let shapes = ctx.graphics(|graphics| {
                    graphics
                        .get(egui::LayerId::background())
                        .unwrap()
                        .all_entries()
                        .last()
                        .unwrap()
                        .shape
                        .clone()
                });
                let Shape::Text(fitted) = shapes else {
                    panic!("expected text");
                };
                let natural_ratio = natural.mesh_bounds.width() / natural.mesh_bounds.height();
                let fitted_ratio =
                    fitted.galley.mesh_bounds.width() / fitted.galley.mesh_bounds.height();
                assert!(
                    (natural_ratio - fitted_ratio).abs() < 0.001,
                    "{text} was distorted: {natural_ratio} -> {fitted_ratio}"
                );
            }
        });
    }

    #[test]
    fn oversized_text_mesh_stays_inside_its_cell_even_when_bold() {
        let ctx = egui::Context::default();
        let cell = Rect::from_min_size(Pos2::new(10.0, 20.0), Vec2::new(3.0, 30.0));
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            paint_text(
                &ctx.layer_painter(egui::LayerId::background()),
                "W",
                egui::FontId::monospace(20.0),
                cell,
                Color32::RED,
                true,
            );
        });
        let primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
        assert!(!primitives.is_empty());
        for primitive in primitives {
            if let egui::epaint::Primitive::Mesh(mesh) = primitive.primitive {
                for vertex in mesh.vertices {
                    assert!(
                        vertex.pos.x >= cell.left() && vertex.pos.x <= cell.right(),
                        "glyph escaped its allocated columns: {:?}",
                        vertex.pos
                    );
                }
            }
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
