use crate::{shortcuts::Action, theme};
use eframe::egui;

#[derive(Default)]
pub struct CommandPalette {
    query: String,
    selected: usize,
}

use crate::commands::{Command, commands as public_commands};

impl Command {
    /// The label, with any detail on a dimmer second line.
    fn row_text(&self, style: &egui::Style) -> egui::text::LayoutJob {
        let mut job = egui::text::LayoutJob::default();
        job.append(
            &self.label(),
            0.0,
            egui::TextFormat::simple(
                egui::FontId::proportional(theme::ui_size(style, 14.0)),
                theme::FG,
            ),
        );
        if let Some(detail) = self.detail {
            job.append(
                &format!("\n{detail}"),
                0.0,
                egui::TextFormat::simple(
                    egui::FontId::proportional(theme::ui_size(style, 12.0)),
                    theme::TAB_INACTIVE_FG,
                ),
            );
        }
        job
    }
}

/// What the search field and the frame take from the window besides the
/// list.
const LIST_RESERVE: f32 = 88.0;

/// The window height at which the palette lists its full height of rows.
pub(crate) fn wanted_height(ctx: &egui::Context) -> f32 {
    let style = ctx.global_style();
    theme::OVERLAY_MAX_TOP
        + theme::ui_extent(&style, LIST_RESERVE)
        + theme::ui_extent(&style, theme::PALETTE_LIST_HEIGHT)
}

fn commands() -> Vec<Command> {
    public_commands()
        .iter()
        .filter(|command| command.visible)
        .cloned()
        .collect()
}

/// Every query word must appear in the label or the keywords, in any
/// order, because the `Category: verb` form fixes an order users may not
/// type.
fn matching(query: &str) -> Vec<Command> {
    let words: Vec<_> = query.split_whitespace().map(str::to_lowercase).collect();
    commands()
        .into_iter()
        .filter(|command| command.matches(&words))
        .collect()
}

