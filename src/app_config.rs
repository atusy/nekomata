//! The application-layer document nekomata owns, beside felis's shared one.
use std::path::{Path, PathBuf};

use crate::config_report::printable;
use crate::layout::{Axis, Direction};
use crate::shortcuts::{Action, AppKeymap};
use felis_client_core::Chord;
use felis_client_core::config::{MAX_FONT_SIZE_LOGICAL_PX, MIN_FONT_SIZE_LOGICAL_PX};

/// The terminal's size limits, so one number means the same in both
/// documents.
const FONT_SIZE_RANGE: std::ops::RangeInclusive<f32> =
    MIN_FONT_SIZE_LOGICAL_PX..=MAX_FONT_SIZE_LOGICAL_PX;

#[derive(Debug, Default, PartialEq)]
struct AppConfig {
    font: UiFont,
    keymap: AppKeymap,
}

/// The font of nekomata's own widgets; terminals use felis's `[font]`.
#[derive(Debug, Default, PartialEq)]
struct UiFont {
    family: Option<String>,
    size_px: Option<f32>,
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
/// `warnings` keeps what was found before a failure, too.
fn read(path: &Path, warnings: &mut Warnings) -> anyhow::Result<AppConfig> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Default::default());
        }
        Err(error) => anyhow::bail!("cannot read {}: {error}", path.display()),
    };
    parse(&text, warnings).map_err(|error| {
        anyhow::anyhow!(
            "config {} is invalid: {}",
            path.display(),
            printable(&error)
        )
    })
}

/// The document resolved against the platform: what the GUI applies.
pub struct Settings {
    pub ui_size: f32,
    pub ui_face: Option<crate::ui_font::Face>,
    pub keymap: AppKeymap,
}

pub fn load() -> anyhow::Result<Settings> {
    let path = path();
    let mut warnings = Warnings::new();
    let config = match &path {
        Some(path) => read(path, &mut warnings),
        None => Ok(AppConfig::default()),
    };
    let settings = config.map(|config| {
        let (ui_size, ui_face) = resolve_font(
            &config.font,
            crate::ui_font::default_size_px,
            crate::ui_font::find,
            &mut warnings,
        );
        Settings {
            ui_size,
            ui_face,
            keymap: config.keymap,
        }
    });
    // Report warnings even when the document fails, so one launch shows
    // every problem found before the error.
    let source = path.as_deref().unwrap_or(Path::new("config.toml"));
    for warning in warnings {
        eprintln!("nekomata: {}: {}", source.display(), printable(&warning));
    }
    settings
}

/// The UI body size and the configured face, with `default_size` and
/// `find` standing in for the platform.
fn resolve_font(
    font: &UiFont,
    default_size: impl FnOnce() -> f32,
    find: impl FnOnce(&str) -> Result<crate::ui_font::Face, crate::ui_font::Unavailable>,
    warnings: &mut Warnings,
) -> (f32, Option<crate::ui_font::Face>) {
    let size = font.size_px.unwrap_or_else(default_size);
    let face = font
        .family
        .as_deref()
        .and_then(|family| match find(family) {
            Ok(face) => Some(face),
            Err(reason) => {
                warnings.push(format!(
                    "`appearance.base.font.family` = {family:?} {reason}; using the default UI font"
                ));
                None
            }
        });
    (size, face)
}

