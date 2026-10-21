#!/usr/bin/env bash
# Desktop app smoke test (D-016): starts the built app, waits until its
# sidecar serves the page, lets the page send a POST to the API from inside
# the webview (as the page itself would), then the app closes its window;
# the app must exit by itself and its sidecar must be gone.
# Usage: scripts/smoke-desktop.sh <app executable> [--expect-gpu-fallback]
#   (Linux: run under xvfb-run)
# The app's sidecar must sit next to it (target/release/orag after
# `cargo build --release -p orag`; Windows and Linux: orag-vulkan too).
# --expect-gpu-fallback (Windows, Linux CI without a GPU): the app must have
# tried orag-vulkan first and run on the CPU, with the fallback marker in its
# home naming why (v0.6 plan, Task 4): `loader-missing` when the Vulkan
# loader cannot be found, `no-eligible-device` when it can, never a probe
# timeout. The app data folder starts empty, so the probe really runs.
set -euo pipefail

app="${1:?usage: scripts/smoke-desktop.sh <app executable> [--expect-gpu-fallback]}"
expect_fallback=0
case "${2:-}" in
  "") ;;
  --expect-gpu-fallback) expect_fallback=1 ;;
  *) echo "unknown option: $2" >&2; exit 2 ;;
esac
# The app's data folder (Tauri app_data_dir, identifier com.orkono.orag).
case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*) data="$APPDATA/com.orkono.orag" ;;
  Darwin) data="$HOME/Library/Application Support/com.orkono.orag" ;;
  *) data="${XDG_DATA_HOME:-$HOME/.local/share}/com.orkono.orag" ;;
esac
marker="$data/orag/gpu-fallback.json"
if [ "$expect_fallback" = 1 ]; then
  rm -f "$marker"
fi
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
if [ "$expect_fallback" = 1 ]; then
  [ -f "$marker" ] || { echo "no GPU fallback marker at $marker" >&2; cat "$log" >&2; exit 1; }
  reason=$(sed -nE 's/.*"reason":"([^"]*)".*/\1/p' "$marker")
  echo "GPU fallback: $reason"
  case "$reason" in
    loader-missing|no-eligible-device) ;;
    *) echo "unexpected GPU fallback reason: $(cat "$marker")" >&2; exit 1 ;;
  esac
  if [ -n "${ORAG_EXPECT_GPU_REASON:-}" ] && [ "$reason" != "$ORAG_EXPECT_GPU_REASON" ]; then
    echo "expected the GPU fallback reason $ORAG_EXPECT_GPU_REASON, got $reason" >&2
    exit 1
  fi
fi
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
