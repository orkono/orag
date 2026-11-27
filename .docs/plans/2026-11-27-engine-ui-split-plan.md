# Engine and UI Split Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `orag` becomes the engine only (no page, no installers, a build guide), and the page, desktop app and installers live in `github.com/orkono/orag-ui`, which pins the engine as a git submodule at a tag.

**Architecture:** Staged so both repositories work at every step. First a webview spike settles how the app's page reaches the engine (gate). Then the engine gains `--allow-origin` and CORS while still serving its page, and gets a written interface, and that is tagged. `orag-ui` is created from that tag with history, the page moves into the app and calls the engine cross-origin, and CI and release move with it. Only then does the engine drop its page and the product files, and `orag-ui` moves its pin to the page-less tag.

**Tech Stack:** Rust 2024 (toolchain 1.98.1), axum 0.8, clap, Tauri 2.12, plain ES modules with Node's built-in test runner, GitHub Actions, `git filter-repo`.

**Status:** approved 2026-11-27 by muse (5 rounds, 12 changes folded in) and gpt-6-astra (4 rounds, 7 changes folded in).

**Spec:** `.docs/specs/2026-11-26-engine-ui-split-design.md` (approved 2026-11-27 by muse and gpt-6-astra). Items cited as "spec 15" refer to its numbered "Decided" list.

## Global Constraints

