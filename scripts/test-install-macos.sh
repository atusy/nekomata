#!/usr/bin/env bash
# Exercise the installer's bundle replacement in a temporary HOME. It never
# touches ~/Applications and runs on any platform.
# shellcheck source-path=SCRIPTDIR
set -euo pipefail

repository=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
test_directory=$(mktemp -d "${TMPDIR:-/tmp}/nekomata-install-test.XXXXXX")
trap 'rm -rf "$test_directory"' EXIT

# Fail each move whose source is listed, and perform the others.
fail_sources=()
mv() {
  local source
  # bash before 4.4 treats an empty array as unset under `set -u`.
  for source in ${fail_sources[@]+"${fail_sources[@]}"}; do
    if [[ "$1" == "$source" ]]; then
      echo "mv: simulated failure: $1" >&2
      return 1
    fi
  done
  command mv "$@"
}

# Replace an installed "previous" app with a staged "new" one in a subshell
# that exits like the installer. @STAGING@ in the arguments names the staging
# directory; those moves fail. Sets $home and $status; stderr goes to a file.
replace() {
  home="$test_directory/$1"
  shift
  mkdir -p "$home/Applications/nekomata.app"
  echo previous > "$home/Applications/nekomata.app/version"
  # A subshell tested with && or || ignores errexit; the installer relies on it.
  set +e
  (
    export HOME="$home"
    # shellcheck source=install-macos.sh
    source "$repository/scripts/install-macos.sh"
    prepare_staging "$HOME/Applications"
    mkdir "$staging/nekomata.app"
    echo new > "$staging/nekomata.app/version"
    for source in "$@"; do
      fail_sources+=("${source//@STAGING@/$staging}")
    done
    replace_application "$HOME/Applications/nekomata.app"
  ) 2> "$home/stderr"
  status=$?
  set -e
}

fail() {
  echo "FAIL: $*" >&2
  sed 's/^/  stderr: /' "$home/stderr" >&2
  exit 1
}

installed_version() {
  cat "$home/Applications/nekomata.app/version" 2>/dev/null || echo missing
}

leftovers() {
  find "$home/Applications" -maxdepth 1 -name '.nekomata-install.*' | wc -l | tr -d ' '
}

replace success
[[ $status == 0 ]] || fail "replacement failed with $status"
[[ $(installed_version) == new ]] || fail "new app not installed: $(installed_version)"
[[ $(leftovers) == 0 ]] || fail 'staging directory left behind'

replace rollback @STAGING@/nekomata.app
[[ $status != 0 ]] || fail 'failed install reported success'
[[ $(installed_version) == previous ]] || fail "previous app not restored: $(installed_version)"
[[ $(leftovers) == 0 ]] || fail 'staging directory left behind after rollback'

replace stranded @STAGING@/nekomata.app @STAGING@/previous.app
[[ $status != 0 ]] || fail 'failed install reported success'
kept=$(find "$home/Applications" -path '*/.nekomata-install.*/previous.app/version' -print)
[[ -n "$kept" && $(cat "$kept") == previous ]] || fail 'previous app was deleted'
grep -F "${kept%/version}" "$home/stderr" > /dev/null || fail 'previous app location not reported'

echo 'install-macos replacement tests passed'
