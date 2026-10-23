#!/usr/bin/env bash
# Run from the repository inside the disposable guest after provisioning/reboot.
set -euo pipefail
source "$HOME/.nix-profile/etc/profile.d/nix.sh"
repo=$(pwd)
mesa="$HOME/nekomata-mesa"
if [[ ! -e "$mesa" ]]; then
  nix build --impure --expr "(import (builtins.getFlake \"$repo\").inputs.nixpkgs {system=builtins.currentSystem;}).mesa.drivers" --out-link "$mesa"
fi
export NEKOMATA_LAB_MESA="$mesa"
exec nix develop -c bash -c '
set -euo pipefail
export LD_LIBRARY_PATH="$NEKOMATA_LAB_MESA/lib:${LD_LIBRARY_PATH:-}"
export LIBGL_DRIVERS_PATH="$NEKOMATA_LAB_MESA/lib/dri"
export __EGL_VENDOR_LIBRARY_FILENAMES="$NEKOMATA_LAB_MESA/share/glvnd/egl_vendor.d/50_mesa.json"
export VK_DRIVER_FILES=$(find "$NEKOMATA_LAB_MESA/share/vulkan/icd.d" -name "*lvp*.json" -print -quit)
export XDG_RUNTIME_DIR=/run/user/$(id -u)
if [[ $# -gt 0 ]]; then exec "$@"; fi
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
bash scripts/test-live.sh
cargo test --locked gpu_shared_device -- --ignored --nocapture
' bash "$@"
