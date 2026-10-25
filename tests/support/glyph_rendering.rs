//! Verify that the egui compositor preserves the upstream terminal pixels.
use super::*;
use felis_client_core::ShadowScreen;
use felis_grid::{
    Attributes, Cell, Color, Grapheme, StyleTable,
    wire::{RowEncode, encode_row},
};
use felis_protocol::{RowPayload, messages::GridMsg};

const COLS: u32 = 40;
const LINES: [&str; 6] = [
    "bold_font auto",
    "bold_italic_font auto",
    "font_family family=\"UDEV Gothic NF\"",
    "font_size 15.0",
    "abcdef ABCDEF 0123456789 @#%&()",
    "iiii llll mmmm WWWW / \\ eeee",
];

fn screen(fg: [u8; 3], bg: [u8; 3]) -> ShadowScreen {
    let mut shadow = ShadowScreen::new(LINES.len() as u16, COLS as u16);
    let mut styles = StyleTable::new();
    let style = styles.intern(Attributes {
        fg: Color::Rgb(fg[0], fg[1], fg[2]),
        bg: Color::Rgb(bg[0], bg[1], bg[2]),
        ..Default::default()
    });
    for (row, line) in LINES.iter().enumerate() {
        let cells: Vec<_> = line
            .chars()
            .map(|c| Cell {
                grapheme: Grapheme::Char(c),
                style,
                ..Default::default()
            })
            .collect();
        let bytes = encode_row(
            RowEncode {
                cells: &cells,
                pad_to: COLS as usize,
                sized_cells: &[],
                soft_wrap_continued: false,
            },
            &styles,
        )
        .unwrap();
        shadow
            .apply(&GridMsg::RowDelta {
                rows: vec![(row as u16, RowPayload(bytes))],
            })
            .unwrap();
    }
    shadow
}

fn capture(name: &str, width: u32, height: u32, pixels: &[u8]) {
    let Some(directory) = std::env::var_os("NEKOMATA_GLYPH_CAPTURE") else {
        return;
    };
    let mut ppm = format!("P6\n{width} {height}\n255\n").into_bytes();
    for pixel in pixels.as_chunks::<4>().0 {
        ppm.extend_from_slice(&pixel[..3]);
    }
    std::fs::write(
        std::path::Path::new(&directory).join(format!("{name}.ppm")),
        ppm,
    )
    .unwrap();
}

#[test]
#[ignore = "requires a GPU adapter and NEKOMATA_FONT; run in nix develop with --ignored"]
fn gpu_compositor_preserves_terminal_pixels() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let instance = wgpu::Instance::default();
    let adapter = runtime
        .block_on(instance.request_adapter(&Default::default()))
        .unwrap();
    println!("adapter: {:?}", adapter.get_info());
    let (device, queue) = runtime
        .block_on(adapter.request_device(&Default::default()))
        .unwrap();
    let stack = FontStack::discover_in_files(
        &[std::env::var_os("NEKOMATA_FONT")
            .expect("run in nix develop")
            .into()],
        None,
        &[],
        &[],
        &Default::default(),
    )
    .unwrap();
    for (name, fg, bg) in [
        ("dark", [192, 160, 223], [36, 34, 53]),
        ("light", [36, 34, 53], [230, 230, 230]),
    ] {
        // 18 logical px at 1x and 2x, plus a smaller low-DPI font.
        for size in [15, 18, 36] {
            let cfg = RendererConfig {
                font_size_physical_px: Some(size as f32),
                theme_fg: Some(format!("#{:02x}{:02x}{:02x}", fg[0], fg[1], fg[2])),
                theme_bg: Some(format!("#{:02x}{:02x}{:02x}", bg[0], bg[1], bg[2])),
                ..Default::default()
            };
            let mut gpu =
                GpuRenderer::new_embedded(device.clone(), queue.clone(), 1, 1, cfg, stack.clone())
                    .unwrap();
            gpu.set_window_focused(false);
            let shadow = screen(fg, bg);
            let metrics = gpu.cell_metrics();
            let (width, height) = (metrics.width * COLS, metrics.height * LINES.len() as u32);
            // Also exercises recreation of the compatible attachment after resize.
            gpu.resize(width, height);
            gpu.render(
                shadow.screen(),
                &[],
                &Images(&ImageShadow::new()),
                &Default::default(),
                0,
                0,
            )
            .unwrap();
            let mut pixels = Vec::new();
            gpu.read_frame(&mut pixels).unwrap();
            capture(&format!("glyph-{name}-{size}"), width, height, &pixels);
            for ppp in [1.0, 1.25, 1.5, 2.0] {
                assert_presented_pixels(
                    &device,
                    &queue,
                    &gpu.embedded_texture_view().unwrap(),
                    [width, height],
                    &pixels,
                    ppp,
                    &format!("glyph-{name}-{size}"),
                );
            }
            assert!(gpu.capture_last_frame().unwrap().painted_pixels > 0);
        }
    }
}

