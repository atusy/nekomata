use eframe::egui::Color32;

pub const BG: Color32 = Color32::from_rgb(20, 23, 31);
pub const FG: Color32 = Color32::from_rgb(216, 222, 233);
pub const ACCENT: Color32 = Color32::from_rgb(163, 190, 140);
pub const TAB_BAR_BG: Color32 = Color32::from_rgb(24, 28, 37);
pub const TAB_ACTIVE_BG: Color32 = Color32::from_rgb(48, 55, 68);
pub const TAB_HOVER_BG: Color32 = Color32::from_rgb(36, 43, 54);
pub const TAB_INACTIVE_FG: Color32 = Color32::from_rgb(140, 149, 167);
pub const ERROR_FG: Color32 = Color32::LIGHT_RED;
pub const PANE_DIVIDER: Color32 = Color32::from_gray(45);

pub const MENU_BORDER: Color32 = Color32::from_rgb(48, 55, 68);
pub const MENU_ROW_HEIGHT: f32 = 30.0;
pub const PALETTE_WIDTH: f32 = 520.0;
pub const PALETTE_ROW_HEIGHT: f32 = 32.0;
/// A palette row with a detail line under its label.
pub const PALETTE_DETAIL_ROW_HEIGHT: f32 = 48.0;
pub const PALETTE_LIST_HEIGHT: f32 = 270.0;

/// The body size the chrome's other text sizes were drawn against.
const DESIGN_BODY_PX: f32 = 14.0;

fn body_scale(style: &eframe::egui::Style) -> f32 {
    style
        .text_styles
        .get(&eframe::egui::TextStyle::Body)
        .map_or(DESIGN_BODY_PX, |font| font.size)
        / DESIGN_BODY_PX
}

/// Scales a text size drawn against a 14 px body to the configured body
/// size.
pub fn ui_size(style: &eframe::egui::Style, nominal: f32) -> f32 {
    nominal * body_scale(style)
}

/// Scales a row height or width drawn against a 14 px body up to fit a
/// larger body. Smaller bodies keep the drawn extent, so rows stay whole
/// pixels and as easy to hit as before.
pub fn ui_extent(style: &eframe::egui::Style, nominal: f32) -> f32 {
    nominal * body_scale(style).max(1.0)
}

