#!/usr/bin/env bash
# Desktop app smoke test (D-016): starts the built app, waits until its
# sidecar serves the page, checks the page and a same-origin API call, quits
# the app and checks that the sidecar is gone.
# Usage: scripts/smoke-desktop.sh <app executable>   (Linux: run under xvfb-run)
# The app's sidecar must sit next to it (target/release/orag after
# `cargo build --release -p orag`).
set -euo pipefail

app="${1:?usage: scripts/smoke-desktop.sh <app executable>}"
work=$(mktemp -d)
ready="$work/ready.txt"
log="$work/app.log"

alive() {
  case "$(uname -s)" in
    MINGW*|MSYS*|CYGWIN*) tasklist //FI "PID eq $1" //NH | grep -q "$1" ;;
    *) kill -0 "$1" 2>/dev/null ;;
  esac
}

ORAG_DESKTOP_READY_FILE="$ready" "$app" >"$log" 2>&1 &
app_pid=$!
trap 'kill "$app_pid" 2>/dev/null || true' EXIT

for _ in $(seq 1 120); do
  [ -s "$ready" ] && break
  if ! kill -0 "$app_pid" 2>/dev/null; then
    echo "the app exited before its service was ready:" >&2
    cat "$log" >&2
    exit 1
  fi
  sleep 1
done
if [ ! -s "$ready" ]; then
  echo "the service did not become ready within 120 s:" >&2
  cat "$log" >&2
  exit 1
fi
read -r url sidecar < "$ready"
echo "page $url, sidecar pid $sidecar"
origin="${url%/}"

curl -fsS "${origin}/v1/version" >/dev/null
status=$(curl -s -o /dev/null -w '%{http_code}' "$origin/")
[ "$status" = 200 ] || { echo "page answered $status" >&2; exit 1; }
# A same-origin POST from the page's origin must pass the Origin check: a
# relative path is refused as invalid input (400), not as forbidden (403).
status=$(curl -s -o "$work/import.json" -w '%{http_code}' -X POST \
  -H "Origin: $origin" -H "Sec-Fetch-Site: same-origin" -H "Content-Type: application/json" \
  --data '{"path":"relative/pack"}' "$origin/v1/models/import")
if [ "$status" = 403 ]; then
  echo "same-origin POST refused (403):" >&2
  cat "$work/import.json" >&2
  exit 1
fi
echo "same-origin POST answered $status"

kill "$app_pid"
wait "$app_pid" 2>/dev/null || true
for _ in $(seq 1 30); do
  alive "$sidecar" || { echo "sidecar stopped with the app"; exit 0; }
  sleep 0.5
done
echo "sidecar $sidecar outlived the app" >&2
exit 1
