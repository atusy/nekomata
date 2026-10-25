//! The application-layer document nekomata owns, beside felis's shared one.
use std::path::{Path, PathBuf};

use crate::layout::{Axis, Direction};
use crate::shortcuts::{Action, AppKeymap};
use felis_client_core::Chord;
use felis_client_core::config::{MAX_FONT_SIZE_LOGICAL_PX, MIN_FONT_SIZE_LOGICAL_PX};

/// The terminal's size limits, so one number means the same in both
/// documents.
const FONT_SIZE_RANGE: std::ops::RangeInclusive<f32> =
    MIN_FONT_SIZE_LOGICAL_PX..=MAX_FONT_SIZE_LOGICAL_PX;

#[derive(Debug, Default, PartialEq)]
pub struct AppConfig {
    pub font: UiFont,
    pub keymap: AppKeymap,
}

/// The font of nekomata's own widgets; terminals use felis's `[font]`.
#[derive(Debug, Default, PartialEq)]
pub struct UiFont {
    pub family: Option<String>,
    pub size_px: Option<f32>,
}

/// Recoverable problems; the caller reports them and keeps the rest.
type Warnings = Vec<String>;

/// `$XDG_CONFIG_HOME/nekomata/config.toml` on Linux and
/// `~/Library/Application Support/nekomata/config.toml` on macOS, as
/// felis resolves its own document.
fn path() -> Option<PathBuf> {
    Some(
        directories::ProjectDirs::from("", "", "nekomata")?
            .config_dir()
            .join("config.toml"),
    )
}

/// A missing file means defaults; an unreadable or malformed one fails.
fn read(path: &Path) -> anyhow::Result<(AppConfig, Warnings)> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Default::default());
        }
        Err(error) => anyhow::bail!("cannot read {}: {error}", path.display()),
    };
    parse(&text).map_err(|error| anyhow::anyhow!("config {} is invalid: {error}", path.display()))
}

pub fn load() -> anyhow::Result<AppConfig> {
    let Some(path) = path() else {
        return Ok(Default::default());
    };
    let (config, warnings) = read(&path)?;
    for warning in warnings {
        eprintln!("nekomata: {}: {warning}", path.display());
    }
    Ok(config)
}

/// Unknown keys and unusable values warn; a value of the wrong shape is an
/// error, as in felis's document.
fn parse(text: &str) -> Result<(AppConfig, Warnings), String> {
    let root: toml::Table = text.parse().map_err(|error| format!("{error}"))?;
    let mut config = AppConfig::default();
    let mut warnings = Warnings::new();
    for (key, value) in &root {
        match key.as_str() {
            "appearance" => {
                parse_appearance(table(value, "appearance")?, &mut config, &mut warnings)?
            }
            "keymap" => {
                let entries = parse_keymap(table(value, "keymap")?, &mut warnings);
                config.keymap.apply(entries, &mut warnings);
            }
            _ => warnings.push(format!("unknown key `{key}`")),
        }
    }
    Ok((config, warnings))
}

fn parse_appearance(
    appearance: &toml::Table,
    config: &mut AppConfig,
    warnings: &mut Warnings,
) -> Result<(), String> {
    for (key, value) in appearance {
        match key.as_str() {
            "base" => {
                for (key, value) in table(value, "appearance.base")? {
                    match key.as_str() {
                        "font" => parse_font(
                            table(value, "appearance.base.font")?,
                            &mut config.font,
                            warnings,
                        )?,
                        _ => warnings.push(format!("unknown key `appearance.base.{key}`")),
                    }
                }
            }
            _ => warnings.push(format!("unknown key `appearance.{key}`")),
        }
    }
    Ok(())
}

