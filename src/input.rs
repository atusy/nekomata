//! What the focused terminal program receives for a frame's egui events.
//!
//! Pure translation: the caller owns the session, the selection and the
//! clipboard, so every encoding rule here is testable without a window.

use eframe::egui;
use felis_protocol::messages::{FKey, Key, KeyEvent, KeyEventKind, KeyLocation, KeyMods, NamedKey};

#[derive(Clone, Debug, PartialEq)]
pub enum TerminalInput {
    /// A key press for felis to encode for the program.
    Key(KeyEvent),
    /// A control byte standing in for a clipboard shortcut.
    Control(Vec<u8>),
    /// Text an input method committed.
    Commit(String),
    /// Clipboard text, subject to the paste rules.
    Paste(String),
    /// Copy the selection to the clipboard.
    Copy,
}

/// `explicit_paste` marks the frame's first paste as one nekomata requested
/// itself, so it is never mistaken for Ctrl+V.
pub fn translate(
    events: &[egui::Event],
    frame_mods: egui::Modifiers,
    mut explicit_paste: bool,
    macos: bool,
) -> Vec<TerminalInput> {
    let mut inputs = Vec::new();
    for (index, event) in events.iter().enumerate() {
        // egui-winit queues a keystroke's text right after its key event;
        // the frame's modifiers may already belong to a later keystroke.
        // A key egui cannot name queues its text alone, so text only
        // belongs to a preceding key that could have typed it.
        let typed_by = match event {
            egui::Event::Text(_) => index.checked_sub(1).and_then(|i| text_key(&events[i])),
            egui::Event::Key {
                modifiers, repeat, ..
            } => Some((*modifiers, *repeat)),
            _ => None,
        };
        let (event_mods, repeat) = typed_by.unwrap_or((frame_mods, false));
        let kind = if repeat {
            KeyEventKind::Repeat
        } else {
            KeyEventKind::Press
        };
        let has_text = text_key(event).is_some()
            && matches!(events.get(index + 1), Some(egui::Event::Text(_)));
        let requested_paste =
            matches!(event, egui::Event::Paste(_)) && std::mem::take(&mut explicit_paste);
        if !requested_paste && let Some(control) = clipboard_control(event, frame_mods, macos) {
            inputs.push(TerminalInput::Control(control));
            continue;
        }
        let key = match event {
            egui::Event::Paste(text) => {
                inputs.push(TerminalInput::Paste(text.clone()));
                continue;
            }
            egui::Event::Copy => {
                inputs.push(TerminalInput::Copy);
                continue;
            }
            egui::Event::Ime(egui::ImeEvent::Commit(text)) => {
                inputs.push(TerminalInput::Commit(text.clone()));
                continue;
            }
            egui::Event::Text(text) => Some((Key::Character(text.clone()), Some(text.clone()))),
            egui::Event::Key {
                key, pressed: true, ..
            } => physical_key(*key, event_mods, has_text).map(|key| (key, None)),
            _ => None,
        };
        if let Some((key, text)) = key {
            let mut mods = key_mods(event_mods);
            if macos
                && text
                    .as_deref()
                    .is_some_and(|text| option_composed(text, event_mods))
            {
                // felis encodes Alt with an ASCII character as ESC + char,
                // which would turn `@` typed with Option into Meta-@.
                mods.remove(KeyMods::ALT);
            }
            inputs.push(TerminalInput::Key(KeyEvent {
                key,
                text,
                mods,
                kind,
                location: KeyLocation::Standard,
            }));
        }
    }
    inputs
}

/// The modifiers and repeat state of a key press that may have typed text:
/// egui-winit queues no text for named keys or while Ctrl or Cmd is held.
fn text_key(event: &egui::Event) -> Option<(egui::Modifiers, bool)> {
    match event {
        egui::Event::Key {
            key,
            pressed: true,
            modifiers,
            repeat,
            ..
        } if !(modifiers.ctrl || modifiers.command || modifiers.mac_cmd)
            && (*key == egui::Key::Space || key_char(*key).is_some()) =>
        {
            Some((*modifiers, *repeat))
        }
        _ => None,
    }
}

/// macOS reports Option as Alt even when it only selected the character
/// typed. A plain space is what Option+Space types when it composes
/// nothing, so it stays Alt+Space.
fn option_composed(text: &str, mods: egui::Modifiers) -> bool {
    mods.alt && !mods.ctrl && !mods.mac_cmd && text != " "
}

fn key_mods(mods: egui::Modifiers) -> KeyMods {
    let mut key_mods = KeyMods::empty();
    key_mods.set(KeyMods::CTRL, mods.ctrl);
    key_mods.set(KeyMods::SHIFT, mods.shift);
    key_mods.set(KeyMods::ALT, mods.alt);
    key_mods.set(KeyMods::SUPER, mods.mac_cmd);
    key_mods
}

