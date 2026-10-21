//! Which service executable the app runs: `orag-vulkan` on a usable GPU,
//! else `orag` (v0.6 plan, G-3). Decided on every start, in order:
//! 1. this session already moved to the CPU (sticky until a retry);
//! 2. `gpu = "off"` in the home's `config.toml`;
//! 3. a fallback marker in the home (`gpu-fallback.json`) from this app
//!    version, younger than [`MARKER_MAX_AGE`];
//! 4. `orag-vulkan gpu-probe` (killed after [`PROBE_TIMEOUT`]): a chosen
//!    device starts `orag-vulkan` on it, anything else the CPU.
//!
//! A Vulkan sidecar that never becomes ready, or crashes a second time in
//! the session, moves the session to the CPU and writes the marker. Exit 75
//! restarts the same executable; exit 76 (the page's GPU retry) deletes the
//! marker and clears the session.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::supervisor::{Backends, Ended, SidecarSpec, Verdict};

pub const MARKER_FILE: &str = "gpu-fallback.json";
pub const MARKER_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(10);
/// The service's "try the GPU again" exit code (`orag::server`, step 87).
pub const EXIT_GPU_RETRY: i32 = 76;
/// Set on every sidecar the app starts, so the service accepts the retry.
pub const GPU_RETRY_ENV: &str = "ORAG_GPU_RETRY";
pub const GPU_DEVICE_ENV: &str = "ORAG_GPU_DEVICE";
pub const FALLBACK_REASON_ENV: &str = "ORAG_GPU_FALLBACK_REASON";
/// The process could not load a DLL (`STATUS_DLL_NOT_FOUND`, Windows).
const STATUS_DLL_NOT_FOUND: i32 = 0xC000_0135_u32 as i32;
/// The dynamic loader could not load a library (glibc's `ld.so`).
const LD_SO_FAILED: i32 = 127;

/// Why the app runs on the CPU after a fallback; shown on the page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Marker {
    pub version: String,
    pub reason: String,
    /// Seconds since the Unix epoch.
    pub at: u64,
}

