//! The felis `config.toml`, resolved with nekomata's `[client.nekomata]` overlay.
use std::path::Path;

use felis_client_core::{ConfigDiagnostics, EffectiveConfig, Severity};

use crate::theme::TerminalTheme;

/// Selects the `[client.nekomata]` overlay in the shared felis file.
pub const CLIENT_ID: &str = "nekomata";

#[derive(Debug, Default)]
pub struct Loaded {
    pub theme: TerminalTheme,
    /// One line per problem, for the terminal that launched nekomata.
    pub diagnostics: Vec<String>,
    /// Set when the file was ignored, so the window can say why.
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
            line(severity, diagnostic)
        })
        .collect();
    let (config, error) = match diagnostics.errors().next() {
        Some(first) => (
            EffectiveConfig::default(),
            Some(format!(
                "Ignored {} and used defaults: {first}",
                path.display()
            )),
        ),
        None => (config, None),
    };
    let theme = TerminalTheme::from_config(&config, &mut |warning| {
        lines.push(line("warning", &warning));
    });
    Loaded {
        theme,
        diagnostics: lines,
        error,
    }
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
    fn an_unusable_file_falls_back_to_defaults_with_an_error() {
        for text in ["[theme\n", "[font]\nsize_px = \"large\"\n"] {
            let loaded = resolve(&format!("[theme]\nbackground = \"#222222\"\n{text}"));
            assert_eq!(loaded.theme, TerminalTheme::default(), "{text}");
            let error = loaded.error.expect(text);
            assert!(
                error.starts_with("Ignored /config/felis/config.toml and used defaults: "),
                "{error}"
            );
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
