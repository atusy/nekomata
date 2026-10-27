//! The local felis document, resolved once for this GUI's client identity.
use eframe::egui;
use felis_client_core::config::{EffectiveConfig, FontStyleConfig};
use felis_protocol::messages::SessionToDaemonMsg;
use felis_render_wgpu::RendererConfig;
use felis_shaping::FaceSpec;
use std::path::Path;
use std::sync::Arc;

use crate::config_report::Problem;

/// The document as nekomata applies it, with every diagnostic felis
/// reports for it. As felis's own startup does, a document with an error
/// is ignored for felis's defaults.
pub fn load() -> (EffectiveConfig, Vec<Problem>) {
    match felis_client_core::config::config_path() {
        Some(path) => load_from(&path),
        None => (EffectiveConfig::default(), vec![]),
    }
}

/// Collects diagnostics instead of letting felis log them through
/// `tracing`, which nothing here subscribes to.
fn load_from(path: &Path) -> (EffectiveConfig, Vec<Problem>) {
    let (config, diagnostics) = EffectiveConfig::diagnose(path, "nekomata");
    let problems = diagnostics
        .iter()
        .map(|diagnostic| match diagnostic.severity {
            felis_client_core::config::Severity::Error => {
                Problem::error(path, diagnostic.to_string())
            }
            felis_client_core::config::Severity::Warning => {
                Problem::warning(path, diagnostic.to_string())
            }
        })
        .collect();
    let config = if diagnostics.has_errors() {
        EffectiveConfig::default()
    } else {
        config
    };
    (config, problems)
}

pub fn install(ctx: &egui::Context, config: EffectiveConfig) {
    ctx.data_mut(|data| data.insert_temp(egui::Id::unique("terminal-config"), Arc::new(config)));
}

pub fn get(ctx: &egui::Context) -> Arc<EffectiveConfig> {
    ctx.data(|data| data.get_temp(egui::Id::unique("terminal-config")))
        .unwrap_or_default()
}

pub fn font_size(config: &EffectiveConfig) -> f32 {
    config
        .font
        .sanitized_font_size_logical_px()
        .unwrap_or(felis_render_wgpu::DEFAULT_FONT_SIZE_LOGICAL_PX)
}

pub fn renderer(config: &EffectiveConfig) -> RendererConfig {
    let face = |c: &FontStyleConfig| FaceSpec {
        family: c.family.clone(),
        features: c.features.clone(),
    };
    RendererConfig {
        font_family: config.font.family.clone(),
        font_fallbacks: config.font.fallback.iter().map(face).collect(),
        font_features: config.font.features.clone(),
        font_bold: face(&config.font.bold),
        font_italic: face(&config.font.italic),
        font_bold_italic: face(&config.font.bold_italic),
        theme_fg: config.theme.foreground.clone(),
        theme_bg: config.theme.background.clone(),
        theme_cursor: config.cursor.color.clone(),
        theme_palette: config.theme.palette.clone().into_overrides(),
        theme_minimum_contrast: Some(config.theme.clamped_minimum_contrast()),
        ..Default::default()
    }
}

