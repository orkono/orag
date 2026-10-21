#!/usr/bin/env bash
# Seed retrieval with the real embedder on `orag` and on `orag-vulkan`, and
# across them (v0.6 plan, Task 3 Steps 2 and 2b): CPU-indexed collections
# queried on the GPU and the reverse, as a user who falls back (or comes
# back) keeps the collections they built. Each run must reach hybrid
# recall@10 >= 0.9 and keep x-001..x-003 in the answer context; every run
# must be within 0.02 of the CPU run. Linux CI: lavapipe is the GPU.
# Usage: scripts/check-gpu-retrieval.sh <orag> <orag-vulkan> <models dir>
set -euo pipefail
cpu="${1:?usage: $0 <orag> <orag-vulkan> <models dir>}"
gpu="${2:?usage: $0 <orag> <orag-vulkan> <models dir>}"
models="${3:?usage: $0 <orag> <orag-vulkan> <models dir>}"
cd "$(dirname "$0")/.."
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
fail() { echo "FAIL: $*" >&2; exit 1; }
tolerance=0.02

home="$tmp/home"
mkdir -p "$home"
ln -s "$(cd "$models" && pwd)" "$home/models"
printf 'bind = "127.0.0.1:0"\nui_bind = "off"\n' >"$home/config.toml"

# run NAME BINARY [--index-dir DIR]: the seed eval; report in $tmp/NAME.json.
run() {
  local name="$1" bin="$2"; shift 2
  echo "== $name ($bin)"
  ORAG_HOME="$home" "$bin" eval retrieval --no-context-budget \
    --corpus eval/corpus/seed --dataset eval/datasets/seed.jsonl \
    --out "$tmp/$name.json" --min-recall-at-10 0.9 \
    --require-in-context x-001,x-002,x-003 "$@" \
    || fail "$name: below the seed gate (see the table above)"
}
recall() { jq '.strategies[] | select(.strategy == "hybrid") | .recall_at_10' "$tmp/$1.json"; }

run cpu "$cpu"
run gpu "$gpu"
run cpu_indexed "$cpu" --index-dir "$tmp/cpu-index"
run gpu_queries_cpu_index "$gpu" --index-dir "$tmp/cpu-index"
run gpu_indexed "$gpu" --index-dir "$tmp/gpu-index"
run cpu_queries_gpu_index "$cpu" --index-dir "$tmp/gpu-index"

# Every run whose queries or index came from orag-vulkan must have had its
# embedder on the GPU (the report's embedding_offload); a CPU fallback
# would pass the recall gates without testing the Vulkan path.
for name in gpu gpu_queries_cpu_index gpu_indexed; do
  jq -e '.embedding_offload.layers_on_gpu > 0 and .embedding_offload.reason == null' \
    "$tmp/$name.json" >/dev/null \
    || fail "$name: the embedder was not on the GPU: $(jq -c .embedding_offload "$tmp/$name.json")"
done
for name in cpu cpu_indexed cpu_queries_gpu_index; do
  [ "$(jq -c .embedding_offload "$tmp/$name.json")" = null ] \
    || fail "$name: the CPU binary reported a GPU offload"
done

base=$(recall cpu)
for name in gpu gpu_queries_cpu_index cpu_queries_gpu_index; do
  value=$(recall "$name")
  awk -v a="$base" -v b="$value" -v t="$tolerance" \
    'BEGIN { d = a - b; if (d < 0) d = -d; exit !(d <= t + 1e-9) }' \
    || fail "$name: hybrid recall@10 $value, CPU $base (tolerance $tolerance)"
  echo "$name: hybrid recall@10 $value (CPU $base)"
done
echo "embedder on $(jq -r .embedding_offload.device "$tmp/gpu.json"): $(jq -r '.embedding_offload | "\(.layers_on_gpu)/\(.layers_total)"' "$tmp/gpu.json") layers"
echo "gpu retrieval ok"
