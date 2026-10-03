# Changelog

All notable changes to ORAG are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/). Every change merged to `main`
bumps the version (see `.docs/orag-decisions.md`, D-017).

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
