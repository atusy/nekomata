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
    let has_text = events
        .iter()
        .any(|event| matches!(event, egui::Event::Text(_)));
    let mut inputs = Vec::new();
    for event in events {
        let event_mods = match event {
            egui::Event::Key { modifiers, .. } => *modifiers,
            _ => frame_mods,
        };
        let kind = match event {
            egui::Event::Key { repeat: true, .. } => KeyEventKind::Repeat,
            _ => KeyEventKind::Press,
        };
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
            inputs.push(TerminalInput::Key(KeyEvent {
                key,
                text,
                mods: key_mods(event_mods),
                kind,
                location: KeyLocation::Standard,
            }));
        }
    }
    inputs
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
        Some(Key::Character(key_character(key)))
    } else {
        None
    }
}

fn key_character(key: egui::Key) -> String {
    key.symbol_or_name().to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_control_c_interrupts_instead_of_copying() {
        assert_eq!(
            clipboard_control(&egui::Event::Copy, egui::Modifiers::CTRL, false),
            Some(vec![3])
        );
    }

    #[test]
    fn control_bracket_keeps_the_character_needed_for_escape() {
        assert_eq!(key_character(egui::Key::OpenBracket), "[");
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
