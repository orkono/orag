# ORAG — Architecture Decision Record

> **Status:** Accepted for v0.1 (2026-10-01; owner revisions D-010, D-013, D-019, D-020 on 2026-10-02)\
> **Supersedes:** conflicting parts of `orag-architecture-design-notes.md`.
> That document remains the long-term vision; this file records what is
> decided now, why, and what evidence would reopen each decision.\
> **Process:** Reached by structured review between Claude (Opus 5.5) and
> Codex (gpt-6-astra) in five rounds (two design rounds, three plan-review rounds ending in "APPROVE WITH FIXES", all fixes applied), with upstream facts checked on
> 2026-10-01. Items marked *provisional* must be confirmed by the evaluation
> harness before the release named in their "Revisit" line.

## Product framing

ORAG is a downloadable, fully offline RAG service. A desktop application
(Tauri) will later sit on top of the same HTTP API so non-technical users can
use it. The service is the product core; the GUI is a client.

---

## D-001 — "Single binary" means one service executable per platform/backend

**Decision.** The headless service ships as one self-contained executable
per supported target:
- `aarch64-apple-darwin` with Metal (llama.cpp's CMake builds it by default
  on Apple; no ORAG feature flag; CI checks the built backend), macOS 14+
  (`MACOSX_DEPLOYMENT_TARGET=14.0` forced in `.cargo/config.toml`; every llama.cpp
  object and the final binary are checked);
- `x86_64-unknown-linux-gnu` (CPU);
- later `x86_64-pc-windows-msvc` (CPU, then Vulkan/CUDA variants).

Everything ORAG builds (llama.cpp, SQLite, sqlite-vec) is linked statically.
The only dynamic dependencies are the platform's base libraries: glibc,
libstdc++ and libgcc_s on Linux (built against glibc 2.35 / GLIBCXX_3.4.30 /
CXXABI_1.3.13), and on macOS system libraries under `/usr/lib` and system
frameworks; `check-binary-deps.sh` enforces this. OpenMP is off on
every target, so there is no dynamic `libgomp`/`libomp` (`check-llama-build.sh`).
Model files live outside the executable. GPU drivers are documented system
prerequisites, not bundled.

**Why.** CUDA needs a matching driver and, depending on linking, runtime
libraries; no single artifact can cover every accelerator. The desktop app is
one installer containing several executables (GUI + service sidecar); the
"single executable" promise applies to the headless service.

**Verification.** Release CI inspects every artifact with `otool -L` / `ldd`
and fails on any non-system dynamic dependency.

## D-002 — One inference runtime in v1: llama.cpp via `llama-cpp-2`

**Decision.** `llama-cpp-2 = "=0.1.158"` provides embeddings, generation and
(later) reranking and OCR-VLM inference. No mistral.rs, no ONNX Runtime (`ort`),
no Candle in v1. All inference sits behind the `Embedder` / `Generator` traits.

**Why.**
- GGUF has the widest model availability; CPU/Metal/CUDA/Vulkan coverage is
  the most mature, which matters for "everyone can use it" desktop hardware.
