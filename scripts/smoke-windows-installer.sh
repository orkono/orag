#!/usr/bin/env bash
# Installs ORAG-Setup-<version>.exe silently for the current user, checks the
# installed orag.exe (version, setup mode with no models, graceful Ctrl-Break),
# uninstalls it and checks that program files go and data stays (Task 47).
# Windows only (Git Bash). Usage: scripts/smoke-windows-installer.sh <setup.exe>
set -euo pipefail
# Git Bash (MSYS2) rewrites arguments that start with '/' into paths. The
# Windows tools here take /SWITCHES, so `win_tool` turns that off for their
# calls only (other tools, such as python, still need the conversion).
win_tool() { MSYS2_ARG_CONV_EXCL='*' MSYS_NO_PATHCONV=1 "$@"; }
cd "$(dirname "$0")/.."
. scripts/version-lib.sh
setup="${1:?usage: $0 <setup.exe>}"
version=$(cargo_version)
app="$LOCALAPPDATA/Programs/ORAG"
app_unix=$(cygpath -u "$app")

# Turkish explicitly: setup otherwise follows the runner's (English) UI language.
win_tool "$setup" /VERYSILENT /SUPPRESSMSGBOXES /CURRENTUSER /NORESTART /LANG=turkish "/LOG=$(cygpath -w "$RUNNER_TEMP/install.log")"
[ -f "$app_unix/orag.exe" ] || { echo "orag.exe was not installed to $app" >&2; cat "$RUNNER_TEMP/install.log" >&2; exit 1; }
for f in README-Windows.txt fetch-model-pack.ps1 THIRD-PARTY-LICENSES.md docs/api.md; do
  [ -f "$app_unix/$f" ] || { echo "$f was not installed" >&2; exit 1; }
done
installed=$("$app_unix/orag.exe" --version | tr -d '\r')
echo "installed: $installed"
[ "$installed" = "orag $version" ] || { echo "expected orag $version" >&2; exit 1; }
menu="$(cygpath -u "$APPDATA")/Microsoft/Windows/Start Menu/Programs/ORAG"
for lnk in "ORAG.lnk" "ORAG modellerini indir (internet).lnk"; do
  [ -f "$menu/$lnk" ] || { echo "Start menu shortcut missing: $lnk" >&2; ls -la "$menu" >&2 || true; exit 1; }
done
# The shortcut runs Windows PowerShell 5.1, not pwsh: the script must parse and
# run there (an unknown preset fails after parsing, before any download).
out=$(powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$(cygpath -w "$app_unix/fetch-model-pack.ps1")" -Preset nope -Yes 2>&1 || true)
grep -q "unknown preset: nope" <<<"$out" || { echo "fetch-model-pack.ps1 on PowerShell 5.1: $out" >&2; exit 1; }

# Data in the real %USERPROFILE%\.orag must survive uninstalling.
data="$(cygpath -u "$USERPROFILE")/.orag"
mkdir -p "$data"
marker="$data/smoke-marker-$$"
echo keep > "$marker"

home=$(mktemp -d)
printf 'bind = "127.0.0.1:7623"\nui_bind = "127.0.0.1:2442"\n' > "$home/config.toml"
# win_tool: "/v1/models" is a URL path, not a file to convert.
body=$(ORAG_HOME="$(cygpath -w "$home")" win_tool target/release/examples/ctrl_break.exe \
  127.0.0.1:2442 /v1/models -- "$(cygpath -w "$app_unix/orag.exe")" serve)
echo "$body"
grep -q '"mode":"setup"' <<<"$body" || { echo "expected setup mode without models" >&2; exit 1; }

win_tool "$app_unix/unins000.exe" /VERYSILENT /SUPPRESSMSGBOXES /NORESTART
for i in $(seq 1 60); do [ -f "$app_unix/orag.exe" ] || break; sleep 1; done
[ ! -f "$app_unix/orag.exe" ] || { echo "orag.exe is still installed after uninstall" >&2; exit 1; }
[ -f "$marker" ] || { echo "uninstall removed data in $data" >&2; exit 1; }
rm -f "$marker"
echo "installer smoke test passed for $version"
