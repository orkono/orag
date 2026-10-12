#!/usr/bin/env bash
# Builds eval/corpus/v02 (Task 42): the documents written for the v0.2 set,
# DOCX/PDF members converted from eval/corpus-src/v02, plus copies of the
# older corpora. Requires pandoc and macOS cupsfilter.
#
# The constitution goes in once, as text: a second copy (the PDF) would hold
# the same evidence under another name, and a hit there would count as a miss.
set -euo pipefail
cd "$(dirname "$0")/.."
src=eval/corpus-src/v02
out=eval/corpus/v02
mkdir -p "$out"

for name in tr-insan-kaynaklari.md tr-izin-yonetmeligi.md en-warranty-terms.md tr-fiyat-listesi.txt; do
  cp "$src/$name" "$out/$name"
done
pandoc "$src/en-pump-manual.md" -o "$out/en-pump-manual.docx"
# Wrapped at spaces first: the printer would break long lines inside words.
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
fold -s -w 78 "$src/tr-sulama-katalogu.txt" > "$tmp/tr-sulama-katalogu.txt"
cupsfilter -i text/plain -m application/pdf "$tmp/tr-sulama-katalogu.txt" > "$out/tr-sulama-katalogu.pdf" 2>/dev/null

cp eval/corpus/seed/*.md "$out/"
cp eval/corpus/anayasa/tr-anayasa.txt "$out/"
cp eval/corpus/ceza/tr-yargitay-1cd-2016-347.pdf "$out/"

echo "v02 corpus written to $out"