fn clipboard_control(event: &egui::Event, mods: egui::Modifiers, macos: bool) -> Option<Vec<u8>> {
    if macos || !mods.ctrl || mods.shift || mods.alt {
        return None;
    }
    let byte = match event {
        egui::Event::Copy => 3,
        egui::Event::Cut => 24,
        egui::Event::Paste(_) => 22,
        _ => return None,
    };
    Some(vec![byte])
}

fn physical_key(key: egui::Key, mods: egui::Modifiers, has_text: bool) -> Option<Key> {
    let named = match key {
        egui::Key::Space if mods.ctrl || (mods.alt && !has_text) => Some(NamedKey::Space),
        egui::Key::Enter => Some(NamedKey::Enter),
        egui::Key::Tab => Some(NamedKey::Tab),
        egui::Key::Escape => Some(NamedKey::Escape),
        egui::Key::Backspace => Some(NamedKey::Backspace),
        egui::Key::Insert => Some(NamedKey::Insert),
        egui::Key::Delete => Some(NamedKey::Delete),
        egui::Key::Home => Some(NamedKey::Home),
        egui::Key::End => Some(NamedKey::End),
        egui::Key::PageUp => Some(NamedKey::PageUp),
        egui::Key::PageDown => Some(NamedKey::PageDown),
        egui::Key::ArrowUp => Some(NamedKey::ArrowUp),
        egui::Key::ArrowDown => Some(NamedKey::ArrowDown),
        egui::Key::ArrowLeft => Some(NamedKey::ArrowLeft),
        egui::Key::ArrowRight => Some(NamedKey::ArrowRight),
        _ => key
            .name()
            .strip_prefix('F')
            .and_then(|n| n.parse().ok())
            .and_then(FKey::new)
            .map(NamedKey::F),
    };
    if let Some(named) = named {
        Some(Key::Named(named))
    } else if mods.ctrl || (mods.alt && !has_text) {
        key_char(key).map(|character| Key::Character(character.into()))
    } else {
        None
    }
}