- One runtime avoids multiplying packaging, binary size and regression work.
- PaddleOCR-VL support is merged in llama.cpp (release b8110); the equivalent
  mistral.rs PRs (#2320/#2356, #2319) were still open on 2026-10-01. One
  community port measured llama.cpp ~2.7× faster per page than mistral.rs for
  PaddleOCR-VL on their hardware (a data point, not a guarantee).

**Cost accepted.** C/C++ toolchain (CMake) in the build, FFI, and binding API
churn. Mitigation: exact version pin, all FFI confined to `infer::llama`.

**Revisit if.** A required model architecture is unsupported in llama.cpp but
supported elsewhere, or measured latency on reference hardware misses targets.

## D-003 — Storage: SQLite + FTS5 + sqlite-vec `=0.1.9` (stable, exact pin)

**Decision.** One SQLite database (`~/.orag/orag.db`, WAL mode) holds
collections, documents, immutable source snapshots, jobs, chunks, the FTS5
lexical index and sqlite-vec `vec0` dense vectors. sqlite-vec is pinned to the
stable `=0.1.9` (brute-force exact search). The `0.1.10-alpha.*` ANN series is
not used: a caret requirement `^0.1.9` would resolve to it, hence the exact pin.
Dense search sits behind the `VectorIndex` boundary so an ANN backend can be
added later without touching retrieval.

**Scale promise (v1).** 100,000 chunks per collection on the reference laptop
(16 GB RAM, Apple M-series and an x86-64 8-core) with warm dense
search-only p95 < 250 ms at 1024 dims, f32. 300k is a stretch benchmark;
millions are explicitly outside v1. Quantization or ANN is introduced only when
measurement shows the target is missed.

**v0.1.0 measurement (owner decision, 2026-10-05).** The release gate runs on
the development machine (Apple M5 Max) as the macOS reference and on GitHub's
ubuntu-22.04 runner (x86-64, 4 vCPU, 16 GB; exact k-NN uses one core) as the
Linux reference. The 16 GB M-series laptop and an 8-core x86-64 box remain the
promise's hardware; until they are measured, release notes state the promise
as measured on those two machines. A failing later measurement takes the
"Revisit" path and corrects the published promise in the next release.

**Revisit if.** The `vector-scale` benchmark (plan Task 21) misses the target.

## D-004 — Backups use SQLite, never `cp`

**Decision.** `orag backup <dest>` runs `VACUUM INTO`. A live `cp orag.db`
can omit WAL content and is documented as unsafe. Bundled SQLite must be
≥ 3.51.3 (WAL-reset corruption fix); a test asserts this.

## D-005 — Retrieval in v1 is deterministic hybrid; no decision models

**Decision.** Every query: FTS5 BM25 top 50 + dense top 50, both constrained to
the collection **before** top-k → Reciprocal Rank Fusion (k = 60) → the top
hit of each list (lexical #1, dense #1) moved to the front → context filled in
that order up to the generator's token budget (max 8 chunks). No reranker, no
query rewriting, no adaptive routing in v0.1.

**Leader promotion (0.2.0-alpha.1).** Plain RRF dropped a chunk that only one
retriever found, even at rank 1: on the PDF of the Turkish constitution,
"Anayasaya göre resmî dili nedir?" had Article 3 at lexical #1 and dense #39
(1/61 + 1/99 ≈ 0.0265), below the 8th context chunk (lexical #21, dense #6,
≈ 0.0275), so the model said the sources did not answer. Promoting each list's
#1 costs at most two context slots and keeps scores unchanged. On
`eval/datasets/anayasa-tr.jsonl` it raised hybrid recall@5 from 0.800 to 0.900
and MRR@10 from 0.762 to 0.825; the seed set did not change.

**Abstention.** Hard abstain (no model call) only when both candidate lists
are empty. The `abstained` flag also marks a model answer that is the refusal
sentence or whose first sentence, uncited, says the sources do not answer
(0.2.0-alpha.5: on the answer sets 4 of 4 unanswerable questions flagged,
before 1 of 4; 0 of 15 answerable ones).
Otherwise the generation prompt instructs the model to answer only from the
numbered sources and to reply with a fixed refusal sentence when they are
insufficient. If retrieval found chunks but none fits the
generator's context window, that is a capacity error, not "not found". No cosine-threshold rejection (a strong lexical match can have a
weak dense score, so thresholding defeats hybrid retrieval).

**Generation (0.2.0-alpha.2).** Answers are sampled with DRY (multiplier 0.8,
base 1.75, allowed length 2, last 256 tokens) in front of argmax:
deterministic, and it stops the copy loops greedy decoding fell into on
repetitive source text (an anonymized court decision). A repeated-line guard
ends an answer that writes the same line (list numbers and markers ignored)
three times in a row; `finish_reason` (`stop`, `length`, `repetition`) tells clients
whether an answer is complete. Chosen with `orag eval answers` over the
production PDF parser (`.docs/benchmarks/2026-10-06-answer-sampler.md`);
Qwen's sampling preset varied by seed and gave no gain. Known limit of the
guard: list numbers are ignored, so three lines in a row that differ only by
a leading number that is content (`1. Ceza Dairesi`, `2. Ceza Dairesi`, ...)
with the same rest are taken for a loop. The leader-promotion
MRR above (0.825) is promotion alone; with the circumflex fold it is 0.833.

**Decision models.** Jev is excluded: only hosted access was verified, no
downloadable weights. Nimble is deferred: it is 9B with Python MLX/PyTorch
tooling, not a cheap laptop router. Small decision/classifier models are
reconsidered only after the evaluation harness shows a failure class that
deterministic routing cannot fix.

**Known limitation (measured 2026-10-04, Task 22).** On very short texts the
0.6B embedder can rank by language before topic: for the English question
"What is the return period for products?" a one-sentence Turkish returns
policy scored 0.380 and an unrelated one-sentence English text 0.393. On the
sections the chunker actually emits (heading + paragraph) the evidence ranks
clearly first or second, also against topic-near same-language distractors,
and every cross-language seed question reaches the answer context. Hybrid
fusion cannot recover such a miss when the languages share no terms. So
short FAQ-style documents in one language queried in another are the weak
spot. v0.1 mitigations: headings stay in chunk text, the release gate
requires each cross-language seed question in the answer context (not just a
mean recall), and the one-sentence probe is kept as a non-blocking
diagnostic. The same checks on CPU (Linux arm64 in Docker) and Metal
(Apple Silicon) differ by at most ~0.007 in cosine, yet one dense rank swaps
(x-002: first on Metal, second on CPU); that is why the gates require the
answer context, never a top-1 rank. Later candidates, each to be judged on a broader short-text
benchmark first: a larger embedding model, multilingual reranking, query
translation. The query prefix stays as Qwen recommends (English instruction,
no document prefix).

**Revisit.** v0.4, using evaluation results.

## D-006 — Turkish-aware lexical normalization without a custom tokenizer

**Decision.** FTS5 indexes a *normalized shadow text*; original text is kept
in `chunks.text` and used for embeddings and citations. One Rust function
normalizes both indexed text and queries: NFKC → drop invisible format
characters (Default_Ignorable: soft hyphen, zero-width space, joiners, BOM,
variation selectors; `unicode61` would split a word at them) → fold `İ`, `I`,
`ı` to `i` (symmetric, so English and Turkish casing both match; the ı/i
distinction is deliberately lost for recall) and drop a U+0307 dot directly
after `i` → drop a U+0302 circumflex after `a`, `i`, `u` (Turkish `resmî`,
`millî`, `kâğıt` match the usual spellings; `ê`, `ô` keep it; normalizer v2,
0.2.0-alpha.1) → private-use characters (PDF glyph codes) become spaces → Unicode
lowercase, `ß` → `ss` → NFC (so `ı` + U+0301 and `í` are the same text).
Tokenizer: `unicode61 remove_diacritics 0` — ç/ş/ğ/ö/ü are preserved. User
queries are turned into quoted OR-terms, never passed to FTS5 as raw syntax: a
term is a run of letters, numbers and combining marks with at least one letter
or number, and FTS5 splits the inside of each quoted term with the index's own
tokenizer, so the query side never mirrors SQLite's character tables. Only the
first 4096 characters of a query are used (a term cut there is dropped), at
most 32 terms. The Unicode tables behind the normalizer (`unicode-normalization`,
pinned with `=`, and the toolchain's std) are part of it: bumping them bumps
`LEXICAL_VERSION`. `tests/lexical_fts5.rs` checks the contract against a
real FTS5 table.

**Known limit.** `unicode61` does not segment scripts written without spaces
(CJK, Thai): a run is one token, so lexical search only finds the whole run,
not a word inside it. Dense retrieval still covers these texts; a segmenting
or trigram tokenizer is a later, evaluation-driven decision.

**Lexical version (0.2.0-alpha.3).** The FTS index is derived data. Each
chunk's FTS text is `chunker::lexical_text`: the full heading path and the body,
both stored in `chunks` (not the embedder's trimmed breadcrumb, which is not
stored). `LEXICAL_VERSION` covers that text and this normalizer; the database
records the version it was built with (`meta.lexical_version`, schema 2), and
`Store::open` rebuilds the index from `chunks` in one transaction when it
differs: no 409, no re-embedding. Versions: 1 (0.1.x), 2 (circumflex fold), 3
(full heading path). A current index is checked without the write lock. Lexical
ties order by chunk id, so leader promotion (D-005) is deterministic.

**Prefix terms (0.2.0-alpha.4).** Query terms of 3+ characters are FTS5
prefix terms (`"dil"*` finds `dili`), query side only (`LexicalQuery`, no
reindex). On the new inflection set hybrid answer context went from 0.833 to
0.900, on the constitution set from 0.950 to 1.000 (`resmi dil` is found);
seed stays at 1.000 with MRR@10 0.939 → 0.903
(`.docs/benchmarks/2026-10-06-lexical-query.md`). Dropping Turkish question
words from the query changed nothing and was not adopted.

**Deferred.** Stemming and an accent-folded secondary field are added only if
the evaluation set (which includes accentless-typing queries) shows a gain.
The circumflex fold passed that test: on `anayasa-tr.jsonl` hybrid answer
context went from 0.900 to 0.950 (`milli marsimiz nedir` now finds `Millî
marşı`), the seed set did not change. Its cost is precision, as with ı/i:
`kâr`/`kar`, `hâlâ`/`hala`, `âlem`/`alem` become one term, and so do French
`sûr`/`sur`, `dû`/`du` and `île` with Turkish `ile`; BM25's IDF damps the very
common ones. Still open: consonant changes (`amaç`/`amacı`), number words
and accentless typing of ç/ş/ğ/ö/ü.
Both spellings are never concatenated into one field (it distorts term
frequencies).

## D-007 — Provisional default models

All are *provisional* until the v0.1 evaluation harness confirms them on the
bilingual set; decision deadline is the v0.2 release.

| Role | Default | Challengers | License |
|---|---|---|---|
| Embedding | Qwen3-Embedding-0.6B (GGUF Q8_0, 1024-d, last-token pooling) | multilingual-e5-large-instruct, BGE-M3 | Apache-2.0 |
| Generation | Qwen3.5-4B Q4_K_M, text-only, non-thinking | Qwen3-4B-Instruct-2507 (fallback), Gemma 4 E4B | Apache-2.0 |
| Reranker (v0.4+, off by default) | Qwen3-Reranker-0.6B on 20 candidates | bge-reranker-v2-m3 | Apache-2.0 |

Notes:
- Qwen3-Embedding: the terminal special token must appear exactly once through
  the artifact's tokenizer configuration; do not blindly append EOS. Query
  side uses the instruction prefix; documents use none.
- Qwen3.5 uses a hybrid Gated DeltaNet/attention architecture supported
  upstream; if the pinned binding fails on it, switch the default to
  Qwen3-4B-Instruct-2507 (conventional attention).
- EmbeddingGemma ranks well on Turkish benchmarks but uses Gemma terms rather
  than Apache/MIT; it may be evaluated but not shipped as default.
- Non-thinking mode is enforced by ORAG's own prompt renderer (manifest
  `prompt_format = "chatml-nothink"`: ChatML with an empty `<think></think>`
  prefill, as the official template does with `enable_thinking=false`);
  llama.cpp's legacy native template formatter cannot do this. Any residual
  `<think>…</think>` span is still stripped from the stream as a safety net.
- Untrusted text is stripped of `<|…|>` control markers before rendering,
  because prompts are tokenized with special-token parsing enabled.

## D-008 — Ingestion is asynchronous and durable

**Decision.** `POST …/documents` stores an immutable source snapshot and
returns `202 Accepted {document_id, job_id}`. A single in-process worker
executes jobs from the `jobs` table. On startup, `running` jobs return to
`queued` (restart recovery). Parsing, chunking and embedding happen outside
the write transaction; **publish** is one transaction that inserts chunks, FTS
rows and vectors and flips the document to `ready` — readers never see a
half-indexed document. Publish re-checks that the document and collection
still exist (delete/ingest race). Identical content (SHA-256) in the same
collection returns the existing document (`duplicate: true`). Document and job ids
are never reused (`AUTOINCREMENT`), and failing or publishing a job only
succeeds while that job is still `running`, so a stale job can never touch a
later upload. One `orag serve` process owns a data directory at a time
(exclusive `orag.lock`).

Deferred: automatic retries, cancellation endpoint, progress percentages
beyond job status.

## D-009 — Embedding-space fingerprint

**Decision.** Each collection is bound to one embedding space whose
fingerprint is SHA-256 over: model id, model file SHA-256, pooling, query prefix,
document prefix, dimension, normalization flag, maximum input tokens,
trailing-EOS rule, chunker version and the encoding version. Vectors from different spaces are never mixed. If the
configured embedder's fingerprint differs from a collection's space, queries
and ingestion into that collection fail with `409 reindex_required`;
uploads are checked before they are accepted (and again by the worker).
Re-indexing (build new space, switch atomically) is v0.2 scope.
The fingerprint uses a hand-written, length-prefixed, fixed-order encoding
with a golden test, so no dependency feature (such as serde_json
`preserve_order`) can silently change it.

**Encoding 2 (0.2.0-alpha.3).** Encoding 1 also hashed the lexical normalizer
version, which never reaches the embedder, so the circumflex fold of
0.2.0-alpha.1 sent every collection to `409 reindex_required` although its
vectors were valid. Encoding 2 drops it; the lexical index has its own version
(D-006). A space stored with an encoding-1 fingerprint is still accepted when
that fingerprint is exactly this descriptor and chunker with normalizer 1 or 2
(`SpaceDescriptor::accepts`), checked on query and ingestion. Stored
fingerprints are not rewritten: two rows of the same model could otherwise
collide on `UNIQUE`. A new collection binds a new encoding-2 space; existing
spaces and vector tables stay as they are, so one model can then have two
space rows and vec0 tables (compatible vectors, never mixed).

## D-010 — Supported formats: TXT, Markdown, DOCX, PDF (owner decision, 2026-10-01)

**Decision.** v0.1 accepts exactly `.txt`, `.md`/`.markdown`, `.docx` and
`.pdf`; everything else is rejected with `415 unsupported_format`. Both binary
formats are parsed in pure Rust so the single-binary property (D-001) holds:
- **DOCX:** `zip` + `roxmltree`. Headings come from paragraph style *names*,
  which stay English in localized Word (Turkish `Balk1` is named `heading 1`).
  The parser also handles lists, tables, content controls, text boxes (once,
  not their `mc:Fallback` copy) and tracked changes (deletions and move
  sources skipped). Each XML part is capped at 64 MB decompressed as a
  ZIP-bomb guard.
- **PDF:** `pdf_oxide =0.3.78`, page by page; its optional ONNX dependency
  stays disabled.
- Under `orag serve`, DOCX/PDF parsing runs in a child process with a 120 s
  deadline. A parser crash (e.g. stack overflow) or hang fails only that
  document. On Linux the child is also capped at 4 GiB of address space
  (`MALLOC_ARENA_MAX=2`) and, where the host allows it, is the OOM killer's
  first choice; if the cap cannot be installed the parse is refused as an
  internal error. The parent reads at most 64 MiB
  of parser output on every platform; macOS lacks only the memory cap.
- JSON text uploads: filename rules come first. Known unsupported document
  types (`.xlsx`, `.html`, …) are 415 whatever `format` says; a `format` that
  contradicts a supported extension is 400 `invalid_input`; `.pdf`/`.docx`
  names or formats are 400 "upload as multipart" (until DOCX/PDF uploads
  land in Task 23b they are 415 like any unsupported type). Without a supported
  extension (`Toplantı 12.10.2026`, `notes.v2`) the given `format` applies,
  else plain text.

This replaces the earlier plan for a PDFium vs `pdf_oxide` spike. During
planning, `pdf_oxide` extracted a Turkish PDF intact (`İ ı ş ğ ç ö ü`).
Uploads are signature-checked before storage: `%PDF-` for PDF, a ZIP header
for DOCX. Binary formats must arrive as multipart uploads. MuPDF is excluded
(AGPL or commercial license). Page-number provenance in citations is v0.2.

## D-011 — OCR / Document AI is not in v1's supported scope

**Decision.** PDF pages without a text layer are not OCRed. The document is
still indexed from its readable pages and carries an
`ocr_required: page(s) … have no extractable text` warning. Externally OCRed
searchable PDFs are accepted. A later optional OCR build (PaddleOCR-VL GGUF via
llama.cpp `mtmd`, plus a layout model) is a separate executable variant,
because a model pack cannot switch on a compile-time feature.

## D-012 — Offline-first provisioning; no implicit network

**Decision.** The service never contacts the network implicitly: no telemetry,
no cloud fallback, no automatic model download in v0.1. Models are installed
from **model packs** (directory: `orag-model.toml` manifest + GGUF + license
file) with `orag models import <dir>`, which verifies SHA-256. A repository
script (`scripts/fetch-model-pack.sh`) builds packs from pinned Hugging Face
revisions on a connected machine, verifying against the SHA-256 that the
Hugging Face API publishes for each LFS file. An in-app downloader is v0.3
(desktop) scope, and it will only run on explicit user action.

## D-013 — Local API access: no authentication, loopback only (owner decision, 2026-10-01)

**Decision.** The API has **no authentication**: no token and no
`Authorization` header. Documents are uploaded and queried by calling the API
directly. Protection comes from reachability:
- the server binds only `127.0.0.1` / `::1` (non-loopback binds are refused at
  startup);
- requests whose `Host` is not a loopback name or address are rejected with
  `403 forbidden_host` (DNS rebinding);
- requests carrying an `Origin` header are rejected with `403 forbidden_origin`
  unless allow-listed (the desktop origin is added in v0.3; the built-in page's
  own origins since D-021), so web pages in a local browser cannot call it.

Request bodies are size-limited (D-019). Retrieved text is treated as
untrusted data inside the prompt, never as instructions.

**Accepted risk, stated precisely.** Loopback TCP has no per-user isolation.
Any process on the same machine can read and delete every indexed document:
other OS user accounts, containers using host networking, and local malware.
That includes a shared terminal server or a family computer with several
accounts. ORAG v1 targets single-user personal machines; README and
`docs/api.md` say this plainly. If multi-user hosts become a target, the
answer is a Unix domain socket with file permissions, or an opt-in token —
not a silent change.

## D-014 — API shape: first-class collections

**Decision.** A `default` collection is created automatically. Routes:

```text
GET    /v1/health
GET    /v1/version                                  (+ effective config)
POST   /v1/collections            GET /v1/collections
DELETE /v1/collections/{collection_id}
POST   /v1/collections/{collection_id}/documents   → 202 {document_id, job_id}
GET    /v1/collections/{collection_id}/documents   (paginated)
GET    /v1/collections/{collection_id}/documents/{document_id}
DELETE /v1/collections/{collection_id}/documents/{document_id}
GET    /v1/jobs/{job_id}
POST   /v1/collections/{collection_id}/query       (stream=true → SSE)
```

The collection is a foreign key and a mandatory retrieval constraint, applied
inside both lexical and dense search before top-k. SSE order: `sources` first,
then `token` events, then `done` (validated citations + trace) or `error`.
Out of scope: permissions, nesting, cross-collection queries.

## D-015 — Code layout: one package, modules not crates

**Decision.** A Cargo workspace containing one package `crates/orag` with a
library and the `orag` binary. Modules: `domain` (pure functions, no I/O),
`store`, `infer`, `ingest`, `retrieval`, `server`, `eval`, `cli`. Traits now:
`Embedder`, `Generator`, `VectorIndex`. `LexicalIndex` and `Reranker` traits
are added when a second implementation exists. The Tauri app joins the
workspace as `apps/desktop` in v0.3.

## D-016 — Desktop: Tauri v2 with the service as managed sidecar (v0.3)

**Decision.** The desktop app bundles the `orag` executable as a Tauri
`externalBin` sidecar, starts it on a free loopback port (its own `config.toml`),
supervises it (restart on crash, kill on exit — no orphans) and talks to it
over the public HTTP API. Crash isolation of native inference code is the
deciding factor over in-process embedding. Windows installers must include
the WebView2 offline installer.

## D-017 — Versioning

**Decision.**
- SemVer. Single source of truth: `[workspace.package] version` in the root
  `Cargo.toml`; every crate inherits it.
- **Owner requirement:** every change merged to `main` that changes behavior,
  code, schema or public docs bumps the version and adds a `CHANGELOG.md`
  entry (Keep a Changelog format). While building milestone M the version is
  `0.M.0-alpha.N`, N incrementing per merged change; the milestone release is
  `0.M.0`; fixes after it are `0.M.P`.
- The version is exposed by `orag --version` and `GET /v1/version`
  (`{version, api: "v1", schema_version, git_sha, config}`).
- The HTTP contract version (`/v1`) changes only on breaking API changes.
- DB schema version lives in `PRAGMA user_version`; migrations are ordered,
  transactional and embedded in the binary; a newer-than-known schema is
  refused; a backup is taken before destructive migrations.
- CI asserts that the top `CHANGELOG.md` version equals the Cargo version.

## D-018 — Evaluation before opinions

**Decision.** v0.1 ships an evaluation harness (`orag eval`) and a seed
bilingual corpus. The target set is ~200 human-checked TR/EN questions:
factual, cross-language, multi-document, identifiers/codes, tables,
accentless typing, unanswerable. Relevance is labeled as
`(document, exact supporting substring)` so labels survive chunker changes.
Metrics reported separately: recall@5/10, MRR@10, nDCG@10 per strategy
(lexical, dense, hybrid), and latency p50/p95. Answer-level metrics
(citation correctness, faithfulness, abstention) arrive with v0.2.

## D-019 — Configuration file, read once at startup (owner decision, 2026-10-01)

**Decision.** `$ORAG_HOME/config.toml` (default `~/.orag/config.toml`) is the
only source of runtime settings. On first start it is created with every key
commented out, so an install follows the built-in defaults of the running
version (e.g. the later 10 MB default) until the user uncomments a line. It is read **once** when `orag serve` starts and is never watched or
reloaded. After editing it, restart the application. Keys:

| Key | Default | Rule |
|---|---|---|
| `bind` | `127.0.0.1:7613` | loopback IP:port only |
| `ui_bind` | `127.0.0.1:2442` | loopback IP:port of the built-in page (D-021), or `"off"` |
| `max_document_mb` | `5` | integer 1–10; the next milestone raises the default to 10; values above 10 are not supported |
| `embedding_model` | `qwen3-embedding-0.6b-q8_0` | installed model pack id |
| `generation_model` | `qwen3.5-4b-q4_k_m` | installed model pack id |
| `log_level` | `info` | error, warn, info, debug, trace |

Unknown keys or invalid values stop startup with a message naming the file.

`ORAG_HOME` is the only environment variable; it locates the directory and
sets nothing else (debug builds also read a test-only parse delay; release
binaries ignore it). Uploads above the limit get `413 too_large` before they are
stored. `GET /v1/version` reports the settings in effect.

**Resource budget.** `max_document_mb` limits each document, not the
service. A 5 MB DOCX/PDF can expand a lot when decompressed, and the HTTP
body may be up to ~6× the limit (JSON escaping). Mitigations:
- only the upload route accepts document-sized bodies; every other route is
  limited to 64 KiB;
- at most 4 uploads are accepted at once (`429 busy` beyond that);
- an upload whose whole body is not received within 60 s gets
  `408 upload_timeout` and frees its slot, so a stalled client holds a slot
  for at most 60 s (a total deadline, ample for ≤ 60 MB over loopback). A
  local process that keeps re-opening stalled uploads can still keep uploads
  busy; on a single-user machine (D-013) that process is the user's own;
- a host that refuses the parser's memory limit fails the job as an internal
  error, never as a bad file;
- binary parsing is isolated with a deadline and output cap (D-010);
- on macOS there is no memory cap; the 120 s deadline and the output cap
  still apply;
- a query waits at most 120 s for the single generation slot, then gets
  `429 busy` (with `Retry-After: 5`); shutdown wakes waiting queries with
  `503 shutting_down`;
  waiting queries are served in arrival order, but a retry after a 429
  joins the end of the queue (accepted: one generation slot, single user);
- graceful shutdown drains open connections for at most 10 s; a full SSE
  buffer gets 2 s per event once shutdown starts;
- these limits (4 uploads, 60 s upload body, 120 s generation wait, 64 KiB,
  64 MiB of parser output, 10 s shutdown drain, 2 s SSE shutdown grace) are fixed
  constants by design: config.toml holds only the owner-chosen keys.

**Upgrades.** Because defaults are commented, a release that changes a
default model makes existing installs use it after the upgrade, and startup
fails if that pack is not installed; uncommenting `generation_model` (and
`embedding_model`) before upgrading keeps the current models. Any change to an
input of the embedding fingerprint (D-009: embedding model id or file SHA-256,
pooling, prefixes, dimension, normalization flag, token limit, trailing-EOS
rule, chunker or normalizer version, encoding version) makes existing
collections answer `409 reindex_required`; a new generation model needs no
reindex. Pinning `embedding_model` avoids the 409 only when the release
changed nothing else in that list. Release notes mark every such change as
**Upgrade note** and say whether pinning is enough.

**Partial indexing.** A `ready` document with an `ocr_required` or
`extraction_failed` warning is only partially indexed: the listed pages are
not searchable.

## D-020 — Development workflow (owner rules, 2026-10-01)

1. Only the orchestrating session runs git write commands. Subagents never
   commit, push, merge or branch.
2. Tasks run strictly in order, one at a time.
3. No worktrees and no pull requests. Each task gets a local branch
   `step/NN-<slug>` from `main`; it is merged with `--no-ff`, `main` is pushed,
   and the branch is deleted.
4. Every task bumps the version and adds a `CHANGELOG.md` entry (D-017).
5. Every task delivers its tests.
6. `/code-review high` runs before every push; findings are fixed first.
7. When undecided, ask a `claude-fable-5-1` subagent first, then gpt-6-astra
   (Codex) if needed.
8. If the usage limit is hit, wait and resume from the last merged step. A
   step is finished only when `main` has its merge commit
   (`merge: step NN <slug> (v<VERSION>)`) and CHANGELOG has its version;
   any other `step/*` branch is the interrupted task (plan P0 rule 8).
9. Tags are never created by the agent; they are handed to the owner.

The owner authorized commit, merge into `main` and push for this repository
only (2026-10-01). This repository-scoped grant overrides the owner's general
"never commit/push" policy here and nowhere else.

## D-021 — Built-in web page on a second loopback port (owner decision, 2026-10-07)

**Decision.** `orag serve` also serves a minimal page (choose, create or
delete a collection; upload a file to the selected collection and follow its
job; ask a question in the selected collection and stream the answer with its
sources) on a second loopback listener, `ui_bind` (default
`127.0.0.1:2442`, `"off"` disables it). Same process, same `AppState`: the
UI listener serves `GET /`, `/app.js`, `/app.css` plus the whole `/v1` API,
so the page calls the API on its own origin. The API listener does not serve
the page.

- **Origins (D-013).** Browsers send `Origin` on same-origin POSTs, so the
  page's origins must be allowed. `allowed_origins` holds exactly
  `http://<bound ui address>` and `http://localhost:<bound ui port>`, taken
  from the bound address (so also with port `0`); with the UI off it stays
  empty. Every other origin still gets `403 forbidden_origin`, and Host checks
  apply on both listeners. No credentials are added: the page has the same
  access as any local process.
- **Assets.** Plain HTML, CSS and one script, embedded with `include_str!`; no
  framework, build step or network resource (D-012). Served with
  `Content-Security-Policy: default-src 'self'` (no inline script),
  `X-Content-Type-Options: nosniff` and `X-Frame-Options: DENY`. Document text
  is untrusted and rendered with `textContent` only.
- **Startup and shutdown.** The UI port is bound before the models load, like
  `bind`; a busy port stops startup with an error naming `ui_bind`. So a
  second instance (another home) needs its own `ui_bind` or `"off"`, as it
  already needs its own `bind`. One shutdown signal stops both listeners
  within the existing 10 s drain.
- **Links.** A cross-site `GET` without `Origin` stays `403` for the API, but
  the page's static files (no data) may be opened from a link on another
  site. Only the page's own origins are trusted, including
  `http://localhost:<port>`: whatever answers there is a local process, which
  can already call the API directly (D-013).
- **Stdout (D-016).** The first line stays `orag listening on http://...`;
  `orag ui on http://...` follows only when the UI is on. The desktop sidecar
  reads only the first line; it should write `ui_bind = "off"` (or port `0`)
  into its own `config.toml` so it never competes for port 2442.

**Why a second port, not the API port.** A page on the API port would make
the API's own origin a trusted web origin for every client of `bind`
(including the desktop's fixed port); a separate listener keeps the allowed
origin tied to the page alone and lets users turn it off without touching the
API.

---

## Rejected or deferred, with reason

| Item | Status | Reason |
|---|---|---|
| mistral.rs as primary runtime | Rejected for v1 | D-002; second runtime doubles packaging; PaddleOCR PRs unmerged |
| ONNX Runtime (`ort`) / fastembed-rs | Deferred | Only needed for an optional OCR layout model |
| HNSW sidecar index | Deferred | Not needed at 100k; SQLite stays authoritative if added |
| sqlite-vec 0.1.10 ANN | Deferred | Alpha; DELETE cost regression noted upstream |
| SQLite `vec1` extension | Deferred | 0.7, "testing is insufficient" upstream |
| Jev | Rejected | No downloadable weights |
| Nimble | Deferred | 9B, Python tooling |
| OCR in v1 | Deferred | D-011 |
| Custom FTS5 tokenizer / stemming | Deferred | D-006; needs eval evidence |
| Electron | Rejected | Larger bundles; Tauri fits a Rust codebase |
| In-process engine in GUI | Rejected | No crash isolation for native inference |

## Sources (checked 2026-10-01)

- sqlite-vec releases: https://github.com/asg017/sqlite-vec/releases
- sqlite-vec vec0 metadata/partition keys: https://alexgarcia.xyz/sqlite-vec/features/vec0.html
- SQLite backup / WAL: https://sqlite.org/backup.html, https://sqlite.org/wal.html
- llama.cpp PaddleOCR-VL (PR #18825, b8110): https://github.com/ggml-org/llama.cpp/releases/tag/b8110
- paddleocr-vl-rs benchmarks: https://github.com/subin9/paddleocr-vl-rs
- llama-cpp-2: https://docs.rs/llama-cpp-2/0.1.158
- Qwen3-Embedding GGUF: https://huggingface.co/Qwen/Qwen3-Embedding-0.6B-GGUF
- Qwen3.5-4B: https://huggingface.co/Qwen/Qwen3.5-4B
- Nimble: https://github.com/bespokelabsai/nimble ; Jev: https://docs.typesafe.ai/introduction
- TR-MTEB: https://aclanthology.org/2025.findings-emnlp.471.pdf
- pdfium-render: https://docs.rs/pdfium-render ; pdf_oxide: https://github.com/yfedoseev/pdf_oxide
- Tauri sidecar: https://v2.tauri.app/develop/sidecar/
- ort linking: https://ort.pyke.io/setup/cargo-features
