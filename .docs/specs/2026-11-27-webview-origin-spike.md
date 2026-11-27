# Webview origin spike

Date: 2026-11-27. Branch `spike/webview-origin` (never merged). Workflow run: https://github.com/orkono/orag/actions/runs/45969795371
(the first run, 45969134355, failed only on macOS because the runner has no `timeout`; fixed in the second).

## Question

Can a page bundled in the Tauri 2.12 desktop app (default origins, CSP `connect-src http://127.0.0.1:47613`) call a loopback HTTP API cross-origin: (a) GET, (b) JSON POST, (c) multipart upload, (d) DELETE, (e) POST stream read with `getReader()`, (f) stream aborted after 2 events then (a) again? The server answers CORS with `Access-Control-Allow-Origin` echoing `Origin`, exposes `Retry-After`, and answers a private-network preflight only if asked.

## Result summary

| OS / webview | Origin | Sec-Fetch-Site | Preflighted | Private-network preflight asked | (a)-(f) ok | Retry-After readable |
|---|---|---|---|---|---|---|
| macOS 15 (CI, WKWebView) | `tauri://localhost` | cross-site | /item/1, /json, /report | no | yes | yes (7) |
| macOS 26 Apple Silicon (local, WKWebView) | `tauri://localhost` | cross-site | /item/1, /json, /report | no | yes | yes (7) |
| Ubuntu 22.04 (CI, WebKitGTK 4.1 under xvfb) | `tauri://localhost` | cross-site | /item/1, /json, /report | no | yes | yes (7) |
| Windows Server 2022 (CI, WebView2) | `http://tauri.localhost` | cross-site | /item/1, /json, /report | no | yes | yes (7) |

(a)-(f) succeeded means all seven page steps (a, b, c, d, e, f1, f2) returned `ok` with status 200; (e) read 5 events and (f1) stopped cleanly after 2 events on abort. Preflights occurred for the JSON POST (`content-type`), DELETE and the final `/report` POST; GET, multipart upload and the header-less stream POST are simple requests. No webview asked for private network access.

## Raw logged headers

Only non-null headers are shown; the full JSON-lines logs are in the workflow run artifacts.

### macOS 15 (CI, WKWebView)

```
GET /json  {"Origin": "tauri://localhost", "Sec-Fetch-Site": "cross-site"}
OPTIONS /json  {"Origin": "tauri://localhost", "Sec-Fetch-Site": "cross-site", "Access-Control-Request-Method": "POST", "Access-Control-Request-Headers": "content-type"}
POST /json  {"Origin": "tauri://localhost", "Sec-Fetch-Site": "cross-site"}
POST /upload  {"Origin": "tauri://localhost", "Sec-Fetch-Site": "cross-site"}
OPTIONS /item/1  {"Origin": "tauri://localhost", "Sec-Fetch-Site": "cross-site", "Access-Control-Request-Method": "DELETE"}
DELETE /item/1  {"Origin": "tauri://localhost", "Sec-Fetch-Site": "cross-site"}
POST /stream  {"Origin": "tauri://localhost", "Sec-Fetch-Site": "cross-site"}
POST /stream  {"Origin": "tauri://localhost", "Sec-Fetch-Site": "cross-site"}
GET /json  {"Origin": "tauri://localhost", "Sec-Fetch-Site": "cross-site"}
OPTIONS /report  {"Origin": "tauri://localhost", "Sec-Fetch-Site": "cross-site", "Access-Control-Request-Method": "POST", "Access-Control-Request-Headers": "content-type"}
POST /report  {"Origin": "tauri://localhost", "Sec-Fetch-Site": "cross-site"}
REPORT origin=tauri://localhost ua=Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko)
  {"name": "a GET /json", "ok": true, "status": 200, "retryAfter": "7", "bodyReadable": true}
  {"name": "b POST /json", "ok": true, "status": 200, "retryAfter": "7", "bodyReadable": true}
  {"name": "c POST /upload", "ok": true, "status": 200, "retryAfter": "7", "bodyReadable": true}
  {"name": "d DELETE /item/1", "ok": true, "status": 200, "retryAfter": "7", "bodyReadable": true}
  {"name": "e POST /stream", "ok": true, "status": 200, "retryAfter": null, "extra": {"events": 5}}
  {"name": "f1 stream aborted after 2", "ok": true, "status": 200, "retryAfter": null, "extra": {"events": 2}}
  {"name": "f2 GET /json again", "ok": true, "status": 200, "retryAfter": "7", "bodyReadable": true}
```

