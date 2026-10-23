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
