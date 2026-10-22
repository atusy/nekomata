#!/usr/bin/env bash
set -euo pipefail

test_directory=$(mktemp -d "${TMPDIR:-/tmp}/nekomata-fish-test.XXXXXX")
trap 'rm -rf "$test_directory"' EXIT
export XDG_CONFIG_HOME="$test_directory/config"
export XDG_DATA_HOME="$test_directory/data"
export XDG_DATA_DIRS="$PWD/share:${XDG_DATA_DIRS:-/usr/local/share:/usr/share}"

markers() {
  local text="$1" result="" pattern=$'\e\\]133;[CD][^\a]*\a'
  while [[ "$text" =~ $pattern ]]; do
    result+="${BASH_REMATCH[0]}"
    text="${text#*"${BASH_REMATCH[0]}"}"
  done
  printf '%s' "$result"
}

actual=$(markers "$(NEKOMATA_SHELL_INTEGRATION=1 fish --interactive --command 'emit fish_prompt; emit fish_prompt')")
expected=$'\e]133;C\a\e]133;D;0\a'
if [[ "$actual" != "$expected" ]]; then
  printf 'Expected one startup boundary, got: %q\n' "$actual" >&2
  exit 1
fi

actual=$(markers "$(env -u NEKOMATA_SHELL_INTEGRATION fish --interactive --command 'emit fish_prompt')")
if [[ -n "$actual" ]]; then
  printf 'Integration affected an unrelated fish: %q\n' "$actual" >&2
  exit 1
fi
