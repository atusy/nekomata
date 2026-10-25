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
pub const PALETTE_WIDTH: f32 = 520.0;
pub const PALETTE_ROW_HEIGHT: f32 = 32.0;
pub const PALETTE_LIST_HEIGHT: f32 = 270.0;

/// The body size the chrome's other text sizes were drawn against.
const DESIGN_BODY_PX: f32 = 14.0;

/// Scales a text size or row height drawn against a 14 px body to the
/// configured body size.
pub fn ui_size(style: &eframe::egui::Style, nominal: f32) -> f32 {
    let body = style
        .text_styles
        .get(&eframe::egui::TextStyle::Body)
        .map_or(DESIGN_BODY_PX, |font| font.size);
    body * nominal / DESIGN_BODY_PX
}

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

pub fn menu_button(ui: &mut eframe::egui::Ui, open: bool) -> eframe::egui::Response {
    use eframe::egui::{self, Sense, TextStyle, WidgetInfo};
    let size = (egui::vec2(
        ui_size(ui.style(), 16.0),
        ui.text_style_height(&TextStyle::Button),
    ) + 2.0 * ui.spacing().button_padding)
        .max(ui.spacing().interact_size);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| {
        WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), open, "Main menu")
    });
    if ui.is_rect_visible(rect) {
        let fill = if open {
            TAB_ACTIVE_BG
        } else if response.hovered() || response.is_pointer_button_down_on() {
            TAB_HOVER_BG
        } else {
            Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, 5, fill);
        let texture_id = egui::Id::unique("nekomata-menu-icon");
        let texture = ui
            .ctx()
            .data(|data| data.get_temp::<egui::TextureHandle>(texture_id))
            .unwrap_or_else(|| {
                let icon =
                    eframe::icon_data::from_png_bytes(include_bytes!("../share/icons/menu.png"))
                        .expect("bundled menu icon is valid PNG");
                let texture = ui.ctx().load_texture(
                    "nekomata-menu-icon",
                    egui::ColorImage::from_rgba_unmultiplied(
                        [icon.width as usize, icon.height as usize],
                        &icon.rgba,
                    ),
                    egui::TextureOptions::LINEAR,
                );
                ui.ctx()
                    .data_mut(|data| data.insert_temp(texture_id, texture.clone()));
                texture
            });
        let side = ui_size(ui.style(), 20.0).min(rect.height());
        ui.painter().image(
            texture.id(),
            // Optically align the cat silhouette with the adjacent tab text.
            egui::Rect::from_center_size(
                rect.center() + egui::vec2(0.0, ui_size(ui.style(), 2.0)),
                egui::vec2(side, side),
            ),
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            if open { FG } else { TAB_INACTIVE_FG },
        );
    }
    if response.hovered()
        && let Some(cursor) = ui.visuals().interact_cursor
    {
        ui.ctx().set_cursor_icon(cursor);
    }
    response
}

