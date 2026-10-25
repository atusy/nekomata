use crate::{layout::Axis, shortcuts::Action, theme};
use eframe::egui;

#[derive(Default)]
pub struct CommandPalette {
    query: String,
    selected: usize,
}

fn commands() -> Vec<(&'static str, Action)> {
    vec![
        ("Find in scrollback", Action::Search),
        ("New workspace", Action::NewWorkspace),
        ("Switch workspace", Action::OpenWorkspaces),
        ("New tab", Action::NewTab),
        ("Split left / right", Action::Split(Axis::Horizontal)),
        ("Split top / bottom", Action::Split(Axis::Vertical)),
        ("Next tab", Action::NextTab),
        ("Previous tab", Action::PreviousTab),
        ("Close pane (detach)", Action::ClosePane),
        ("Close tab (detach)", Action::CloseTab),
        ("Reset font size", Action::FontReset),
        (
            "Previous prompt",
            Action::Terminal(felis_client_core::Action::ScrollToPrompt(
                felis_protocol::messages::PromptJump::Previous,
            )),
        ),
        (
            "Next prompt",
            Action::Terminal(felis_client_core::Action::ScrollToPrompt(
                felis_protocol::messages::PromptJump::Next,
            )),
        ),
        (
            "Scroll to oldest output",
            Action::Terminal(felis_client_core::Action::Scroll(
                felis_client_core::ScrollStep::Home,
            )),
        ),
        (
            "Scroll to live output",
            Action::Terminal(felis_client_core::Action::Scroll(
                felis_client_core::ScrollStep::End,
            )),
        ),
    ]
}

// Terminal bindings can be overridden in felis configuration. Only display
// the workspace shortcuts whose bindings are owned by nekomata.
fn shortcut(action: &Action) -> &'static str {
    match action {
        Action::NewTab => "Alt+C",
        Action::Split(Axis::Horizontal) => "Alt+W  Alt+V",
        Action::Split(Axis::Vertical) => "Alt+W  Alt+S",
        Action::NextTab => "Alt+T",
        Action::PreviousTab => "Alt+Shift+T",
        Action::ClosePane => "Alt+W  C",
        _ => "",
    }
}

