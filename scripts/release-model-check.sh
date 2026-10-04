#!/usr/bin/env bash
# Release gate: real models + seed retrieval eval + binary dependency check.
set -euo pipefail
: "${ORAG_MODEL_DIR:?set ORAG_MODEL_DIR to a directory of installed model packs}"
# Absolute, resolved from the caller's directory before the cd below, so the
# symlink and cargo test (cwd crates/orag) see the same directory.
ORAG_MODEL_DIR=$(cd "$ORAG_MODEL_DIR" && pwd)
export ORAG_MODEL_DIR
cd "$(dirname "$0")/.."
# The gate tests the generation model the product ships, never a shell override.
unset ORAG_TEST_GENERATION_MODEL
# Seed-set quality floor for hybrid retrieval with the real embedder.
min_recall_at_10=0.9
real_model_tests=3

scripts/check-llama-build.sh --release

# Every real-model test must run: a build without them reports "0 passed" and exits 0.
log=$(mktemp)
home=$(mktemp -d)
trap 'rm -rf "$home" "$log"' EXIT
cargo test --release --locked --test real_models -- --ignored --test-threads=1 2>&1 | tee "$log"
if ! grep -q "test result: ok. $real_model_tests passed" "$log"; then
  echo "expected $real_model_tests real-model tests to pass" >&2
  exit 1
fi

# `cargo test` may relink the binary with dev-dependency features: rebuild the
# shipped one, then check and measure exactly that binary.
cargo build --release --locked
# No --release-floor here: this also runs on newer local distros. CI
# (ubuntu-22.04) enforces the glibc/libstdc++ floor; release.yml (Task 24)
# will enforce it on the artifacts.
scripts/check-binary-deps.sh target/release/orag
ln -s "$ORAG_MODEL_DIR" "$home/models"
report=target/release-seed-eval.json
rm -f "$report"
ORAG_HOME="$home" target/release/orag eval retrieval \
  --corpus eval/corpus/seed --dataset eval/datasets/seed.jsonl \
  --out "$report" --min-recall-at-10 "$min_recall_at_10"
echo "release model check passed"
