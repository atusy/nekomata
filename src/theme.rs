use std::{collections::BTreeMap, sync::Arc};

use eframe::egui::{self, Color32};
use felis_client_core::EffectiveConfig;
use felis_protocol::messages::SessionToDaemonMsg;
use felis_render_core::palette::parse_hex_color;

pub const BG: Color32 = Color32::from_rgb(20, 23, 31);
pub const FG: Color32 = Color32::from_rgb(216, 222, 233);
pub const ACCENT: Color32 = Color32::from_rgb(163, 190, 140);
pub const TAB_BAR_BG: Color32 = Color32::from_rgb(24, 28, 37);
pub const TAB_ACTIVE_BG: Color32 = Color32::from_rgb(48, 55, 68);
pub const TAB_HOVER_BG: Color32 = Color32::from_rgb(36, 43, 54);
pub const TAB_INACTIVE_FG: Color32 = Color32::from_rgb(140, 149, 167);

pub const ANSI_PALETTE: [[u8; 3]; 16] = [
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

/// Colors that felis paints terminal cells with and reports to programs.
#[derive(Debug, Clone, PartialEq)]
pub struct TerminalTheme {
    fg: [u8; 3],
    bg: [u8; 3],
    cursor: [u8; 3],
    palette: BTreeMap<u8, [u8; 3]>,
    minimum_contrast: f32,
}

impl Default for TerminalTheme {
    fn default() -> Self {
        Self {
            fg: rgb(FG),
            bg: rgb(BG),
            cursor: rgb(ACCENT),
            palette: (0..).zip(ANSI_PALETTE).collect(),
            minimum_contrast: 1.0,
        }
    }
}

fn rgb(color: Color32) -> [u8; 3] {
    [color.r(), color.g(), color.b()]
}

impl TerminalTheme {
    /// Applies `[theme]` and `[cursor] color` over nekomata's own colors.
    ///
    /// Unset values keep nekomata's defaults rather than felis's, and an unset
    /// cursor keeps the accent color rather than felis's reverse video.
    /// felis validates neither hex strings nor reports them, so malformed
    /// colors are reported through `warn` and keep their default.
    pub fn from_config(config: &EffectiveConfig, warn: &mut dyn FnMut(String)) -> Self {
        let mut theme = Self::default();
        let mut color = |key: &str, value: Option<&str>, slot: &mut [u8; 3]| {
            let Some(value) = value else { return };
            match parse_hex_color(value) {
                Some(rgb) => *slot = rgb,
                None => warn(format!(
                    "{key}: {value:?} is not #rrggbb; using the default"
                )),
            }
        };
        color(
            "theme.foreground",
            config.theme.foreground.as_deref(),
            &mut theme.fg,
        );
        color(
            "theme.background",
            config.theme.background.as_deref(),
            &mut theme.bg,
        );
        color(
            "cursor.color",
            config.cursor.color.as_deref(),
            &mut theme.cursor,
        );
        for (index, value) in config.theme.palette.clone().into_overrides() {
            match parse_hex_color(&value) {
                Some(rgb) => {
                    theme.palette.insert(index, rgb);
                }
                None => warn(format!(
                    "theme.palette slot {index}: {value:?} is not #rrggbb; using the default"
                )),
            }
        }
        theme.minimum_contrast = config.theme.clamped_minimum_contrast();
        theme
    }

    pub fn foreground(&self) -> Color32 {
        let [r, g, b] = self.fg;
        Color32::from_rgb(r, g, b)
    }

    pub fn background(&self) -> Color32 {
        let [r, g, b] = self.bg;
        Color32::from_rgb(r, g, b)
    }

    /// Whether programs should pick colors for a dark background.
    pub fn is_dark(&self) -> bool {
        use felis_render_core::{contrast::contrast_ratio, palette::srgb_to_linear_rgba};
        let bg = srgb_to_linear_rgba(self.bg, 1.0);
        contrast_ratio(bg, [1.0; 4]) > contrast_ratio(bg, [0.0, 0.0, 0.0, 1.0])
    }

    /// The renderer's base theme, before programs override colors at runtime.
    pub fn render(&self) -> felis_render_core::palette::Theme {
        use felis_render_core::palette::{Theme, srgb_to_linear_rgba};
        let mut theme = Theme {
            fg: srgb_to_linear_rgba(self.fg, 1.0),
            bg: srgb_to_linear_rgba(self.bg, 1.0),
            cursor: Some(srgb_to_linear_rgba(self.cursor, 1.0)),
            minimum_contrast: self.minimum_contrast,
            ..Theme::default()
        };
        for (&index, &rgb) in &self.palette {
            theme.palette[usize::from(index)] = Some(rgb);
        }
        theme
    }

    /// Lets felis answer `OSC 4/10/11/12` queries with the painted colors.
    pub fn configure(&self) -> SessionToDaemonMsg {
        let tuple = |[r, g, b]: [u8; 3]| (r, g, b);
        SessionToDaemonMsg::ConfigureTheme {
            fg: Some(tuple(self.fg)),
            bg: Some(tuple(self.bg)),
            cursor: Some(tuple(self.cursor)),
            palette: self
                .palette
                .iter()
                .map(|(&index, &rgb)| (index, tuple(rgb)))
                .collect(),
        }
    }
}

fn theme_id() -> egui::Id {
    egui::Id::unique("nekomata-terminal-theme")
}

/// Shares the window's terminal theme with every pane and felis connection.
pub fn install(ctx: &egui::Context, theme: TerminalTheme) {
    ctx.data_mut(|data| data.insert_temp(theme_id(), Arc::new(theme)));
}

pub fn current(ctx: &egui::Context) -> Arc<TerminalTheme> {
    ctx.data(|data| data.get_temp(theme_id()))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn themed(text: &str) -> (TerminalTheme, Vec<String>) {
        let loaded = crate::config::resolve(text);
        let warnings = loaded
            .diagnostics
            .iter()
            .map(|line| {
                line.strip_prefix("nekomata: /config/felis/config.toml: warning: ")
                    .unwrap_or(line)
                    .to_owned()
            })
            .collect();
        (loaded.theme, warnings)
    }

    #[test]
    fn an_empty_config_keeps_nekomatas_colors() {
        let (theme, warnings) = themed("");
        assert_eq!(theme, TerminalTheme::default());
        assert!(warnings.is_empty());
        assert!(theme.is_dark());
    }

    #[test]
    fn configured_colors_reach_the_renderer_and_the_daemon_report() {
        let (theme, warnings) = themed(
            r##"
            [theme]
            foreground = "#101112"
            background = "#fafbfc"
            minimum_contrast = 4.5

            [theme.palette]
            red = "#aa0000"
            indexed = { "100" = "#010203" }

            [cursor]
            color = "#00ff00"
            "##,
        );
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(theme.foreground(), Color32::from_rgb(0x10, 0x11, 0x12));
        assert_eq!(theme.background(), Color32::from_rgb(0xfa, 0xfb, 0xfc));
        assert!(!theme.is_dark());
        let render = theme.render();
        assert_eq!(render.palette[1], Some([0xaa, 0, 0]));
        assert_eq!(render.palette[2], Some(ANSI_PALETTE[2]));
        assert_eq!(render.palette[100], Some([1, 2, 3]));
        assert_eq!(render.palette[101], None);
        assert_eq!(render.minimum_contrast, 4.5);
        let SessionToDaemonMsg::ConfigureTheme {
            fg,
            bg,
            cursor,
            palette,
        } = theme.configure()
        else {
            panic!("not a theme message");
        };
        assert_eq!(fg, Some((0x10, 0x11, 0x12)));
        assert_eq!(bg, Some((0xfa, 0xfb, 0xfc)));
        assert_eq!(cursor, Some((0, 0xff, 0)));
        assert_eq!(palette.get(&1), Some(&(0xaa, 0, 0)));
        assert_eq!(palette.get(&100), Some(&(1, 2, 3)));
        assert_eq!(palette.len(), 17);
    }

    #[test]
    fn malformed_colors_warn_and_keep_their_defaults() {
        let (theme, warnings) = themed(
            r##"
            [theme]
            background = "navy"
            foreground = "#123456"

            [theme.palette]
            blue = "#12345"

            [cursor]
            color = "green"
            "##,
        );
        assert_eq!(theme.background(), BG);
        assert_eq!(theme.foreground(), Color32::from_rgb(0x12, 0x34, 0x56));
        assert_eq!(theme.render().palette[4], Some(ANSI_PALETTE[4]));
        assert_eq!(
            theme.render().cursor,
            TerminalTheme::default().render().cursor
        );
        assert_eq!(
            warnings,
            [
                "theme.background: \"navy\" is not #rrggbb; using the default",
                "cursor.color: \"green\" is not #rrggbb; using the default",
                "theme.palette slot 4: \"#12345\" is not #rrggbb; using the default",
            ]
        );
    }
}
