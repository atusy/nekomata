//! CPU submission comparison; GPU completion is drained outside each sample.
use super::*;
use felis_client_core::{ShadowScreen, pull::PullScheduler, redraw::RedrawScheduler};
use felis_grid::{
    Cell, Grapheme, StyleTable,
    wire::{RowEncode, encode_row},
};
use felis_protocol::{RowPayload, messages::GridMsg};
use std::{hint::black_box, time::Instant};

fn write_row(screen: &mut ShadowScreen, row: u16, text: &str) {
    let cells: Vec<_> = text
        .chars()
        .map(|c| Cell {
            grapheme: Grapheme::Char(c),
            ..Default::default()
        })
        .collect();
    let row_bytes = encode_row(
        RowEncode {
            cells: &cells,
            pad_to: cells.len(),
            sized_cells: &[],
            soft_wrap_continued: false,
        },
        &StyleTable::new(),
    )
    .unwrap();
    screen
        .apply(&GridMsg::RowDelta {
            rows: vec![(row, RowPayload(row_bytes))],
        })
        .unwrap();
}

fn measure(host: &RenderState, panes: usize, rows: u16, cols: u16, dirty: &str) -> f64 {
    let ctx = egui::Context::default();
    ctx.data_mut(|data| data.insert_temp(egui::Id::unique("terminal-gpu"), host.clone()));
    let line: String =
        "let value = terminal.render_frame(); // 0123456789 abcdefghijklmnopqrstuvwxyz "
            .chars()
            .cycle()
            .take(cols.into())
            .collect();
    let mut alternate = line.clone().into_bytes();
    alternate[0] = b'X';
    let alternate = String::from_utf8(alternate).unwrap();
    let mut states: Vec<_> = (0..panes)
        .map(|_| {
            let mut state = State {
                screen: ShadowScreen::new(rows, cols),
                pull: PullScheduler::new(false),
                redraw: RedrawScheduler::new(),
                colors: Default::default(),
                images: ImageShadow::new(),
                session_id: None,
                error: None,
                connected: true,
                reconnects: 0,
                reconnecting: false,
                detached: false,
                exited: false,
                clipboard: None,
            };
            for row in 0..rows {
                write_row(&mut state.screen, row, &line);
            }
            state.redraw.request();
            state
        })
        .collect();
    let mut renderers: Vec<_> = (0..panes)
        .map(|_| Renderer::new(&ctx, 15.0, 1.0).unwrap())
        .collect();
    let mut samples = Vec::new();
    for frame in 0..110 {
        if frame > 0 {
            let text = if frame % 2 == 0 { &line } else { &alternate };
            for state in &mut states {
                match dirty {
                    "one row" => {
                        write_row(&mut state.screen, rows / 2, text);
                        state.redraw.request();
                    }
                    "all rows" => {
                        for row in 0..rows {
                            write_row(&mut state.screen, row, text);
                        }
                        state.redraw.request();
                    }
                    _ => {}
                }
            }
        }
        let start = Instant::now();
        let output = crate::test_support::run(&ctx, egui::RawInput::default(), |ui| {
            for (i, (renderer, state)) in renderers.iter_mut().zip(&mut states).enumerate() {
                let rect = Rect::from_min_size(
                    Pos2::new((i % 2) as f32 * 2100.0, (i / 2) as f32 * 1500.0),
                    Vec2::new(f32::from(cols) * 10.0, f32::from(rows) * 24.0),
                );
                renderer
                    .paint(
                        &ui.ctx()
                            .layer_painter(egui::LayerId::background())
                            .with_clip_rect(rect),
                        state,
                        rect,
                        false,
                        None,
                    )
                    .unwrap();
            }
        });
        black_box(output.tessellate(&ctx));
        let elapsed = start.elapsed().as_secs_f64() * 1000.0;
        host.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        if frame >= 10 {
            samples.push(elapsed);
        }
    }
    samples.sort_by(f64::total_cmp);
    samples[samples.len() / 2]
}

#[test]
#[ignore = "release GPU submission benchmark; run with --release --ignored --nocapture"]
fn benchmark_gpu_submission() {
    if cfg!(debug_assertions) {
        panic!("use --release");
    }
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let instance = wgpu::Instance::default();
    let adapter = runtime
        .block_on(instance.request_adapter(&Default::default()))
        .unwrap();
    println!("adapter: {:?}", adapter.get_info());
    let (device, queue) = runtime
        .block_on(adapter.request_device(&Default::default()))
        .unwrap();
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let renderer = Arc::new(egui::mutex::RwLock::new(eframe::egui_wgpu::Renderer::new(
        &device,
        format,
        Default::default(),
    )));
    let host = RenderState {
        adapter,
        available_adapters: vec![],
        instance,
        device,
        queue,
        target_format: format,
        renderer,
    };
    println!(
        "panes,grid,damage,legacy_cpu_ms,mesh_cpu_ms,gpu_submit_cpu_ms,mesh_to_gpu,legacy_to_gpu"
    );
    for (panes, rows, cols) in [(1, 24, 80), (1, 60, 200), (4, 40, 120)] {
        for dirty in ["none", "one row", "all rows"] {
            let (legacy, _) =
                crate::mesh_baseline::benchmark::measure(false, panes, rows, cols, dirty);
            let (mesh, _) =
                crate::mesh_baseline::benchmark::measure(true, panes, rows, cols, dirty);
            let gpu = measure(&host, panes, rows, cols, dirty);
            println!(
                "{panes},{cols}x{rows},{dirty},{legacy:.4},{mesh:.4},{gpu:.4},{:.2},{:.2}",
                mesh / gpu,
                legacy / gpu
            );
        }
    }
}