/// The ASCII character a character key types without modifiers, as keymaps
/// spell it and felis encodes it; `None` for keys that name no character.
pub fn key_char(key: egui::Key) -> Option<char> {
    match key {
        // egui displays this key as U+2212.
        egui::Key::Minus => Some('-'),
        // egui has no symbol for this key and names it "Quote".
        egui::Key::Quote => Some('\''),
        _ => {
            let mut chars = key.symbol_or_name().chars();
            let character = chars.next().filter(|c| c.is_ascii_graphic())?;
            chars
                .next()
                .is_none()
                .then(|| character.to_ascii_lowercase())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    fn text(text: &str) -> egui::Event {
        egui::Event::Text(text.into())
    }

    fn sent(key: Key, text: Option<&str>, mods: KeyMods, kind: KeyEventKind) -> TerminalInput {
        TerminalInput::Key(KeyEvent {
            key,
            text: text.map(str::to_owned),
            mods,
            kind,
            location: KeyLocation::Standard,
        })
    }

    fn character(c: &str) -> Key {
        Key::Character(c.into())
    }

    #[test]
    fn a_key_without_text_is_encoded_even_when_another_key_in_the_frame_typed_text() {
        let alt = egui::Modifiers::ALT;
        let got = translate(
            &[
                press(egui::Key::E, alt),
                press(egui::Key::B, alt),
                text("∫"),
            ],
            alt,
            false,
            false,
        );
        assert_eq!(
            got,
            [
                sent(character("e"), None, KeyMods::ALT, KeyEventKind::Press),
                sent(character("∫"), Some("∫"), KeyMods::ALT, KeyEventKind::Press),
            ]
        );
    }

    #[test]
    fn text_carries_the_modifiers_and_repeat_of_the_key_that_typed_it() {
        let got = translate(
            &[
                egui::Event::Key {
                    key: egui::Key::B,
                    physical_key: None,
                    pressed: true,
                    repeat: true,
                    modifiers: egui::Modifiers::ALT,
                },
                text("b"),
                press(egui::Key::C, egui::Modifiers::NONE),
                text("c"),
            ],
            egui::Modifiers::NONE,
            false,
            false,
        );
        assert_eq!(
            got,
            [
                sent(
                    character("b"),
                    Some("b"),
                    KeyMods::ALT,
                    KeyEventKind::Repeat
                ),
                sent(
                    character("c"),
                    Some("c"),
                    KeyMods::empty(),
                    KeyEventKind::Press
                ),
            ]
        );
    }

    #[test]
    fn macos_option_composed_text_is_sent_without_alt() {
        for (key, composed, mods, expected) in [
            // German Option+L and JIS Option+¥ compose ASCII.
            (egui::Key::L, "@", egui::Modifiers::ALT, KeyMods::empty()),
            (
                egui::Key::Backslash,
                "\\",
                egui::Modifiers::ALT,
                KeyMods::empty(),
            ),
            (
                egui::Key::L,
                "Ò",
                egui::Modifiers::ALT | egui::Modifiers::SHIFT,
                KeyMods::SHIFT,
            ),
        ] {
            let got = translate(&[press(key, mods), text(composed)], mods, false, true);
            assert_eq!(
                got,
                [sent(
                    character(composed),
                    Some(composed),
                    expected,
                    KeyEventKind::Press
                )],
                "{composed}"
            );
        }
    }

    #[test]
    fn option_text_keeps_alt_where_option_composes_nothing() {
        let alt = egui::Modifiers::ALT;
        // Option+Space types a plain space; elsewhere Alt is meta.
        for (key, typed, macos) in [(egui::Key::Space, " ", true), (egui::Key::B, "b", false)] {
            let got = translate(&[press(key, alt), text(typed)], alt, false, macos);
            assert_eq!(
                got,
                [sent(
                    character(typed),
                    Some(typed),
                    KeyMods::ALT,
                    KeyEventKind::Press
                )],
                "{typed:?}"
            );
        }
    }

    #[test]
    fn text_from_an_unnamed_key_is_not_paired_with_a_modifier_or_named_key() {
        // egui reports modifier keys as keys but has no key for `<`, so the
        // ISO key that types it yields text alone.
        for before in [egui::Key::ShiftLeft, egui::Key::ArrowLeft] {
            let got = translate(
                &[
                    egui::Event::Key {
                        key: before,
                        physical_key: None,
                        pressed: true,
                        repeat: true,
                        modifiers: egui::Modifiers::ALT,
                    },
                    text("<"),
                ],
                egui::Modifiers::SHIFT,
                false,
                false,
            );
            assert_eq!(
                got.last(),
                Some(&sent(
                    character("<"),
                    Some("<"),
                    KeyMods::SHIFT,
                    KeyEventKind::Press
                )),
                "{before:?}"
            );
        }
        let ctrl = egui::Modifiers::CTRL;
        assert_eq!(
            translate(&[press(egui::Key::A, ctrl), text("<")], ctrl, false, false)[0],
            sent(character("a"), None, KeyMods::CTRL, KeyEventKind::Press),
            "a key that cannot have typed the text still reaches the program"
        );
    }

    #[test]
    fn text_without_a_key_uses_the_frame_modifiers() {
        let got = translate(&[text("ろ")], egui::Modifiers::SHIFT, false, false);
        assert_eq!(
            got,
            [sent(
                character("ろ"),
                Some("ろ"),
                KeyMods::SHIFT,
                KeyEventKind::Press
            )]
        );
    }

    #[test]
    fn linux_control_c_interrupts_instead_of_copying() {
        assert_eq!(
            clipboard_control(&egui::Event::Copy, egui::Modifiers::CTRL, false),
            Some(vec![3])
        );
    }

    #[test]
    fn control_bracket_keeps_the_character_needed_for_escape() {
        assert_eq!(
            physical_key(egui::Key::OpenBracket, egui::Modifiers::CTRL, false),
            Some(Key::Character("[".into()))
        );
    }

    #[test]
    fn control_punctuation_names_the_ascii_key() {
        for (key, character) in [(egui::Key::Minus, "-"), (egui::Key::Quote, "'")] {
            assert_eq!(
                physical_key(key, egui::Modifiers::CTRL, false),
                Some(Key::Character(character.into())),
                "{key:?}"
            );
        }
    }

    #[test]
    fn modified_keys_send_only_single_ascii_characters() {
        for key in egui::Key::ALL {
            if let Some(Key::Character(character)) =
                physical_key(*key, egui::Modifiers::CTRL, false)
            {
                assert!(
                    character.len() == 1 && character.is_ascii(),
                    "{key:?} sends {character:?}"
                );
            }
        }
    }

    #[test]
    fn composed_alt_text_does_not_emit_a_second_physical_character() {
        assert_eq!(physical_key(egui::Key::E, egui::Modifiers::ALT, true), None);
    }

    #[test]
    fn function_keys_reach_terminal_programs_without_text_events() {
        assert_eq!(
            physical_key(egui::Key::F1, egui::Modifiers::NONE, false),
            Some(Key::Named(NamedKey::F(FKey::lit(1))))
        );
    }
}
