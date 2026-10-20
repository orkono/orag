#!/usr/bin/env bash
# Writes <dir>/SHA256SUMS: one `<sha256>  <name>` line per release file in
# <dir>, sorted by name, in the format `sha256sum -c` and `shasum -a 256 -c`
# read. Refuses a missing or empty folder, a subfolder and an existing file,
# so a release never ships a partial or stale list.
# Usage: scripts/write-checksums.sh <dir>
set -euo pipefail
dir="${1:?usage: scripts/write-checksums.sh <dir>}"
[ -d "$dir" ] || { echo "not a folder: $dir" >&2; exit 1; }
cd "$dir"
[ ! -e SHA256SUMS ] || { echo "$dir/SHA256SUMS already exists" >&2; exit 1; }
if command -v sha256sum >/dev/null; then hash() { sha256sum -- "$1"; }; else hash() { shasum -a 256 -- "$1"; }; fi

files=()
while IFS= read -r -d '' entry; do
  name="${entry#./}"
  [ -f "$name" ] || { echo "not a plain file: $dir/$name" >&2; exit 1; }
  files+=("$name")
done < <(find . -mindepth 1 -maxdepth 1 -print0 | LC_ALL=C sort -z)
[ "${#files[@]}" -gt 0 ] || { echo "no files in $dir" >&2; exit 1; }

out=$(mktemp)
trap 'rm -f "$out"' EXIT
for name in "${files[@]}"; do
  hash "$name" | awk -v n="$name" '{ print $1 "  " n }' >>"$out"
done
chmod 644 "$out"
mv "$out" SHA256SUMS
trap - EXIT
echo "wrote $dir/SHA256SUMS (${#files[@]} files)"