// Exercise the final egui draw, not only the terminal's offscreen attachment.
fn assert_presented_pixels(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    source: &wgpu::TextureView,
    size: [u32; 2],
    expected: &[u8],
    ppp: f32,
    name: &str,
) {
    use eframe::egui_wgpu::{Renderer as EguiRenderer, RendererOptions, ScreenDescriptor};
    let [width, height] = size;
    let canvas = [width + 32, height + 32];
    let ctx = egui::Context::default();
    ctx.set_pixels_per_point(ppp);
    let mut renderer = EguiRenderer::new(
        device,
        wgpu::TextureFormat::Rgba8Unorm,
        RendererOptions {
            dithering: false,
            ..Default::default()
        },
    );
    let texture = renderer.register_native_texture(device, source, wgpu::FilterMode::Nearest);
    // One unused physical pixel produces a half-pixel top margin. Fractional
    // pane X positions also occur when the window is divided into columns.
    let rect = terminal_rect(
        Rect::from_min_size(
            Pos2::new(7.5, 9.0) / ppp,
            Vec2::new(width as f32, height as f32 + 1.0) / ppp,
        ),
        LINES.len() as u16,
        height as f32 / LINES.len() as f32 / ppp,
        ppp,
    );
    let output = crate::test_support::run(
        &ctx,
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(
                Pos2::ZERO,
                Vec2::new(canvas[0] as f32, canvas[1] as f32) / ppp,
            )),
            ..Default::default()
        },
        |ui| {
            ui.ctx()
                .layer_painter(egui::LayerId::background())
                .with_clip_rect(rect)
                .image(
                    texture,
                    rect,
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );
        },
    );
    let primitives = output.tessellate(&ctx);
    let descriptor = ScreenDescriptor {
        size_in_pixels: canvas,
        pixels_per_point: ppp,
    };
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("egui glyph presentation test"),
        size: wgpu::Extent3d {
            width: canvas[0],
            height: canvas[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    let pitch = (canvas[0] * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("egui glyph presentation readback"),
        size: u64::from(pitch * canvas[1]),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    let callbacks = renderer.update_buffers(device, queue, &mut encoder, &primitives, &descriptor);
    assert!(callbacks.is_empty());
    {
        let mut pass = encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("egui glyph presentation"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            })
            .forget_lifetime();
        renderer.render(&mut pass, &primitives, &descriptor);
    }
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &target,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(pitch),
                rows_per_image: Some(canvas[1]),
            },
        },
        wgpu::Extent3d {
            width: canvas[0],
            height: canvas[1],
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, |result| result.unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let mapped = buffer.slice(..).get_mapped_range().unwrap();
    let x = (rect.min.x * ppp).round() as usize;
    let y = (rect.min.y * ppp).round() as usize;
    let actual: Vec<_> = (0..height as usize)
        .flat_map(|row| {
            let offset = (y + row) * pitch as usize + x * 4;
            mapped[offset..offset + width as usize * 4].iter().copied()
        })
        .collect();
    capture(&format!("{name}-presented-{ppp}"), width, height, &actual);
    let errors = actual
        .iter()
        .zip(expected)
        .filter(|(a, b)| a.abs_diff(**b) > 1)
        .count();
    assert_eq!(
        errors, 0,
        "egui must preserve every source pixel at {ppp}x ({rect:?})"
    );
}
