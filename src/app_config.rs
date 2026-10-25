//! The application-layer document nekomata owns, beside felis's shared one.
use std::path::{Path, PathBuf};

const FONT_SIZE_RANGE: std::ops::RangeInclusive<f32> = 4.0..=72.0;

#[derive(Debug, Default, PartialEq)]
pub struct AppConfig {
    pub font: UiFont,
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
            // Parsed with the application keymap.
            "keymap" => {
                table(value, "keymap")?;
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
                out.family = Some(family.to_owned());
            }
            "size_px" => {
                // Either TOML spelling is accepted: 15 and 15.0 are one size.
                let size = match value {
                    toml::Value::Integer(size) => *size as f32,
                    toml::Value::Float(size) => *size as f32,
                    _ => return Err(format!("`{PATH}.size_px` must be a number")),
                };
                out.size_px = if size.is_nan() || size <= 0.0 {
                    warnings.push(format!(
                        "`{PATH}.size_px` must be positive; using the default"
                    ));
                    None
                } else if !FONT_SIZE_RANGE.contains(&size) {
                    let clamped = size.clamp(*FONT_SIZE_RANGE.start(), *FONT_SIZE_RANGE.end());
                    warnings.push(format!(
                        "`{PATH}.size_px` = {size} is outside [4, 72]; using {clamped}"
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
}