pub fn configure(config: &EffectiveConfig) -> SessionToDaemonMsg {
    use felis_render_core::palette::{configured_palette_report, configured_theme_report};
    let (fg, bg, cursor) = configured_theme_report(
        config.theme.foreground.as_deref(),
        config.theme.background.as_deref(),
        config.cursor.color.as_deref(),
    );
    SessionToDaemonMsg::ConfigureTheme {
        fg,
        bg,
        cursor,
        palette: configured_palette_report(&config.theme.palette.clone().into_overrides()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config_report::Severity;
    use felis_client_core::config::ConfigDocument;
    use felis_render_core::palette::linear_rgba_to_srgb8;

    /// `EffectiveConfig` has no `PartialEq`; its serialized form stands in.
    fn is_default(config: &EffectiveConfig) -> bool {
        let serialized = |config| toml::Value::try_from(config).unwrap();
        serialized(config) == serialized(&EffectiveConfig::default())
    }

    fn load_text(text: &str) -> (EffectiveConfig, Vec<Problem>, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, text).unwrap();
        let (config, problems) = load_from(&path);
        (config, problems, path)
    }

    #[test]
    fn a_missing_document_is_felis_defaults_without_problems() {
        let dir = tempfile::tempdir().unwrap();
        let (config, problems) = load_from(&dir.path().join("config.toml"));
        assert!(is_default(&config));
        assert!(problems.is_empty());
    }

    #[test]
    fn a_broken_document_falls_back_to_defaults_and_reports_its_error() {
        for text in ["[font", "[font]\nsize_px = 15\nfamily = 1"] {
            let (config, problems, path) = load_text(text);
            assert!(is_default(&config), "{text}");
            assert!(
                problems
                    .iter()
                    .any(|problem| problem.severity == Severity::Error && problem.source == path),
                "{text}: {problems:?}"
            );
        }
    }

    #[test]
    fn an_unreadable_document_falls_back_to_defaults_and_reports_why() {
        let dir = tempfile::tempdir().unwrap();
        // A directory where the file should be cannot be read as one.
        let (config, problems) = load_from(dir.path());
        assert!(is_default(&config));
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].severity, Severity::Error);
        assert!(problems[0].message.contains("cannot read"), "{problems:?}");
    }

    #[test]
    fn warnings_are_reported_and_the_rest_of_the_document_applies() {
        let (config, problems, path) = load_text("[font]\nsize_px = 15\nbogus = 1");
        assert_eq!(font_size(&config), 15.0);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert_eq!(problems[0].severity, Severity::Warning);
        assert_eq!(problems[0].source, path);
        assert!(
            problems[0].message.starts_with("font.bogus: "),
            "{problems:?}"
        );
    }

    #[test]
    fn nekomata_overlay_drives_renderer_and_daemon_colors() {
        let document = ConfigDocument::parse(
            r##"
            [font]
            family = "monospace"
            size_px = 14
            [theme]
            foreground = "#112233"
            [client.felis.theme]
            foreground = "#ffffff"
            [client.nekomata.font]
            size_px = 19
            [client.nekomata.theme]
            background = "#445566"
            [client.nekomata.theme.palette]
            blue = "#778899"
            [client.nekomata.cursor]
            color = "#aabbcc"
        "##,
            None,
        )
        .unwrap();
        let (config, diagnostics) = document.resolve("nekomata");
        assert!(!diagnostics.has_errors());
        let ctx = egui::Context::default();
        install(&ctx, config);
        let config = get(&ctx);
        assert_eq!(font_size(&config), 19.0);
        let renderer_config = renderer(&config);
        assert_eq!(renderer_config.font_family.as_deref(), Some("monospace"));
        let theme = felis_render_wgpu::Renderer::theme_from_config(&renderer_config);
        let SessionToDaemonMsg::ConfigureTheme {
            fg,
            bg,
            cursor,
            palette,
        } = configure(&config)
        else {
            panic!("expected theme report");
        };
        assert_eq!(fg, Some((0x11, 0x22, 0x33)));
        assert_eq!(bg, Some((0x44, 0x55, 0x66)));
        assert_eq!(cursor, Some((0xaa, 0xbb, 0xcc)));
        assert_eq!(palette, [(4, (0x77, 0x88, 0x99))].into());
        assert_eq!(linear_rgba_to_srgb8(theme.fg), [0x11, 0x22, 0x33, 255]);
        assert_eq!(linear_rgba_to_srgb8(theme.bg), [0x44, 0x55, 0x66, 255]);
        assert_eq!(theme.palette[4], Some([0x77, 0x88, 0x99]));
    }

    #[test]
    fn nekomata_overlay_selects_the_osc52_policy() {
        use felis_client_core::config::Osc52Policy;
        let resolve = |toml: &str| {
            let (config, diagnostics) = ConfigDocument::parse(toml, None)
                .unwrap()
                .resolve("nekomata");
            assert!(!diagnostics.has_errors());
            config.clipboard.osc_52
        };
        assert_eq!(resolve(""), Osc52Policy::Mirror);
        assert_eq!(
            resolve(
                r#"
                [clipboard]
                osc_52 = "system"
                [client.nekomata.clipboard]
                osc_52 = "mirror"
            "#
            ),
            Osc52Policy::Mirror
        );
        assert_eq!(
            resolve(
                r#"
                [client.nekomata.clipboard]
                osc_52 = "system"
            "#
            ),
            Osc52Policy::System
        );
    }
}
