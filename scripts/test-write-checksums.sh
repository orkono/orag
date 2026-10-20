#!/usr/bin/env bash
# Tests scripts/write-checksums.sh: one SHA-256 line per release file, sorted,
# checkable with `sha256sum -c` / `shasum -a 256 -c`, and refusing bad input.
set -euo pipefail
cd "$(dirname "$0")/.."
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
fail() { echo "FAIL: $*" >&2; exit 1; }
check() { if command -v sha256sum >/dev/null; then sha256sum -c "$@"; else shasum -a 256 -c "$@"; fi; }

mkdir "$tmp/dist"
printf 'b' >"$tmp/dist/b.tar.gz"
printf 'a' >"$tmp/dist/ORAG_0.0.0_x64-setup.exe"
printf 'c c' >"$tmp/dist/name with space.deb"
scripts/write-checksums.sh "$tmp/dist"
sums="$tmp/dist/SHA256SUMS"
[ -f "$sums" ] || fail "no SHA256SUMS written"
[ "$(wc -l <"$sums" | tr -d ' ')" = 3 ] || fail "expected 3 lines: $(cat "$sums")"
grep -q '^ca978112ca1bbdcafac231b39a23dc4da786eff8147c4e72b9807785afee48bb  ORAG_0.0.0_x64-setup.exe$' "$sums" \
  || fail "wrong line for the installer: $(cat "$sums")"
grep -q 'SHA256SUMS' "$sums" && fail "the sums file lists itself"
[ "$(cut -c67- "$sums")" = "$(cut -c67- "$sums" | LC_ALL=C sort)" ] || fail "lines are not sorted by name"
(cd "$tmp/dist" && check SHA256SUMS >/dev/null) || fail "the file does not verify"
printf 'x' >>"$tmp/dist/b.tar.gz"
(cd "$tmp/dist" && check SHA256SUMS >/dev/null 2>&1) && fail "a changed file still verifies"

scripts/write-checksums.sh "$tmp/dist" 2>/dev/null && fail "an existing SHA256SUMS was overwritten"
mkdir "$tmp/empty"
scripts/write-checksums.sh "$tmp/empty" 2>/dev/null && fail "an empty folder was accepted"
scripts/write-checksums.sh "$tmp/missing" 2>/dev/null && fail "a missing folder was accepted"
mkdir -p "$tmp/nested/sub" && printf 'n' >"$tmp/nested/top.zip" && printf 'n' >"$tmp/nested/sub/x"
scripts/write-checksums.sh "$tmp/nested" 2>/dev/null && fail "a folder with a subfolder was accepted"
echo "write-checksums: all tests passed"
