#!/usr/bin/env bash
# Builds ORAG-Setup-<version>.exe with Inno Setup 6 (Task 47) from the release
# binary. Windows only (Git Bash); used by CI (smoke test) and release.yml.
# Usage: scripts/build-windows-installer.sh <output-dir>
set -euo pipefail
# Git Bash (MSYS2) rewrites arguments that start with '/' into paths. The
# Windows tools here take /SWITCHES, so `win_tool` turns that off for their
# calls only (other tools, such as python, still need the conversion).
win_tool() { MSYS2_ARG_CONV_EXCL='*' MSYS_NO_PATHCONV=1 "$@"; }
cd "$(dirname "$0")/.."
. scripts/version-lib.sh
out="${1:?usage: $0 <output-dir>}"
version=$(cargo_version)
exe=target/release/orag.exe
[ -f "$exe" ] || { echo "$exe is missing; build the release first" >&2; exit 1; }

iscc=$(command -v iscc || command -v ISCC || true)
for guess in "/c/Program Files (x86)/Inno Setup 6/ISCC.exe" "/c/Program Files/Inno Setup 6/ISCC.exe"; do
  [ -n "$iscc" ] || { [ -f "$guess" ] && iscc="$guess"; }
done
[ -n "$iscc" ] || { echo "Inno Setup 6 (ISCC.exe) not found; choco install innosetup" >&2; exit 1; }

stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
mkdir -p "$stage/docs" "$out"
# Inno Setup resolves a relative OutputDir against the .iss file's folder.
out=$(cd "$out" && pwd)
cp "$exe" "$stage/orag.exe"
cp LICENSE-MIT LICENSE-APACHE README.md CHANGELOG.md "$stage/"
cp docs/api.md "$stage/docs/"
cp assets/orag-icon.ico packaging/windows/README-Windows.txt scripts/fetch-model-pack.ps1 "$stage/"
scripts/third-party-licenses.sh "$stage/THIRD-PARTY-LICENSES.md"

win() { cygpath -w "$1"; }
win_tool "$iscc" "/DAppVersion=$version" "/DSourceDir=$(win "$stage")" "/DOutputDir=$(win "$out")" \
  "$(win packaging/windows/orag.iss)"
ls -l "$out/ORAG-Setup-$version.exe"
