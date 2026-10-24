use eframe::egui::{Event, InputState, Key};

use crate::layout::Axis;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    CopyCommandOutput,
    EditCommandOutput,
    EditScrollback,
    NewSession,
    Sessions,
    NewTab,
    Split(Axis),
    ClosePane,
    CloseTab,
    NextPane,
    NextTab,
    PreviousTab,
    Focus(Key),
    FontScale(f32),
    FontReset,
    Palette,
    Search,
    Paste,
    Copy,
}

#[derive(Default)]
pub struct Shortcuts {
    prefix: bool,
}

impl Shortcuts {
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
            let alt = mods.alt && !mods.ctrl && !mods.command;
            let plain = !mods.alt && !mods.ctrl && !mods.command && !mods.shift;
            let action = if std::mem::take(&mut self.prefix) {
                match (*key, alt && !mods.shift, plain) {
                    (Key::S, true, _) => Some(Action::Split(Axis::Vertical)),
                    (Key::V, true, _) => Some(Action::Split(Axis::Horizontal)),
                    (Key::C, _, true) => Some(Action::ClosePane),
                    (Key::H | Key::J | Key::K | Key::L, _, true) => Some(Action::Focus(*key)),
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
            } else if mods.mac_cmd && !mods.ctrl && !mods.alt {
                match key {
                    Key::F if !mods.shift => Some(Action::Search),
                    Key::Num0 => Some(Action::FontReset),
                    Key::Equals | Key::Plus => Some(Action::FontScale(1.1)),
                    Key::Minus => Some(Action::FontScale(1.0 / 1.1)),
                    _ => None,
                }
            } else if mods.ctrl && mods.shift && !mods.alt && !mods.mac_cmd {
                match key {
                    Key::F => Some(Action::Search),
                    Key::V => Some(Action::Paste),
                    Key::C => Some(Action::Copy),
                    _ => None,
                }
            } else {
                None
            };
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

    fn press(key: Key, modifiers: Modifiers) -> Event {
        Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    #[test]
    fn search_shortcuts_capture_only_the_documented_chords() {
        for (modifiers, opens_search) in [
            (Modifiers::CTRL | Modifiers::SHIFT, true),
            (Modifiers::MAC_CMD, true),
            (Modifiers::CTRL, false),
            (Modifiers::MAC_CMD | Modifiers::SHIFT, false),
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
                                vec![Action::Search]
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
                            Action::FontReset,
                            Action::FontScale(1.1),
                            Action::FontScale(1.0 / 1.1),
                            Action::Copy,
                            Action::Paste
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
