use std::collections::BTreeMap;

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
    AttachSession,
    ChangeFontSize,
    Search,
    Paste,
    Copy,
}

/// nekomata's own bindings, keyed by chord sequence: felis binds single
/// chords only, and splits need a prefix key.
#[derive(Clone, Debug, PartialEq)]
pub struct AppKeymap {
    bindings: BTreeMap<Vec<Chord>, Action>,
}

enum Lookup {
    Action(Action),
    Prefix,
    Miss,
}

impl Default for AppKeymap {
    fn default() -> Self {
        let bindings = [
            ("alt+c", Action::NewTab),
            ("alt+t", Action::NextTab),
            ("alt+shift+t", Action::PreviousTab),
            ("alt+shift+p", Action::Palette),
            ("alt+w alt+s", Action::Split(Axis::Vertical)),
            ("alt+w alt+v", Action::Split(Axis::Horizontal)),
            ("alt+w c", Action::ClosePane),
            ("alt+w h", Action::Focus(Direction::Left)),
            ("alt+w j", Action::Focus(Direction::Down)),
            ("alt+w k", Action::Focus(Direction::Up)),
            ("alt+w l", Action::Focus(Direction::Right)),
        ];
        Self {
            bindings: bindings
                .into_iter()
                .map(|(keys, action)| (parse_sequence(keys).unwrap(), action))
                .collect(),
        }
    }
}

