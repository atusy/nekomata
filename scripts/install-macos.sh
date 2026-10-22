#!/usr/bin/env bash
set -euo pipefail

if [[ $(uname -s) != Darwin ]]; then
  echo 'This installer requires macOS.' >&2
  exit 1
fi

repository=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
package_directory="${XDG_DATA_HOME:-$HOME/.local/share}/nekomata"
application="$HOME/Applications/nekomata.app"

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
staging=$(mktemp -d "$HOME/Applications/.nekomata-install.XXXXXX")
trap 'chmod -R u+w "$staging"; rm -rf "$staging"' EXIT
/usr/bin/ditto "$package_directory/package/Applications/nekomata.app" "$staging/nekomata.app"
chmod -R u+w "$staging/nekomata.app"
# Nix store files are read-only. Replace the bundle instead of overwriting
# individual files, which also avoids modifying an executable that is running.
if [[ -e "$application" ]]; then
  chmod -R u+w "$application"
  mv "$application" "$staging/previous.app"
fi
if ! mv "$staging/nekomata.app" "$application"; then
  if [[ -e "$staging/previous.app" ]]; then
    mv "$staging/previous.app" "$application"
  fi
  exit 1
fi
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "$application"
echo "Installed $application"
