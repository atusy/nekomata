# Installation

nekomata is distributed as a Nix flake, so you need Nix with flakes enabled.
The flake builds for `aarch64-darwin`, `x86_64-linux` and `aarch64-linux`. On
Linux the GUI needs X11 or Wayland, and a Vulkan-capable GPU driver; the
terminal renderer refuses adapters without compatible texture views.

nekomata is developed on macOS (Apple silicon). Linux has been checked only in
a virtual machine (X11, software Vulkan), covering images, palette changes,
keyboard input and copying from a selection. With the current GPU renderer,
native keyboard delivery, mouse selection, resize gestures and high-DPI
rendering on macOS, and IME composition on both platforms, have not yet been
formally validated; see the [validation report](../native-rendering-validation.md).

The felis daemon ships in the same package and starts automatically when
nekomata opens a new shell; you do not install it separately.

## Try it

```sh
nix run github:atusy/nekomata
```

From a clone, `nix run .` works too. To build first and run the result:

```sh
nix build
./result/bin/nekomata
```

## Install the `nekomata` command

```sh
nix profile install github:atusy/nekomata
```

The same command also drives [workspaces](workspaces.md) and
[panes](sessions.md) from a shell.

## Install the macOS app

To launch nekomata from the Dock or Finder, run this in a clone. Run it again
to update.

```sh
bash scripts/install-macos.sh
open ~/Applications/nekomata.app
```

Drag `~/Applications/nekomata.app` to the Dock to keep it there. The installer
also links `~/.local/bin/nekomata`; put `~/.local/bin` on your `PATH` to use
the command from any terminal. An existing file of that name is not
overwritten. The link that keeps the Nix dependencies alive lives at
`~/.local/share/nekomata/package` (under `XDG_DATA_HOME` when set).

### Code signing

The installer signs the app so macOS privacy permissions survive updates.

- On first run it creates a local-only certificate named
  `nekomata Local Code Signing` and reuses it afterwards.
- The private key is stored non-exportable in your default keychain, and only
  `codesign` may use it without asking you.
- The certificate is not marked as trusted. macOS recognizes the app by the
  certificate's fingerprint, which needs no trust, so trust settings and the
  system keychain are left alone. Earlier versions of the installer trusted the
  certificate for code signing; to drop that setting, run
  `security remove-trusted-cert ~/.local/share/nekomata/signing/certificate.pem`.
- The public certificate is saved at
  `~/.local/share/nekomata/signing/certificate.pem` (under `XDG_DATA_HOME`
  when set).
- To sign with an existing certificate, set `NEKOMATA_SIGNING_IDENTITY` to its
  name or fingerprint.

When macOS asks to authorize a keychain operation, review it and allow it.
After the signing identity changes, restart the GUI and grant data access
again where asked.

## Shells

- New tabs start in the directory you ran `nekomata` from, or in your home
  directory when you launch the macOS app. Splits of a local pane start in
  the directory of the pane you split, when its shell reports it (OSC 7).
- On macOS shells start as login shells (`$SHELL -l`), so zsh reads
  `.zprofile` and friends.
- Changes to your shell configuration apply to new sessions. A reattached
  session keeps its original environment.

### fish

The package includes a startup hook for fish. It tells the terminal where
fish's initialization ends, so the very first prompt also redraws when you
resize a pane. Your fish configuration is not modified, and the hook applies
only to fish started by nekomata. It needs fish 4 or later (`mark-prompt`).
