use std::os::unix::process::CommandExt;
use std::process::Command;

/// SIGTERM (a graceful stop for `orag serve`) when the spawning thread dies.
pub(super) fn die_with_parent(command: &mut Command) {
    // SAFETY: getpid is async-signal-safe and called before fork.
    let parent = unsafe { libc::getpid() };
    // SAFETY: prctl, getppid and _exit are async-signal-safe; the closure
    // allocates nothing and touches no lock between fork and exec.
    unsafe {
        command.pre_exec(move || {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            // The parent died before the signal was armed: nobody would send it.
            if libc::getppid() != parent {
                libc::_exit(0);
            }
            Ok(())
        });
    }
}
