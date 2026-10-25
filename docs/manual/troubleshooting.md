# Troubleshooting

## nekomata reports an incompatible daemon

After an update, a nekomata daemon from an older version may still be
running. nekomata refuses to replace it, because that would discard your
workspaces. The new `nekomata daemon stop` cannot stop it either, because the
old daemon rejects every request from the new version. Close every window,
then end the old daemon process:

```sh
pkill -f 'nekomata-bin daemon run'
```

Your workspace layouts are lost, but your shells keep running in felis;
[attach to them again](sessions.md) after restarting.

## An update to felis does not take effect

Restarting the GUI does not restart the felis daemon, which owns your shells.
Ask the running daemon to replace itself with the felis bundled with
nekomata, keeping every session:

```sh
nix develop github:atusy/nekomata --command felis daemon upgrade
```

If the daemon refuses, have it stop once your last shell exits, then start
nekomata again:

```sh
nix develop github:atusy/nekomata --command felis daemon stop --when-empty
```

`nix develop` provides the felis revision nekomata is pinned to, at the cost
of downloading the development environment; use it rather than another felis
on your `PATH`.

## macOS asks for permissions in another app's name

nekomata shares a running felis daemon. If kitty or another app started that
daemon, macOS attributes permission requests to that app. To change this, end
your existing sessions, stop the felis daemon, then launch nekomata from the
Dock so that nekomata starts the daemon.

## A setting has no effect

- Restart the GUI; configuration is read only at startup.
- Check that the setting is in the right file. Terminal settings belong in
  felis's `config.toml`, application settings in nekomata's (see
  [Configuration](configuration.md)).
- Some felis sections are not honored by nekomata yet: `window`, `shader`,
  `mouse` and `clipboard`.
- Look for warnings on standard error.

## Layouts are gone

Workspaces live in the nekomata daemon's memory. Stopping that daemon, a
crash or an OS restart loses them. Your felis shells survive; list them with
`nekomata felis sessions list` and [attach to them again](sessions.md).
