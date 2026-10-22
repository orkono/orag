# Changelog

All notable changes to ORAG are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/). Every change merged to `main`
bumps the version (see `.docs/orag-decisions.md`, D-017).

## [0.6.0-alpha.11] - 2026-10-22

- Step 88d: the GPU bench's CPU floor. `.github/workflows/bench-reference.yml` runs the bench's own CPU runs for both tiers on the v0.5 CPU reference runner; its result, `eval/bench/v06-cpu-reference.json` (the 4B finds 8 of 9 facts and abstains on 055 and 058; the 9B finds 9 of 9 and abstains on 058), is built into the desktop app, so a bench on a PC checks its CPU answers against it. `.docs/benchmarks/v0.6-gpu.md` records the reference.

## [0.6.0-alpha.10] - 2026-10-21

- Step 88c: the GPU bench is wired to the page. Pre-releases of the Windows and Linux desktop apps show **Ekran kartı ölçümü / Graphics card bench** in the help panel; it opens a one-button view (`?bench=1`, desktop only) whose button navigates to `orag-bench://run`. The app acts on it only from the service page and only once at a time: it pauses the service (keeping its GPU policy), runs the bench on a thread, shows the progress and **Durdur** on its status page (`orag-bench://cancel`, only from that page while it runs), opens the results file's folder when done, and **Devam** (`orag-bench://resume`) resumes the same policy. Quitting cancels the bench and waits for it. `orag eval retrieval` takes a hidden `--gpu-auto`, which the bench uses to set `gpu = "off"` aside; the lavapipe workflow checks that an index-only bench and `--gpu-auto` stay on the GPU with `gpu = "off"` while plain retrieval keeps the setting. `docs/desktop.md` describes the bench.

## [0.6.0-alpha.9] - 2026-10-21

- Step 88b: the desktop app's GPU bench core (`apps/desktop/src/bench`), not wired to a button yet: the run order (index A on the CPU, per tier three paired CPU/Vulkan rounds, index B on the GPU, retrieval across both indexes in both directions) with its skip rules; each job as a contained child with `--exit-on-stdin-close`, a 2 h timeout, answer-line progress and its report; the merge into `orag-bench/1` (cold and warm medians, warm speed-up, the per-fact pass rule, unanswerable questions by abstention, the CPU floor against a reference, retrieval within 0.02, full offload, and the gate inputs); the v0.2 corpus, the 10 bench questions and the retrieval labels built into the desktop app only; the results file written whole through a temp file. The supervisor can pause and resume with the same GPU policy, so a session's CPU fallback survives the bench. `orag eval bench --index-only` needs only the embedder; `orag eval retrieval` takes a hidden `--exit-on-stdin-close`.

## [0.6.0-alpha.8] - 2026-10-21

- Step 88a: a hidden `orag eval bench` runs the GPU bench's fixed questions (`eval/datasets/v06-bench.jsonl`, 10 v0.2 dev lines) on one backend for one tier: `--build-index --index-only` builds the index once, later runs reuse it with nothing embedded again, `--first-only` answers only the first question for another cold start. It prints one JSON line per answer, writes a run report (build, models, sampler, load time, both models' offload, every answer with `found_facts`), reports a tier whose models are not installed as `not-installed`, sets `gpu = "off"` aside, and exits when stdin closes with `--exit-on-stdin-close`. Answer eval results gain `found_facts`: which expected texts were found, by index.

## [0.6.0-alpha.7] - 2026-10-21

- Step 87: `GET /v1/version` and `GET /v1/models` report `gpu` (backend, whether the answer model is on a graphics card and how much of it, the card, and why not), computed from where the model was really loaded; the desktop's fallback reason only fills `reason`. The page's sidebar shows the graphics card line, the last answer's time and where it ran, and "Ekran kartını yeniden dene" after a desktop fallback: `POST /v1/system/gpu-retry` exits with 76 for the desktop app to try the card again, and answers 409 anywhere else (`orag start` removes `ORAG_GPU_RETRY` from its child). `speed.gpu_p50_answer_s` is `null` until it is measured on a real card.

## [0.6.0-alpha.6] - 2026-10-21

- Step 86b: the Windows and Linux desktop bundles and CLI archives carry `orag-vulkan` beside `orag` (`tauri.windows.conf.json`, `tauri.linux.conf.json`, `scripts/stage-sidecar.sh`), so the app's GPU choice from step 86 is live; macOS is unchanged. Its imports are checked with `check-binary-deps.sh --vulkan` in the desktop and release workflows. The desktop smoke test on Windows and Linux runners, which have no GPU, checks that the app tried `orag-vulkan` and runs on the CPU with the reason in `gpu-fallback.json`; on Windows without a Vulkan loader the reason must be `loader-missing`. `docs/desktop.md` and the README explain the graphics card choice.

## [0.6.0-alpha.5] - 2026-10-21

- Step 86: the desktop app chooses `orag-vulkan` or `orag` on every start (`apps/desktop/src/gpu.rs`): `gpu = "off"`, then a fallback marker (`gpu-fallback.json` in the service home, this version, under 7 days), then `orag-vulkan gpu-probe` (killed after 10 s). A GPU sidecar that never gets ready, hangs past the startup timeout or crashes a second time moves the session to the CPU, writes the marker and passes the reason in `ORAG_GPU_FALLBACK_REASON`; exit 75 restarts the same sidecar; exit 76 clears the marker and tries the GPU again. On Windows the app turns system error dialogs off before its first spawn, so a missing `vulkan-1.dll` shows none. Bundles do not carry `orag-vulkan` yet (step 86b).

## [0.6.0-alpha.4] - 2026-10-21

