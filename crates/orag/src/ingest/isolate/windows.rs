//! Windows side of parser isolation: each child runs in its own Job Object,
//! which caps its memory, kills everything in it at once, and kills the child
//! when `orag serve` dies (the job handle closes with the parent).

use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::os::windows::process::CommandExt;
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, ExitStatus};

use windows_sys::Win32::Foundation::{HANDLE, WAIT_FAILED, WAIT_OBJECT_0};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_LIMIT_PROCESS_MEMORY,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject,
};
use windows_sys::Win32::System::Threading::{
    CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW, WaitForSingleObject,
};

use crate::error::{OragError, Result};

/// Exit codes from this value up are exception statuses (NTSTATUS warning
/// and error severity, C++ exceptions): the process died of a crash
/// (access violation, stack overflow, Rust's abort / fast fail, heap
/// corruption, an out-of-memory failure under the job's cap, a breakpoint).
/// Plain exit codes below it are error exits, as on unix.
const FIRST_EXCEPTION_CODE: u32 = 0x8000_0000;

/// A parser child inside its own Job Object.
pub(super) struct Contained {
    child: Child,
    /// Closing it (drop) kills the child: `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`.
    job: OwnedHandle,
}

/// Starts `command` with no console (so console Ctrl-C/Ctrl-Break and window
/// closing never reach it: `orag serve` stops it and reports `Interrupted`)
/// and puts it into a new job before anything is written to its stdin. The
/// child reads all of stdin before doing any work, so it cannot start a
/// process outside the job. A child that cannot be contained is killed.
pub(super) fn spawn(command: &mut Command) -> io::Result<Contained> {
    let job = new_job()?;
    command.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
    let mut child = command.spawn()?;
    // SAFETY: both handles are valid and owned (the job by `job`, the process by `child`).
    let assigned = unsafe {
        AssignProcessToJobObject(
            job.as_raw_handle() as HANDLE,
            child.as_raw_handle() as HANDLE,
        )
    };
    if assigned == 0 {
        let err = io::Error::last_os_error();
        let _ = child.kill();
        let _ = child.wait();
        return Err(io::Error::new(
            err.kind(),
            format!("cannot put the parser process into a job object: {err}"),
        ));
    }
    Ok(Contained { child, job })
}

/// A job that kills its processes when its last handle closes, on an
/// unhandled exception (no error-report dialog), and caps each process's
/// committed memory like the Linux address-space cap.
fn new_job() -> io::Result<OwnedHandle> {
    // SAFETY: null attributes and name create an anonymous, non-inheritable job.
    let raw = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if raw.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `raw` is a fresh handle we now own exclusively.
    let job = unsafe { OwnedHandle::from_raw_handle(raw as _) };
    // SAFETY: an all-zero JOBOBJECT_EXTENDED_LIMIT_INFORMATION is valid (no limits).
    let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
        | JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION
        | JOB_OBJECT_LIMIT_PROCESS_MEMORY;
    limits.ProcessMemoryLimit =
        usize::try_from(super::CHILD_MEMORY_LIMIT_BYTES).unwrap_or(usize::MAX);
    // SAFETY: the pointer and size describe `limits`, which outlives the call.
    let set = unsafe {
        SetInformationJobObject(
            job.as_raw_handle() as HANDLE,
            JobObjectExtendedLimitInformation,
            std::ptr::from_ref(&limits).cast(),
            u32::try_from(std::mem::size_of_val(&limits)).unwrap_or(u32::MAX),
        )
    };
    if set == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(job)
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

    /// True once the child has exited. The process handle stays open until
    /// `wait`, so the process id cannot be reused before the job is killed.
    pub(super) fn exited(&mut self) -> io::Result<bool> {
        // SAFETY: the process handle is valid while `self.child` lives.
        let state = unsafe { WaitForSingleObject(self.child.as_raw_handle() as HANDLE, 0) };
        if state == WAIT_FAILED {
            return Err(io::Error::last_os_error());
        }
        Ok(state == WAIT_OBJECT_0)
    }

    /// Ends every process in the job (the child and anything it started).
    pub(super) fn kill_tree(&mut self) {
        // SAFETY: the job handle is valid while `self.job` lives.
        let terminated = unsafe { TerminateJobObject(self.job.as_raw_handle() as HANDLE, 1) };
        if terminated == 0 {
            let _ = self.child.kill();
        }
    }

    /// Collects the exit status; call after `kill_tree`.
    pub(super) fn wait(&mut self) -> io::Result<ExitStatus> {
        self.child.wait()
    }
}

