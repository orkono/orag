# Engine and UI split: `orag` becomes the engine, `orag-ui` the product

Status: approved (2026-11-27; muse and gpt-6-astra).

## Intent

`orag` grew from an engine into an engine plus a product: a web page compiled
into `orag`, a Tauri desktop app, installers and release packaging, all in one
repository. The owner wants `orag` back to being only an engine that other
programs run as a separate process, and the user-facing product moved to a new
public repository, `github.com/orkono/orag-ui`.

The engine is meant to be embedded in other programs **as an engine**, not as a
library: a program ships the `orag` executable, starts it, and talks to it over
the HTTP API and the command line, exactly as the desktop app does today
(D-016).

## Goals

- `orag` contains the engine only: ingestion, retrieval, generation, the model
  store and downloads, GPU support, the CLI, the HTTP API, and the evaluation
  tools that measure the engine.
- `orag` serves no web page. `orag start` stays, without a page.
- `orag` publishes **no built artifacts**: no installers, no `.exe`, `.dmg`,
  `.AppImage`, `.deb`, `.zip`, no checksums file. A release is a git tag with
  release notes. The repository documents how to build the engine on each
  operating system.
- `orag-ui` contains everything a person sees or installs: the web page, the
  desktop app, the installers, the release packaging, the GPU-bench screen.
- `orag-ui` builds the engine from source and puts it in its installers.
- The interface between the two is written down and versioned, so other
  programs can rely on it too.

## Non-goals

- Turning the engine into a Rust library or crate other programs link against.
- Changing what the product does for end users: the desktop app keeps every
  feature it has in 0.6 (setup, ask, documents, collections, GPU status and
  retry, the GPU bench).
- Redesigning the HTTP API. Only the changes the split needs.
- Code signing, auto-update, or new platforms.

## Anti-goals (a failure even if it works)

- The engine still knows about the product: it serves or renders UI, or keeps
  a route, flag or wording that only one particular app could use. A
  supervisor protocol any embedding program can use (the GPU fallback and
  retry variables, exit codes, the first stdout line) is engine interface, not
  product.
- `orag-ui` carries a copied or patched engine instead of a pinned upstream one.
- Existing download links in `orag`'s release history stop working.
- An existing user's `config.toml` stops the new engine from starting.
- A desktop user loses a feature or their data folder when updating to the
  first `orag-ui` release.

## Constraints

- Both repositories are public. License stays `MIT OR Apache-2.0` in both.
- D-012 (offline, no implicit network), D-013 (loopback only, no
  authentication, Host and Origin checks) and D-019 (`config.toml` read once)
  still hold. `ORAG_HOME` is the only variable meant for people; the engine
  already reads supervisor variables set by the program that starts it
  (`ORAG_SUPERVISED`, `ORAG_GPU_FALLBACK_REASON`, `ORAG_GPU_RETRY`,
  `ORAG_GPU_DEVICE`, `ORAG_VULKAN_ALLOW_SOFTWARE`). Those become part of the
  written engine interface, not hidden desktop details.
- The engine's Windows build settings (AVX2 floor, static C runtime, in
  `.cargo/config.toml` and `packaging/windows/static-crt.cmake`) must apply when
  `orag-ui` builds the engine, so `orag-ui` builds it inside the engine's own
  checkout.
- Owner rules: every merged change bumps the version and adds a CHANGELOG entry
  (D-017), in each repository; decisions are recorded in each repository's
  decisions file.
- v0.6 is waiting for the GPU benchmark on the owner's PC. `0.6.0-alpha.12` is
  published for that run and must stay as it is. `main` is at
  `0.6.0-alpha.13` (docs only since alpha.12); that is the base of the split.

## Approaches (how `orag-ui` gets the engine)

- **A. Git submodule — chosen by the owner.** `orag-ui/engine/` is a submodule
  of `orag`, pinned to an engine tag. The pin is in `orag-ui`'s history and an
  upgrade is one commit. CI builds the engine inside `engine/` with the
  engine's own build settings. The GPU-bench screen embeds the question set,
  corpus and reference at compile time from `engine/eval/` (`include_bytes!`
  paths retargeted to the submodule) instead of a copy; the installed app
  reads nothing from `engine/` at run time. Cost: contributors must use
  `git submodule update`.
