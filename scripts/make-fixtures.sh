#!/usr/bin/env bash
# Regenerates the committed DOCX/PDF test fixtures. Requires pandoc and macOS cupsfilter.
set -euo pipefail
cd "$(dirname "$0")/.."
out=crates/orag/tests/fixtures
mkdir -p "$out"
tmp=$(mktemp -d)
cat > "$tmp/sample.md" <<'MD'
# Kargo Politikası

## İade Koşulları

Ürünler 14 gün içinde iade edilebilir.

- Orijinal ambalaj
- Fatura

| Sipariş tutarı | Kargo ücreti |
|---|---|
| 500 TL altı | 49,90 TL |
| 500 TL ve üzeri | Ücretsiz |
MD
pandoc "$tmp/sample.md" -o "$out/sample.docx"
printf 'Kargo Politikasi\n\nİade süresi 14 gündür. Iğdır, şık, çay, öğün.\n' > "$tmp/sample.txt"
cupsfilter -i text/plain -m application/pdf "$tmp/sample.txt" > "$out/sample-tr.pdf" 2>/dev/null
rm -rf "$tmp"
echo "fixtures written to $out"
