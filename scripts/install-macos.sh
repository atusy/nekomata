#!/usr/bin/env bash
set -euo pipefail

# The staging directory is removed on exit unless `staging` is cleared first.
prepare_staging() {
  staging=$(mktemp -d "$1/.nekomata-install.XXXXXX")
  trap remove_staging EXIT
}

remove_staging() {
  if [[ -n ${staging:-} ]]; then
    chmod -R u+w "$staging"
    rm -rf "$staging"
  fi
}

# Nix store files are read-only. Replace the bundle instead of overwriting
# individual files, which also avoids modifying an executable that is running.
replace_application() {
  local application="$1"
  if [[ -e "$application" ]]; then
    chmod -R u+w "$application"
    mv "$application" "$staging/previous.app"
  fi
  if ! mv "$staging/nekomata.app" "$application"; then
    if [[ -e "$staging/previous.app" ]] && ! mv "$staging/previous.app" "$application"; then
      # Removing staging now would delete the only copy of the previous app.
      echo "Could not restore the previous app; it is kept at $staging/previous.app" >&2
      staging=
    fi
    exit 1
  fi
}

main() {
  if [[ $(uname -s) != Darwin ]]; then
    echo 'This installer requires macOS.' >&2
    exit 1
  fi

  repository=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
  package_directory="${XDG_DATA_HOME:-$HOME/.local/share}/nekomata"
  application="$HOME/Applications/nekomata.app"
  command_link="$HOME/.local/bin/nekomata"

  if [[ -e "$application" ]]; then
    identifier=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$application/Contents/Info.plist")
    if [[ "$identifier" != io.github.atusy.nekomata ]]; then
      echo "Refusing to replace an unrelated application: $application" >&2
      exit 1
    fi
  fi

  # Keep the installed wrapper's Nix dependencies alive across garbage collection.
  mkdir -p "$package_directory" "$HOME/Applications"
  nix build "$repository" --out-link "$package_directory/package"
  prepare_staging "$HOME/Applications"
  /usr/bin/ditto "$package_directory/package/Applications/nekomata.app" "$staging/nekomata.app"
  chmod -R u+w "$staging/nekomata.app"
  # Sign from the inside out, before replacing the installed app. A persistent
  # certificate keeps the designated requirement stable across rebuilds.
  identity=$(bash "$repository/scripts/macos-signing-identity.sh")
  /usr/bin/codesign --force --sign "$identity" --timestamp=none \
    --identifier io.github.atusy.nekomata.bin "$staging/nekomata.app/Contents/MacOS/nekomata-bin"
  /usr/bin/codesign --force --sign "$identity" --timestamp=none \
    --identifier io.github.atusy.nekomata "$staging/nekomata.app"
  /usr/bin/codesign --verify --deep --strict --verbose=2 "$staging/nekomata.app"
  # Nix store timestamps are fixed; give macOS a fresh bundle modification
  # time so the Dock refreshes its icon after an application replacement.
  touch "$staging/nekomata.app"
  replace_application "$application"
  /System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "$application"

  # Terminals outside nekomata lack the launcher's PATH, so expose the CLI wrapper.
  # Point at the GC-rooted package link so reinstalling updates the command too.
  if [[ -e "$command_link" || -L "$command_link" ]] \
    && [[ $(readlink "$command_link" 2>/dev/null) != "$package_directory/package/bin/nekomata" ]]; then
    echo "Skipping $command_link: it exists and is not managed by this installer" >&2
  else
    mkdir -p "$(dirname "$command_link")"
    ln -sfn "$package_directory/package/bin/nekomata" "$command_link"
    echo "Linked $command_link"
  fi
  echo "Installed $application"
}

# Sourcing defines the functions without installing, for tests.
if [[ ${BASH_SOURCE[0]} == "$0" ]]; then
  main "$@"
fi
