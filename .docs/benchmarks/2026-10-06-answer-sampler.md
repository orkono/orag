# Answer sampler choice (0.2.0-alpha.2)

`orag eval answers` with the default packs (`qwen3.5-4b-q4_k_m`,
`qwen3-embedding-0.6b-q8_0`), Apple M5 Max, Metal, 2026-10-06. The
repeated-line guard was on for every profile, so a loop shows as a
repetition stop instead of a 1024-token answer.

## Why PDF corpora

The first run used pypdf text of the same two documents. There greedy never
looped. Over the PDFs, which `orag serve` parses with `pdf_oxide`, greedy
looped on `c-05` (36 numbered copies of one line until the 1024-token cap
before the guard learned to ignore list numbers). The answer corpora are
therefore the PDFs: `eval/corpus/ceza`, `eval/corpus/anayasa-pdf`.

## Court decision (`answers-ceza-tr.jsonl`: 7 answerable, 3 of them lists; 2 unanswerable)

| sampler | coverage | complete | refused unanswerable | length stops | repetition stops | p50 ms | p95 ms |
|---|---|---|---|---|---|---|---|
| greedy | 0.857 | 6 | 0 | 0 | 1 | 1234 | 3623 |
| dry | 1.000 | 7 | 0 | 0 | 0 | 1488 | 3729 |
| presence | 1.000 | 7 | 0 | 0 | 0 | 1462 | 3871 |
| qwen:1 | 0.786 | 5 | 0 | 0 | 0 | 1612 | 2207 |
| qwen:2 | 0.857 | 6 | 0 | 0 | 0 | 1349 | 3385 |
| qwen:3 | 1.000 | 7 | 0 | 0 | 0 | 1201 | 2705 |

## Constitution (`answers-anayasa-tr.jsonl`: 8 answerable, 2 of them lists; 2 unanswerable)

| sampler | coverage | complete | refused unanswerable | length stops | repetition stops | p50 ms | p95 ms |
|---|---|---|---|---|---|---|---|
| greedy | 1.000 | 8 | 1 | 0 | 0 | 1264 | 1523 |
| dry | 1.000 | 8 | 1 | 0 | 0 | 1396 | 1680 |
| presence | 1.000 | 8 | 1 | 0 | 0 | 1693 | 2003 |
| qwen:1 | 1.000 | 8 | 0 | 0 | 0 | 1983 | 2532 |
| qwen:2 | 1.000 | 8 | 1 | 0 | 0 | 1927 | 2201 |
| qwen:3 | 1.000 | 8 | 1 | 0 | 0 | 1867 | 2303 |

Re-run after the review fixes (guard counts consecutive copies only; `length`
only when the answer would have gone on): greedy, dry and presence gave the
same coverage, completes and stops on both sets; p50 within 0.1 s.

## Decision

`dry` (DRY multiplier 0.8, base 1.75, allowed length 2, last 256 tokens,
then argmax). It fixes greedy's loop, keeps every expected fact, stays
deterministic (reproducible evaluations and answers), and is faster than the
presence penalty. The presence penalty subtracts from every token already
generated, including Turkish suffix tokens; DRY only penalizes extending an
n-gram that already occurred. Qwen's recommended sampling preset varies from
seed to seed (coverage 0.786–1.000 on the court decision) and gives no gain
here.

Limits: 15 answerable and 4 unanswerable probes over two documents. Several
unanswerable probes were declined in other words than the refusal sentence and
are not counted as refusals (step 29). Re-run on the 200-question set (D-018).
