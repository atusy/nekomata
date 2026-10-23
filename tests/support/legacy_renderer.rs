// Frozen per-cell painting path from nekomata 1f7eb18. Benchmark only.
use crate::theme::{BG, FG};
use eframe::egui::{self, Color32, FontId, Pos2, Rect, Stroke, Vec2};
use felis_client_core::ShadowScreen;
use felis_grid::{AttrFlags, Color, Grapheme};
#[path = "legacy_glyph.rs"]
mod glyph;

pub fn paint(painter: &egui::Painter, shadow: &ShadowScreen, rect: Rect, font_size: f32) {
    let font = FontId::monospace(font_size);
    let cw = painter.ctx().fonts_mut(|f| f.glyph_width(&font, 'M'));
    let ch = (font_size * 1.4).ceil();
    let screen = shadow.screen();
    let rows = screen.rows();
    let cols = screen.cols();
    let text_rect = rect;
    let selection: Option<(usize, usize)> = None;
    painter.rect_filled(rect, 0, BG);
    let visible_rows = screen.rows().min(rows);
    let visible_cols = screen.cols().min(cols);
    for row in 0..visible_rows {
        for col in 0..visible_cols {
            let Some(cell) = screen.cell(row, col) else {
                continue;
            };
            let attr = screen.style(cell.style);
            let mut bg = color(attr.bg, BG);
            let mut fg = color(attr.fg, FG);
            if attr.flags.contains(AttrFlags::REVERSE) ^ shadow.reverse_video() {
                std::mem::swap(&mut bg, &mut fg);
            }
            let index = usize::from(row) * usize::from(screen.cols()) + usize::from(col);
            if selection.is_some_and(|(a, b)| (a.min(b)..=a.max(b)).contains(&index)) {
                bg = Color32::from_rgb(64, 78, 94);
            }
            let pos = text_rect.min + Vec2::new(f32::from(col) * cw, f32::from(row) * ch);
            // Continue the last column's background to the right edge. Keep
            // row heights fixed so bottom bars don't grow into the unused
            // fractional row below the terminal grid.
            let max = Pos2::new(
                if col + 1 == visible_cols {
                    text_rect.right()
                } else {
                    pos.x + cw
                },
                pos.y + ch,
            );
            painter.rect_filled(Rect::from_min_max(pos, max), 0, bg);
        }
    }
    for row in 0..screen.rows().min(rows) {
        for col in 0..screen.cols().min(cols) {
            let Some(cell) = screen.cell(row, col) else {
                continue;
            };
            let attr = screen.style(cell.style);
            if attr.flags.contains(AttrFlags::CONCEAL) {
                continue;
            }
            let mut fg = color(attr.fg, FG);
            if attr.flags.contains(AttrFlags::REVERSE) ^ shadow.reverse_video() {
                fg = color(attr.bg, BG);
            }
            if attr.flags.contains(AttrFlags::FAINT) {
                fg = fg.gamma_multiply(0.6);
            }
            let pos = text_rect.min + Vec2::new(f32::from(col) * cw, f32::from(row) * ch);
            let painted = matches!(cell.grapheme, Grapheme::Char(c) if glyph::paint_braille(
                painter, c, Rect::from_min_size(pos, Vec2::new(cw, ch)), fg,
            ));
            if !painted {
                let text = match cell.grapheme {
                    Grapheme::Ascii(b) => char::from(b).to_string(),
                    Grapheme::Char(c) => c.to_string(),
                    Grapheme::Cluster(id) => shadow.cluster_str(id).unwrap_or("�").to_owned(),
                    _ => continue,
                };
                let wide = col + 1 < visible_cols
                    && screen
                        .cell(row, col + 1)
                        .is_some_and(|next| matches!(next.grapheme, Grapheme::Spacer));
                glyph::paint_text(
                    painter,
                    &text,
                    font.clone(),
                    Rect::from_min_size(pos, Vec2::new(if wide { 2.0 * cw } else { cw }, ch)),
                    fg,
                    attr.flags.contains(AttrFlags::BOLD),
                );
            }
            if attr.flags.contains(AttrFlags::UNDERLINE) {
                painter.line_segment(
                    [
                        pos + Vec2::new(0.0, ch - 2.0),
                        pos + Vec2::new(cw, ch - 2.0),
                    ],
                    Stroke::new(1.0_f32, fg),
                );
            }
        }
    }
}

fn color(color: Color, default: Color32) -> Color32 {
    match color {
        Color::Default => default,
        Color::Rgb(r, g, b) => Color32::from_rgb(r, g, b),
        Color::Indexed(n) => {
            const BASE: [[u8; 3]; 16] = [
                [46, 52, 64],
                [191, 97, 106],
                [163, 190, 140],
                [235, 203, 139],
                [129, 161, 193],
                [180, 142, 173],
                [136, 192, 208],
                [229, 233, 240],
                [76, 86, 106],
                [191, 97, 106],
                [163, 190, 140],
                [235, 203, 139],
                [129, 161, 193],
                [180, 142, 173],
                [143, 188, 187],
                [236, 239, 244],
            ];
            let [r, g, b] = if n < 16 {
                BASE[n as usize]
            } else if n >= 232 {
                [8 + (n - 232) * 10; 3]
            } else {
                let n = n - 16;
                let ramp = |v| if v == 0 { 0 } else { 55 + 40 * v };
                [ramp(n / 36), ramp(n / 6 % 6), ramp(n % 6)]
            };
            Color32::from_rgb(r, g, b)
        }
    }
}
