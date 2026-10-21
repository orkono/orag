#!/usr/bin/env bash
# Builds `orag-vulkan` (v0.6, D-001): the service with llama.cpp's Vulkan
# backend, in its own target directory so the CPU `orag` build is never
# touched, then checks the llama.cpp build (scripts/check-llama-build.sh).
# Needs the Vulkan SDK (VULKAN_SDK, glslc, SPIRV-Headers). Windows and Linux.
# Usage: scripts/build-vulkan.sh [--release]
#   -> target/vulkan/<debug|release>/orag-vulkan[.exe]
set -euo pipefail
cd "$(dirname "$0")/.."
profile=debug
case "${1:-}" in
  "") ;;
  --release) profile=release ;;
  *) echo "usage: $0 [--release]" >&2; exit 2 ;;
esac
ext=""
case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*) ext=".exe" ;;
  Darwin) echo "orag-vulkan is not built on macOS (Metal is in orag)" >&2; exit 2 ;;
esac
export CARGO_TARGET_DIR=target/vulkan
scripts/check-llama-build.sh -p orag --features vulkan "$@"
out="target/vulkan/$profile"
cp "$out/orag$ext" "$out/orag-vulkan$ext"
echo "built $out/orag-vulkan$ext ($(wc -c <"$out/orag-vulkan$ext" | tr -d ' ') bytes)"
