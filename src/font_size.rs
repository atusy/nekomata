//! The size control that `Font: Change size…` opens.
use crate::{shortcuts::Action, theme};
use eframe::egui;

/// The factor one larger or smaller step multiplies the size by.
pub(crate) const STEP: f32 = 1.1;

/// Returns the size change chosen this frame and whether the control was
/// dismissed. `size` is the current terminal font size in points.
pub(crate) fn show(ctx: &egui::Context, size: f32) -> (Option<Action>, bool) {
    let (up, down) = ctx.input_mut(|input| {
        (
            input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
            input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
        )
    });
    let mut chosen = if up {
        Some(Action::FontScale(STEP))
    } else if down {
        Some(Action::FontScale(1.0 / STEP))
    } else {
        None
    };
    let id = egui::Id::unique("font_size");
    let top = (ctx.content_rect().height() * 0.12).min(64.0);
    let response = egui::Modal::new(id)
        .area(egui::Modal::default_area(id).anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, top)))
        // The text behind must stay visible while it resizes.
        .backdrop_color(egui::Color32::TRANSPARENT)
        .frame(theme::overlay_frame())
        .show(ctx, |ui| {
            theme::menu_style(ui.style_mut());
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Font size").color(theme::TAB_INACTIVE_FG));
                ui.add_space(theme::ui_extent(ui.style(), 12.0));
                let row = theme::ui_extent(ui.style(), 28.0);
                let narrow = theme::ui_extent(ui.style(), 30.0);
                let wide = theme::ui_extent(ui.style(), 52.0);
                let button = |text| egui::Button::new(text).fill(theme::TAB_HOVER_BG);
                if ui
                    .add_sized([narrow, row], button("−"))
                    .on_hover_text("Smaller · ↓")
                    .clicked()
                {
                    chosen = Some(Action::FontScale(1.0 / STEP));
                }
                ui.add_sized([wide, row], egui::Label::new(format!("{size:.0} pt")));
                if ui
                    .add_sized([narrow, row], button("+"))
                    .on_hover_text("Larger · ↑")
                    .clicked()
                {
                    chosen = Some(Action::FontScale(STEP));
                }
                ui.add_space(theme::ui_extent(ui.style(), 8.0));
                if ui
                    .add_sized([wide, row], button("Reset"))
                    .on_hover_text("Back to the configured size")
                    .clicked()
                {
                    chosen = Some(Action::FontReset);
                }
            });
        });
    (chosen, response.should_close())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(ctx: &egui::Context, keys: &[egui::Key]) -> (Option<Action>, bool) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            events: keys
                .iter()
                .map(|key| egui::Event::Key {
                    key: *key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                })
                .collect(),
            ..Default::default()
        };
        let mut result = (None, false);
        let _ = crate::test_support::run(ctx, input, |ui| result = show(ui.ctx(), 13.0));
        result
    }

    #[test]
    fn arrow_keys_step_the_size() {
        let ctx = egui::Context::default();
        frame(&ctx, &[]);
        assert_eq!(
            frame(&ctx, &[egui::Key::ArrowUp]),
            (Some(Action::FontScale(STEP)), false)
        );
        assert_eq!(
            frame(&ctx, &[egui::Key::ArrowDown]),
            (Some(Action::FontScale(1.0 / STEP)), false)
        );
    }

    #[test]
    fn escape_closes_without_changing_the_size() {
        let ctx = egui::Context::default();
        frame(&ctx, &[]);
        assert_eq!(frame(&ctx, &[egui::Key::Escape]), (None, true));
    }
}
