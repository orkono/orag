# ORAG desktop app

The desktop app (v0.3, D-016) runs the ORAG service for you and shows its
page in its own window, so there is no terminal and no browser to open. It is
the same service and the same page as `orag start`; only the window and the
data folder are its own.

## Install

| System | File in the release (size) | Notes |
|---|---|---|
| macOS 14+ (Apple Silicon) | `ORAG_<version>_aarch64.dmg` (10 MB) | Drag **ORAG** to Applications. The app is not signed or notarized yet: the first time, right-click it, choose **Open**, then **Open** again (or *System Settings → Privacy & Security → Open Anyway*). |
| Windows 10 22H2 / 11 x86-64 (AVX2) | `ORAG_<version>_x64-setup.exe` (215 MB, most of it the offline WebView2 installer) | Per user, no administrator rights. It installs WebView2 if Windows lacks it (offline, included). Not code-signed yet: SmartScreen may ask once (*More info → Run anyway*). |
| Linux x86-64 (glibc 2.35+) | `ORAG_<version>_amd64.AppImage` (85 MB) or `.deb` (11 MB) | The AppImage runs after `chmod +x`; the `.deb` installs with `sudo apt install ./ORAG_<version>_amd64.deb`. Needs WebKitGTK 4.1 (present on Ubuntu 22.04+ and Debian 12+). |

The models are not in these files; the two default packs need about 3.1 GB
more (see *First start*). Every release also has `SHA256SUMS` and GitHub build
provenance to check a download with; see `docs/signing.md`.

## Graphics card (Windows and Linux)

From 0.6 the Windows and Linux apps carry two copies of the service: `orag`
(CPU) and `orag-vulkan` (llama.cpp's Vulkan backend, for NVIDIA, AMD and
Intel graphics cards). On every start the app checks for a discrete graphics
card with at least 2 GiB free memory (`orag-vulkan gpu-probe`, at most 10 s)
and, when there is one, runs `orag-vulkan` on it; the models are fitted to
the card's free memory, and what does not fit stays on the CPU. Without a
usable card, without a Vulkan driver, or when `gpu = "off"` is in the data
folder's `config.toml`, it runs `orag` exactly as before. Integrated graphics
are not used yet.

If `orag-vulkan` cannot start, does not become ready, or stops twice in one
session, the app switches to `orag` for the rest of the session and writes
`gpu-fallback.json` (version, reason, time) into the service's data folder;
later starts of the same version use the CPU straight away for 7 days, then
try the card again. Reasons: `loader-missing` (no Vulkan driver),
`no-eligible-device`, `probe-timeout`, `start-timeout`, `start-failed`,
`crashed-twice`, or the probe's exit code. Deleting the file makes the next
start try the card. On Windows a missing `vulkan-1.dll` shows no system
error window; the app just uses the CPU.

### Graphics card bench (pre-releases)

Pre-releases of the Windows and Linux apps have a link at the bottom of the
help panel, **Ekran kartı ölçümü / Graphics card bench**, which opens a page
with one button. The bench stops the service, then answers the same 10
questions on `orag` and on `orag-vulkan` (three cold starts each, per
installed model size), checks every graphics card answer against the
processor's, and compares retrieval across indexes built on either. It can
take 30 to 60 minutes; leave the computer alone, plugged in, with sleep off.
The app shows the progress and a **Durdur / Stop** button. At the end it
writes `orag-bench-<time>.json` into the app data folder and opens that
folder; **Devam / Continue** starts the service again on the same processor
or graphics card choice as before. The bench sets `gpu = "off"` aside for
its own runs only; it never touches `gpu-fallback.json`.

## First start

The app starts the service in **setup mode** until the two models are
installed. Its page shows **Bu bilgisayar** (memory, processor, cores, GPU,
free disk) and the answer models that fit this computer: *Standart*
(`qwen3.5-4b-q4_k_m`, about 3.1 GB with the embedder; at least 7 GiB RAM,
8 GiB recommended) and *Büyük* (`qwen3.5-9b-q4_k_m`, about 5.9 GB; at least
11 GiB, 12 GiB recommended; slower and uses more memory). Below the
recommended memory a model can still be picked, with a warning that answers
may be slow. Without an Apple GPU the page shows the median answer time
measured on a 4-vCPU GitHub test machine (about 108 s for *Standart*); a
given PC can be faster or slower. *Standart* is the recommended one whenever it fits; a model set by
hand in `config.toml` is listed first as *Şu anki*. One that does not fit is greyed out with the reason; a computer
nothing fits is told so. Pick one and press **Modelleri indir**: the two
models download and are checked, the choice is written as `generation_model`
into the data folder's `config.toml`, and the app restarts the service by
itself; the page reloads with the models loaded. The download
runs only when you press the button. It fetches only the pinned Hugging Face
revisions that `scripts/fetch-model-pack.sh` uses, checks every file by
SHA-256, honours `HTTPS_PROXY`, `HTTP_PROXY` and `NO_PROXY`, and continues
where it stopped if the connection breaks or you cancel it (D-012).

Without internet, use the offline way: build packs on a connected machine
with `scripts/fetch-model-pack.sh` (bash) or `fetch-model-pack.ps1`
(PowerShell), copy them over, and type the folder's full path into
*Modelleri kur*. To turn the download off for a fully offline install, add
`models.download = false` to the app's `config.toml` (see below). The app
itself makes no network request; besides the download you ask for, the
service makes none either (no telemetry, no update check).

## Where the data is

The app uses its own ORAG home, separate from `~/.orag` that `orag start`
and `orag serve` use, so the two never share a database:

| System | Folder |
|---|---|
| macOS | `~/Library/Application Support/com.orkono.orag/orag` |
| Windows | `%APPDATA%\com.orkono.orag\orag` |
| Linux | `~/.local/share/com.orkono.orag/orag` |

`config.toml` there is written on the first start with both ports on
`127.0.0.1:0` (a free port each start) and is never overwritten afterwards.
Uninstalling the app keeps this folder; delete it to remove your documents
and models.

## Behaviour

- One window per user: opening the app again brings the existing window to
  the front.
- The service listens on loopback only (D-013). Links that leave the ORAG page
  open in your browser; the page itself cannot call into the app.
- If the service stops, the app starts it again; after 3 stops within 60 s
  it shows the last lines the service printed, so they can go into a bug
  report.
- Closing the window stops the service. If the app is killed or crashes, the
  service stops too (on Windows through a kill-on-close Job, on Linux through
  a parent-death signal, everywhere through the closed pipe it reads).
