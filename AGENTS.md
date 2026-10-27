# Repository Guidelines

## Project Structure & Module Organization

nekomata is a Rust 2024 native terminal frontend built with eframe/egui. The felis daemon owns shells, PTYs, terminal parsing, and persistent sessions.

- `src/main.rs`, `src/cli.rs`: entry point, window options, and command-line parsing.
- `src/app.rs`: window UI — tabs, panes, overlays, and input dispatch.
- `src/render.rs`, `src/presentation.rs`: composing felis's GPU renderer into egui, and native input and repaint deadlines adapted to felis presentation state.
- `src/session.rs`, `src/ssh.rs`, `src/colors.rs`: asynchronous felis connections, ordered input and terminal state; the SSH relay; per-session color state.
- `src/terminal_search.rs`, `src/links.rs`, `src/paste.rs`, `src/rectangle.rs`: scrollback search, OSC 8 link opening, paste confirmation, and rectangular selection.
- `src/daemon.rs`, `src/workspace.rs`, `src/workspace_client.rs`: retained workspaces, versioned leases, and asynchronous layout synchronization.
- `src/control.rs`: per-window CLI commands and workspace-aware window discovery.
- `src/layout.rs`, `src/shortcuts.rs`: pane layout, navigation, and keyboard actions.
- `src/commands.rs`, `src/command_palette.rs`, `src/font_size.rs`, `src/picker.rs`: the public command registry, palette, the font size control, and the felis session picker.
- `src/app_config.rs`, `src/ui_font.rs`: nekomata's own `config.toml` (UI font and application keymap) and UI font lookup; `src/terminal_config.rs` resolves felis's shared document.
- `src/theme.rs`: shared UI colors, metrics, widget styles, and chrome fonts.
- `src/test_support.rs`, `tests/support/`: CPU-only egui test helpers, and test-only renderer baselines, glyph checks and benchmarks included through `#[path]`.
- `share/`: fish integration and macOS bundle metadata/native launcher.
- `scripts/`: isolated integration tests and macOS installation.
- `flake.nix`: packages, development environments, runtime fonts, and checks.
- `docs/`: documentation, described below.

## Documentation

Write all documentation in English. Each kind of document has one home:

- `README.md`: a short introduction for users; link to the manual for details.
- `docs/manual/`: how to use nekomata, organized by user task. Summarize behavior and link to the spec for exact rules.
- `docs/spec/`: normative observable behavior. Update it in the same commit as the behavior it describes.
- `docs/ADR/`: architecture decisions only — ownership, boundaries, and their invariants — following `docs/ADR/template.md`. An ADR is a decision, not a description, and may diverge from the code: neither rewrite the decision to match nor record the divergence in the ADR; analyze a gap when it matters or file it as a GitHub issue. Keep no status tables, validation logs, or change history; `git log` holds them. Delete a superseded ADR rather than renaming it with a `_` prefix.
- Other `docs/*.md` files are rendering and validation reports; they describe the code at the time they were written, not its current state: [rendering performance](docs/rendering-performance.md), [felis rendering reuse audit](docs/felis-rendering-reuse-audit.md), [Vitro validation](docs/vitro-rendering-validation.md), [native macOS validation](docs/native-rendering-validation.md) and [glyph placement](docs/glyph-placement-validation.md).

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