fn parse_font(font: &toml::Table, out: &mut UiFont, warnings: &mut Warnings) -> Result<(), String> {
    const PATH: &str = "appearance.base.font";
    for (key, value) in font {
        match key.as_str() {
            "family" => {
                let family = value
                    .as_str()
                    .ok_or_else(|| format!("`{PATH}.family` must be a string"))?;
                // No face is named by blanks; searching would only cost a
                // scan of every installed font.
                out.family = if family.trim().is_empty() {
                    warnings.push(format!(
                        "`{PATH}.family` is empty; using the default UI font"
                    ));
                    None
                } else {
                    Some(family.to_owned())
                };
            }
            "size_px" => {
                // Either TOML spelling is accepted: 15 and 15.0 are one size.
                let size = match value {
                    toml::Value::Integer(size) => *size as f32,
                    toml::Value::Float(size) => *size as f32,
                    _ => return Err(format!("`{PATH}.size_px` must be a number")),
                };
                // An overflowing value converts to infinity; like NaN it has
                // no meaningful clamp, so felis also uses the default.
                out.size_px = if !size.is_finite() || size <= 0.0 {
                    warnings.push(format!(
                        "`{PATH}.size_px` must be a positive finite number; using the default"
                    ));
                    None
                } else if !FONT_SIZE_RANGE.contains(&size) {
                    let (min, max) = (*FONT_SIZE_RANGE.start(), *FONT_SIZE_RANGE.end());
                    let clamped = size.clamp(min, max);
                    warnings.push(format!(
                        "`{PATH}.size_px` = {size} is outside [{min}, {max}]; using {clamped}"
                    ));
                    Some(clamped)
                } else {
                    Some(size)
                };
            }
            _ => warnings.push(format!("unknown key `{PATH}.{key}`")),
        }
    }
    Ok(())
}

/// felis's per-entry rule: a malformed entry is dropped with a warning and
/// the other bindings apply. Entries apply in byte order of their keys, so
/// of two spellings of one sequence the later-sorting one wins, as in felis.
fn parse_keymap(
    keymap: &toml::Table,
    warnings: &mut Warnings,
) -> Vec<(String, Vec<Chord>, Option<Action>)> {
    let mut spellings: Vec<_> = keymap.keys().collect();
    spellings.sort();
    spellings
        .into_iter()
        .filter_map(|spelling| {
            let parsed = crate::shortcuts::parse_sequence(spelling)
                .and_then(|keys| Ok((keys, parse_binding(&keymap[spelling])?)));
            match parsed {
                Ok((keys, action)) => Some((spelling.clone(), keys, action)),
                Err(error) => {
                    warnings.push(format!("`keymap.\"{spelling}\"` is dropped: {error}"));
                    None
                }
            }
        })
        .collect()
}

/// felis's binding kinds that act on one terminal or felis session; they
/// belong in felis's `[keymap]`.
const TERMINAL_KINDS: &[&str] = &[
    "send_string",
    "paste",
    "copy",
    "reload",
    "detach",
    "toggle_fullscreen",
    "font_size",
    "scroll",
    "scroll_to_prompt",
    "open_scrollback_search",
    "switch_session",
    "new_session",
    "kill_session",
    "pipe",
    "run",
];

