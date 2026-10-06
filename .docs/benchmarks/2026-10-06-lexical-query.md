# Lexical query mode (0.2.0-alpha.4)

`orag eval retrieval --lexical-query <mode>` with `qwen3-embedding-0.6b-q8_0`,
Apple M5 Max, 2026-10-06. `prefix:<n>` turns every query term of at least
`n` characters into an FTS5 prefix term (`"dil"*` also finds `dili`,
`dilinin`). A tried "stop" mode (Turkish question words such as `nedir`,
`hangi`, `kaç` left out) is not listed: it gave exactly the `exact` numbers on
all three sets (BM25's IDF already gives those words almost no weight), and
with `prefix:3` it lost `ana-006`, so it was dropped.

`anayasa-cekim-tr.jsonl` (new, 30 questions) asks in word forms the text does
not use (`mirasa` for `miras`, `kanaatini` for `kanaatlerini`, `dil` for
`dili`).

Hybrid strategy, `in context` = evidence among the 8 chunks of the answer
context:

| mode | inflection set: recall@10 / MRR@10 / in context | constitution set | seed set |
|---|---|---|---|
| exact (0.2.0-alpha.3) | 0.867 / 0.729 / 0.833 | 0.950 / 0.833 / 0.950 | 1.000 / 0.939 / 1.000 |
| **prefix:3** | **0.900 / 0.742 / 0.900** | **1.000 / 0.840 / 1.000** | 1.000 / 0.903 / 1.000 |
| prefix:4 | 0.867 / 0.719 / 0.867 | 0.950 / 0.814 / 0.900 | 1.000 / 0.903 / 1.000 |
| prefix:5 | 0.867 / 0.719 / 0.867 | 0.950 / 0.814 / 0.900 | 1.000 / 0.903 / 1.000 |

Lexical strategy alone, in context: inflection 0.767 → 0.800, constitution
0.950 → 1.000 (`ana-001`, "resmi dil" vs "resmî dili", is found at last),
seed unchanged at 0.857.

## Decision

`prefix:3`. Three characters is the shortest Turkish root that matters here
(`dil`); `prefix:4` misses it and loses `ana-006`. Cost: seed MRR@10 0.939 →
0.903 (one answer moves from first to second place; every seed question stays
in context) and noise from unrelated words with the same start (`dil` also
matches `dilekçe`).

Answer level (`orag eval answers`, served `dry` sampler): constitution
unchanged (coverage 1.000); court decision 1.000 → 0.952 because one answer
(`c-02`) described the first ground of reversal without its article number
(`230`); the context order changed, the content did not.

Still missed with every mode: `cek-19` (`angaryaya` → `Angarya`: the dense
side does not find it either), `cek-25`, `cek-30` (number words: `üye sayısı`
vs `onbeş üyeden`, `milletvekillerinin sayısı` vs `altıyüz milletvekilinden`).
Consonant changes (`kitap`/`kitabı`, `amaç`/`amacı`) are out of reach for
prefixes; a stemmer in a second FTS column is the next candidate, judged on a
larger set.
