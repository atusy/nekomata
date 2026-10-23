//! Compose the upstream felis GPU renderer into egui on the same device.
use crate::session::State;
use anyhow::{Context, Result};
use eframe::{
    egui::{self, Color32, Pos2, Rect, TextureId, Vec2},
    egui_wgpu::{RenderState, wgpu},
};
use felis_client_core::ImageShadow;
use felis_render_core::{ImageData, ImageSource, SelectionRange};
use felis_render_wgpu::{Renderer as GpuRenderer, RendererConfig};
use felis_shaping::{FontStack, StyleFaces};
use std::sync::Arc;

pub fn initialize(cc: &eframe::CreationContext<'_>) -> Result<()> {
    let gpu = cc
        .wgpu_render_state
        .as_ref()
        .context("wgpu renderer required")?;
    cc.egui_ctx
        .data_mut(|data| data.insert_temp(egui::Id::unique("terminal-gpu"), gpu.clone()));
    load_fonts(&cc.egui_ctx)?;
    Ok(())
}

pub fn load_fonts(ctx: &egui::Context) -> Result<Arc<FontStack>> {
    let key = egui::Id::unique("terminal-fonts");
    if let Some(stack) = ctx.data(|data| data.get_temp::<Arc<FontStack>>(key)) {
        return Ok(stack);
    }
    let styles = StyleFaces::default();
    let stack = if let Some(path) = std::env::var_os("NEKOMATA_FONT") {
        FontStack::discover_in_files(&[path.into()], None, &[], &[], &styles)?
    } else {
        FontStack::auto_discover(None, &[], &[], &styles)?
    };
    let stack = Arc::new(stack);
    ctx.data_mut(|data| data.insert_temp(key, stack.clone()));
    Ok(stack)
}

pub struct Renderer {
    gpu: GpuRenderer,
    host: RenderState,
    texture: TextureId,
    size: [u32; 2],
    font_size: f32,
    ppp: f32,
    color_revision: u64,
    layout: felis_client_core::ImageLayout,
    rendered: bool,
    focused: bool,
    selection: Option<(usize, usize)>,
}

