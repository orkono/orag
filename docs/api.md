# ORAG HTTP API (v1)

ORAG serves a small JSON API on the local machine. Every example below is a
real response captured from `orag serve --dev-fake-models` (fake models, so
answers are placeholders; real models answer from the documents).

## Access

- **No authentication.** There are no keys, tokens or accounts (D-013).
- **Local machine only.** The server binds to a loopback address
  (`127.0.0.1:7613` by default; `127.0.0.1` or `::1` only) and rejects a
  request whose `Host` is not loopback (`403 forbidden_host`) or that comes
  from a web page (`Origin` or `Sec-Fetch-Site: cross-site`,
  `403 forbidden_origin`). Any account or process on the same machine can
  call it, so v1 is meant for single-user personal machines.
- **Allowed origins.** The only web origins accepted are those of the
  built-in page (`ui_bind`, default `http://127.0.0.1:2442` and
  `http://localhost:2442`; the real port when it is `0`). With
  `ui_bind = "off"` no origin is accepted. Every other page gets
  `403 forbidden_origin`. A request the browser marks
  `Sec-Fetch-Site: same-origin` (the page's own) is accepted whatever its
  `Origin` says. Each refusal is logged on stderr as `request refused` with
  the Host, Origin and `Sec-Fetch-Site` that decided it.
- Request and response bodies are JSON (`Content-Type: application/json`),
  except multipart uploads and the SSE query stream.

## Formats and limits

| Item | Value |
|---|---|
| Supported formats | TXT (`.txt`), Markdown (`.md`, `.markdown`), DOCX (`.docx`), PDF (`.pdf`) |
| Document size | `max_document_mb` in `config.toml`: default 10, allowed 1–10 |
| Concurrent uploads | 4; the next one gets `429 busy` |
| Upload body time | 60 s; a slower body gets `408 upload_timeout` |
| Answers | one at a time; a query waits up to 120 s for the slot, then `429 busy` |
| Page size | `limit` 1–200, default 50 |

- DOCX and PDF are uploaded as `multipart/form-data` (field `file`). Their
  bytes are checked before anything is stored: a PDF must contain `%PDF-`, a
  DOCX must be a Word package (a renamed `.xlsx`/`.pptx`/`.odt` is refused).
- TXT and Markdown can also be sent as JSON text. JSON content is always UTF-8
  text and the `filename` is only a label: an unknown extension (`notes.v2`,
  `Toplantı 12.10.2026`) is stored as plain text. Known unsupported types
  (`.doc`, `.xlsx`, `.pptx`, `.html`, `.csv`, `.json`, `.zip`, …) are refused
  with `415 unsupported_format`, and a `.docx`/`.pdf` name in JSON is
  `400 invalid_input` ("upload as multipart").
- In a multipart upload the extension decides; with no supported extension the
  part's `Content-Type` decides; anything else is `415 unsupported_format`.
- Each entry in a document's `warnings` is a sentence that starts with a
  code and a colon, for example `"ocr_required: page(s) 2-3 have no
  extractable text (scanned?) and were not indexed"`. Match on the prefix:
  - `ocr_required`: PDF pages without a text layer (scanned) were skipped;
    ORAG does not OCR;
  - `extraction_failed`: PDF pages that could not be read were skipped;
  - `suspicious_layout`: PDF pages whose extracted text looks garbled — one
    warning per reason, listing the pages: `unmapped characters` (U+FFFD or
    control characters from a font without a usable ToUnicode map; they are
    removed before indexing) or `words split into single letters` (runs of
    four or more one-letter tokens). Advisory only: the pages are indexed,
    but answers drawn from them can be wrong. Not detected: words run
    together, and columns read in the wrong order;
  - `styles_unreadable`: a DOCX's styles could not be read, so headings and
    lists set by styles were not detected (the text is indexed);
  - `no_text`: the document has no extractable text at all (for example a
    fully scanned PDF); it is `ready` with 0 chunks and answers nothing.

  A `ready` document with `ocr_required` or `extraction_failed` is only
  partially indexed.

## Configuration

`$ORAG_HOME/config.toml` (default `~/.orag/config.toml`) is read **once at
startup**; edit it and restart `orag serve`. It is created with every key
commented out, so built-in defaults apply until a line is uncommented.

| Key | Default | Meaning |
|---|---|---|
| `bind` | `"127.0.0.1:7613"` | Loopback address; port `0` picks a free port |
| `ui_bind` | `"127.0.0.1:2442"` | Loopback address of the built-in web page (port `0` picks a free port), or `"off"` |
| `max_document_mb` | `10` | Largest document accepted, 1–10 MB |
| `embedding_model` | `"qwen3-embedding-0.6b-q8_0"` | Installed embedding pack id |
| `generation_model` | `"qwen3.5-4b-q4_k_m"` | Installed generation pack id |
| `log_level` | `"info"` | `error`, `warn`, `info`, `debug` or `trace` (stderr) |
| `models.download` | `true` | Setup mode may download the missing catalog models when asked (`POST /v1/models/download`); `false` turns it off for fully offline installs |

`GET /v1/version` shows the values in effect (`bind` and `ui_bind` with the
real port).

## Built-in web page

`orag serve` also serves a small page on `ui_bind` and prints
`orag ui on http://127.0.0.1:2442` as its second stdout line (the first line,
`orag listening on ...`, is unchanged). The page uses this same API on its own
origin (collections, uploads, jobs, queries; it adds no endpoint of its own):
`GET /`, `/app.css`, `/favicon.svg` and the page's script modules (`/app.js` and the modules it imports) are served there with
`Content-Security-Policy: default-src 'self'` and
`X-Content-Type-Options: nosniff`; the `/v1` routes answer on both ports.

## Errors

Every error, including unknown routes, has the same shape:

```json
{"error":{"code":"not_found","message":"collection 99 not found"}}
```

| Status | `code` | When |
|---|---|---|
| 400 | `invalid_input` | Bad JSON, unknown field, empty or too long query, bad path or query value, a `.docx`/`.pdf` sent as JSON, a multipart body without a `file` field, bytes that do not match the format (a renamed `.xlsx` as `.docx`, a non-PDF as `.pdf`) |
| 400 | `invalid_multipart` | A multipart body that cannot be parsed (500 with the same code if the server could not read the body stream) |
| 400 | `download_unavailable` | `POST /v1/models/download` while a missing configured model is not in the download catalog |
| 400 | `unknown_model` | `POST /v1/models/download` names a `generation_model` that is not one of the offered generator tiers |
| 403 | `forbidden_host` | `Host` is not a loopback address |
| 403 | `forbidden_origin` | A browser cross-origin request |
| 403 | `download_disabled` | `POST /v1/models/download` while `models.download = false` |
| 404 | `not_found` | Unknown collection, document, job or route |
| 405 | `method_not_allowed` | The route exists, the method does not (`Allow` header lists methods) |
| 408 | `upload_timeout` | The upload body did not arrive within 60 s |
| 409 | `conflict` | E.g. deleting the default collection |
| 409 | `model_not_eligible` | `POST /v1/models/download` names an offered generator this machine cannot take now (RAM, disk or CPU) |
| 409 | `reindex_required` | The collection was indexed in a different embedding space: another embedding model, or a release that changed the chunker (see below) |
| 413 | `too_large` | Document over `max_document_mb`, or a request body over its limit |
| 415 | `unsupported_format` | A document type ORAG does not support |
| 415 | `unsupported_media_type` | A JSON route called without `Content-Type: application/json` |
| 429 | `busy` | 4 uploads in progress, or the answer slot stayed taken for 120 s. Carries `Retry-After: 5` |
| 500 | `internal` | A server-side failure; details are in the server log, not the response |
| 503 | `shutting_down` | The service is stopping and starts no new work |
| 503 | `models_missing` | Setup mode: the configured models are not installed, so upload, query and reindex are refused (see `GET /v1/models`) |

Client timeouts for queries should exceed 120 s plus the answer time, because
a query waits for the single answer slot before it gets `busy`.

### `reindex_required` (409): recovery

A collection keeps the embedding space it was indexed with. If the embedding
model, or anything else in that space, changed, queries and uploads on that
collection return `409 reindex_required`. Then:

1. **Reindex the collection** (since 0.2.0-alpha.10):
   `POST /v1/collections/{collection_id}/reindex` (or *Yeniden indeksle* in the
   built-in page) indexes every document again from its stored copy with the
   current model. Nothing needs to be uploaded again and ids stay the same.
2. **Or pin the previous model.** Set `embedding_model` back to the previous pack
   id in `config.toml` and restart. This works only if nothing else in the
   embedding space changed: the space fingerprint covers the model id and file
   checksum, pooling, query and document prefixes, dimensions, normalization,
   token limit, the trailing-EOS rule, the chunker version and the fingerprint
   encoding (D-009). A change of the lexical search (normalizer, indexed text)
   is not part of it: ORAG rebuilds the full-text index at startup instead. A release that changes any of them says
   so in an **Upgrade note** in `CHANGELOG.md`; then pinning is not enough.
3. **Or re-upload.** A collection with no documents follows the
   current embedding model again. Either delete every document in the
   collection (`DELETE .../documents/{id}` works while it is
   `reindex_required`) and upload the files again, which keeps the collection
   id (this is the way for the default collection, which cannot be deleted);
   or create a new collection, upload the files there and use its id.

## Endpoints

### `GET /v1/health`

```text
HTTP/1.1 200 OK
{"status":"ok"}
```

### `GET /v1/version`

```text
HTTP/1.1 200 OK
{"api":"v1","config":{"bind":"127.0.0.1:7613","embedding_model":"qwen3-embedding-0.6b-q8_0","generation_model":"qwen3.5-4b-q4_k_m","log_level":"info","max_document_mb":5,"ui_bind":"127.0.0.1:2442"},"git_sha":"239d9e66c70d","schema_version":1,"version":"0.1.0"}
```

Captured from 0.1.0. Since 0.2.0-alpha.19 the default `max_document_mb` is
`10` (an install whose `config.toml` pins `5` keeps 5), and `schema_version`
follows the database schema.

### `GET /v1/collections`

Collection `1` (`default`) always exists.

```text
HTTP/1.1 200 OK
{"collections":[{"created_at":"2026-10-05T10:27:14.162Z","document_count":0,"id":1,"name":"default"}]}
```

### `POST /v1/collections`

Body: `{"name": "<name>"}`.

```bash
curl -s -H 'Content-Type: application/json' -d '{"name":"Destek"}' \
  http://127.0.0.1:7613/v1/collections
```

```text
HTTP/1.1 201 Created
{"id":2,"name":"Destek","document_count":0,"created_at":"2026-10-05T10:27:16.366Z"}
```

### `DELETE /v1/collections/{collection_id}`

Deletes a collection with its documents. `204 No Content`; the default
collection is `409 conflict`:

```text
HTTP/1.1 409 Conflict
{"error":{"code":"conflict","message":"conflict: the default collection cannot be deleted"}}
```

### `POST /v1/collections/{collection_id}/documents`

Queues a document for indexing and returns at once. `202 Accepted` for a new
document; `200 OK` with `"duplicate": true` when the collection already holds
the same bytes. Follow progress with the job or the document.

Multipart (any supported format):

```bash
curl -s -F "file=@crates/orag/tests/fixtures/sample.docx" \
  http://127.0.0.1:7613/v1/collections/1/documents
curl -s -F "file=@crates/orag/tests/fixtures/sample-tr.pdf" \
  http://127.0.0.1:7613/v1/collections/1/documents
```

```text
HTTP/1.1 202 Accepted
{"document_id":1,"duplicate":false,"job_id":1}
```

JSON text (TXT and Markdown only): `{"content": "...", "filename": "..."}`,
optional `"format": "text" | "markdown"`.

```bash
curl -s -H 'Content-Type: application/json' \
  -d '{"filename":"iade.md","content":"# İade\n\n14 gün içinde iade."}' \
  http://127.0.0.1:7613/v1/collections/1/documents
```

```text
HTTP/1.1 202 Accepted
{"document_id":3,"duplicate":false,"job_id":3}
```

A PDF sent as JSON:

```text
HTTP/1.1 400 Bad Request
{"error":{"code":"invalid_input","message":"invalid input: pdf files must be uploaded as multipart/form-data (field `file`)"}}
```

### `POST /v1/collections/{collection_id}/reindex`

Indexes every document of the collection again from the copy stored at upload,
with the embedding model and chunker of the running server. No body. The
chunks are removed and each document is queued with a new job at once, so
until the jobs finish, queries find only the documents already done. While a
document of the collection is still `queued` or `indexing` (an upload or an
earlier reindex) the call is refused with `409 conflict`, so a repeated call
cannot undo finished work; try again when they are `ready` or `failed`.
`404 not_found` for an unknown collection.

```bash
curl -s -X POST http://127.0.0.1:7613/v1/collections/1/reindex
```

```text
HTTP/1.1 202 Accepted
{"collection_id":1,"queued_documents":3}
```

### `GET /v1/models`

The configured models and whether they are installed. `mode` is `setup` when
the server started without one of them (D-023): then upload, query and reindex
answer `503 models_missing` until the packs are imported and ORAG restarts.

`download` says whether `POST /v1/models/download` can fetch the missing
models (D-012 amendment): `available`; when it is `false`, a `code`
(`conflict`, `download_disabled` or `download_unavailable`, the same codes the
`POST` would answer) and a `reason` (plain English: not in setup mode, `models.download = false` in
`config.toml`, or which configured model is not in the download catalog),
and `bytes`, the total size of the missing models' files (`null` when
nothing can be downloaded).