impl Marker {
    fn holds(&self, version: &str, now: u64) -> bool {
        self.version == version && now.saturating_sub(self.at) < MARKER_MAX_AGE.as_secs()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuSetting {
    Auto,
    Off,
}

/// What `orag-vulkan gpu-probe` said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeOutcome {
    /// The device the service would use (llama.cpp's index).
    Chosen(usize),
    /// It ran, and found no device ORAG would use.
    NoDevice,
    /// It did not run to the end, for this reason.
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Cpu,
    Vulkan { device: usize },
}

/// Steps 2-4 of the module documentation. The probe runs only when the
/// setting and the marker leave the choice open. Returns the backend, the
/// reason for the CPU (None when the CPU is the user's choice), and whether
/// the reason is new (to be written as the marker).
pub fn choose_backend(
    setting: GpuSetting,
    marker: Option<&Marker>,
    version: &str,
    now: u64,
    probe: impl FnOnce() -> ProbeOutcome,
) -> (Backend, Option<String>, bool) {
    if setting == GpuSetting::Off {
        return (Backend::Cpu, None, false);
    }
    if let Some(marker) = marker.filter(|m| m.holds(version, now)) {
        return (Backend::Cpu, Some(marker.reason.clone()), false);
    }
    match probe() {
        ProbeOutcome::Chosen(device) => (Backend::Vulkan { device }, None, false),
        ProbeOutcome::NoDevice => (Backend::Cpu, Some("no-eligible-device".into()), true),
        ProbeOutcome::Failed(reason) => (Backend::Cpu, Some(reason), true),
    }
}

/// The reason a GPU sidecar or probe that exited with `code` gives.
pub fn exit_reason(code: Option<i32>) -> String {
    match code {
        Some(STATUS_DLL_NOT_FOUND) | Some(LD_SO_FAILED) => "loader-missing".into(),
        Some(code) => format!("exit-{code}"),
        None => "killed".into(),
    }
}

pub fn read_marker(home: &Path) -> Option<Marker> {
    let text = std::fs::read_to_string(home.join(MARKER_FILE)).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn write_marker(home: &Path, marker: &Marker) -> std::io::Result<()> {
    let text = serde_json::to_string(marker).map_err(std::io::Error::other)?;
    let tmp = home.join(format!("{MARKER_FILE}.tmp"));
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, home.join(MARKER_FILE))
}

/// Deletes the marker; a marker that is not there is not an error.
pub fn delete_marker(home: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(home.join(MARKER_FILE)) {
        Err(err) if err.kind() != std::io::ErrorKind::NotFound => Err(err),
        _ => Ok(()),
    }
}

/// `gpu` in the home's `config.toml`; anything but `"off"` (a missing file,
/// a file the service will reject) leaves the choice to the probe.
pub fn read_setting(home: &Path) -> GpuSetting {
    let off = std::fs::read_to_string(home.join("config.toml"))
        .ok()
        .and_then(|text| text.parse::<toml::Table>().ok())
        .is_some_and(|table| table.get("gpu").and_then(|v| v.as_str()) == Some("off"));
    if off {
        GpuSetting::Off
    } else {
        GpuSetting::Auto
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Runs `orag-vulkan gpu-probe` with `spec`'s environment, killing and
/// reaping it after `timeout` or as soon as `cancelled` turns true.
pub fn run_probe(
    spec: &SidecarSpec,
    timeout: Duration,
    cancelled: &dyn Fn() -> bool,
) -> ProbeOutcome {
    use std::io::Read;
    use std::process::{Command, Stdio};
    let mut command = Command::new(&spec.program);
    command
        .arg("gpu-probe")
        .envs(spec.env.iter().map(|(k, v)| (k, v)))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = match crate::contain::spawn(&mut command) {
        Ok(child) => child,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return ProbeOutcome::Failed("orag-vulkan-missing".into());
        }
        Err(_) => return ProbeOutcome::Failed("probe-not-started".into()),
    };
    let mut stdout = child.stdout.take();
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        if let Some(out) = stdout.as_mut() {
            let _ = out.read_to_string(&mut text);
        }
        text
    });
    let deadline = std::time::Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if cancelled() => break Err("probe-cancelled"),
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => break Err("probe-timeout"),
        }
    };
    let status = match status {
        Ok(status) => status,
        Err(reason) => {
            let _ = child.kill();
            let _ = child.wait();
            return ProbeOutcome::Failed(reason.into());
        }
    };
    let text = reader.join().unwrap_or_default();
    if !status.success() {
        return ProbeOutcome::Failed(exit_reason(status.code()));
    }
    let chosen = text
        .lines()
        .rev()
        .find_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .map(|json| json["chosen"].as_u64());
    match chosen {
        Some(Some(index)) => ProbeOutcome::Chosen(index as usize),
        Some(None) => ProbeOutcome::NoDevice,
        None => ProbeOutcome::Failed("probe-unreadable".into()),
    }
}

/// Runs the probe for a spec; the second argument says the app is closing.
type Probe = dyn FnMut(&SidecarSpec, &dyn Fn() -> bool) -> ProbeOutcome + Send;

/// The policy the supervisor runs on Windows and Linux bundles.
pub struct GpuBackends {
    cpu: SidecarSpec,
    vulkan: SidecarSpec,
    home: PathBuf,
    version: String,
    probe: Box<Probe>,
    /// Why this session runs on the CPU after a fallback; None otherwise.
    session: Option<String>,
    /// After a retry the marker is not read again this session (it may
    /// not have been deletable, or another start wrote it meanwhile).
    ignore_marker: bool,
    vulkan_crashes: u32,
    current: Backend,
}

impl GpuBackends {
    /// `cpu` and `vulkan` are the same `serve` spec for `orag` and
    /// `orag-vulkan`; `home` is the sidecar's `ORAG_HOME`.
    pub fn new(cpu: SidecarSpec, vulkan: SidecarSpec, home: PathBuf) -> GpuBackends {
        Self::with_probe(
            cpu,
            vulkan,
            home,
            env!("CARGO_PKG_VERSION").into(),
            |spec, cancelled| run_probe(spec, PROBE_TIMEOUT, cancelled),
        )
    }

