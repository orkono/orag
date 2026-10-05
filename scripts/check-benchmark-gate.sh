#!/usr/bin/env bash
# D-003 release gate: a PASS row at 100k x 1024 (k = 50, 50 queries, 250 ms target) for the
# given version on both reference platforms. The other columns (insert s, p50, p95, max) vary.
set -euo pipefail
cd "$(dirname "$0")/.."
version="${1:?usage: $0 <version>}"
file=".docs/benchmarks/vector-scale.md"
for platform in macos-aarch64 linux-x86_64; do
  num='[0-9]+(\.[0-9]+)?'
  row="^\| ${platform} \| ${version//./\\.} \| 100000 \| 1024 \| 50 \| 50 \| ${num} \| ${num} \| ${num} \| ${num} \| 250 \| PASS \|$"
  if ! grep -qE "$row" "$file"; then
    echo "missing passing 100k/1024 row for ${platform} at ${version} in ${file}" >&2
    exit 1
  fi
done
echo "benchmark gate passed for ${version}"
