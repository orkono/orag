#!/usr/bin/env bash
# Renders the PNG copies of the project icon from its SVG source.
# Usage: scripts/render-icon.sh [--check]   (--check: fail if the PNGs are stale)
set -euo pipefail
cd "$(dirname "$0")/.."
command -v rsvg-convert >/dev/null || { echo "rsvg-convert (librsvg) is required" >&2; exit 1; }
src=assets/orag-icon.svg
for size in 512 256; do
  png="assets/orag-icon-${size}.png"
  if [ "${1:-}" = "--check" ]; then
    tmp=$(mktemp)
    rsvg-convert "$src" -w "$size" -h "$size" -o "$tmp"
    cmp -s "$tmp" "$png" || { rm -f "$tmp"; echo "$png is stale; run scripts/render-icon.sh" >&2; exit 1; }
    rm -f "$tmp"
  else
    rsvg-convert "$src" -w "$size" -h "$size" -o "$png"
  fi
done
echo "icon PNGs match $src"
