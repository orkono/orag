<p align="center">
  <img src="assets/orag-icon.svg" alt="ORAG icon: a sickle (orak)" width="128" height="128">
</p>

# ORAG

ORAG is an offline, local-first RAG (retrieval-augmented generation) service in
a single executable. You give it your documents; it indexes them on your
machine and answers questions from them with cited sources, using local models
through llama.cpp. It needs no Python, Docker, database server or cloud
account, and it never reaches the network on its own: on the first start the
page downloads the models only when you press *Modelleri indir*, or you
install them from offline packs (D-001, D-012).

## Status

`v0.3.0`: the engine, the built-in web page and a desktop application for
macOS, Windows and Linux (`docs/desktop.md`) that runs the service and shows
the page in its own window.

- Formats: TXT, Markdown, DOCX and PDF, up to 10 MB per document by default
  (5 MB in v0.1.0). Scanned PDFs are not OCRed: pages without a text layer
  are reported with the warning `ocr_required`, and pages whose text looks
  garbled with `suspicious_layout`. Sources from a PDF carry their page
  numbers (`pages`, shown as *s. 12–13* on the page).
- Retrieval: BM25 + dense vectors fused with reciprocal rank fusion, each
  list's top hit offered to the answer context first; answers cite their
  sources as `[n]` and abstain when the documents do not support an answer.
  Answers can be streamed (SSE).
- Models: the v0.2 evaluation kept Qwen3-Embedding-0.6B and Qwen3.5-4B as
  the defaults (D-007, `.docs/benchmarks/v0.2-model-decision.md`).
- **The 200-question evaluation set is synthetic:** written by one model and
  audited row by row by a model of another family, not by people (D-018); its
  numbers compare builds and models, they are not a quality guarantee. Questions whose answer needs two documents are the
  weakest group (`.docs/benchmarks/v0.3-set-rewrite.md`).

## System requirements

- macOS 14 or newer on Apple Silicon (Metal is used automatically), or Linux
  x86-64 with glibc 2.35 or newer (Ubuntu 22.04+, Debian 12+, Fedora 36+; the
  binary is built on Ubuntu 22.04), or Windows 10 22H2 / 11 x86-64 on a CPU
  with AVX2 (Intel 2013+, AMD Zen); no Visual C++ runtime is needed.
- 16 GB RAM recommended.
- About 3.1 GB of disk for the two installed default models, plus your
  documents. `orag models import` copies a pack, so the pack directory you
  imported from needs another 3.1 GB until you delete it.

Measured speed with the default models on an Apple M5 Max (Metal), for a
~600-token prompt: the first answer token arrives in about 0.2 s, then tokens
stream at roughly 110–130 per second (`.docs/benchmarks/v0.2-seed-eval.md`). A 16 GB
M-series laptop and CPU-only machines (Linux, Windows) are slower and are not
measured yet.
Dense search over 100 000 chunks stays below 250 ms (p95), measured on an
Apple M5 Max and on an x86-64 Linux machine with 16 GB RAM (a GitHub-hosted
runner); a 16 GB M-series laptop is measured after v0.1.0
(`.docs/benchmarks/vector-scale.md`, D-003).

## Desktop app

Install `ORAG_<version>_aarch64.dmg` (macOS), `ORAG_<version>_x64-setup.exe`
(Windows) or the `.AppImage` / `.deb` (Linux) from the release and open
**ORAG**: it starts the service and shows the page in its own window, with its
own data folder. See `docs/desktop.md` for the first start, the data folder
and the unsigned-app prompts, and `docs/signing.md` to check a download
(`SHA256SUMS`, `gh attestation verify`).

## Windows


Run `ORAG-Setup-<version>.exe` from the release (per user, no administrator
rights; it installs to `%LOCALAPPDATA%\Programs\ORAG`) and open **ORAG** from
the Start menu; or unzip `orag-<version>-x86_64-pc-windows-msvc.zip` anywhere
you can write to and run `orag.exe start`. The page opens at
<http://127.0.0.1:2442> in setup mode until the models are installed. The page
shows this computer (*Bu bilgisayar*) and the answer models that fit it: the
recommended `qwen3.5-4b-q4_k_m` (6.5 GB RAM) and, with 9 GB RAM or more,
`qwen3.5-9b-q4_k_m` (slower, uses more memory). Pick one and press *Modelleri indir*;
the embedder and that model download (about 3.1 GB for the default), the
choice is written as `generation_model` into `config.toml`, and ORAG
restarts by itself. The Start menu also has
*ORAG modellerini indir (internet)*, which downloads the same two packs
(after asking) into `%USERPROFILE%\ORAG-paketleri` with
`fetch-model-pack.ps1`. Data lives in
`%USERPROFILE%\.orag` (or `%ORAG_HOME%`); uninstalling keeps it. Without
internet on the PC, build packs elsewhere with `scripts/fetch-model-pack.sh`
(bash) or `fetch-model-pack.ps1` (PowerShell, also in the installer); copy them to
the PC and type their folder into *Modelleri kur*, or run
`orag.exe models import <folder>\<pack>`. Windows runs on the CPU (no GPU
offload), so answers are expected to be slower than on Apple Silicon; CPU
speed is not measured yet. Ctrl-C and Ctrl-Break stop ORAG gracefully;
closing the console window stops it too, but Windows gives it only a few
seconds, so indexing that was cut off resumes on the next start.
The binary is not code-signed yet, so SmartScreen may ask once before it
runs.

