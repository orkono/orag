#!/usr/bin/env bash
# Copies the release `orag` binary to where Tauri's externalBin expects it:
# apps/desktop/binaries/orag-<target triple>[.exe] (D-016). On Windows and
# Linux it also stages `orag-vulkan` (v0.6, scripts/build-vulkan.sh), which
# tauri.windows.conf.json and tauri.linux.conf.json bundle beside it.
# Usage: scripts/stage-sidecar.sh [<target triple>]   (default: the rustc host)
set -euo pipefail
cd "$(dirname "$0")/.."

triple="${1:-$(rustc -vV | sed -n 's/^host: //p')}"
if [ -z "$triple" ]; then
  echo "could not determine the target triple" >&2
  exit 1
fi
ext=""
case "$triple" in *windows*) ext=".exe" ;; esac

src="target/release/orag$ext"
[ -n "${1:-}" ] && [ -f "target/$triple/release/orag$ext" ] && src="target/$triple/release/orag$ext"
if [ ! -f "$src" ]; then
  echo "missing $src: build it first with cargo build --release -p orag" >&2
  exit 1
fi
mkdir -p apps/desktop/binaries
cp "$src" "apps/desktop/binaries/orag-$triple$ext"
echo "staged apps/desktop/binaries/orag-$triple$ext"

case "$triple" in
  *apple-darwin*) exit 0 ;;
esac
vulkan="target/vulkan/release/orag-vulkan$ext"
if [ ! -f "$vulkan" ]; then
  echo "missing $vulkan: build it first with scripts/build-vulkan.sh --release" >&2
  exit 1
fi
cp "$vulkan" "apps/desktop/binaries/orag-vulkan-$triple$ext"
# The unbundled app (cargo tauri build's target/release/orag-desktop, run by
# smoke-desktop.sh) finds its sidecars beside it.
cp "$vulkan" "target/release/orag-vulkan$ext"
echo "staged apps/desktop/binaries/orag-vulkan-$triple$ext"
