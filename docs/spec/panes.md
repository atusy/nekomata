# Panes

Each pane presents one felis session from one felis daemon: the local daemon
at startup, another local daemon by socket path, or a daemon reached over SSH.

## Pane IDs

A pane ID identifies a pane within its workspace and survives GUI restarts. It
is distinct from the felis session ID.

## New shells

- A new tab uses the local daemon nekomata started with.
- A split uses the connection target of the pane it splits.
- A local split starts in the directory its source shell last reported through
  OSC 7. Other new local shells, and splits without a report, start in
  nekomata's working directory, or in the home directory when that is `/`.
  The macOS app launcher changes to the home directory before starting
  nekomata.
- A new local shell runs `$SHELL` (else `/bin/sh`); on macOS it runs as a login
  shell (`-l`).
- New local shells receive `NEKOMATA_WORKSPACE_ID` (once the window's
  workspace is known), `NEKOMATA_WINDOW_ID`, `NEKOMATA_PANE_ID` and
  `NEKOMATA_SHELL_INTEGRATION=1`. nekomata never rewrites the environment of an
  existing shell.
- A new remote shell runs the remote daemon's default command. It receives
  none of the above: no `$SHELL`, working directory or environment is
  forwarded.
- Only a new local shell starts its felis daemon on demand. Attaching to a
  session never starts a daemon, and nekomata never starts a remote daemon.

`FELIS_SOCKET` selects a local felis daemon other than the default.

## Listing felis sessions

```sh
nekomata felis sessions list [--format human|json]
```

The listing is delegated to the bundled felis CLI. Attaching a session from the command palette opens a session of the startup local daemon in a new tab.

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
- A second attach to a pane while one is in progress fails, as does any
  attach while the window is switching workspaces.
- There is no CLI command that only detaches.

## Window resolution

A CLI command targets a window as follows:

1. `--window`, when given.
2. Otherwise, if `NEKOMATA_WORKSPACE_ID` holds a workspace ID (else, if the
   legacy `NEKOMATA_SESSION_ID` does), the window that workspace is attached
   to. If that workspace has no window or the nekomata daemon is unreachable,
   the command fails; it does not fall back further.
3. Otherwise `NEKOMATA_WINDOW_ID`, when set.
4. Otherwise the only running GUI window. With no window or several, the
   command fails; with several, it reports the window discovery directory.

Because resolution goes through the workspace ID, a shell keeps reaching its
window after the GUI restarts. A shell carries the IDs it was started with,
so a shell moved to another workspace by hand still targets its original
workspace's window; pass `--window` there.

## SSH

- SSH commands run on the machine running the GUI. Hosts may be
  `~/.ssh/config` names or `user@host`.
- The remote host needs a compatible `felis-daemon` on `PATH`, already
  running: nekomata relays to it and never starts it, even for new shells.
- Authentication must be non-interactive (public keys, SSH agent). Passwords
  and first-time host-key confirmation are not prompted for.
