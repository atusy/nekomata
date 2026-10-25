use eframe::egui::{Event, InputState, Key};
use felis_client_core::{
    Action as TerminalAction, Chord, ClipboardScope, IpcAction, KeyCode, Keymap,
    Modifiers as TerminalModifiers, NamedKey,
};

use crate::layout::{Axis, Direction};

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Terminal(TerminalAction),
    NewWorkspace,
    OpenWorkspaces,
    NewTab,
    Split(Axis),
    ClosePane,
    CloseTab,
    NextPane,
    NextTab,
    PreviousTab,
    Focus(Direction),
    FontScale(f32),
    FontReset,
    Palette,
    Search,
    Paste,
    Copy,
}

pub struct Shortcuts {
    prefix: bool,
    terminal: Keymap,
}

impl Default for Shortcuts {
    fn default() -> Self {
        Self {
            prefix: false,
            // Preserve bare End for terminal editors unless explicitly bound.
            terminal: Keymap::default_for_platform().with_overrides([(
                Chord::bare(KeyCode::Named(NamedKey::End)),
                felis_client_core::BindingValue::Unbind,
            )]),
        }
    }
}

// Keep nekomata's workspace shortcuts separate from reusable terminal actions.
// The default map also contains actions for features not exposed here (reload,
// PRIMARY clipboard); those keys must continue to reach the program.
fn supported_terminal_action(action: &TerminalAction) -> bool {
    matches!(
        action,
        TerminalAction::Copy {
            what: ClipboardScope::System
        } | TerminalAction::Paste {
            from: ClipboardScope::System
        } | TerminalAction::FontSize(_)
            | TerminalAction::Scroll(_)
            | TerminalAction::ScrollToPrompt(_)
            | TerminalAction::Ipc(IpcAction::OpenScrollbackSearch)
    )
}

fn terminal_chord(key: Key, modifiers: eframe::egui::Modifiers) -> Chord {
    let mut mods = TerminalModifiers::empty();
    mods.set(TerminalModifiers::CONTROL, modifiers.ctrl);
    mods.set(TerminalModifiers::SHIFT, modifiers.shift);
    mods.set(TerminalModifiers::ALT, modifiers.alt);
    // egui's `command` is also true for Ctrl on Linux; it is not SUPER.
    mods.set(TerminalModifiers::SUPER, modifiers.mac_cmd);
    let key = match key {
        Key::ArrowUp => KeyCode::Named(NamedKey::ArrowUp),
        Key::ArrowDown => KeyCode::Named(NamedKey::ArrowDown),
        Key::ArrowLeft => KeyCode::Named(NamedKey::ArrowLeft),
        Key::ArrowRight => KeyCode::Named(NamedKey::ArrowRight),
        Key::Home => KeyCode::Named(NamedKey::Home),
        Key::End => KeyCode::Named(NamedKey::End),
        Key::PageUp => KeyCode::Named(NamedKey::PageUp),
        Key::PageDown => KeyCode::Named(NamedKey::PageDown),
        Key::Insert => KeyCode::Named(NamedKey::Insert),
        // egui displays this key as U+2212; bindings name the ASCII key.
        Key::Minus => KeyCode::Character("-".into()),
        _ => {
            let name = key.symbol_or_name().to_ascii_lowercase();
            name.parse::<Chord>()
                .map(|chord| chord.key)
                .unwrap_or(KeyCode::Character(name))
        }
    };
    Chord::with_mods(mods, key)
}

impl Shortcuts {
    pub fn from_config(config: &felis_client_core::config::EffectiveConfig) -> Self {
        let mut shortcuts = Self::default();
        shortcuts
            .terminal
            .apply_overrides(config.keymap.compile(None));
        shortcuts
    }