**The generator choice (step 82).** `machine` is what the server measured on
this computer, without spawning anything or using the network (the fixed
facts once per process, available RAM and free disk on every call):

| Field | Meaning |
|---|---|
| `os`, `arch` | `macos`/`linux`/`windows`, `aarch64`/`x86_64` |
| `total_ram_bytes`, `available_ram_bytes` | Physical RAM and what is free now (`null` when unknown) |
| `logical_cores`, `physical_cores` | CPU threads and cores (`null` when unknown) |
| `cpu_model` | The CPU's name, e.g. `Apple M5 Max` |
| `cpu_features` | On x86-64 `{"avx2","fma","f16c","bmi2"}` booleans (read with CPUID); `"not_applicable"` elsewhere |
| `apple_silicon`, `metal` | An Apple Silicon Mac, where llama.cpp runs on the GPU through Metal |
| `free_disk_bytes` | Free space where the models are installed |
| `gpu` | Information only, never used to decide: `{"name","memory_bytes"}` (Windows: DXGI; Linux: `/sys/class/drm`), or `null` (always on macOS, where `metal` says it) |

`download` then also carries the generator tiers the setup page offers:

| Field | Meaning |
|---|---|
| `options[]` | Each offered tier, smallest first: `id`, `size` (`default`, `large`), `current`, `label_key` (the page's dictionary key), `bytes` (the pack), `installed`, `need_ram_bytes` (total RAM the whole process needs: measured peak + 2 GiB), `speed` (`metal_p50_answer_s`, `cpu_p50_answer_s`: typical answer seconds, `null` until measured), `download_bytes` (what a download still fetches: 0 when installed, less after an interrupted run), `verdict` and `warnings`. When the configured `generation_model` is a catalog generator but not an offered tier (e.g. a hand-set `qwen3-4b-instruct-2507-q4_k_m`), it comes first as the current option: `current: true`, `size: null`, `label_key` `setup.tier.current`, its own tier's need or else the default tier's. A configured id outside the catalog is not listed |
| `verdict` | `recommended` (the default tier, whenever it fits; the large tier is never recommended: it is slower and uses more memory), `fits`, `too_big_ram` (total RAM below the need) or `too_big_disk` (free disk below what the embedder and the option still have to fetch + 512 MiB); unknown RAM or disk never blocks |
| `options[].warnings` | `["low_available_memory"]` when the tier fits but less RAM is free right now (a warning, not a block) |
| `support` | `{"status": "supported" \| "unsupported", "code", "missing_cpu_features"}`; `code` is `cpu_unsupported` (Windows only: an x86-64 CPU without AVX2, FMA, F16C and BMI2, which the Windows binary is built for) or `no_tier_fits` (no option fits). `missing_cpu_features` lists the missing instructions on any x86-64 system |
| `warnings` | `single_option` when only one option fits; `slow_cpu` on an x86-64 CPU without that floor outside Windows (it runs, more slowly) |
| `selected` | The option the page starts with: the configured `generation_model` if it is an option that fits, else the recommended one; `null` when unsupported |
| `embedding_bytes` | Bytes of the embedder still to download (0 when installed; `.part` files of an interrupted run count as downloaded) |
| `configured_downloadable` | Whether the configured models are eligible for a `POST` without a body. `true` only when `available` is `true` for the configured models; it does not account for a download or import already running, which makes any `POST` answer `409 conflict` until it ends. `false` whenever that `POST` would be refused for the models themselves: outside setup mode or with the models installed (`409 conflict`), with `models.download = false` (`403 download_disabled`; `available` is `false` too), or when a configured model is not in the download catalog (`400 download_unavailable`). Only in that last case can `available` still be `true`: when the selected option (and a missing embedder) can be downloaded, `bytes` is the embedder plus that option, and choosing it writes `generation_model` |

The small tier (`qwen3.5-2b-q4_k_m`) failed the quality rule and is not
offered (`.docs/benchmarks/v0.5-model-tiers.md`); it stays downloadable only as
a hand-set `generation_model`.

```bash
curl -s http://127.0.0.1:7613/v1/models
```

```text
HTTP/1.1 200 OK
{"download":{"available":true,"bytes":3380111382,"code":null,"embedding_bytes":639161950,"options":[{"bytes":2740949432,"id":"qwen3.5-4b-q4_k_m","installed":false,"current":false,"label_key":"setup.tier.default","need_ram_bytes":6979321856,"size":"default","speed":{"cpu_p50_answer_s":null,"metal_p50_answer_s":0.9},"verdict":"recommended","warnings":[]},{"bytes":5680534008,"id":"qwen3.5-9b-q4_k_m","installed":false,"current":false,"label_key":"setup.tier.large","need_ram_bytes":9663676416,"size":"large","speed":{"cpu_p50_answer_s":null,"metal_p50_answer_s":2.5},"verdict":"fits","warnings":[]}],"reason":null,"selected":"qwen3.5-4b-q4_k_m","support":{"code":null,"missing_cpu_features":[],"status":"supported"},"warnings":[]},"embedding":{"id":"qwen3-embedding-0.6b-q8_0","installed":false},"generation":{"id":"qwen3.5-4b-q4_k_m","installed":false},"machine":{"apple_silicon":true,"arch":"aarch64","available_ram_bytes":86288089088,"cpu_features":"not_applicable","cpu_model":"Apple M5 Max","free_disk_bytes":662947000320,"gpu":null,"logical_cores":18,"metal":true,"os":"macos","physical_cores":18,"total_ram_bytes":137438953472},"mode":"setup","models_dir":"/home/u/.orag/models"}
```

### `POST /v1/models/download`

Setup mode only. Downloads every missing configured model from Hugging Face
and installs it, as `orag models import` would. No body, or `{}`, downloads
the configured models. `{"generation_model": "<id>"}` downloads that
generator instead of the configured one (with the embedder if it is
missing); the id must be one of `GET /v1/models` `download.options` and pass
the same check now (an offered option, `fits` or `recommended`, a supported
machine). Choosing the current option downloads the configured model and
leaves `config.toml` as it is. Once every file passed SHA-256 and was installed, the server sets
only `generation_model = "<id>"` in `<home>/config.toml` (comments and order
kept, written atomically: a temporary file, fsync, rename), and only then
restarts as below; on any failure `config.toml` is left as it was, and a key
that already names the model is not rewritten. It runs
only when a client calls it; the server never downloads on its own. Only the
catalog presets can be downloaded (`qwen3-embedding-0.6b-q8_0`,
`qwen3.5-4b-q4_k_m`, `qwen3.5-2b-q4_k_m`, `qwen3.5-9b-q4_k_m`,
`qwen3-4b-instruct-2507-q4_k_m`), each pinned to the
Hugging Face revision and SHA-256 of `scripts/fetch-model-pack.sh`; every
file is checked by SHA-256 before it is installed. `HTTPS_PROXY`,
`HTTP_PROXY` and `NO_PROXY` are honoured.

The download runs in its own task: the answer comes at once, and progress is
read with `GET /v1/models/download`. Files are written to
`<models_dir>/.download/<id>/` as `<file>.part`; a cancelled or broken
download continues from there on the next start (a Range request). Free disk
space is checked first when the system reports it.

| Status | `code` | When |
|---|---|---|
| 202 | | Started; the body is the status body below |
| 400 | `invalid_input` | A body other than none, `{}` or `{"generation_model": "<id>"}` |
| 400 | `download_unavailable` | A missing configured model is not in the catalog (the message names it); import its pack instead |
| 400 | `unknown_model` | `generation_model` is not an offered tier (the message lists them) |
| 403 | `download_disabled` | `models.download = false` in `config.toml` |
| 409 | `conflict` | Not in setup mode, or a download or an import is already running (they share one slot) |
| 409 | `model_not_eligible` | The tier does not fit this machine now; the message names why (`too_big_ram`, `too_big_disk`, `cpu_unsupported`, `no_tier_fits`) |

```bash
curl -s -X POST http://127.0.0.1:7613/v1/models/download
curl -s -X POST http://127.0.0.1:7613/v1/models/download \
  -H 'Content-Type: application/json' -d '{"generation_model": "qwen3.5-9b-q4_k_m"}'
```

```text
HTTP/1.1 202 Accepted
{"error":null,"models":[{"id":"qwen3-embedding-0.6b-q8_0","received":0,"state":"waiting","total":639161950},{"id":"qwen3.5-4b-q4_k_m","received":0,"state":"waiting","total":2740949432}],"restart":null,"state":"running"}
```

**Status body.**

| Field | Values |
|---|---|
| `state` | `idle` (nothing started since the server started), `running`, `done`, `failed`, `cancelled` |
| `models[]` | one entry per model being downloaded: `id`, `state` (`waiting`, `downloading`, `verifying`, `installed`, `failed`), `received` and `total` bytes |
| `error` | `null`, or `{"code", "message"}` when `state` is `failed` or `cancelled`; the message is plain English and names the host or file |
| `restart` | `null` until `done`; then `"automatic"`, `"manual"` or `"none"`, as for `POST /v1/models/import` |

`error.code` is one of:

| `code` | Meaning |
|---|---|
| `network` | The host could not be reached: no connection within 30 s, or no data for 60 s (check the internet connection or the proxy variables) |
| `http` | Hugging Face answered with an error status |
| `checksum` | A file did not match its SHA-256; its `.part` file is deleted, so the next start fetches it again |
| `disk` | Not enough free space (the message names the folder and the bytes needed), or a write failed |
| `cancelled` | `DELETE /v1/models/download` stopped it |
| `internal` | Any other failure; details are in the server log |
| `config` | The models are installed, but the chosen `generation_model` could not be written into `config.toml` (the message names the file and the cause); nothing restarts. Set the key by hand and restart, or `POST` again |

When every configured model is installed, `state` is `done` and `restart`
says what happens next, exactly as after an import: `"automatic"` under
`orag start` and the desktop app (the server exits with code 75 and is
started again), `"manual"` otherwise (restart ORAG yourself). After a
`failed` or `cancelled` download, `POST` again to continue.

### `GET /v1/models/download`

Always `200` with the status body; `state` is `idle` before any download.

```bash
curl -s http://127.0.0.1:7613/v1/models/download
```

```text
HTTP/1.1 200 OK
{"error":null,"models":[{"id":"qwen3-embedding-0.6b-q8_0","received":639161950,"state":"installed","total":639161950},{"id":"qwen3.5-4b-q4_k_m","received":313673333,"state":"downloading","total":2740949432}],"restart":null,"state":"running"}
```

### `DELETE /v1/models/download`

Asks a running download to stop and answers `200` with the status at that
moment, which may still read `running`; the download stops within a second,
and the status then reads `state` `cancelled` with `error.code` `cancelled`.
The `.part` files are kept, so the next `POST` resumes where it stopped. With
no download running it returns the current status unchanged.

```bash
curl -s -X DELETE http://127.0.0.1:7613/v1/models/download
```

```text
HTTP/1.1 200 OK
{"error":null,"models":[{"id":"qwen3-embedding-0.6b-q8_0","received":639161950,"state":"installed","total":639161950},{"id":"qwen3.5-4b-q4_k_m","received":313673333,"state":"downloading","total":2740949432}],"restart":null,"state":"running"}
```

### `POST /v1/models/import`

Setup mode only: a server that has its models answers `409 conflict` (import
more with `orag models import` and restart). Body
`{"path": "<full folder path on this computer>"}`: a model pack (a folder with
`orag-model.toml`) or a folder of pack folders; `~/` at the start means the
home folder, and a relative path is refused. Each pack is copied into
`models_dir` and verified by SHA-256; one already installed and working is
skipped. The pack folder and its files must be regular files, not symbolic
links, and (on Linux and macOS) owned by the user ORAG runs as. One import
runs at a time (`409 conflict` otherwise), also when the client leaves before
it ends. `400 invalid_input` names the folder or pack when it does not exist,
holds no pack, fails verification, or `models_dir` holds a broken earlier copy
of the same model (delete that folder and import again); a host failure (a
full disk) is `500 internal`.

`restart` says what happens next: `"automatic"` (both models now installed,
the server runs under `orag start`: it exits with code 75 and is started
again), `"manual"` (the same, but nothing restarts it: restart ORAG yourself),
or `"none"` (a configured model is still missing). The server never loads a
model while running.

```bash
curl -s -H 'Content-Type: application/json' -d '{"path":"~/orag-packs"}' \
  http://127.0.0.1:7613/v1/models/import
```

```text
HTTP/1.1 200 OK
{"imported":["qwen3-embedding-0.6b-q8_0","qwen3.5-4b-q4_k_m"],"restart":"automatic"}
```

### `GET /v1/jobs/{job_id}`

`status` is `queued`, `running`, `succeeded` or `failed` (with `error`).

```text
HTTP/1.1 200 OK
{"id":3,"document_id":3,"collection_id":1,"status":"succeeded","error":null,"created_at":"2026-10-05T10:27:16.390Z","updated_at":"2026-10-05T10:27:16.438Z"}
```

### `GET /v1/collections/{collection_id}/documents`

Query parameters `limit` (1–200, default 50) and `after_id` (cursor). The
next page starts at `next_after_id`; it is `null` when the page was not
full (a full last page returns a cursor whose next page is empty). The example
below is `GET /v1/collections/1/documents?limit=2` on a collection with three
documents.
Document `status` is `queued`, `indexing`, `ready` or `failed`.

```text
HTTP/1.1 200 OK
{"documents":[{"chunk_count":1,"collection_id":1,"created_at":"2026-10-05T10:27:16.375Z","error":null,"filename":"sample.docx","format":"docx","id":1,"size_bytes":10992,"source_sha256":"8ccc0ffedf51624176b6f5ff9ea2cee47e5fdfcca476b652732b60e2527560ae","status":"ready","title":"Kargo Politikası","updated_at":"2026-10-05T10:27:16.405Z","warnings":[]},{"chunk_count":1,"collection_id":1,"created_at":"2026-10-05T10:27:16.383Z","error":null,"filename":"sample-tr.pdf","format":"pdf","id":2,"size_bytes":18226,"source_sha256":"f75ffc1a0748a046ef9e675806636b84cebca9665886f04739aff2461d20b9f3","status":"ready","title":"sample-tr.pdf","updated_at":"2026-10-05T10:27:16.437Z","warnings":[]}],"next_after_id":2}
```

### `GET /v1/collections/{collection_id}/documents/{document_id}`

```text
HTTP/1.1 200 OK
{"id":3,"collection_id":1,"filename":"iade.md","format":"markdown","title":"İade","status":"ready","error":null,"chunk_count":1,"warnings":[],"source_sha256":"7c5e9ca194eed1aa1bad0152d71ebec42654115e8082f3dc84a981024ffab25e","size_bytes":30,"created_at":"2026-10-05T10:27:16.390Z","updated_at":"2026-10-05T10:27:16.438Z"}
```

### `DELETE /v1/collections/{collection_id}/documents/{document_id}`

`204 No Content`; an unknown document is `404 not_found`.

### `POST /v1/collections/{collection_id}/query`

Body: `{"query": "<question>", "stream": false}`. The question is 1–2000
characters. ORAG retrieves passages (BM25 + dense, fused), answers only from
them and cites them as `[n]`, where `n` is a source `number`. When the
documents do not support an answer it abstains (`"abstained": true`).

`pages` is the first and last physical page of a PDF source, 1-based
(`[12, 13]`); it is `null` for TXT, Markdown and DOCX (their place is
`heading_path`) and for PDF chunks indexed before 0.2.0-alpha.16, until the
collection is reindexed.

`heading_path` is the headings above the passage, outermost first. Markdown
and DOCX take them from their markup. TXT and PDF have none, so since
0.3.0-alpha.4 heading lines are recognised: lines in capitals (parts and
chapters such as `BİRİNCİ KISIM` or `BÖLÜM 2` rank highest), `-- name --`
lines, and titles numbered with a Roman numeral or a letter (`IV. Egemenlik`,
`A. Özel hayatın gizliliği`). Headers repeated on most pages of a PDF are not
headings (D-024). TXT and PDF chunks indexed before that
keep `[]` until the collection is reindexed.

`rank_score` is a ranking signal (reciprocal rank fusion), **not** a
confidence or a probability. Candidates are taken in fused order, except that
the top hits of the lexical list and of the dense list are offered first: both
lists' #1, then each list's #2-#4 (a chunk too long for the context budget is
still skipped), so `rank_score` does not always decrease along `sources`.

