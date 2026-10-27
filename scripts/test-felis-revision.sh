#!/usr/bin/env bash
# Exercises check-felis-revision.sh against copies of the pinned files.
set -euo pipefail

source_root=$(cd "$(dirname "$0")/.." && pwd)
check="$source_root/scripts/check-felis-revision.sh"
test_directory=$(mktemp -d "${TMPDIR:-/tmp}/nekomata-revision-test.XXXXXX")
trap 'rm -rf "$test_directory"' EXIT
files=(
  Cargo.toml
  Cargo.lock
  flake.nix
  flake.lock
  vendor/felis-render-wgpu/Cargo.toml
  vendor/felis-render-wgpu/UPSTREAM.md
)
other=0123456789abcdef0123456789abcdef01234567

fresh_copy() {
  chmod -R u+w "$test_directory" 2>/dev/null || true
  rm -rf "$test_directory/root"
  for file in "${files[@]}"; do
    mkdir -p "$test_directory/root/$(dirname "$file")"
    cp "$source_root/$file" "$test_directory/root/$file"
    chmod u+w "$test_directory/root/$file"
  done
}

fresh_copy
if ! bash "$check" "$test_directory/root" >/dev/null; then
  echo "The pinned files must agree on one felis revision" >&2
  exit 1
fi
revision=$(grep -o 'felis-terminal/felis?rev=[0-9a-f]\{40\}' "$test_directory/root/flake.nix" | head -n 1)
revision=${revision##*=}

for file in "${files[@]}"; do
  fresh_copy
  # Moving only the last mention catches a file that pins felis more than once.
  python3 - "$test_directory/root/$file" "$revision" "$other" <<'PY'
import sys
path, old, new = sys.argv[1:]
text = open(path).read()
index = text.rindex(old)
open(path, "w").write(text[:index] + new + text[index + len(old):])
PY
  if bash "$check" "$test_directory/root" >/dev/null 2>&1; then
    echo "A felis revision change in $file alone went unnoticed" >&2
    exit 1
  fi
done

fresh_copy
sed -i.bak 's/^Revision: .*/Revision: unknown/' "$test_directory/root/vendor/felis-render-wgpu/UPSTREAM.md"
if bash "$check" "$test_directory/root" >/dev/null 2>&1; then
  echo "UPSTREAM.md without the pinned revision went unnoticed" >&2
  exit 1
fi