## Configuration

`~/.orag/config.toml` (or `$ORAG_HOME/config.toml`) is created on first start
with every key commented out, so the built-in defaults apply until you
uncomment a line. It is read **once at startup**; after editing it, restart
`orag serve`.

| Key | Default | Meaning |
|---|---|---|
| `bind` | `"127.0.0.1:7613"` | Address of the HTTP API. Loopback only (`127.0.0.1` or `::1`); port `0` picks a free port |
| `ui_bind` | `"127.0.0.1:2442"` | Address of the built-in web page, loopback only, or `"off"` |
| `max_document_mb` | `10` | Largest document accepted, in MB (1–10) |
| `embedding_model` | `"qwen3-embedding-0.6b-q8_0"` | Installed embedding model pack (`orag models list`) |
| `generation_model` | `"qwen3.5-4b-q4_k_m"` | Installed generation model pack; the setup page writes the model you picked here (only this key) |
| `log_level` | `"info"` | `error`, `warn`, `info`, `debug` or `trace`, on stderr |
| `models.download` | `true` | Whether setup mode may download the missing models when you press *Modelleri indir*; set `models.download = false` for a fully offline install |

## Build

```bash
cargo build --release
```

The binary is `target/release/orag`. On Apple Silicon llama.cpp uses Metal
automatically.

## Install models

On a computer with internet access, start ORAG and press *Modelleri indir*
on the page (see *Run*). It downloads only the configured models, from
pinned Hugging Face revisions, checks every file by SHA-256, honours
`HTTPS_PROXY`, `HTTP_PROXY` and `NO_PROXY`, and continues where it stopped
if the connection breaks. It runs only when you press the button, and only
in setup mode. A company proxy that re-signs HTTPS with its own certificate is
refused (ORAG trusts only the public root certificates it ships with); use the
offline packs there.

Offline, build the packs on a machine with internet access (each file is pinned to a
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
orag start   # starts the service and opens the page in your browser
orag serve   # the service alone (for scripts, services and the desktop app)
```

`orag start` runs `orag serve`, opens <http://127.0.0.1:2442> once it is up,
and restarts it after models are imported from the page; if ORAG already runs,
it only opens the page. Without installed models ORAG starts in **setup
mode**. The page offers *Modelleri indir*: press it and wait while the two
default models (about 3.1 GB) download and are verified. Without internet,
type the folder you copied the packs to (for example `~/orag-packs`) into
*Modelleri kur* instead. Either way ORAG restarts by itself under
`orag start` (under `orag serve`, restart it yourself). Until then uploading
and asking are refused with `models_missing`. Set `models.download = false`
in `config.toml` to hide the download for a fully offline install.

It prints `orag listening on http://127.0.0.1:7613` (the API) and then
`orag ui on http://127.0.0.1:2442`, and stops on Ctrl-C or SIGTERM. Indexing
that was interrupted resumes on the next start.

Open **<http://127.0.0.1:2442>** in a browser for the built-in page. The
sidebar holds the collections (*+* creates one), the upload area (drop files
anywhere on the page, several at once) and the selected collection's
documents; the title bar reindexes or deletes the collection. Ask in the box
at the bottom (Enter sends, Shift+Enter starts a new line, *Durdur* stops):
answers stream in, each `[n]` links to its source, and the questions stay on
screen until you switch collections. *Nasıl kullanılır?* explains ORAG and
its three steps; it opens by itself on the first visit. The page is part of the binary and loads nothing
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

- `max_document_mb` per document: 10 MB by default, configurable 1–10. A
  large text-only document (millions of words) can keep indexing busy for
  many minutes; other uploads wait `queued` meanwhile.
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

The v0.2 sets are `eval/datasets/v02-retrieval.jsonl` (200 questions over
12 documents, dev and holdout split) and `v02-answers.jsonl` (60 answer
probes, scored with `orag eval answers`). See [`eval/README.md`](eval/README.md)
and `.docs/benchmarks/`.

## Versioning

ORAG uses Semantic Versioning (D-017). Every change merged to `main` bumps the
version and adds a `CHANGELOG.md` entry; while milestone M is built the version
is `0.M.0-alpha.N`, the milestone release is `0.M.0`. `orag --version` and
`GET /v1/version` report it.

## License

MIT OR Apache-2.0, at your option. Model licenses ship inside each model pack.
