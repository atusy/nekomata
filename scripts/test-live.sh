#!/usr/bin/env bash
set -euo pipefail

# Keep the test's daemon and sessions away from the user's normal felis socket.
test_directory=$(mktemp -d "${TMPDIR:-/tmp}/nekomata-test.XXXXXX")
chmod 700 "$test_directory"
export FELIS_SOCKET="$test_directory/daemon.sock"
export NEKOMATA_TEST_SOCKET="$FELIS_SOCKET"
cleanup() {
  felis --socket "$FELIS_SOCKET" daemon stop --force >/dev/null 2>&1 || true
  rm -rf "$test_directory"
}
trap cleanup EXIT
bash scripts/test-fish-integration.sh
cargo build --locked --bin nekomata
cargo test --locked live_ -- --ignored --nocapture --skip live_ssh_carrier
# Exercise the SSH stdio carrier against a private local relay without contacting
# an external host or relying on the user's SSH credentials/configuration.
mkdir "$test_directory/bin"
cat > "$test_directory/bin/ssh" <<'SSH'
#!/bin/sh
set -eu
[ "$#" = 5 ]
[ "$1" = '-oBatchMode=yes' ]
[ "$2" = 'nekomata-test-host' ]
[ "$3" = 'felis-daemon' ]
[ "$4" = 'relay' ]
[ "$5" = '--no-spawn' ]
exec felis-daemon relay --socket "$NEKOMATA_TEST_SOCKET" --no-spawn
SSH
chmod +x "$test_directory/bin/ssh"
PATH="$test_directory/bin:$PATH" NEKOMATA_TEST_SSH=1 \
  cargo test --locked live_ssh_carrier -- --ignored --nocapture
