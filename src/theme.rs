use eframe::egui::Color32;
use felis_protocol::messages::SessionToDaemonMsg;

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

pub fn terminal_theme() -> felis_render_core::palette::Theme {
    use felis_render_core::palette::{Theme, srgb_to_linear_rgba};
    let linear = |color: Color32| srgb_to_linear_rgba([color.r(), color.g(), color.b()], 1.0);
    let mut theme = Theme {
        fg: linear(FG),
        bg: linear(BG),
        cursor: Some(linear(ACCENT)),
        ..Theme::default()
    };
    for (slot, rgb) in theme.palette.iter_mut().zip(ANSI_PALETTE) {
        *slot = Some(rgb);
    }
    theme
}

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

pub fn configure() -> SessionToDaemonMsg {
    let rgb = |color: Color32| (color.r(), color.g(), color.b());
    SessionToDaemonMsg::ConfigureTheme {
        fg: Some(rgb(FG)),
        bg: Some(rgb(BG)),
        cursor: Some(rgb(ACCENT)),
        palette: ANSI_PALETTE
            .into_iter()
            .enumerate()
            .map(|(index, [r, g, b])| (index as u8, (r, g, b)))
            .collect(),
    }
}