pub fn menu_style(style: &mut eframe::egui::Style) {
    use eframe::egui::{self, Stroke, Vec2};
    egui::containers::menu::menu_style(style);
    style.override_font_id = Some(egui::FontId::proportional(ui_size(style, 14.0)));
    style.spacing.button_padding = Vec2::new(10.0, 5.0);
    style.spacing.item_spacing = Vec2::new(4.0, 2.0);
    style.spacing.interact_size.y = ui_extent(style, MENU_ROW_HEIGHT);
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

/// The farthest an overlay sits below the window top.
pub const OVERLAY_MAX_TOP: f32 = 64.0;

/// The padding between an overlay's frame and its content.
pub const OVERLAY_MARGIN: f32 = 12.0;

/// Distance from the window top to the palette and the controls it opens:
/// a little below the tab bar, closer on short windows.
pub fn overlay_top(viewport_height: f32) -> f32 {
    (viewport_height * 0.12).min(OVERLAY_MAX_TOP)
}

/// The floating panel behind the command palette and the controls it opens.
pub fn overlay_frame() -> eframe::egui::Frame {
    use eframe::egui;
    egui::Frame::new()
        .shadow(egui::epaint::Shadow {
            offset: [0, 8],
            blur: 24,
            spread: 0,
            color: Color32::from_black_alpha(100),
        })
        .fill(TAB_BAR_BG)
        .stroke(egui::Stroke::new(1.0, MENU_BORDER))
        .corner_radius(10)
        .inner_margin(OVERLAY_MARGIN as i8)
}

pub fn palette_button(ui: &mut eframe::egui::Ui, open: bool) -> eframe::egui::Response {
    use eframe::egui::{self, Sense, TextStyle, WidgetInfo};
    let size = (egui::vec2(
        ui_size(ui.style(), 16.0),
        ui.text_style_height(&TextStyle::Button),
    ) + 2.0 * ui.spacing().button_padding)
        .max(ui.spacing().interact_size);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| {
        WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), open, "Command palette")
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
        let texture_id = egui::Id::unique("nekomata-palette-button-icon");
        let texture = ui
            .ctx()
            .data(|data| data.get_temp::<egui::TextureHandle>(texture_id))
            .unwrap_or_else(|| {
                let icon = eframe::icon_data::from_png_bytes(include_bytes!(
                    "../share/icons/palette-button.png"
                ))
                .expect("bundled palette button icon is valid PNG");
                let texture = ui.ctx().load_texture(
                    "nekomata-palette-button-icon",
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

/// The family that draws emoji in color where an installed font can.
const EMOJI_FAMILY: &str = "emoji";
/// The proportional fonts without the color emoji font, for symbols that
/// default to text presentation. A symbol none of them has can still come
/// from a color font the system fallback finds, rather than show as a box.
const TEXT_FAMILY: &str = "text";

/// `text` laid out without wrapping so that each grapheme cluster takes the
/// presentation Unicode gives it: emoji such as 😀 keep their own colors,
/// while symbols such as ✳, which also have color emoji glyphs, are drawn in
/// `color` like the letters around them.
///
/// For text that comes from programs, such as terminal titles; egui itself
/// picks a font per character regardless of presentation.
///
/// Every row is `line_height` tall, usually [`title_line_height`], whatever
/// the faces drawing it, so emoji neither heighten nor shorten it.
pub(crate) fn presented(
    text: &str,
    size: f32,
    line_height: f32,
    color: Color32,
) -> eframe::egui::text::LayoutJob {
    use eframe::egui::{FontFamily, FontId, TextFormat, epaint::text::has_emoji_presentation};
    use unicode_segmentation::UnicodeSegmentation as _;
    let mut job = eframe::egui::text::LayoutJob {
        // Text without sections, such as an empty title, still takes a row.
        first_row_min_height: line_height,
        ..Default::default()
    };
    let format = |family: &str| TextFormat {
        // Each family's line height would otherwise come from its first face,
        // and the color emoji font's differs from the text faces'. One height
        // for every section, with each centered in it, keeps emoji where a
        // single family would place them.
        line_height: Some(line_height),
        valign: eframe::egui::Align::Center,
        ..TextFormat::simple(FontId::new(size, FontFamily::Name(family.into())), color)
    };
    let text_format = format(TEXT_FAMILY);
    // Tabs lay their titles out every frame; ASCII has no emoji to find.
    if text.is_ascii() {
        job.append(text, 0.0, text_format);
        return job;
    }
    let emoji_format = format(EMOJI_FAMILY);
    let mut run_start = 0;
    let mut run_is_emoji = false;
    for (at, cluster) in text.grapheme_indices(true) {
        let emoji =
            has_emoji_presentation(cluster) || is_keycap(cluster) || is_emoji_zwj_sequence(cluster);
        if emoji != run_is_emoji {
            if at > run_start {
                let format = if run_is_emoji {
                    &emoji_format
                } else {
                    &text_format
                };
                job.append(&text[run_start..at], 0.0, format.clone());
            }
            run_start = at;
            run_is_emoji = emoji;
        }
    }
    if text.len() > run_start {
        let format = if run_is_emoji {
            emoji_format
        } else {
            text_format
        };
        job.append(&text[run_start..], 0.0, format);
    }
    job
}

/// Whether `cluster` is a keycap, such as 1️⃣, which Unicode lists as emoji
/// even without the emoji presentation selector that
/// `has_emoji_presentation` looks for; a text presentation selector keeps it
/// text.
fn is_keycap(cluster: &str) -> bool {
    let mut chars = cluster.chars();
    chars
        .next()
        .is_some_and(|base| base.is_ascii_digit() || base == '#' || base == '*')
        && matches!(chars.as_str(), "\u{20E3}" | "\u{FE0F}\u{20E3}")
}

/// Whether `cluster` joins emoji with zero width joiners, such as 👁‍🗨,
/// which Unicode lists as emoji even when the symbols joined default to text
/// and no emoji presentation selector follows them. A script's own joiner
/// inside a word, as in Devanagari, joins no emoji, and a text presentation
/// selector keeps the sequence text.
fn is_emoji_zwj_sequence(cluster: &str) -> bool {
    use unicode_properties::UnicodeEmoji as _;
    const ZWJ: char = '\u{200D}';
    let mut joined = 0;
    cluster.contains(ZWJ)
        && !cluster.contains('\u{FE0E}')
        && cluster
            .chars()
            .filter(|c| !matches!(*c, ZWJ | '\u{FE0F}'))
            .all(|c| {
                joined += 1;
                !c.is_ascii() && c.is_emoji_char()
            })
        && joined >= 2
}

/// The height of a row of text-family faces at `size`, which titles laid out
/// by [`presented`] keep whatever emoji they hold.
pub(crate) fn title_line_height(ui: &eframe::egui::Ui, size: f32) -> f32 {
    use eframe::egui::{FontFamily, FontId};
    ui.fonts_mut(|fonts| fonts.row_height(&FontId::new(size, FontFamily::Name(TEXT_FAMILY.into()))))
}

pub fn tab_button(
    ui: &mut eframe::egui::Ui,
    title: &str,
    selected: bool,
) -> eframe::egui::Response {
    use eframe::egui::{self, Sense, TextStyle, WidgetInfo};
    let color = if selected { FG } else { TAB_INACTIVE_FG };
    let size = TextStyle::Button.resolve(ui.style()).size;
    // A newline a program put in its title would make this tab taller than
    // the rest of the bar, and some faces draw control characters as boxes,
    // so tabs show them as spaces, unlike other text.
    let shown: std::borrow::Cow<'_, str> = if title.contains(char::is_control) {
        title
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect::<String>()
            .into()
    } else {
        title.into()
    };
    let galley =
        ui.painter()
            .layout_job(presented(&shown, size, title_line_height(ui, size), color));
    let size = (galley.size() + 2.0 * ui.spacing().button_padding).max(ui.spacing().interact_size);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
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

/// Application fonts are independent of terminal configuration. A
/// configured `primary` leads proportional text; the system faces stay as
/// its fallbacks. The color emoji font, when installed, precedes the
/// monochrome emoji, and the text and emoji families that [`presented`]
/// lays titles out in follow the same order without and with it first.
pub fn chrome_fonts(primary: Option<crate::ui_font::Face>) -> eframe::egui::FontDefinitions {
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
            before_emoji(&mut fonts, FontFamily::Monospace, "cjk");
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
                before_emoji(&mut fonts, kind, "symbols");
            }
            break;
        }
    }
    if let Some(face) = primary {
        let mut data = FontData::from_owned(face.bytes);
        data.index = face.index;
        fonts.font_data.insert("configured".into(), data.into());
        fonts
            .families
            .entry(FontFamily::Proportional)
            .or_default()
            .insert(0, "configured".into());
    }
    let text = fonts.families[&FontFamily::Proportional].clone();
    let mut emoji = text.clone();
    if let Some(data) = color_emoji() {
        fonts.font_data.insert("color-emoji".into(), data.into());
        // Ahead of the monochrome emoji, so that text egui lays out by
        // itself, such as typed input, shows emoji the text faces lack in
        // color too.
        for kind in [FontFamily::Monospace, FontFamily::Proportional] {
            before_emoji(&mut fonts, kind, "color-emoji");
        }
        emoji.insert(0, "color-emoji".into());
    }
    fonts
        .families
        .insert(FontFamily::Name(TEXT_FAMILY.into()), text);
    fonts
        .families
        .insert(FontFamily::Name(EMOJI_FAMILY.into()), emoji);
    fonts
}

/// Adds `name` to `kind` after the faces added before it but ahead of egui's
/// bundled emoji fonts, which share many symbols, such as ⚙, with system
/// faces but draw them as emoji.
fn before_emoji(
    fonts: &mut eframe::egui::FontDefinitions,
    kind: eframe::egui::FontFamily,
    name: &str,
) {
    let family = fonts.families.entry(kind).or_default();
    let at = family
        .iter()
        .position(|face| face == "NotoEmoji-Regular")
        .unwrap_or(family.len());
    family.insert(at, name.into());
}

/// The platform's color emoji font, found by family name. Apple Color Emoji
/// alone is close to 200 MB, so it stays mapped where nothing can rewrite it
/// (see `ui_font::load`), and a font a user installed is read instead.
fn color_emoji() -> Option<eframe::egui::FontData> {
    ["Apple Color Emoji", "Noto Color Emoji"]
        .into_iter()
        .find_map(|family| {
            let chosen = crate::ui_font::locate(family)?;
            Some(eframe::egui::FontData::from_blob(
                chosen.bytes,
                chosen.index,
            ))
        })
}

#[cfg(test)]
mod tests {
    #[test]
    fn chrome_text_sizes_and_rows_follow_the_configured_body() {
        use eframe::egui::{FontId, Style, TextStyle};
        let mut style = Style::default();
        style
            .text_styles
            .insert(TextStyle::Body, FontId::proportional(21.0));
        assert_eq!(super::ui_size(&style, 14.0), 21.0);
        assert_eq!(super::ui_size(&style, 12.0), 18.0);
        super::menu_style(&mut style);
        assert_eq!(style.override_font_id.unwrap().size, 21.0);
        assert_eq!(style.spacing.interact_size.y, 1.5 * super::MENU_ROW_HEIGHT);
    }

    #[test]
    fn chrome_extents_grow_with_the_body_but_never_shrink() {
        use eframe::egui::{FontId, Style, TextStyle};
        let mut style = Style::default();
        style
            .text_styles
            .insert(TextStyle::Body, FontId::proportional(13.0));
        assert_eq!(super::ui_size(&style, 14.0), 13.0);
        assert_eq!(super::ui_extent(&style, 30.0), 30.0);
        super::menu_style(&mut style);
        assert_eq!(style.spacing.interact_size.y, super::MENU_ROW_HEIGHT);
        style
            .text_styles
            .insert(TextStyle::Body, FontId::proportional(28.0));
        assert_eq!(super::ui_extent(&style, 30.0), 60.0);
    }

    #[test]
    fn a_configured_face_leads_proportional_text_before_system_fallbacks() {
        use eframe::egui::FontFamily;
        let default = super::chrome_fonts(None);
        let fonts = super::chrome_fonts(Some(crate::ui_font::Face {
            bytes: vec![],
            index: 2,
        }));
        let proportional = &fonts.families[&FontFamily::Proportional];
        assert_eq!(proportional[0], "configured");
        assert_eq!(
            proportional[1..],
            default.families[&FontFamily::Proportional]
        );
        assert_eq!(fonts.font_data["configured"].index, 2);
        assert_eq!(
            fonts.families[&FontFamily::Monospace],
            default.families[&FontFamily::Monospace]
        );
        // Titles laid out by presentation use the configured face too; only
        // the color emoji font may come before it, for emoji.
        let named = |name: &str| &fonts.families[&FontFamily::Name(name.into())];
        assert_eq!(named(super::TEXT_FAMILY)[0], "configured");
        let emoji = named(super::EMOJI_FAMILY);
        let lead = usize::from(emoji[0] == "color-emoji");
        assert_eq!(emoji[lead], "configured");
    }

    #[test]
    fn the_color_emoji_font_precedes_monochrome_emoji_except_for_text() {
        use eframe::egui::FontFamily;
        let fonts = super::chrome_fonts(None);
        let family = |family: FontFamily| &fonts.families[&family];
        let at = |names: &[String], name: &str| names.iter().position(|n| n == name);
        let text = family(FontFamily::Name(super::TEXT_FAMILY.into()));
        assert_eq!(at(text, "color-emoji"), None);
        if !fonts.font_data.contains_key("color-emoji") {
            eprintln!("skipped: no color emoji font is installed here");
            return;
        }
        for kind in [FontFamily::Proportional, FontFamily::Monospace] {
            let names = family(kind.clone());
            let color = at(names, "color-emoji").unwrap();
            assert_eq!(
                Some(color + 1),
                at(names, "NotoEmoji-Regular"),
                "{kind:?}: {names:?}"
            );
        }
        assert_eq!(
            family(FontFamily::Name(super::EMOJI_FAMILY.into()))[0],
            "color-emoji"
        );
    }

    #[test]
    fn system_faces_precede_egui_emoji_fonts_in_both_families() {
        use eframe::egui::FontFamily;
        let fonts = super::chrome_fonts(None);
        for kind in [FontFamily::Proportional, FontFamily::Monospace] {
            let names = &fonts.families[&kind];
            // The faces installed here, in the order each family must try them.
            let order: Vec<_> = ["cjk", "symbols", "color-emoji", "NotoEmoji-Regular"]
                .into_iter()
                .filter(|name| fonts.font_data.contains_key(*name))
                .map(|name| names.iter().position(|n| n == name))
                .collect();
            assert!(
                order.iter().all(Option::is_some) && order.is_sorted(),
                "{kind:?}: {names:?}"
            );
        }
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn symbols_keep_their_monochrome_face_ahead_of_the_emoji_fonts() {
        use eframe::egui;
        let ctx = egui::Context::default();
        let mut fonts = super::chrome_fonts(None);
        fonts.families.insert(
            egui::FontFamily::Name("symbols-only".into()),
            vec!["symbols".into()],
        );
        ctx.set_fonts(fonts);
        let _ = crate::test_support::run(&ctx, egui::RawInput::default(), |ui| {
            for family in [
                egui::FontFamily::Proportional,
                egui::FontFamily::Name(super::TEXT_FAMILY.into()),
            ] {
                // Apple Symbols has the gear, as do both emoji fonts.
                let galley = ui.painter().layout_no_wrap(
                    "⚙".into(),
                    egui::FontId::new(14.0, family.clone()),
                    egui::Color32::WHITE,
                );
                let glyph = &galley.rows[0].glyphs[0];
                assert!(!glyph.is_color, "{family:?}");
                let face = |glyph: &egui::epaint::text::Glyph| {
                    (glyph.advance_width, glyph.font_face_height)
                };
                assert_eq!(
                    face(glyph),
                    face(
                        &ui.painter()
                            .layout_no_wrap(
                                "⚙".into(),
                                egui::FontId::new(
                                    14.0,
                                    egui::FontFamily::Name("symbols-only".into())
                                ),
                                egui::Color32::WHITE,
                            )
                            .rows[0]
                            .glyphs[0]
                    ),
                    "{family:?} draws the gear with the symbols face"
                );
            }
        });
    }

    #[test]
    fn palette_button_matches_tab_ink_and_shows_its_open_state() {
        use eframe::egui::{self, epaint::Shape};
        for font_size in [4.0, 10.0, 14.0, 24.0, 48.0, 72.0] {
            for density in [1.0, 1.1, 1.25, 1.5, 2.0] {
                let ctx = egui::Context::default();
                ctx.set_pixels_per_point(density);
                ctx.set_fonts(super::chrome_fonts(None));
                ctx.set_theme(egui::Theme::Dark);
                ctx.style_mut_of(egui::Theme::Dark, |style| {
                    for text_style in [egui::TextStyle::Body, egui::TextStyle::Button] {
                        style
                            .text_styles
                            .insert(text_style, egui::FontId::proportional(font_size));
                    }
                });
                for open in [false, true] {
                    let mut button_rect = egui::Rect::NOTHING;
                    let frame = crate::test_support::run(&ctx, egui::RawInput::default(), |ui| {
                        ui.spacing_mut().button_padding = egui::vec2(15.0, 6.0);
                        ui.spacing_mut().interact_size.y = super::ui_extent(ui.style(), 30.0).max(
                            ui.text_style_height(&egui::TextStyle::Button)
                                + 2.0 * ui.spacing().button_padding.y,
                        );
                        ui.horizontal(|ui| {
                            button_rect = super::palette_button(ui, open).rect;
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
                            Shape::Mesh(mesh) if button_rect.contains_rect(mesh.calc_bounds()) => {
                                Some(mesh)
                            }
                            _ => None,
                        })
                        .collect();
                    assert_eq!(
                        icons.len(),
                        1,
                        "the palette button must show the nekomata image"
                    );
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
                        "palette button optical offset: font={font_size}, density={density}, icon={icon_rect:?}, title_center={title_center}, button={button_rect:?}"
                    );
                    assert!((icon_rect.center().x - button_rect.center().x).abs() <= 0.5 / density);
                    let highlighted = frame.shapes.iter().any(|shape| matches!(&shape.shape,
                    Shape::Rect(rect) if rect.rect == button_rect && rect.fill == super::TAB_ACTIVE_BG));
                    assert_eq!(
                        highlighted, open,
                        "the palette button highlight must follow the palette"
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
            ctx.set_fonts(super::chrome_fonts(None));
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
        ctx.set_fonts(super::chrome_fonts(None));
        let _ = crate::test_support::run(&ctx, egui::RawInput::default(), |ui| {
            assert!(ui.fonts_mut(|fonts| fonts.has_glyphs(
                &egui::FontId::proportional(14.0),
                "⠇ 確認する felis設定の読み込み | nekomata"
            )));
        });
    }
    #[test]
    fn tab_titles_cover_the_status_marker_agents_put_before_them() {
        use eframe::egui;
        let ctx = egui::Context::default();
        ctx.set_fonts(super::chrome_fonts(None));
        let family = |name: &str| egui::FontId::new(14.0, egui::FontFamily::Name(name.into()));
        let _ = crate::test_support::run(&ctx, egui::RawInput::default(), |ui| {
            // Claude Code titles an idle session "✳ <topic>". Tabs lay its
            // text out in the text family, which has no color emoji font.
            assert!(
                ui.fonts_mut(|fonts| fonts.has_glyphs(&family(super::TEXT_FAMILY), "✳ Kakehashi"))
            );
            assert!(ui.fonts_mut(|fonts| fonts.has_glyphs(&family(super::EMOJI_FAMILY), "😀")));
        });
    }

    #[test]
    fn every_title_keeps_its_tab_one_row_tall() {
        use eframe::egui;
        let ctx = egui::Context::default();
        ctx.set_fonts(super::chrome_fonts(None));
        let mut heights = Vec::new();
        let _ = crate::test_support::run(&ctx, egui::RawInput::default(), |ui| {
            for title in ["", "😀", "neko\nmata", "nekomata"] {
                heights.push(super::tab_button(ui, title, false).rect.height());
            }
        });
        assert!(
            heights.iter().all(|height| *height == heights[3]),
            "a title without text or with only emoji must not shrink its tab, \
             nor a newline grow it: {heights:?}"
        );
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn an_emoji_does_not_heighten_a_tab_under_a_short_configured_face() {
        use eframe::egui;
        // Helvetica's line is shorter than the color emoji font's.
        let face = crate::ui_font::find("Helvetica").unwrap();
        let ctx = egui::Context::default();
        ctx.set_fonts(super::chrome_fonts(Some(face)));
        let mut heights = Vec::new();
        for _ in 0..2 {
            heights.clear();
            let _ = crate::test_support::run(&ctx, egui::RawInput::default(), |ui| {
                for title in ["abc", "abc 😀", "😀", ""] {
                    heights.push(super::tab_button(ui, title, false).rect.height());
                }
            });
        }
        assert!(
            heights.iter().all(|height| *height == heights[0]),
            "{heights:?}"
        );
    }

    #[test]
    fn tabs_show_control_characters_in_titles_as_spaces() {
        use eframe::egui;
        // egui's bundled faces alone, as on a system without the optional
        // fonts, draw a missing-glyph box for a newline.
        let ctx = egui::Context::default();
        let shown: Vec<_> = crate::test_support::run(&ctx, egui::RawInput::default(), |ui| {
            super::tab_button(ui, "build\nfailed\r\tagain", false);
        })
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
            _ => None,
        })
        .collect();
        assert_eq!(shown, ["build failed  again"]);
    }

    #[test]
    fn presented_text_gives_emoji_clusters_the_emoji_family() {
        use eframe::egui::FontFamily;
        let job = super::presented("✳ ✳\u{FE0F}😀 ok", 14.0, 21.0, super::FG);
        let sections: Vec<_> = job
            .sections
            .iter()
            .map(|section| {
                (
                    &job.text[section.byte_range.start.0..section.byte_range.end.0],
                    section.format.font_id.family.clone(),
                )
            })
            .collect();
        let family = |name: &str| FontFamily::Name(name.into());
        assert_eq!(
            sections,
            [
                ("✳ ", family(super::TEXT_FAMILY)),
                ("✳\u{FE0F}😀", family(super::EMOJI_FAMILY)),
                (" ok", family(super::TEXT_FAMILY)),
            ],
            "an emoji presentation selector makes its whole cluster emoji"
        );
    }

    #[test]
    fn emoji_joined_without_selectors_are_emoji() {
        use eframe::egui::FontFamily;
        let job = super::presented("👁 👁\u{200D}🗨⛹\u{200D}♀ क्\u{200D}ष", 14.0, 21.0, super::FG);
        let sections: Vec<_> = job
            .sections
            .iter()
            .map(|section| {
                (
                    &job.text[section.byte_range.start.0..section.byte_range.end.0],
                    section.format.font_id.family.clone(),
                )
            })
            .collect();
        let family = |name: &str| FontFamily::Name(name.into());
        assert_eq!(
            sections,
            [
                ("👁 ", family(super::TEXT_FAMILY)),
                ("👁\u{200D}🗨⛹\u{200D}♀", family(super::EMOJI_FAMILY)),
                (" क्\u{200D}ष", family(super::TEXT_FAMILY)),
            ],
            "a lone text-default symbol stays text, and so does a script's own joiner"
        );
    }

    #[test]
    fn keycaps_are_emoji_with_or_without_a_selector() {
        use eframe::egui::FontFamily;
        let job = super::presented(
            "1 1\u{20E3}#\u{FE0F}\u{20E3} *2\u{FE0E}\u{20E3}",
            14.0,
            21.0,
            super::FG,
        );
        let sections: Vec<_> = job
            .sections
            .iter()
            .map(|section| {
                (
                    &job.text[section.byte_range.start.0..section.byte_range.end.0],
                    section.format.font_id.family.clone(),
                )
            })
            .collect();
        let family = |name: &str| FontFamily::Name(name.into());
        assert_eq!(
            sections,
            [
                ("1 ", family(super::TEXT_FAMILY)),
                ("1\u{20E3}#\u{FE0F}\u{20E3}", family(super::EMOJI_FAMILY)),
                (" *2\u{FE0E}\u{20E3}", family(super::TEXT_FAMILY)),
            ],
            "a text presentation selector keeps a keycap text"
        );
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn tab_titles_draw_emoji_in_color_and_symbols_in_the_title_ink() {
        use eframe::egui;
        let ctx = egui::Context::default();
        ctx.set_fonts(super::chrome_fonts(None));
        for (selected, ink) in [(true, super::FG), (false, super::TAB_INACTIVE_FG)] {
            let mut glyphs = Vec::new();
            let mut inks = Vec::new();
            for _ in 0..2 {
                let output = crate::test_support::run(&ctx, egui::RawInput::default(), |ui| {
                    super::tab_button(ui, "✳ 😀 確認", selected);
                });
                (glyphs, inks) = (Vec::new(), Vec::new());
                for shape in &output.shapes {
                    if let egui::Shape::Text(text) = &shape.shape {
                        glyphs.extend(
                            text.galley
                                .rows
                                .iter()
                                .flat_map(|row| row.glyphs.iter())
                                .filter(|glyph| !glyph.chr.is_whitespace())
                                .map(|glyph| (glyph.chr, glyph.is_color)),
                        );
                        inks.extend(
                            text.galley
                                .job
                                .sections
                                .iter()
                                .map(|section| section.format.color),
                        );
                    }
                }
            }
            assert_eq!(
                glyphs,
                [('✳', false), ('😀', true), ('確', false), ('認', false)],
                "emoji keep their own colors, while a symbol that defaults to \
                 text presentation, like the ✳ Claude Code puts before its \
                 titles, is an outline the tab's ink colors"
            );
            assert!(
                !inks.is_empty() && inks.iter().all(|color| *color == ink),
                "selected={selected}: {inks:?}"
            );
        }
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn emoji_in_presented_titles_sit_where_plain_text_places_them() {
        use eframe::egui;
        let ctx = egui::Context::default();
        ctx.set_fonts(super::chrome_fonts(None));
        let _ = crate::test_support::run(&ctx, egui::RawInput::default(), |ui| {
            let title = "H😀確";
            let plain = ui.painter().layout_no_wrap(
                title.into(),
                egui::FontId::proportional(14.0),
                egui::Color32::WHITE,
            );
            let presented = ui.painter().layout_job(super::presented(
                title,
                14.0,
                super::title_line_height(ui, 14.0),
                egui::Color32::WHITE,
            ));
            let tops = |galley: &egui::Galley| {
                galley.rows[0]
                    .glyphs
                    .iter()
                    .map(|glyph| (glyph.chr, glyph.pos.y))
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                tops(&presented),
                tops(&plain),
                "the color emoji font's shorter line must not push emoji down"
            );
            assert_eq!(presented.size().y, plain.size().y);
        });
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn tab_titles_share_a_baseline_across_japanese_and_latin() {
        use eframe::egui;
        let ctx = egui::Context::default();
        ctx.set_fonts(super::chrome_fonts(None));
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
