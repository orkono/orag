<p align="center">
  <img src="assets/orag-icon.svg" alt="ORAG icon: a sickle (orak)" width="128" height="128">
</p>

# ORAG

ORAG is an offline, local-first RAG (retrieval-augmented generation) service in
a single executable. You give it your documents; it indexes them on your
machine and answers questions from them with cited sources, using local models
through llama.cpp. It needs no Python, Docker, database server or cloud
account, and it never reaches the network on its own: models are installed
from offline packs (D-001, D-012).

## Status

`v0.1.0`: the first usable release.

- Formats: TXT, Markdown, DOCX and PDF, up to 5 MB per document by default.
  Scanned PDFs are not OCRed; pages without a text layer are reported with the
  warning `ocr_required`.
- Retrieval: BM25 + dense vectors fused with reciprocal rank fusion, each
  list's top hit offered to the answer context first; answers
  cite their sources as `[n]` and abstain when the documents do not support an
  answer. Answers can be streamed (SSE).
- A desktop application arrives in v0.3; v0.1 is an HTTP API.
- **The default models and retrieval settings are provisional (D-007).** A
  200-question evaluation in v0.2 decides them; v0.1's small seed set only
  proves that the pipeline works.

## System requirements

- macOS 14 or newer on Apple Silicon (Metal is used automatically), or Linux
  x86-64 with glibc 2.35 or newer (Ubuntu 22.04+, Debian 12+, Fedora 36+; the
  binary is built on Ubuntu 22.04). Windows arrives in v0.2.
- 16 GB RAM recommended.
- About 3.3 GB of disk for the two installed default models, plus your
  documents. `orag models import` copies a pack, so the pack directory you
  imported from needs another 3.3 GB until you delete it.

Measured speed with the default models on an Apple M5 Max (Metal), for a
~600-token prompt: the first answer token arrives in about 0.2 s, then tokens
stream at roughly 110–130 per second (`.docs/benchmarks/v0.1-seed-eval.md`). A 16 GB
M-series laptop and a CPU-only Linux machine are slower; v0.2 measures them.
Dense search over 100 000 chunks stays below 250 ms (p95), measured on an
Apple M5 Max and on an x86-64 Linux machine with 16 GB RAM (a GitHub-hosted
runner); a 16 GB M-series laptop is measured after v0.1.0
(`.docs/benchmarks/vector-scale.md`, D-003).

## Configuration

`~/.orag/config.toml` (or `$ORAG_HOME/config.toml`) is created on first start
with every key commented out, so the built-in defaults apply until you
uncomment a line. It is read **once at startup**; after editing it, restart
`orag serve`.

| Key | Default | Meaning |
|---|---|---|
| `bind` | `"127.0.0.1:7613"` | Address of the HTTP API. Loopback only (`127.0.0.1` or `::1`); port `0` picks a free port |
| `ui_bind` | `"127.0.0.1:2442"` | Address of the built-in web page, loopback only, or `"off"` |
| `max_document_mb` | `5` | Largest document accepted, in MB (1–10) |
| `embedding_model` | `"qwen3-embedding-0.6b-q8_0"` | Installed embedding model pack (`orag models list`) |
| `generation_model` | `"qwen3.5-4b-q4_k_m"` | Installed generation model pack |
| `log_level` | `"info"` | `error`, `warn`, `info`, `debug` or `trace`, on stderr |

## Build

```bash
cargo build --release
```

The binary is `target/release/orag`. On Apple Silicon llama.cpp uses Metal
automatically.

## Install models (offline)

On a machine with internet access, build the packs (each file is pinned to a
revision and checked by SHA-256):

```bash
scripts/fetch-model-pack.sh qwen3-embedding-0.6b-q8_0 ~/orag-packs
scripts/fetch-model-pack.sh qwen3.5-4b-q4_k_m ~/orag-packs
```

Copy `~/orag-packs` to the target machine (for example on a USB drive) and
import them there; import verifies every file:

```bash
orag models import ~/orag-packs/qwen3-embedding-0.6b-q8_0
orag models import ~/orag-packs/qwen3.5-4b-q4_k_m
orag models list
```

