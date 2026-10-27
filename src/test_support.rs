//! CPU-only egui tests intentionally discard GPU texture updates.
use eframe::egui;
use std::ops::Deref;

pub struct CpuFrame(egui::FullOutput);
impl Deref for CpuFrame {
    type Target = egui::FullOutput;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl Drop for CpuFrame {
    fn drop(&mut self) {
        self.0.textures_delta.clear();
    }
}
impl CpuFrame {
    pub fn tessellate(mut self, ctx: &egui::Context) -> Vec<egui::ClippedPrimitive> {
        ctx.tessellate(std::mem::take(&mut self.0.shapes), self.0.pixels_per_point)
    }
}
pub fn run(ctx: &egui::Context, input: egui::RawInput, ui: impl FnMut(&mut egui::Ui)) -> CpuFrame {
    CpuFrame(ctx.run_ui(input, ui))
}

/// The private felis socket `scripts/test-live.sh` provisions. Live tests
/// call this first so a test run outside the runner, which would otherwise
/// reach the user's normal daemon through `FELIS_SOCKET`, fails instead.
pub fn private_felis_socket() -> String {
    let socket = std::env::var("NEKOMATA_TEST_SOCKET").expect("use scripts/test-live.sh");
    assert_eq!(
        std::env::var("FELIS_SOCKET").ok().as_deref(),
        Some(socket.as_str()),
        "FELIS_SOCKET must name the private test daemon"
    );
    socket
}

/// The private GUI runtime directory `scripts/test-live.sh` provisions, which
/// holds the nekomata daemon and window control sockets of live tests. The
/// runner creates it beside the private felis socket; requiring that keeps a
/// runtime directory from the user's own environment from being used.
pub fn private_runtime_dir() -> std::path::PathBuf {
    let dir = std::path::PathBuf::from(
        std::env::var_os("NEKOMATA_RUNTIME_DIR").expect("use scripts/test-live.sh"),
    );
    let socket = std::path::PathBuf::from(private_felis_socket());
    assert_eq!(
        dir.parent(),
        socket.parent(),
        "NEKOMATA_RUNTIME_DIR must sit beside the private test socket"
    );
    dir
}
