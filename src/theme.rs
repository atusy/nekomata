use eframe::egui::Color32;
use felis_protocol::messages::SessionToDaemonMsg;

pub const BG: Color32 = Color32::from_rgb(20, 23, 31);
pub const FG: Color32 = Color32::from_rgb(216, 222, 233);
pub const ACCENT: Color32 = Color32::from_rgb(163, 190, 140);
pub const TAB_BAR_BG: Color32 = Color32::from_rgb(24, 28, 37);
pub const TAB_ACTIVE_BG: Color32 = Color32::from_rgb(48, 55, 68);
pub const TAB_HOVER_BG: Color32 = Color32::from_rgb(36, 43, 54);
pub const TAB_INACTIVE_FG: Color32 = Color32::from_rgb(140, 149, 167);

pub fn configure() -> SessionToDaemonMsg {
    let rgb = |color: Color32| (color.r(), color.g(), color.b());
    SessionToDaemonMsg::ConfigureTheme {
        fg: Some(rgb(FG)),
        bg: Some(rgb(BG)),
        cursor: Some(rgb(ACCENT)),
    }
}