```bash
curl -s -H 'Content-Type: application/json' \
  -d '{"query":"İade süresi?"}' http://127.0.0.1:7613/v1/collections/1/query
```

```json
{"answer":"This is a development answer from fake models [1].",
 "sources":[
  {"number":1,"chunk_id":2,"document_id":2,"title":"sample-tr.pdf","filename":"sample-tr.pdf","heading_path":[],"pages":[1,1],"ordinal":0,"excerpt":"Kargo Politikasi\n\nİade süresi 14 gündür. Iğdır,\n\nşık, çay, öğün.","rank_score":0.03278688524590164,"lexical_rank":1,"dense_rank":1},
  {"number":2,"chunk_id":3,"document_id":3,"title":"İade","filename":"iade.md","heading_path":["İade"],"pages":null,"ordinal":0,"excerpt":"14 gün içinde iade.","rank_score":0.03225806451612903,"lexical_rank":2,"dense_rank":2},
  {"number":3,"chunk_id":1,"document_id":1,"title":"Kargo Politikası","filename":"sample.docx","heading_path":["Kargo Politikası","İade Koşulları"],"pages":null,"ordinal":0,"excerpt":"Ürünler 14 gün içinde iade edilebilir. …","rank_score":0.031746031746031744,"lexical_rank":3,"dense_rank":3}],
 "citations":[1],"invalid_citations":[],"citation_markers":[{"start":46,"end":49,"numbers":[1]}],"abstained":false,"uncited":false,"finish_reason":"stop",
 "trace":{"retrieval":{"strategy":"hybrid","lexical_hits":3,"dense_hits":3,"fused_hits":3,"embed_ms":0,"lexical_ms":0,"dense_ms":0,"embedding_space":"2f000b84a7e48cb77669bd94a0b120ec9445c6c04e11c293ce13a142a9499e41"},
  "context_chunks":3,"skipped_chunks":0,"prompt_tokens":174,"completion_tokens":9,"generation_ms":0,"generator":"fake-generator"}}
```

