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

Files ending in `.md`, `.markdown`, `.txt`, `.pdf` or `.docx` (any letter case)
are indexed; PDF and DOCX go through the same parsers as `orag serve`.
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

## Datasets

| corpus | dataset | questions | purpose |
|---|---|---|---|
| `eval/corpus/seed` | `seed.jsonl` | 14 answerable, 2 unanswerable | harness check, release gate |
| `eval/corpus/anayasa` | `anayasa-tr.jsonl` | 20 answerable, 2 unanswerable | Turkish recall on a long legal text: inflected forms (`resmi dil` vs `resmî dili`), the circumflex, accentless typing |
| `eval/corpus/anayasa` | `anayasa-cekim-tr.jsonl` | 30 answerable | questions in word forms the text does not use (`mirasa` for `miras`, `kanaatini` for `kanaatlerini`) |

```bash
orag eval retrieval --corpus eval/corpus/anayasa --dataset eval/datasets/anayasa-tr.jsonl
```

`tr-anayasa.txt` is the text layer of the Constitution of the Republic of
Türkiye as published by the Grand National Assembly
(<https://cdn.tbmm.gov.tr/TbmmWeb/Anayasa/anayasa_2018.pdf>, downloaded
2026-10-06), extracted page by page with pypdf and left unedited, page numbers
and footnote marks included. Article 31 of Law No. 5846 (FSEK) allows statutes
to be reproduced freely.

`ana-001` (`resmi dil`, evidence `resmî dili`) was missed until query terms
became prefix terms in 0.2.0-alpha.4 (D-006).

## Answer evaluation

`orag eval answers` runs the whole answer path (retrieval, prompt, generation
with the installed models) and scores what a user sees. It compares sampler
profiles side by side and never touches `$ORAG_HOME/orag.db`.

```bash
orag eval answers --corpus eval/corpus/ceza --dataset eval/datasets/answers-ceza-tr.jsonl \
  --sampler greedy,dry,presence,qwen:1 --out answers.json
```

Probe format (JSONL): `id`, `lang`, `query`, `answerable`, and for answerable
probes `expect`: texts a complete answer contains, compared after lexical
normalization (case, `İ`/`ı` and the circumflex do not matter). `|` separates
alternatives (`600|altıyüz`).

| column | meaning |
|---|---|
| coverage | mean share of `expect` texts found, over answerable probes |
| complete | answerable probes with every `expect` text found |
| refused answerable | answerable probes answered with the refusal sentence |
| refused unanswerable | unanswerable probes answered with the refusal sentence (wanted) |
| length stops | answers cut by `max_output_tokens` (`finish_reason: length`) |
| repetition stops | answers stopped by the repeated-line guard (`finish_reason: repetition`) |
| distinct lines | mean share of unique non-empty lines per answer |

Refusals are counted from `abstained` (see `docs/api.md`): the refusal
sentence, or an uncited first sentence that says the sources do not answer.

| corpus | dataset | probes |
|---|---|---|
| `eval/corpus/ceza` | `answers-ceza-tr.jsonl` | 7 answerable (3 lists), 2 unanswerable |
| `eval/corpus/anayasa-pdf` | `answers-anayasa-tr.jsonl` | 8 answerable (2 lists), 2 unanswerable |

Answer corpora are PDFs, parsed by the production parser (`pdf_oxide`): the
copy loop this set was built for appeared only with that text, not with pypdf
text of the same files. `tr-yargitay-1cd-2016-347.pdf` is a browser print of an
anonymized decision of the Court of Cassation, 1st Criminal Chamber
(2015/3839 E., 2016/347 K., 02/02/2016); `anayasa-pdf/tr-anayasa.pdf` is the
TBMM PDF the retrieval corpus was extracted from.
Article 31 of Law No. 5846 (FSEK) allows court decisions to be reproduced
freely. Anonymization removed the names, so one phrase repeats many times:
the text that made the 4B model loop.
