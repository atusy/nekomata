//! The felis `config.toml`, resolved with nekomata's `[client.nekomata]` overlay.
use std::path::Path;

use felis_client_core::{
    ConfigDiagnostics, DiagnosticKind, EffectiveConfig, Severity, config::FontConfig,
};

use crate::{
    theme::TerminalTheme,
    workspace::{FontSize, MAX_FONT_SIZE, MIN_FONT_SIZE},
};

/// Selects the `[client.nekomata]` overlay in the shared felis file.
pub const CLIENT_ID: &str = "nekomata";

#[derive(Debug, Default)]
pub struct Loaded {
    pub theme: TerminalTheme,
    /// `[font]` faces; the size is applied separately as a default.
    pub font: FontConfig,
    /// `[font] size_px`, the default for sessions that were never zoomed.
    pub font_size: FontSize,
    /// One entry per problem, for the terminal that launched nekomata.
    pub diagnostics: Vec<String>,
    /// Set when the file was ignored, so the window can say why in one line.
    pub error: Option<String>,
}

/// Never fails: a missing file means defaults, and an unusable one is
/// reported and replaced by defaults so the window still opens.
pub fn load() -> Loaded {
    let Some(path) = felis_client_core::config_path() else {
        return Loaded::default();
    };
    let (config, diagnostics) = EffectiveConfig::diagnose(&path, CLIENT_ID);
    let loaded = settle(&path, config, &diagnostics);
    for line in &loaded.diagnostics {
        eprintln!("{line}");
    }
    loaded
}

fn settle(path: &Path, config: EffectiveConfig, diagnostics: &ConfigDiagnostics) -> Loaded {
    let line = |severity: &str, problem: &dyn std::fmt::Display| {
        format!("nekomata: {}: {severity}: {problem}", path.display())
    };
    let mut lines: Vec<String> = diagnostics
        .iter()
        .map(|diagnostic| {
            let severity = match diagnostic.severity {
                Severity::Error => "error",
                Severity::Warning => "warning",
            };
            // A read failure's message already names the file.
            if diagnostic.kind == DiagnosticKind::Io {
                format!("nekomata: {severity}: {diagnostic}")
            } else {
                line(severity, diagnostic)
            }
        })
        .collect();
    let (config, error) = match diagnostics.errors().next() {
        Some(first) => {
            // A TOML syntax error carries a multi-line source excerpt; the
            // window notice keeps its first line and stderr has the rest.
            let problem = first.to_string();
            let problem = problem.lines().next().unwrap_or_default();
            let notice = if first.kind == DiagnosticKind::Io {
                format!("Used default settings: {problem}")
            } else {
                format!("Ignored {} and used defaults: {problem}", path.display())
            };
            (EffectiveConfig::default(), Some(notice))
        }
        None => (config, None),
    };
    let theme = TerminalTheme::from_config(&config, &mut |warning| {
        lines.push(line("warning", &warning));
    });
    let font_size = config.font.sanitized_font_size_logical_px();
    // Compare with the written value: felis has already clamped into its
    // own wider band and warned about that.
    if let (Some(written), Some(size)) = (config.font.size_px, font_size) {
        let used = FontSize::new(Some(size)).get();
        if used != size {
            lines.push(line(
                "warning",
                &format!(
                    "font.size_px: {written} is outside nekomata's \
                     {MIN_FONT_SIZE}..={MAX_FONT_SIZE} band; using {used}"
                ),
            ));
        }
    }
    Loaded {
        theme,
        font_size: FontSize::new(font_size),
        font: config.font,
        diagnostics: lines.iter().map(|line| printable(line)).collect(),
        error: error.as_deref().map(printable),
    }
}

/// Escapes control characters a key in the file may carry, so echoing a
/// diagnostic cannot drive the terminal that launched nekomata.
fn printable(text: &str) -> String {
    text.chars()
        .flat_map(|c| {
            let keep = !c.is_control() || c == '\n';
            let (kept, escaped) = if keep {
                (Some(c), None)
            } else {
                (None, Some(c.escape_default()))
            };
            kept.into_iter().chain(escaped.into_iter().flatten())
        })
        .collect()
}

