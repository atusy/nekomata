mod app;
mod cli;
mod control;
mod daemon;
mod glyph;
mod layout;
mod picker;
mod session;
mod shortcuts;
mod theme;
mod workspace;

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = cli::parse(&args).unwrap_or_else(|err| {
        eprintln!("{err}\n\n{}", cli::HELP);
        std::process::exit(2);
    });
    match command {
        cli::Command::Gui => {}
        cli::Command::DaemonRun => {
            if let Err(error) = daemon::run() {
                eprintln!("{error:#}");
                std::process::exit(1);
            }
            return Ok(());
        }
        cli::Command::DaemonStop | cli::Command::SessionsList => {
            match daemon::command(matches!(command, cli::Command::DaemonStop)) {
                Ok(output) => println!("{output}"),
                Err(error) => {
                    eprintln!("{error:#}");
                    std::process::exit(1);
                }
            }
            return Ok(());
        }
        cli::Command::Pane { window, action } => {
            match control::run(window, action) {
                Ok(output) => println!("{output}"),
                Err(err) => {
                    eprintln!("{err:#}");
                    std::process::exit(1);
                }
            }
            return Ok(());
        }
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
        Box::new(move |cc| Ok(Box::new(app::App::new(cc)?))),
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
