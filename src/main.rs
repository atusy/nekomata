mod app;
mod cli;
mod layout;
mod picker;
mod session;
mod shortcuts;
mod theme;

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = cli::parse(&args).unwrap_or_else(|err| {
        eprintln!("{err}\n\n{}", cli::HELP);
        std::process::exit(2);
    });
    match command {
        cli::Command::Gui => {}
        cli::Command::Help => {
            println!("{}", cli::HELP);
            return Ok(());
        }
        cli::Command::List(options) => {
            run_felis(&["sessions", "list"], &options);
        }
    }
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
        Box::new(move |cc| Ok(Box::new(app::App::new(cc)))),
    )
}

fn run_felis(command: &[&str], args: &[String]) -> ! {
    match std::process::Command::new("felis")
        .args(command)
        .args(args)
        .status()
    {
        Ok(status) => std::process::exit(status.code().unwrap_or(1)),
        Err(err) => {
            eprintln!("Could not run felis: {err}. Use nix run or nix develop.");
            std::process::exit(2);
        }
    }
}