### macOS 26 Apple Silicon (local, WKWebView)

```
GET /json  {"Origin": "tauri://localhost", "Sec-Fetch-Site": "cross-site"}
OPTIONS /json  {"Origin": "tauri://localhost", "Sec-Fetch-Site": "cross-site", "Access-Control-Request-Method": "POST", "Access-Control-Request-Headers": "content-type"}
POST /json  {"Origin": "tauri://localhost", "Sec-Fetch-Site": "cross-site"}
POST /upload  {"Origin": "tauri://localhost", "Sec-Fetch-Site": "cross-site"}
OPTIONS /item/1  {"Origin": "tauri://localhost", "Sec-Fetch-Site": "cross-site", "Access-Control-Request-Method": "DELETE"}
DELETE /item/1  {"Origin": "tauri://localhost", "Sec-Fetch-Site": "cross-site"}
POST /stream  {"Origin": "tauri://localhost", "Sec-Fetch-Site": "cross-site"}
POST /stream  {"Origin": "tauri://localhost", "Sec-Fetch-Site": "cross-site"}
GET /json  {"Origin": "tauri://localhost", "Sec-Fetch-Site": "cross-site"}
OPTIONS /report  {"Origin": "tauri://localhost", "Sec-Fetch-Site": "cross-site", "Access-Control-Request-Method": "POST", "Access-Control-Request-Headers": "content-type"}
POST /report  {"Origin": "tauri://localhost", "Sec-Fetch-Site": "cross-site"}
REPORT origin=tauri://localhost ua=Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko)
  {"name": "a GET /json", "ok": true, "status": 200, "retryAfter": "7", "bodyReadable": true}
  {"name": "b POST /json", "ok": true, "status": 200, "retryAfter": "7", "bodyReadable": true}
  {"name": "c POST /upload", "ok": true, "status": 200, "retryAfter": "7", "bodyReadable": true}
  {"name": "d DELETE /item/1", "ok": true, "status": 200, "retryAfter": "7", "bodyReadable": true}
  {"name": "e POST /stream", "ok": true, "status": 200, "retryAfter": null, "extra": {"events": 5}}
  {"name": "f1 stream aborted after 2", "ok": true, "status": 200, "retryAfter": null, "extra": {"events": 2}}
  {"name": "f2 GET /json again", "ok": true, "status": 200, "retryAfter": "7", "bodyReadable": true}
```

### Ubuntu 22.04 (CI, WebKitGTK 4.1 under xvfb)

```
GET /json  {"Origin": "tauri://localhost", "Sec-Fetch-Site": "cross-site"}
OPTIONS /json  {"Origin": "tauri://localhost", "Sec-Fetch-Site": "cross-site", "Access-Control-Request-Method": "POST", "Access-Control-Request-Headers": "content-type"}
POST /json  {"Origin": "tauri://localhost", "Sec-Fetch-Site": "cross-site"}
POST /upload  {"Origin": "tauri://localhost", "Sec-Fetch-Site": "cross-site"}
OPTIONS /item/1  {"Origin": "tauri://localhost", "Sec-Fetch-Site": "cross-site", "Access-Control-Request-Method": "DELETE"}
DELETE /item/1  {"Origin": "tauri://localhost", "Sec-Fetch-Site": "cross-site"}
POST /stream  {"Origin": "tauri://localhost", "Sec-Fetch-Site": "cross-site"}
POST /stream  {"Origin": "tauri://localhost", "Sec-Fetch-Site": "cross-site"}
GET /json  {"Origin": "tauri://localhost", "Sec-Fetch-Site": "cross-site"}
OPTIONS /report  {"Origin": "tauri://localhost", "Sec-Fetch-Site": "cross-site", "Access-Control-Request-Method": "POST", "Access-Control-Request-Headers": "content-type"}
POST /report  {"Origin": "tauri://localhost", "Sec-Fetch-Site": "cross-site"}
REPORT origin=tauri://localhost ua=Mozilla/5.0 (X11; Ubuntu; Linux x86_64) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/60.5 Safari/605.1.15
  {"name": "a GET /json", "ok": true, "status": 200, "retryAfter": "7", "bodyReadable": true}
  {"name": "b POST /json", "ok": true, "status": 200, "retryAfter": "7", "bodyReadable": true}
  {"name": "c POST /upload", "ok": true, "status": 200, "retryAfter": "7", "bodyReadable": true}
  {"name": "d DELETE /item/1", "ok": true, "status": 200, "retryAfter": "7", "bodyReadable": true}
  {"name": "e POST /stream", "ok": true, "status": 200, "retryAfter": null, "extra": {"events": 5}}
  {"name": "f1 stream aborted after 2", "ok": true, "status": 200, "retryAfter": null, "extra": {"events": 2}}
  {"name": "f2 GET /json again", "ok": true, "status": 200, "retryAfter": "7", "bodyReadable": true}
```

