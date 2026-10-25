use eframe::egui::Color32;

pub const BG: Color32 = Color32::from_rgb(20, 23, 31);
pub const FG: Color32 = Color32::from_rgb(216, 222, 233);
pub const ACCENT: Color32 = Color32::from_rgb(163, 190, 140);
pub const TAB_BAR_BG: Color32 = Color32::from_rgb(24, 28, 37);
pub const TAB_ACTIVE_BG: Color32 = Color32::from_rgb(48, 55, 68);
pub const TAB_HOVER_BG: Color32 = Color32::from_rgb(36, 43, 54);
pub const TAB_INACTIVE_FG: Color32 = Color32::from_rgb(140, 149, 167);

pub const MENU_BORDER: Color32 = Color32::from_rgb(48, 55, 68);
pub const MENU_WIDTH: f32 = 256.0;
pub const MENU_ROW_HEIGHT: f32 = 30.0;

pub fn menu_style(style: &mut eframe::egui::Style) {
    use eframe::egui::{self, Stroke, Vec2};
    egui::containers::menu::menu_style(style);
    style.override_font_id = Some(egui::FontId::proportional(14.0));
    style.spacing.button_padding = Vec2::new(10.0, 5.0);
    style.spacing.item_spacing = Vec2::new(4.0, 2.0);
    style.spacing.interact_size.y = MENU_ROW_HEIGHT;
    style.spacing.menu_margin = egui::Margin::same(6);
    style.visuals.window_fill = TAB_BAR_BG;
    style.visuals.window_stroke = Stroke::new(1.0_f32, MENU_BORDER);
    style.visuals.window_corner_radius = egui::CornerRadius::same(8);
    style.visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, MENU_BORDER);
    for state in [
        &mut style.visuals.widgets.inactive,
        &mut style.visuals.widgets.hovered,
        &mut style.visuals.widgets.active,
        &mut style.visuals.widgets.open,
    ] {
        state.corner_radius = egui::CornerRadius::same(5);
        state.bg_stroke = Stroke::NONE;
        state.fg_stroke = Stroke::new(1.0_f32, FG);
        state.expansion = 0.0;
    }
    style.visuals.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
    style.visuals.widgets.hovered.weak_bg_fill = TAB_ACTIVE_BG;
    style.visuals.widgets.active.weak_bg_fill = TAB_HOVER_BG;
    style.visuals.widgets.open.weak_bg_fill = TAB_ACTIVE_BG;
}

/// Application fonts are independent of terminal configuration.
pub fn chrome_fonts() -> eframe::egui::FontDefinitions {
    use eframe::egui::{FontData, FontDefinitions, FontFamily};
    let mut fonts = FontDefinitions::default();
    for path in [
        "/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
    ] {
        if let Ok(bytes) = std::fs::read(path) {
            fonts
                .font_data
                .insert("cjk".into(), FontData::from_owned(bytes).into());
            // Use one face for both Japanese and Latin in titles. Mixing a
            // Latin primary with this taller fallback shifts CJK baselines.
            fonts
                .families
                .entry(FontFamily::Proportional)
                .or_default()
                .insert(0, "cjk".into());
            fonts
                .families
                .entry(FontFamily::Monospace)
                .or_default()
                .push("cjk".into());
            break;
        }
    }
    // Spinner titles can contain Braille, which neither the default egui
    // fonts nor the CJK face necessarily covers.
    for path in [
        "/System/Library/Fonts/Apple Symbols.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/truetype/noto/NotoSansSymbols2-Regular.ttf",
    ] {
        if let Ok(bytes) = std::fs::read(path) {
            fonts
                .font_data
                .insert("symbols".into(), FontData::from_owned(bytes).into());
            for kind in [FontFamily::Monospace, FontFamily::Proportional] {
                fonts
                    .families
                    .entry(kind)
                    .or_default()
                    .push("symbols".into());
            }
            break;
        }
    }
    fonts
}

#[cfg(test)]
mod tests {
    #[test]
    #[cfg(target_os = "macos")]
    fn tab_titles_cover_braille_spinners_and_japanese() {
        use eframe::egui;
        let ctx = egui::Context::default();
        ctx.set_fonts(super::chrome_fonts());
        let _ = crate::test_support::run(&ctx, egui::RawInput::default(), |ui| {
            assert!(ui.fonts_mut(|fonts| fonts.has_glyphs(
                &egui::FontId::proportional(14.0),
                "⠇ 確認する felis設定の読み込み | nekomata"
            )));
        });
    }
    #[test]
    #[cfg(target_os = "macos")]
    fn tab_titles_share_a_baseline_across_japanese_and_latin() {
        use eframe::egui;
        let ctx = egui::Context::default();
        ctx.set_fonts(super::chrome_fonts());
        let _ = crate::test_support::run(&ctx, egui::RawInput::default(), |ui| {
            let galley = ui.painter().layout_no_wrap(
                "H確f認".into(),
                egui::FontId::proportional(14.0),
                egui::Color32::WHITE,
            );
            let glyphs = &galley.rows[0].glyphs;
            let baseline = glyphs[0].pos.y;
            for glyph in glyphs {
                assert!(
                    (glyph.pos.y - baseline).abs() <= 0.5,
                    "{} is off baseline: {} vs {}",
                    glyph.chr,
                    glyph.pos.y,
                    baseline
                );
            }
        });
    }
}