pub fn tab_button(
    ui: &mut eframe::egui::Ui,
    title: &str,
    selected: bool,
) -> eframe::egui::Response {
    use eframe::egui::{self, Sense, TextStyle, WidgetInfo};
    let color = if selected { FG } else { TAB_INACTIVE_FG };
    let galley = ui.painter().layout_no_wrap(
        title.to_owned(),
        TextStyle::Button.resolve(ui.style()),
        color,
    );
    let size = (galley.size() + 2.0 * ui.spacing().button_padding).max(ui.spacing().interact_size);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response
        .widget_info(|| WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), selected, title));
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&response);
        ui.painter().rect_filled(
            rect,
            visuals.corner_radius,
            if selected {
                TAB_ACTIVE_BG
            } else {
                visuals.weak_bg_fill
            },
        );
        if galley.mesh_bounds.is_finite() {
            // Font line boxes include asymmetric ascent/descent whitespace.
            // Keep the button size, but center the visible glyphs vertically.
            let pos = egui::pos2(
                rect.center().x - galley.size().x / 2.0,
                rect.center().y - galley.mesh_bounds.center().y,
            );
            ui.painter().galley(pos, galley, color);
        }
    }
    if response.hovered()
        && let Some(cursor) = ui.visuals().interact_cursor
    {
        ui.ctx().set_cursor_icon(cursor);
    }
    response
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
    fn menu_trigger_matches_tab_ink_and_shows_its_open_state() {
        use eframe::egui::{self, epaint::Shape};
        for font_size in [4.0, 10.0, 14.0, 24.0, 48.0, 72.0] {
            for density in [1.0, 1.1, 1.25, 1.5, 2.0] {
                let ctx = egui::Context::default();
                ctx.set_pixels_per_point(density);
                ctx.set_fonts(super::chrome_fonts());
                ctx.set_theme(egui::Theme::Dark);
                ctx.style_mut_of(egui::Theme::Dark, |style| {
                    for text_style in [egui::TextStyle::Body, egui::TextStyle::Button] {
                        style
                            .text_styles
                            .insert(text_style, egui::FontId::proportional(font_size));
                    }
                });
                for open in [false, true] {
                    let mut menu_rect = egui::Rect::NOTHING;
                    let frame = crate::test_support::run(&ctx, egui::RawInput::default(), |ui| {
                        ui.spacing_mut().button_padding = egui::vec2(15.0, 6.0);
                        ui.spacing_mut().interact_size.y = super::ui_size(ui.style(), 30.0).max(
                            ui.text_style_height(&egui::TextStyle::Button)
                                + 2.0 * ui.spacing().button_padding.y,
                        );
                        ui.horizontal(|ui| {
                            menu_rect = super::menu_button(ui, open).rect;
                            egui::ScrollArea::horizontal()
                                .auto_shrink([false, true])
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        super::tab_button(ui, "nekomata", false);
                                    });
                                });
                        });
                    });
                    let icons: Vec<_> = frame
                        .shapes
                        .iter()
                        .filter_map(|shape| match &shape.shape {
                            Shape::Mesh(mesh) if menu_rect.contains_rect(mesh.calc_bounds()) => {
                                Some(mesh)
                            }
                            _ => None,
                        })
                        .collect();
                    assert_eq!(icons.len(), 1, "menu must show the nekomata image");
                    let icon_rect = icons[0].calc_bounds();
                    assert!(icons[0].vertices.iter().all(|vertex| vertex.color
                        == if open {
                            super::FG
                        } else {
                            super::TAB_INACTIVE_FG
                        }));
                    let title_center = frame
                        .shapes
                        .iter()
                        .find_map(|shape| match &shape.shape {
                            Shape::Text(text) if text.galley.job.text == "nekomata" => {
                                Some(text.visual_bounding_rect().center().y)
                            }
                            _ => None,
                        })
                        .unwrap();
                    assert!(
                        (icon_rect.center().y - title_center - 2.0 * font_size / 14.0).abs()
                            <= 1.0 / density,
                        "menu optical offset: font={font_size}, density={density}, icon={icon_rect:?}, title_center={title_center}, menu={menu_rect:?}"
                    );
                    assert!((icon_rect.center().x - menu_rect.center().x).abs() <= 0.5 / density);
                    let highlighted = frame.shapes.iter().any(|shape| matches!(&shape.shape,
                    Shape::Rect(rect) if rect.rect == menu_rect && rect.fill == super::TAB_ACTIVE_BG));
                    assert_eq!(
                        highlighted, open,
                        "menu highlight must follow popup visibility"
                    );
                }
            }
        }
    }

    #[test]
    fn tab_title_ink_is_vertically_centered() {
        use eframe::egui::{self, epaint::Shape};
        for density in [1.0, 2.0] {
            let ctx = egui::Context::default();
            ctx.set_pixels_per_point(density);
            ctx.set_fonts(super::chrome_fonts());
            for title in [
                "確認する felis設定の読み込み | nekomata",
                "⠇ 確認する",
                "English gy",
            ] {
                for selected in [false, true] {
                    let mut rect = egui::Rect::NOTHING;
                    let frame = crate::test_support::run(&ctx, egui::RawInput::default(), |ui| {
                        ui.spacing_mut().button_padding = egui::vec2(15.0, 6.0);
                        rect = super::tab_button(ui, title, selected).rect;
                    });
                    let ink = frame
                        .shapes
                        .iter()
                        .find_map(|shape| match &shape.shape {
                            Shape::Text(text) if text.galley.job.text == title => {
                                Some(text.visual_bounding_rect())
                            }
                            _ => None,
                        })
                        .expect("tab title is painted");
                    let difference = ink.center().y - rect.center().y;
                    assert!(
                        difference.abs() <= 0.5 / density,
                        "title={title:?}, density={density}, offset={difference}, ink={ink:?}, button={rect:?}"
                    );
                }
            }
        }
    }

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
