# Changelog

All notable changes to ORAG are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/). Every change merged to `main`
bumps the version (see `.docs/orag-decisions.md`, D-017).

## [0.2.0-alpha.9] - 2026-10-07

- Web page: a *Belgeler* section lists the documents of the selected collection (file name, status, chunk count, size, warnings and errors), 50 at a time with *Daha fazla göster*, and deletes a document after a confirmation; the collection's document count updates. The list follows the selected collection and shows a new upload while it is indexed. No API change.

## [0.2.0-alpha.8] - 2026-10-07

- Web page: collections. A *Koleksiyon* section selects the collection to work in (with its document count; the choice is remembered in the browser), creates a new one and deletes the selected one after a confirmation (the `default` collection cannot be deleted). Uploads and questions use the selected collection only, so a question no longer searches documents uploaded for another topic; before, the page always used the default collection. No API change.

## [0.2.0-alpha.7] - 2026-10-07

- Built-in web page (D-021): `orag serve` also serves a small page on `ui_bind` (default `http://127.0.0.1:2442`, `"off"` disables it). *Dosya yükle* uploads a TXT/Markdown/DOCX/PDF file to the default collection and follows its indexing; *Sorgu yap* streams the answer, then shows the sources and an abstention notice. Plain HTML/CSS/JS embedded in the binary, `Content-Security-Policy: default-src 'self'`, nothing loaded from the network.
- New config key `ui_bind` (loopback only, bound before the models load; a busy port stops startup). `GET /v1/version` reports it with the real port. stdout gets a second line, `orag ui on http://...`, after the unchanged `orag listening on ...` line.
- The API now accepts exactly the page's own origins (`http://<ui address>`, `http://localhost:<ui port>`); every other origin still gets `403 forbidden_origin`. With `ui_bind = "off"` no origin is accepted, as before.

## [0.2.0-alpha.6] - 2026-10-06

- Project icon: a sickle (*orak*, the name's origin) on a cream rounded square. `assets/orag-icon.svg` is the source; `assets/orag-icon-512.png` and `assets/orag-icon-256.png` are renders of it for places that need a bitmap (GitHub social preview, organization avatar, the v0.3 desktop app), written by `scripts/render-icon.sh` (`--check` fails if they are stale). The README shows the icon at the top, and release tarballs include `assets/orag-icon.svg` so the packaged README shows it too.

## [0.2.0-alpha.5] - 2026-10-06

- `abstained` is also `true` when the model refuses in its own words: the first sentence says, without a citation, that the sources do not answer (`Verilen kaynaklarda ... belirtilmemiştir.`, `The sources do not mention ...`), even if cited background follows. Before, only the exact refusal sentence counted. On the answer sets: unanswerable questions flagged 4 of 4 (before 1 of 4), answerable ones 0 of 15. The answer text is unchanged.
- The refusal check needs a word for the sources in that first sentence (`kaynak`, `belge`, `source`, `document`), so a negative fact (`Anayasada ölüm cezası bulunmamaktadır.`) is an answer; a dot after a digit (`3. madde`) ends no sentence, and a citation right after the period belongs to it.
- `docs/api.md` response examples show the fingerprint of the fake models under encoding 2.

## [0.2.0-alpha.4] - 2026-10-06

- Query terms of three or more characters are FTS5 prefix terms, so a Turkish suffix no longer hides a match: `resmi dil` finds `resmî dili`, `mirasa` finds `miras`. Query side only (`LexicalQuery`), no reindex. Hybrid answer context: new inflection set 0.833 → 0.900, constitution set 0.950 → 1.000, seed set unchanged at 1.000 (MRR@10 0.939 → 0.903). Details in `.docs/benchmarks/2026-10-06-lexical-query.md`.
- Numbers are never prefix terms (`104` must not find `1040`); only terms with a letter are.
- Fixes from the review: the repeated-line guard strips one list marker followed by a space, so dotted dates and thousands (`01.02.2016 …`, `1.000 TL …`) no longer make three lines look alike; `orag eval answers` treats a dot between digits as part of the number (`600` is not found in `1.600`); an FTS index built by a newer release is refused instead of rebuilt down; seeded samplers are built per answer instead of cached per seed; `normalize::fts_query` delegates to `LexicalQuery::EXACT`.
- New retrieval set `eval/datasets/anayasa-cekim-tr.jsonl`: 30 questions asked in word forms the constitution text does not use.
- `orag eval retrieval` reports the lexical query mode and takes a hidden `--lexical-query exact|prefix:<n>` for experiments. Leaving Turkish question words out of the query was measured too: no change, not adopted.

## [0.2.0-alpha.3] - 2026-10-06

- The full-text (lexical) index is versioned separately from the embedding space. Schema 2 adds a `meta` table that records the lexical version; when it differs, `Store::open` rebuilds the FTS index from the stored chunks in one transaction (logged as `lexical index built`). A lexical change (normalizer, indexed text) no longer causes `409 reindex_required` or re-embedding.
- Embedding-space fingerprint encoding 2 no longer hashes the normalizer version. Spaces stored with an encoding-1 fingerprint of the same model and chunker (0.1.x, 0.2.0-alpha.1/2) are still accepted, so their vectors keep serving; fingerprints are not rewritten.
- Each chunk's FTS text is now the full heading path plus the body, both stored in `chunks` (before: the embedder's trimmed breadcrumb, which is not stored and could not be rebuilt). Lexical version 3.
- Lexical ranking breaks BM25 ties by chunk id, so the promoted lexical leader is deterministic.
- Fixes to 0.2.0-alpha.2 from its review: `orag eval answers` defaults to the served sampler (`dry`), not `greedy`; expected texts match on word boundaries (`600` is not found in `1600`); a probe set without answerable probes is an error; the samplers are built once per profile and reset between answers (DRY scans the vocabulary when built); the `length` check after the budget uses argmax and is skipped for a cancelled answer; the answer-engine tests moved to `retrieval/answer/tests.rs`.
- The startup lexical rebuild waits for another process's write lock like a migration (up to 300 s) instead of failing after the 5 s busy timeout.
- **Upgrade note:** schema 2 is applied at first start (with the usual automatic backup); the FTS index is then rebuilt once. Collections that 0.2.0-alpha.1/2 answered with `409 reindex_required` after the normalizer change serve again without re-uploading; checked on a copy of a 372-chunk database from 0.2.0-alpha.1. Older binaries refuse schema 2.

