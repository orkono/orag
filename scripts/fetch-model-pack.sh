#!/usr/bin/env bash
# Builds an offline ORAG model pack on a connected machine (D-012).
# Usage: scripts/fetch-model-pack.sh <preset> <output-parent-dir>
# Then, on any machine: orag models import <output-parent-dir>/<preset>
set -euo pipefail

preset="${1:-}"
out_parent="${2:-}"
if [ -z "$preset" ] || [ -z "$out_parent" ]; then
  echo "usage: $0 <qwen3-embedding-0.6b-q8_0|qwen3.5-4b-q4_k_m|qwen3-4b-instruct-2507-q4_k_m> <output-dir>" >&2
  exit 1
fi

case "$preset" in
  qwen3-embedding-0.6b-q8_0)
    repo="Qwen/Qwen3-Embedding-0.6B-GGUF"
    revision="370f27d7550e0def9b39c1f16d3fbaa13aa67728"
    file="Qwen3-Embedding-0.6B-Q8_0.gguf"
    sha="06507c7b42688469c4e7298b0a1e16deff06caf291cf0a5b278c308249c3e439"
    license_url="https://www.apache.org/licenses/LICENSE-2.0.txt"
    license_sha="cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30"
    role_section=$'[embedding]\ndimensions = 1024\npooling = "last"\nmax_tokens = 1024\nquery_prefix = "Instruct: Given a search query, retrieve relevant passages that answer the query\\nQuery: "\ndocument_prefix = ""\nrequire_trailing_eos = true'
    role="embedding"
    ;;
  qwen3.5-4b-q4_k_m)
    repo="unsloth/Qwen3.5-4B-GGUF"
    revision="e87f176479d0855a907a41277aca2f8ee7a09523"
    file="Qwen3.5-4B-Q4_K_M.gguf"
    sha="00fe7986ff5f6b463e62455821146049db6f9313603938a70800d1fb69ef11a4"
    license_url="https://huggingface.co/Qwen/Qwen3.5-4B/resolve/851bf6e806efd8d0a36b00ddf55e13ccb7b8cd0a/LICENSE"
    license_sha="bbedc3fda3305820b977265f01b8619d87570a6739de3a5582c3464840f1e57a"
    role_section=$'[generation]\ncontext_tokens = 8192\nmax_output_tokens = 1024\nprompt_format = "chatml-nothink"'
    role="generation"
    ;;
  qwen3-4b-instruct-2507-q4_k_m)
    repo="unsloth/Qwen3-4B-Instruct-2507-GGUF"
    revision="a06e946bb6b655725eafa393f4a9745d460374c9"
    file="Qwen3-4B-Instruct-2507-Q4_K_M.gguf"
    sha="3605803b982cb64aead44f6c1b2ae36e3acdb41d8e46c8a94c6533bc4c67e597"
    license_url="https://huggingface.co/Qwen/Qwen3-4B-Instruct-2507/resolve/cdbee75f17c01a7cc42f958dc650907174af0554/LICENSE"
    license_sha="832dd9e00a68dd83b3c3fb9f5588dad7dcf337a0db50f7d9483f310cd292e92e"
    role_section=$'[generation]\ncontext_tokens = 8192\nmax_output_tokens = 1024\nprompt_format = "chatml"'
    role="generation"
    ;;
  *)
    echo "unknown preset: $preset" >&2
    exit 1
    ;;
esac

sha256() {
  if command -v sha256sum >/dev/null; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi
}

# download URL SHA256 TARGET: TARGET ends up with exactly the pinned bytes or
# not at all. A resumed .part that does not verify (another revision, a proxy
# page) is deleted, so the next run starts clean instead of failing again.
download() {
  local url="$1" expected="$2" target="$3" actual
  if [ -f "$target" ] && [ "$(sha256 "$target")" = "$expected" ]; then
    return 0
  fi
  rm -f "$target"
  echo "downloading $url"
  if ! curl -fL --retry 3 -C - -o "$target.part" "$url"; then
    # A resume the server refuses (or a .part from elsewhere): start over once.
    rm -f "$target.part"
    curl -fL --retry 3 -o "$target.part" "$url" || { rm -f "$target.part"; exit 1; }
  fi
  actual=$(sha256 "$target.part")
  if [ "$actual" != "$expected" ]; then
    rm -f "$target.part"
    echo "checksum mismatch for $(basename "$target"): expected $expected, got $actual (removed; run again)" >&2
    exit 1
  fi
  mv "$target.part" "$target"
}

dest="$out_parent/$preset"
mkdir -p "$dest"
# The manifest is written last, so a pack without one was never finished.
rm -f "$dest/orag-model.toml"
download "https://huggingface.co/$repo/resolve/$revision/$file" "$sha" "$dest/$file"
download "$license_url" "$license_sha" "$dest/LICENSE"

cat > "$dest/orag-model.toml" <<EOF
id = "$preset"
role = "$role"
file = "$file"
sha256 = "$sha"
license = "Apache-2.0"
license_file = "LICENSE"
source = "https://huggingface.co/$repo"
revision = "$revision"

$role_section
EOF
echo "pack ready: $dest"