- Step 85b: the gpu-retrieval workflow runs seed retrieval with the real embedder on `orag`, on `orag-vulkan` (Mesa's lavapipe as the GPU) and across them: a CPU-built index queried on the GPU and the reverse. Each must reach hybrid recall@10 0.9 with x-001..x-003 in the answer context and stay within 0.02 of the CPU run (`scripts/check-gpu-retrieval.sh`). `orag eval retrieval` gains a hidden `--index-dir` that keeps an index or reuses the one a run left there.

## [0.6.0-alpha.3] - 2026-10-21

- Step 85: `orag-vulkan` puts each model on one GPU, fitted to its free memory by llama.cpp's fit with exactly the context the model then uses (embedder, generator, reranker), or loads it CPU-only with no GPU device at all. The device is decided once per process: `gpu = "off"` in `config.toml` (new key, default `"auto"`), then `ORAG_GPU_DEVICE`, then the automatic choice; a fit that finds no room, a fit with zero GPU layers, or a model or first context that cannot be allocated after the fit falls back to a CPU-only load with its reason. A mixture-of-experts fit that keeps expert tensors in host memory counts as a partial offload. CI checks every branch on Mesa's lavapipe. The CPU `orag` and macOS are unchanged.

## [0.6.0-alpha.2] - 2026-10-21

- Step 84: `orag-vulkan gpu-probe` prints the GPU devices llama.cpp's Vulkan backend sees (index, driver name, discrete/integrated, total and free memory) and the one ORAG would use: the discrete GPU with the most free memory and at least 2 GiB free; integrated GPUs and software rasterisers (llvmpipe, SwiftShader) are never chosen. The CPU `orag` reports no devices. CI checks the probe with no Vulkan driver and with Mesa's lavapipe as the only device.

## [0.6.0-alpha.1] - 2026-10-21

- GPU support for Windows and Linux begins (plan `.docs/plans/2026-10-21-orag-v0.6-gpu-plan.md`, reviewed by muse and gpt-6-astra). Step 83 adds the `orag-vulkan` build: the service with llama.cpp's Vulkan backend (`scripts/build-vulkan.sh`, feature `vulkan`), whose only extra import is the system Vulkan loader (`check-binary-deps.sh --vulkan`; D-001 amended). A hidden `gpu-probe --fit <gguf>` runs llama.cpp's memory fit in that build to prove it links. The CPU `orag` is unchanged and nothing ships yet.

## [0.5.0] - 2026-10-20

- **ORAG v0.5.0.** The setup page shows this computer and the answer models that fit it, and downloads the one the user picks: the default `qwen3.5-4b-q4_k_m` (recommended) or `qwen3.5-9b-q4_k_m` (larger, slower). Each tier has a RAM floor and a recommended RAM measured on Mac (Metal) and Linux (CPU); a hand-set generator is kept; the choice is written to `config.toml` only after the download verifies (steps 81-82). Packages are still unsigned.
- Release gates on the candidate `0.5.0-alpha.1`: real-model check unchanged from v0.4.1 (v02 hybrid in context 0.931, hybrid p95 14.4 ms on Metal; answers expected facts 0.925, abstention P/R 0.944/0.850, no uncited answer; seed recall@10 1.000); D-003 dense search at 100k p95 97.3 ms on Apple M5 Max and 226.3 ms on the Linux reference (both PASS). Answer time on a 4-vCPU CPU without a GPU is about 108 s (4B) and 194 s (9B), so GPU support for Windows and Linux is the next step.

## [0.5.0-alpha.1] - 2026-10-20

- The setup page shows this computer (memory, CPU, cores, graphics, free disk) and the answer models that fit it, and downloads the one the user picks (steps 81-82). Two tiers are offered: the default `qwen3.5-4b-q4_k_m` (recommended) and `qwen3.5-9b-q4_k_m` ("Large: slower, uses more memory"); the 2B failed the quality rule (abstention recall 0.200) and is not offered. Each tier has a hard RAM floor (4B 7 GiB, 9B 11 GiB) and a recommended RAM (8 / 12 GiB) from the larger of the Mac and Linux CPU peaks; between them it is offered with a warning (`.docs/benchmarks/v0.5-model-tiers.md`).
- The chosen generator is written to `generation_model` in `config.toml` only after the download verifies; comments, line endings and a symlinked file are kept. A hand-set generator is offered as "Current". `GET /v1/models` gains `machine` and `options`; `POST /v1/models/download` takes an optional `{"model": id}` (`docs/api.md`). Missing AVX2/FMA/F16C/BMI2 blocks only the Windows build; elsewhere it is a slow-CPU warning.

## [0.4.1] - 2026-10-20

- **ORAG v0.4.1.** The web UI and the desktop app's status page speak Turkish and English, with a TR/EN menu; the page opens in Turkish unless English was chosen (step 78). Every release now carries `SHA256SUMS` and GitHub build provenance for each file, and the macOS desktop build signs and notarizes once the owner adds the Apple secrets (step 79, `docs/signing.md`); the packages of this release are still unsigned. The desktop app no longer reads its window's address, which could crash it on macOS before a page had loaded.
- Release gates on the candidate `0.4.1-alpha.2`: real-model check unchanged from v0.4.0 (v02 hybrid in context 0.931, hybrid p95 14.2 ms on Metal; answers expected facts 0.925, abstention P/R 0.944/0.850, no uncited answer; seed recall@10 1.000); D-003 dense search at 100k p95 104.5 ms on Apple M5 Max and 222.4 ms on the Linux reference (both PASS). The release workflow's macOS desktop smoke passed on GitHub.

## [0.4.1-alpha.2] - 2026-10-20

- **Release checksums and provenance (step 79).** Every release now carries `SHA256SUMS` and GitHub build provenance for each file (`gh attestation verify <file> --repo orkono/orag`). The macOS desktop build signs and notarizes once the `APPLE_*` repository secrets are set and is built unsigned without them; Windows signing waits for the owner's choice of provider. See `docs/signing.md`.
- Fix: the desktop app could crash on macOS when it read the window's address before a page had loaded (closing the window during startup, or in the release smoke test). It no longer reads the address: each page reports a language choice to the app the moment it is made, and the app saves it at once.

## [0.4.1-alpha.1] - 2026-10-20

- **Turkish and English web UI (step 78).** Every UI string lives in one dictionary (`ui/i18n.js`, Turkish and English with the same keys); a TR/EN menu in the top bar switches the page live, including the setup and download progress views, and the choice is kept in the browser (`orag.lang`). The page always opens in Turkish unless English was chosen; the browser language is not used (owner decision). Sizes and percentages follow the language ("3,1 GB", "%45" / "3.1 GB", "45%"). The desktop status page gets the same menu. Server responses and the API are unchanged; tests check that both languages have the same keys and placeholders and that no Turkish text is left outside the dictionary.
- The v0.4.0 desktop installer was checked by the owner on a real Windows PC: *Modelleri indir* downloaded both models and ORAG came up ready by itself.

## [0.4.0] - 2026-10-19

- **ORAG v0.4.0.** The setup page downloads the models with one button (step 76): pinned Hugging Face revisions, SHA-256 checked, resumable, with progress, cancel and a free-space check; ORAG restarts by itself under `orag start` and in the desktop app, so a new install needs no hand-made model packs. Offline packs still work, and `models.download = false` turns the download off. Five code-review rounds (gpt-6-astra) hardened cancel, links in the models folder, proxies and TLS time limits.
- Release gates on the candidate `0.4.0-alpha.1`: real-model check unchanged from the recorded baselines (v02 hybrid in context 0.931, hybrid p95 14.2 ms on Metal; answers expected facts 0.925, abstention P/R 0.944/0.850, no uncited answer; seed recall@10 1.000); D-003 dense search at 100k p95 96.4 ms on Apple M5 Max and 225.7 ms on the Linux reference (both PASS). The packages are still unsigned. D-020: release tags are now pushed by the agent (owner decision).

## [0.4.0-alpha.1] - 2026-10-19

- **Download the models from the setup page (step 76, D-012 amendment, owner decision).** In setup mode the page leads with one *Modelleri indir* button (with the total size): ORAG downloads the configured models from pinned Hugging Face revisions (the same pins as `scripts/fetch-model-pack.sh`, checked by a test), verifies every file by SHA-256, installs them exactly as `orag models import` would, and restarts by itself under `orag start` and in the desktop app. The page shows per-model and total progress, *Durdur* and *Tekrar dene*/*Devam et*; a download resumes where it stopped (HTTP `Range`), survives a page reload, checks free disk space first, honours `HTTPS_PROXY`/`HTTP_PROXY`/`NO_PROXY`, and fails after 60 s with no data. The offline import stays under *Model paketlerim var (çevrimdışı kurulum)*.
- API: `POST`, `GET` and `DELETE /v1/models/download` (setup mode only; `409` while another download or import runs), and a `download` member on `GET /v1/models` (`available`, `code`, `reason`, `bytes`). New config key `models.download` (default `true`); `false` turns the download off (`403 download_disabled`). Downloadable: qwen3-embedding-0.6b-q8_0, qwen3.5-4b-q4_k_m and qwen3-4b-instruct-2507-q4_k_m.
- New dependency `ureq` 3.4.2 with rustls and ring (no OpenSSL). TLS trusts the bundled Mozilla roots, so a proxy that re-signs HTTPS with a private CA is refused; offline packs remain the way in there. Checked end to end on macOS against Hugging Face from an empty home: both default models (3,380,111,382 bytes) downloaded, a cancel at 313 MB resumed from the same byte, and the server started ready with them.

## [0.3.0] - 2026-10-15

- **ORAG v0.3.0.** A desktop application for macOS, Windows and Linux (Tauri, D-016) runs the service in its own data folder, shows the page in its own window, restarts the service after setup and after crashes, and leaves no service behind when it closes or is killed (`docs/desktop.md`). The engine gains cross-language fusion (step 48), Turkish stem terms in lexical search (step 51), heading detection in plain text and PDF (step 52) and an `uncited` flag on answers that cite no source (step 67).
- Evaluation: the 200-question v02 set was rewritten by one model family and audited by another (step 49); scorer defects were fixed (steps 54, 56, 58); `orag eval retrieval` measures the prompt context exactly as the answer path builds it (step 55); a fresh 130-row v0.4 holdout was added (step 60). The set is still synthetic and not audited by people (D-018).
- Tried and not adopted, each under a rule written before scoring: context diversity (step 50), stricter manual headings (step 53), a 10-chunk context (step 57), bge-m3 as the default embedder (steps 59, 66: better retrieval and faster on CPU, but one unsupported uncited answer), rerankers (steps 61-65: +8 dev questions, but 214 ms or more of latency on Metal and seconds on a CPU), preventing uncited answers at generation time (step 68: no dev target). The default models stay Qwen3-Embedding-0.6B and Qwen3.5-4B.
- Release artifacts: besides the CLI archives and the Windows CLI installer, the draft release carries the desktop app as `ORAG_<version>_aarch64.dmg`, `ORAG_<version>_x64-setup.exe` (with the offline WebView2 installer), `ORAG_<version>_amd64.AppImage` and `.deb`, each smoke-tested in the release build. Not signed or notarized yet.
- Release gates on the candidate `0.3.0-alpha.28`: real-model check unchanged from the recorded baselines (v02 hybrid in context 0.931, hybrid p95 13.9 ms on Metal; answers expected facts 0.925, abstention P/R 0.944/0.850, no uncited answer; seed recall@10 1.000); D-003 dense search at 100k p95 100.8 ms on Apple M5 Max and 208.7 ms on the Linux reference (both PASS).

## [0.3.0-alpha.28] - 2026-10-15

- **Desktop app documented (step 74).** New `docs/desktop.md`: install files and sizes, the unsigned-app prompts per system, the first start in setup mode, the app's own data folder per system, and how the window and the service behave. README status and a *Desktop app* section; D-016 records what was built.

## [0.3.0-alpha.27] - 2026-10-15

- **Desktop bundles in releases (step 73).** The release build wraps the same checked `orag` binary in the desktop app on all three platforms, runs the desktop smoke test and uploads the macOS `.dmg`, the Windows NSIS installer (with the offline WebView2 installer), the Linux `.AppImage` and `.deb` next to the CLI archives, so a tag's draft release carries them.

## [0.3.0-alpha.26] - 2026-10-15

- **Desktop window (step 72, D-016).** The app starts its sidecar in a private home (`<app data>/orag`, `config.toml` with both ports on `127.0.0.1:0` written once), opens the service page when it is ready and shows restarts and failures on its own status page (the last stderr lines as text, never as HTML). Navigation stays on the app page and the current service page; other links open in the system browser; the service page gets no Tauri commands. A second launch focuses the existing window. `scripts/smoke-desktop.sh` runs the built app on all three platforms: the service page sends a POST from inside the webview, the app closes its window and must exit by itself, and its sidecar must be gone.

## [0.3.0-alpha.25] - 2026-10-15

- **Desktop sidecar supervisor (step 71, D-016).** `orag serve --exit-on-stdin-close` (hidden) stops gracefully when stdin reaches end of file, also during startup, so a GUI parent without a console can stop it on every OS. In `apps/desktop` a supervisor starts `orag serve` supervised, reports its two address lines, restarts it after exit 75 and after crashes (at most 3 times in 60 s, then fails with the last 20 stderr lines), fails a start that prints no lines within 180 s, and stops it by closing stdin, killing it after 10 s. No orphan when the app dies: the app enters a kill-on-close Job on Windows, the child asks for SIGTERM on parent death on Linux, and the closed stdin covers macOS; a test kills the app hard and checks the real service is gone within 5 s.

## [0.3.0-alpha.24] - 2026-10-15

- **Desktop app skeleton (step 70, D-016).** New workspace member `apps/desktop` (Tauri 2.12, outside `default-members`, so the service's `cargo build`/`cargo test` are unchanged): a window with a loading page, no Tauri commands granted, the `orag` executable bundled next to the app (`scripts/stage-sidecar.sh`, `externalBin`), icons from the ORAK icon. New `desktop` workflow tests and bundles it on macOS (`.dmg`), Windows (NSIS with the offline WebView2 installer) and Linux (`.AppImage`, `.deb`). It does not start the service yet. D-023: a desktop sidecar runs supervised and is restarted after setup and crashes. D-012: the in-app model downloader moves to v0.4.

## [0.3.0-alpha.23] - 2026-10-15

- **Desktop application plan (step 69, D-016).** `.docs/plans/2026-10-15-orag-v0.3-desktop-plan.md`: a Tauri v2 app in `apps/desktop` that runs `orag serve` as a supervised sidecar on loopback, shows the built-in page in its own window, restarts the service after setup and crashes, and leaves no orphan (stdin-close shutdown on every OS, a Windows Job assigned at process creation, Linux parent-death signal). v0.3.0 waits for it. Reviewed by gpt-6-sol; no code yet.

## [0.3.0-alpha.22] - 2026-10-15

- **Preventing uncited answers: not pursued (step 68).** With the default models 1 of 109 answers is uncited, and it is in the v0.4 holdout, so a stronger citation prompt or a retry has no dev target to be judged on; a retry would also change the streaming API. No code change; the step 67 notice stays, and uncited answers and `v2-ans-047` wait for a larger answer dev set in v0.4 (`.docs/benchmarks/v0.3-uncited-prevention.md`).

## [0.3.0-alpha.21] - 2026-10-15

- **Uncited answers are marked (step 67).** An answer that is not an abstention and cites no source (no `[n]`, or only numbers no source has) now has `"uncited": true` in the query response and the `done` stream event (`docs/api.md`); the web page shows it with the notice "Bu yanıt hiçbir kaynağa atıf yapmıyor; belgelerden doğrulayın." The answer text is unchanged and no model call is added. `orag eval answers` reports an "uncited answers" count per run and per category.

## [0.3.0-alpha.20] - 2026-10-15

- **Windows smoke numbers corrected (step 66).** On the Windows runner bge-m3 had the higher MRR@10 (0.905 → 0.929) and recall@5 (0.929 → 1.000); the benchmark record had the pair reversed. No decision changes.

## [0.3.0-alpha.19] - 2026-10-15

- **bge-m3 decided on the v0.4 holdout (step 66, D-007): not adopted.** It passed every retrieval and cost condition of a rule written before the holdout was read (in context +3 with no loss on the v0.4 holdout, +7 on v0.2; hybrid p95 11 vs 14 ms on Metal and 115 vs 389 ms on an x86-64 CPU; +149 MB), but with its context the generator once answered without a citation and added a detail the source does not give, which the answer condition forbids. The default embedder stays `qwen3-embedding-0.6b-q8_0`. New `model-cpu` workflow measures embedders on GitHub x86-64 Linux and Windows runners (`.docs/benchmarks/v0.4-bge-m3.md`).

## [0.3.0-alpha.18] - 2026-10-14

- **Reranker on sentence windows (step 65, D-005).** `orag eval retrieval --rerank-window N` scores only the sentences of each chunk around the query terms (long lines split around the match, unmatched chunks scored whole, the rate reported per question). Every run still adds 214 ms or more of hybrid p95 on Metal (budget 150 ms), and with bge-m3 the reranker adds one net dev point; retrieval is unchanged, the reranker track is closed for v0.4 and the v0.4 holdout stays unread (`.docs/benchmarks/v0.4-rerank-windows.md`).

## [0.3.0-alpha.17] - 2026-10-14

- **Reranker latency re-measured (step 64, D-005).** Batching reranker pairs into one decode is 8-12× faster on single sentences but not on 384-512-token chunks, so it was reverted; the best reranker variant still adds 825 ms of p95 on Metal (budget 150 ms), and on a 6-core x86-64 CPU the fastest adds 12.5 s. Retrieval is unchanged and the v0.4 holdout stays unread (`.docs/benchmarks/v0.4-reranker-batched.md`).

## [0.3.0-alpha.16] - 2026-10-14

- **v0.4 retrieval decision: not adopted (step 63, D-005).** 26 combinations of embedder (qwen3-embedding, bge-m3) and reranker (none, Qwen3-Reranker-0.6B, bge-reranker-v2-m3; depth 16-30; leaders pinned or not) were scored on v0.2 dev under a rule written before scoring. The best reach +8 in context with no losses (bge-m3 with a reranker at depth 30), but every reranker variant adds 204-2010 ms of p95 latency against a 150 ms budget, and bge-m3 alone misses its gate, so retrieval is unchanged. The v0.4 holdout stays unread; the next step batches reranker pairs (`.docs/benchmarks/v0.4-retrieval-decision.md`).

## [0.3.0-alpha.15] - 2026-10-14

- **Reranker for experiments (step 62, v0.4 plan Tasks 1-3 and 5).** A `Reranker` trait, a llama.cpp cross-encoder with rank pooling (pair input for bge-reranker-v2-m3, the GGUF rerank template for Qwen3-Reranker-0.6B; the passage is cut to fit, never the question; vocabulary special tokens are broken in question and passage), and an optional rerank step that scores the first fused candidates before the answer pool is cut, falling back to the fused order on error or timeout. Only the eval can switch it on (`orag eval retrieval --rerank-model <id>`); the served path is unchanged. Both candidates pass the qualification triples. Reranker packs use the new `reranking` role.

## [0.3.0-alpha.14] - 2026-10-14

- **v0.4 reranker plan (step 61).** `.docs/plans/2026-10-14-orag-v0.4-reranker-plan.md`: a `Reranker` trait with llama.cpp rank pooling, an optional rerank step over the first fused candidates (scored before the answer pool is cut, falling back to fused order on error or timeout), eval flags, and one pre-registered rule that decides embedder (bge-m3) and reranker together with a single read of the v0.4 holdout. Reviewed by gpt-6-sol; no code yet.

## [0.3.0-alpha.13] - 2026-10-14

- **A fresh holdout set for v0.4 (step 60, D-018).** `eval/datasets/v04-holdout-retrieval.jsonl` (100 questions) and `v04-holdout-answers.jsonl` (30 probes) ask about facts the v0.2 set does not use, over the same corpus, so the v0.4 model decisions (bge-m3, a reranker) can be confirmed on questions nothing was tuned on; each chosen candidate is scored on it once. Written by `claude-opus-5-5`, audited by `gpt-6-sol` (23 of 130 rows replaced), not human-checked. Current default: hybrid in context 0.909, expected facts 0.950.

## [0.3.0-alpha.12] - 2026-10-14

- **10-chunk answer context tried and rejected (step 57, D-005).** Five leaders per list in a 10-chunk context recovered two dev near misses (dev hybrid in context 81/89 → 83/89; holdout unchanged), but answers became 38% slower at the median and one unanswerable question got an unsupported answer, so the context stays at 8 chunks (`.docs/benchmarks/v0.3-fusion-near-miss.md`).

## [0.3.0-alpha.11] - 2026-10-14

- **bge-m3 re-measured, not adopted (step 59, D-007).** On the rewritten v0.2 set and the current engine `bge-m3-q8_0` raises hybrid in context from 0.931 to 0.971 and fixes both remaining multi-document misses, but its recall@10 dev gain is +5 against the pre-registered ≥ 6, so the default embedder stays `qwen3-embedding-0.6b-q8_0`; bge-m3 is the first candidate of the v0.4 model review (`.docs/benchmarks/v0.3-bge-m3.md`).

## [0.3.0-alpha.10] - 2026-10-14

- **Hedged refusals are abstentions (step 58, D-005).** An answer whose first sentence, uncited, says the documents do "not explicitly" state or mention the answer is now flagged as abstained, like "not stated" before (`v2-ans-041`). The answer eval no longer counts an expected fact inside an abstained answer (`v2-ans-011` said the period is not stated while quoting it). On the v0.2 answers: expected facts 0.950 → 0.925, abstention P/R 0.941/0.800 → 0.944/0.850; no answer text changed, only how two are counted.

## [0.3.0-alpha.9] - 2026-10-14

- **Number words in the answer scorer (step 56, D-018).** An expected fact in words no longer matches inside a longer number: `iki iş gün` is not found in "on iki iş gündür", `iki` not in "iki yüz", `10` not in "10 bin", `four hours` not in "twenty-four hours", `on sekiz bin` not in "on sekiz bin beş yüz" (Turkish and English, by language, so English "on two days" still matches). Re-scoring the current answers changed no probe; expected facts stay 0.950.

## [0.3.0-alpha.8] - 2026-10-14

- **The retrieval eval scores the context the model really sees (step 55, D-018).** `orag eval retrieval` now chooses the answer context as `orag serve` does: from 16 fused candidates, at most 8 chunks, skipping any that does not fit the answer model's token budget (it loads the answer model only to count tokens). The report names that model (`context_budget`) and counts skipped chunks. On every current set nothing changed (v02 hybrid in context 0.931, per question identical, no chunk skipped), so earlier numbers stand (`.docs/benchmarks/v0.3-eval-parity.md`).

## [0.3.0-alpha.7] - 2026-10-14

- **Working-day answer probes fixed (step 54, D-018).** Five probes of `v02-answers.jsonl` expected only `iş günü`, so the correct "10 iş gündür" scored as a miss; they now expect `iş gün`, which every inflection matches while a wrong number in digits still does not (number words such as `on iki` for `iki` were and stay a scorer limitation). On 0.3.0-alpha.6 expected facts read 0.950 instead of 0.925 (`v2-ans-038`); a scoring correction, not an engine gain (`.docs/benchmarks/v0.3-set-rewrite.md`).

## [0.3.0-alpha.6] - 2026-10-14

- **Manual-style heading variant tried and rejected (step 53).** On an appliance manual PDF, heading detection finds many false headings (diagram labels, paragraphs in capitals). A stricter variant (no hyphen-broken lines, no runs of three or more capitals lines, a capitals title needs real text after it) changed no v0.2 question and kept 229 of 230 step 52 headings, and cut the manual's headings from 105 to 16, but missed its pre-registered limit of 10 (also when re-scored once after two implementation defects were fixed), so parsing is unchanged (`.docs/benchmarks/v0.3-manual-headings.md`, D-024).

## [0.3.0-alpha.5] - 2026-10-14

- **Lexical prefix stems (step 51, D-006).** Query words longer than 7 characters, with a letter and no digit, also search their first 7 characters as a prefix (`dinlenmeyi` finds `dinlenmek`); query side only, no reindex. Chosen among stems of 4-7 characters by a rule written before scoring: on the v02 set hybrid in context 0.920 → 0.931 (dev +1, holdout +1, no losses), the inflection set +1. Answers are net unchanged once scorer artefacts are removed (the raw expected-facts rise 0.875 → 0.925 is inflated by the `iş günü` probe defect); v2-ans-011, v2-ans-041 and v2-ans-047 are known answer regressions to recheck (`.docs/benchmarks/v0.3-lexical-stems.md`).
- **Deeper ranks in the retrieval eval.** A hidden `orag eval retrieval --rank-depth <10-50>` reports each label's lexical, dense and hybrid rank below 10, to tell evidence fusion dropped from evidence no list returns; recall, MRR, nDCG and in-context are unchanged (latency covers the deeper retrieval).

## [0.3.0-alpha.4] - 2026-10-14

- **Headings in plain text and PDF.** TXT and PDF documents have no heading markup, so heading lines are now recognised: lines in capitals (ranked by `KISIM`/`PART` and `BÖLÜM`/`CHAPTER`/`SECTION`), `-- name --` lines, and titles numbered with a Roman numeral or a letter (`IV. Egemenlik`, `A. Özel hayatın gizliliği`). A chunk now starts at such a heading and sources show its heading path, as for Markdown and DOCX (D-024). Running headers repeated on most PDF pages are not headings. On the v0.2 set hybrid in context rises from 0.909 to 0.920; answers stay as correct (two of three string-check misses are correct answers worded “iş gündür”) and prompts are shorter (p50 1.7 s → 0.9 s). Decided by a rule written before scoring (`.docs/benchmarks/v0.3-plain-headings.md`). No reindex is forced: existing TXT and PDF documents get headings when their collection is reindexed.

## [0.3.0-alpha.3] - 2026-10-13

- **Context diversity tried and rejected.** A per-document cap in the answer context and a tier for chunks both retrieval lists found were scored on the rewritten v0.2 set under a rule written before scoring; neither passed (the consensus tier: dev −1, holdout −2, cross-language 0.880 → 0.680), so retrieval ordering is unchanged. The remaining multi-document misses are single-passage recall problems (`.docs/benchmarks/v0.3-context-diversity.md`, D-005).

## [0.3.0-alpha.2] - 2026-10-13

- **v0.2 evaluation set rewritten (step 49, D-018).** The 200 questions and 60 answer probes over the unchanged `eval/corpus/v02` were written again by `claude-opus-5-5` and audited row by row by `gpt-6-sol`, a model of another family; the audit changed 43 of 260 rows (1 wrong, 42 weak). Categories, counts and split rules are unchanged; the set is still not human-checked. New baseline with the current engine: hybrid in context 0.909 (multi-document 0.800, cross-language 0.880), answers with the expected facts 0.925 (`.docs/benchmarks/v0.3-set-rewrite.md`). 36 probes accept the other spellings of a correct answer (`18|onsekiz`, `2 adet|iki adet`), as the old set did. Earlier model and fusion decisions stay recorded as old-set results.

## [0.3.0-alpha.1] - 2026-10-13

- **Cross-language retrieval (fusion).** Each list's top 4 hits now lead the fused order (both #1s first, then #2-#4), instead of only each list's #1 (D-005). On the v0.2 set hybrid in context rises from 0.886 to 0.943 (cross-language 0.600 → 0.840, multi-document 0.850 → 1.000), and answers with the expected facts from 0.850 to 0.925 (cross-language 2 of 5 → 5 of 5); answers take about 0.5 s longer and one more unanswerable probe is answered. Decided by a rule written before scoring (`.docs/benchmarks/v0.3-fusion.md`).

## [0.2.1] - 2026-10-12

- **Windows installer (Task 47).** `ORAG-Setup-<version>.exe` (Inno Setup 6) installs ORAG per user without administrator rights into `%LOCALAPPDATA%\Programs\ORAG`, with Start menu entries *ORAG* (`orag.exe start`), *ORAG modellerini indir (internet)* and the uninstaller, an optional desktop shortcut, and a Turkish/English `README-Windows.txt`. Uninstalling removes program files only and says that `%USERPROFILE%\.orag` (documents, models) stays; an upgrade closes a running ORAG first. The binary is not code-signed yet (D-001); README-Windows explains SmartScreen.
- `fetch-model-pack.ps1`, the PowerShell twin of `fetch-model-pack.sh`: the same presets, revisions and SHA-256 hashes (`scripts/check-pack-presets.sh` compares them in CI), resumable downloads, the same pack layout; it asks before downloading. Without `-Preset` it builds the two default packs into `%USERPROFILE%\ORAG-paketleri`.
- CI builds the installer on windows-2022 on every push, installs it silently, checks the installed `orag.exe` (`--version`, setup mode without models, a graceful stop on Ctrl-Break within 15 s via `examples/ctrl_break.rs`), uninstalls it and checks that data stays. `release.yml` attaches the installer to the draft release.
- `scripts/render-icon.sh` also renders `assets/orag-icon.ico` (16/32/48/256) for the installer.
- Installer review fixes: the scripts stop Git Bash from rewriting `/SWITCHES` into paths; `fetch-model-pack.ps1` enables TLS 1.2 on Windows PowerShell 5.1, makes `-OutDir` absolute, keeps a partial download when a resume fails and checks a complete `.part` instead of downloading it again; the installer is per user only and names the real data folder (or `ORAG_HOME`) after uninstalling; the smoke test checks the Start menu shortcuts, runs the pack script on Windows PowerShell 5.1 and proves a marker in `%USERPROFILE%\.orag` survives uninstall; `ctrl_break` refuses an address already in use; `check-pack-presets.sh` also compares the MIT text, the manifest keys and the default packs.

## [0.2.0] - 2026-10-12

- **ORAG v0.2.0.** The engine and the built-in web page are complete for now: page numbers in PDF sources, a `suspicious_layout` warning, 10 MB documents by default, setup mode and `orag start`, answer-level evaluation, a 200-question TR/EN set, and Windows x86-64 builds. The default models stay Qwen3-Embedding-0.6B and Qwen3.5-4B (D-007).
- Release artifacts: `release.yml` also builds Windows (`orag-<version>-x86_64-pc-windows-msvc.zip`, checked by `check-binary-deps.sh`), and a `v*` tag creates a **draft** GitHub release with every artifact for the owner to publish. `vector-scale.yml` also runs when a `bench/**` branch is pushed.
- `scripts/compare-v01.sh` scores the `v0.1.0` tag and the current build with the same embedder on `seed.jsonl` and on the text-only subset of the v02 set. For this release it **failed** on seed MRR@10 (0.903 against 0.939: one cross-language question, x-002, moved from rank 1 to rank 2 and stayed in the answer context) and improved on the 90-question subset (recall@10 0.856 → 0.911, MRR@10 0.700 → 0.754). The failure was waived; the evidence and the reformed rule for v0.3 are in `.docs/benchmarks/v0.2-seed-eval.md`.
- Release gates on the candidate `0.2.0-alpha.23`: real-model release check passed; D-003 dense search at 100k p95 98.1 ms on Apple M5 Max and 226.4 ms on the Linux reference (both PASS).
- README: a Windows section, the v0.2 status (the evaluation set is synthetic and its human audit is pending), and the v02 evaluation sets.

## [0.2.0-alpha.23] - 2026-10-12

- **Web page finished for v0.2 (Task 45).** A busy answer slot (`429`, which the server sends only after waiting for the slot itself) shows *Başka bir yanıt üretiliyor; bir kez daha denenecek.* and retries once after `Retry-After`. A stream that drops mid-answer (the body fails, not only ends early) or an unreachable server offers *Tekrar sor*, which asks again without losing a question being typed and is spent once used. The page's pure logic (page labels, warning labels, finish notices, the retry delay) lives in `format.js`, the SSE reader is tested with in-memory streams, both under `node --test` in CI on Ubuntu (a non-served `package.json` makes Node load them as ES modules on any version), and a router test checks that every module the page references is served with its content type and the CSP header.
- Acceptance, in headless Chrome against `orag serve` with real models from an empty home (supervised as under `orag start`), 13 scenarios passed: setup → import → restart → ready; seven files at once (four indexed; an oversized `.txt`, an `.xlsx` and a renamed fake `.pdf` each refused with the server's message); a streamed answer with linked citations and PDF page labels; *Durdur* frees the slot for the next question; an abstained answer; delete a document with confirmation; labelled inputs and live regions; version and models in the footer; the page notices a stopped server and recovers; a stream killed mid-answer offers *Tekrar sor*, which asks again and keeps a draft. Not exercised in the browser: the 429 retry (the server waits 120 s before it), the length and repetition notices, drag-and-drop, and collection create, delete and reindex (unchanged since step 36b).

## [0.2.0-alpha.22] - 2026-10-12

- **Model decision for v0.2 (Task 44, D-007): no change.** Qwen3-Embedding-0.6B and Qwen3.5-4B stay the defaults. The rule was written into `.docs/benchmarks/v0.2-model-decision.md` before scoring. BGE-M3 is better on every hybrid and dense headline number (hybrid recall@10 0.937 vs 0.891, and the only challenger that helps cross-language questions) but wins only +3 net questions on `dev`, where the rule asks for +6; it is the first candidate for the next review. multilingual-e5-large-instruct is rejected (worse on holdout and on multi-document questions); Qwen3-4B-Instruct-2507 stays the fallback generator (faster, but four expected facts fewer and a lower abstention F1).
- The bge-m3 and e5 packs ship the MIT text as `LICENSE` and the upstream model card as `MODEL_CARD.md` (upstream declares MIT only in the card); the bge-m3 preset allocates a 1024-token context instead of 8192. `scripts/fetch-model-pack.sh` sets the manifest license per preset and no longer picks it up from the environment.
- `tests/embedder_qualification.rs` prints the measured token counts, compares Metal and CPU vectors (documents and queries) through `ORAG_TEST_BACKEND_VECTORS` and fails when it is unset, and adds a hard-negative ranking set (a candidate must do at least as well as the default).
- `tests/embedder_qualification.rs` qualifies an embedding model before it is scored (unit 1024-d vectors, token counts against the reference tokenizer, ranking on ten hand-written triples, and vectors written out for a Metal-vs-CPU comparison); run with `ORAG_TEST_EMBEDDING_MODEL=<id>`.
- `scripts/fetch-model-pack.sh` has presets for `bge-m3-q8_0` and `multilingual-e5-large-instruct-q8_0` (pinned revisions, SHA-256 checked, MIT), so the comparison can be repeated; the manifest's `license` now follows the preset.

## [0.2.0-alpha.21] - 2026-10-12

- `orag eval answers` reports answer-level metrics (D-018) per sampler profile, overall and per category and split: `valid_citation_rate` (answered probes whose every `[n]` names a given source), `cited_evidence_coverage` (an expected text occurs in a chunk the answer cites: evidence coverage, not faithfulness), `expected_fact_rate`, abstention as a confusion matrix with unanswerable as positive (precision and recall, `null` when there is nothing to measure) and `finish_reasons`. The Markdown table leads with them; the JSON has each probe's `valid_citations` and `cited_evidence`.
- Experimental `--judge`: the local generator answers "Does the answer only state what the cited sources say? Answer YES or NO." per answered answerable probe that cites a source, seeing each source under the `[n]` the answer uses; reported as `experimental.judge_yes_rate` (`null` when nothing was judged). A judge that cannot run leaves the probe unjudged. No gate uses it.
- A valid citation needs at least one `[n]`; cited evidence counts only answers that were not refused; a rate with nothing to measure is `null` (`–` in the table), not 0.
- Real-model baseline on the v0.2 answer set (`.docs/benchmarks/v0.2-model-decision.md`): expected facts 0.850, cited evidence 0.825, valid citations 1.000, abstention precision/recall 0.773/0.850; cross-language is the weakest category (2 of 5).

## [0.2.0-alpha.20] - 2026-10-12

- **Setup mode (D-023).** Without installed model packs, `orag serve` no longer stops: it starts in setup mode, logs which models are missing, and runs the API and the page without a worker. Upload, query and reindex answer `503 models_missing` (before any body is read or any embedding space is checked, so an indexed collection never offers a reindex that would empty it). A broken install (a bad manifest, the wrong role, a folder that cannot be read) still stops startup, and a build without an inference backend fails at once.
- `GET /v1/models` reports `mode` (`setup` or `ready`), the configured models and whether each is installed. `POST /v1/models/import` (setup mode only) installs packs from a local folder given as a full path (`~/` allowed), verifying each by SHA-256; pack files must be regular files owned by ORAG's user, one import runs at a time, and a broken leftover install is named instead of skipped.
- Once both models are installed, ORAG restarts to load them: under the new `orag start` the server exits with code 75 and the launcher runs it again; under plain `orag serve` the page asks the user to restart.
- **`orag start`** runs `orag serve`, opens the page in the browser, restarts it after a model import (at most 3 times in 60 s), and on Ctrl-C lets it stop gracefully. If ORAG already runs, it only opens the page. `orag serve` is unchanged for scripts, services and the desktop sidecar (D-016).
- The page shows *Modelleri kur* in setup mode: what is missing, where packs come from, a folder field and *İçe aktar*; uploading and asking are disabled until the models are installed, and the page reloads once ORAG is back.

## [0.2.0-alpha.19] - 2026-10-12

- The default `max_document_mb` is now 10 (was 5); the allowed range stays 1–10. Uploads of up to 10 MB pass the JSON-escape and multipart body limits (tested with a 9.5 MB upload both ways).
- Parser limits are sized for the largest allowed document: a DOCX part may decompress to 192 MiB (was 64), with up to 8 million XML nodes (was 2 million); a PDF may yield 96 MiB of text (was 64); a parser child may return 128 MiB (was 64). Measured: a 10 MB text-only DOCX holds ~72 MB of XML at a 7x deflate ratio, and real Word markup deflates up to ~15x. A test pins these limits against `MAX_DOCUMENT_MB_LIMIT`.
- **Upgrade note:** an install whose `config.toml` leaves `max_document_mb` commented out follows the new default of 10 MB after a restart; one that pins `max_document_mb = 5` keeps 5. An existing `config.toml` still shows the old comment `# max_document_mb = 5` (the file is only written when missing): that comment is stale, so uncommenting it pins 5 MB. `GET /v1/version` shows the limit in effect.
- With 10 MB documents, four parallel JSON uploads can hold about 0.5 GB before the size check, and one large text-only document can keep the single ingest worker busy for many minutes (D-019 resource budget).

## [0.2.0-alpha.18] - 2026-10-12

- The v0.2 evaluation set (Task 42, D-018): `eval/datasets/v02-retrieval.jsonl` (200 questions in eight categories: factual, cross-language, multi-document, identifiers, tables, accentless, inflection, unanswerable; split into `dev` and `holdout` by document family) and `v02-answers.jsonl` (60 answer probes) over `eval/corpus/v02`, built by `scripts/make-v02-corpus.sh` from six fictional documents written for ORAG (PDF catalog, DOCX pump manual, two HR documents, warranty terms, price list) plus the older corpora. The set is model-authored and reviewed by a second model run, not human-checked (`eval/datasets/v02-audit.md`).
- `orag eval retrieval` reads the optional dataset fields `category`, `split`, `family` and `require_all` (answer probes: the first three), adds "By category" and "By split" tables, and writes every question with its label ranks to the JSON report (`per_query`). Every metric is now per question with one rule: a question's labels are alternative evidence (any one is enough), or all needed for `require_all` questions. Recall and the in-context share count the question as found by that rule; MRR uses the best label, or the last needed one; nDCG scores the best label against rank 1, or coverage of every label for `require_all`. Earlier sets have one label per question, so their numbers are unchanged. The loader refuses a family or a piece of evidence that sits in both `dev` and `holdout`, and `require_all` outside `multi_document`.
- Baseline with the default models: `.docs/benchmarks/v0.2-model-decision.md`. Hybrid retrieval puts the evidence of 0.886 of the answerable questions in the answer context; cross-language questions are the weak spot (dense alone 0.880, hybrid 0.600).

## [0.2.0-alpha.17] - 2026-10-12

- PDF pages whose extracted text looks garbled get an advisory `suspicious_layout` warning, one per reason, listing the pages: `unmapped characters` (U+FFFD or control characters at 1 % or more: a font without a usable ToUnicode map) or `words split into single letters` (at least 40 % of tokens in runs of four or more one-letter tokens). A page needs 200 non-space characters before it is judged; short lines, answer keys (`1. A 2. C`), formulas and icon-font bullets are no sign. The pages are still indexed; the warning says answers from them can be wrong.
- pdf_oxide's U+FFFD preservation is switched on so the unmapped-characters rule can see them; the characters are removed again before indexing, so indexed text is unchanged.
- The web page shows document warnings with Turkish labels plus the pages and reason they name ("Taranmış sayfalar okunamadı (s. 3–5)", "Bazı sayfaların metni bozuk olabilir (s. 2–3; harflere bölünmüş kelimeler)"); the full server text is the tooltip.

## [0.2.0-alpha.16] - 2026-10-12

- PDF sources carry their physical pages: every source in a query answer has `pages`, the first and last 1-based page of its chunk (`[12, 13]`), or `null` for TXT, Markdown and DOCX (whose place is `heading_path`). The web page shows them after the file name (`s. 12–13`).
- The PDF parser records each block's page; the chunker keeps the page range of the blocks in a chunk (a window of a split block has that block's page; a heading with no body keeps its own). Pages are provenance only: chunk boundaries and texts are unchanged, so the embedding space (D-009) and the lexical index stay valid; tested by chunking the same document with and without pages.
- **Upgrade note:** schema 3 adds `chunks.page_start` and `chunks.page_end` (nullable; the usual pre-upgrade backup is taken). PDF chunks indexed before this version have no pages until their collection is reindexed (*Yeniden indeksle*); nothing else needs a reindex. Older binaries refuse schema 3; to go back, restore the pre-upgrade backup.
- New fixture `tests/fixtures/two-pages.pdf` (`scripts/make-fixtures.sh`).

## [0.2.0-alpha.15] - 2026-10-12

- Windows x86-64 (`x86_64-pc-windows-msvc`) builds and is tested in CI next to macOS and Linux (D-001). `orag.exe` links the C runtime statically (no Visual C++ redistributable; llama.cpp gets the same runtime through `packaging/windows/static-crt.cmake`, because its CMake appends its own runtime flag after `/MT`; `check-llama-build.sh` checks it and names `cargo clean -p llama-cpp-sys-2` for a build directory configured before) and imports only system DLLs (`check-binary-deps.sh` reads its import table); llama.cpp is built for the CPU floor AVX2/FMA/F16C/BMI2 (Intel 2013+, AMD Zen), checked in its CMake cache by `check-llama-build.sh`.
- The DOCX/PDF parser child runs in a Windows Job Object: it dies with `orag serve`, is capped at 4 GiB of memory, shows no error dialog, and is killed together with anything it started. A crash (an exception status such as an access violation, stack overflow or abort) blames the file; plain exit codes and statuses from the host (a blocked or failing start, a console Ctrl-C) are host problems, as on unix. `orag serve` stops gracefully on Ctrl-C, Ctrl-Break, closing the console and (as a service) system shutdown; logoff is ignored.
- Process isolation is split into `isolate/unix.rs` and `isolate/windows.rs` behind one interface that keeps the rule "probe without reaping, kill the whole group, then reap".
- CI also runs for `step/**` branches (pushed before their merge, no pull request; D-020 rule 10). The parser test that sends stop signals now waits for the child to report that its signal dispositions are installed, instead of a fixed 0.5 s, which failed on a cold start of a freshly built binary.
- On Windows, backups (`orag backup`, and the copy taken before a schema upgrade) flush the finished copy through a writable handle, because Windows refuses to flush a read-only one; unix keeps the read handle.
- README: Windows 10 22H2 / 11 x86-64 with AVX2 in the system requirements.

## [0.2.0-alpha.14] - 2026-10-12

- Web page redesigned as an application: a sidebar with the collections (*+* creates one), the upload area and the selected collection's documents; a workspace with the collection's title bar (reindex, delete) and a conversation whose question cards stay on screen until another collection is selected. Answers show their cited sources, with the unused ones folded away. Files can be dropped anywhere on the page. The page's own confirmation dialogs replace the browser's, and results appear as notifications. Enter sends a question, Shift+Enter starts a new line. The sidebar footer shows the ORAG version and the two models; when ORAG is unreachable a banner says so and the page resumes by itself. Colours follow the icon (forest green, cream), with a dark theme that follows the system and a narrow-screen layout.
- *Nasıl kullanılır?*: a window that explains what ORAG is, that documents, questions and answers stay on this computer, the three steps (choose a collection, upload, ask and open the sources), tips and shortcuts. It opens by itself on the first visit and from the sidebar afterwards. No API change.
- The page's own requests are accepted when the browser marks them `Sec-Fetch-Site: same-origin`, whatever their `Origin` says. A browser that rewrote `Origin` to `http://127.0.0.1` (port dropped) on every request got `403` for the page's files. Other sites stay refused as before. Each refused request is now logged (`request refused`, with Host, Origin and `Sec-Fetch-Site`).

## [0.2.0-alpha.13] - 2026-10-10

- Plan for the rest of v0.2 (page provenance, layout warning, 10 MB default, setup mode and launcher, 200-question set, answer metrics, model decision, page completeness, release) and the Windows build and installer

## [0.2.0-alpha.12] - 2026-10-07

- Web page is easier to use: the collection selector sits at the top with its management (create, reindex, delete) folded under *Koleksiyonu yönet*; *Sorgu yap* comes first, then *Dosya yükle* and *Belgeler*. Questions are sent with Ctrl/⌘ + Enter, can be stopped (*Durdur*, the partial answer stays) and copied (*Kopyala*); an empty collection says to upload first. Files are dropped on the page or chosen, several at once, and each gets its own line that follows its indexing; the size limit is checked before sending. Errors are explained in Turkish (unreachable server, unsupported type, size, busy, reindex needed, duplicate name, ...) with the API message below. Long source excerpts start folded. Status badges, keyboard focus rings and a dark theme that follows the system.
- The page script is split into small ES modules (`app.js` imports the rest), all embedded and served by the UI listener. No API change.

## [0.2.0-alpha.11] - 2026-10-07

- Query answers carry `citation_markers`: every `[n]` marker in `answer` order with its place (`answer[start..end]`, UTF-8 byte offsets) and the source numbers it cites (a range expanded). Same rules as `citations` (markers in fenced code blocks are not citations; `[01]` and years are not markers). In the JSON response and the SSE `done` event; an added field, no breaking change.
- Web page: citations link to their sources. When an answer is complete, the digits of each marker, as the model wrote it, link to their source, which is highlighted and scrolled into view (no history entry); a number with no source is struck through. Sources the answer does not cite are dimmed and marked *yanıtta kullanılmadı*. The page takes the markers from the server instead of parsing the answer.

## [0.2.0-alpha.10] - 2026-10-07

- Reindex (D-022): `POST /v1/collections/{collection_id}/reindex` indexes every document of a collection again from the copy stored at upload, with the running embedding model and chunker; `202 {collection_id, queued_documents}`. This is the recovery for `409 reindex_required` without uploading files again; collection and document ids stay. Refused with `409 conflict` while a document of the collection is still queued or indexing, so a repeated call cannot undo finished work. Until the new jobs finish, queries find only the documents already done; a query that races the reindex gets no dense hits instead of a 404.
- Web page: *Yeniden indeksle* in the collection section (with a confirmation); a `reindex_required` error on upload or query now says to use it. Rows of documents still queued or indexing refresh by themselves until they finish.
- An ingest job checks every 0.5 s that it is still claimed and stops otherwise, so deleting a document while it is indexed (or a reindex) no longer lets the worker finish parsing and embedding it for nothing.

## [0.2.0-alpha.9] - 2026-10-07

- Web page: a *Belgeler* section lists the documents of the selected collection (file name, status, chunk count, size, warnings and errors), 50 at a time with *Daha fazla göster*, and deletes a document after a confirmation; the collection's document count updates. The list follows the selected collection and shows a new upload while it is indexed. No API change.

## [0.2.0-alpha.8] - 2026-10-07

- Web page: collections. A *Koleksiyon* section selects the collection to work in (with its document count; the choice is remembered in the browser), creates a new one and deletes the selected one after a confirmation (the `default` collection cannot be deleted). Uploads and questions use the selected collection only, so a question no longer searches documents uploaded for another topic; before, the page always used the default collection. No API change.

## [0.2.0-alpha.7] - 2026-10-07

- Built-in web page (D-021): `orag serve` also serves a small page on `ui_bind` (default `http://127.0.0.1:2442`, `"off"` disables it). *Dosya yükle* uploads a TXT/Markdown/DOCX/PDF file to the default collection and follows its indexing; *Sorgu yap* streams the answer, then shows the sources and an abstention notice. Plain HTML/CSS/JS embedded in the binary, `Content-Security-Policy: default-src 'self'`, nothing loaded from the network.
- New config key `ui_bind` (loopback only, bound before the models load; a busy port stops startup). `GET /v1/version` reports it with the real port. stdout gets a second line, `orag ui on http://...`, after the unchanged `orag listening on ...` line.
- The API now accepts exactly the page's own origins (`http://<ui address>`, `http://localhost:<ui port>`); every other origin still gets `403 forbidden_origin`. With `ui_bind = "off"` no origin is accepted, as before.

## [0.2.0-alpha.6] - 2026-10-06

- Project icon: a sickle (*orak*, the name's origin) on a cream rounded square. `assets/orag-icon.svg` is the source; `assets/orag-icon-512.png` and `assets/orag-icon-256.png` are renders of it for places that need a bitmap (GitHub social preview, organization avatar, the v0.3 desktop app), written by `scripts/render-icon.sh` (`--check` fails if they are stale). The README shows the icon at the top, and release tarballs include `assets/orag-icon.svg` so the packaged README shows it too.

## [0.2.0-alpha.5] - 2026-10-06

- `abstained` is also `true` when the model refuses in its own words: the first sentence says, without a citation, that the sources do not answer (`Verilen kaynaklarda ... belirtilmemiştir.`, `The sources do not mention ...`), even if cited background follows. Before, only the exact refusal sentence counted. On the answer sets: unanswerable questions flagged 4 of 4 (before 1 of 4), answerable ones 0 of 15. The answer text is unchanged.
- The refusal check needs a word for the sources in that first sentence (`kaynak`, `belge`, `source`, `document`), so a negative fact (`Anayasada ölüm cezası bulunmamaktadır.`) is an answer; a dot after a digit (`3. madde`) ends no sentence, and a citation right after the period belongs to it.
- `docs/api.md` response examples show the fingerprint of the fake models under encoding 2.

## [0.2.0-alpha.4] - 2026-10-06

- Query terms of three or more characters are FTS5 prefix terms, so a Turkish suffix no longer hides a match: `resmi dil` finds `resmî dili`, `mirasa` finds `miras`. Query side only (`LexicalQuery`), no reindex. Hybrid answer context: new inflection set 0.833 → 0.900, constitution set 0.950 → 1.000, seed set unchanged at 1.000 (MRR@10 0.939 → 0.903). Details in `.docs/benchmarks/2026-10-06-lexical-query.md`.
- Numbers are never prefix terms (`104` must not find `1040`); only terms with a letter are.
- Fixes from the review: the repeated-line guard strips one list marker followed by a space, so dotted dates and thousands (`01.02.2016 …`, `1.000 TL …`) no longer make three lines look alike; `orag eval answers` treats a dot between digits as part of the number (`600` is not found in `1.600`); an FTS index built by a newer release is refused instead of rebuilt down; seeded samplers are built per answer instead of cached per seed; `normalize::fts_query` delegates to `LexicalQuery::EXACT`.
- New retrieval set `eval/datasets/anayasa-cekim-tr.jsonl`: 30 questions asked in word forms the constitution text does not use.
- `orag eval retrieval` reports the lexical query mode and takes a hidden `--lexical-query exact|prefix:<n>` for experiments. Leaving Turkish question words out of the query was measured too: no change, not adopted.

## [0.2.0-alpha.3] - 2026-10-06

- The full-text (lexical) index is versioned separately from the embedding space. Schema 2 adds a `meta` table that records the lexical version; when it differs, `Store::open` rebuilds the FTS index from the stored chunks in one transaction (logged as `lexical index built`). A lexical change (normalizer, indexed text) no longer causes `409 reindex_required` or re-embedding.
- Embedding-space fingerprint encoding 2 no longer hashes the normalizer version. Spaces stored with an encoding-1 fingerprint of the same model and chunker (0.1.x, 0.2.0-alpha.1/2) are still accepted, so their vectors keep serving; fingerprints are not rewritten.
- Each chunk's FTS text is now the full heading path plus the body, both stored in `chunks` (before: the embedder's trimmed breadcrumb, which is not stored and could not be rebuilt). Lexical version 3.
- Lexical ranking breaks BM25 ties by chunk id, so the promoted lexical leader is deterministic.
- Fixes to 0.2.0-alpha.2 from its review: `orag eval answers` defaults to the served sampler (`dry`), not `greedy`; expected texts match on word boundaries (`600` is not found in `1600`); a probe set without answerable probes is an error; the samplers are built once per profile and reset between answers (DRY scans the vocabulary when built); the `length` check after the budget uses argmax and is skipped for a cancelled answer; the answer-engine tests moved to `retrieval/answer/tests.rs`.
- The startup lexical rebuild waits for another process's write lock like a migration (up to 300 s) instead of failing after the 5 s busy timeout.
- **Upgrade note:** schema 2 is applied at first start (with the usual automatic backup); the FTS index is then rebuilt once. Collections that 0.2.0-alpha.1/2 answered with `409 reindex_required` after the normalizer change serve again without re-uploading; checked on a copy of a 372-chunk database from 0.2.0-alpha.1. Older binaries refuse schema 2.

## [0.2.0-alpha.2] - 2026-10-06

- Answers end with a `finish_reason` (`stop`, `length`, `repetition`) in the JSON response and the SSE `done` event, so a client can tell a cut answer from a complete one; `docs/api.md` also states that `abstained` needs the exact refusal sentence.
- Repeated-line guard: an answer that writes the same line (list numbers and markers ignored, at least 12 characters) three times in a row is stopped with `finish_reason: repetition`. On an anonymized court decision the 4B model used to repeat one bullet until the 1024-token cap. A line that recurs between other lines (the same finding under several defendants, a table separator) never trips it. `length` is reported only when the answer would have gone on.
- Answers are sampled with DRY in front of argmax instead of plain greedy: still deterministic, no copy loops. Chosen with the new answer evaluation: on the court decision greedy covered 0.857 of the expected facts and looped once, DRY 1.000 without loops; on the constitution all profiles covered 1.000 (`.docs/benchmarks/2026-10-06-answer-sampler.md`).
- `orag eval answers --corpus DIR --dataset FILE --sampler greedy,dry,presence,qwen[:seed]`: runs the full answer path per sampler profile and reports expected-fact coverage, refusals, length and repetition stops, distinct-line ratio and latency. New probe sets `eval/datasets/answers-ceza-tr.jsonl` and `answers-anayasa-tr.jsonl` over PDF corpora (`eval/corpus/ceza`, `eval/corpus/anayasa-pdf`).
- `orag eval retrieval` and `orag eval answers` also index PDF and DOCX corpus files, through the production parsers: the loop above only appeared with `pdf_oxide` text, not with pypdf text of the same file.

## [0.2.0-alpha.1] - 2026-10-06

- Hybrid retrieval always puts the top hit of the lexical list and of the dense list first, before the rest in RRF order. Plain RRF dropped a chunk that only one retriever found, even at rank 1: on a PDF of the Turkish constitution, "Anayasaya göre resmî dili nedir?" had Article 3 at lexical #1 and dense #39 and the model answered that the sources did not say. Scores (`rank_score`) are unchanged, so they no longer always decrease along `sources`.
- Lexical normalizer v2: a circumflex on a, i, u is dropped (`resmî`, `millî`, `kâğıt`, `Ûmit` match `resmi`, `milli`, `kağıt`, `Umit`); other letters keep it (`fête`).
- New evaluation set `eval/datasets/anayasa-tr.jsonl` (20 answerable, 2 unanswerable Turkish questions) over `eval/corpus/anayasa/tr-anayasa.txt`, the text of the Turkish constitution. Hybrid, before → after: recall@5 0.800 → 0.900, recall@10 0.900 → 0.950, MRR@10 0.762 → 0.833, in answer context 0.900 → 0.950; the seed set is unchanged. Known miss: ana-001 (`resmi dil` vs `resmî dili`, no stemming).
- The `reindex_required` message and the API error table name the embedding space (model, chunker or normalizer), not only the model.
- **Upgrade note:** the normalizer version is part of the embedding-space fingerprint, so every collection indexed before this version answers `409 reindex_required`; pinning the previous model does not help. Re-upload the documents (see `docs/api.md`, `reindex_required`).

## [0.1.0] - 2026-10-05

- First usable release: offline RAG service for TXT, Markdown, DOCX and PDF with hybrid retrieval, grounded streamed answers with citations, evaluation harness
- Documentation: `README.md` (requirements, configuration, offline model install, run, backups, upgrades, limits, evaluation, versioning) and `docs/api.md` (every route, error code, limit, config key, the SSE event contract and `reindex_required` recovery in v0.1), with responses captured from the dev server.
- `.github/workflows/release.yml`: on `v*` tags or manual dispatch, builds macOS and Linux (ubuntu-22.04) release binaries, checks them (`check-llama-build.sh --release`, `check-binary-deps.sh --release-floor`) and uploads `orag-<version>-<target>.tar.gz` (the executable bit kept) with the licenses, `THIRD-PARTY-LICENSES.md` (`scripts/third-party-licenses.sh`: every crate built into the binary, without proc-macro crates, plus llama.cpp's vendored C/C++ libraries; a crate without a license file gets its authors and the standard text), README, CHANGELOG and API docs. It publishes nothing.
- `scripts/check-benchmark-gate.sh`: the D-003 gate needs a PASS row at 100k x 1024 (k 50, 50 queries, 250 ms target) on both references for the release candidate, the version before the docs-only release bump (here 0.1.0-alpha.31; `scripts/check-benchmark-gate.sh 0.1.0-alpha.31`). Results: macOS (M5 Max) p95 103.5 ms, Linux (GitHub runner, x86-64, 16 GB) p95 225.3 ms; the Linux margin is thin and the 16 GB/8-core references follow after v0.1.0.
- `.docs/benchmarks/v0.1-seed-eval.md`: the first D-007 data point (seed set, default models): hybrid recall@10 1.000, every label in the answer context; first token in about 0.2 s, then about 110–130 tokens/s decoding on an M5 Max. The seed set is too small to choose models; v0.2's 200-question set decides.
- The `vector-scale` workflow also prints the runner's CPU and memory to the log.

## [0.1.0-alpha.31] - 2026-10-05

- Split the 1 243-line `tests/api.rs` into one `api` test binary with modules (`support`, `system`, `documents`, `query`, `binary`, each with explicit imports). The same 43 tests and assertions; test ids now carry the module path (`query::query_returns_grounded_answer_json`), so `--exact` filters need it.
- Linux reference measurement recorded in `.docs/benchmarks/vector-scale.md`: GitHub ubuntu-22.04 runner (x86-64, 16 GB), 0.1.0-alpha.29, 100k x 1024 p95 152.2 ms, PASS.

## [0.1.0-alpha.30] - 2026-10-05

- `orag eval retrieval` ignores hidden corpus files (AppleDouble `._name` files macOS leaves on USB drives, dotfiles, also with non-UTF-8 names) and directories named like documents; a label that names a hidden file says so.
- Owner decision recorded under D-003: the v0.1.0 vector-scale gate runs on the development machine (macOS) and the `vector-scale` workflow runner (Linux), both on the release candidate; the 16 GB M-series laptop and an 8-core x86-64 box follow after v0.1.0, and release notes say where the promise was measured.

## [0.1.0-alpha.29] - 2026-10-05

- DOCX and PDF uploads (multipart only; JSON text with a .docx/.pdf name or format is 400 "upload as multipart"). The signature is checked before anything is stored, on a blocking thread: a PDF must start with `%PDF-`; a DOCX must be a ZIP of at most 10 000 entries whose `[Content_Types].xml` (UTF-8 or UTF-16) declares a Word main part, so a renamed .xlsx/.pptx/.odt, also one that embeds a .docx, is refused. One detection rule set (extension first, then content type) for multipart and JSON; other document types stay 415.
- Under `orag serve` every DOCX/PDF is parsed in a child `orag __parse` process: own process group, killed as a group while still unreaped (no pid-reuse race), 120 s deadline, 64 MiB result cap, stderr drained and logged (2 KiB), a watchdog that `_exit`s an orphaned child; on Linux a 4 GiB address-space cap, `MALLOC_ARENA_MAX=2`, `oom_score_adj` 1000, and the child is `/proc/self/exe`, so an in-place upgrade cannot swap the parser. A crash or hang fails that one document, never the service.
- Shutdown during a parse stops the child through the worker's own shutdown signal; the job stays running and resumes once after restart (tested end to end, including that shutdown is prompt). The child ignores SIGTERM/SIGINT/SIGHUP, so systemd's control-group kill or `pkill orag` cannot make a parse look like a crash. Failure attribution: crash signals blame the file, a parser panic is a rejected file, SIGKILL (OOM killer, operator) and error exits are host problems.
- `orag serve` and parser children share one `_exit` helper; text and Markdown parsing no longer copy the document for line endings; the binary end-to-end tests are split into `tests/cli.rs` and `tests/isolation.rs` with shared helpers.

## [0.1.0-alpha.28] - 2026-10-05

- DOCX parser (zip + roxmltree): headings from styles (with `w:basedOn` inheritance and Strict OOXML), paragraphs, list items (direct and style numbering, numId 0 = off), tables (one row per line, `|` escaped), text boxes once, tracked deletions skipped; the main part and styles are found through the package relationships. A broken styles part is a `styles_unreadable` warning, not a failure.
- PDF parser (pdf_oxide =0.3.78, no `ort`): page text as paragraphs, words hyphenated at line ends joined (a one-letter prefix such as "e-fatura" keeps its hyphen), `ocr_required` for pages without a text layer, `extraction_failed` for unreadable pages; no pages is an error.
- Hostile input is bounded before it can hurt the process: per-part decompressed size (also when the ZIP header lies), XML nesting depth checked by a non-recursive scan (deep XML used to overflow the stack and abort), XML node count, namespace declarations (thousands made parsing quadratic), PDF page count, extracted text and a time limit between pages. Part names in errors are quoted and shortened.
- Nothing calls the parsers yet: DOCX/PDF uploads open in 23b together with isolated parsing, signature checks and the JSON rules. Test fixtures come from `scripts/make-fixtures.sh` (pandoc, cupsfilter).

## [0.1.0-alpha.27] - 2026-10-04

- Embedding release gate, after an external review: the bilingual check uses the sections the chunker emits; the plan's one-sentence probe (which the 0.6B model fails: English question 0.380 for the Turkish policy vs 0.393 for an unrelated English sentence) stays as a diagnostic that the gate runs separately and never fails on; a new dense check requires each cross-language seed question's evidence within rank 3 among topic-near distractors in both languages. The limitation is recorded under D-005.
- `orag eval retrieval` reports `in context (top 8)` per strategy and the ids that miss it; `--require-in-context ID,...` fails when hybrid leaves any of them out. The release gate requires the three cross-language seed questions, so one subgroup cannot hide in the mean recall.
- Checked on Metal and on CPU (Linux arm64 in Docker): all real-model tests pass; cosines differ by at most ~0.007 and one dense rank swaps, so no gate requires a top-1 rank.
- `.github/workflows/vector-scale.yml`: manual job that runs the D-003 benchmark on GitHub's ubuntu-22.04 runner as the Linux reference; it refuses to run unless the runner is x86-64 with at least 15 GB, is limited to 60 minutes, and writes the machine and results to the job summary.
- **Versions:** this fix takes alpha.27, so 23a/23b produce alpha.28/29 (Task 24 is still 0.1.0). This supersedes the alpha.23 entry's numbering.

## [0.1.0-alpha.26] - 2026-10-04

- `scripts/fetch-model-pack.sh <preset> <dir>` builds offline packs for qwen3-embedding-0.6b-q8_0, qwen3.5-4b-q4_k_m and qwen3-4b-instruct-2507-q4_k_m from pinned Hugging Face revisions. Model and LICENSE are both verified by SHA-256 via a .part file that is deleted on mismatch, and the manifest is written last, so an unfinished pack has none.
- `scripts/release-model-check.sh` (release gate) builds the release binary, checks its dependencies, requires all 3 real-model tests to run and pass with the shipped generation model (an ORAG_TEST_GENERATION_MODEL override is ignored), and fails when seed hybrid recall@10 is below 0.9 (new `orag eval retrieval --min-recall-at-10`). It cleans up on failure and can be run again.
- `scripts/check-binary-deps.sh <binary> [--release-floor]` allows only system libraries, plus a glibc/libstdc++ floor (Linux) or macOS 14 (minos); finding no GLIBC versions is an error. CI now checks the debug and the release binary on both images.
- Fix: a process that loaded a real model aborted at exit on macOS (llama.cpp Metal `GGML_ASSERT([rsets->data count] == 0)`), because the embedder's and generator's worker threads were never joined. Dropping them now closes the queue and waits for the worker (a worker panic is logged), a generation stops at its next step however its caller ends, and if a task that did not stop in time still holds a model, `orag serve` exits with `_exit` instead of running llama.cpp's teardown.
- Verified on Apple Silicon with the real packs: release gate passed (seed eval: dense recall@10 1.000, hybrid recall@10 1.000), both generation models answer in Turkish with citations and abstain when unsupported, and `orag serve` answered and exited cleanly. Qwen3.5-4B stays the default (no D-007 fallback needed).

## [0.1.0-alpha.25] - 2026-10-04

- `orag eval vector-scale [--chunks 100000] [--dimensions 1024] [--queries 50] [--work-dir DIR]`: synthetic dense-search latency benchmark for the D-003 gate (warm p95 < 250 ms at 100k × 1024); exits non-zero when the gate fails and never touches ORAG_HOME.
- The benchmark cannot pass by measuring nothing: every search must return k hits, sizes are bounded (1-1 000 000 chunks, 20-10 000 queries), an all-zero random vector is drawn again, "insert s" counts SQLite inserts only, and --work-dir puts the throwaway database on the disk that is measured (Linux /tmp can be tmpfs).
- Recorded in .docs/benchmarks/vector-scale.md: 100k × 1024 p95 99.9 ms (PASS) and 300k p95 294.0 ms (informational) on an Apple M5 Max; the two reference machines (M-series 16 GB, x86-64 Linux 16 GB) are still to be measured before v0.1.0.
- Release builds link on macOS again: thin LTO dropped Rust's `__isPlatformVersionAtLeast`, which llama.cpp's Metal code needs, so build.rs links clang's runtime archive explicitly (no new dynamic dependency) and reruns when the toolchain moves; CI now builds the release profile on macOS.

## [0.1.0-alpha.24] - 2026-10-04

- `orag eval retrieval --corpus DIR --dataset FILE [--out FILE]`: indexes a corpus into a throwaway database and compares lexical, dense and hybrid retrieval (recall@5/10, MRR@10, label-based nDCG@10, p50/p95 latency); Markdown to stdout, JSON to a new --out file. It never touches ORAG_HOME's database, and with --dev-fake-models it reads no config.
- Bilingual seed corpus (4 documents) and dataset (16 questions: Turkish, English, cross-language, accentless, unanswerable) with eval/README.md for the 200-question target set.
- Mistakes are errors, not zero scores: every label must match a chunk of its document (parsed text, no Markdown syntax), and an empty corpus, a failed or empty file, or two identical files stop the run with the file name; .md/.markdown/.txt are matched in any case. The dataset and --out are checked before the model loads.
- eval loads the same embedder as serve (app::load_embedder); tempfile is now a runtime dependency.

## [0.1.0-alpha.23] - 2026-10-04

- Cancellation tests wait for events instead of fixed times: they wait for the generation permit to come back (it is released after the last token is counted), and the abandoned JSON query is dropped mid-generation. A macOS CI runner had failed a follow-up query that waited for a whole slow answer; the follow-up now only checks that a new stream starts.
- **Versions:** this out-of-band fix takes alpha.23, so Tasks 20-22 produce alpha.24-26 and 23a/23b produce alpha.27/28 (Task 24 is still 0.1.0). This supersedes the alpha.3 entry's numbering.

## [0.1.0-alpha.22] - 2026-10-03

- `orag serve`: config.toml is read once at startup; prints exactly one stdout line `orag listening on http://<addr>` (bind = "127.0.0.1:0" picks a free port, reported by /v1/version); an exclusive orag.lock refuses a second instance on the same home; interrupted jobs resume.
- The port is bound before the models load, so a busy port fails at once; SIGINT/SIGTERM handlers are installed before the listening line; the HTTP drain (10 s, now inside server::serve) and the worker/blocking tasks (5 s) are bounded, so shutdown cannot hang.
- `orag backup <DEST>`: a read-only, consistent copy that is safe while the server runs; a missing database (for example a mistyped ORAG_HOME) is an error and nothing is created, the live database is never migrated, and an existing DEST is never overwritten.

## [0.1.0-alpha.21] - 2026-10-03

- POST /v1/collections/{id}/query: JSON answer or SSE stream (sources, tokens, then done or one error event), with sources, validated citations, abstention and a per-query trace.
- One generation slot: a query waits at most 120 s, then gets 429 busy with Retry-After; queued and running queries get 503 shutting_down, never a truncated 200.
- A disconnected client (JSON or SSE) stops generation and frees the slot; a client that stops reading loses its stream after 30 s.
- Unknown collection and embedding-model mismatch are checked before and again after the wait, so they stay 404/409 statuses instead of a 200 stream with an error event.
- A prompt the generator refuses after budgeting is reported as a model error, not as the user's invalid input; HTTP and engine share one question validation.

## [0.1.0-alpha.20] - 2026-10-03

- Collections (list, create, delete), document upload (JSON text or multipart file), cursor-paginated listing, get/delete, and job status endpoints.
- Uploads: content type, collection and model compatibility are checked before one of 4 upload slots is taken; the body has 60 s; a multipart file is streamed and stopped as soon as it passes max_document_mb.
- One set of JSON-text format rules (SourceFormat::detect_text) with whitespace-trimmed extensions, so padded names and dotfiles such as 'rapor.xlsx ' or '.html' are refused like any other unsupported type.
- Bad path and query values are JSON invalid_input errors (ApiPath/ApiQuery); an unsupported JSON media type is 415 unsupported_media_type.

## [0.1.0-alpha.19] - 2026-10-03

- HTTP server foundation: loopback only, no authentication (D-013); Host must be a loopback name with an optional digits-only port; Origin is checked against allowed_origins after normalization, and cross-site browser requests without an Origin are refused.
- JSON errors everywhere, including unknown routes (not_found), wrong methods (method_not_allowed) and a missing JSON content type (415 unsupported_media_type); busy() adds Retry-After.
- GET /v1/health and GET /v1/version (with schema version and the effective config). serve() starts the app's own shutdown, so requests waiting for a slot or a body get 503 shutting_down.

## [0.1.0-alpha.18] - 2026-10-03

- Hybrid retrieval (lexical, dense, RRF) with a per-query trace; lexical search keeps working after an embedding model change.
- Answer engine: token-budgeted context (two candidates per slot, skipped chunks counted in the trace), streamed sources, tokens and a summary with validated [n] citations; empty retrieval abstains without calling the model.
- Source text is quoted line by line (every line-break character) so it cannot forge a source header or the question; a refusal is detected in either language only when the answer is the refusal itself.
- A question too long for the embedding or answer model is InvalidInput; a consumer Break is final, so no Done follows.

## [0.1.0-alpha.17] - 2026-10-03

- Ingestion worker: parse, chunk and embed outside the write lock, then publish atomically; restart recovery requeues interrupted jobs.
- A document deleted at any point while its job runs is discarded, not failed; a reindex-required collection fails the job before any embedding work.
- A panicking job is failed instead of left running; shutdown stops a running job between embedding batches and leaves it for the next start; the worker also stops if the shutdown sender is dropped.
- The chunker config is validated against the embedding model at startup (IngestContext::check) and kept unchanged when the model has room; user-facing job errors never leak internal details.

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
