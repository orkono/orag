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

The models are not in these files; the two default packs need about 3.3 GB
more (see *First start*).

## First start

The app starts the service in **setup mode** until the two default models are
installed, and its page asks for a model pack folder (*Modelleri kur*). Build
packs on a connected machine with `scripts/fetch-model-pack.sh` (bash) or
`fetch-model-pack.ps1` (PowerShell) and type the folder's full path into the
page. After the import the app restarts the service by itself and the page
reloads with the models loaded. The app makes no network request of its own;
an in-app downloader is planned for v0.4 (D-012).

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
