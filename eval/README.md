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
| `category` | string, optional | `factual`, `cross_language`, `multi_document`, `identifier`, `table`, `accentless`, `inflection` or `unanswerable` (the last one exactly for unanswerable questions) |
| `split` | string, optional | `dev` (used to tune) or `holdout` (only to confirm) |
| `family` | string, optional | the documents a question belongs to; paraphrases of one fact share a family and a split. A family, and a piece of evidence, may sit in one split only (the loader refuses otherwise) |
| `require_all` | bool, optional | the question counts as found only when **every** label is found (needs two or more labels); otherwise any one label is enough |

Older sets have no `category` or `split`; the report groups them as
`uncategorized` and `unsplit`. Answer probes take the same three optional
fields.

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

- **recall@5, recall@10:** share of questions found in the top 5 / 10 chunks:
  any one label, or every label for `require_all` questions.
- **MRR@10:** reciprocal rank at which the question is answered: its best
  label, or for `require_all` questions the last label it needs (0 when one
  is missing).
- **nDCG@10:** for `require_all` questions, coverage of every label (each
  label counts once); otherwise the best label against an ideal of rank 1.
  Always in [0, 1].
- **in context:** share of questions found within the chunks the answer
  prompt can hold (`max_context_chunks`, 8), by the same rule.
- **p50 / p95 ms:** retrieval latency per query.

A set with categories or splits also gets "By category" and "By split"
tables. The JSON report lists every question with its label ranks
(`per_query`), so two runs can be compared question by question.

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

## The v0.2 set

`eval/corpus/v02` is built by `scripts/make-v02-corpus.sh` from the sources in
`eval/corpus-src/v02` (a fictional irrigation catalog printed to PDF, a pump
manual converted to DOCX, two related HR documents, warranty terms and a
plain-text price list, all written for ORAG) plus copies of the seed
documents, `tr-anayasa.txt` and the court decision. The constitution is in
it once, as text: a PDF copy would hold the same evidence under another name.
Questions and probes were written by `claude-opus-5-5` and audited row by
row by `gpt-6-sol`, models of two families, not people
(`eval/datasets/v02-audit.md`, step 49). Tune on `dev`, confirm on `holdout`.

```bash
orag eval retrieval --corpus eval/corpus/v02 --dataset eval/datasets/v02-retrieval.jsonl
orag eval answers --corpus eval/corpus/v02 --dataset eval/datasets/v02-answers.jsonl --sampler dry
```

## Datasets

| corpus | dataset | questions | purpose |
|---|---|---|---|
| `eval/corpus/seed` | `seed.jsonl` | 14 answerable, 2 unanswerable | harness check, release gate |
| `eval/corpus/anayasa` | `anayasa-tr.jsonl` | 20 answerable, 2 unanswerable | Turkish recall on a long legal text: inflected forms (`resmi dil` vs `resmî dili`), the circumflex, accentless typing |
| `eval/corpus/anayasa` | `anayasa-cekim-tr.jsonl` | 30 answerable | questions in word forms the text does not use (`mirasa` for `miras`, `kanaatini` for `kanaatlerini`) |
| `eval/corpus/v02` | `v02-retrieval.jsonl` | 175 answerable, 25 unanswerable | the v0.2 decision set (D-018): eight categories, `dev`/`holdout` splits |

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

`cek-19`, `cek-24` and `cek-30` reach the answer context since query words longer than
7 characters also add a 7-character prefix stem (step 51, D-006).

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
| expected facts | answerable probes whose answer holds every `expect` text (`expected_fact_rate`) |
| cited evidence | answerable, not refused probes where an `expect` text occurs in a chunk the answer **cites** (`cited_evidence_coverage`). Evidence coverage, not faithfulness: the cited chunk can hold the fact while the answer says something else |
| valid citations | answered (not refused) probes with at least one `[n]`, every one naming a source that was given (`valid_citation_rate`); an answer without any `[n]` is not validly cited |
| abstention P / R | refusals as a confusion matrix with "no answer in the corpus" as positive: precision = refused unanswerable / all refusals, recall = refused unanswerable / all unanswerable |
| coverage | mean share of `expect` texts found, over answerable probes |
| length stops | answers cut by `max_output_tokens` (`finish_reason: length`) |
| repetition stops | answers stopped by the repeated-line guard (`finish_reason: repetition`) |
| distinct lines | mean share of unique non-empty lines per answer |

A rate with nothing to measure (no answered probe, no unanswerable probe, no
refusal) is `–` in the table and `null` in the JSON, never 0. The JSON keeps
the older counts `complete`, `refused_answerable` and `refused_unanswerable`
(not in the table) and adds `finish_reasons` (`stop`, `length`, `repetition`),
the metrics per `category` and per `split` (`by_category`, `by_split`), and
each probe's `valid_citations`, `cited_evidence` and answer.

`--judge` (experimental, off by default) asks the local generator, per
answered answerable probe that cites a source, "Does the answer only state
what the cited sources say? Answer YES or NO.", showing each cited source
under the `[n]` the answer uses. The share of YES among the judged probes is
reported as `experimental.judge_yes_rate` (`null` when none was judged); a
judge that cannot run leaves the probe unjudged. No gate uses it: a model judging answers of
its own kind is a weak signal (D-018).

Refusals are counted from `abstained` (see `docs/api.md`): the refusal
sentence, or an uncited first sentence that says the sources do not answer.

| corpus | dataset | probes |
|---|---|---|
| `eval/corpus/ceza` | `answers-ceza-tr.jsonl` | 7 answerable (3 lists), 2 unanswerable |
| `eval/corpus/anayasa-pdf` | `answers-anayasa-tr.jsonl` | 8 answerable (2 lists), 2 unanswerable |
| `eval/corpus/v02` | `v02-answers.jsonl` | 40 answerable, 20 unanswerable |

Answer corpora are PDFs, parsed by the production parser (`pdf_oxide`): the
copy loop this set was built for appeared only with that text, not with pypdf
text of the same files. `tr-yargitay-1cd-2016-347.pdf` is a browser print of an
anonymized decision of the Court of Cassation, 1st Criminal Chamber
(2015/3839 E., 2016/347 K., 02/02/2016); `anayasa-pdf/tr-anayasa.pdf` is the
TBMM PDF the retrieval corpus was extracted from.
Article 31 of Law No. 5846 (FSEK) allows court decisions to be reproduced
freely. Anonymization removed the names, so one phrase repeats many times:
the text that made the 4B model loop.