#[cfg(test)]
pub fn resolve(text: &str) -> Loaded {
    let path = Path::new("/config/felis/config.toml");
    match felis_client_core::ConfigDocument::parse(text, Some(path.into())) {
        Ok(document) => {
            let (config, diagnostics) = document.resolve(CLIENT_ID);
            settle(path, config, &diagnostics)
        }
        Err(error) => settle(path, EffectiveConfig::default(), &error.diagnostics()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::Color32;

    #[test]
    fn nekomata_overlay_wins_over_shared_settings() {
        let loaded = resolve(
            r##"
            [theme]
            foreground = "#111111"
            background = "#222222"

            [client.nekomata.theme]
            foreground = "#333333"
            "##,
        );
        assert_eq!(
            loaded.theme.foreground(),
            Color32::from_rgb(0x33, 0x33, 0x33)
        );
        assert_eq!(
            loaded.theme.background(),
            Color32::from_rgb(0x22, 0x22, 0x22)
        );
        assert!(loaded.diagnostics.is_empty(), "{:?}", loaded.diagnostics);
    }

    #[test]
    fn other_clients_overlays_are_inert() {
        let loaded = resolve(
            r##"
            [client.felis.theme]
            foreground = "#333333"

            [client.felis]
            not_a_key = 1
            "##,
        );
        assert_eq!(loaded.theme, TerminalTheme::default());
        assert!(loaded.diagnostics.is_empty(), "{:?}", loaded.diagnostics);
        assert_eq!(loaded.error, None);
    }

    #[test]
    fn unknown_keys_warn_and_keep_the_rest() {
        let loaded = resolve(
            r##"
            [theme]
            background = "#222222"
            backgrund = "#333333"
            "##,
        );
        assert_eq!(
            loaded.theme.background(),
            Color32::from_rgb(0x22, 0x22, 0x22)
        );
        assert_eq!(loaded.error, None);
        assert_eq!(
            loaded.diagnostics,
            [
                "nekomata: /config/felis/config.toml: warning: theme.backgrund: unknown key ignored (typo, or written for another client?)"
            ]
        );
    }

    #[test]
    fn the_configured_font_size_becomes_the_default() {
        let loaded = resolve("[font]\nsize_px = 13.0\n\n[client.nekomata.font]\nsize_px = 18.0\n");
        assert_eq!(loaded.font_size, FontSize::new(Some(18.0)));
        assert_eq!(loaded.font_size.zoom, None);
        assert!(loaded.diagnostics.is_empty(), "{:?}", loaded.diagnostics);
        assert_eq!(resolve("").font_size, FontSize::default());
    }

    #[test]
    fn a_font_size_below_nekomatas_band_warns_and_clamps() {
        let loaded = resolve("[font]\nsize_px = 5.0\n");
        assert_eq!(loaded.font_size.get(), 6.0);
        assert_eq!(
            loaded.diagnostics,
            [
                "nekomata: /config/felis/config.toml: warning: font.size_px: 5 is outside nekomata's 6..=72 band; using 6"
            ]
        );
    }

    #[test]
    fn size_warnings_name_the_written_value_once() {
        let loaded = resolve("[font]\nsize_px = 2.0\n");
        assert_eq!(loaded.font_size.get(), 6.0);
        let ours: Vec<_> = loaded
            .diagnostics
            .iter()
            .filter(|line| line.contains("nekomata's"))
            .collect();
        assert_eq!(
            ours,
            [
                "nekomata: /config/felis/config.toml: warning: font.size_px: 2 is outside nekomata's 6..=72 band; using 6"
            ]
        );

        let loaded = resolve("[font]\nsize_px = 100.0\n");
        assert_eq!(loaded.font_size.get(), 72.0);
        assert_eq!(loaded.diagnostics.len(), 1, "{:?}", loaded.diagnostics);
        assert!(!loaded.diagnostics[0].contains("nekomata's"));
    }

    #[test]
    fn diagnostics_escape_control_characters_from_the_file() {
        let loaded = resolve("[theme]\n\"\\u001b]0;x\\u0007\" = 1\n");
        assert_eq!(loaded.diagnostics.len(), 1, "{:?}", loaded.diagnostics);
        let line = &loaded.diagnostics[0];
        assert!(!line.chars().any(|c| c.is_control()), "{line:?}");
        assert!(line.contains(r"\u{1b}]0;x\u{7}"), "{line}");
    }

    #[test]
    fn an_unreadable_file_names_its_path_once() {
        let path = Path::new("/config/felis/config.toml");
        let error = felis_client_core::LoadError::Read {
            path: path.into(),
            source: std::io::Error::from(std::io::ErrorKind::PermissionDenied),
        };
        let loaded = settle(path, EffectiveConfig::default(), &error.diagnostics());
        assert_eq!(loaded.theme, TerminalTheme::default());
        assert_eq!(loaded.diagnostics.len(), 1);
        let [line] = &loaded.diagnostics[..] else {
            unreachable!()
        };
        assert!(
            line.starts_with("nekomata: error: cannot read /config/felis/config.toml: "),
            "{line}"
        );
        let error = loaded.error.unwrap();
        assert_eq!(
            error.matches("/config/felis/config.toml").count(),
            1,
            "{error}"
        );
    }

    #[test]
    fn an_unusable_file_falls_back_to_defaults_with_an_error() {
        for text in ["[theme\n", "[font]\nsize_px = \"large\"\n"] {
            let loaded = resolve(&format!("[theme]\nbackground = \"#222222\"\n{text}"));
            assert_eq!(loaded.theme, TerminalTheme::default(), "{text}");
            let error = loaded.error.expect(text);
            assert!(
                error.starts_with("Ignored /config/felis/config.toml and used defaults: "),
                "{error}"
            );
            assert!(!error.contains('\n'), "{error:?}");
            assert!(
                loaded
                    .diagnostics
                    .iter()
                    .any(|line| line.contains(": error: ")),
                "{:?}",
                loaded.diagnostics
            );
        }
    }
}