impl CommandPalette {
    pub fn show(&mut self, ctx: &egui::Context) -> (Option<Action>, bool) {
        let (up, down, enter) = ctx.input_mut(|input| {
            (
                input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                input.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
            )
        });
        let viewport = ctx.content_rect();
        let top = (viewport.height() * 0.12).min(64.0);
        let width = (viewport.width() - 48.0).clamp(100.0, theme::PALETTE_WIDTH);
        let list_height = (viewport.height() - top - 88.0).clamp(32.0, theme::PALETTE_LIST_HEIGHT);
        let mut chosen = None;
        let id = egui::Id::unique("command_palette");
        let response = egui::Modal::new(id)
            .area(
                egui::Modal::default_area(id)
                    .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, top)),
            )
            .frame(
                egui::Frame::new()
                    .shadow(egui::epaint::Shadow {
                        offset: [0, 8],
                        blur: 24,
                        spread: 0,
                        color: egui::Color32::from_black_alpha(100),
                    })
                    .fill(theme::TAB_BAR_BG)
                    .stroke(egui::Stroke::new(1.0, theme::MENU_BORDER))
                    .corner_radius(10)
                    .inner_margin(12),
            )
            .show(ctx, |ui| {
                theme::menu_style(ui.style_mut());
                ui.visuals_mut().weak_text_color = Some(theme::TAB_INACTIVE_FG);
                ui.style_mut().visuals.selection.bg_fill = theme::TAB_ACTIVE_BG;
                ui.style_mut().visuals.selection.stroke = egui::Stroke::new(1.0, theme::FG);
                ui.set_width(width);
                let search = ui.add_sized(
                    [width, 32.0],
                    egui::TextEdit::singleline(&mut self.query)
                        .hint_text("Search commands…")
                        .font(egui::FontId::proportional(theme::ui_size(ui.style(), 14.0)))
                        .margin(egui::vec2(10.0, 7.0)),
                );
                if !search.has_focus() {
                    search.request_focus();
                }
                let query = self.query.trim().to_lowercase();
                let matches: Vec<_> = commands()
                    .into_iter()
                    .filter(|(label, _)| label.to_lowercase().contains(&query))
                    .collect();
                if search.changed() {
                    self.selected = 0;
                }
                self.selected = self.selected.min(matches.len().saturating_sub(1));
                if !matches.is_empty() {
                    if down {
                        self.selected = (self.selected + 1) % matches.len();
                    }
                    if up {
                        self.selected = (self.selected + matches.len() - 1) % matches.len();
                    }
                }
                ui.add_space(6.0);
                egui::ScrollArea::vertical()
                    .id_salt("command_results")
                    .max_height(list_height)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if matches.is_empty() {
                            ui.add_space(16.0);
                            ui.label(
                                egui::RichText::new("No matching commands")
                                    .color(theme::TAB_INACTIVE_FG),
                            );
                        }
                        for (index, (label, action)) in matches.iter().enumerate() {
                            let selected = index == self.selected;
                            let row = ui.add_sized(
                                [ui.available_width(), theme::PALETTE_ROW_HEIGHT],
                                egui::Button::new(*label)
                                    .shortcut_text(shortcut(action))
                                    .selected(selected),
                            );
                            if selected && (up || down || search.changed()) {
                                row.scroll_to_me(Some(egui::Align::Center));
                            }
                            if row.clicked() || (enter && selected) {
                                chosen = Some(action.clone());
                            }
                        }
                    });
            });
        let dismissed = response.should_close();
        (if dismissed { None } else { chosen }, dismissed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame(
        ctx: &egui::Context,
        palette: &mut CommandPalette,
        keys: &[egui::Key],
    ) -> (Option<Action>, bool) {
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
        let _ = crate::test_support::run(ctx, input, |ui| result = palette.show(ui.ctx()));
        result
    }
    #[test]
    fn arrow_selection_is_activated_by_enter() {
        let ctx = egui::Context::default();
        let mut palette = CommandPalette::default();
        frame(&ctx, &mut palette, &[]);
        frame(&ctx, &mut palette, &[egui::Key::ArrowDown]);
        assert_eq!(
            frame(&ctx, &mut palette, &[egui::Key::Enter]).0,
            Some(Action::NewWorkspace)
        );
    }
    #[test]
    fn typing_filters_commands_and_resets_keyboard_selection() {
        let ctx = egui::Context::default();
        let mut palette = CommandPalette::default();
        frame(&ctx, &mut palette, &[]);
        frame(&ctx, &mut palette, &[]);
        frame(&ctx, &mut palette, &[egui::Key::ArrowDown]);
        let _ = crate::test_support::run(
            &ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 600.0),
                )),
                events: vec![egui::Event::Text(" TAB ".into())],
                ..Default::default()
            },
            |ui| {
                palette.show(ui.ctx());
            },
        );
        assert_eq!(palette.query, " TAB ");
        assert_eq!(
            frame(&ctx, &mut palette, &[egui::Key::Enter]).0,
            Some(Action::NewTab)
        );
    }

    #[test]
    fn empty_results_do_not_execute_and_escape_is_consumed() {
        let ctx = egui::Context::default();
        let mut palette = CommandPalette {
            query: "no such command".into(),
            selected: 0,
        };
        frame(&ctx, &mut palette, &[]);
        assert_eq!(
            frame(
                &ctx,
                &mut palette,
                &[egui::Key::ArrowDown, egui::Key::Enter]
            ),
            (None, false)
        );
        assert_eq!(
            frame(&ctx, &mut palette, &[egui::Key::Escape]),
            (None, true)
        );
        assert!(!ctx.input(|input| input.key_pressed(egui::Key::Escape)));
    }

    #[test]
    fn navigating_above_first_result_wraps_to_last() {
        let ctx = egui::Context::default();
        let mut palette = CommandPalette::default();
        frame(&ctx, &mut palette, &[]);
        frame(&ctx, &mut palette, &[egui::Key::ArrowUp]);
        assert_eq!(
            frame(&ctx, &mut palette, &[egui::Key::Enter]).0,
            commands().pop().map(|(_, action)| action)
        );
    }

    #[test]
    fn palette_fits_small_and_large_viewports() {
        for size in [egui::vec2(520.0, 320.0), egui::vec2(800.0, 600.0)] {
            let ctx = egui::Context::default();
            let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
            let mut palette = CommandPalette::default();
            for _ in 0..3 {
                let _ = crate::test_support::run(
                    &ctx,
                    egui::RawInput {
                        screen_rect: Some(viewport),
                        ..Default::default()
                    },
                    |ui| {
                        palette.show(ui.ctx());
                    },
                );
            }
            let rect = ctx
                .memory(|memory| memory.area_rect(egui::Id::unique("command_palette")))
                .unwrap();
            assert!(
                viewport.contains_rect(rect),
                "viewport={viewport:?}, palette={rect:?}"
            );
        }
    }
}