## Run

```bash
orag serve
```

It prints `orag listening on http://127.0.0.1:7613` (the API) and then
`orag ui on http://127.0.0.1:2442`, and stops on Ctrl-C or SIGTERM. Indexing
that was interrupted resumes on the next start.

Open **<http://127.0.0.1:2442>** in a browser for the built-in page. *Koleksiyon*
selects the collection to work in, creates a new one or deletes the selected
one (with its documents; the `default` collection stays) or reindexes it
with the current model (*Yeniden indeksle*, e.g. after `409 reindex_required`;
no new upload needed). *Belgeler* lists
the documents of the selected collection (status, chunks, size, warnings) and
deletes one after a confirmation. *Dosya yükle*
uploads a TXT, Markdown, DOCX or PDF file to the selected collection and shows
its indexing progress; *Sorgu yap* asks a question in the selected collection
only and streams the answer with its sources; each `[n]` in the answer links to source `n`, and sources the answer does not cite are marked. The page is part of the binary and loads nothing
from the network. Set `ui_bind = "off"` in `config.toml` to turn it off. A
second `orag serve` (another `ORAG_HOME`) needs its own `bind` and `ui_bind`
(or `ui_bind = "off"`): a busy port stops startup.

The API needs **no credentials** and is reachable **only from this machine**
(127.0.0.1). That also means any account or process on the same machine can
use it (D-013): v1 is meant for single-user personal machines.

```bash
# Upload a document (DOCX/PDF need multipart; TXT/Markdown can also be JSON)
curl -s -F "file=@crates/orag/tests/fixtures/sample.docx" \
  http://127.0.0.1:7613/v1/collections/1/documents

# Ask a question
curl -s -H 'Content-Type: application/json' \
  -d '{"query":"İade süresi?"}' http://127.0.0.1:7613/v1/collections/1/query

# Stream the answer (server-sent events)
curl -N -s -H 'Content-Type: application/json' \
  -d '{"query":"İade süresi?","stream":true}' http://127.0.0.1:7613/v1/collections/1/query
```

The full API, with every route, error code and the streaming contract, is in
[`docs/api.md`](docs/api.md).

## Backups

```bash
orag backup ~/orag-backup-2026-10-05.db
```

It writes a consistent copy and is safe while the server runs. Never copy
`orag.db` with `cp`: a live copy can miss recent writes (D-004).

## Upgrades

Before upgrading, pin `embedding_model` and `generation_model` in
`config.toml`, so a new default cannot change the models you indexed with.
Read the **Upgrade note** bullets in `CHANGELOG.md`: a release that changes the
embedding space says so, and collections indexed before it then answer
`409 reindex_required`; reindex the collection (`POST /v1/collections/{id}/reindex` or *Yeniden indeksle* in the page; see [`docs/api.md`](docs/api.md)).

## Limits

- `max_document_mb` per document: 5 MB by default, configurable 1–10.
- At most 4 uploads at once; one answer is generated at a time (a query waits
  up to 120 s for its turn).
- A `ready` document with an `ocr_required` or `extraction_failed` warning is
  only partially indexed; one with `no_text` (for example a fully scanned PDF)
  is not indexed at all. Scanned pages are not OCRed.

## Evaluation

```bash
# Lexical vs dense vs hybrid retrieval on a labeled question set
orag eval retrieval --corpus eval/corpus/seed --dataset eval/datasets/seed.jsonl

# Dense search latency at scale (the D-003 gate: 100k chunks, p95 < 250 ms)
orag eval vector-scale --chunks 100000 --dimensions 1024 --queries 50
```

See [`eval/README.md`](eval/README.md) and `.docs/benchmarks/`.

## Versioning

ORAG uses Semantic Versioning (D-017). Every change merged to `main` bumps the
version and adds a `CHANGELOG.md` entry; while milestone M is built the version
is `0.M.0-alpha.N`, the milestone release is `0.M.0`. `orag --version` and
`GET /v1/version` report it.

## License

MIT OR Apache-2.0, at your option. Model licenses ship inside each model pack.