### Windows Server 2022 (CI, WebView2)

```
GET /json  {"Origin": "http://tauri.localhost", "Sec-Fetch-Site": "cross-site"}
OPTIONS /json  {"Origin": "http://tauri.localhost", "Sec-Fetch-Site": "cross-site", "Access-Control-Request-Method": "POST", "Access-Control-Request-Headers": "content-type"}
POST /json  {"Origin": "http://tauri.localhost", "Sec-Fetch-Site": "cross-site"}
POST /upload  {"Origin": "http://tauri.localhost", "Sec-Fetch-Site": "cross-site"}
OPTIONS /item/1  {"Origin": "http://tauri.localhost", "Sec-Fetch-Site": "cross-site", "Access-Control-Request-Method": "DELETE"}
DELETE /item/1  {"Origin": "http://tauri.localhost", "Sec-Fetch-Site": "cross-site"}
POST /stream  {"Origin": "http://tauri.localhost", "Sec-Fetch-Site": "cross-site"}
POST /stream  {"Origin": "http://tauri.localhost", "Sec-Fetch-Site": "cross-site"}
GET /json  {"Origin": "http://tauri.localhost", "Sec-Fetch-Site": "cross-site"}
OPTIONS /report  {"Origin": "http://tauri.localhost", "Sec-Fetch-Site": "cross-site", "Access-Control-Request-Method": "POST", "Access-Control-Request-Headers": "content-type"}
POST /report  {"Origin": "http://tauri.localhost", "Sec-Fetch-Site": "cross-site"}
REPORT origin=http://tauri.localhost ua=Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36 Edg/131.0.0.0
  {"name": "a GET /json", "ok": true, "status": 200, "retryAfter": "7", "bodyReadable": true}
  {"name": "b POST /json", "ok": true, "status": 200, "retryAfter": "7", "bodyReadable": true}
  {"name": "c POST /upload", "ok": true, "status": 200, "retryAfter": "7", "bodyReadable": true}
  {"name": "d DELETE /item/1", "ok": true, "status": 200, "retryAfter": "7", "bodyReadable": true}
  {"name": "e POST /stream", "ok": true, "status": 200, "retryAfter": null, "extra": {"events": 5}}
  {"name": "f1 stream aborted after 2", "ok": true, "status": 200, "retryAfter": null, "extra": {"events": 2}}
  {"name": "f2 GET /json again", "ok": true, "status": 200, "retryAfter": "7", "bodyReadable": true}
```

## Outcome: A (exact origins)

Every webview sent a stable, usable origin: `tauri://localhost` on macOS and Linux, `http://tauri.localhost` on Windows (default, non-HTTPS scheme). Each call was ordinary cross-origin (`Sec-Fetch-Site: cross-site`), preflights worked with a plain 204 and an echoed `Access-Control-Allow-Origin`, `Retry-After` was readable because of `Access-Control-Expose-Headers`, streaming reads and abort worked, and no webview sent a private-network preflight header. None sent `null`. The plan therefore continues as written: the engine allow-list can name these exact origins. The Linux result is under xvfb on a CI image; the Windows one is WebView2 on a Server runner, so real-desktop behaviour on older WebView2 or WebKitGTK builds was not exercised.