- B. A version file plus a fetch script that clones the tag into an ignored
  folder. Same result, less visible pin, one more script on every path.
- Rejected: `orag` CI publishing bare engine executables for `orag-ui` to
  download. It contradicts "orag publishes no built artifacts".

## Decided

Owner decisions (2026-11-26):

1. `orag` is the engine only; the web page leaves it; `orag start` stays,
   without a page.
2. The engine is not a library; other programs run it as a process.
3. `orag` publishes no installers or binaries; source and a build guide only.
4. The product moves to `github.com/orkono/orag-ui` (public, empty today).
5. `orag-ui` pins the engine as a git submodule (approach A).

Calls made in the design (the owner can overrule):

6. Moved files keep their git history (`git filter-repo` into `orag-ui`).
7. `orag`'s existing releases (0.3.0–0.5.0, 0.6.0-alpha.12) stay untouched,
   assets included. The README points downloads to `orag-ui`.
8. `orag` CI keeps building and testing on Linux, macOS and Windows, and the
   `orag-vulkan` jobs, but uploads nothing.
9. Model management stays in the engine (status, import, download, setup
   choice API), because every program that embeds the engine needs models.
   Only the page that used it leaves.
10. `orag eval bench` stays in the engine (it measures the engine), and so do
    its inputs: the question set, the corpus and the CPU reference
    `eval/bench/v06-cpu-reference.json`, which `bench-reference.yml` produces
    from an engine commit. Running the bench from one button, merging the runs
    and writing the results file move to `orag-ui`, which reads those inputs
    from `engine/eval/`. A new reference reaches the app through an engine pin
    bump, never a hand copy.
11. Timing: split now. 0.6.0 is released from the new layout (engine tag
    `v0.6.0`, app release `0.6.0` from `orag-ui`). The GPU benchmark from
    `0.6.0-alpha.12` is still valid: the split does not change the measured
    code.
12. Versions: both start from `0.6.0-alpha.13`, the current `main`, and are bumped
    independently from then on. The app keeps the product name **ORAG**, its
    Tauri identifier `com.orkono.orag` and its data folder, so an installed 0.5
    or alpha updates in place.
13. The "unsupported processor" message proposed to the owner on 2026-11-23
    (a clear message instead of a silent crash on CPUs without AVX2, found on
    the UTM Windows VM) is not decided yet; if approved it belongs to
    `orag-ui`'s desktop app, not this split.
14. Products after the split: `orag-ui` ships the desktop app only (Windows
    setup, macOS `.dmg`, Linux packages, as today). The Inno Setup installer
    (`ORAG-Setup-<v>.exe`, `orag.exe` plus `orag start` in a browser) and the
    CLI archives (`orag-<v>-<target>.zip`/`.tar.gz`) are discontinued: the
    page they opened no longer exists, and command-line users build the
    engine from source (owner decision 3). This supersedes the CLI-archive
    line of the v0.6 plan's Task 7. Their data in `~/.orag`
    (`%USERPROFILE%\.orag`) is untouched and still used by a self-built
    `orag start`; the release notes and the build guide say how to move it
    into the desktop app's data folder by hand. No automatic migration.
15. The `/v1` API is not broken. Fields and routes the page uses stay and are
    documented as generic: `label_key` (a stable message key a client may
    translate), `selected` (the configured generator when it is eligible, else the
    recommended option, else the first eligible one; distinct from
    `recommended`), `POST
    /v1/system/gpu-retry` and the GPU variables (the supervisor protocol).
    `serve --ui-port-hint` stops doing anything and is accepted and ignored
    for the 0.6 line, then removed. Log lines that mention "the page" are
    reworded.
16. Engine tags: every engine version `orag-ui` pins is a tag
    (`v0.6.0-alpha.N` or `v0.6.0`), pushed by the agent as today.
    `orag`'s `release.yml` keeps running on `v*` tags but only creates a
    GitHub Release with the CHANGELOG notes and no assets, and only for
    tags without `-`; alpha tags stay plain tags. `orag-ui` CI fails if
    `engine/` is not exactly at a `v*` tag.
17. One-time reset: the page's selected collection and "help already seen"
    flag live in the browser storage of the old page address, so they reset
    once when the page moves into the app. Accepted; the language choice is
    kept because the app also stores it.
