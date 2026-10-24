mod app;
mod cli;
mod colors;
mod command_output;
mod control;
mod daemon;
mod editor_output;
mod layout;
mod links;
#[cfg(test)]
#[path = "../tests/support/mesh_renderer.rs"]
mod mesh_baseline;
mod picker;
mod presentation;
mod render;
mod session;
mod shortcuts;
mod ssh;
mod terminal_search;
#[cfg(test)]
mod test_support;
mod theme;
mod workspace;
mod workspace_client;

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = cli::parse(&args).unwrap_or_else(|err| {
        eprintln!("{err}\n\n{}", cli::HELP);
        std::process::exit(2);
    });
    match command {
        cli::Command::EditOutput { file, argv } => {
            let code =
                editor_output::run(std::path::Path::new(&file), &argv).unwrap_or_else(|error| {
                    eprintln!("{error:#}");
                    1
                });
            std::process::exit(code);
        }
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
        cli::Command::Session { window, session } => {
            match control::run_request(window, control::Request::Switch { session }) {
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
    eframe::run_native(
        APP_ID,
        native_options(),
        Box::new(move |cc| Ok(Box::new(app::App::new(cc)?))),
    )
}

/// Name passed to eframe; it also keys eframe's on-disk storage.
const APP_ID: &str = "nekomata";

fn native_options() -> eframe::NativeOptions {
    eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("nekomata")
            .with_inner_size([1100.0, 740.0])
            .with_min_inner_size([480.0, 320.0]),
        ..Default::default()
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_geometry_persists_under_the_nekomata_app_id() {
        let options = native_options();

        assert!(options.persist_window);
        assert!(options.persistence_path.is_none());
        assert!(options.viewport.app_id.is_none());
        let dir = eframe::storage_dir(APP_ID).expect("eframe persistence storage directory");
        assert_eq!(dir.file_name(), Some(std::ffi::OsStr::new("nekomata")));
    }

    #[test]
    fn first_launch_uses_the_default_window_size() {
        let viewport = native_options().viewport;

        assert_eq!(viewport.inner_size, Some([1100.0, 740.0].into()));
        assert_eq!(viewport.min_inner_size, Some([480.0, 320.0].into()));
    }
}
