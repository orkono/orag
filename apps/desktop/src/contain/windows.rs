use std::io;
use std::sync::OnceLock;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject,
};
use windows_sys::Win32::System::Threading::GetCurrentProcess;

/// The Job's handle, kept open (never closed) for the life of the app: when
/// the app process ends, Windows closes it and kills every process in the Job.
struct AppJob {
    _handle: HANDLE,
}
// SAFETY: a Job handle is a kernel object reference, usable from any thread.
unsafe impl Send for AppJob {}
unsafe impl Sync for AppJob {}

static APP_JOB: OnceLock<Result<AppJob, String>> = OnceLock::new();

/// Puts the app in a kill-on-close Job once. Children created afterwards are
/// in the Job from their creation (nested Jobs, Windows 8+), so there is no
/// moment in which a child exists outside it. WebView2 processes the app
/// starts later join it too and end with the app, which is wanted.
///
/// An app that already runs in a Job forbidding nesting cannot join one; the
/// error is returned and the caller still starts the sidecar, which then
/// relies on the stdin-close stop alone.
pub(super) fn enter_app_job() -> io::Result<()> {
    match APP_JOB.get_or_init(create_and_enter) {
        Ok(_) => Ok(()),
        Err(message) => Err(io::Error::other(message.clone())),
    }
}

fn create_and_enter() -> Result<AppJob, String> {
    // SAFETY: plain Win32 calls with valid arguments; the handle is closed on
    // every failure path and otherwise kept for the life of the process.
    unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() {
            return Err(format!("CreateJobObjectW: {}", io::Error::last_os_error()));
        }
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let size = u32::try_from(std::mem::size_of_val(&limits)).unwrap_or(u32::MAX);
        if SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            (&raw const limits).cast(),
            size,
        ) == 0
        {
            let err = io::Error::last_os_error();
            CloseHandle(job);
            return Err(format!("SetInformationJobObject: {err}"));
        }
        if AssignProcessToJobObject(job, GetCurrentProcess()) == 0 {
            let err = io::Error::last_os_error();
            CloseHandle(job);
            return Err(format!("AssignProcessToJobObject: {err}"));
        }
        Ok(AppJob { _handle: job })
    }
}