18. The GPU-bench link and `?bench=1` dialog are gated on the **app's**
    version (passed into the page by the app), not the engine's: with
    independent versions a pre-release app pinned to a stable engine would
    otherwise lose the bench, and a stable app with an alpha engine would
    show it. Tests cover both mismatched cases.

## What moves and what stays

Moves to `orag-ui`:

- `crates/orag/src/server/ui/` (page, its tests, `package.json`). The page's
  favicon comes from the submodule (`engine/assets/orag-icon.svg`); the SVG
  itself stays in the engine, whose README shows it.
- `apps/desktop/` in full (app, status page, tests, icons, Tauri configs).
- Release packaging for the desktop app: the scripts that stage, smoke-test,
  checksum or license-bundle a distributable (`smoke-desktop.sh`,
  `stage-sidecar.sh`, `write-checksums.sh`, `test-write-checksums.sh`,
  `third-party-licenses.sh`). `third-party-licenses.sh` runs `cargo tree -p
  orag` today; in `orag-ui` it must cover the engine (run inside `engine/`)
  and the app's own dependency tree.
  Rule for the scripts not named here: a script that builds or checks the
  engine stays; one that produces or checks something people download moves.
- `.github/workflows/desktop.yml`, the installer part of `release.yml`, the
  installer and page-test steps of `ci.yml`.
- `docs/desktop.md`, `docs/signing.md`, and the README's install and page
  sections.

Deleted (discontinued by decision 14; their history stays in `orag`):
`packaging/windows/orag.iss`, `packaging/windows/README-Windows.txt`,
`build-windows-installer.sh`, `smoke-windows-installer.sh`, the CLI-archive
packaging in `release.yml`, and the icon copies only the Inno installer used
(`assets/orag-icon.ico`, the PNG copies, `scripts/render-icon.sh`). The SVG
stays.

Stays in `orag`:

- `crates/orag` without the page: CLI, HTTP API, setup mode, model store and
  downloads, GPU code, `eval` (including `eval bench`).
- `eval/` (datasets, corpora, the CPU reference results the bench compares
  against are produced here by `bench-reference.yml`).
- Engine workflows: `ci.yml` (engine jobs), `bench-reference.yml`,
  `gpu-retrieval.yml`, `model-cpu.yml`, `tier-cpu.yml`, `vector-scale.yml`.
- Build settings and engine checks (`.cargo/config.toml`,
  `packaging/windows/static-crt.cmake`, `build-vulkan.sh`,
  `check-*.sh`, `platform-floors.sh`, version scripts).
- `docs/api.md`, `.docs/` decisions and plans, CHANGELOG.

## Engine changes the split needs

- Remove the page listener and its routes (`server/ui.rs`, the `ui_bind`
  listener in `app.rs`, D-021). A `config.toml` that still sets `ui_bind` must
  keep working: the key is accepted and ignored with one warning line.
- `serve` stops printing its second stdout line (`orag ui on …`); only the
  first, listening line remains. The desktop supervisor's `wait_ready`, which
  today returns `Ready { api, ui }` only after both lines and otherwise
  times out after 180 s ("started without its web page"), becomes API-only.
  This is a change to the engine's process interface and is named as such in
  the interface document and the CHANGELOG.
- `orag start` keeps starting `serve` and restarting it on `EXIT_RESTART`
  (D-023), prints where the API listens, and opens nothing.
- Allowed origins and CORS: today the only allowed origins are the page's own
  and the engine sends no CORS headers, because the page always called the
  API from its own origin. The desktop app's page will now be served by the
  app (`tauri://localhost` on macOS and Linux, `http://tauri.localhost` on
  Windows) and call the API cross-origin, with preflighted JSON and multipart
  requests and the SSE stream. So:
  - the program that starts the engine names the origins it allows
    (explicit origins only, no wildcard; Host checks unchanged, D-013);
  - the engine answers `OPTIONS` preflights and sends
    `Access-Control-Allow-Origin` (and the needed allow-headers/methods) for
    those origins only, and for nothing else;
  - the app's CSP gains a `connect-src` entry for the engine's loopback
    address.
  The origins are named at spawn time with a repeatable `serve
  --allow-origin <origin>` option, never in `config.toml`: the desktop writes
  its `config.toml` once (`create_new`) and never updates it, so a new key
  would never reach existing installs, and D-019 rejects unknown keys.
  **Gate:** before any CORS code, a recorded spike on WKWebView (macOS),
  WebKitGTK (Linux) and WebView2 (Windows) shows the exact `Origin` header and
  preflight behaviour of a page loaded from the app calling the loopback API
  (JSON, multipart, SSE). If any webview sends no usable origin (`null` or
  opaque), the exact-origin plan is dropped for the fallback: the app serves
  its bundled page from its own loopback HTTP listener, so the page has an
  ordinary `http://127.0.0.1:<port>` origin the app names with
  `--allow-origin`. Neither path weakens D-013.
