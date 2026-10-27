#!/usr/bin/env bash
# Fails unless every file that pins felis names the revision flake.nix pins.
# Usage: check-felis-revision.sh [repository root]
set -euo pipefail

root=${1:-$(cd "$(dirname "$0")/.." && pwd)}
cd "$root"
url='git.natsukium.com/felis-terminal/felis'
failed=0

fail() {
  printf '%s\n' "$*" >&2
  failed=1
}

expected=$(grep -o "felis.url = \"git+https://$url?rev=[0-9a-f]\{40\}\"" flake.nix |
  grep -o '[0-9a-f]\{40\}' || true)
if [[ $(printf '%s' "$expected" | grep -c '') != 1 ]]; then
  echo "flake.nix must pin the felis input to exactly one revision" >&2
  exit 1
fi

# Every Cargo git dependency on felis must name that revision.
for manifest in Cargo.toml vendor/felis-render-wgpu/Cargo.toml; do
  while IFS= read -r line; do
    [[ "$line" == *"rev = \"$expected\""* ]] || fail "$manifest pins another felis revision: $line"
  done < <(grep -F "git = \"https://$url\"" "$manifest")
  grep -qF "git = \"https://$url\"" "$manifest" || fail "$manifest has no felis git dependency"
done

# Cargo.lock records the requested and the resolved revision of each crate.
while IFS= read -r line; do
  [[ "$line" == "source = \"git+https://$url?rev=$expected#$expected\"" ]] ||
    fail "Cargo.lock resolves felis elsewhere: $line"
done < <(grep -F "$url" Cargo.lock)
grep -qF "$url?rev=$expected" Cargo.lock || fail "Cargo.lock does not lock felis $expected"

for field in locked original; do
  actual=$(jq -r ".nodes.felis.$field.rev" flake.lock)
  [[ "$actual" == "$expected" ]] || fail "flake.lock $field felis revision is $actual"
done

grep -q "^Revision: $expected\b" vendor/felis-render-wgpu/UPSTREAM.md ||
  fail "vendor/felis-render-wgpu/UPSTREAM.md does not record revision $expected"

if [[ $failed != 0 ]]; then
  echo "Update every felis pin to one revision (expected $expected from flake.nix)" >&2
  exit 1
fi
