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
cargo test --locked live_ -- --ignored --nocapture
