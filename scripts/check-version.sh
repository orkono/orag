#!/usr/bin/env bash
# CI guard: Cargo version == top CHANGELOG entry (== git tag on tag builds).
set -euo pipefail
cd "$(dirname "$0")/.."
. scripts/version-lib.sh

cargo_version=$(cargo_version)
changelog_version=$(changelog_version)

if [ "$cargo_version" != "$changelog_version" ]; then
  echo "Cargo.toml ($cargo_version) != CHANGELOG.md ($changelog_version)" >&2
  exit 1
fi
if [ "${GITHUB_REF_TYPE:-}" = "tag" ] && [ "${GITHUB_REF_NAME:-}" != "v$cargo_version" ]; then
  echo "tag ${GITHUB_REF_NAME} != v$cargo_version" >&2
  exit 1
fi
echo "version $cargo_version consistent"