## [0.2.0-alpha.2] - 2026-10-06

- Answers end with a `finish_reason` (`stop`, `length`, `repetition`) in the JSON response and the SSE `done` event, so a client can tell a cut answer from a complete one; `docs/api.md` also states that `abstained` needs the exact refusal sentence.
- Repeated-line guard: an answer that writes the same line (list numbers and markers ignored, at least 12 characters) three times in a row is stopped with `finish_reason: repetition`. On an anonymized court decision the 4B model used to repeat one bullet until the 1024-token cap. A line that recurs between other lines (the same finding under several defendants, a table separator) never trips it. `length` is reported only when the answer would have gone on.
- Answers are sampled with DRY in front of argmax instead of plain greedy: still deterministic, no copy loops. Chosen with the new answer evaluation: on the court decision greedy covered 0.857 of the expected facts and looped once, DRY 1.000 without loops; on the constitution all profiles covered 1.000 (`.docs/benchmarks/2026-10-06-answer-sampler.md`).
- `orag eval answers --corpus DIR --dataset FILE --sampler greedy,dry,presence,qwen[:seed]`: runs the full answer path per sampler profile and reports expected-fact coverage, refusals, length and repetition stops, distinct-line ratio and latency. New probe sets `eval/datasets/answers-ceza-tr.jsonl` and `answers-anayasa-tr.jsonl` over PDF corpora (`eval/corpus/ceza`, `eval/corpus/anayasa-pdf`).
- `orag eval retrieval` and `orag eval answers` also index PDF and DOCX corpus files, through the production parsers: the loop above only appeared with `pdf_oxide` text, not with pypdf text of the same file.

## [0.2.0-alpha.1] - 2026-10-06

- Hybrid retrieval always puts the top hit of the lexical list and of the dense list first, before the rest in RRF order. Plain RRF dropped a chunk that only one retriever found, even at rank 1: on a PDF of the Turkish constitution, "Anayasaya göre resmî dili nedir?" had Article 3 at lexical #1 and dense #39 and the model answered that the sources did not say. Scores (`rank_score`) are unchanged, so they no longer always decrease along `sources`.
- Lexical normalizer v2: a circumflex on a, i, u is dropped (`resmî`, `millî`, `kâğıt`, `Ûmit` match `resmi`, `milli`, `kağıt`, `Umit`); other letters keep it (`fête`).
- New evaluation set `eval/datasets/anayasa-tr.jsonl` (20 answerable, 2 unanswerable Turkish questions) over `eval/corpus/anayasa/tr-anayasa.txt`, the text of the Turkish constitution. Hybrid, before → after: recall@5 0.800 → 0.900, recall@10 0.900 → 0.950, MRR@10 0.762 → 0.833, in answer context 0.900 → 0.950; the seed set is unchanged. Known miss: ana-001 (`resmi dil` vs `resmî dili`, no stemming).
- The `reindex_required` message and the API error table name the embedding space (model, chunker or normalizer), not only the model.
- **Upgrade note:** the normalizer version is part of the embedding-space fingerprint, so every collection indexed before this version answers `409 reindex_required`; pinning the previous model does not help. Re-upload the documents (see `docs/api.md`, `reindex_required`).

