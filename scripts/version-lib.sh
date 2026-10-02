# Version helpers shared by bump-version.sh and check-version.sh (sourced).
# Run from the repository root.

# cargo_version: the single top-level `version = "..."` of Cargo.toml
# ([workspace.package]); fails if there is not exactly one.
cargo_version() {
  if [ "$(grep -c '^version = "' Cargo.toml)" -ne 1 ]; then
    echo "Cargo.toml must contain exactly one top-level version line" >&2
    return 1
  fi
  sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml
}

# changelog_version: the newest heading of CHANGELOG.md (`## [x.y.z...]`).
# ORAG keeps no `## [Unreleased]` section: every change is released with its
# own version (D-017), so such a heading is rejected. Fails if the file is
# missing or has no version heading.
changelog_version() {
  if [ ! -r CHANGELOG.md ]; then
    echo "CHANGELOG.md is missing or unreadable" >&2
    return 1
  fi
  local version
  version=$(awk 'match($0, /^## \[[^]]*\]/) { print substr($0, 5, RLENGTH - 5); exit }' CHANGELOG.md)
  if [ -z "$version" ]; then
    echo "CHANGELOG.md has no \`## [x.y.z]\` heading" >&2
    return 1
  fi
  if [ "$version" = "Unreleased" ]; then
    echo "CHANGELOG.md has an [Unreleased] section; ORAG releases every change, so add its items with scripts/bump-version.sh instead" >&2
    return 1
  fi
  echo "$version"
}