`citations` are the source numbers the answer cites; `invalid_citations` are
numbers it cited that no source has. `citation_markers` lists every marker in
answer order: `answer[start..end]` is the marker as written (UTF-8 byte
offsets, `end` exclusive, from `[` to `]`; `[1]`, `[1, 3]`, `[1-3]`, `[^1]`)
and `numbers` the source numbers it cites, a range expanded, valid or not.
Markers inside fenced code blocks are not citations. A client can link
citations from this without parsing the answer. `trace` shows how the answer was made.

`finish_reason` says why the answer ended:

| value | meaning |
|---|---|
| `stop` | the model finished the answer (or ORAG abstained without asking it) |
| `length` | the output budget (`max_output_tokens` of the model pack) ran out before the answer ended; it is cut off |
| `repetition` | the model wrote the same line three times in a row and generation was stopped; the answer is incomplete |

`abstained` is `true` when the answer is the refusal sentence (`Bu bilgi
belgelerde bulunamadı.` / `I could not find this in the documents.`), or when
its first sentence says in other words, without a citation, that the sources
do not answer (`Verilen kaynaklarda ... belirtilmemiştir.`, `The sources do not
mention ...`). The model may add cited background after such a sentence; the
answer text is returned unchanged.

`uncited` is `true` when the answer is not an abstention and cites no source
(no `[n]`, or only numbers no source has): nothing in it can be checked
against the documents. The answer is returned unchanged; the web page shows
it with a notice. A refusal the detector does not recognize can also be
`uncited`.

