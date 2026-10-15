//! Spawning the sidecar so that it cannot outlive the app (D-016).
//!
//! A clean exit closes the sidecar's stdin (`--exit-on-stdin-close`), which
//! also happens when the app dies on macOS, since the OS closes its pipes.
//! The crash fallbacks:
//! - Windows: the app puts itself in a kill-on-close Job before its first
//!   spawn, so every child is in the Job from creation; when the app dies
//!   its handle closes and Windows ends the children. (`std::process` cannot
//!   pass `PROC_THREAD_ATTRIBUTE_JOB_LIST`; this gives the same guarantee.)
//! - Linux: the child asks for SIGTERM when its parent thread dies
//!   (`PR_SET_PDEATHSIG`) and exits at once if the parent is already gone.

use std::io;
use std::process::{Child, Command};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(windows)]
mod windows;

/// Spawns `command` contained as the module documentation describes.
pub fn spawn(command: &mut Command) -> io::Result<Child> {
    #[cfg(windows)]
    if let Err(err) = windows::enter_app_job() {
        // Not fatal: the stdin-close stop still ends the sidecar with the app.
        eprintln!("orag-desktop: no kill-on-close Job for the sidecar: {err}");
    }
    #[cfg(target_os = "linux")]
    linux::die_with_parent(command);
    command.spawn()
}
