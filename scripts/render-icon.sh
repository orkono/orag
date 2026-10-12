#!/usr/bin/env bash
# Renders the PNG copies and the Windows .ico (16/32/48/256, used by the
# installer) of the project icon from its SVG source.
# Usage: scripts/render-icon.sh [--check]   (--check: fail if a copy is stale)
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
command -v magick >/dev/null || { echo "magick (ImageMagick) is required for the .ico" >&2; exit 1; }
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
layers=()
for size in 16 32 48 256; do
  rsvg-convert "$src" -w "$size" -h "$size" -o "$work/$size.png"
  layers+=("$work/$size.png")
done
ico=assets/orag-icon.ico
if [ "${1:-}" = "--check" ]; then
  magick "${layers[@]}" "$work/check.ico"
  cmp -s "$work/check.ico" "$ico" || { echo "$ico is stale; run scripts/render-icon.sh" >&2; exit 1; }
else
  magick "${layers[@]}" "$ico"
fi
echo "icon PNGs and .ico match $src"