## [0.1.0] - 2026-10-05

- First usable release: offline RAG service for TXT, Markdown, DOCX and PDF with hybrid retrieval, grounded streamed answers with citations, evaluation harness
- Documentation: `README.md` (requirements, configuration, offline model install, run, backups, upgrades, limits, evaluation, versioning) and `docs/api.md` (every route, error code, limit, config key, the SSE event contract and `reindex_required` recovery in v0.1), with responses captured from the dev server.
- `.github/workflows/release.yml`: on `v*` tags or manual dispatch, builds macOS and Linux (ubuntu-22.04) release binaries, checks them (`check-llama-build.sh --release`, `check-binary-deps.sh --release-floor`) and uploads `orag-<version>-<target>.tar.gz` (the executable bit kept) with the licenses, `THIRD-PARTY-LICENSES.md` (`scripts/third-party-licenses.sh`: every crate built into the binary, without proc-macro crates, plus llama.cpp's vendored C/C++ libraries; a crate without a license file gets its authors and the standard text), README, CHANGELOG and API docs. It publishes nothing.
- `scripts/check-benchmark-gate.sh`: the D-003 gate needs a PASS row at 100k x 1024 (k 50, 50 queries, 250 ms target) on both references for the release candidate, the version before the docs-only release bump (here 0.1.0-alpha.31; `scripts/check-benchmark-gate.sh 0.1.0-alpha.31`). Results: macOS (M5 Max) p95 103.5 ms, Linux (GitHub runner, x86-64, 16 GB) p95 225.3 ms; the Linux margin is thin and the 16 GB/8-core references follow after v0.1.0.
- `.docs/benchmarks/v0.1-seed-eval.md`: the first D-007 data point (seed set, default models): hybrid recall@10 1.000, every label in the answer context; first token in about 0.2 s, then about 110–130 tokens/s decoding on an M5 Max. The seed set is too small to choose models; v0.2's 200-question set decides.
- The `vector-scale` workflow also prints the runner's CPU and memory to the log.

## [0.1.0-alpha.31] - 2026-10-05

- Split the 1 243-line `tests/api.rs` into one `api` test binary with modules (`support`, `system`, `documents`, `query`, `binary`, each with explicit imports). The same 43 tests and assertions; test ids now carry the module path (`query::query_returns_grounded_answer_json`), so `--exact` filters need it.
- Linux reference measurement recorded in `.docs/benchmarks/vector-scale.md`: GitHub ubuntu-22.04 runner (x86-64, 16 GB), 0.1.0-alpha.29, 100k x 1024 p95 152.2 ms, PASS.

## [0.1.0-alpha.30] - 2026-10-05

- `orag eval retrieval` ignores hidden corpus files (AppleDouble `._name` files macOS leaves on USB drives, dotfiles, also with non-UTF-8 names) and directories named like documents; a label that names a hidden file says so.
- Owner decision recorded under D-003: the v0.1.0 vector-scale gate runs on the development machine (macOS) and the `vector-scale` workflow runner (Linux), both on the release candidate; the 16 GB M-series laptop and an 8-core x86-64 box follow after v0.1.0, and release notes say where the promise was measured.

## [0.1.0-alpha.29] - 2026-10-05

- DOCX and PDF uploads (multipart only; JSON text with a .docx/.pdf name or format is 400 "upload as multipart"). The signature is checked before anything is stored, on a blocking thread: a PDF must start with `%PDF-`; a DOCX must be a ZIP of at most 10 000 entries whose `[Content_Types].xml` (UTF-8 or UTF-16) declares a Word main part, so a renamed .xlsx/.pptx/.odt, also one that embeds a .docx, is refused. One detection rule set (extension first, then content type) for multipart and JSON; other document types stay 415.
- Under `orag serve` every DOCX/PDF is parsed in a child `orag __parse` process: own process group, killed as a group while still unreaped (no pid-reuse race), 120 s deadline, 64 MiB result cap, stderr drained and logged (2 KiB), a watchdog that `_exit`s an orphaned child; on Linux a 4 GiB address-space cap, `MALLOC_ARENA_MAX=2`, `oom_score_adj` 1000, and the child is `/proc/self/exe`, so an in-place upgrade cannot swap the parser. A crash or hang fails that one document, never the service.
- Shutdown during a parse stops the child through the worker's own shutdown signal; the job stays running and resumes once after restart (tested end to end, including that shutdown is prompt). The child ignores SIGTERM/SIGINT/SIGHUP, so systemd's control-group kill or `pkill orag` cannot make a parse look like a crash. Failure attribution: crash signals blame the file, a parser panic is a rejected file, SIGKILL (OOM killer, operator) and error exits are host problems.
- `orag serve` and parser children share one `_exit` helper; text and Markdown parsing no longer copy the document for line endings; the binary end-to-end tests are split into `tests/cli.rs` and `tests/isolation.rs` with shared helpers.

## [0.1.0-alpha.28] - 2026-10-05

- DOCX parser (zip + roxmltree): headings from styles (with `w:basedOn` inheritance and Strict OOXML), paragraphs, list items (direct and style numbering, numId 0 = off), tables (one row per line, `|` escaped), text boxes once, tracked deletions skipped; the main part and styles are found through the package relationships. A broken styles part is a `styles_unreadable` warning, not a failure.
- PDF parser (pdf_oxide =0.3.78, no `ort`): page text as paragraphs, words hyphenated at line ends joined (a one-letter prefix such as "e-fatura" keeps its hyphen), `ocr_required` for pages without a text layer, `extraction_failed` for unreadable pages; no pages is an error.
- Hostile input is bounded before it can hurt the process: per-part decompressed size (also when the ZIP header lies), XML nesting depth checked by a non-recursive scan (deep XML used to overflow the stack and abort), XML node count, namespace declarations (thousands made parsing quadratic), PDF page count, extracted text and a time limit between pages. Part names in errors are quoted and shortened.
- Nothing calls the parsers yet: DOCX/PDF uploads open in 23b together with isolated parsing, signature checks and the JSON rules. Test fixtures come from `scripts/make-fixtures.sh` (pandoc, cupsfilter).

## [0.1.0-alpha.27] - 2026-10-04

- Embedding release gate, after an external review: the bilingual check uses the sections the chunker emits; the plan's one-sentence probe (which the 0.6B model fails: English question 0.380 for the Turkish policy vs 0.393 for an unrelated English sentence) stays as a diagnostic that the gate runs separately and never fails on; a new dense check requires each cross-language seed question's evidence within rank 3 among topic-near distractors in both languages. The limitation is recorded under D-005.
- `orag eval retrieval` reports `in context (top 8)` per strategy and the ids that miss it; `--require-in-context ID,...` fails when hybrid leaves any of them out. The release gate requires the three cross-language seed questions, so one subgroup cannot hide in the mean recall.
- Checked on Metal and on CPU (Linux arm64 in Docker): all real-model tests pass; cosines differ by at most ~0.007 and one dense rank swaps, so no gate requires a top-1 rank.
- `.github/workflows/vector-scale.yml`: manual job that runs the D-003 benchmark on GitHub's ubuntu-22.04 runner as the Linux reference; it refuses to run unless the runner is x86-64 with at least 15 GB, is limited to 60 minutes, and writes the machine and results to the job summary.
- **Versions:** this fix takes alpha.27, so 23a/23b produce alpha.28/29 (Task 24 is still 0.1.0). This supersedes the alpha.23 entry's numbering.

## [0.1.0-alpha.26] - 2026-10-04

- `scripts/fetch-model-pack.sh <preset> <dir>` builds offline packs for qwen3-embedding-0.6b-q8_0, qwen3.5-4b-q4_k_m and qwen3-4b-instruct-2507-q4_k_m from pinned Hugging Face revisions. Model and LICENSE are both verified by SHA-256 via a .part file that is deleted on mismatch, and the manifest is written last, so an unfinished pack has none.
- `scripts/release-model-check.sh` (release gate) builds the release binary, checks its dependencies, requires all 3 real-model tests to run and pass with the shipped generation model (an ORAG_TEST_GENERATION_MODEL override is ignored), and fails when seed hybrid recall@10 is below 0.9 (new `orag eval retrieval --min-recall-at-10`). It cleans up on failure and can be run again.
- `scripts/check-binary-deps.sh <binary> [--release-floor]` allows only system libraries, plus a glibc/libstdc++ floor (Linux) or macOS 14 (minos); finding no GLIBC versions is an error. CI now checks the debug and the release binary on both images.
- Fix: a process that loaded a real model aborted at exit on macOS (llama.cpp Metal `GGML_ASSERT([rsets->data count] == 0)`), because the embedder's and generator's worker threads were never joined. Dropping them now closes the queue and waits for the worker (a worker panic is logged), a generation stops at its next step however its caller ends, and if a task that did not stop in time still holds a model, `orag serve` exits with `_exit` instead of running llama.cpp's teardown.
- Verified on Apple Silicon with the real packs: release gate passed (seed eval: dense recall@10 1.000, hybrid recall@10 1.000), both generation models answer in Turkish with citations and abstain when unsupported, and `orag serve` answered and exited cleanly. Qwen3.5-4B stays the default (no D-007 fallback needed).

## [0.1.0-alpha.25] - 2026-10-04

- `orag eval vector-scale [--chunks 100000] [--dimensions 1024] [--queries 50] [--work-dir DIR]`: synthetic dense-search latency benchmark for the D-003 gate (warm p95 < 250 ms at 100k × 1024); exits non-zero when the gate fails and never touches ORAG_HOME.
- The benchmark cannot pass by measuring nothing: every search must return k hits, sizes are bounded (1-1 000 000 chunks, 20-10 000 queries), an all-zero random vector is drawn again, "insert s" counts SQLite inserts only, and --work-dir puts the throwaway database on the disk that is measured (Linux /tmp can be tmpfs).
- Recorded in .docs/benchmarks/vector-scale.md: 100k × 1024 p95 99.9 ms (PASS) and 300k p95 294.0 ms (informational) on an Apple M5 Max; the two reference machines (M-series 16 GB, x86-64 Linux 16 GB) are still to be measured before v0.1.0.
- Release builds link on macOS again: thin LTO dropped Rust's `__isPlatformVersionAtLeast`, which llama.cpp's Metal code needs, so build.rs links clang's runtime archive explicitly (no new dynamic dependency) and reruns when the toolchain moves; CI now builds the release profile on macOS.

## [0.1.0-alpha.24] - 2026-10-04

- `orag eval retrieval --corpus DIR --dataset FILE [--out FILE]`: indexes a corpus into a throwaway database and compares lexical, dense and hybrid retrieval (recall@5/10, MRR@10, label-based nDCG@10, p50/p95 latency); Markdown to stdout, JSON to a new --out file. It never touches ORAG_HOME's database, and with --dev-fake-models it reads no config.
- Bilingual seed corpus (4 documents) and dataset (16 questions: Turkish, English, cross-language, accentless, unanswerable) with eval/README.md for the 200-question target set.
- Mistakes are errors, not zero scores: every label must match a chunk of its document (parsed text, no Markdown syntax), and an empty corpus, a failed or empty file, or two identical files stop the run with the file name; .md/.markdown/.txt are matched in any case. The dataset and --out are checked before the model loads.
- eval loads the same embedder as serve (app::load_embedder); tempfile is now a runtime dependency.

## [0.1.0-alpha.23] - 2026-10-04

- Cancellation tests wait for events instead of fixed times: they wait for the generation permit to come back (it is released after the last token is counted), and the abandoned JSON query is dropped mid-generation. A macOS CI runner had failed a follow-up query that waited for a whole slow answer; the follow-up now only checks that a new stream starts.
- **Versions:** this out-of-band fix takes alpha.23, so Tasks 20-22 produce alpha.24-26 and 23a/23b produce alpha.27/28 (Task 24 is still 0.1.0). This supersedes the alpha.3 entry's numbering.

## [0.1.0-alpha.22] - 2026-10-03

- `orag serve`: config.toml is read once at startup; prints exactly one stdout line `orag listening on http://<addr>` (bind = "127.0.0.1:0" picks a free port, reported by /v1/version); an exclusive orag.lock refuses a second instance on the same home; interrupted jobs resume.
- The port is bound before the models load, so a busy port fails at once; SIGINT/SIGTERM handlers are installed before the listening line; the HTTP drain (10 s, now inside server::serve) and the worker/blocking tasks (5 s) are bounded, so shutdown cannot hang.
- `orag backup <DEST>`: a read-only, consistent copy that is safe while the server runs; a missing database (for example a mistyped ORAG_HOME) is an error and nothing is created, the live database is never migrated, and an existing DEST is never overwritten.

## [0.1.0-alpha.21] - 2026-10-03

- POST /v1/collections/{id}/query: JSON answer or SSE stream (sources, tokens, then done or one error event), with sources, validated citations, abstention and a per-query trace.
- One generation slot: a query waits at most 120 s, then gets 429 busy with Retry-After; queued and running queries get 503 shutting_down, never a truncated 200.
- A disconnected client (JSON or SSE) stops generation and frees the slot; a client that stops reading loses its stream after 30 s.
- Unknown collection and embedding-model mismatch are checked before and again after the wait, so they stay 404/409 statuses instead of a 200 stream with an error event.
- A prompt the generator refuses after budgeting is reported as a model error, not as the user's invalid input; HTTP and engine share one question validation.

## [0.1.0-alpha.20] - 2026-10-03

- Collections (list, create, delete), document upload (JSON text or multipart file), cursor-paginated listing, get/delete, and job status endpoints.
- Uploads: content type, collection and model compatibility are checked before one of 4 upload slots is taken; the body has 60 s; a multipart file is streamed and stopped as soon as it passes max_document_mb.
- One set of JSON-text format rules (SourceFormat::detect_text) with whitespace-trimmed extensions, so padded names and dotfiles such as 'rapor.xlsx ' or '.html' are refused like any other unsupported type.
- Bad path and query values are JSON invalid_input errors (ApiPath/ApiQuery); an unsupported JSON media type is 415 unsupported_media_type.

## [0.1.0-alpha.19] - 2026-10-03

- HTTP server foundation: loopback only, no authentication (D-013); Host must be a loopback name with an optional digits-only port; Origin is checked against allowed_origins after normalization, and cross-site browser requests without an Origin are refused.
- JSON errors everywhere, including unknown routes (not_found), wrong methods (method_not_allowed) and a missing JSON content type (415 unsupported_media_type); busy() adds Retry-After.
- GET /v1/health and GET /v1/version (with schema version and the effective config). serve() starts the app's own shutdown, so requests waiting for a slot or a body get 503 shutting_down.

## [0.1.0-alpha.18] - 2026-10-03

- Hybrid retrieval (lexical, dense, RRF) with a per-query trace; lexical search keeps working after an embedding model change.
- Answer engine: token-budgeted context (two candidates per slot, skipped chunks counted in the trace), streamed sources, tokens and a summary with validated [n] citations; empty retrieval abstains without calling the model.
- Source text is quoted line by line (every line-break character) so it cannot forge a source header or the question; a refusal is detected in either language only when the answer is the refusal itself.
- A question too long for the embedding or answer model is InvalidInput; a consumer Break is final, so no Done follows.

## [0.1.0-alpha.17] - 2026-10-03

- Ingestion worker: parse, chunk and embed outside the write lock, then publish atomically; restart recovery requeues interrupted jobs.
- A document deleted at any point while its job runs is discarded, not failed; a reindex-required collection fails the job before any embedding work.
- A panicking job is failed instead of left running; shutdown stops a running job between embedding batches and leaves it for the next start; the worker also stops if the shutdown sender is dropped.
- The chunker config is validated against the embedding model at startup (IngestContext::check) and kept unchanged when the model has room; user-facing job errors never leak internal details.

## [0.1.0-alpha.16] - 2026-10-03

- LlamaGenerator: streaming generation on a worker thread with back-pressure and cancellation; prompts as ChatML, ChatML no-think, the model's own GGUF template, or a plain transcript.
- Prompt injection guard: the text of every guarded special token in the vocabulary (control and user-defined, 2+ non-blank characters) is broken in message content; the prompt is tokenized whole, as the model was trained, and refused if a guarded token did not come from template markup.
- A GGUF chat template is applied with placeholder slots, its trimming detected per call, and tried with three conversation layouts at load before the KV cache is allocated. context_tokens may be at most 4x the trained context.
- UTF-8 reassembly of token bytes; reasoning (<think>) spans are hidden only for formats that reason, including templates that open <think> in the prompt; other control tokens are never streamed.

## [0.1.0-alpha.15] - 2026-10-03

- llama.cpp backend (llama-cpp-2 =0.1.158, default features off: no OpenMP) behind the `llama` feature, on by default; Metal on Apple Silicon.
- LlamaEmbedder: rejects at load a manifest whose dimensions differ from the model's output width, whose max_tokens exceed the trained context, or that requires an EOS the model lacks. Guarantees exactly one trailing EOS when required; overlong input is an error, never truncated.
- Document and query text are tokenized with parse_special = false, so text such as `</s>` cannot become a control token. Blank text is rejected by every embedder, including the fake.
- scripts/check-llama-build.sh checks the real build: Metal backend built on Apple Silicon, no OpenMP, llama.cpp objects target macOS 14. CI fetches (and caches) the pinned 1.2 MB stories260K fixture for llama smoke tests.

## [0.1.0-alpha.14] - 2026-10-03

- Offline model packs: `orag models import|list|verify`. A pack is a manifest (`orag-model.toml`), GGUF weights and a license file.
- Import copies into a private staging directory, hashes the weights as they are written, and renames into place; a checksum mismatch installs nothing. Staging left by an interrupted import is removed after 24 h.
- Manifests are validated: plain file names that are not the manifest itself, distinct weights and license files, lowercase SHA-256, the role matching its section.
- Model lookups accept only plain ids and require the manifest id to equal its directory name; `models list` reports broken packs as warnings instead of failing.
- `orag version` reports the supported schema version.

## [0.1.0-alpha.13] - 2026-10-03

- Embedding spaces: a collection binds to a space on first ingest; collections with the same fingerprint share one vec0 table. A collection with no chunks follows a model change instead of answering reindex_required; a space no collection uses is dropped.
- Atomic publish of chunks, FTS rows and unit-length vectors in one IMMEDIATE transaction; results of a deleted document or stale job are discarded.
- Lexical (FTS5/BM25) and dense (sqlite-vec, exact k-NN) search scoped to the collection; the dense query is normalized so scores are true cosine; k is capped at 4096 (MAX_SEARCH_K).
- get_chunks takes the collection id and never returns another collection's chunks.
- Deleting a document or collection removes chunks, FTS and vector rows, jobs and now-unused sources in one transaction; the default collection is protected.

## [0.1.0-alpha.12] - 2026-10-03

- Inference boundaries: Embedder, Generator and VectorIndex traits; the RAG core never sees llama.cpp types.
- Embedding-space fingerprint: SHA-256 over a fixed-order, length-prefixed encoding with chunker and normalizer versions; golden-value test; new descriptor fields fail to compile until encoded.
- l2_normalize returns an error for zero or non-finite vectors and computes the norm in f64. Deterministic fakes enforce the same token, context and output limits as real models; fixtures match whole words.

## [0.1.0-alpha.11] - 2026-10-03

- Collections: create/list/get with NFC names, case- and Turkish-I-insensitive uniqueness, invisible characters refused.
- Documents: content-addressed source snapshots (SHA-256), duplicate detection per collection, a re-upload of a failed document retries it with a new job; NFC filenames reduced to their base name, hidden and bidi characters refused; strict page limits (1-200).
- Jobs: durable queue with IMMEDIATE claims, failure recording that reports whether it applied, and restart recovery.

## [0.1.0-alpha.10] - 2026-10-02

- SQLite store: WAL, sqlite-vec 0.1.9 (pinned), orag application id; refuses other apps' databases and non-UTF-8 paths.
- Migrations: one transaction each with BEGIN IMMEDIATE and a re-read version (safe under concurrent opens), foreign keys off with foreign_key_check, newer schema refused.
- Backups: complete pre-upgrade copy per upgrade (removed if nothing changed), VACUUM INTO via a synced temp file so a partial file never appears; chunks must share their document's collection.

## [0.1.0-alpha.9] - 2026-10-02

- Reciprocal Rank Fusion (k = 60) of lexical and dense rankings; ranks count distinct chunks, ties break by chunk id.
- Citation extraction: raw-text scan for [n], [n, m], [a-b], [^n] markers (1-3 digits), skipping only closed code fences (also inside lists and quotes) in linear time; tuned to never miss a real citation, accepted limitations documented in the module.

## [0.1.0-alpha.8] - 2026-10-02

- Structure-aware chunker: heading breadcrumbs, token budget with exact counting, overlapping windows cut at word and grapheme boundaries

## [0.1.0-alpha.7] - 2026-10-02

- Text and Markdown parsing into document blocks with strict UTF-8 validation and format detection

## [0.1.0-alpha.6] - 2026-10-02

- Turkish-aware lexical normalization and injection-safe FTS5 query builder, checked against a real FTS5 table

## [0.1.0-alpha.5] - 2026-10-02

- Configuration file (`config.toml`) read once at startup: loopback bind, 1-10 MB document limit, model names

## [0.1.0-alpha.4] - 2026-10-02

- Cargo workspace, `orag` binary with `--version` and `version` command, CI, version scripts

## [0.1.0-alpha.3] - 2026-10-02

Plan fixes from Muse's review, a Fable 5.1 second opinion and repeated `/code-review` rounds.

- **Versions:** every task's version moves up by one (this step is alpha.3): Task N produces `0.1.0-alpha.(N+3)` through Task 22, and 23a/23b produce alpha.26/27. This supersedes the alpha.2 entry's numbering.
- **llama.cpp binding:** the llama-cpp-2 calls of Tasks 12–13 are stated as verified (crate source file:line, compiled during planning); the stale `token_eos` note is removed. Default features are off, so OpenMP is off and there is no dynamic `libgomp` (D-001). Task 13 no longer accepts each token twice (`sample` already accepts). There is no `metal` feature: llama-cpp-2 enables Metal itself on Apple Silicon.
- **Build and platform guards:** `scripts/platform-floors.sh` holds every supported floor (macOS 14; glibc 2.35, GLIBCXX_3.4.30, CXXABI_1.3.13, GCC_7.0.0) and the version helpers. `.cargo/config.toml` forces `MACOSX_DEPLOYMENT_TARGET=14.0` for every build. `scripts/check-llama-build.sh` checks the real build output (Metal backend built, no OpenMP, every llama.cpp archive at macOS ≤ 14). `scripts/check-binary-deps.sh` takes strict arguments, checks the final macOS binary's minimum version, and on the ubuntu-22.04 CI and release image (`--release-floor`) the Linux symbol-version floor. CI and Linux releases build on Ubuntu 22.04.
- **Fingerprint:** hand-written, length-prefixed, fixed-order encoding, exhaustive over the descriptor's fields, with a golden test (D-009).
- **Retrieval test:** the collection-scope dense-search test uses skewed data that would catch post-top-k filtering (pre-filtering verified against sqlite-vec 0.1.9).
- **Capacity and shutdown:** a query waits at most 120 s for the generation slot, then gets `429 busy` with `Retry-After: 5`. Server shutdown has one signal (`AppState::begin_shutdown()` / `subscribe_shutdown()`), shared with the ingest worker; every waiting point (generation slot, upload body) goes through `AppState::until_shutdown` and ends with `503 shutting_down`. A consumer's Break is final (no `Done` follows): a JSON query stopped by shutdown gets `503`, never `500` or a truncated `200`; an SSE stream ends with one `event: error`, while an already finished answer is still delivered. On shutdown a full SSE buffer gets 2 s per event, and graceful shutdown drains for at most 10 s. Parser isolation (23b) keeps its own process-wide stop, `isolate::request_shutdown()` in `app.rs`.
- **Task 23** is split into 23a (parsers, unused until wired) and 23b (format detection, isolation, signature checks and JSON rules in one merge), so `main` never accepts a binary upload without all of them.
- **Release docs:** system requirements (macOS 14+, glibc 2.35+), provisional-retrieval note, measured generation speed, and accurate manual recovery for `409 reindex_required` (the default collection cannot be deleted; pinning the old model works only when nothing else in the space changed).

## [0.1.0-alpha.2] - 2026-10-02

- Plan fixes from the Codex review: parser output is capped in the parent; the Linux memory cap is checked and verified by a Linux test; an explicit JSON `format` can no longer bypass filename rules; an end-to-end test covers interrupted isolated ingestion; at most 4 uploads are accepted at once (`429 busy`).
- Resume procedure inspects the current branch and uncommitted work before switching.
- Uploads whose whole body is not received within 60 s get `408 upload_timeout`; the JSON filename-first rule lands in Task 17; parser stderr is drained instead of cut off; the parent's output buffer never grows past its cap; a refused memory limit is an internal error; the test-only parse delay works in debug builds only; Task 16's dev-dependency commands are fixed; only the upload route accepts large bodies (64 KiB elsewhere); D-010 documents the JSON format rules.
- Documented the per-service resource budget, upgrade notes for default-model changes, and partial indexing; the Task 22 fallback now updates both model defaults.
- Version numbers in the plan shifted by one; Task 1 now produces `0.1.0-alpha.3`.

## [0.1.0-alpha.1] - 2026-10-02

- Owner decisions recorded: no API authentication (loopback only, Host/Origin checks), formats TXT/MD/DOCX/PDF, 5 MB document limit (configurable 1-10 MB), `config.toml` read once at startup (D-010, D-011, D-013, D-019).
- Development workflow rules (D-020): step branches merged into `main` without PRs, orchestrator-only git, `/code-review` before every push, mandatory tests and changelog per step.
- Plan: verified DOCX (zip + roxmltree) and PDF (pdf_oxide) ingestion task replaces the PDF spike; versions renumbered.

## [0.1.0-alpha.0] - 2026-10-01

- Architecture decision record (`.docs/orag-decisions.md`) reached by Claude/Codex review.
- v0.1 implementation plan (`.docs/plans/2026-10-01-orag-v0.1-implementation-plan.md`).
