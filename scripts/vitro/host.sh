#!/usr/bin/env bash
# Keep VM keys, disks and config separate from the user's normal Vitro setup.
set -euo pipefail
repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
lab="${NEKOMATA_VITRO_LAB:-$HOME/.local/share/nekomata-vitro-lab}"
mkdir -p "$lab/tools"
export XDG_CONFIG_HOME="$lab/config"
export XDG_DATA_HOME="$lab/data"
export XDG_STATE_HOME="$lab/state"
export XDG_CACHE_HOME="$lab/cache"
if [[ ! -x "$lab/tools/vitro/bin/vitro" ]]; then
  nix build github:lambdalisue/vitro/672373f2c7807cbd675211d302dc044a400957d7 --out-link "$lab/tools/vitro"
fi
if [[ ! -x "$lab/tools/qemu/bin/qemu-system-aarch64" ]]; then
  nix build --inputs-from "$repo" nixpkgs#qemu --out-link "$lab/tools/qemu"
fi
export PATH="$lab/tools/qemu/bin:$PATH"
exec "$lab/tools/vitro/bin/vitro" "$@"
