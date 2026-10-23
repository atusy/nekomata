//! Same-process CPU comparison against the frozen 1f7eb18 painting path.
//! Includes egui frame processing and tessellation; excludes GPU submission.
use super::*;
use std::{hint::black_box, time::Instant};

#[path = "legacy_renderer.rs"]
mod legacy;

fn configure_legacy_font(ctx: &egui::Context) {
    if let Some(path) = std::env::var_os("NEKOMATA_FONT") {
        let mut fonts = egui::FontDefinitions::default();
        fonts.font_data.insert(
            "terminal".into(),
            egui::FontData::from_owned(std::fs::read(path).unwrap()).into(),
        );
        fonts
            .families
            .get_mut(&egui::FontFamily::Monospace)
            .unwrap()
            .insert(0, "terminal".into());
        ctx.set_fonts(fonts);
    }
}

fn measure(shared: bool, panes: usize, rows: u16, cols: u16, dirty: &str) -> (f64, usize) {
    let ctx = egui::Context::default();
    configure_legacy_font(&ctx);
    let mut screens: Vec<_> = (0..panes).map(|_| ShadowScreen::new(rows, cols)).collect();
    let mut renderers: Vec<_> = (0..panes)
        .map(|_| Renderer::new(&ctx, 15.0, 1.0).unwrap())
        .collect();
    let text = "let value = terminal.render_frame(); // 0123456789 abcdefghijklmnopqrstuvwxyz ";
    let line: String = text.chars().cycle().take(usize::from(cols)).collect();
    let mut alternate = line.clone().into_bytes();
    alternate[0] = b'X';
    let alternate = String::from_utf8(alternate).unwrap();
    for screen in &mut screens {
        for row in 0..rows {
            super::tests::write_row(screen, row, &line);
        }
    }
    let mut samples = Vec::new();
    let mut primitive_count = 0;
    // Warm caches before sampling. Each backend receives the same row deltas.
    for frame in 0..110 {
        if frame > 0 {
            let text = if frame % 2 == 0 { &line } else { &alternate };
            for screen in &mut screens {
                match dirty {
                    "one row" => super::tests::write_row(screen, rows / 2, text),
                    "all rows" => {
                        for row in 0..rows {
                            super::tests::write_row(screen, row, text);
                        }
                    }
                    _ => {}
                }
            }
        }
        let start = Instant::now();
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            for (i, (renderer, screen)) in renderers.iter_mut().zip(&screens).enumerate() {
                // The previous frontend used a 21px row at size 15. Keep every
                // cell visible for both font metric systems on the same grid.
                let rect = Rect::from_min_size(
                    Pos2::new((i % 2) as f32 * 2100.0, (i / 2) as f32 * 1500.0),
                    Vec2::new(f32::from(cols) * 10.0, f32::from(rows) * 24.0),
                );
                let painter = ctx
                    .layer_painter(egui::LayerId::background())
                    .with_clip_rect(rect);
                if shared {
                    renderer.paint(&painter, screen, rect, false, None);
                } else {
                    legacy::paint(&painter, screen, rect, 15.0);
                }
            }
        });
        let primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
        primitive_count = primitives.len();
        black_box(primitives);
        if frame >= 10 {
            samples.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        for screen in &mut screens {
            screen.clear_damage();
        }
    }
    samples.sort_by(f64::total_cmp);
    (samples[samples.len() / 2], primitive_count)
}

#[test]
#[ignore = "release CPU rendering benchmark; run with --release --ignored --nocapture"]
fn benchmark_terminal_rendering() {
    if cfg!(debug_assertions) {
        panic!("use cargo test --release");
    }
    println!("panes,grid,damage,legacy_ms,shared_ms,speedup,legacy_primitives,shared_primitives");
    for (panes, rows, cols) in [(1, 24, 80), (1, 60, 200), (4, 40, 120)] {
        for dirty in ["none", "one row", "all rows"] {
            let (old, old_primitives) = measure(false, panes, rows, cols, dirty);
            let (new, new_primitives) = measure(true, panes, rows, cols, dirty);
            println!(
                "{panes},{cols}x{rows},{dirty},{old:.4},{new:.4},{:.2},{old_primitives},{new_primitives}",
                old / new
            );
        }
    }
}
