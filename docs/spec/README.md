# nekomata specification

These documents specify nekomata's observable behavior: what a user, a script or
another felis client can rely on. They are normative and must match the code;
change them in the same commit as the behavior.

| Document | Covers |
| --- | --- |
| [configuration](configuration.md) | Configuration documents, locations, supported settings and error handling |
| [keymap](keymap.md) | Key dispatch, default bindings and application keymap rules |
| [command palette](command-palette.md) | Opening the palette, finding commands and what labels promise |
| [workspaces](workspaces.md) | Workspace contents, leases, startup, switching and the nekomata daemon |
| [panes](panes.md) | Pane connections, retargeting from the CLI and window resolution |
| [terminal](terminal.md) | Terminal interactions: selection, links, paste, search, font size, scrolling, reconnection |
| [window](window.md) | Window size and position, and overlays |
| [command line](cli.md) | Help, exit statuses and internal commands |
| [accessibility](accessibility.md) | The platform accessibility tree and what it does not expose |

[Architecture decisions](../ADR/) record why components own what they do.
They do not specify behavior. The [user manual](../manual/) explains how to use
nekomata and links here for exact rules.