/// `None` is `unbind`.
fn parse_binding(value: &toml::Value) -> Result<Option<Action>, String> {
    let binding = value
        .as_table()
        .ok_or("a binding must be a table with a `kind`")?;
    let kind = binding
        .get("kind")
        .and_then(toml::Value::as_str)
        .ok_or("a binding needs a string `kind`")?;
    let field = |name: &str| -> Result<&str, String> {
        binding
            .get(name)
            .ok_or_else(|| format!("`{kind}` needs `{name}`"))?
            .as_str()
            .ok_or_else(|| format!("`{name}` must be a string"))
    };
    let invalid = |name: &str, value: &str| format!("`{value}` is not a `{name}` for `{kind}`");
    let (action, fields): (_, &[&str]) = match kind {
        "unbind" => (None, &[]),
        "new_tab" => (Some(Action::NewTab), &[]),
        "close_tab" => (Some(Action::CloseTab), &[]),
        "switch_tab" => (
            Some(match field("to")? {
                "next" => Action::NextTab,
                "previous" => Action::PreviousTab,
                other => return Err(invalid("to", other)),
            }),
            &["to"],
        ),
        "split" => (
            Some(match field("direction")? {
                "down" => Action::Split(Axis::Vertical),
                "right" => Action::Split(Axis::Horizontal),
                other => return Err(invalid("direction", other)),
            }),
            &["direction"],
        ),
        "close_pane" => (Some(Action::ClosePane), &[]),
        "focus_pane" => (
            Some(
                match (
                    binding.contains_key("direction"),
                    binding.contains_key("to"),
                ) {
                    (true, true) => {
                        return Err("`focus_pane` takes `direction` or `to`, not both".into());
                    }
                    (false, true) => match field("to")? {
                        "next" => Action::NextPane,
                        other => return Err(invalid("to", other)),
                    },
                    _ => Action::Focus(match field("direction")? {
                        "left" => Direction::Left,
                        "down" => Direction::Down,
                        "up" => Direction::Up,
                        "right" => Direction::Right,
                        other => return Err(invalid("direction", other)),
                    }),
                },
            ),
            &["direction", "to"],
        ),
        "command_palette" => (Some(Action::Palette), &[]),
        "new_workspace" => (Some(Action::NewWorkspace), &[]),
        "open_workspaces" => (Some(Action::OpenWorkspaces), &[]),
        kind if TERMINAL_KINDS.contains(&kind) => {
            return Err(format!(
                "`{kind}` is a terminal binding; set it in felis's config.toml"
            ));
        }
        _ => return Err(format!("unknown kind `{kind}`")),
    };
    if let Some(unknown) = binding
        .keys()
        .find(|name| *name != "kind" && !fields.contains(&name.as_str()))
    {
        return Err(format!("`{kind}` has no field `{unknown}`"));
    }
    Ok(action)
}