/// A crash (an exception status, see `FIRST_EXCEPTION_CODE`) is the file's
/// fault, like a crash signal on unix; a plain error exit is a host problem:
/// `run_child` reports a bad file, also a parser panic, as a `Rejected`
/// result, never as an exit code. `TerminateJobObject` (our own kill) exits
/// with 1 but is never classified: the stop and deadline paths return first.
pub(super) fn classify(status: ExitStatus) -> OragError {
    match status.code().map(|code| code as u32) {
        Some(code) if code >= FIRST_EXCEPTION_CODE => super::file_fault(),
        _ => OragError::Internal(format!("the parser process failed ({status})")),
    }
}

/// The child has no console (`CREATE_NO_WINDOW`), so console control events
/// never reach it; nothing to ignore.
pub(super) fn ignore_stop_signals() {}

/// The job kills the child when the parent dies (its handle closes), so the
/// watchdog only needs the absolute deadline.
pub(super) fn orphan_check() -> impl Fn() -> bool + Send + 'static {
    || false
}

/// The memory cap is the job's (set by the parent in `spawn`).
pub(super) fn apply_child_limits() -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::os::windows::process::ExitStatusExt;

    use super::*;

    #[test]
    fn crash_codes_are_the_files_fault_and_error_exits_the_hosts() {
        let status = |code: u32| ExitStatus::from_raw(code);
        // Access violation, illegal instruction, stack overflow, heap
        // corruption, fast fail (Rust abort), breakpoint, C++ exception.
        let crashes = [
            0xC000_0005,
            0xC000_001D,
            0xC000_00FD,
            0xC000_0374,
            0xC000_0409,
            0x8000_0003,
            0xE06D_7363,
        ];
        for code in crashes {
            assert!(
                matches!(classify(status(code)), OragError::InvalidInput(_)),
                "{code:#x}"
            );
        }
        for code in [1, 3, 101] {
            assert!(
                matches!(classify(status(code)), OragError::Internal(_)),
                "{code}"
            );
        }
    }

    #[test]
    fn closing_the_job_kills_the_child() {
        let mut command = Command::new("cmd");
        command.args(["/C", "ping -n 60 127.0.0.1 >NUL"]);
        let mut contained = spawn(&mut command).unwrap();
        assert!(!contained.exited().unwrap());
        let Contained { mut child, job } = contained;
        drop(job);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while child.try_wait().unwrap().is_none() {
            assert!(
                std::time::Instant::now() < deadline,
                "child survived its job"
            );
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }

    #[test]
    fn kill_tree_ends_grandchildren() {
        // cmd starts ping as a grandchild; both are in the job.
        let mut command = Command::new("cmd");
        command
            .args(["/C", "ping -n 60 127.0.0.1"])
            .stdout(std::process::Stdio::piped());
        let mut contained = spawn(&mut command).unwrap();
        let (_, stdout, _) = contained.take_pipes();
        // Let cmd start ping (inside the job) before the kill.
        std::thread::sleep(std::time::Duration::from_millis(500));
        contained.kill_tree();
        contained.wait().unwrap();
        // The pipe closes only when every process holding it (ping too) is
        // gone; ping alone would keep it open for about a minute.
        let (done, reached) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut rest = Vec::new();
            let _ = std::io::Read::read_to_end(&mut stdout.unwrap(), &mut rest);
            let _ = done.send(());
        });
        reached
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("a grandchild kept the pipe open: kill_tree missed it");
    }
}
