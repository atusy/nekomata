mod app;
mod layout;
mod resize;
mod session;

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let attach = match args.as_slice() {
        [] => None,
        [flag, id] if flag == "--attach" => Some(id.clone()),
        [flag] if flag == "--help" || flag == "-h" => {
            println!(
                "nekomata [--attach SESSION_ID]\n\nA tabbed, split-pane GUI for felis.\nClose panes to detach and keep their shells running."
            );
            return Ok(());
        }
        _ => {
            eprintln!("Usage: nekomata [--attach SESSION_ID]");
            std::process::exit(2);
        }
    };
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("nekomata")
            .with_inner_size([1100.0, 740.0])
            .with_min_inner_size([480.0, 320.0]),
        ..Default::default()
    };
    eframe::run_native(
        "nekomata",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, attach)))),
    )
}
