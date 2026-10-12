#!/usr/bin/env bash
# Installs ORAG-Setup-<version>.exe silently for the current user, checks the
# installed orag.exe (version, setup mode with no models, graceful Ctrl-Break),
# uninstalls it and checks that program files go and data stays (Task 47).
# Windows only (Git Bash). Usage: scripts/smoke-windows-installer.sh <setup.exe>
set -euo pipefail
cd "$(dirname "$0")/.."
. scripts/version-lib.sh
setup="${1:?usage: $0 <setup.exe>}"
version=$(cargo_version)
app="$LOCALAPPDATA/Programs/ORAG"
app_unix=$(cygpath -u "$app")

"$setup" /VERYSILENT /SUPPRESSMSGBOXES /CURRENTUSER /NORESTART "/LOG=$(cygpath -w "$RUNNER_TEMP/install.log")"
[ -f "$app_unix/orag.exe" ] || { echo "orag.exe was not installed to $app" >&2; cat "$RUNNER_TEMP/install.log" >&2; exit 1; }
for f in README-Windows.txt fetch-model-pack.ps1 THIRD-PARTY-LICENSES.md docs/api.md; do
  [ -f "$app_unix/$f" ] || { echo "$f was not installed" >&2; exit 1; }
done
"$app_unix/orag.exe" --version | tee /dev/stderr | grep -qx "orag $version"

home=$(mktemp -d)
printf 'bind = "127.0.0.1:7623"\nui_bind = "127.0.0.1:2442"\n' > "$home/config.toml"
body=$(ORAG_HOME="$(cygpath -w "$home")" target/release/examples/ctrl_break.exe \
  127.0.0.1:2442 /v1/models -- "$(cygpath -w "$app_unix/orag.exe")" serve)
echo "$body"
grep -q '"mode":"setup"' <<<"$body" || { echo "expected setup mode without models" >&2; exit 1; }

"$app_unix/unins000.exe" /VERYSILENT /SUPPRESSMSGBOXES /NORESTART
for i in $(seq 1 60); do [ -f "$app_unix/orag.exe" ] || break; sleep 1; done
[ ! -f "$app_unix/orag.exe" ] || { echo "orag.exe is still installed after uninstall" >&2; exit 1; }
[ -f "$home/config.toml" ] || { echo "the data folder was touched by uninstall" >&2; exit 1; }
echo "installer smoke test passed for $version"
