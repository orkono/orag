# Retrieval evaluation

`orag eval retrieval` indexes a corpus into a throwaway database and compares
lexical, dense and hybrid retrieval on a labeled question set (D-018). It never
touches `$ORAG_HOME/orag.db`.

```bash
orag eval retrieval --corpus eval/corpus/seed --dataset eval/datasets/seed.jsonl --out eval-report.json
```

The Markdown table goes to stdout; `--out` also writes the report as JSON to a
new file (an existing file is refused before the run starts). Use
`--dev-fake-models` to run without installed models: the numbers then only
check the harness, not retrieval quality, and no config is read.

## Dataset format (JSONL)

One JSON object per line; blank lines are ignored.

| field | type | meaning |
|---|---|---|
| `id` | string | unique within the file |
| `lang` | string | language of the question (`tr`, `en`) |
| `query` | string | the question as a user would type it |
| `relevant` | array of `{document, contains}` | evidence for the answer; empty for unanswerable questions |
| `answerable` | bool | `true` needs at least one `relevant` label, `false` needs none |

- `document` is the corpus file name (for example `tr-kargo-politikasi.md`).
- `contains` is a short, unique piece of the document text that proves the
  answer. It is matched against the **parsed** text, so leave out Markdown
  syntax: write `timeout_seconds parametresi`, not `` `timeout_seconds` parametresi ``,
  and `14 gün`, not `**14 gün**`. Table rows keep their cells:
  `500 TL ve üzeri | Ücretsiz`. Whitespace differences (line breaks, repeated
  spaces) are ignored, and the text must sit inside one chunk.

Before any metric is computed, every label is checked: a label whose document
is not in the corpus, or whose text no chunk of that document contains, stops
the run with the question id. A typo is never reported as a retrieval miss.

Labels name a document and a text span, not a chunk, so they stay valid when
the chunker changes.

## Corpus

Files ending in `.md`, `.markdown` or `.txt` (any letter case) are indexed.
An empty corpus, a file that fails to index, or two files with identical
content (they would be stored as one document) stop the run with the file name.
Hidden files (names starting with `.`, such as the AppleDouble `._name`
files macOS leaves on USB drives) are ignored.

## Metrics

Computed over answerable questions at depth 10:

- **recall@5, recall@10:** share of labels found in the top 5 / 10 chunks.
- **MRR@10:** reciprocal rank of the first chunk that satisfies any label.
- **nDCG@10:** evidence coverage over labels (each label counts once), always in [0, 1].
- **p50 / p95 ms:** retrieval latency per query.

## The 200-question target set

The seed set (16 questions) checks the harness. The target set for model and
fusion decisions has about 200 questions and must cover:

- factual questions answered by one sentence,
- cross-language questions (Turkish question, English document, and the reverse),
- multi-document questions (labels in two or more documents),
- identifiers and codes (error codes, part numbers, paths),
- answers in tables,
- accentless typing (`istanbul ici`, `iade suresi`),
- unanswerable questions (no evidence in the corpus).
