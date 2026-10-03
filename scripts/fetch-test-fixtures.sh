#!/usr/bin/env bash
# Downloads the 1.2 MB stories260K GGUF used by llama smoke tests (pinned revision + SHA-256).
set -euo pipefail
cd "$(dirname "$0")/.."

url="https://huggingface.co/ggml-org/tiny-llamas/resolve/def3e2dd70df35ecbf6403ea347de4c5977220c1/stories260K.gguf"
expected="047bf46455a544931cff6fef14d7910154c56afbc23ab1c5e56a72e69912c04b"
dest="test-fixtures/stories260K.gguf"

sha256() {
  if command -v sha256sum >/dev/null; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi
}

mkdir -p test-fixtures
if [ ! -f "$dest" ]; then
  curl -fsSL --retry 3 -o "$dest.part" "$url"
  mv "$dest.part" "$dest"
fi
actual=$(sha256 "$dest")
if [ "$actual" != "$expected" ]; then
  echo "checksum mismatch for $dest: $actual" >&2
  rm -f "$dest"
  exit 1
fi
echo "fixture ok: $dest"
