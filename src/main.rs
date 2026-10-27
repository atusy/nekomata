mod app;
mod app_config;
mod cli;
mod colors;
mod command_palette;
mod commands;
mod config_report;
mod control;
mod daemon;
mod font_size;
mod input;
mod layout;
mod links;
#[cfg(test)]
#[path = "../tests/support/mesh_renderer.rs"]
mod mesh_baseline;
mod paste;
mod picker;
mod presentation;
mod rectangle;
mod render;
mod session;
mod shortcuts;
mod ssh;
mod terminal_config;
mod terminal_search;
#[cfg(test)]
mod test_support;
mod theme;
mod ui_font;
mod workspace;
mod workspace_client;

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if cli::is_deprecated(&args) {
        eprintln!("nekomata: `sessions` is deprecated; use `workspaces`");
    }
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
        cli::Command::DaemonStop | cli::Command::WorkspacesList => {
            match daemon::command(matches!(command, cli::Command::DaemonStop)) {
                Ok(output) => println!("{output}"),
                Err(error) => {
                    eprintln!("{error:#}");
                    std::process::exit(1);
                }
            }
            return Ok(());
        }
        cli::Command::Workspace { window, workspace } => {
            match control::run_request(window, control::Request::Switch { workspace }) {
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
    // anyhow's chain survives only as text: eframe boxes the error and its
    // Display shows only the outermost context.
    let result = eframe::run_native(
        APP_ID,
        native_options(),
        Box::new(move |cc| Ok(Box::new(app::App::new(cc).map_err(|e| format!("{e:#}"))?))),
    );
    // Returning the error from main would print its Debug form.
    if let Err(error) = result {
        eprintln!("nekomata: {error}");
        std::process::exit(1);
    }
    Ok(())
}

/// Name passed to eframe; it also keys eframe's on-disk storage.
const APP_ID: &str = "nekomata";

fn native_options() -> eframe::NativeOptions {
    eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("nekomata")
            .with_icon(
                eframe::icon_data::from_png_bytes(include_bytes!("../share/icons/nekomata.png"))
                    .expect("bundled application icon is valid PNG"),
            )
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
    fn first_launch_window_is_not_clamped_by_the_minimum_size() {
        let viewport = native_options().viewport;
        let size = viewport.inner_size.expect("first-launch inner size");
        let min = viewport.min_inner_size.expect("minimum inner size");

        assert!(size.x >= min.x && size.y >= min.y, "{size:?} < {min:?}");
    }
}
