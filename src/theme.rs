use eframe::egui::Color32;
use felis_protocol::messages::SessionToDaemonMsg;

pub const BG: Color32 = Color32::from_rgb(20, 23, 31);
pub const FG: Color32 = Color32::from_rgb(216, 222, 233);
pub const ACCENT: Color32 = Color32::from_rgb(163, 190, 140);

pub fn configure() -> SessionToDaemonMsg {
    let rgb = |color: Color32| (color.r(), color.g(), color.b());
    SessionToDaemonMsg::ConfigureTheme {
        fg: Some(rgb(FG)),
        bg: Some(rgb(BG)),
        cursor: Some(rgb(ACCENT)),
    }
}