impl CommandPalette {
    /// Hints cover application bindings; felis fallback bindings are not shown.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        hints: &crate::shortcuts::Hints,
        prompt_unavailable: Option<&'static str>,
    ) -> (Option<Action>, bool) {
        let (up, down, enter) = ctx.input_mut(|input| {
            (
                input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                input.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
            )
        });
        let viewport = ctx.content_rect();
        let top = theme::overlay_top(viewport.height());
        let style = ctx.global_style();
        let width =
            (viewport.width() - 48.0).clamp(100.0, theme::ui_extent(&style, theme::PALETTE_WIDTH));
        let row_height = theme::ui_extent(&style, theme::PALETTE_ROW_HEIGHT);
        let detail_row_height = theme::ui_extent(&style, theme::PALETTE_DETAIL_ROW_HEIGHT);
        let list_height = (viewport.height() - top - theme::ui_extent(&style, LIST_RESERVE)).clamp(
            row_height,
            theme::ui_extent(&style, theme::PALETTE_LIST_HEIGHT),
        );
        let mut chosen = None;
        let id = egui::Id::unique("command_palette");
        let response = egui::Modal::new(id)
            .area(
                egui::Modal::default_area(id)
                    .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, top)),
            )
            .frame(theme::overlay_frame())
            .show(ctx, |ui| {
                theme::menu_style(ui.style_mut());
                ui.visuals_mut().weak_text_color = Some(theme::TAB_INACTIVE_FG);
                ui.style_mut().visuals.selection.bg_fill = theme::TAB_ACTIVE_BG;
                ui.style_mut().visuals.selection.stroke = egui::Stroke::new(1.0, theme::FG);
                ui.set_width(width);
                let search = ui.add_sized(
                    [width, row_height],
                    egui::TextEdit::singleline(&mut self.query)
                        .hint_text("Search commands…")
                        .font(egui::FontId::proportional(theme::ui_size(ui.style(), 14.0)))
                        .align(egui::Align2::LEFT_CENTER)
                        .margin(egui::vec2(10.0, 7.0)),
                );
                if !search.has_focus() {
                    search.request_focus();
                }
                let matches = matching(&self.query);
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
                    .min_scrolled_height(list_height)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if matches.is_empty() {
                            ui.add_space(16.0);
                            ui.label(
                                egui::RichText::new("No matching commands")
                                    .color(theme::TAB_INACTIVE_FG),
                            );
                        }
                        for (index, mut command) in matches.into_iter().enumerate() {
                            let unavailable = if matches!(
                                command.action,
                                Action::Terminal(felis_client_core::Action::ScrollToPrompt(_))
                            ) {
                                prompt_unavailable
                            } else {
                                None
                            };
                            if let Some(reason) = unavailable {
                                command.detail = Some(reason);
                            }
                            let selected = index == self.selected;
                            let height = match command.detail {
                                Some(_) => detail_row_height,
                                None => row_height,
                            };
                            let row = ui
                                .add_enabled_ui(unavailable.is_none(), |ui| {
                                    ui.add_sized(
                                        [ui.available_width(), height],
                                        egui::Button::new(command.row_text(ui.style()))
                                            .shortcut_text(hints.get(&command.action))
                                            .selected(selected),
                                    )
                                })
                                .inner;
                            if selected && (up || down || search.changed()) {
                                row.scroll_to_me(Some(egui::Align::Center));
                            }
                            if unavailable.is_none() && (row.clicked() || (enter && selected)) {
                                chosen = Some(command.action.clone());
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
    use crate::layout::{Axis, Direction};
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
        let _ = crate::test_support::run(ctx, input, |ui| {
            result = palette.show(
                ui.ctx(),
                &crate::shortcuts::AppKeymap::default().hints(),
                None,
            )
        });
        result
    }
    #[test]
    fn wanted_height_follows_the_body_size() {
        let ctx = egui::Context::default();
        let small = wanted_height(&ctx);
        ctx.global_style_mut(|style| {
            style
                .text_styles
                .insert(egui::TextStyle::Body, egui::FontId::proportional(28.0));
        });
        assert!(wanted_height(&ctx) > 1.5 * small);
    }

    #[test]
    fn a_window_of_the_wanted_height_lists_as_many_rows_at_a_large_body_as_at_a_small_one() {
        let rows_at = |body: f32| {
            let ctx = egui::Context::default();
            ctx.global_style_mut(|style| {
                style
                    .text_styles
                    .insert(egui::TextStyle::Body, egui::FontId::proportional(body));
            });
            let height = wanted_height(&ctx);
            let mut palette = CommandPalette::default();
            let mut rows = 0;
            for _ in 0..2 {
                let output = crate::test_support::run(
                    &ctx,
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1200.0, height),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        palette.show(
                            ui.ctx(),
                            &crate::shortcuts::AppKeymap::default().hints(),
                            None,
                        );
                    },
                );
                rows = output
                    .shapes
                    .iter()
                    .filter(|shape| {
                        matches!(&shape.shape, egui::Shape::Text(text)
                            if text.galley.text().starts_with("Tab: "))
                    })
                    .count();
            }
            rows
        };
        assert!(rows_at(14.0) >= 4);
        assert_eq!(rows_at(28.0), rows_at(14.0));
    }

    #[test]
    fn the_search_text_is_centered_in_its_field_at_a_large_body() {
        let ctx = egui::Context::default();
        ctx.global_style_mut(|style| {
            style
                .text_styles
                .insert(egui::TextStyle::Body, egui::FontId::proportional(28.0));
        });
        let mut palette = CommandPalette::default();
        let mut hint_center = None;
        for _ in 0..2 {
            let output = crate::test_support::run(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1200.0, 800.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    palette.show(
                        ui.ctx(),
                        &crate::shortcuts::AppKeymap::default().hints(),
                        None,
                    );
                },
            );
            hint_center = output.shapes.iter().find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == "Search commands…" => {
                    Some(text.visual_bounding_rect().center().y)
                }
                _ => None,
            });
        }
        // The field starts below the overlay offset and the frame's margin.
        let field_top = theme::overlay_top(800.0) + 12.0;
        let field_center =
            field_top + theme::ui_extent(&ctx.global_style(), theme::PALETTE_ROW_HEIGHT) / 2.0;
        let offset = (hint_center.unwrap() - field_center).abs();
        assert!(offset <= 2.0, "{offset}");
    }

    #[test]
    fn unavailable_prompt_jumps_explain_why_and_recover_without_reopening() {
        use felis_client_core::Action::ScrollToPrompt;
        use felis_protocol::messages::PromptJump;
        for (query, direction) in [
            ("previous prompt", PromptJump::Previous),
            ("next prompt", PromptJump::Next),
        ] {
            for reason in [
                "Unavailable while disconnected",
                "Unavailable in the alternate screen",
            ] {
                let ctx = egui::Context::default();
                let mut palette = CommandPalette {
                    query: query.into(),
                    selected: 0,
                };
                frame(&ctx, &mut palette, &[]);
                let enter = egui::Event::Key {
                    key: egui::Key::Enter,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                };
                let mut result = (None, false);
                let output = crate::test_support::run(
                    &ctx,
                    egui::RawInput {
                        events: vec![enter],
                        ..Default::default()
                    },
                    |ui| {
                        result = palette.show(
                            ui.ctx(),
                            &crate::shortcuts::AppKeymap::default().hints(),
                            Some(reason),
                        );
                    },
                );
                assert_eq!(result, (None, false));
                let row = output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) if text.galley.text().contains(reason) => {
                            Some(text.pos)
                        }
                        _ => None,
                    })
                    .expect("the disabled row must show its reason");
                let pos = row + egui::vec2(5.0, 5.0);
                for pressed in [true, false] {
                    let _ = crate::test_support::run(
                        &ctx,
                        egui::RawInput {
                            events: vec![
                                egui::Event::PointerMoved(pos),
                                egui::Event::PointerButton {
                                    pos,
                                    button: egui::PointerButton::Primary,
                                    pressed,
                                    modifiers: egui::Modifiers::NONE,
                                },
                            ],
                            ..Default::default()
                        },
                        |ui| {
                            result = palette.show(
                                ui.ctx(),
                                &crate::shortcuts::AppKeymap::default().hints(),
                                Some(reason),
                            );
                        },
                    );
                    assert_eq!(result, (None, false));
                }
                assert_eq!(
                    frame(&ctx, &mut palette, &[egui::Key::Enter]).0,
                    Some(Action::Terminal(ScrollToPrompt(direction)))
                );
            }
        }
    }

    #[test]
    fn font_reset_is_a_direct_command_instead_of_opening_the_size_control() {
        let ctx = egui::Context::default();
        let mut palette = CommandPalette {
            query: "font reset".into(),
            selected: 0,
        };
        frame(&ctx, &mut palette, &[]);
        assert_eq!(
            frame(&ctx, &mut palette, &[egui::Key::Enter]).0,
            Some(Action::FontReset)
        );
    }

    #[test]
    fn line_and_half_page_scrolls_are_direct_palette_commands() {
        use felis_client_core::{Action::Scroll, ScrollStep};
        for (query, step) in [
            ("scroll one line up", ScrollStep::LineUp),
            ("scroll one line down", ScrollStep::LineDown),
            ("scroll half page up", ScrollStep::HalfPageUp),
            ("scroll half page down", ScrollStep::HalfPageDown),
        ] {
            let ctx = egui::Context::default();
            let mut palette = CommandPalette {
                query: query.into(),
                selected: 0,
            };
            frame(&ctx, &mut palette, &[]);
            assert_eq!(
                frame(&ctx, &mut palette, &[egui::Key::Enter]).0,
                Some(Action::Terminal(Scroll(step))),
                "{query}"
            );
        }
    }

    #[test]
    fn shortcut_help_is_not_a_palette_command() {
        assert!(matching("keyboard shortcuts").is_empty());
    }

    #[test]
    fn labels_name_a_category_then_an_imperative_verb_phrase() {
        const CATEGORIES: &[&str] = &[
            "Workspace",
            "Tab",
            "Pane",
            "Session",
            "Scrollback",
            "Clipboard",
            "Font",
        ];
        const VERBS: &[&str] = &[
            "Attach", "Change", "Close", "Copy", "Create", "Decrease", "Find", "Focus", "Increase",
            "Jump", "Paste", "Reset", "Scroll", "Show", "Split", "Switch",
        ];
        for command in commands() {
            let label = command.label();
            let (category, phrase) = label.split_once(": ").expect(&label);
            assert!(CATEGORIES.contains(&category), "{label}");
            let verb = phrase.split([' ', '…']).next().unwrap();
            assert!(VERBS.contains(&verb), "{label} does not start with a verb");
        }
        let label = |action| {
            commands()
                .into_iter()
                .find(|command| command.action == action)
                .unwrap()
                .label()
        };
        assert_eq!(label(Action::NewTab), "Tab: Create");
        assert_eq!(label(Action::NextTab), "Tab: Switch to next");
        assert_eq!(label(Action::Split(Axis::Vertical)), "Pane: Split below");
        assert_eq!(label(Action::OpenWorkspaces), "Workspace: Switch…");
    }

    #[test]
    fn commands_once_outside_the_palette_are_listed() {
        let labels: Vec<_> = commands()
            .into_iter()
            .map(|command| (command.action.clone(), command.label()))
            .collect();
        for (action, label) in [
            (Action::NextPane, "Pane: Focus next"),
            (Action::Focus(Direction::Left), "Pane: Focus left"),
            (Action::Focus(Direction::Down), "Pane: Focus down"),
            (Action::Focus(Direction::Up), "Pane: Focus up"),
            (Action::Focus(Direction::Right), "Pane: Focus right"),
            (Action::Copy, "Clipboard: Copy selection"),
            (Action::Paste, "Clipboard: Paste"),
            (Action::AttachSession, "Session: Attach…"),
            (Action::ChangeFontSize, "Font: Change size…"),
        ] {
            assert!(labels.contains(&(action, label.into())), "{label}");
        }
    }

    #[test]
    fn attach_explains_the_local_target_and_new_tab() {
        let command = commands()
            .into_iter()
            .find(|c| c.action == Action::AttachSession)
            .unwrap();
        let text = command.row_text(&egui::Style::default()).text;
        assert!(text.contains("local") && text.contains("new tab"), "{text}");
    }

    #[test]
    fn closing_rows_say_that_the_shell_keeps_running() {
        for action in [Action::ClosePane, Action::CloseTab] {
            let command = commands()
                .into_iter()
                .find(|command| command.action == action)
                .unwrap();
            let text = command.row_text(&egui::Style::default()).text;
            assert!(text.starts_with(&command.label()), "{text}");
            assert!(text.contains("keep"), "{text}");
        }
    }

    #[test]
    fn query_words_match_in_any_order_including_keywords() {
        let actions = |query| {
            matching(query)
                .into_iter()
                .map(|command| command.action)
                .collect::<Vec<_>>()
        };
        assert_eq!(actions("new tab"), [Action::NewTab]);
        assert_eq!(actions("TAB  New"), [Action::NewTab]);
        assert_eq!(actions("search"), [Action::Search]);
        assert_eq!(actions("pane detach"), [Action::ClosePane]);
        assert_eq!(actions("tab next"), [Action::NextTab]);
        assert!(actions("tab zzz").is_empty());
        assert_eq!(matching("  ").len(), commands().len());
    }

    #[test]
    fn arrow_selection_is_activated_by_enter() {
        let ctx = egui::Context::default();
        let mut palette = CommandPalette::default();
        frame(&ctx, &mut palette, &[]);
        frame(&ctx, &mut palette, &[egui::Key::ArrowDown]);
        assert_eq!(
            frame(&ctx, &mut palette, &[egui::Key::Enter]).0,
            commands().into_iter().nth(1).map(|command| command.action)
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
                palette.show(
                    ui.ctx(),
                    &crate::shortcuts::AppKeymap::default().hints(),
                    None,
                );
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
            commands().pop().map(|command| command.action)
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
                        palette.show(
                            ui.ctx(),
                            &crate::shortcuts::AppKeymap::default().hints(),
                            None,
                        );
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
