//! Unix side of parser isolation: the child leads its own process group,
//! which is killed as a whole while the child is still unreaped.

use std::io;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, ExitStatus};

use crate::error::{OragError, Result};

/// A parser child and everything it may start.
pub(super) struct Contained {
    child: Child,
}

/// Starts `command` as the leader of its own process group, so a terminal
/// Ctrl-C reaches `orag serve`, which then stops the child itself and
/// reports `Interrupted` instead of a crash.
pub(super) fn spawn(command: &mut Command) -> io::Result<Contained> {
    command.process_group(0);
    // Few malloc arenas keep address-space use (RLIMIT_AS) proportional to real use.
    command.env("MALLOC_ARENA_MAX", "2");
    Ok(Contained {
        child: command.spawn()?,
    })
}

impl Contained {
    pub(super) fn take_pipes(
        &mut self,
    ) -> (Option<ChildStdin>, Option<ChildStdout>, Option<ChildStderr>) {
        (
            self.child.stdin.take(),
            self.child.stdout.take(),
            self.child.stderr.take(),
        )
    }

    /// True once the child has exited, without reaping it (`WNOWAIT`).
    pub(super) fn exited(&mut self) -> io::Result<bool> {
        let pid = libc::id_t::from(self.child.id());
        // SAFETY: `info` is a zeroed, writable siginfo_t; waitid only fills it in.
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        let flags = libc::WEXITED | libc::WNOHANG | libc::WNOWAIT;
        // SAFETY: valid id type and pointer; WNOWAIT leaves the child waitable.
        if unsafe { libc::waitid(libc::P_PID, pid, &mut info, flags) } != 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: after a successful waitid, si_pid is set (0 if nothing changed).
        Ok(unsafe { info.si_pid() } != 0)
    }

    /// SIGKILL to the child's process group; the child alone if it leads
    /// none. Called while the child is unreaped, so the group id is still ours.
    pub(super) fn kill_tree(&mut self) {
        let group_killed = libc::pid_t::try_from(self.child.id()).is_ok_and(|pid| {
            // SAFETY: killpg only sends a signal; the group id is our unreaped
            // child's own pid, so no other process group can be affected.
            unsafe { libc::killpg(pid, libc::SIGKILL) == 0 }
        });
        if !group_killed {
            let _ = self.child.kill();
        }
    }

    /// Reaps the child; call after `kill_tree`.
    pub(super) fn wait(&mut self) -> io::Result<ExitStatus> {
        self.child.wait()
    }
}

/// Why a child that ended without success failed:
/// - a crash signal (stack overflow, abort, the memory cap): the file;
/// - SIGKILL (OOM killer, an operator, a cgroup limit) or a stop signal the
///   child ignores (SIGTERM/INT/HUP) still killing it: something outside orag;
/// - an exit code: `run_child` reports a bad file (also a parser panic) as a
///   `Rejected` result, so an error exit is a host problem.
pub(super) fn classify(status: ExitStatus) -> OragError {
    if let Some(signal) = status.signal() {
        let outside = [libc::SIGKILL, libc::SIGTERM, libc::SIGINT, libc::SIGHUP];
        if outside.contains(&signal) {
            return OragError::Internal(format!(
                "the parser process was killed from outside orag (signal {signal}; \
                 the host may be out of memory)"
            ));
        }
        return super::file_fault();
    }
    OragError::Internal(format!("the parser process failed ({status})"))
}

/// The parent stops the child itself (SIGKILL to its process group) and the
/// watchdog ends an orphan, so stop signals sent to every process at once
/// (systemd's control-group kill, `pkill orag`) must not end a parse early:
/// that would look like a crash. The parent then reports `Interrupted`.
pub(super) fn ignore_stop_signals() {
    for signal in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
        // SAFETY: setting a standard signal's disposition to SIG_IGN has no
        // preconditions and installs no handler code.
        unsafe {
            libc::signal(signal, libc::SIG_IGN);
        }
    }
}

/// Returns a check that turns true once the parent process is gone.
pub(super) fn orphan_check() -> impl Fn() -> bool + Send + 'static {
    let parent = std::os::unix::process::parent_id();
    move || std::os::unix::process::parent_id() != parent
}

/// Linux: cap the child's address space and make it the OOM killer's first
/// choice, so a decompression bomb cannot take down `orag serve` (which holds
/// the models). Other unix platforms rely on the parse deadline.
/// Fails (and the parse is refused) if the memory cap cannot be installed; a
/// stricter inherited limit is kept.
#[cfg(target_os = "linux")]
pub(super) fn apply_child_limits() -> Result<()> {
    // Best effort: some containers forbid it. The address-space cap below is
    // the real guard, and a warning here would be logged on every parse.
    let _ = std::fs::write("/proc/self/oom_score_adj", "1000");
    let (current_soft, current_hard) = address_space_limit()?;
    let cap = super::CHILD_MEMORY_LIMIT_BYTES as libc::rlim_t;
    let hard = current_hard.min(cap);
    let limit = libc::rlimit {
        rlim_cur: current_soft.min(hard),
        rlim_max: hard,
    };
    // SAFETY: setrlimit only reads `limit` and changes the limits of this (child) process alone.
    if unsafe { libc::setrlimit(libc::RLIMIT_AS, &limit) } != 0 {
        return Err(OragError::Internal(format!(
            "cannot install the parser memory limit: {}",
            io::Error::last_os_error()
        )));
    }
    Ok(())
}

/// Current (soft, hard) RLIMIT_AS of this process.
#[cfg(target_os = "linux")]
pub(super) fn address_space_limit() -> Result<(libc::rlim_t, libc::rlim_t)> {
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: getrlimit writes into the valid `limit` struct we pass.
    if unsafe { libc::getrlimit(libc::RLIMIT_AS, &mut limit) } != 0 {
        return Err(OragError::Internal(format!(
            "cannot read the address-space limit: {}",
            io::Error::last_os_error()
        )));
    }
    Ok((limit.rlim_cur, limit.rlim_max))
}

#[cfg(not(target_os = "linux"))]
pub(super) fn apply_child_limits() -> Result<()> {
    Ok(())
}