/// Whitespace separates the chords of a sequence; each chord is spelled
/// as in felis's `[keymap]`.
pub fn parse_sequence(spelling: &str) -> Result<Vec<Chord>, String> {
    let keys = spelling
        .split_whitespace()
        .map(|chord| chord.parse::<Chord>().map_err(|error| error.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    if keys.is_empty() {
        return Err("empty key sequence".into());
    }
    // `Shortcuts::consume` cancels a pending sequence on any Escape, so a
    // later Escape chord could never be reached.
    if keys[1..]
        .iter()
        .any(|chord| chord.key == KeyCode::Named(NamedKey::Escape))
    {
        return Err("Escape cancels a pending sequence, so it can only start one".into());
    }
    if !keys.iter().all(is_deliverable) {
        return Err("the window cannot report this chord, so it never arrives as a key".into());
    }
    if keys.iter().any(is_clipboard_chord) {
        return Err(
            "the window turns this chord into a clipboard event, so it never arrives as a key"
                .into(),
        );
    }
    Ok(keys)
}

/// egui reports Super only as macOS's Cmd, and only keys it has a
/// [`Key`] for; any other chord parses but can never be pressed.
fn is_deliverable(chord: &Chord) -> bool {
    if !cfg!(target_os = "macos") && chord.mods.contains(TerminalModifiers::SUPER) {
        return false;
    }
    Key::ALL.iter().any(|&key| terminal_key(key) == chord.key)
}

/// Mirrors egui-winit's clipboard shortcuts (`is_cut_command` and siblings):
/// with Alt and Shift up, Command (Cmd on macOS, Ctrl elsewhere) plus X, C
/// or V becomes `Event::Cut`/`Copy`/`Paste` instead of `Event::Key`.
fn is_clipboard_chord(chord: &Chord) -> bool {
    let command = if cfg!(target_os = "macos") {
        TerminalModifiers::SUPER
    } else {
        TerminalModifiers::CONTROL
    };
    chord.mods.contains(command)
        && !chord
            .mods
            .intersects(TerminalModifiers::ALT | TerminalModifiers::SHIFT)
        && matches!(&chord.key, KeyCode::Character(c) if matches!(c.as_str(), "x" | "c" | "v"))
}

impl AppKeymap {
    /// Applies user entries over the defaults. `None` unbinds a sequence and
    /// every longer one it begins. A sequence whose leading chords are bound
    /// on their own could never complete, so it is dropped.
    pub fn apply(
        &mut self,
        entries: Vec<(String, Vec<Chord>, Option<Action>)>,
        warnings: &mut Vec<String>,
    ) {
        let mut configured = BTreeMap::new();
        for (spelling, keys, action) in entries {
            match action {
                None => {
                    self.bindings.retain(|bound, _| !bound.starts_with(&keys));
                    // Entries apply in spelling order, so an unbind can undo
                    // a binding the user wrote under another spelling.
                    configured.retain(|bound: &Vec<Chord>, removed: &mut String| {
                        let keep = !bound.starts_with(&keys);
                        if !keep {
                            warnings.push(format!(
                                "`keymap.\"{removed}\"` is removed by the unbind `keymap.\"{spelling}\"`"
                            ));
                        }
                        keep
                    });
                }
                Some(action) => {
                    self.bindings.insert(keys.clone(), action);
                    configured.insert(keys, spelling);
                }
            }
        }
        let unreachable: Vec<_> = self
            .bindings
            .keys()
            .filter(|keys| (1..keys.len()).any(|len| self.bindings.contains_key(&keys[..len])))
            .cloned()
            .collect();
        for keys in unreachable {
            self.bindings.remove(&keys);
            // Defaults yield silently to a chord the user bound on its own.
            if let Some(spelling) = configured.get(&keys) {
                warnings.push(format!(
                    "`keymap.\"{spelling}\"` is dropped because its leading keys are bound on their own"
                ));
            }
        }
    }

    /// The key text the palette shows for each bound action: its shortest sequence,
    /// the first in key order among equals.
    pub fn hints(&self) -> Hints {
        let mut hints: Vec<(Action, &[Chord])> = vec![];
        for (keys, action) in &self.bindings {
            match hints.iter_mut().find(|(bound, _)| bound == action) {
                Some((_, shortest)) if keys.len() < shortest.len() => *shortest = keys,
                Some(_) => {}
                None => hints.push((action.clone(), keys)),
            }
        }
        Hints(
            hints
                .into_iter()
                .map(|(action, keys)| {
                    let text = keys.iter().map(chord_label).collect::<Vec<_>>();
                    // Two spaces read as "then" in the palette.
                    (action, text.join("  "))
                })
                .collect(),
        )
    }

    fn lookup(&self, keys: &[Chord]) -> Lookup {
        use std::ops::Bound;
        // Sequences sort directly after their prefixes, so the first entry
        // from `keys` on is either `keys` itself or the nearest longer one.
        match self
            .bindings
            .range::<[Chord], _>((Bound::Included(keys), Bound::Unbounded))
            .next()
        {
            Some((bound, action)) if bound[..] == *keys => Lookup::Action(action.clone()),
            Some((bound, _)) if bound.starts_with(keys) => Lookup::Prefix,
            _ => Lookup::Miss,
        }
    }
}

/// Labels for the actions nekomata's keymap binds, built once per keymap.
#[derive(Debug)]
pub struct Hints(Vec<(Action, String)>);

impl Hints {
    /// Empty for an unbound action.
    pub fn get(&self, action: &Action) -> &str {
        self.0
            .iter()
            .find(|(bound, _)| bound == action)
            .map_or("", |(_, text)| text)
    }
}

/// `alt+shift+t` as the palette shows it: `Alt+Shift+T`.
fn chord_label(chord: &Chord) -> String {
    let mut label = String::new();
    for (modifier, name) in [
        (TerminalModifiers::CONTROL, "Ctrl"),
        (TerminalModifiers::ALT, "Alt"),
        (TerminalModifiers::SHIFT, "Shift"),
        (
            TerminalModifiers::SUPER,
            if cfg!(target_os = "macos") {
                "Cmd"
            } else {
                "Super"
            },
        ),
    ] {
        if chord.mods.contains(modifier) {
            label.push_str(name);
            label.push('+');
        }
    }
    // felis spells named keys in snake case (`page_up`).
    let key = Chord::bare(chord.key.clone()).to_string();
    for word in key.split('_') {
        let mut chars = word.chars();
        label.extend(chars.next().map(|first| first.to_ascii_uppercase()));
        label.push_str(chars.as_str());
    }
    label
}

pub struct Shortcuts {
    pending: Vec<Chord>,
    /// The chord a sequence last consumed, whose key may still be held.
    held: Option<Chord>,
    keymap: AppKeymap,
    hints: Hints,
    terminal: Keymap,
}

impl Default for Shortcuts {
    fn default() -> Self {
        let keymap = AppKeymap::default();
        Self {
            pending: Vec::new(),
            held: None,
            hints: keymap.hints(),
            keymap,
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
    Chord::with_mods(mods, terminal_key(key))
}

fn terminal_key(key: Key) -> KeyCode {
    match key {
        Key::ArrowUp => KeyCode::Named(NamedKey::ArrowUp),
        Key::ArrowDown => KeyCode::Named(NamedKey::ArrowDown),
        Key::ArrowLeft => KeyCode::Named(NamedKey::ArrowLeft),
        Key::ArrowRight => KeyCode::Named(NamedKey::ArrowRight),
        Key::Home => KeyCode::Named(NamedKey::Home),
        Key::End => KeyCode::Named(NamedKey::End),
        Key::PageUp => KeyCode::Named(NamedKey::PageUp),
        Key::PageDown => KeyCode::Named(NamedKey::PageDown),
        Key::Insert => KeyCode::Named(NamedKey::Insert),
        _ => match crate::input::key_char(key) {
            Some(character) => KeyCode::Character(character.into()),
            None => {
                let name = key.symbol_or_name().to_ascii_lowercase();
                name.parse::<Chord>()
                    .map(|chord| chord.key)
                    .unwrap_or(KeyCode::Character(name))
            }
        },
    }
}

impl Shortcuts {
    /// nekomata's own `keymap` resolves first; felis's shared `config`
    /// supplies the terminal bindings behind it.
    pub fn from_config(
        config: &felis_client_core::config::EffectiveConfig,
        keymap: AppKeymap,
    ) -> Self {
        let mut shortcuts = Self {
            hints: keymap.hints(),
            keymap,
            ..Self::default()
        };
        shortcuts
            .terminal
            .apply_overrides(config.keymap.compile(None));
        shortcuts
    }

    pub fn hints(&self) -> &Hints {
        &self.hints
    }

    pub fn consume(&mut self, input: &mut InputState) -> Vec<Action> {
        let mut actions = Vec::new();
        let mut suppress_text = false;
        if !input.focused {
            self.pending.clear();
            self.held = None;
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
                repeat,
                ..
            } = event
            else {
                return true;
            };
            suppress_text = false;
            let chord = terminal_chord(*key, *mods);
            // A held key that a sequence consumed, whether it started,
            // continued, completed or cancelled it, repeats into nothing.
            if *repeat && self.held.as_ref() == Some(&chord) {
                suppress_text = true;
                return false;
            }
            self.held = None;
            let pending = !self.pending.is_empty();
            if pending && *key == Key::Escape {
                self.pending.clear();
                self.held = Some(chord);
                suppress_text = true;
                return false;
            }
            // `pending` is cleared rather than replaced, so its allocation
            // is reused across key presses.
            self.pending.push(chord);
            let action = match self.keymap.lookup(&self.pending) {
                Lookup::Prefix => {
                    self.held = self.pending.last().cloned();
                    suppress_text = true;
                    return false;
                }
                Lookup::Action(action) => {
                    if pending {
                        self.held = self.pending.last().cloned();
                    }
                    Some(action)
                }
                // An unfinished sequence gives the key to the program, not
                // to a terminal binding it happens to match.
                Lookup::Miss if pending => None,
                Lookup::Miss => self
                    .terminal
                    .resolve(&self.pending[0])
                    .filter(|action| supported_terminal_action(action))
                    .cloned()
                    .map(Action::Terminal),
            };
            self.pending.clear();
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
        let mut shortcuts = Shortcuts::from_config(&config, AppKeymap::default());
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
            let mut shortcuts = Shortcuts::from_config(&config, AppKeymap::default());
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
            // felis adds the Cmd chords to its default keymap on macOS only.
            (Modifiers::MAC_CMD, cfg!(target_os = "macos")),
            (Modifiers::CTRL, false),
            // felis ignores Shift on Cmd chords.
            (
                Modifiers::MAC_CMD | Modifiers::SHIFT,
                cfg!(target_os = "macos"),
            ),
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
    fn configured_spellings_match_the_chords_egui_keys_produce() {
        for (spelling, key, modifiers) in [
            ("alt+shift+t", Key::T, Modifiers::ALT | Modifiers::SHIFT),
            ("alt+T", Key::T, Modifiers::ALT | Modifiers::SHIFT),
            ("shift+alt+t", Key::T, Modifiers::ALT | Modifiers::SHIFT),
            ("alt+w", Key::W, Modifiers::ALT),
            ("c", Key::C, Modifiers::NONE),
            ("ctrl+a", Key::A, Modifiers::CTRL),
            ("alt+left", Key::ArrowLeft, Modifiers::ALT),
            ("f5", Key::F5, Modifiers::NONE),
            ("'", Key::Quote, Modifiers::NONE),
            ("ctrl+'", Key::Quote, Modifiers::CTRL),
            (";", Key::Semicolon, Modifiers::NONE),
            ("`", Key::Backtick, Modifiers::NONE),
            ("[", Key::OpenBracket, Modifiers::NONE),
            ("\\", Key::Backslash, Modifiers::NONE),
            ("ctrl++", Key::Plus, Modifiers::CTRL),
            ("-", Key::Minus, Modifiers::NONE),
            ("space", Key::Space, Modifiers::NONE),
        ] {
            assert_eq!(
                spelling.parse::<Chord>().unwrap(),
                terminal_chord(key, modifiers),
                "{spelling}"
            );
        }
    }

    #[test]
    fn a_pending_sequence_swallows_escape_and_gives_misses_to_the_program() {
        let ctrl_shift = Modifiers {
            ctrl: true,
            shift: true,
            command: true,
            ..Default::default()
        };
        let (actions, remaining) = consume_events(vec![
            press(Key::W, Modifiers::ALT),
            press(Key::Escape, Modifiers::NONE),
            Event::Text("x".into()),
            press(Key::C, Modifiers::NONE),
            press(Key::W, Modifiers::ALT),
            press(Key::PageUp, Modifiers::SHIFT),
            press(Key::PageUp, Modifiers::SHIFT),
            press(Key::W, Modifiers::ALT),
            press(Key::F, ctrl_shift),
        ]);
        assert_eq!(
            actions,
            vec![Action::Terminal(TerminalAction::Scroll(
                felis_client_core::ScrollStep::HalfPageUp
            ))]
        );
        assert_eq!(
            remaining,
            vec![
                press(Key::C, Modifiers::NONE),
                press(Key::PageUp, Modifiers::SHIFT),
                press(Key::F, ctrl_shift),
            ]
        );
    }

    #[test]
    fn holding_a_prefix_keeps_its_sequence_pending() {
        let repeat = |key, modifiers| Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: true,
            modifiers,
        };
        let (actions, remaining) = consume_events(vec![
            press(Key::W, Modifiers::ALT),
            repeat(Key::W, Modifiers::ALT),
            Event::Text("∑".into()),
            repeat(Key::W, Modifiers::ALT),
            press(Key::S, Modifiers::ALT),
        ]);
        assert_eq!(actions, vec![Action::Split(Axis::Vertical)]);
        assert!(remaining.is_empty(), "{remaining:?}");
    }

    #[test]
    fn holding_the_key_that_ends_a_sequence_sends_nothing_more() {
        let repeat = |key, modifiers| Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: true,
            modifiers,
        };
        let (actions, remaining) = consume_events(vec![
            press(Key::W, Modifiers::ALT),
            press(Key::S, Modifiers::ALT),
            repeat(Key::S, Modifiers::ALT),
            Event::Text("ß".into()),
            press(Key::W, Modifiers::ALT),
            press(Key::Escape, Modifiers::NONE),
            repeat(Key::Escape, Modifiers::NONE),
        ]);
        assert_eq!(actions, vec![Action::Split(Axis::Vertical)]);
        assert!(remaining.is_empty(), "{remaining:?}");
    }

    #[test]
    fn holding_a_bound_key_repeats_its_action() {
        let (actions, remaining) = consume_events(vec![
            press(Key::PageUp, Modifiers::SHIFT),
            Event::Key {
                key: Key::PageUp,
                physical_key: None,
                pressed: true,
                repeat: true,
                modifiers: Modifiers::SHIFT,
            },
        ]);
        let scroll = Action::Terminal(TerminalAction::Scroll(
            felis_client_core::ScrollStep::HalfPageUp,
        ));
        assert_eq!(actions, vec![scroll.clone(), scroll]);
        assert!(remaining.is_empty());
    }

    #[test]
    fn default_alt_w_sequences_split_close_and_focus() {
        let mut events = vec![];
        for (key, modifiers) in [
            (Key::S, Modifiers::ALT),
            (Key::V, Modifiers::ALT),
            (Key::C, Modifiers::NONE),
            (Key::H, Modifiers::NONE),
            (Key::J, Modifiers::NONE),
            (Key::K, Modifiers::NONE),
            (Key::L, Modifiers::NONE),
        ] {
            events.extend([press(Key::W, Modifiers::ALT), press(key, modifiers)]);
        }
        let (actions, remaining) = consume_events(events);
        assert_eq!(
            actions,
            vec![
                Action::Split(Axis::Vertical),
                Action::Split(Axis::Horizontal),
                Action::ClosePane,
                Action::Focus(Direction::Left),
                Action::Focus(Direction::Down),
                Action::Focus(Direction::Up),
                Action::Focus(Direction::Right),
            ]
        );
        assert!(remaining.is_empty());
    }

    #[test]
    fn hints_name_the_active_bindings() {
        let hints = AppKeymap::default().hints();
        assert_eq!(hints.get(&Action::NewTab), "Alt+C");
        assert_eq!(hints.get(&Action::PreviousTab), "Alt+Shift+T");
        assert_eq!(hints.get(&Action::Split(Axis::Horizontal)), "Alt+W  Alt+V");
        assert_eq!(hints.get(&Action::ClosePane), "Alt+W  C");
        assert_eq!(hints.get(&Action::CloseTab), "");

        let mut keymap = AppKeymap::default();
        keymap.apply(
            vec![
                ("alt+c".into(), parse_sequence("alt+c").unwrap(), None),
                (
                    "ctrl+shift+page_up".into(),
                    parse_sequence("ctrl+shift+page_up").unwrap(),
                    Some(Action::NewTab),
                ),
                (
                    "ctrl+a '".into(),
                    parse_sequence("ctrl+a '").unwrap(),
                    Some(Action::CloseTab),
                ),
            ],
            &mut vec![],
        );
        let hints = keymap.hints();
        assert_eq!(hints.get(&Action::NewTab), "Ctrl+Shift+PageUp");
        assert_eq!(hints.get(&Action::CloseTab), "Ctrl+A  '");
    }

    #[test]
    fn a_hint_prefers_the_shortest_binding() {
        let mut keymap = AppKeymap::default();
        keymap.apply(
            vec![(
                "alt+w n".into(),
                parse_sequence("alt+w n").unwrap(),
                Some(Action::NewTab),
            )],
            &mut vec![],
        );
        assert_eq!(keymap.hints().get(&Action::NewTab), "Alt+C");
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
