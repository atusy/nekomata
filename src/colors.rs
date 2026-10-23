//! Bounded per-session color state interpreted through felis's renderer effects.
use felis_protocol::messages::GridMsg;
use felis_render_core::palette::Theme;

#[derive(Default)]
pub struct Colors {
    runtime_theme: [Option<[u8; 3]>; 3],
    runtime_palette: std::collections::BTreeMap<u8, [u8; 3]>,
    revision: u64,
}
impl Colors {
    pub fn apply_grid(&mut self, msg: &GridMsg) {
        use felis_client_core::renderer_effect::{RendererEffect, renderer_effect};
        match renderer_effect(msg) {
            Some(RendererEffect::ThemeOverride { channel, rgb }) => {
                self.runtime_theme[channel as usize] = rgb.map(Into::into)
            }
            Some(RendererEffect::PaletteOverride {
                index,
                rgb: Some(rgb),
            }) => {
                self.runtime_palette.insert(index, rgb.into());
            }
            Some(RendererEffect::PaletteOverride { index, rgb: None }) => {
                self.runtime_palette.remove(&index);
            }
            Some(RendererEffect::ResetPalette) => self.runtime_palette.clear(),
            Some(RendererEffect::ResetSessionColors) => {
                self.runtime_theme = [None; 3];
                self.runtime_palette.clear();
            }
            Some(RendererEffect::ReverseVideo(_)) | None => return,
        }
        self.revision = self.revision.wrapping_add(1);
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn theme(&self) -> Theme {
        let mut palette = [None; 256];
        for (&index, &rgb) in &self.runtime_palette {
            palette[usize::from(index)] = Some(rgb);
        }
        felis_render_core::palette::effective_theme(
            &crate::theme::terminal_theme(),
            &self.runtime_theme,
            &palette,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use felis_protocol::messages::{PaletteAction, ThemeAction, ThemeChannel};
    use felis_render_core::palette::linear_rgba_to_srgb8;

    #[test]
    fn runtime_palette_changes_override_the_configured_palette_until_reset() {
        let mut colors = Colors::default();
        colors.apply_grid(&GridMsg::PaletteColor {
            index: 4,
            action: PaletteAction::Set { rgb: (12, 34, 56) },
        });
        assert_eq!(colors.theme().palette[4], Some([12, 34, 56]));
        colors.apply_grid(&GridMsg::PaletteColor {
            index: 4,
            action: PaletteAction::Reset,
        });
        assert_eq!(
            colors.theme().palette[4],
            Some(crate::theme::ANSI_PALETTE[4])
        );
    }
    #[test]
    fn rehydration_drops_colors_from_the_previous_session() {
        let mut colors = Colors::default();
        colors.apply_grid(&GridMsg::ThemeColor {
            channel: ThemeChannel::Foreground,
            action: ThemeAction::Set { rgb: (1, 2, 3) },
        });
        assert_eq!(linear_rgba_to_srgb8(colors.theme().fg), [1, 2, 3, 255]);
        colors.apply_grid(&GridMsg::RehydrateBegin);
        assert_eq!(
            linear_rgba_to_srgb8(colors.theme().fg),
            crate::theme::FG.to_array()
        );
    }
}
