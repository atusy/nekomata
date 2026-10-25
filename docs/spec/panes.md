# Panes

Each pane presents one felis session from one felis daemon: the local daemon
at startup, another local daemon by socket path, or a daemon reached over SSH.

## Pane IDs

A pane ID identifies a pane within its workspace and survives GUI restarts. It
is distinct from the felis session ID.

## New shells

- A new tab uses the local daemon nekomata started with.
- A split uses the connection target of the pane it splits.
- A split starts in the current local directory the shell reported through
  OSC 7, else in nekomata's startup directory.
- A new shell starts in the home directory when nekomata is launched as an app,
  and in the invoking directory when launched from a CLI.
- On macOS the shell is started as `$SHELL -l`, a login shell.
- New local shells receive `NEKOMATA_WORKSPACE_ID`, `NEKOMATA_WINDOW_ID` and
  `NEKOMATA_PANE_ID`. nekomata never rewrites the environment of an existing
  shell.
- A new remote shell does not inherit the local shell path, working directory
  or environment.

`FELIS_SOCKET` selects a local felis daemon other than the default.

## Listing felis sessions

```sh
nekomata felis sessions list [--format human|json]
```

The listing is delegated to the bundled felis CLI. The `Attach…` menu item
opens a session of the startup local daemon in a new tab.

## Retargeting a pane

```sh
nekomata panes list [--window WINDOW_ID]
nekomata panes attach [PANE_ID] [--window WINDOW_ID] [--ssh HOST] [--daemon SOCKET] --session SESSION_ID
```

- Omitting `PANE_ID` selects the active pane when the GUI receives the
  request.
- Omitting both `--daemon` and `--ssh` keeps the pane's current target,
  including its SSH host and socket.
- `--daemon` alone names a local socket. A relative path resolves against the
  CLI's working directory.
- `--ssh` alone uses the remote host's default socket. With `--daemon`, the
  path names a remote socket and a relative path resolves against the remote
  relay's working directory. Paths are passed without remote shell expansion.
- `SESSION_ID` may be any prefix felis resolves uniquely.
- The target daemon must be running and the session must exist.
- The pane switches only after connecting and attaching succeed. The previous
  session is detached and keeps running. On failure the pane keeps its
  session and the CLI exits with an error.
- A second attach to a pane while one is in progress fails.
- There is no CLI command that only detaches.

## Window resolution

The window a CLI command targets is the first of:

1. `--window`;
2. the window currently attached to the workspace in `NEKOMATA_WORKSPACE_ID`
   (or, when unset, the legacy `NEKOMATA_SESSION_ID`);
3. `NEKOMATA_WINDOW_ID`;
4. the only running GUI window.

When several candidates remain, the command fails and reports the window
discovery directory. Because resolution goes through the workspace ID, a shell
keeps reaching its window after the GUI restarts.

## SSH

- SSH commands run on the machine running the GUI. Hosts may be
  `~/.ssh/config` names or `user@host`.
- The remote host needs a compatible `felis-daemon` on `PATH`.
- Authentication must be non-interactive (public keys, SSH agent). Passwords
  and first-time host-key confirmation are not prompted for.
