#!/usr/bin/env bash
# Bumps the workspace version and prepends a CHANGELOG entry.
# Usage: scripts/bump-version.sh <new-version> <bullet> [<bullet>...]
# All-or-nothing: on any failure Cargo.toml, Cargo.lock and CHANGELOG.md are restored.
set -euo pipefail
cd "$(dirname "$0")/.."
. scripts/version-lib.sh

# MAJOR.MINOR.PATCH[-(alpha|beta|rc).N], numbers without leading zeros (semver).
num='(0|[1-9][0-9]*)'
semver="^$num\\.$num\\.$num(-(alpha|beta|rc)\\.$num)?$"

new="${1:-}"
shift || true
if [[ ! "$new" =~ $semver ]]; then
  echo "invalid version: '$new' (expected e.g. 0.1.0 or 0.1.0-alpha.4)" >&2
  exit 1
fi
if [ "$#" -eq 0 ]; then
  echo "at least one changelog bullet is required" >&2
  exit 1
fi
for bullet in "$@"; do
  if [ -z "${bullet//[[:space:]]/}" ]; then
    echo "changelog bullets must not be empty" >&2
    exit 1
  fi
done

current=$(cargo_version)
if [[ ! "$current" =~ $semver ]]; then
  echo "Cargo.toml version '$current' is not of the supported form; fix it by hand first" >&2
  exit 1
fi
# Start only from a consistent state, so a lost CHANGELOG entry is not hidden.
# (Assigned first so `set -e` stops on a missing or headless CHANGELOG.md.)
top=$(changelog_version)
if [ "$top" != "$current" ]; then
  echo "CHANGELOG.md top entry ($top) != Cargo.toml ($current); fix that first" >&2
  exit 1
fi

# Semver precedence: a.b.c-pre < a.b.c; pre-releases compare kind, then number.
version_key() {
  local core="${1%%-*}" pre=""
  [ "$core" != "$1" ] && pre="${1#*-}"
  IFS=. read -r major minor patch <<<"$core"
  if [ -z "$pre" ]; then
    printf '%09d.%09d.%09d.9.%09d\n' "$major" "$minor" "$patch" 0
  else
    local kind="${pre%%.*}" n="${pre#*.}" rank
    case "$kind" in alpha) rank=1 ;; beta) rank=2 ;; rc) rank=3 ;; esac
    printf '%09d.%09d.%09d.%d.%09d\n' "$major" "$minor" "$patch" "$rank" "$n"
  fi
}
if [[ ! "$(version_key "$current")" < "$(version_key "$new")" ]]; then
  echo "new version $new must be greater than the current $current" >&2
  exit 1
fi

backup=$(mktemp -d)
trap 'rm -rf "$backup"' EXIT
cp Cargo.toml Cargo.lock CHANGELOG.md "$backup/"
restore() {
  cp "$backup/Cargo.toml" "$backup/Cargo.lock" "$backup/CHANGELOG.md" .
  echo "bump failed; Cargo.toml, Cargo.lock and CHANGELOG.md restored" >&2
}
trap 'restore' ERR
# Ctrl-C or kill mid-bump also restores; the backup is removed only afterwards.
trap 'restore; rm -rf "$backup"; exit 130' INT
trap 'restore; rm -rf "$backup"; exit 143' TERM

sed -i.bak "s/^version = \"$current\"$/version = \"$new\"/" Cargo.toml
rm -f Cargo.toml.bak
grep -qx "version = \"$new\"" Cargo.toml || { echo "Cargo.toml version was not updated" >&2; false; }

entry="$backup/entry.md"
{
  printf '## [%s] - %s\n\n' "$new" "$(date -u +%Y-%m-%d)"
  for bullet in "$@"; do printf -- '- %s\n' "$bullet"; done
  printf '\n'
} > "$entry"
# Insert before the newest heading.
awk -v f="$entry" '!done && /^## \[[^]]*\]/ { while ((getline line < f) > 0) print line; done = 1 } { print }' \
  CHANGELOG.md > "$backup/CHANGELOG.new"
grep -qF "## [$new]" "$backup/CHANGELOG.new" || { echo "CHANGELOG.md has no version heading to insert before" >&2; false; }
cp "$backup/CHANGELOG.new" CHANGELOG.md

# Updates the workspace's own version in Cargo.lock (offline first, no upgrades).
if ! offline_err=$(cargo update --workspace --offline 2>&1 >/dev/null); then
  echo "offline Cargo.lock update failed, retrying online:" >&2
  echo "$offline_err" >&2
  cargo update --workspace >/dev/null
fi
echo "bumped $current -> $new"
