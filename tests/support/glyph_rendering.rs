//! Compare real GPU output with the rasterizer's coverage, independent of the shader.
use super::*;
use felis_client_core::ShadowScreen;
use felis_grid::{
    Attributes, Cell, Color, Grapheme, StyleTable,
    wire::{RowEncode, encode_row},
};
use felis_protocol::{
    ImageId, RowPayload,
    messages::{GridMsg, ImageFormat, ImageMsg, ImageTarget, RequestedExtent},
};
use felis_shaping::ShapeCache;

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

fn assert_coverage(pixels: &[u8], stack: &FontStack, size: u32, fg: [u8; 3], bg: [u8; 3]) {
    let metrics = stack.primary().cell_metrics(size as f32);
    let width = metrics.width * COLS;
    let mut cache = ShapeCache::default();
    let mut edge_pixels = 0;
    let mut worst = 0;
    // These leading glyphs fit naturally and have no neighbour ink. Other
    // fixture glyphs may intentionally be fitted/shaped by render-core.
    for (row, line) in LINES.iter().enumerate() {
        let bitmap = cache.get_or_insert(stack.primary(), line.chars().next().unwrap(), size);
        for y in 0..metrics.height {
            for x in 0..metrics.width {
                let bx = x as i32 - bitmap.left;
                let by = y as i32 - metrics.ascent as i32 + bitmap.top;
                let coverage = if bx >= 0
                    && by >= 0
                    && bx < bitmap.width() as i32
                    && by < bitmap.height() as i32
                {
                    bitmap.pixels().as_bytes()[(by as u32 * bitmap.width() + bx as u32) as usize]
                } else {
                    0
                };
                edge_pixels += usize::from(coverage > 0 && coverage < 255);
                let a = f32::from(coverage) / 255.0;
                let offset = ((row as u32 * metrics.height + y) * width + x) as usize * 4;
                for c in 0..3 {
                    let expected =
                        (f32::from(fg[c]) * a + f32::from(bg[c]) * (1.0 - a)).round() as u8;
                    worst = worst.max(pixels[offset + c].abs_diff(expected));
                }
                assert_eq!(pixels[offset + 3], 255);
            }
        }
    }
    assert!(edge_pixels > 20, "fixture must exercise partial coverage");
    assert!(
        worst <= 2,
        "{size}px fg={fg:?} bg={bg:?}: sRGB coverage error {worst}"
    );
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
fn gpu_glyph_coverage_matches_srgb_composition() {
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
            assert_coverage(&pixels, &stack, size, fg, bg);
            assert!(gpu.capture_last_frame().unwrap().painted_pixels > 0);

            // The text pass must blend against the actual destination, not
            // an assumed cell background, and keep the image z-order intact.
            let image_color = [64, 96, 128];
            let mut images = ImageShadow::new();
            let id = ImageId(1);
            for msg in [
                ImageMsg::Header {
                    id,
                    target: ImageTarget::New {
                        width,
                        height,
                        format: ImageFormat::Rgb24,
                        serial: None,
                    },
                },
                ImageMsg::Chunk {
                    id,
                    bytes: image_color.repeat((width * height) as usize).into(),
                },
                ImageMsg::Complete { id },
            ] {
                images.apply(&msg).unwrap();
            }
            for z_index in [-1, 1] {
                images
                    .apply(&ImageMsg::Placement {
                        image_id: id,
                        placement_id: Some(felis_protocol::PlacementId(1)),
                        anchor_row: 1,
                        anchor_col: 1,
                        cols: COLS as u16,
                        rows: LINES.len() as u16,
                        source: None,
                        z_index,
                        requested: Some(RequestedExtent {
                            cols: COLS as u16,
                            rows: LINES.len() as u16,
                        }),
                        serial: None,
                    })
                    .unwrap();
                let mut layout = Default::default();
                images.layout_into(
                    &mut layout,
                    shadow.screen(),
                    (metrics.width as f32, metrics.height as f32),
                    0,
                );
                gpu.render(
                    shadow.screen(),
                    &images.take_dirty_images(),
                    &Images(&images),
                    &layout,
                    0,
                    0,
                )
                .unwrap();
                gpu.read_frame(&mut pixels).unwrap();
                if z_index < 0 {
                    assert_coverage(&pixels, &stack, size, fg, image_color);
                } else {
                    assert!(
                        pixels
                            .as_chunks::<4>()
                            .0
                            .iter()
                            .all(|p| *p == [64, 96, 128, 255])
                    );
                }
            }
        }
    }
}