- License `MIT OR Apache-2.0` in both repositories.
- D-012 offline, D-013 loopback only with Host and Origin checks, D-019 `config.toml` read once with unknown keys refused: all still hold. Explicit origins only, never a wildcard.
- D-017 in each repository: every merged change runs `scripts/bump-version.sh <ver> <bullet>` and has a CHANGELOG entry. `orag` continues from `0.6.0-alpha.15` (this plan's own commit). `orag-ui` inherits the version at its import tag and bumps from there with its own copy of the version scripts. This is a builder call: spec 12 says "both start from alpha.13", but `orag` has moved since.
- `0.6.0-alpha.12` and every existing `orag` release and asset stay untouched (spec 7).
- The app keeps the product name `ORAG`, the identifier `com.orkono.orag` and its data folder (spec 12).
- Origins the app names: `tauri://localhost` (macOS, Linux) and `http://tauri.localhost` (Windows), unless Task 1 selects the fallback.
- The `/v1` API is not broken. `GET /v1/version` keeps `config.ui_bind`, always `"off"`, through 0.6 (spec 15 and Engine changes).
- `serve --ui-port-hint <port>` stays accepted and ignored through the 0.6 line (spec 15).
- Every engine version `orag-ui` pins is a `v*` tag (spec 16).
- Language rules: chat with the owner in Turkish. Everything in the repositories is in English.

## Review Focus

1. **An existing desktop install's `config.toml`** has `bind = "127.0.0.1:0"` and `ui_bind = "127.0.0.1:0"`. Expected: the page-less engine starts, logs one warning, and the app works. Pinned in Task 11 (config test) and Task 12 (desktop smoke with that file).
2. **The API port changes on every engine restart**: `EXIT_RESTART` after setup or a model download, exit 76 for GPU retry, a crash restart. Expected: the page reloads with the new address, not the old one. Pinned in Task 7 (`a_restart_hands_the_page_the_new_api_port`).
3. **A 429 `busy` read cross-origin.** Without `Access-Control-Expose-Headers: Retry-After` the page cannot see the wait time (`format.js:116`), so it retries at once. Pinned in Task 2 (`cors_exposes_retry_after`).
4. **Another web page in the user's browser** calls the loopback API. Expected: 403 for the request and its preflight, and no `Access-Control-Allow-Origin` on any response. Pinned in Task 2.
5. **Stopping an answer mid-stream**: the SSE fetch is aborted across origins. Expected: the stream ends cleanly and the next question works. Pinned in Task 1 (the spike records it) and Task 9 (desktop smoke).

---

## Phase A — Gate

### Task 1: Webview origin spike (throwaway)

**Files (in `orag-ui`, branch `spike/webview-origin`, never merged):**
- Create: `spike/src-tauri/` (a minimal Tauri 2.12 app: one window, `frontendDist = "../page"`, CSP `default-src 'self'; connect-src http://127.0.0.1:47613`)
- Create: `spike/page/index.html`, `spike/page/spike.js`
- Create: `spike/server.py` (stdlib `http.server` on `127.0.0.1:47613`)
- Create: `.github/workflows/spike.yml` (matrix `macos-15`, `ubuntu-22.04` under `xvfb-run` with `libwebkit2gtk-4.1-dev`, `windows-2022`)

**Record in `orag`:**
- Create: `.docs/specs/2026-11-27-webview-origin-spike.md`

- [ ] **Step 1: Write `server.py`.** It logs every request as one JSON line to `spike-log.jsonl`: method, path, and the `Origin`, `Sec-Fetch-Site`, `Access-Control-Request-Method`, `Access-Control-Request-Headers` and `Access-Control-Request-Private-Network` headers.
  - It answers `OPTIONS` with 204 and echoes `Access-Control-Allow-Origin: <Origin>`. It also sends `Access-Control-Allow-Methods: GET, POST, DELETE`, `Access-Control-Allow-Headers: content-type, accept`, `Access-Control-Expose-Headers: Retry-After` and, when the preflight asked for it, `Access-Control-Allow-Private-Network: true`.
  - It answers every other request with those headers, plus `Retry-After: 7` and a small JSON body.
  - `/stream` answers `text/event-stream` with 5 events, 300 ms apart.
  - `POST /report` stores the page's results and exits the server.
- [ ] **Step 2: Write `spike.js`.** It runs the requests below in order. For each one it records `ok`, `status`, whether the body was readable, and the `Retry-After` value read through `response.headers`. It then POSTs everything to `/report`.
  - (a) `GET /json` with `Accept: application/json`
  - (b) `POST /json` with `Content-Type: application/json`
  - (c) `POST /upload` with a `FormData` file
  - (d) `DELETE /item/1`
  - (e) `POST /stream`, read through `response.body.getReader()`
  - (f) the same stream, aborted with `AbortController` after 2 events, then (a) again
- [ ] **Step 3: Write the app.** The app closes itself 60 s after start. The workflow job starts `server.py` in the background, runs the built app binary (`cargo build` in `spike/src-tauri`, not a bundle) with a 90 s limit, then stops the server whether or not `/report` arrived. It prints `spike-log.jsonl` and uploads it in an `if: always()` step. A missing report is itself a result (Outcome B), not a hang.
- [ ] **Step 4: Run on all three runners, and locally on macOS.** Expected: the workflow passes, and each OS uploads a log containing the page's report.
- [ ] **Step 5: Write the spike record.** For each OS, record the exact `Origin` and `Sec-Fetch-Site` values, which requests were preflighted, whether a preflight asked for private-network access, and whether (a) to (f) succeeded. Then pick one outcome:
  - **Outcome A (exact origins):** every webview sends a usable, stable origin and (a) to (f) succeed. Continue with this plan as written.
  - **Outcome B (fallback):** some webview sends `null` or an opaque origin, or blocks the loopback call. The app then serves its bundled page from its own loopback listener (`http://127.0.0.1:<port>`), and names that origin with `--allow-origin`. Tasks 2 to 5 and 8 to 12 do not change. Before starting Task 6, rewrite Tasks 6 and 7 for the app listener and get the change approved through the owner decision protocol.
- [ ] **Step 6: Commit the record in `orag`.** Tag nothing.

```bash
git add .docs/specs/2026-11-27-webview-origin-spike.md
git commit -m "docs: record the webview origin spike"
```

---

## Phase B — Engine additions (`orag`, additive; the page still works)

### Task 2: `serve --allow-origin` and CORS for named origins

**Files:**
- Create: `crates/orag/src/server/cors.rs`
- Modify: `crates/orag/src/cli.rs` (the `Serve` variant, around `:29-41`)
- Modify: `crates/orag/src/app.rs` (`ServeOptions`; the allowed origins, around `:133-137`)
- Modify: `crates/orag/src/server/mod.rs` (`with_layers`; `AppState::allowed_origins`, `:80`)
- Modify: `crates/orag/src/server/security.rs` (preflight passes the origin check only for allowed origins)
- Test: `crates/orag/tests/api/cors.rs` (register it in `tests/api/main.rs`), plus a CLI parse test in `crates/orag/tests/cli.rs`

**Interfaces:**
- Produces:
  - `serve --allow-origin <ORIGIN>`, repeatable, visible in `--help`. Each value must parse as a URL with a scheme and host and no path beyond `/`. Anything else is a usage error that names the flag.
  - `ServeOptions.allow_origins: Vec<String>`
  - `pub fn cors_layer(...)` in `cors.rs`, applied to the API router and to the UI router.

- [ ] **Step 1: Write the failing tests in `tests/api/cors.rs`.** Use `support::app()` with allowed origins `["tauri://localhost"]`, set through `map_state`.
  - `preflight_from_allowed_origin_is_answered`: `OPTIONS /v1/collections` with `Origin: tauri://localhost`, `Access-Control-Request-Method: POST`, `Access-Control-Request-Headers: content-type` → 204. Exact headers: `access-control-allow-origin: tauri://localhost`, `access-control-allow-methods: GET, POST, DELETE`, `access-control-allow-headers: accept, content-type`, `access-control-max-age: 600`, `vary: origin`.
  - `simple_response_carries_allow_origin`: `GET /v1/health` with that Origin → 200 and `access-control-allow-origin: tauri://localhost`.
  - `cors_exposes_retry_after`: any allowed-origin response carries `access-control-expose-headers: retry-after`.
  - `other_origin_gets_403_and_no_cors_headers`: `GET` and `OPTIONS` with `Origin: http://evil.example` → 403, and no `access-control-*` header at all.
  - `no_origin_gets_no_cors_headers`: `GET /v1/health` with no Origin → 200 without `access-control-allow-origin`.
  - `sse_answer_streams_to_allowed_origin`: `POST /v1/collections/{id}/query` (route at `server/mod.rs:297`) with a streaming request and the allowed Origin → 200, `text/event-stream`, and the allow-origin header present.
  - `private_network_preflight_is_allowed_for_named_origins`: a preflight from the allowed origin with `Access-Control-Request-Private-Network: true` gets `access-control-allow-private-network: true`. The same preflight from `http://evil.example` gets 403 without it.
- [ ] **Step 2: Confirm the tests fail.** Run `cargo test --locked --test api cors`. Expected: FAIL. The preflight answers 405, and no allow-origin header is sent.
- [ ] **Step 3: Implement `cors.rs`.** It is a middleware after `check_host_and_origin`.
  - A request with an allowed `Origin`: answer `OPTIONS` itself with 204 and the headers above, plus `access-control-allow-private-network: true` when the preflight asks for it. Otherwise run the request and add `access-control-allow-origin` (the request's Origin, as sent), `access-control-expose-headers: retry-after` and `vary: origin`.
  - Anything else passes through untouched.
  - Reuse `security::origin_allowed`.
  - In `app.rs`, extend `state.allowed_origins` with `options.allow_origins`, before the UI origins. Values are normalised the same way `origin_allowed` compares them.
- [ ] **Step 4: Add the CLI tests in `tests/cli.rs`.**
  - `serve_rejects_a_malformed_allow_origin`: `orag serve --allow-origin "not a url"` exits 2, and stderr contains `--allow-origin`.
  - `serve_rejects_an_allow_origin_with_a_path`: `orag serve --allow-origin "http://x/y"` exits 2.
- [ ] **Step 5: Confirm the tests pass.** Run `cargo test --locked --test api cors && cargo test --locked --test cli allow_origin`. Expected: PASS.
- [ ] **Step 6: Run the full checks.** Run `cargo fmt --all --check && cargo clippy --locked --all-targets -- -D warnings && cargo test --locked`. Expected: PASS.
- [ ] **Step 7: Document the flag in `docs/api.md`.** In its security section: named origins and the headers they get.
- [ ] **Step 8: Bump, commit, merge.** Run `scripts/bump-version.sh 0.6.0-alpha.16 "serve --allow-origin: CORS for origins the starting program names (engine and UI split)"`, then `git add` the files above plus `Cargo.toml Cargo.lock CHANGELOG.md` and commit `feat: serve --allow-origin and CORS for named origins`. Get the code review (astra through codex-orkono), then merge.

### Task 3: Written engine interface, checked against the CLI

**Files:**
- Create: `docs/engine-interface.md`
- Create: `crates/orag/tests/interface.rs`
- Modify: `crates/orag/src/cli.rs` (drop `hide = true` from flags the app relies on: `--exit-on-stdin-close`, the `gpu-probe` subcommand, and the eval flags listed below. Keep `--dev-fake-models` and `--ui-port-hint` hidden.)
- Modify: `docs/api.md`: document `label_key` (a stable message key a client may translate) and `selected`, with spec 15's wording copied verbatim: "the configured generator when it is eligible, else the recommended option, else the first eligible one; distinct from `recommended`". Also `POST /v1/system/gpu-retry`.

**Interfaces:**
- Produces: `docs/engine-interface.md`, with one markdown table per section. The flag table has columns `Command | Flag | Since | Meaning`, one row per flag, and the flag written in backticks as `--name`. Task 8's checker and this task's test parse that table.

Sections, built from the call sites in `apps/desktop/src/` (`supervisor.rs:50-70`, `gpu.rs`, `bench/run.rs:70-115`), not from memory:
1. Starting: `orag serve` and its flags `--exit-on-stdin-close`, `--allow-origin`, `--ui-port-hint` (ignored, through 0.6).
2. Stdout: the first line `orag listening on http://<addr>` (D-016). The second line `orag ui on …` is deprecated: it disappears in the next tag and must not be relied on.
3. Exit codes: 0; 75 `EXIT_RESTART` (setup finished, start again); 76 `EXIT_GPU_RETRY`; anything else is a failure.
4. Environment: `ORAG_HOME` for people. The supervisor protocol: `ORAG_SUPERVISED`, `ORAG_GPU_RETRY`, `ORAG_GPU_FALLBACK_REASON`, `ORAG_GPU_DEVICE`, `ORAG_VULKAN_ALLOW_SOFTWARE`, each with who sets it and what the engine does with it.
5. `config.toml`: a link to the config table, and the rule that a starter must not rely on new keys reaching an existing file.
6. `gpu-probe` output: the JSON line's fields.
7. `eval bench` and `eval retrieval`: every flag the bench uses (`--build-index`, `--index-only`, `--tier`, `--index-dir`, `--first-only`, `--no-context-budget`, `--gpu-auto`, `--corpus`, `--dataset`, `--out`, `--exit-on-stdin-close`) and the output files.
8. Compatibility promise. Within a minor line, flags, exit codes, variables and the first stdout line only gain options. A removal needs a minor version bump and a CHANGELOG line marked **interface**.

- [ ] **Step 1: Write the failing test in `tests/interface.rs`.**
  - `every_documented_flag_exists`: read `docs/engine-interface.md`, collect `(command, flag)` from the flag table, and assert that `orag::cli::Cli::command()` finds the subcommand path and the long flag for each. Hidden flags count as existing.
  - `every_interface_flag_is_visible_except_deprecated`: each documented flag whose Meaning does not start with "Deprecated" is not hidden (`Arg::is_hide_set()` false).
- [ ] **Step 2: Confirm it fails.** Run `cargo test --locked --test interface`. Expected: FAIL, because the document does not exist.
- [ ] **Step 3: Write the document and unhide the flags.** If `Cli` is not reachable from tests, make it `pub` through `lib.rs`.
- [ ] **Step 4: Confirm it passes.** Run `cargo test --locked --test interface && cargo test --locked`. Expected: PASS.
- [ ] **Step 5: Bump, commit, review, merge.** Bump to `0.6.0-alpha.17` and commit `docs: the engine interface for programs that start orag`.

### Task 4: Build guide and the first engine tag for `orag-ui`

**Files:**
- Create: `docs/building.md`
- Modify: `README.md` (link the guide)
- Modify: `.github/workflows/release.yml`
  - replace the `build` and `draft` jobs with one `notes` job on `ubuntu-22.04`, which runs only when `startsWith(github.ref, 'refs/tags/v') && !contains(github.ref_name, '-')` (the workflow also runs on `workflow_dispatch` and `release-check/**` branches, `release.yml:5-9`), with `permissions: contents: write`. The workflow's top level stays `contents: read` (`release.yml:13`).
  - it extracts the tag's CHANGELOG section and runs `gh release create "$TAG" --notes-file notes.md --verify-tag` with `GH_TOKEN: ${{ github.token }}`, and no assets (spec 16)
  - this comes here, before the first tag `orag-ui` pins, so no engine tag builds assets again

- [ ] **Step 1: Write `docs/building.md`.** Cover Linux, macOS and Windows; the CPU build (`cargo build --release --locked`) and Vulkan (`scripts/build-vulkan.sh --release`); the toolchain (`rust-toolchain.toml` 1.98.1, CMake, a C/C++ compiler, MSVC on Windows, the Vulkan SDK 1.4.363.0 for Vulkan). The platform floors come from `scripts/platform-floors.sh` (macOS 14, Windows x86-64 AVX2/FMA/F16C/BMI2, glibc 2.35), and the guide says why `.cargo/config.toml` forces them. Also cover how to check a build (`scripts/check-llama-build.sh --release`, `scripts/check-binary-deps.sh`), running `orag start` on a self-built engine, and moving `~/.orag` (`%USERPROFILE%\.orag`) into the desktop app's data folder by hand, with the per-OS path of that folder (spec 14).
- [ ] **Step 2: Verify the guide.** On macOS, run the CPU build exactly as the guide says. Expected: `target/release/orag --version` prints the current version.
- [ ] **Step 3: Check the release workflow.** Run `actionlint .github/workflows/release.yml`. Expected: no errors.
- [ ] **Step 4: Bump, commit, merge, tag.** Bump to `0.6.0-alpha.18`, commit `docs: build guide; releases carry notes only`, merge, then run `git tag -a v0.6.0-alpha.18 -m "engine with --allow-origin and the interface document"` and `git push origin v0.6.0-alpha.18`.
  - Expected: `release.yml` runs and skips `notes`, because the tag has a `-`.
  - Expected: no release and no assets are created.

---

## Phase C — `orag-ui`

### Task 5: Create `orag-ui` with history and the engine submodule

**Files (new repository `orkono/orag-ui`, local path `/Users/osmanhomek/Development/geonni/orag-ui`):**
- From `orag` history:
  - `apps/desktop/`
  - `crates/orag/src/server/ui/`, renamed to `apps/desktop/ui/page/`
  - `scripts/smoke-desktop.sh`, `stage-sidecar.sh`, `write-checksums.sh`, `test-write-checksums.sh`, `third-party-licenses.sh`, `bump-version.sh`, `version-lib.sh`, `check-version.sh`
  - `.github/workflows/desktop.yml`, `docs/desktop.md`, `docs/signing.md`
  - `LICENSE-MIT`, `LICENSE-APACHE`, `rust-toolchain.toml`, `.gitignore`, `Cargo.lock`
- Create: `Cargo.toml` (workspace), `.cargo/config.toml`, `CHANGELOG.md`, `README.md`, `.docs/orag-ui-decisions.md`, `.gitmodules`
- Create: `apps/desktop/tests/build_settings.rs`
- Modify: `apps/desktop/src/bench/data.rs:12-49` (`include_bytes!` paths to `../../engine/eval/...`) and its test helper `repo()` (`:100`, used at `:132`), which must join `../../engine`
- Modify: `scripts/stage-sidecar.sh`
  - sources from `engine/target/release/orag` and `engine/target/vulkan/release/orag-vulkan`
  - besides `apps/desktop/binaries/`, it also runs `mkdir -p target/release target/debug` (after the split the engine builds in `engine/`, so the root `target/` may not exist yet, and `set -euo pipefail` at `:7` would abort), then copies `orag$ext` into both **before** the `*apple-darwin*` early exit (`stage-sidecar.sh:29`); `orag-vulkan` gets the same copy only in the Windows and Linux part after that exit, because the unbundled app resolves the sidecar beside `orag-desktop` (`sidecar.rs:34-38`); `cargo tauri dev` and the desktop smoke depend on it
- Move: `.github/workflows/desktop.yml` to `.github/workflow-sources/desktop.yml` in the same commit as the import, so Actions does not run it before Task 9 adapts it (it builds `-p orag` and calls root scripts that `orag-ui` lacks, `desktop.yml:62`)

**Interfaces:**
- Produces:
  - The submodule `engine/` at tag `v0.6.0-alpha.18`.
  - The workspace has `members = ["apps/desktop"]`, `exclude = ["engine"]`, and `[workspace.package]` with `version` (as imported), `edition = "2024"`, `rust-version = "1.98"`, `license = "MIT OR Apache-2.0"`, `repository = "https://github.com/orkono/orag-ui"`, plus the same `[profile.release]` as `orag`.
  - `.cargo/config.toml` holds `MACOSX_DEPLOYMENT_TARGET = { value = "14.0", force = true }` and `[target.x86_64-pc-windows-msvc] rustflags = ["-C", "target-feature=+crt-static,+avx2,+fma,+f16c,+bmi2"]`. The desktop compiles no llama.cpp, so it needs no `LLAMA_STATIC_CRT` or CMake entry. The AVX2 floor stays until spec 13 is decided.

- [ ] **Step 1: Import with history, locally.** In a fresh clone of `orag` at `v0.6.0-alpha.18`, run `git filter-repo` with `--path` for each file above and `--path-rename crates/orag/src/server/ui/:apps/desktop/ui/page/`. Push nothing yet.
  - Expected: `git log --follow apps/desktop/ui/page/api.js` shows commits from before the split.
- [ ] **Step 2: Add the submodule, the workspace files and the retargeted paths, before anything compiles.**
  - Run `git submodule add https://github.com/orkono/orag.git engine && git -C engine checkout v0.6.0-alpha.18`.
  - Write the workspace files above. The new `CHANGELOG.md` opens with a line pointing to `orag`'s CHANGELOG for history before the split. `README.md` is a stub: what the app is, and that the engine lives in `orkono/orag`. The decisions file starts with `U-001`: the split, citing `orag` D-025 (Task 11).
  - Park `desktop.yml`.
  - Retarget `bench/data.rs` (the `include_bytes!` constants and the test helper `repo()`) and `stage-sidecar.sh`. `include_bytes!` fails to compile until this is done.
- [ ] **Step 3: Build and stage the engine.** Tauri's build script copies the `externalBin` sidecars (`tauri.conf.json:30`) while compiling, so `apps/desktop/binaries/` must be filled before any `cargo` command at the root. Run `(cd engine && cargo build --release --locked -p orag) && scripts/stage-sidecar.sh`. Expected: `apps/desktop/binaries/orag-aarch64-apple-darwin` and `target/release/orag` exist.
- [ ] **Step 4: Write the failing test `apps/desktop/tests/build_settings.rs`.** Use the `toml` crate, a dependency already.
  - `shared_cargo_settings_match_the_engine`: parse `../../.cargo/config.toml` and `../../engine/.cargo/config.toml`. For every key present in both, in `[env]` (value and its `force` and `relative` flags) and in each `[target.*]` table (`rustflags`), assert they are equal. A `relative = true` path is resolved against the directory that contains each file's `.cargo` folder.
  - `toolchain_matches_the_engine`: `rust-toolchain.toml` `channel` equals `engine/rust-toolchain.toml` `channel`.
- [ ] **Step 5: Prune the lock file.** Run `cargo check -p orag-desktop` once without `--locked`. Expected: the only change to the lock file is removed packages; `git diff --stat Cargo.lock` shows deletions only. If any version changed, restore the file and investigate.
- [ ] **Step 6: Run the tests.** Run `cargo test --locked -p orag-desktop && node --test apps/desktop/ui/page/test/*.test.mjs apps/desktop/tests/*.test.mjs`. Expected: PASS, including `build_settings` and the bench data tests.
- [ ] **Step 7: Commit and push.** Only after Step 6 passes: commit `chore: orag-ui from orag with history; engine pinned at v0.6.0-alpha.18` and push `main` to the empty `orkono/orag-ui`.

### Task 6: The page lives in the app and calls the engine cross-origin

**Files:**
- Move: `apps/desktop/ui/page/test/*` to `apps/desktop/tests/page/`. Keep `package.json` next to the page modules: Node needs it to load them as ES modules.
- Modify: `apps/desktop/ui/page/index.html` (relative asset paths: `app.css`, `app.js`, `favicon.svg`)
- Modify: `apps/desktop/ui/page/api.js:28-45` (an API base)
- Modify: `apps/desktop/ui/page/status.js:78-86` (`reachable()` calls `fetch("/v1/health")` directly; it goes through `apiUrl`)
- Modify: `apps/desktop/ui/page/bench.js:12-35` (gate on the app version; the bench link keeps the query)
- Modify: `apps/desktop/ui/page/index.html:224` (`#bench-link` gets its `href` from `bench.js`, not the static `?bench=1`)
- Modify: `apps/desktop/build.rs` (copy `engine/assets/orag-icon.svg` to `ui/page/favicon.svg`, which goes in `.gitignore`)
- Modify: `apps/desktop/tauri.conf.json` (CSP)
- Modify: `apps/desktop/src/window.rs:93-102` (the marker script also defines `__ORAG_APP_VERSION__`)

**Interfaces:**
- Consumes: Task 2's CORS and the origins listed under Global Constraints.
- Produces:
  - The page URL is `<local origin>page/index.html?api=<port>#lang=<tr|en>[&at=<ms>]`. The query carries the API port and the fragment stays the language's alone. So `with_language` (`window.rs:234`) and `writeFragment` (`i18n.js:806`), which both rewrite the whole fragment, need no change, and `writeFragment` already keeps `location.search`.
  - `api.js` exports `apiBase(search: string): string | null`, which reads `api` from the query and returns `http://127.0.0.1:<port>` only for a decimal port from 1 to 65535, else null. It also exports `apiUrl(path: string): string`, which reads `globalThis.location?.search ?? ""` and returns the base plus `path`, or `path` unchanged when `apiBase` is null. The moved tests then keep working unchanged: some set `location.search = ""` (`fake-dom.mjs:307`), some install no `location`, and `fakeFetch` matches the exact `METHOD /v1/...` (`fake-dom.mjs:340`). `request(path, options)` and `status.js`'s `reachable()` use `apiUrl`.
  - `bench.js` exports `benchHref(search: string): string`, the current query plus `bench=1`, and sets `#bench-link`'s `href` to it.
  - `__ORAG_APP_VERSION__` is a read-only global set by the marker script from `env!("CARGO_PKG_VERSION")`.
  - `benchLinkShown(info, inDesktop, appVersion)` gates on `appVersion`, not `info.version`.
  - CSP: `default-src 'self'; style-src 'self' 'unsafe-inline'; connect-src 'self' http://127.0.0.1:*`.

- [ ] **Step 1: Write the failing page tests in `apps/desktop/tests/page/`.**
  - `api-base.test.mjs`:
    - `apiBase("?api=42017") === "http://127.0.0.1:42017"`; `apiBase("?api=0")`, `apiBase("?api=70000")`, `apiBase("?api=1e3")`, `apiBase("?api=42017@evil")` and `apiBase("")` are all `null`.
    - `a_language_change_keeps_the_api_port`: with `location` at `/page/index.html?api=42017#lang=tr`, choosing English leaves `location.search === "?api=42017"` and the fragment `lang=en&at=…`.
    - `api_url_without_a_base_is_the_path`: with no `location`, and with `location.search === ""`, `apiUrl("/v1/health") === "/v1/health"`.
    - `the_health_check_uses_the_api_base`: with a fake `fetch`, `reachable()` calls `http://127.0.0.1:42017/v1/health`.
  - `bench.test.mjs`, the link: `benchHref("?api=42017") === "?api=42017&bench=1"`, and `benchRequested("?api=42017&bench=1", true) === true`.
  - `bench.test.mjs`: these replace the existing version cases.
    - `benchLinkShown({version:"0.6.0", gpu:{backend:"vulkan"}}, true, "0.6.1-alpha.1") === true` (pre-release app, stable engine)
    - `benchLinkShown({version:"0.6.0-alpha.18", gpu:{backend:"vulkan"}}, true, "0.6.0") === false` (stable app, alpha engine)
    - Metal and outside the desktop still give `false`.
- [ ] **Step 2: Confirm they fail.** Run `node --test apps/desktop/tests/page/*.test.mjs apps/desktop/tests/*.test.mjs`. Expected: FAIL; `apiBase` is not exported, and the bench cases fail.
- [ ] **Step 3: Implement the page changes.** Make the asset paths in `index.html` relative, and fix every test import path after the move. Then run `rg -n "fetch\(" apps/desktop/ui/page apps/desktop/src`. Expected: only `api.js`, the `apiUrl` call in `status.js`, and `SMOKE_SCRIPT` (absolute URL after Task 7).
- [ ] **Step 4: Confirm the tests pass.** Run the node command above. Expected: PASS.
- [ ] **Step 5: Implement the app side.** Write the favicon copy in `build.rs`, the CSP, and the marker script's `__ORAG_APP_VERSION__`. Run `cargo test --locked -p orag-desktop`. Expected: PASS.
- [ ] **Step 6: Bump and commit.** Bump with `orag-ui`'s `scripts/bump-version.sh`, then commit `feat: the page ships in the app and calls the engine API`.

### Task 7: Supervisor and window use the API line, the app origin and page paths

**Files:**
- Modify: `apps/desktop/src/supervisor.rs`
  - `:40-70`: `SidecarSpec::serve` adds `--allow-origin <local origin without trailing slash>`. Remove `with_ui_port_hint`.
  - `:78`: `Ready { api: String }`
  - `:409-460` (`wait_ready`): return on the first `orag listening on` line. Ignore any later line, including `orag ui on`. The timeout message no longer mentions the web page.
- Modify: `apps/desktop/src/window.rs`
  - `:125-148` (`action_for(event: &SidecarEvent, local: &Url) -> Action`): `Ready { api }` → `Action::Open(page_url(local, port))`. A non-loopback or port-less `api` is refused, as `ui` is today. `handle_event` (`app.rs:318-325`) still applies `with_language` to that URL, unchanged.
  - `:257-265` (`navigation_allowed`): by path.
  - `SMOKE_SCRIPT` (`:63-80`): its `fetch("/v1/models/import")` (`:74`) becomes a cross-origin POST to `http://127.0.0.1:<port>`, with the port read from `api` in its own `location.search` and checked like `apiBase`. Otherwise the POST goes to the app origin, never gets `400 invalid_input`, and `smoke-desktop.sh:93` fails.
- Modify: `apps/desktop/src/app.rs`
  - `Pages`: drop `remember_ui_port`.
  - `page_loaded` (`:190-206`): use `is_status_page` and `is_service_page`.
- Modify: `apps/desktop/src/prefs.rs` (remove `UI_PORT_FILE`, `read_ui_port`, `write_ui_port`)
- Modify: `apps/desktop/src/home.rs:7-12` (`DEFAULT_CONFIG` keeps only `bind = "127.0.0.1:0"` and reworded comments)
- Test: the unit tests in `window.rs` and `supervisor.rs`; `apps/desktop/tests/supervisor.rs`; `apps/desktop/src/fake.rs` (the fake sidecar prints only the listening line)

**Interfaces:**
- Consumes:
  - `--allow-origin` (Task 2)
  - the page URL format (Task 6)
- Produces:
  - In `window.rs`: `pub fn page_url(local: &Url, api_port: u16) -> Url` (`<local>page/index.html?api=<port>`), `pub fn is_status_page(url: &Url, local: &Url) -> bool` (path `/` or `/index.html`), `pub fn is_service_page(url: &Url, local: &Url) -> bool` (path `/page/index.html`). Both page checks also require the same origin as `local`.
  - In `supervisor.rs`: `SidecarEvent::Ready { api: String }`.

- [ ] **Step 1: Write the failing tests.**
  - `ready_opens_the_page_with_the_api_port`: `Ready { api: "http://127.0.0.1:42000" }` → `Action::Open` whose URL is `<local>page/index.html?api=42000`; `with_language` on it gives `…?api=42000#lang=…`.
  - `a_non_loopback_api_address_is_refused`: the same as today's `a_non_loopback_page_address_is_refused`, but for `api`.
  - `pages_are_told_apart_by_path`: `<local>index.html` is the status page; `<local>page/index.html` is the service page; `<local>page/other.html` and `http://127.0.0.1:1/page/index.html` are neither.
  - `the_bench_channel_rules_hold_by_path`: `run` is accepted only while on the service page and `resume` only on the status page. Keep the existing `channel.rs` tests, and feed them the path-based state.
  - `a_restart_hands_the_page_the_new_api_port`: two `Ready` events with ports 42000 and 42001 give two `Open` URLs, the second with `?api=42001`.
  - `the_service_is_started_with_the_app_origin`: `SidecarSpec::serve(..).args == ["serve", "--exit-on-stdin-close", "--allow-origin", "tauri://localhost"]` on macOS and Linux, and `http://tauri.localhost` on Windows. Use `cfg` in the test.
  - `wait_ready_needs_only_the_listening_line`, in `tests/supervisor.rs`: a fake child that prints only `orag listening on http://127.0.0.1:1` gives `Ready` within 2 s.
- [ ] **Step 2: Confirm they fail.** Run `cargo test --locked -p orag-desktop`. Expected: FAIL; `page_url` is undefined, and the `Ready` shape differs.
- [ ] **Step 3: Implement.** Make the changes listed under Files. Then delete the tests for the removed port hint and for the UI port file.
- [ ] **Step 4: Confirm the tests pass.** Run `cargo test --locked -p orag-desktop && node --test apps/desktop/tests/page/*.test.mjs apps/desktop/tests/*.test.mjs`. Expected: PASS.
- [ ] **Step 5: Run the app locally on macOS.** Run `scripts/stage-sidecar.sh` after a release engine build in `engine/`, then `cargo tauri dev --no-dev-server` from `apps/desktop`. By default the Tauri CLI serves the frontend from an HTTP dev server, which would change the page origin away from `local_origin()` and the named CORS origin. Expected: the setup page and the main page load, and asking a question works with `--dev-fake-models` off and real models present. Do not report success until this is done.
- [ ] **Step 6: Bump and commit.** Commit `feat: the app starts the engine with its origin and opens its own page`.

### Task 8: Check the app's engine calls against the interface document

**Files:**
- Create: `apps/desktop/tests/engine_interface.rs`

**Interfaces:**
- Consumes: Task 3's `engine/docs/engine-interface.md` flag table and environment table.

- [ ] **Step 1: Write the test.**
  - `every_engine_flag_the_app_passes_is_documented`: scan `apps/desktop/src/**/*.rs` for string literals matching `"--[a-z][a-z0-9-]*"`. Exclude `src/fake.rs` and `src/fake_eval.rs`, which are test doubles. Assert that each literal appears in the document's flag table.
  - `every_engine_variable_the_app_sets_is_documented`: string literals matching `"ORAG_[A-Z_]+"` in the same files must appear in the environment table. The app's own `ORAG_DESKTOP_*` variables are excluded.
- [ ] **Step 2: Run it.** Run `cargo test --locked -p orag-desktop --test engine_interface`. Expected: PASS. If it fails, the fix goes in the engine's document, through an engine change and a new tag, never an exclusion here.
- [ ] **Step 3: Sanity-check the test.** Temporarily add `"--not-a-flag"` to `bench/run.rs`. Expected: FAIL. Then revert.
- [ ] **Step 4: Bump and commit.** Commit `test: the app uses only documented engine flags and variables`.

### Task 9: `orag-ui` CI and release

**Files:**
- Create: `.github/workflows/ci.yml`. It takes the app and page steps of `orag`'s `ci.yml` and all of `.github/workflow-sources/desktop.yml` (parked in Task 5), which is then deleted.
- Create: `.github/workflows/release.yml`. It takes the desktop and checksum parts of `orag`'s `release.yml`.
- Create: `scripts/check-engine-pin.sh`
- Modify: `scripts/third-party-licenses.sh` (runs `cargo tree` in `engine/` for `-p orag` and at the root for `-p orag-desktop`, and merges the two)
- Modify: `scripts/smoke-desktop.sh`, `scripts/stage-sidecar.sh` (the `engine/` paths)

**Interfaces:**
- Produces: `scripts/check-engine-pin.sh` exits 0 when `git -C engine rev-parse HEAD` equals the peeled commit of some `refs/tags/v*` in `git ls-remote --tags https://github.com/orkono/orag`. Otherwise it exits 1 and prints the commit.

- [ ] **Step 1: Write the pin check.** Run it in place: `scripts/check-engine-pin.sh`. Expected: exit 0. Then `git -C engine checkout HEAD~1 && scripts/check-engine-pin.sh`. Expected: exit 1. Then restore the tag checkout.
- [ ] **Step 2: Write `ci.yml`.**
  - Checkout uses `submodules: true` and runs `scripts/check-engine-pin.sh` first.
  - Per OS (`ubuntu-22.04`, `macos-15`, `windows-2022`): build the engine with `cargo build --release --locked` in `engine/`, then run `engine/scripts/check-llama-build.sh --release` and `engine/scripts/check-binary-deps.sh` from inside `engine/`.
  - Vulkan builds with `engine/scripts/build-vulkan.sh --release` on Linux and Windows (its default is debug, and staging copies the release binary), with the same SDK steps as `orag`.
  - Then `scripts/stage-sidecar.sh`, before any `cargo` command at the root: Tauri's build script needs the `externalBin` files (Task 5 Step 3).
  - Then `cargo fmt --check`, `clippy` and `cargo test --locked` at the root; `node --test apps/desktop/tests/page/*.test.mjs apps/desktop/tests/*.test.mjs`; `scripts/test-write-checksums.sh`; `scripts/check-version.sh`.
  - Then the desktop bundle and smoke tests, exactly as the parked `desktop.yml` runs them, including the `missing-loader` Windows job.
- [ ] **Step 3: Write `release.yml`.** On `v*` tags: the same build, bundles for every OS, `SHA256SUMS` from `write-checksums.sh`, build provenance, and the third-party licenses. It creates a **draft** release, marked pre-release when the tag contains `-`.
- [ ] **Step 4: Verify on CI.** Push to a branch and open a PR. Expected: every CI job passes on all three OSes, including the desktop smoke and the Windows installer smoke without a Vulkan driver.
- [ ] **Step 5: Verify a release.** Merge, bump, and tag the `orag-ui` version as `v<version>`. Expected: a draft pre-release with the Windows setup, macOS `.dmg`, Linux `.AppImage` and `.deb`, `SHA256SUMS`, and the provenance. Leave it as a draft.

---

## Phase D — Engine becomes engine-only (`orag`)

### Task 10: Remove the product from `orag`

First in Phase D, so `orag`'s own `apps/desktop` (whose `wait_ready` needs both stdout lines, `supervisor.rs:409`) and its CI are gone before the engine drops the second line in Task 11.

**Files:**
- Delete:
  - `apps/desktop/`, and the `"apps/desktop"` entry in the root `Cargo.toml` members (keep `default-members` or drop it if it equals `members`)
  - the scripts `smoke-desktop.sh`, `stage-sidecar.sh`, `write-checksums.sh`, `test-write-checksums.sh`, `third-party-licenses.sh`, `build-windows-installer.sh`, `smoke-windows-installer.sh`, `render-icon.sh`
  - `packaging/windows/orag.iss`, `packaging/windows/README-Windows.txt`
  - `assets/orag-icon.ico`, `assets/orag-icon-256.png`, `assets/orag-icon-512.png`
  - `.github/workflows/desktop.yml`
  - `docs/desktop.md`, `docs/signing.md`
- Keep: `assets/orag-icon.svg`, `packaging/windows/static-crt.cmake`, and every other `scripts/` file. By the spec's rule, each script left over builds or checks the engine. Check that `fetch-model-pack.*`, `release-model-check.sh` and `check-pack-presets.sh` do; move any that only serve the app to `orag-ui` in its own commit there.
- Modify: `.github/workflows/ci.yml`
  - remove the `test-write-checksums` step (`:39-40`) and the installer step (`:66-74`)
  - the node step (`:47-48`) runs only the page's tests, `node --test crates/orag/src/server/ui/test/*.test.mjs`, until Task 11 removes it; `check-pack-presets.sh` stays
  - the Vulkan job is unchanged
- Modify: `Cargo.lock` (prune it as in Task 5 Step 4, deletions only)
- Modify: `README.md`: remove the install and desktop sections; add "Download the app: github.com/orkono/orag-ui/releases", and say that older releases (0.3.0–0.6.0-alpha.12) stay on this repository's releases page.

- [ ] **Step 1: Make the deletions and workflow edits.**
- [ ] **Step 2: Run the checks.** Run `cargo build --locked && cargo test --locked && scripts/check-version.sh`. Expected: PASS. Then `git diff --stat Cargo.lock`. Expected: deletions only.
- [ ] **Step 3: Check the workflows.** Run `actionlint .github/workflows/*.yml`, or `gh workflow view` if actionlint is missing. Expected: no errors.
- [ ] **Step 4: Bump, commit, review, merge.** Bump to `0.6.0-alpha.19` and commit `chore!: the desktop app and installers live in orkono/orag-ui`. Expected: `ci.yml` passes on all three OSes, and the Vulkan job passes too.

### Task 11: Remove the page from the engine; the engine-only tag

**Files:**
- Delete:
  - `crates/orag/src/server/ui.rs`
  - `crates/orag/src/server/ui/`
  - `crates/orag/tests/api/ui.rs`, with its line in `tests/api/main.rs`
- Modify: `crates/orag/src/server/mod.rs`
  - remove `ui_router` (`:252-256`) and the `ui` module
  - rename `serve_with_ui` (`:339`) to `serve` and give it one listener
- Modify: `crates/orag/src/server/security.rs`
  - remove the `is_asset` exception (`:14`, `:39-40`)
  - reword the refusal log (`:100`) to "request refused (Host or Origin not allowed; see docs/api.md)"
- Modify: `crates/orag/src/app.rs`
  - remove `UI_PREFIX`, the UI listener (`:66-71`, `:131-138`) and its stdout line (`:154-156`), and `bind_ui_listener` (`:191-215`) with its test (`:520`)
  - set `effective["ui_bind"] = "off"` always
- Modify: `crates/orag/src/config.rs`
  - `:20-40`, `:68`, `:98-100`, `:164-177`, `:221-230` and the tests at `:553-603`
  - `ui_bind` is still parsed as a string, but its value is not used: any value is accepted
  - the address field leaves `Config`; a `pub ignored_ui_bind: bool` takes its place, and `effective()` reports `"off"`
  - the config is loaded before logging starts (`cli.rs:216` before `app.rs:45`), so `Config` does not log. `run_server` logs once at warn level, after logging is set up, when `ignored_ui_bind` is true: "ui_bind in config.toml is ignored: orag no longer serves a web page"
- Modify: `crates/orag/src/cli.rs:37-40`: `--ui-port-hint` stays hidden, documented as "Deprecated: accepted and ignored". `Start`'s help becomes "Start ORAG; restarts it after a model import".
- Modify: `crates/orag/src/launcher.rs`
  - remove `open_in_browser` and the UI-line handling (`:1`, `:34-52`, `:68-120`, and the tests' `orag ui on` lines at `:226-249`)
  - an already-running instance prints "orag is already running; its API is at http://<bind>"
- Modify: "the page" wording in log lines: `app.rs:93`, `server/mod.rs:185`, `server/models.rs:1,51,111`, `server/choice.rs:105`, `server/download.rs:243`. Each becomes "a client" or "the API" as fits, and `models import` stays named.
- Modify: `docs/api.md` (`:17-19`, `:79`, `:87`, `:92-93`, `:172`), `docs/engine-interface.md` (the second stdout line is removed, marked **interface**), `README.md` (the `ui_bind` row, page sections `:100`, `:195-225`; the README now describes the engine only: what it is, the build guide, `docs/api.md`, `docs/engine-interface.md`)
- Modify: `.github/workflows/ci.yml`: remove the page's node test step
- Create: decision `D-025 — Engine and UI split` in `.docs/orag-decisions.md`. It records owner decisions 1–5 and calls 6–18. It supersedes D-016's "desktop app in this repository" and the stdout second line, and D-021 in full; D-023 keeps `orag start` without a page.
- Modify: `.docs/plans/2026-10-21-orag-v0.6-gpu-plan.md`, step 89: release 0.6.0 as engine tag `v0.6.0` (notes only) plus the `orag-ui` app release `0.6.0`.

- [ ] **Step 1: Write the failing tests.**
  - In `config.rs`:
    - `a_ui_bind_in_config_is_accepted_and_ignored`: `ui_bind = "127.0.0.1:0"` loads, `ignored_ui_bind` is true, and `effective()["ui_bind"] == "off"`. Without the key, `ignored_ui_bind` is false.
    - `a_desktop_config_from_0_5_still_loads`: the exact text `bind = "127.0.0.1:0"\nui_bind = "127.0.0.1:0"\n` loads.
  - In `tests/cli.rs`:
    - `serve_prints_only_the_listening_line`: run `orag serve --dev-fake-models --exit-on-stdin-close` with a temporary `ORAG_HOME` whose `config.toml` sets `ui_bind = "127.0.0.1:0"`; read stdout until stdin is closed. Expected: exactly one line, starting `orag listening on http://127.0.0.1:`, and stderr contains `ui_bind in config.toml is ignored` once.
    - `serve_accepts_the_ignored_ui_port_hint`: `--ui-port-hint 42017` is accepted.
  - Retire the UI-listener tests in `tests/cli.rs`: `start_server_with_ui` (`:178`) and its callers (`:205`, `:226`, `:257`), `ui_off_prints_only_the_api_line` (`:240`), and `a_busy_ui_port_is_reported_before_models_load` (`:274`). That last one calls `.output()` with no timeout; once `ui_bind` is ignored the engine serves forever and CI hangs. Move what they also check onto the API listener: `/v1/version` reports `"ui_bind":"off"`, a busy **API** port is reported before models load (with a timeout), and the shutdown assertion of the `:257` test.
  - In `launcher.rs`, `start_restarts_on_exit_restart_and_opens_nothing`: the fake child script prints only the listening line, exits 75 once, then 0. Expected: two spawns, and the opener closure is never called. The opener parameter is removed after this test passes; until then the test asserts it is not called.
  - In `tests/api`, `static_asset_paths_are_not_served`: `GET /app.js` and `GET /` → 404.
- [ ] **Step 2: Confirm they fail.** Run `cargo test --locked`. Expected: the new tests FAIL.
- [ ] **Step 3: Implement the removal.** Make the changes listed under Files.
- [ ] **Step 4: Confirm the tests pass.** Run `cargo fmt --all --check && cargo clippy --locked --all-targets -- -D warnings && cargo test --locked && cargo test --locked --test interface`. Expected: PASS.
- [ ] **Step 5: Check for leftovers.** Run `rg -n "ui_bind|orag ui on|the page|web page|favicon" crates docs README.md`. Expected: only the deprecation notes, the ignored-key warning and the `/v1/version` note.
- [ ] **Step 6: Bump, commit, review, merge, tag.** Bump to `0.6.0-alpha.20`. The CHANGELOG bullet starts "**Interface:** serve no longer prints `orag ui on`; the page has moved to orkono/orag-ui". Commit `feat!: the engine serves no web page`, get the review, merge. Then run `git tag -a v0.6.0-alpha.20 -m "engine only"` and push it. Expected: `ci.yml` passes on all three OSes, including the Vulkan job.

### Task 12: `orag-ui` pins the engine-only tag

**Files (`orag-ui`):**
- Modify: `engine` (submodule to `v0.6.0-alpha.20`)
- Modify: `scripts/smoke-desktop.sh`: add a run that starts the app over a data folder whose `orag/config.toml` is the 0.5 text from Task 11 Step 1

- [ ] **Step 1: Bump the pin.** Run `git -C engine fetch --tags && git -C engine checkout v0.6.0-alpha.20`, then `scripts/check-engine-pin.sh`. Expected: exit 0.
- [ ] **Step 1b: Rebuild and restage the engine.** The root workspace excludes `engine/`, and `stage-sidecar.sh` only copies binaries that already exist, so without this step the checks below would run against alpha.18. Run `(cd engine && cargo build --release --locked -p orag) && scripts/stage-sidecar.sh`. On Linux and Windows, also run `engine/scripts/build-vulkan.sh --release` before staging; the script builds debug by default (`build-vulkan.sh:10`), and staging copies the release binary (`stage-sidecar.sh:31`). Expected: `target/release/orag --version` prints `0.6.0-alpha.20`, and so does `target/release/orag-vulkan --version` on Linux and Windows.
- [ ] **Step 2: Run the local tests.** Run `cargo test --locked && node --test apps/desktop/tests/page/*.test.mjs apps/desktop/tests/*.test.mjs`. Expected: PASS.
- [ ] **Step 3: Add the old-config smoke run.** Expected: the app reaches its page, and the engine log shows the single `ui_bind ... ignored` warning.
- [ ] **Step 4: Verify on CI and release.** Push a branch, open a PR, and wait for CI. Expected: every job passes on all three OSes. Merge, bump, tag. Expected: a draft pre-release, as in Task 9 Step 5. Its notes say that the Inno installer and the CLI archives are discontinued, and how to move `~/.orag` into the app's data folder by hand (spec 14), linking `engine/docs/building.md`.
- [ ] **Step 5: Report to the owner.** The split is done. Releasing 0.6.0 is still step 89 of the GPU plan, and still waits for the GPU bench result.
