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
nekomata, keeping every session. Run this in the nekomata clone you installed
from, so that you get the felis revision your nekomata is pinned to:

```sh
nix develop --command felis daemon upgrade
```

If the daemon refuses, have it stop once your last shell exits, then start
nekomata again:

```sh
nix develop --command felis daemon stop --when-empty
```

If you installed with `nix profile`, run the commands with the locked flake
URL that `nix profile list` shows for nekomata (for example
`nix develop 'github:atusy/nekomata/<rev>' --command …`); a plain
`github:atusy/nekomata` follows the current default branch, whose felis may
differ from yours. `nix develop` downloads the development environment;
prefer it over another felis on your `PATH`, which may be a different
revision.

## macOS asks for permissions in another app's name

nekomata shares a running felis daemon. If that daemon was started from
another app — for example by a felis command run in kitty — macOS attributes
permission requests to that app. To change this, end
your existing sessions, stop the felis daemon, then launch nekomata from the
Dock so that nekomata starts the daemon.

## A setting has no effect

- Restart the GUI; configuration is read only at startup.
- Check that the setting is in the right file. Terminal settings belong in
  felis's `config.toml`, application settings in nekomata's (see
  [Configuration](configuration.md)).
- Some felis sections are not honored by nekomata yet: `window`, `shader`,
  `mouse` and `clipboard`.
- Look for warnings on standard error. Only nekomata's own file reports
  warnings there; to check felis's file as nekomata reads it, run:

  ```sh
  nix develop --command felis config check --client nekomata
  ```

  (in your nekomata clone, or with the locked flake URL described
  [above](#an-update-to-felis-does-not-take-effect)).

## Layouts are gone

Workspaces live in the nekomata daemon's memory, so stopping or crashing that
daemon loses them. Your felis shells survive that; list them with
`nekomata felis sessions list` and [attach to them again](sessions.md). After
an OS restart, the shells are gone as well.
