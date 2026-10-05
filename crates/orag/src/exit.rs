//! Process exit without native teardown.

/// Ends the process at once: `_exit` skips atexit handlers and C++ static
/// destructors. Used where those cannot run safely: a llama.cpp context still
/// alive at `exit` makes its static Metal teardown abort (SIGABRT), and a
/// parser child's watchdog fires while its main thread may hold locks those
/// destructors need. Pending stdio is flushed first. An unfinished job is
/// requeued on the next start and SQLite's WAL survives an abrupt exit.
pub(crate) fn exit_without_native_teardown(code: i32) -> ! {
    use std::io::Write;
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
    unsafe extern "C" {
        fn _exit(status: i32) -> !;
    }
    // SAFETY: `_exit` is async-signal-safe, takes a plain int and never returns.
    unsafe { _exit(code) }
}
