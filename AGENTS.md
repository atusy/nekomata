# Repository Guidelines

## Project Structure & Module Organization

nekomata is a Rust 2024 native terminal frontend built with eframe/egui. The felis daemon owns shells, PTYs, terminal parsing, and persistent sessions.

- `src/app.rs`: window UI and terminal rendering.
- `src/session.rs`: asynchronous felis connections, input, and terminal state.
- `src/daemon.rs`, `src/workspace.rs`, `src/workspace_client.rs`: retained nekomata sessions, versioned leases, and asynchronous layout synchronization.
- `src/control.rs`: per-window CLI commands and session-aware window discovery.
- `src/layout.rs`, `src/shortcuts.rs`: pane layout, navigation, and keyboard actions.
- `src/cli.rs`, `src/picker.rs`, `src/theme.rs`: session commands, selection UI, and shared colors.
- `src/config.rs`: the shared felis `config.toml` resolved with the `[client.nekomata]` overlay.
- `share/`: fish integration and macOS bundle metadata/native launcher.
- `scripts/`: isolated integration tests and macOS installation.
- `flake.nix`: packages, development environments, runtime fonts, and checks.

## Build, Test, and Development Commands

Use Nix for reproducible development. Run Cargo commands inside `nix develop`:

```sh
cargo run                                  # Start the development GUI
cargo test --locked                        # Run ordinary Rust tests
cargo fmt --check                          # Check Rust formatting
cargo clippy --locked --all-targets -- -D warnings
bash scripts/test-live.sh                   # Run isolated daemon integration tests
```

Use `nix run .` to launch the packaged app, `nix build` to build it, and `nix flake check` for package and fish integration checks. Format Nix with `nix fmt -- flake.nix`.

On macOS, `bash scripts/install-macos.sh` installs or updates `~/Applications/nekomata.app`; restart the GUI to load changes.

## Coding Style & Naming Conventions

Follow rustfmt defaults: four-space indentation, `snake_case` functions/modules, and `PascalCase` types. Keep UI operations on the GUI thread and IPC work asynchronous. Share rendering constants through `src/theme.rs`. Keep Cargo and Nix felis revisions synchronized when updating upstream dependencies.

## Testing Guidelines

Rust tests live in inline `#[cfg(test)]` modules. Use descriptive behavior-oriented names. Add regression tests for input, focus, layout, or protocol changes. Ignored `live_*` tests require `scripts/test-live.sh`, which provisions a private socket and cleans up its daemon. The runner isolates both nekomata and felis daemons. Never use the user's normal daemon for destructive tests. Report native GUI checks separately from automated results; no numerical coverage target is configured.

## Commit & Pull Request Guidelines

Always commit incrementally as work progresses. Create a small, cohesive commit for each completed logical change; do not accumulate unrelated changes into a single final commit. Preserve pre-existing worktree changes unless explicitly asked to include them.

Make small, behavior-focused commits using Conventional Commits, such as `fix: honor the terminal cursor shape` or `docs: clarify upstream location`. Explain motivation in the commit body. Review staged diffs and run relevant checks before committing.

PRs should describe the problem, resulting behavior, and validation. Link related issues when applicable; include screenshots for visual changes and disclose unverified platform behavior. Closing a pane detaches its session; preserve that contract unless explicitly changing it.
