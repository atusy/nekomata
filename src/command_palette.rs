use crate::{layout::Axis, shortcuts::Action, theme};
use eframe::egui;

#[derive(Default)]
pub struct CommandPalette {
    query: String,
    selected: usize,
}

/// The closed set of label prefixes; see the naming ADR
/// (docs/ADR/20261026-230753-name-palette-commands-by-category-and-verb-phrase.md).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Category {
    Workspace,
    Tab,
    Pane,
    Scrollback,
    Font,
}

impl Category {
    fn name(self) -> &'static str {
        match self {
            Self::Workspace => "Workspace",
            Self::Tab => "Tab",
            Self::Pane => "Pane",
            Self::Scrollback => "Scrollback",
            Self::Font => "Font",
        }
    }
}

struct Command {
    category: Category,
    phrase: &'static str,
    action: Action,
    detail: Option<&'static str>,
    /// Words that find the command without appearing in it.
    keywords: &'static [&'static str],
}

impl Command {
    fn new(category: Category, phrase: &'static str, action: Action) -> Self {
        Self {
            category,
            phrase,
            action,
            detail: None,
            keywords: &[],
        }
    }

    fn keywords(self, keywords: &'static [&'static str]) -> Self {
        Self { keywords, ..self }
    }

    fn matches(&self, words: &[String]) -> bool {
        let text = format!("{} {}", self.label(), self.keywords.join(" ")).to_lowercase();
        words.iter().all(|word| text.contains(word.as_str()))
    }

    fn detail(self, detail: &'static str) -> Self {
        Self {
            detail: Some(detail),
            ..self
        }
    }

    fn label(&self) -> String {
        format!("{}: {}", self.category.name(), self.phrase)
    }

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

fn commands() -> Vec<Command> {
    use Category::*;
    use felis_client_core::{Action::Scroll, Action::ScrollToPrompt, ScrollStep};
    use felis_protocol::messages::PromptJump;
    let command = Command::new;
    let terminal = Action::Terminal;
    vec![
        command(Workspace, "Create", Action::NewWorkspace).keywords(&["new"]),
        command(Workspace, "Switch…", Action::OpenWorkspaces).keywords(&["open", "list"]),
        command(Tab, "Create", Action::NewTab).keywords(&["new"]),
        command(Tab, "Switch to next", Action::NextTab),
        command(Tab, "Switch to previous", Action::PreviousTab),
        command(Tab, "Close", Action::CloseTab)
            .detail("Detach its sessions; the shells keep running")
            .keywords(&["detach"]),
        command(Pane, "Split right", Action::Split(Axis::Horizontal))
            .keywords(&["vertical", "side by side"]),
        command(Pane, "Split below", Action::Split(Axis::Vertical))
            .keywords(&["horizontal", "down"]),
        command(Pane, "Close", Action::ClosePane)
            .detail("Detach its session; the shell keeps running")
            .keywords(&["detach"]),
        command(Scrollback, "Find…", Action::Search).keywords(&["search", "grep"]),
        command(
            Scrollback,
            "Jump to previous prompt",
            terminal(ScrollToPrompt(PromptJump::Previous)),
        )
        .detail("Requires shell integration"),
        command(
            Scrollback,
            "Jump to next prompt",
            terminal(ScrollToPrompt(PromptJump::Next)),
        )
        .detail("Requires shell integration"),
        command(
            Scrollback,
            "Scroll to top",
            terminal(Scroll(ScrollStep::Home)),
        )
        .keywords(&["oldest", "home"]),
        command(
            Scrollback,
            "Scroll to bottom",
            terminal(Scroll(ScrollStep::End)),
        )
        .keywords(&["live", "end"]),
        command(Font, "Reset size", Action::FontReset).keywords(&["zoom", "text"]),
    ]
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
    /// Terminal bindings belong to felis's document, so `hints` covers only
    /// the shortcuts nekomata's own keymap binds.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        hints: &crate::shortcuts::Hints,
    ) -> (Option<Action>, bool) {
        let (up, down, enter) = ctx.input_mut(|input| {
            (
                input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                input.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
            )
        });
        let viewport = ctx.content_rect();
        let top = (viewport.height() * 0.12).min(64.0);
        let style = ctx.global_style();
        let width =
            (viewport.width() - 48.0).clamp(100.0, theme::ui_extent(&style, theme::PALETTE_WIDTH));
        let row_height = theme::ui_extent(&style, theme::PALETTE_ROW_HEIGHT);
        let list_height = (viewport.height() - top - theme::ui_extent(&style, 88.0)).clamp(
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
                    [width, row_height],
                    egui::TextEdit::singleline(&mut self.query)
                        .hint_text("Search commands…")
                        .font(egui::FontId::proportional(theme::ui_size(ui.style(), 14.0)))
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
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if matches.is_empty() {
                            ui.add_space(16.0);
                            ui.label(
                                egui::RichText::new("No matching commands")
                                    .color(theme::TAB_INACTIVE_FG),
                            );
                        }
                        for (index, command) in matches.iter().enumerate() {
                            let selected = index == self.selected;
                            let height = match command.detail {
                                Some(_) => row_height * 1.5,
                                None => row_height,
                            };
                            let row = ui.add_sized(
                                [ui.available_width(), height],
                                egui::Button::new(command.row_text(ui.style()))
                                    .shortcut_text(hints.get(&command.action))
                                    .selected(selected),
                            );
                            if selected && (up || down || search.changed()) {
                                row.scroll_to_me(Some(egui::Align::Center));
                            }
                            if row.clicked() || (enter && selected) {
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
            result = palette.show(ui.ctx(), &crate::shortcuts::AppKeymap::default().hints())
        });
        result
    }
    #[test]
    fn labels_name_a_category_then_an_imperative_verb_phrase() {
        const VERBS: &[&str] = &[
            "Attach", "Change", "Close", "Copy", "Create", "Find", "Focus", "Jump", "Paste",
            "Reset", "Scroll", "Show", "Split", "Switch",
        ];
        for command in commands() {
            let label = command.label();
            let (category, phrase) = label.split_once(": ").expect(&label);
            assert_eq!(category, command.category.name(), "{label}");
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
        let first = |query| {
            matching(query)
                .first()
                .map(|command| command.action.clone())
        };
        assert_eq!(first("new tab"), Some(Action::NewTab));
        assert_eq!(first("TAB  New"), Some(Action::NewTab));
        assert_eq!(first("search"), Some(Action::Search));
        assert_eq!(first("pane detach"), Some(Action::ClosePane));
        assert_eq!(first("tab next"), Some(Action::NextTab));
        assert!(matching("tab zzz").is_empty());
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
            Some(Action::OpenWorkspaces)
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
                palette.show(ui.ctx(), &crate::shortcuts::AppKeymap::default().hints());
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
                        palette.show(ui.ctx(), &crate::shortcuts::AppKeymap::default().hints());
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
