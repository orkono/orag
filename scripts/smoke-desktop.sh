#!/usr/bin/env bash
# Desktop app smoke test (D-016): starts the built app, waits until its
# sidecar serves the page, lets the page send a POST to the API from inside
# the webview (as the page itself would), then the app closes its window;
# the app must exit by itself and its sidecar must be gone.
# Usage: scripts/smoke-desktop.sh <app executable>   (Linux: run under xvfb-run)
# The app's sidecar must sit next to it (target/release/orag after
# `cargo build --release -p orag`).
set -euo pipefail

app="${1:?usage: scripts/smoke-desktop.sh <app executable>}"
work=$(mktemp -d)
ready="$work/ready.txt"
smoke="$work/smoke.txt"
log="$work/app.log"

alive() {
  case "$(uname -s)" in
    MINGW*|MSYS*|CYGWIN*) tasklist //FI "PID eq $1" //NH | grep -q "$1" ;;
    *) kill -0 "$1" 2>/dev/null ;;
  esac
}

ORAG_DESKTOP_READY_FILE="$ready" ORAG_DESKTOP_SMOKE_FILE="$smoke" "$app" >"$log" 2>&1 &
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
# The service page loaded in the webview and sent its own POST (the app may
# already be closing by now): a relative pack path is refused as invalid input
# (400); a 403 would mean the page's origin is not accepted.
for _ in $(seq 1 60); do
  [ -s "$smoke" ] && break
  sleep 1
done
if [ ! -s "$smoke" ]; then
  echo "the page did not report its API call within 60 s:" >&2
  cat "$log" >&2
  exit 1
fi
read -r post_status post_body < "$smoke"
echo "webview POST answered $post_status: $post_body"
if [ "$post_status" != 400 ] || ! printf '%s' "$post_body" | grep -q invalid_input; then
  echo "expected 400 invalid_input from the webview's POST" >&2
  exit 1
fi

# The app closed its window after reporting: it must exit on its own.
for _ in $(seq 1 60); do
  kill -0 "$app_pid" 2>/dev/null || break
  sleep 0.5
done
if kill -0 "$app_pid" 2>/dev/null; then
  echo "the app did not exit after its window closed" >&2
  exit 1
fi
for _ in $(seq 1 30); do
  alive "$sidecar" || { echo "sidecar stopped with the app"; exit 0; }
  sleep 0.5
done
echo "sidecar $sidecar outlived the app" >&2
exit 1