fn table<'a>(value: &'a toml::Value, path: &str) -> Result<&'a toml::Table, String> {
    value
        .as_table()
        .ok_or_else(|| format!("`{path}` must be a table"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_file_is_the_default_config() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            read(&dir.path().join("config.toml")).unwrap(),
            Default::default()
        );
    }

    #[test]
    fn the_base_font_takes_a_family_and_either_number_spelling() {
        for size in ["15", "15.0"] {
            let (config, warnings) = parse(&format!(
                "[appearance.base.font]\nfamily = \"UDev Gothic NF\"\nsize_px = {size}"
            ))
            .unwrap();
            assert!(warnings.is_empty(), "{warnings:?}");
            assert_eq!(
                config.font,
                UiFont {
                    family: Some("UDev Gothic NF".into()),
                    size_px: Some(15.0)
                }
            );
        }
    }

    #[test]
    fn unknown_keys_warn_and_the_rest_applies() {
        let (config, warnings) = parse(
            "top = 1\n[appearance]\ntheme = \"x\"\n[appearance.tab_bar]\n[appearance.base]\ncolors = {}\n[appearance.base.font]\nsize_px = 16\nweight = 400",
        )
        .unwrap();
        assert_eq!(config.font.size_px, Some(16.0));
        assert_eq!(
            warnings,
            [
                "unknown key `appearance.base.colors`",
                "unknown key `appearance.base.font.weight`",
                "unknown key `appearance.tab_bar`",
                "unknown key `appearance.theme`",
                "unknown key `top`",
            ]
        );
    }

    #[test]
    fn unusable_sizes_warn_and_fall_back_or_clamp() {
        for (size, expected) in [
            ("0", None),
            ("-3", None),
            ("nan", None),
            ("inf", None),
            ("-inf", None),
            ("1e300", None),
            ("2", Some(4.0)),
            ("100.5", Some(72.0)),
        ] {
            let (config, warnings) =
                parse(&format!("[appearance.base.font]\nsize_px = {size}")).unwrap();
            assert_eq!(config.font.size_px, expected, "{size}");
            assert_eq!(warnings.len(), 1, "{size}");
        }
    }

    #[test]
    fn an_empty_family_is_unset_with_a_warning() {
        for family in ["", "  "] {
            let (config, warnings) =
                parse(&format!("[appearance.base.font]\nfamily = \"{family}\"")).unwrap();
            assert_eq!(config.font.family, None);
            assert_eq!(warnings.len(), 1, "{family:?}: {warnings:?}");
        }
    }

    #[test]
    fn sizes_at_the_range_edges_apply_without_warnings() {
        for size in [MIN_FONT_SIZE_LOGICAL_PX, MAX_FONT_SIZE_LOGICAL_PX] {
            let (config, warnings) =
                parse(&format!("[appearance.base.font]\nsize_px = {size}")).unwrap();
            assert_eq!(config.font.size_px, Some(size));
            assert!(warnings.is_empty(), "{size}: {warnings:?}");
        }
    }

    #[test]
    fn syntax_and_shape_errors_fail_the_document() {
        for text in [
            "[appearance",
            "appearance = 1",
            "[appearance]\nbase = 1",
            "[appearance.base]\nfont = \"x\"",
            "[appearance.base.font]\nfamily = 1",
            "[appearance.base.font]\nsize_px = \"15\"",
            "keymap = 1",
        ] {
            assert!(parse(text).is_err(), "{text}");
        }
    }

    #[test]
    fn an_invalid_file_names_its_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[appearance").unwrap();
        let error = read(&path).unwrap_err().to_string();
        assert!(error.contains(&path.display().to_string()), "{error}");
    }

    /// Feeds key presses through a keymap configured by `text`.
    fn press(text: &str, keys: &[(eframe::egui::Key, eframe::egui::Modifiers)]) -> Vec<Action> {
        let (config, warnings) = parse(text).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        let mut shortcuts = crate::shortcuts::Shortcuts::default();
        shortcuts.set_keymap(config.keymap);
        let events = keys
            .iter()
            .map(|&(key, modifiers)| eframe::egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            })
            .collect();
        let ctx = eframe::egui::Context::default();
        let mut actions = vec![];
        let _ = crate::test_support::run(
            &ctx,
            eframe::egui::RawInput {
                events,
                ..Default::default()
            },
            |ui| actions = ui.ctx().input_mut(|input| shortcuts.consume(input)),
        );
        actions
    }

    #[test]
    fn an_empty_keymap_keeps_the_defaults() {
        assert_eq!(parse("[keymap]").unwrap().0.keymap, AppKeymap::default());
    }

    #[test]
    fn every_kind_binds_its_action() {
        use eframe::egui::{Key, Modifiers};
        let text = r#"[keymap]
            "f1" = { kind = "new_tab" }
            "f2" = { kind = "close_tab" }
            "f3" = { kind = "switch_tab", to = "next" }
            "f4" = { kind = "switch_tab", to = "previous" }
            "f5" = { kind = "split", direction = "down" }
            "f6" = { kind = "split", direction = "right" }
            "f7" = { kind = "close_pane" }
            "f8" = { kind = "focus_pane", direction = "left" }
            "f9" = { kind = "focus_pane", to = "next" }
            "f10" = { kind = "command_palette" }
            "f11" = { kind = "new_workspace" }
            "f12" = { kind = "open_workspaces" }
        "#;
        let keys = [
            Key::F1,
            Key::F2,
            Key::F3,
            Key::F4,
            Key::F5,
            Key::F6,
            Key::F7,
            Key::F8,
            Key::F9,
            Key::F10,
            Key::F11,
            Key::F12,
        ]
        .map(|key| (key, Modifiers::NONE));
        assert_eq!(
            press(text, &keys),
            [
                Action::NewTab,
                Action::CloseTab,
                Action::NextTab,
                Action::PreviousTab,
                Action::Split(Axis::Vertical),
                Action::Split(Axis::Horizontal),
                Action::ClosePane,
                Action::Focus(Direction::Left),
                Action::NextPane,
                Action::Palette,
                Action::NewWorkspace,
                Action::OpenWorkspaces,
            ]
        );
    }

    #[test]
    fn configured_sequences_replace_and_unbind_defaults() {
        use eframe::egui::{Key, Modifiers};
        let text = r#"[keymap]
            "alt+c" = { kind = "unbind" }
            "alt+w" = { kind = "unbind" }
            "ctrl+a s" = { kind = "split", direction = "down" }
            "ctrl+a ctrl+a" = { kind = "close_pane" }
            "alt+t" = { kind = "new_tab" }
        "#;
        assert_eq!(
            press(
                text,
                &[
                    (Key::C, Modifiers::ALT),
                    (Key::W, Modifiers::ALT),
                    (Key::S, Modifiers::ALT),
                    (Key::A, Modifiers::CTRL),
                    (Key::S, Modifiers::NONE),
                    (Key::A, Modifiers::CTRL),
                    (Key::A, Modifiers::CTRL),
                    (Key::T, Modifiers::ALT),
                    (Key::T, Modifiers::ALT | Modifiers::SHIFT),
                ]
            ),
            [
                Action::Split(Axis::Vertical),
                Action::ClosePane,
                Action::NewTab,
                Action::PreviousTab,
            ]
        );
    }

    #[test]
    fn malformed_bindings_are_dropped_and_the_rest_apply() {
        let (config, warnings) = parse(
            r#"[keymap]
            "alt+x" = { kind = "nope" }
            "alt+y" = { kind = "split", direction = "vertical" }
            "alt+z" = { kind = "new_tab", extra = 1 }
            "ctrl+ctrl+a" = { kind = "new_tab" }
            "alt+q" = "new_tab"
            "alt+f" = { kind = "copy", what = "system" }
            "alt+g" = { kind = "focus_pane", direction = "left", to = "next" }
            "alt+h" = { kind = "switch_tab" }
            "   " = { kind = "new_tab" }
        "#,
        )
        .unwrap();
        assert_eq!(config.keymap, AppKeymap::default());
        assert_eq!(warnings.len(), 9, "{warnings:#?}");
        assert!(
            warnings
                .iter()
                .any(|warning| warning.contains("felis's config.toml")),
            "{warnings:#?}"
        );
    }

    #[test]
    fn a_sequence_behind_a_bound_chord_is_dropped_with_a_warning() {
        let (config, warnings) = parse(
            r#"[keymap]
            "alt+c x" = { kind = "close_tab" }
        "#,
        )
        .unwrap();
        assert_eq!(config.keymap, AppKeymap::default());
        assert_eq!(warnings.len(), 1, "{warnings:#?}");
    }

    #[test]
    fn binding_a_default_prefix_on_its_own_replaces_its_sequences_silently() {
        use eframe::egui::{Key, Modifiers};
        let text = "[keymap]\n\"alt+w\" = { kind = \"close_tab\" }";
        assert_eq!(
            press(text, &[(Key::W, Modifiers::ALT), (Key::C, Modifiers::NONE)]),
            [Action::CloseTab]
        );
    }

    #[test]
    fn later_sorting_spellings_of_one_chord_win() {
        use eframe::egui::{Key, Modifiers};
        let text = r#"[keymap]
            "shift+alt+n" = { kind = "close_tab" }
            "alt+shift+n" = { kind = "new_tab" }
        "#;
        assert_eq!(
            press(text, &[(Key::N, Modifiers::ALT | Modifiers::SHIFT)]),
            [Action::CloseTab]
        );
    }
}