#### Streaming (`"stream": true`)

The response is `text/event-stream`. Events, in order:

1. `sources` (once): `{"sources":[...]}`, the same objects as above;
2. `token` (zero or more): `{"text":"..."}`;
3. `done` (once): the answer without the sources, which already came in the
   `sources` event: `answer`, `citations`, `invalid_citations`,
   `citation_markers`, `abstained`, `uncited`,
   `finish_reason` and `trace`, as in the non-streaming response.

If something fails, the stream ends with one `error` event instead of `done`.
Its data has the same shape as an error response:
`{"error":{"code":"...","message":"..."}}`. An `error` may also be the first and only
event, for example `shutting_down` before sources were sent. A stream that
ends without `done` or `error` was cut off (for example a client that stopped
reading for 30 s). Closing the connection stops generation and frees the
answer slot.

```bash
curl -N -s -H 'Content-Type: application/json' \
  -d '{"query":"İade süresi?","stream":true}' http://127.0.0.1:7613/v1/collections/1/query
```

```text
event: sources
data: {"sources":[{"chunk_id":2,"dense_rank":1,"document_id":2,"excerpt":"Kargo Politikasi\n\nİade süresi 14 gündür. …","filename":"sample-tr.pdf","heading_path":[],"lexical_rank":1,"number":1,"ordinal":0,"pages":[1,1],"rank_score":0.03278688524590164,"title":"sample-tr.pdf"}, …]}

event: token
data: {"text":"This "}

event: token
data: {"text":"is "}

…

event: done
data: {"answer":"This is a development answer from fake models [1].","citations":[1],"invalid_citations":[],"citation_markers":[{"start":46,"end":49,"numbers":[1]}],"abstained":false,"uncited":false,"finish_reason":"stop","trace":{"retrieval":{"strategy":"hybrid","lexical_hits":3,"dense_hits":3,"fused_hits":3,"embed_ms":0,"lexical_ms":0,"dense_ms":0,"embedding_space":"2f000b84a7e48cb77669bd94a0b120ec9445c6c04e11c293ce13a142a9499e41"},"context_chunks":3,"skipped_chunks":0,"prompt_tokens":174,"completion_tokens":9,"generation_ms":0,"generator":"fake-generator"}}
```

Errors before streaming starts (unknown collection, invalid query,
`reindex_required`, `busy`) are ordinary JSON error responses with their
status code, not events.