/// Unknown keys and unusable values warn; a value of the wrong shape is an
/// error, as in felis's document. `[keymap]` entries are the exception:
/// they follow felis's per-entry rule and a malformed one is dropped.
fn parse(text: &str, warnings: &mut Warnings) -> Result<AppConfig, String> {
    let root: toml::Table = text.parse().map_err(|error| format!("{error}"))?;
    let mut config = AppConfig::default();
    for (key, value) in &root {
        match key.as_str() {
            "appearance" => parse_appearance(table(value, "appearance")?, &mut config, warnings)?,
            "keymap" => {
                let entries = parse_keymap(table(value, "keymap")?, warnings);
                config.keymap.apply(entries, warnings);
            }
            _ => warnings.push(format!("unknown key `{key}`")),
        }
    }
    Ok(config)
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

/// `None` is `unbind`.
fn parse_binding(value: &toml::Value) -> Result<Option<Action>, String> {
    let binding = value
        .as_table()
        .ok_or("a binding must be a table with a `kind`")?;
    let kind = binding
        .get("kind")
        .and_then(toml::Value::as_str)
        .ok_or("a binding needs a string `kind`")?;
    if let Some(command) = crate::commands::commands()
        .iter()
        .find(|command| command.kind == kind)
    {
        if let Some(unknown) = binding.keys().find(|name| *name != "kind") {
            return Err(format!("`{kind}` has no field `{unknown}`"));
        }
        return Ok(Some(command.action.clone()));
    }
    let field = |name: &str| -> Result<&str, String> {
        binding
            .get(name)
            .ok_or_else(|| format!("`{kind}` needs `{name}`"))?
            .as_str()
            .ok_or_else(|| format!("`{name}` must be a string"))
    };
    let invalid = |name: &str, value: &str, accepted: &[&str]| {
        let quoted: Vec<_> = accepted.iter().map(|value| format!("`{value}`")).collect();
        let (last, rest) = quoted.split_last().expect("every field accepts a value");
        let choices = match rest {
            [] => last.clone(),
            rest => format!("{} or {last}", rest.join(", ")),
        };
        format!("`{value}` is not a `{name}` for `{kind}`; use {choices}")
    };
    let (action, fields): (_, &[&str]) = match kind {
        "unbind" => (None, &[]),
        "new_tab" => (Some(Action::NewTab), &[]),
        "close_tab" => (Some(Action::CloseTab), &[]),
        "switch_tab" => (
            Some(match field("to")? {
                "next" => Action::NextTab,
                "previous" => Action::PreviousTab,
                other => return Err(invalid("to", other, &["next", "previous"])),
            }),
            &["to"],
        ),
        "split" => (
            Some(match field("direction")? {
                "below" => Action::Split(Axis::Vertical),
                "right" => Action::Split(Axis::Horizontal),
                other => return Err(invalid("direction", other, &["below", "right"])),
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
                        other => return Err(invalid("to", other, &["next"])),
                    },
                    _ => Action::Focus(match field("direction")? {
                        "left" => Direction::Left,
                        "down" => Direction::Down,
                        "up" => Direction::Up,
                        "right" => Direction::Right,
                        other => {
                            return Err(invalid(
                                "direction",
                                other,
                                &["left", "down", "up", "right"],
                            ));
                        }
                    }),
                },
            ),
            &["direction", "to"],
        ),
        "command_palette" => (Some(Action::Palette), &[]),
        "new_workspace" => (Some(Action::NewWorkspace), &[]),
        "open_workspaces" => (Some(Action::OpenWorkspaces), &[]),
        _ => {
            return Err(format!(
                "unknown kind `{kind}`; terminal bindings may belong in felis's config.toml"
            ));
        }
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

    /// The document with the warnings it produced.
    fn parse(text: &str) -> Result<(AppConfig, Warnings), String> {
        let mut warnings = Warnings::new();
        super::parse(text, &mut warnings).map(|config| (config, warnings))
    }

    #[test]
    fn a_missing_file_is_the_default_config() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            read(&dir.path().join("config.toml"), &mut Warnings::new()).unwrap(),
            Default::default()
        );
    }

    #[test]
    fn an_unset_size_uses_the_platform_default_and_no_family_skips_the_search() {
        let mut warnings = Warnings::new();
        let (size, face) = resolve_font(
            &UiFont::default(),
            || 13.0,
            |_| panic!("no family to search for"),
            &mut warnings,
        );
        assert_eq!((size, face.is_none()), (13.0, true));
        assert!(warnings.is_empty());
    }

    #[test]
    fn a_configured_size_wins_and_a_found_family_supplies_the_face() {
        let font = UiFont {
            family: Some("UDev Gothic NF".into()),
            size_px: Some(17.0),
        };
        let mut warnings = Warnings::new();
        let (size, face) = resolve_font(
            &font,
            || 13.0,
            |family| {
                assert_eq!(family, "UDev Gothic NF");
                Ok(crate::ui_font::Face {
                    bytes: vec![],
                    index: 3,
                })
            },
            &mut warnings,
        );
        assert_eq!((size, face.map(|face| face.index)), (17.0, Some(3)));
        assert!(warnings.is_empty());
    }

    #[test]
    fn a_missing_family_warns_with_its_key() {
        let font = UiFont {
            family: Some("No Such Family".into()),
            size_px: None,
        };
        let mut warnings = Warnings::new();
        let (_, face) = resolve_font(
            &font,
            || 13.0,
            |_| Err(crate::ui_font::Unavailable::NotInstalled),
            &mut warnings,
        );
        assert!(face.is_none());
        assert_eq!(
            warnings,
            [
                "`appearance.base.font.family` = \"No Such Family\" is not installed; using the default UI font"
            ]
        );
    }

    #[test]
    fn an_unloadable_family_warns_with_the_real_reason() {
        let font = UiFont {
            family: Some("UDev Gothic NF".into()),
            size_px: None,
        };
        let mut warnings = Warnings::new();
        let (_, face) = resolve_font(
            &font,
            || 13.0,
            |_| {
                Err(crate::ui_font::Unavailable::Unreadable(
                    "cannot open /fonts/u.ttf: Permission denied".into(),
                ))
            },
            &mut warnings,
        );
        assert!(face.is_none());
        assert_eq!(
            warnings,
            [
                "`appearance.base.font.family` = \"UDev Gothic NF\" cannot be loaded (cannot open /fonts/u.ttf: Permission denied); using the default UI font"
            ]
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
        let (config, mut warnings) = parse(
            "top = 1\n[appearance]\ntheme = \"x\"\n[appearance.tab_bar]\n[appearance.base]\ncolors = {}\n[appearance.base.font]\nsize_px = 16\nweight = 400",
        )
        .unwrap();
        assert_eq!(config.font.size_px, Some(16.0));
        // The order follows toml's map, which is not part of the contract.
        warnings.sort();
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
            assert_eq!(
                warnings,
                ["`appearance.base.font.family` is empty; using the default UI font"],
                "{family:?}"
            );
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
    fn warnings_found_before_a_structural_error_are_kept() {
        let mut warnings = Warnings::new();
        let text = "[appearance]\nalpha = 1\n[appearance.base.font]\nfamily = 1";
        assert!(super::parse(text, &mut warnings).is_err());
        assert_eq!(warnings, ["unknown key `appearance.alpha`"]);
    }

    #[test]
    fn control_characters_from_the_file_are_escaped_for_stderr() {
        let (_, warnings) = parse("\"\\u001b]0;x\\u0007\" = 1").unwrap();
        assert_eq!(warnings.len(), 1);
        let shown = printable(&warnings[0]);
        assert!(!shown.chars().any(char::is_control), "{shown}");
        assert!(shown.contains("\\u{1b}]0;x\\u{7}"), "{shown}");
    }

    #[test]
    fn a_parse_error_keeps_its_line_layout() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[appearance").unwrap();
        let error = read(&path, &mut Warnings::new()).unwrap_err().to_string();
        // toml points at the error on the line below the excerpt.
        assert!(error.contains("\n1 | [appearance\n"), "{error}");
    }

    #[test]
    fn an_invalid_file_names_its_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[appearance").unwrap();
        let error = read(&path, &mut Warnings::new()).unwrap_err().to_string();
        assert!(error.contains(&path.display().to_string()), "{error}");
    }

    /// Feeds key taps (press, then release) through a keymap configured by
    /// `text`.
    fn press(text: &str, keys: &[(eframe::egui::Key, eframe::egui::Modifiers)]) -> Vec<Action> {
        let (config, warnings) = parse(text).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        let mut shortcuts = crate::shortcuts::Shortcuts::from_config(
            &felis_client_core::config::EffectiveConfig::default(),
            config.keymap,
        );
        let events = keys
            .iter()
            .flat_map(|&(key, modifiers)| {
                [true, false].map(|pressed| eframe::egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers,
                })
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
    fn session_attach_can_be_bound_by_its_public_kind() {
        use eframe::egui::{Key, Modifiers};
        assert_eq!(
            press(
                "[keymap]\n\"alt+a\" = { kind = \"session:attach\" }",
                &[(Key::A, Modifiers::ALT)],
            ),
            [Action::AttachSession]
        );
    }

    #[test]
    fn every_public_command_is_bindable_without_a_separate_kind_list() {
        use eframe::egui::{Key, Modifiers};
        let mut kinds = std::collections::BTreeSet::new();
        for command in crate::commands::commands() {
            assert!(
                kinds.insert(command.kind),
                "duplicate kind {}",
                command.kind
            );
            let text = format!(
                r#"[keymap]
"alt+c" = {{ kind = "unbind" }}
"alt+t" = {{ kind = "unbind" }}
"alt+shift+t" = {{ kind = "unbind" }}
"alt+shift+p" = {{ kind = "unbind" }}
"alt+w" = {{ kind = "unbind" }}
"alt+q" = {{ kind = {:?} }}"#,
                command.kind
            );
            assert_eq!(
                press(&text, &[(Key::Q, Modifiers::ALT)]),
                std::slice::from_ref(&command.action),
                "{}",
                command.kind
            );
            let (config, _) = parse(&text).unwrap();
            assert_eq!(
                config.keymap.hints().get(&command.action),
                "Alt+Q",
                "{}",
                command.kind
            );
        }
    }

    #[test]
    fn canonical_commands_reject_arguments_instead_of_ignoring_them() {
        for command in crate::commands::commands() {
            let text = format!(
                "[keymap]\n\"alt+q\" = {{ kind = {:?}, unexpected = true }}",
                command.kind
            );
            let (config, warnings) = parse(&text).unwrap();
            assert_eq!(config.keymap, AppKeymap::default());
            assert_eq!(
                warnings,
                [format!(
                    "`keymap.\"alt+q\"` is dropped: `{}` has no field `unexpected`",
                    command.kind
                )]
            );
        }
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
            "f5" = { kind = "split", direction = "below" }
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
    fn focus_pane_moves_in_every_direction_or_to_the_next_pane() {
        use eframe::egui::{Key, Modifiers};
        let text = r#"[keymap]
            "f1" = { kind = "focus_pane", direction = "left" }
            "f2" = { kind = "focus_pane", direction = "down" }
            "f3" = { kind = "focus_pane", direction = "up" }
            "f4" = { kind = "focus_pane", direction = "right" }
            "f5" = { kind = "focus_pane", to = "next" }
        "#;
        let keys = [Key::F1, Key::F2, Key::F3, Key::F4, Key::F5].map(|key| (key, Modifiers::NONE));
        assert_eq!(
            press(text, &keys),
            [
                Action::Focus(Direction::Left),
                Action::Focus(Direction::Down),
                Action::Focus(Direction::Up),
                Action::Focus(Direction::Right),
                Action::NextPane,
            ]
        );
    }

    #[test]
    fn configured_sequences_replace_and_unbind_defaults() {
        use eframe::egui::{Key, Modifiers};
        let text = r#"[keymap]
            "alt+c" = { kind = "unbind" }
            "alt+w" = { kind = "unbind" }
            "ctrl+a s" = { kind = "split", direction = "below" }
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
    fn a_three_chord_sequence_fires_only_when_complete() {
        use eframe::egui::{Key, Modifiers};
        let text = "[keymap]\n\"ctrl+a b c\" = { kind = \"close_tab\" }";
        let none = Modifiers::NONE;
        assert_eq!(
            press(
                text,
                &[
                    (Key::A, Modifiers::CTRL),
                    (Key::B, none),
                    (Key::C, none),
                    (Key::A, Modifiers::CTRL),
                    (Key::B, none),
                    (Key::D, none),
                ]
            ),
            [Action::CloseTab]
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
    fn an_unknown_value_is_reported_with_the_accepted_ones() {
        let (config, warnings) = parse(
            r#"[keymap]
            "alt+y" = { kind = "split", direction = "down" }
        "#,
        )
        .unwrap();
        assert_eq!(config.keymap, AppKeymap::default());
        assert_eq!(
            warnings,
            [
                "`keymap.\"alt+y\"` is dropped: `down` is not a `direction` for `split`; use `below` or `right`"
            ]
        );
    }

    #[test]
    fn application_kinds_do_not_shadow_felis_kinds_except_unbind() {
        let schema = schemars::schema_for!(felis_client_core::BindingValue);
        let variants = schema.as_value()["oneOf"]
            .as_array()
            .expect("felis binding schema must enumerate its variants");
        assert!(
            variants.len() > 1,
            "the binding schema must not be empty or just unbind"
        );
        let mut shared_unbind = false;
        for variant in variants {
            let kind = variant["properties"]["kind"]["const"]
                .as_str()
                .expect("each binding variant must name its kind");
            let binding = toml::Value::Table(toml::Table::from_iter([(
                "kind".into(),
                toml::Value::String(kind.into()),
            )]));
            let result = parse_binding(&binding);
            if kind == "unbind" {
                assert_eq!(result, Ok(None));
                shared_unbind = true;
            } else {
                // Missing required arguments still identify a recognized application
                // kind; only an unknown-kind error proves the names do not overlap.
                assert!(
                    matches!(&result, Err(error) if error.starts_with(&format!("unknown kind `{kind}`;"))),
                    "felis kind `{kind}` overlaps the application vocabulary: {result:?}"
                );
            }
        }
        assert!(
            shared_unbind,
            "unbind must remain the explicit shared exception"
        );
    }

    #[test]
    fn unknown_kinds_suggest_the_terminal_document_without_claiming_support() {
        for kind in ["copy", "run", "new_tba", "future_terminal_action"] {
            let (config, warnings) =
                parse(&format!("[keymap]\n\"alt+q\" = {{ kind = \"{kind}\" }}")).unwrap();
            assert_eq!(config.keymap, AppKeymap::default(), "{kind}");
            assert_eq!(
                warnings,
                [format!(
                    "`keymap.\"alt+q\"` is dropped: unknown kind `{kind}`; terminal bindings may belong in felis's config.toml"
                )]
            );
        }
    }

    #[test]
    fn escape_cannot_continue_a_sequence() {
        let (config, warnings) = parse(
            r#"[keymap]
            "ctrl+a escape" = { kind = "close_tab" }
            "ctrl+a shift+escape" = { kind = "close_tab" }
            "escape" = { kind = "new_tab" }
        "#,
        )
        .unwrap();
        assert_eq!(warnings.len(), 2, "{warnings:#?}");
        assert!(
            warnings.iter().all(|warning| warning.contains("Escape")),
            "{warnings:#?}"
        );
        let mut expected = AppKeymap::default();
        expected.apply(
            vec![(
                "escape".into(),
                crate::shortcuts::parse_sequence("escape").unwrap(),
                Some(Action::NewTab),
            )],
            &mut vec![],
        );
        assert_eq!(config.keymap, expected);
    }

    #[test]
    fn clipboard_chords_are_dropped_because_they_never_arrive_as_keys() {
        let command = if cfg!(target_os = "macos") {
            "super"
        } else {
            "ctrl"
        };
        let (config, warnings) = parse(&format!(
            r#"[keymap]
            "{command}+c" = {{ kind = "new_tab" }}
            "{command}+x" = {{ kind = "new_tab" }}
            "alt+w {command}+v" = {{ kind = "close_tab" }}
            "{command}+shift+c" = {{ kind = "close_tab" }}
        "#
        ))
        .unwrap();
        assert_eq!(warnings.len(), 3, "{warnings:#?}");
        assert!(
            warnings.iter().all(|warning| warning.contains("clipboard")),
            "{warnings:#?}"
        );
        let mut expected = AppKeymap::default();
        let spelling = format!("{command}+shift+c");
        expected.apply(
            vec![(
                spelling.clone(),
                crate::shortcuts::parse_sequence(&spelling).unwrap(),
                Some(Action::CloseTab),
            )],
            &mut vec![],
        );
        assert_eq!(config.keymap, expected);
    }

    #[test]
    fn chords_the_window_cannot_deliver_are_dropped() {
        let (config, warnings) = parse(
            r#"[keymap]
            "alt+é" = { kind = "new_tab" }
            "alt+w ctrl+¥" = { kind = "close_tab" }
            "super+n" = { kind = "close_tab" }
            "ctrl+'" = { kind = "command_palette" }
        "#,
        )
        .unwrap();
        let undeliverable = if cfg!(target_os = "macos") { 2 } else { 3 };
        assert_eq!(warnings.len(), undeliverable, "{warnings:#?}");
        assert!(
            warnings
                .iter()
                .all(|warning| warning.contains("never arrives")),
            "{warnings:#?}"
        );
        let mut expected = AppKeymap::default();
        let mut entries = vec![(
            "ctrl+'".to_string(),
            crate::shortcuts::parse_sequence("ctrl+'").unwrap(),
            Some(Action::Palette),
        )];
        if cfg!(target_os = "macos") {
            entries.push((
                "super+n".into(),
                crate::shortcuts::parse_sequence("super+n").unwrap(),
                Some(Action::CloseTab),
            ));
        }
        expected.apply(entries, &mut vec![]);
        assert_eq!(config.keymap, expected);
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
    fn an_unbind_that_removes_a_configured_binding_warns() {
        let (config, warnings) = parse(
            r#"[keymap]
            "Alt+w v" = { kind = "split", direction = "right" }
            "alt+w" = { kind = "unbind" }
        "#,
        )
        .unwrap();
        assert!(
            matches!(&warnings[..], [warning] if warning.contains("\"Alt+w v\"") && warning.contains("\"alt+w\"")),
            "{warnings:#?}"
        );
        let mut expected = AppKeymap::default();
        expected.apply(
            vec![(
                "alt+w".into(),
                crate::shortcuts::parse_sequence("alt+w").unwrap(),
                None,
            )],
            &mut vec![],
        );
        assert_eq!(config.keymap, expected);
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