    pub fn consume(&mut self, input: &mut InputState) -> Vec<Action> {
        let mut actions = Vec::new();
        let mut suppress_text = false;
        if !input.focused {
            self.prefix = false;
            return actions;
        }
        input.events.retain(|event| {
            if matches!(event, Event::Text(_)) {
                return !suppress_text;
            }
            let Event::Key {
                key,
                pressed: true,
                modifiers: mods,
                ..
            } = event
            else {
                return true;
            };
            suppress_text = false;
            let alt = mods.alt && !mods.ctrl && !mods.command && !mods.mac_cmd;
            let plain = !mods.alt && !mods.ctrl && !mods.command && !mods.mac_cmd && !mods.shift;
            let prefix = std::mem::take(&mut self.prefix);
            let action = if prefix {
                match (*key, alt && !mods.shift, plain) {
                    (Key::S, true, _) => Some(Action::Split(Axis::Vertical)),
                    (Key::V, true, _) => Some(Action::Split(Axis::Horizontal)),
                    (Key::C, _, true) => Some(Action::ClosePane),
                    (Key::H, _, true) => Some(Action::Focus(Direction::Left)),
                    (Key::J, _, true) => Some(Action::Focus(Direction::Down)),
                    (Key::K, _, true) => Some(Action::Focus(Direction::Up)),
                    (Key::L, _, true) => Some(Action::Focus(Direction::Right)),
                    (Key::Escape, _, _) => {
                        suppress_text = true;
                        return false;
                    }
                    _ => None,
                }
            } else if alt {
                match (*key, mods.shift) {
                    (Key::W, false) => {
                        self.prefix = true;
                        suppress_text = true;
                        return false;
                    }
                    (Key::C, false) => Some(Action::NewTab),
                    (Key::T, false) => Some(Action::NextTab),
                    (Key::T, true) => Some(Action::PreviousTab),
                    (Key::P, true) => Some(Action::Palette),
                    _ => None,
                }
            } else {
                None
            };
            let action = action.or_else(|| {
                if prefix {
                    return None;
                }
                self.terminal
                    .resolve(&terminal_chord(*key, *mods))
                    .filter(|action| supported_terminal_action(action))
                    .cloned()
                    .map(Action::Terminal)
            });
            if let Some(action) = action {
                actions.push(action);
                suppress_text = true;
                false
            } else {
                true
            }
        });
        actions
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::Modifiers;

    #[test]
    fn configured_binding_can_replace_a_shared_action() {
        let document = felis_client_core::config::ConfigDocument::parse(
            "[client.nekomata.keymap]\n\"ctrl+shift+f\" = { kind = \"copy\", what = \"system\" }",
            None,
        )
        .unwrap();
        let (config, diagnostics) = document.resolve("nekomata");
        assert!(!diagnostics.has_errors());
        let mut shortcuts = Shortcuts::from_config(&config);
        let ctx = eframe::egui::Context::default();
        let _ = crate::test_support::run(
            &ctx,
            eframe::egui::RawInput {
                events: vec![press(Key::F, Modifiers::CTRL | Modifiers::SHIFT)],
                ..Default::default()
            },
            |ui| {
                assert_eq!(
                    ui.ctx().input_mut(|input| shortcuts.consume(input)),
                    vec![Action::Terminal(TerminalAction::Copy {
                        what: ClipboardScope::System
                    })]
                );
            },
        );
    }

    #[test]
    fn configured_plain_alt_and_function_keys_reach_terminal_bindings() {
        for (chord, key, modifiers) in [
            ("end", Key::End, Modifiers::NONE),
            ("alt+s", Key::S, Modifiers::ALT),
            ("f9", Key::F9, Modifiers::NONE),
            ("ctrl+up", Key::ArrowUp, Modifiers::CTRL),
        ] {
            let document = felis_client_core::config::ConfigDocument::parse(
                &format!("[keymap]\n\"{chord}\" = {{ kind = \"copy\", what = \"system\" }}"),
                None,
            )
            .unwrap();
            let (config, diagnostics) = document.resolve("nekomata");
            assert!(!diagnostics.has_errors());
            assert_eq!(config.keymap.compile(None).len(), 1);
            let mut shortcuts = Shortcuts::from_config(&config);
            let ctx = eframe::egui::Context::default();
            let _ = crate::test_support::run(
                &ctx,
                eframe::egui::RawInput {
                    events: vec![press(key, modifiers)],
                    ..Default::default()
                },
                |ui| {
                    assert_eq!(
                        ui.ctx().input_mut(|input| shortcuts.consume(input)),
                        vec![Action::Terminal(TerminalAction::Copy {
                            what: ClipboardScope::System
                        })],
                        "{chord}"
                    )
                },
            );
        }
    }

    fn press(key: Key, modifiers: Modifiers) -> Event {
        Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    fn consume_events(events: Vec<Event>) -> (Vec<Action>, Vec<Event>) {
        let ctx = eframe::egui::Context::default();
        let mut shortcuts = Shortcuts::default();
        let mut result = (vec![], vec![]);
        let _ = crate::test_support::run(
            &ctx,
            eframe::egui::RawInput {
                events,
                ..Default::default()
            },
            |ui| {
                result = ui
                    .ctx()
                    .input_mut(|input| (shortcuts.consume(input), input.events.clone()));
            },
        );
        result
    }

    #[test]
    fn shared_navigation_bindings_use_terminal_actions_without_leaking_text() {
        use felis_client_core::{Action as TerminalAction, ScrollStep};
        use felis_protocol::messages::PromptJump;
        let ctrl_shift = Modifiers {
            ctrl: true,
            shift: true,
            command: true,
            ..Default::default()
        };
        let (actions, remaining) = consume_events(vec![
            press(Key::PageUp, Modifiers::SHIFT),
            press(Key::PageDown, Modifiers::SHIFT),
            press(Key::Home, ctrl_shift),
            press(Key::End, ctrl_shift),
            press(Key::Z, ctrl_shift),
            Event::Text("Z".into()),
            press(Key::X, ctrl_shift),
            Event::Text("X".into()),
        ]);
        assert_eq!(
            actions,
            vec![
                Action::Terminal(TerminalAction::Scroll(ScrollStep::HalfPageUp)),
                Action::Terminal(TerminalAction::Scroll(ScrollStep::HalfPageDown)),
                Action::Terminal(TerminalAction::Scroll(ScrollStep::Home)),
                Action::Terminal(TerminalAction::Scroll(ScrollStep::End)),
                Action::Terminal(TerminalAction::ScrollToPrompt(PromptJump::Previous)),
                Action::Terminal(TerminalAction::ScrollToPrompt(PromptJump::Next)),
            ]
        );
        assert!(remaining.is_empty());
    }

    #[test]
    fn unsupported_shared_defaults_and_option_composition_reach_the_terminal() {
        let ctrl_shift = Modifiers {
            ctrl: true,
            shift: true,
            command: true,
            ..Default::default()
        };
        let events = vec![
            press(Key::R, ctrl_shift),
            press(Key::Insert, Modifiers::SHIFT),
            press(Key::End, Modifiers::NONE),
            press(Key::E, Modifiers::ALT),
            Event::Text("é".into()),
            Event::Ime(eframe::egui::ImeEvent::Commit("猫".into())),
        ];
        let (actions, remaining) = consume_events(events.clone());
        assert!(actions.is_empty());
        assert_eq!(remaining, events);
    }

    #[test]
    fn search_shortcuts_follow_the_shared_keymap_and_capture_only_its_chords() {
        for (modifiers, opens_search) in [
            (Modifiers::CTRL | Modifiers::SHIFT, true),
            (Modifiers::MAC_CMD, true),
            (Modifiers::CTRL, false),
            (Modifiers::MAC_CMD | Modifiers::SHIFT, true), // felis ignores Shift on Cmd chords
            (Modifiers::CTRL | Modifiers::SHIFT | Modifiers::ALT, false),
            (Modifiers::NONE, false),
        ] {
            let ctx = eframe::egui::Context::default();
            let mut shortcuts = Shortcuts::default();
            let _ = crate::test_support::run(
                &ctx,
                eframe::egui::RawInput {
                    events: vec![press(Key::F, modifiers), Event::Text("f".into())],
                    ..Default::default()
                },
                |ui| {
                    ui.ctx().input_mut(|input| {
                        let actions = shortcuts.consume(input);
                        assert_eq!(
                            actions,
                            if opens_search {
                                vec![Action::Terminal(TerminalAction::Ipc(
                                    felis_client_core::IpcAction::OpenScrollbackSearch,
                                ))]
                            } else {
                                vec![]
                            }
                        );
                        assert_eq!(input.events.is_empty(), opens_search);
                    });
                },
            );
        }
    }

    #[test]
    fn prefix_survives_frames_and_consumes_option_text() {
        let ctx = eframe::egui::Context::default();
        let mut shortcuts = Shortcuts::default();
        for (events, expected) in [
            (
                vec![press(Key::W, Modifiers::ALT), Event::Text("∑".into())],
                vec![],
            ),
            (
                vec![press(Key::V, Modifiers::ALT), Event::Text("√".into())],
                vec![Action::Split(Axis::Horizontal)],
            ),
            (vec![press(Key::C, Modifiers::ALT)], vec![Action::NewTab]),
            (
                vec![press(Key::T, Modifiers::ALT | Modifiers::SHIFT)],
                vec![Action::PreviousTab],
            ),
        ] {
            let _ = crate::test_support::run(
                &ctx,
                eframe::egui::RawInput {
                    events,
                    ..Default::default()
                },
                |root_ui| {
                    let context = root_ui.ctx().clone();
                    let ctx = &context;
                    ctx.input_mut(|input| {
                        assert_eq!(shortcuts.consume(input), expected);
                        assert!(input.events.is_empty());
                    });
                },
            );
        }
    }

    #[test]
    fn shortcuts_preserve_unrelated_text_and_match_font_and_paste_keys() {
        let ctx = eframe::egui::Context::default();
        let mut shortcuts = Shortcuts::default();
        for (key, step) in [
            ("0", felis_client_core::FontSizeStep::Reset),
            ("=", felis_client_core::FontSizeStep::Increase),
            ("-", felis_client_core::FontSizeStep::Decrease),
        ] {
            shortcuts.terminal.insert(
                Chord::with_mods(TerminalModifiers::SUPER, KeyCode::Character(key.into())),
                TerminalAction::FontSize(step),
            );
        }
        let events = vec![
            press(Key::C, Modifiers::ALT),
            Event::Text("ç".into()),
            press(Key::X, Modifiers::NONE),
            Event::Text("x".into()),
            press(Key::Num0, Modifiers::MAC_CMD),
            press(Key::Equals, Modifiers::MAC_CMD),
            press(Key::Minus, Modifiers::MAC_CMD),
            press(
                Key::C,
                Modifiers {
                    ctrl: true,
                    shift: true,
                    command: true,
                    ..Default::default()
                },
            ),
            press(
                Key::V,
                Modifiers {
                    ctrl: true,
                    shift: true,
                    command: true,
                    ..Default::default()
                },
            ),
        ];
        let _ = crate::test_support::run(
            &ctx,
            eframe::egui::RawInput {
                events,
                ..Default::default()
            },
            |root_ui| {
                let context = root_ui.ctx().clone();
                let ctx = &context;
                ctx.input_mut(|input| {
                    assert_eq!(
                        shortcuts.consume(input),
                        vec![
                            Action::NewTab,
                            Action::Terminal(TerminalAction::FontSize(
                                felis_client_core::FontSizeStep::Reset
                            )),
                            Action::Terminal(TerminalAction::FontSize(
                                felis_client_core::FontSizeStep::Increase
                            )),
                            Action::Terminal(TerminalAction::FontSize(
                                felis_client_core::FontSizeStep::Decrease
                            )),
                            Action::Terminal(TerminalAction::Copy {
                                what: ClipboardScope::System
                            }),
                            Action::Terminal(TerminalAction::Paste {
                                from: ClipboardScope::System
                            })
                        ]
                    );
                    assert_eq!(
                        input.events,
                        vec![press(Key::X, Modifiers::NONE), Event::Text("x".into())]
                    );
                });
            },
        );
    }

    #[test]
    fn unmatched_keys_and_old_shortcuts_reach_the_shell() {
        let ctx = eframe::egui::Context::default();
        let mut shortcuts = Shortcuts::default();
        let events = vec![
            press(Key::T, Modifiers::COMMAND),
            press(Key::C, Modifiers::CTRL),
        ];
        let _ = crate::test_support::run(
            &ctx,
            eframe::egui::RawInput {
                events: events.clone(),
                ..Default::default()
            },
            |root_ui| {
                let context = root_ui.ctx().clone();
                let ctx = &context;
                ctx.input_mut(|input| {
                    assert!(shortcuts.consume(input).is_empty());
                    assert_eq!(input.events, events);
                });
            },
        );
    }
}
