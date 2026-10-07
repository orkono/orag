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
  `403 forbidden_origin`.
- Request and response bodies are JSON (`Content-Type: application/json`),
  except multipart uploads and the SSE query stream.

## Formats and limits

| Item | Value |
|---|---|
| Supported formats | TXT (`.txt`), Markdown (`.md`, `.markdown`), DOCX (`.docx`), PDF (`.pdf`) |
| Document size | `max_document_mb` in `config.toml`: default 5, allowed 1–10 |
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
| `max_document_mb` | `5` | Largest document accepted, 1–10 MB |
| `embedding_model` | `"qwen3-embedding-0.6b-q8_0"` | Installed embedding pack id |
| `generation_model` | `"qwen3.5-4b-q4_k_m"` | Installed generation pack id |
| `log_level` | `"info"` | `error`, `warn`, `info`, `debug` or `trace` (stderr) |

`GET /v1/version` shows the values in effect (`bind` and `ui_bind` with the
real port).

## Built-in web page

`orag serve` also serves a small page on `ui_bind` and prints
`orag ui on http://127.0.0.1:2442` as its second stdout line (the first line,
`orag listening on ...`, is unchanged). The page uses this same API on its own
origin (collections, uploads, jobs, queries; it adds no endpoint of its own):
`GET /`, `/app.js`, `/app.css` and `/favicon.svg` are served there with
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
| 403 | `forbidden_host` | `Host` is not a loopback address |
| 403 | `forbidden_origin` | A browser cross-origin request |
| 404 | `not_found` | Unknown collection, document, job or route |
| 405 | `method_not_allowed` | The route exists, the method does not (`Allow` header lists methods) |
| 408 | `upload_timeout` | The upload body did not arrive within 60 s |
| 409 | `conflict` | E.g. deleting the default collection |
| 409 | `reindex_required` | The collection was indexed in a different embedding space: another embedding model, or a release that changed the chunker (see below) |
| 413 | `too_large` | Document over `max_document_mb`, or a request body over its limit |
| 415 | `unsupported_format` | A document type ORAG does not support |
| 415 | `unsupported_media_type` | A JSON route called without `Content-Type: application/json` |
| 429 | `busy` | 4 uploads in progress, or the answer slot stayed taken for 120 s. Carries `Retry-After: 5` |
| 500 | `internal` | A server-side failure; details are in the server log, not the response |
| 503 | `shutting_down` | The service is stopping and starts no new work |

Client timeouts for queries should exceed 120 s plus the answer time, because
a query waits for the single answer slot before it gets `busy`.

### `reindex_required` (409): recovery in v0.1

A collection keeps the embedding space it was indexed with. If the embedding
model, or anything else in that space, changed, queries and uploads on that
collection return `409 reindex_required`. Automatic reindexing arrives in
v0.2. In v0.1:

1. **Pin the previous model.** Set `embedding_model` back to the previous pack
   id in `config.toml` and restart. This works only if nothing else in the
   embedding space changed: the space fingerprint covers the model id and file
   checksum, pooling, query and document prefixes, dimensions, normalization,
   token limit, the trailing-EOS rule, the chunker version and the fingerprint
   encoding (D-009). A change of the lexical search (normalizer, indexed text)
   is not part of it: ORAG rebuilds the full-text index at startup instead. A release that changes any of them says
   so in an **Upgrade note** in `CHANGELOG.md`; then pinning is not enough.
2. **Otherwise re-upload.** A collection with no documents follows the
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

`rank_score` is a ranking signal (reciprocal rank fusion), **not** a
confidence or a probability. Candidates are taken in fused order, except that
the top hit of the lexical list and of the dense list are offered first (a
chunk too long for the context budget is still skipped), so `rank_score` does
not always decrease along `sources`.

```bash
curl -s -H 'Content-Type: application/json' \
  -d '{"query":"İade süresi?"}' http://127.0.0.1:7613/v1/collections/1/query
```

```json
{"answer":"This is a development answer from fake models [1].",
 "sources":[
  {"number":1,"chunk_id":2,"document_id":2,"title":"sample-tr.pdf","filename":"sample-tr.pdf","heading_path":[],"ordinal":0,"excerpt":"Kargo Politikasi\n\nİade süresi 14 gündür. Iğdır,\n\nşık, çay, öğün.","rank_score":0.03278688524590164,"lexical_rank":1,"dense_rank":1},
  {"number":2,"chunk_id":3,"document_id":3,"title":"İade","filename":"iade.md","heading_path":["İade"],"ordinal":0,"excerpt":"14 gün içinde iade.","rank_score":0.03225806451612903,"lexical_rank":2,"dense_rank":2},
  {"number":3,"chunk_id":1,"document_id":1,"title":"Kargo Politikası","filename":"sample.docx","heading_path":["Kargo Politikası","İade Koşulları"],"ordinal":0,"excerpt":"Ürünler 14 gün içinde iade edilebilir. …","rank_score":0.031746031746031744,"lexical_rank":3,"dense_rank":3}],
 "citations":[1],"invalid_citations":[],"abstained":false,"finish_reason":"stop",
 "trace":{"retrieval":{"strategy":"hybrid","lexical_hits":3,"dense_hits":3,"fused_hits":3,"embed_ms":0,"lexical_ms":0,"dense_ms":0,"embedding_space":"2f000b84a7e48cb77669bd94a0b120ec9445c6c04e11c293ce13a142a9499e41"},
  "context_chunks":3,"skipped_chunks":0,"prompt_tokens":174,"completion_tokens":9,"generation_ms":0,"generator":"fake-generator"}}
```

`citations` are the source numbers the answer cites; `invalid_citations` are
numbers it cited that no source has. `trace` shows how the answer was made.

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

#### Streaming (`"stream": true`)

The response is `text/event-stream`. Events, in order:

1. `sources` (once): `{"sources":[...]}`, the same objects as above;
2. `token` (zero or more): `{"text":"..."}`;
3. `done` (once): the answer without the sources, which already came in the
   `sources` event: `answer`, `citations`, `invalid_citations`, `abstained`,
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
data: {"sources":[{"chunk_id":2,"dense_rank":1,"document_id":2,"excerpt":"Kargo Politikasi\n\nİade süresi 14 gündür. …","filename":"sample-tr.pdf","heading_path":[],"lexical_rank":1,"number":1,"ordinal":0,"rank_score":0.03278688524590164,"title":"sample-tr.pdf"}, …]}

event: token
data: {"text":"This "}

event: token
data: {"text":"is "}

…

event: done
data: {"answer":"This is a development answer from fake models [1].","citations":[1],"invalid_citations":[],"abstained":false,"finish_reason":"stop","trace":{"retrieval":{"strategy":"hybrid","lexical_hits":3,"dense_hits":3,"fused_hits":3,"embed_ms":0,"lexical_ms":0,"dense_ms":0,"embedding_space":"2f000b84a7e48cb77669bd94a0b120ec9445c6c04e11c293ce13a142a9499e41"},"context_chunks":3,"skipped_chunks":0,"prompt_tokens":174,"completion_tokens":9,"generation_ms":0,"generator":"fake-generator"}}
```

Errors before streaming starts (unknown collection, invalid query,
`reindex_required`, `busy`) are ordinary JSON error responses with their
status code, not events.
