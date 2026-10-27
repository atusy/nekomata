# Sessions and pane targets

Each pane shows one felis session, a single shell running in a felis daemon.
Closing the pane leaves the session running, so you can attach to it later.

## Reattaching a session

`Session: Attach…` in the command palette lists the sessions of the local
daemon and opens the one you pick in a new tab. To list them from a shell:

```sh
nekomata felis sessions list
nekomata felis sessions list --format json
```

Without installing nekomata, use
`nix run github:atusy/nekomata -- felis sessions list`. To move a session into
an existing pane instead, see [Retargeting a pane](#retargeting-a-pane).

nekomata uses the default local felis daemon. Start nekomata with
`FELIS_SOCKET` set to use another one.

## Retargeting a pane

You can switch a pane of a running GUI to another session from the command
line.

```sh
# Show the window's ID and its pane IDs, targets and session IDs
nekomata panes list

# Switch the focused pane to another session on the same daemon
nekomata panes attach --session SESSION_ID

# Switch pane 2 to a session on another local daemon
nekomata panes attach 2 --daemon /path/to/felis.sock --session SESSION_ID

# Switch to a session on an SSH host
nekomata panes attach --ssh dev-server --session SESSION_ID
nekomata panes attach --ssh dev-server --daemon /run/user/1000/felis.sock --session SESSION_ID
```

- Pane IDs belong to the workspace and stay the same across GUI restarts.
- Session IDs may be shortened as long as felis can resolve them uniquely.
- The target daemon must be running and the session must exist.
- On success the previous session keeps running in felis. On failure the pane
  is unchanged and the command exits with an error.
- Splitting an SSH pane creates the new shell on the same host. New tabs use
  the local daemon.

Run from a nekomata shell, these act on the window showing that shell's
workspace. Add `--window WINDOW_ID` to pick a window explicitly — for example
when several GUIs are running, or for a shell you moved into another
workspace, which still points at the workspace it was started in, or after
the nekomata daemon restarts. The `WINDOW` column of `nekomata workspaces
list` shows the IDs of running windows.

## Preparing an SSH host

- The host needs a compatible `felis-daemon` on the `PATH` of non-interactive
  SSH sessions (check with `ssh HOST command -v felis-daemon`), and the daemon
  must already be running there; nekomata never starts it.
- Use a `~/.ssh/config` host name or `user@host`.
- nekomata cannot answer password or first-time host-key prompts. Make sure
  plain `ssh` connects with public keys or an SSH agent first.
- Shells created on the host run the remote daemon's default shell. Your local
  `$SHELL`, working directory and environment are not forwarded.

See the [pane specification](../spec/panes.md) for the exact rules.
