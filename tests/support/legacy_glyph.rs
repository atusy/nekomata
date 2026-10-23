// Frozen baseline from nekomata 1f7eb18, used only by the rendering benchmark.
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
