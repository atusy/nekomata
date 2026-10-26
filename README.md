# nekomata

**Tabs and split panes for [felis](https://git.natsukium.com/felis-terminal/felis) sessions, in a native window.**

nekomata (猫又, the two-tailed cat) is a GUI for the felis terminal. felis
keeps your shells alive in a daemon; nekomata arranges them in tabs and split
panes and remembers how you left them.

- **Restart without losing your place.** Quit the GUI or update its UI, start
  it again, and your tabs, splits and running commands are back.
- **Close a pane, keep the shell.** Shells live in felis, so you can attach to
  them again later, from any window.
- **Panes from anywhere.** Point a pane at another felis daemon, locally or
  over SSH, from the command line.
- **One terminal configuration.** Fonts, colors and terminal keys come from
  felis's own `config.toml`, shared with every felis client.

## Try it

With [Nix](https://nixos.org/) and flakes enabled:

```sh
nix run github:atusy/nekomata
```

The felis daemon ships with nekomata and starts when nekomata opens a new
shell.
nekomata is developed on macOS (Apple silicon); the flake also builds for
Linux (x86_64, aarch64), which is less tested. See
[installation](docs/manual/installation.md) for requirements.

## Get around

| Action | Keys |
| --- | --- |
| New tab | Alt+C |
| Next / previous tab | Alt+T / Alt+Shift+T |
| Split down | Alt+W → Alt+S |
| Split right | Alt+W → Alt+V |
| Move between panes | Alt+W → H / J / K / L |
| Close pane (the shell keeps running) | Alt+W → C |
| Command palette | Alt+Shift+P |
| Search scrollback | Ctrl+Shift+F, or Cmd+F on macOS |

Everything else is in the `≡` menu at the left of the tab bar.

## Make it yours

Terminal settings go in felis's `config.toml`. Add a `[client.nekomata]`
overlay for settings that should apply only in nekomata:

```toml
# ~/Library/Application Support/felis/config.toml (macOS)
# ~/.config/felis/config.toml (Linux)
[client.nekomata.font]
size_px = 16
```

Keys for tabs and panes, and the UI font, go in nekomata's own
`config.toml`:

```toml
# ~/Library/Application Support/nekomata/config.toml (macOS)
# ~/.config/nekomata/config.toml (Linux)
[keymap]
"ctrl+a s" = { kind = "split", direction = "down" }
"ctrl+a v" = { kind = "split", direction = "right" }
```

## Learn more

The [user manual](docs/manual/README.md) covers installing the macOS app,
workspaces, reattaching sessions, SSH panes and every configuration option.

For contributors, [docs/spec](docs/spec/README.md) specifies behavior and
[docs/ADR](docs/ADR/) records architecture decisions.

## License

Apache-2.0. nekomata uses [felis](https://git.natsukium.com/felis-terminal/felis)
(Apache-2.0) as its terminal engine.