- `GET /v1/version` keeps reporting `config.ui_bind`, always as `"off"`
  (today's value when the page is disabled), through the 0.6 line, documented
  as deprecated; it is removed only with a contract version bump.
- A written engine interface for programs that embed the engine, in `docs/`,
  built from the actual call sites in `apps/desktop/` (not from memory) and
  checked by a test that fails when the app uses an undocumented flag. It
  covers at least: the HTTP API (`docs/api.md`); the `serve` first stdout
  line (D-016); `serve --exit-on-stdin-close`, `--allow-origin`,
  `--ui-port-hint` (ignored); exit codes, including `EXIT_RESTART` and 76
  (GPU retry); `ORAG_HOME`, `config.toml` and the supervisor variables;
  `gpu-probe` output; `eval bench` and `eval retrieval` with every flag the
  bench uses (`--gpu-auto`, `--index-dir`, `--no-context-budget`,
  `--first-only`, `--exit-on-stdin-close`, ...) and their output files; and a
  compatibility promise (what may change in a minor release). Hidden flags
  the app relies on become documented and stay.
- A build guide (`docs/building.md` or the README) for Linux, macOS and Windows,
  CPU and Vulkan builds, with the CPU floor and toolchain versions.
- A new decision record replacing the parts of D-016 and D-021 that the split
  changes.

## `orag-ui` layout

- `engine/` (submodule, pinned tag), the page, the desktop app, packaging,
  scripts, workflows, docs, CHANGELOG, a decisions file.
- Its own Cargo workspace at the repository root with the desktop app as a
  member and its own `[workspace.package]` (version, edition, rust-version,
  license, and `repository = "https://github.com/orkono/orag-ui"`), so
  the app versions independently; `orag`'s root `Cargo.toml` drops
  `apps/desktop` from its members. The engine is built with `cargo` inside
  `engine/` and is not a workspace member.
- Its own `.cargo/config.toml` carrying the desktop-relevant settings that
  today live in `orag`'s (forced `MACOSX_DEPLOYMENT_TARGET = "14.0"`, the
  Windows static C runtime). Cargo merges ancestor configs into builds run in
  `engine/`, so every key both files set must have identical values; a CI
  check compares the shared keys of the two files directly (`orag-ui`'s
  `.cargo/config.toml` against `engine/.cargo/config.toml`: `[env]` entries
  with their `force`/`relative` flags, `LLAMA_STATIC_CRT`, the CMake
  toolchain entry, and the per-target `rustflags`).
- The desktop app tells its two pages apart by path, not by origin: today
  `on_local`/`on_service` compare origins (`apps/desktop/src/app.rs`), and
  once both pages come from the app's origin the service page would never be
  recognised, silently disabling the bench channel and the desktop smoke.
  The channel rules and their tests (bench `run` only from the service page,
  `resume` only from the local page) are kept.
- The page is bundled into the app and loaded from it, and talks to the engine
  over the API. The app still starts and supervises the engine as today.
- CI builds the engine from `engine/` (CPU and Vulkan), then the app, then the
  installers, on the same three runners as today, with the existing smoke
  tests (installer without a Vulkan driver, desktop smoke).
- Releases: draft, pre-release for `-` versions, as today, with `SHA256SUMS`
  and build provenance, from `orag-ui`.

## Left to the builder

- Step order and how the move is staged so neither repository is broken in
  between.
- The exact allowed-origins mechanism, within the constraints above.
- Which unnamed scripts move, by the rule above.
- How the page finds the engine's API address when it is no longer served from
  the same port (the app already reads the first stdout line).
- Whether `orag-ui` keeps the page as plain files or adds a build step (today
  there is none).
