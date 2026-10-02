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

**Decision.** The headless service ships as one statically linked executable
per supported target: `aarch64-apple-darwin` (Metal), `x86_64-unknown-linux-gnu`
(CPU), later `x86_64-pc-windows-msvc` (CPU, then Vulkan/CUDA variants). Model
files live outside the executable. GPU drivers are documented system
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

**Revisit if.** The `vector-scale` benchmark (plan Task 21) misses the target.

## D-004 — Backups use SQLite, never `cp`

**Decision.** `orag backup <dest>` runs `VACUUM INTO`. A live `cp orag.db`
can omit WAL content and is documented as unsafe. Bundled SQLite must be
≥ 3.51.3 (WAL-reset corruption fix); a test asserts this.

## D-005 — Retrieval in v1 is deterministic hybrid; no decision models

**Decision.** Every query: FTS5 BM25 top 50 + dense top 50, both constrained to
the collection **before** top-k → Reciprocal Rank Fusion (k = 60) → context
filled in fused order up to the generator's token budget (max 8 chunks). No
reranker, no query rewriting, no adaptive routing in v0.1.

**Abstention.** Hard abstain only when both candidate lists are empty.
Otherwise the generation prompt instructs the model to answer only from the
numbered sources and to reply with a fixed refusal sentence when they are
insufficient. If retrieval found chunks but none fits the
generator's context window, that is a capacity error, not "not found". No cosine-threshold rejection (a strong lexical match can have a
weak dense score, so thresholding defeats hybrid retrieval).

**Decision models.** Jev is excluded: only hosted access was verified, no
downloadable weights. Nimble is deferred: it is 9B with Python MLX/PyTorch
tooling, not a cheap laptop router. Small decision/classifier models are
reconsidered only after the evaluation harness shows a failure class that
deterministic routing cannot fix.

**Revisit.** v0.4, using evaluation results.

## D-006 — Turkish-aware lexical normalization without a custom tokenizer

**Decision.** FTS5 indexes a *normalized shadow text*; original text is kept
in `chunks.text` and used for embeddings and citations. One Rust function
normalizes both indexed text and queries: NFKC → fold `İ`, `I`, `ı` to `i`
(symmetric, so English and Turkish casing both match; the ı/i distinction is
deliberately lost for recall) → Unicode lowercase. Tokenizer:
`unicode61 remove_diacritics 0` — ç/ş/ğ/ö/ü are preserved. User queries are
turned into quoted OR-terms, never passed to FTS5 as raw syntax.

**Deferred.** Stemming and an accent-folded secondary field are added only if
the evaluation set (which includes accentless-typing queries) shows a gain.
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
fingerprint is SHA-256 over: model file SHA-256, pooling, query prefix,
document prefix, dimension, normalization flag, maximum input tokens,
trailing-EOS rule, chunker version and normalizer version. Vectors from different spaces are never mixed. If the
configured embedder's fingerprint differs from a collection's space, queries
and ingestion into that collection fail with `409 reindex_required`;
uploads are checked before they are accepted (and again by the worker).
Re-indexing (build new space, switch atomically) is v0.2 scope.

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
  document. On Linux the child is also capped at 2 GiB of address space and
  is the OOM killer's first choice. On macOS only the deadline applies.
- JSON text uploads: a filename with a supported extension sets the format;
  known unsupported document types (`.xlsx`, `.html`, …) are 415; other dotted
  names (`Toplantı 12.10.2026`) are plain text.

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
  unless allow-listed (the desktop origin is added in v0.3), so web pages in a
  local browser cannot call it.

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
| `max_document_mb` | `5` | integer 1–10; the next milestone raises the default to 10; values above 10 are not supported |
| `embedding_model` | `qwen3-embedding-0.6b-q8_0` | installed model pack id |
| `generation_model` | `qwen3.5-4b-q4_k_m` | installed model pack id |
| `log_level` | `info` | error, warn, info, debug, trace |

Unknown keys or invalid values stop startup with a message naming the file.
`ORAG_HOME` is the only environment variable; it locates the directory and
sets nothing else. Uploads above the limit get `413 too_large` before they are
stored. `GET /v1/version` reports the settings in effect.

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
8. If the usage limit is hit, wait and resume from the last merged step. An
   unmerged `step/*` branch marks the interrupted task.
9. Tags are never created by the agent; they are handed to the owner.

The owner authorized commit, merge into `main` and push for this repository
only (2026-10-01). This repository-scoped grant overrides the owner's general
"never commit/push" policy here and nowhere else.

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