impl Renderer {
    pub fn new(ctx: &egui::Context, font_size: f32, ppp: f32) -> Result<Self> {
        let host = ctx
            .data(|data| data.get_temp::<RenderState>(egui::Id::unique("terminal-gpu")))
            .context("terminal GPU was not initialized")?;
        let cfg = RendererConfig {
            font_size_physical_px: Some(font_size * ppp),
            font_files: std::env::var_os("NEKOMATA_FONT")
                .map(Into::into)
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let mut gpu = GpuRenderer::new_embedded(
            host.device.clone(),
            host.queue.clone(),
            1,
            1,
            cfg,
            (*load_fonts(ctx)?).clone(),
        )?;
        gpu.set_theme(&crate::theme::terminal_theme());
        let view = gpu
            .embedded_texture_view()
            .context("missing embedded GPU target")?;
        let texture = host.renderer.write().register_native_texture(
            &host.device,
            &view,
            wgpu::FilterMode::Nearest,
        );
        Ok(Self {
            gpu,
            host,
            texture,
            size: [1, 1],
            font_size,
            ppp,
            color_revision: 0,
            layout: Default::default(),
            rendered: false,
            focused: false,
            selection: None,
        })
    }

    pub fn set_font(&mut self, font_size: f32, ppp: f32) {
        if self.font_size != font_size || self.ppp != ppp {
            self.gpu.reload_font_size(font_size * ppp);
            self.font_size = font_size;
            self.ppp = ppp;
            self.rendered = false;
        }
    }

    pub fn cell_size(&self) -> Vec2 {
        let metrics = self.gpu.cell_metrics();
        Vec2::new(metrics.width as f32, metrics.height as f32) / self.ppp
    }

    pub fn content_rect(&self, rect: Rect) -> Rect {
        let (x, y) = self.gpu.content_origin_px();
        rect.translate(Vec2::new(x, y) / self.ppp)
    }

    /// A successful draw submits to the shared GPU queue before egui samples it.
    /// No read_frame/capture path is used by this interactive renderer.
    pub fn paint(
        &mut self,
        painter: &egui::Painter,
        state: &mut State,
        rect: Rect,
        focused: bool,
        selection: Option<(usize, usize)>,
    ) -> Result<()> {
        if state.render_ready() {
            let size = [
                (rect.width() * self.ppp).round().max(1.0) as u32,
                (rect.height() * self.ppp).round().max(1.0) as u32,
            ];
            let changed = !self.rendered
                || self.size != size
                || self.focused != focused
                || self.selection != selection;
            if changed {
                state.redraw.request();
            }
            if !state.redraw.flush() {
                painter.image(
                    self.texture,
                    rect,
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );
                return Ok(());
            }
            self.focused = focused;
            self.selection = selection;
            if self.size != size {
                self.gpu.resize(size[0], size[1]);
                let view = self
                    .gpu
                    .embedded_texture_view()
                    .context("missing resized GPU target")?;
                self.host
                    .renderer
                    .write()
                    .update_egui_texture_from_wgpu_texture(
                        &self.host.device,
                        &view,
                        wgpu::FilterMode::Nearest,
                        self.texture,
                    );
                self.size = size;
            }
            if self.color_revision != state.colors.revision() {
                self.gpu.set_theme(&state.colors.theme());
                self.color_revision = state.colors.revision();
            }
            self.gpu.set_reverse_video(state.screen.reverse_video());
            self.gpu.set_window_focused(focused);
            self.gpu.set_cursor_blink_visible(true);
            let cols = usize::from(state.screen.screen().cols()).max(1);
            self.gpu.set_selection(selection.map(|(a, b)| {
                let (a, b) = (a.min(b), a.max(b));
                SelectionRange {
                    start: ((a / cols) as u16, (a % cols) as u16),
                    end: ((b / cols) as u16, (b % cols) as u16),
                    rectangle: false,
                }
            }));
            let metrics = self.gpu.cell_metrics();
            state.images.layout_into(
                &mut self.layout,
                state.screen.screen(),
                (metrics.width as f32, metrics.height as f32),
                state.screen.viewport(),
            );
            let dirty = state.images.take_dirty_images();
            let result = self.gpu.render(
                state.screen.screen(),
                &dirty,
                &Images(&state.images),
                &self.layout,
                state.screen.viewport(),
                state.screen.viewport_max(),
            );
            if let Err(error) = result {
                state.redraw.request();
                return Err(error.into());
            }
            state.redraw.painted();
            state.screen.clear_damage();
            self.rendered = true;
        }
        if self.rendered {
            painter.image(
                self.texture,
                rect,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        Ok(())
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        self.host.renderer.write().free_texture(&self.texture);
    }
}

struct Images<'a>(&'a ImageShadow);
impl ImageSource for Images<'_> {
    fn image_data(&self, id: felis_protocol::ImageId) -> Option<ImageData<'_>> {
        let image = self.0.image(id).filter(|image| image.is_drawable())?;
        Some(ImageData {
            width: image.width,
            height: image.height,
            format: image.format,
            pixels: image.pixels(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use felis_grid::{
        Attributes, Cell, Color, StyleTable,
        wire::{RowEncode, encode_row},
    };
    use felis_protocol::{
        ImageId, RowPayload,
        messages::{GridMsg, ImageFormat, ImageMsg, ImageTarget},
    };

    #[test]
    #[ignore = "requires a GPU adapter; run inside the Vitro desktop with --ignored"]
    fn gpu_shared_device_preserves_palette_and_image_pixels() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let instance = wgpu::Instance::default();
        let adapter = runtime
            .block_on(instance.request_adapter(&Default::default()))
            .expect("GPU adapter");
        let (device, queue) = runtime
            .block_on(adapter.request_device(&Default::default()))
            .expect("GPU device");
        let ctx = egui::Context::default();
        let cfg = RendererConfig {
            font_size_physical_px: Some(15.0),
            ..Default::default()
        };
        let mut gpu = GpuRenderer::new_embedded(
            device,
            queue,
            64,
            48,
            cfg,
            (*load_fonts(&ctx).unwrap()).clone(),
        )
        .unwrap();
        gpu.set_theme(&crate::theme::terminal_theme());
        gpu.set_window_focused(false);
        assert!(gpu.embedded_texture_view().is_some());
        let mut shadow = felis_client_core::ShadowScreen::new(2, 4);
        let mut styles = StyleTable::new();
        let style = styles.intern(Attributes {
            bg: Color::Indexed(4),
            ..Default::default()
        });
        let cells = [Cell {
            style,
            ..Default::default()
        }; 4];
        let row = encode_row(
            RowEncode {
                cells: &cells,
                pad_to: 4,
                sized_cells: &[],
                soft_wrap_continued: false,
            },
            &styles,
        )
        .unwrap();
        shadow
            .apply(&GridMsg::RowDelta {
                rows: vec![(0, RowPayload(row))],
            })
            .unwrap();
        let mut images = ImageShadow::new();
        let id = ImageId(1);
        for msg in [
            ImageMsg::Header {
                id,
                target: ImageTarget::New {
                    width: 4,
                    height: 4,
                    format: ImageFormat::Rgb24,
                    serial: None,
                },
            },
            ImageMsg::Chunk {
                id,
                bytes: [240, 20, 40].repeat(16).into(),
            },
            ImageMsg::Complete { id },
            ImageMsg::Placement {
                image_id: id,
                placement_id: None,
                anchor_row: 1,
                anchor_col: 1,
                cols: 1,
                rows: 1,
                source: None,
                z_index: 1,
                requested: Some(felis_protocol::messages::RequestedExtent { cols: 1, rows: 1 }),
                serial: None,
            },
        ] {
            images.apply(&msg).unwrap();
        }
        let metrics = gpu.cell_metrics();
        let mut layout = Default::default();
        images.layout_into(
            &mut layout,
            shadow.screen(),
            (metrics.width as f32, metrics.height as f32),
            0,
        );
        gpu.apply_palette_override(4, Some((12, 34, 56)));
        gpu.render(
            shadow.screen(),
            &images.take_dirty_images(),
            &Images(&images),
            &layout,
            0,
            0,
        )
        .unwrap();
        let mut pixels = Vec::new();
        // Readback is confined to this validation test, never interactive paint.
        gpu.read_frame(&mut pixels).unwrap();
        assert_eq!(pixels.len(), 64 * 48 * 4);
        let (ox, oy) = gpu.content_origin_px();
        let sample = |col: f32| {
            let x = (ox + col * metrics.width as f32) as usize;
            let y = (oy + metrics.height as f32 * 0.5) as usize;
            &pixels[(y * 64 + x) * 4..][..4]
        };
        assert_eq!(sample(0.5), [240, 20, 40, 255]);
        assert_eq!(sample(3.5), [12, 34, 56, 255]);
    }
}

#[cfg(test)]
#[path = "../tests/support/gpu_benchmark.rs"]
mod benchmark;