    pub fn with_probe(
        cpu: SidecarSpec,
        vulkan: SidecarSpec,
        home: PathBuf,
        version: String,
        probe: impl FnMut(&SidecarSpec, &dyn Fn() -> bool) -> ProbeOutcome + Send + 'static,
    ) -> GpuBackends {
        GpuBackends {
            cpu: with_env(cpu, GPU_RETRY_ENV, "1"),
            vulkan: with_env(vulkan, GPU_RETRY_ENV, "1"),
            home,
            version,
            probe: Box::new(probe),
            session: None,
            ignore_marker: false,
            vulkan_crashes: 0,
            current: Backend::Cpu,
        }
    }

    fn cpu_spec(&self, reason: Option<&str>) -> SidecarSpec {
        match reason {
            Some(reason) => with_env(self.cpu.clone(), FALLBACK_REASON_ENV, reason),
            None => self.cpu.clone(),
        }
    }

    /// Moves the session to the CPU; the marker carries the reason across
    /// app starts, the session keeps it even when the marker cannot be written.
    fn fall_back(&mut self, reason: String) -> String {
        let marker = Marker {
            version: self.version.clone(),
            reason: reason.clone(),
            at: now_secs(),
        };
        if let Err(err) = write_marker(&self.home, &marker) {
            eprintln!("orag-desktop: could not write the GPU fallback marker: {err}");
        }
        self.session = Some(reason.clone());
        reason
    }
}

fn with_env(mut spec: SidecarSpec, key: &str, value: &str) -> SidecarSpec {
    spec.env.retain(|(k, _)| k != key);
    spec.env.push((key.into(), value.into()));
    spec
}

impl Backends for GpuBackends {
    fn next(&mut self, cancelled: &dyn Fn() -> bool) -> Option<SidecarSpec> {
        if let Some(reason) = self.session.clone() {
            self.current = Backend::Cpu;
            return Some(self.cpu_spec(Some(&reason)));
        }
        let marker = if self.ignore_marker {
            None
        } else {
            read_marker(&self.home)
        };
        let vulkan = self.vulkan.clone();
        let probe = &mut self.probe;
        let (backend, reason, new) = choose_backend(
            read_setting(&self.home),
            marker.as_ref(),
            &self.version,
            now_secs(),
            || probe(&vulkan, cancelled),
        );
        // Closing while the probe ran: no start, and no reason to keep.
        if cancelled() {
            return None;
        }
        self.current = backend;
        Some(match backend {
            Backend::Vulkan { device } => {
                with_env(self.vulkan.clone(), GPU_DEVICE_ENV, &device.to_string())
            }
            Backend::Cpu => {
                let reason = match reason {
                    Some(reason) if new => Some(self.fall_back(reason)),
                    Some(reason) => {
                        self.session = Some(reason.clone());
                        Some(reason)
                    }
                    None => None,
                };
                self.cpu_spec(reason.as_deref())
            }
        })
    }

    fn ended(&mut self, ended: &Ended) -> Verdict {
        let code = match ended {
            Ended::Exited(code) | Ended::NotReady { code, .. } => *code,
        };
        if code == Some(EXIT_GPU_RETRY) {
            if let Err(err) = delete_marker(&self.home) {
                eprintln!("orag-desktop: could not delete the GPU fallback marker: {err}");
            }
            self.ignore_marker = true;
            self.session = None;
            self.vulkan_crashes = 0;
            return Verdict::Fresh;
        }
        // A model import's restart, even before Ready: never a GPU failure.
        if code == Some(crate::supervisor::EXIT_RESTART) {
            return Verdict::Default;
        }
        if self.current == Backend::Cpu {
            return Verdict::Default;
        }
        match ended {
            Ended::NotReady {
                timed_out: true, ..
            } => Verdict::Fallback(self.fall_back("start-timeout".into())),
            Ended::NotReady { code: None, .. } => {
                Verdict::Fallback(self.fall_back("start-failed".into()))
            }
            Ended::NotReady { code, .. } => Verdict::Fallback(self.fall_back(exit_reason(*code))),

            Ended::Exited(_) => {
                self.vulkan_crashes += 1;
                if self.vulkan_crashes >= 2 {
                    Verdict::Fallback(self.fall_back("crashed-twice".into()))
                } else {
                    Verdict::Default
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
