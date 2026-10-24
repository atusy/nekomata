//! The local felis document, resolved once for this GUI's client identity.
use eframe::egui;
use felis_client_core::config::{EffectiveConfig, FontStyleConfig};
use felis_protocol::messages::SessionToDaemonMsg;
use felis_render_wgpu::RendererConfig;
use felis_shaping::FaceSpec;
use std::sync::Arc;

pub fn load(ctx: &egui::Context) -> anyhow::Result<()> {
    install(ctx, EffectiveConfig::try_load("nekomata")?);
    Ok(())
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
    use felis_client_core::config::ConfigDocument;
    use felis_render_core::palette::linear_rgba_to_srgb8;

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
}
