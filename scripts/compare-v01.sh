#!/usr/bin/env bash
# Release gate (Task 46): retrieval of the current build must not fall behind
# v0.1.0 on seed.jsonl and on the text-only subset of the v02 set, scored by
# both builds with the same embedder on the same corpora.
set -euo pipefail
: "${ORAG_MODEL_DIR:?set ORAG_MODEL_DIR to a directory of installed model packs}"
ORAG_MODEL_DIR=$(cd "$ORAG_MODEL_DIR" && pwd)
cd "$(dirname "$0")/.."
root=$(pwd)
# v0.1.0's eval reads only text corpus files and rejects labels it cannot match.
min_subset=90

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/v01"
git archive v0.1.0 | tar -x -C "$tmp/v01"
# A separate, kept target dir so reruns do not rebuild v0.1.0 from scratch.
(cd "$tmp/v01" && CARGO_TARGET_DIR="$root/target/compare-v01" cargo build -q --release --locked)
cargo build -q --release --locked
v01="$root/target/compare-v01/release/orag"
cur="$root/target/release/orag"

mkdir -p "$tmp/subset/corpus"
for f in eval/corpus/v02/*; do
  case "$f" in *.md|*.markdown|*.txt) cp "$f" "$tmp/subset/corpus/" ;; esac
done
python3 - "$tmp/subset" <<'PY'
import json, os, sys
out = sys.argv[1]
text = set(os.listdir(os.path.join(out, "corpus")))
kept = []
for line in open("eval/datasets/v02-retrieval.jsonl", encoding="utf-8"):
    if not line.strip():
        continue
    q = json.loads(line)
    if q["answerable"] and not all(r["document"] in text for r in q["relevant"]):
        continue
    kept.append({k: q[k] for k in ("id", "lang", "query", "relevant", "answerable") if k in q})
with open(os.path.join(out, "dataset.jsonl"), "w", encoding="utf-8") as f:
    for q in kept:
        f.write(json.dumps(q, ensure_ascii=False) + "\n")
print(f"text-only subset: {len(kept)} questions")
PY

run() { # build name corpus dataset
  local home="$tmp/home-$2-$1"
  mkdir -p "$home"
  ln -s "$ORAG_MODEL_DIR" "$home/models"
  ORAG_HOME="$home" "$1" eval retrieval --corpus "$3" --dataset "$4" \
    --out "$tmp/$2.json" > "$tmp/$2.md"
}
run "$v01" v01-seed eval/corpus/seed eval/datasets/seed.jsonl
run "$cur" cur-seed eval/corpus/seed eval/datasets/seed.jsonl
run "$v01" v01-subset "$tmp/subset/corpus" "$tmp/subset/dataset.jsonl"
run "$cur" cur-subset "$tmp/subset/corpus" "$tmp/subset/dataset.jsonl"

python3 - "$tmp" "$min_subset" <<'PY'
import json, sys
tmp, min_subset = sys.argv[1], int(sys.argv[2])
failed = False
for name in ("seed", "subset"):
    old = json.load(open(f"{tmp}/v01-{name}.json"))
    new = json.load(open(f"{tmp}/cur-{name}.json"))
    o = next(s for s in old["strategies"] if s["strategy"] == "hybrid")
    n = next(s for s in new["strategies"] if s["strategy"] == "hybrid")
    print(f"\n{name}: {n['queries']} questions ({old['embedding_model']} / {new['embedding_model']})")
    print("| metric | v0.1.0 | current |\n|---|---|---|")
    for m in ("recall_at_10", "mrr_at_10", "in_context"):
        print(f"| hybrid {m} | {o[m]:.3f} | {n[m]:.3f} |")
    if name == "subset" and n["queries"] < min_subset:
        print(f"subset has {n['queries']} questions, fewer than {min_subset}")
        failed = True
    for m in ("recall_at_10", "mrr_at_10"):
        if n[m] + 1e-9 < o[m]:
            print(f"REGRESSION: hybrid {m} {n[m]:.3f} < v0.1.0 {o[m]:.3f}")
            failed = True
    # v0.1.0 reports no per-question ranks; the paired list uses the answer
    # context (top context_chunks), which both reports carry.
    lost = sorted(set(n["missed_in_context"]) - set(o["missed_in_context"]))
    won = sorted(set(o["missed_in_context"]) - set(n["missed_in_context"]))
    print(f"in context only in v0.1.0 ({len(lost)}): {', '.join(lost) or '-'}")
    print(f"in context only in current ({len(won)}): {', '.join(won) or '-'}")
sys.exit(1 if failed else 0)
PY
echo "no regression against v0.1.0"
