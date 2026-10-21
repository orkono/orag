//! The bench's navigation channel (v0.6 step 88): the service page's
//! button navigates to `orag-bench://run`, the app's status page to
//! `orag-bench://cancel` and `orag-bench://resume`. The app cancels every
//! such navigation, and acts only on an exact command from the page it
//! currently shows, as `orag-lang:` (`window::language_report`).

use tauri::Url;

pub const BENCH_SCHEME: &str = "orag-bench";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Run,
    Cancel,
    Resume,
}

/// Which page the window shows, as the app tracks it (`on_navigation`
/// sees only the target URL).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shown {
    Service,
    Local,
    Other,
}

/// Where the bench is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Running,
    /// Finished; the service waits for "Devam".
    Done,
}

pub fn is_bench(url: &Url) -> bool {
    url.scheme() == BENCH_SCHEME
}

/// The command `url` names, or None for anything but exactly
/// `orag-bench://<run|cancel|resume>`.
pub fn command(url: &Url) -> Option<Command> {
    let bare = is_bench(url)
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && url.path().is_empty()
        && url.query().is_none()
        && url.fragment().is_none();
    if !bare {
        return None;
    }
    match url.host_str()? {
        "run" => Some(Command::Run),
        "cancel" => Some(Command::Cancel),
        "resume" => Some(Command::Resume),
        _ => None,
    }
}

/// Whether the app acts on `command` now: a run only from the service
/// page while no bench is going (one at a time), a cancel only from the
/// status page while it runs, a resume only from it once it is done.
pub fn allowed(command: Command, shown: Shown, phase: Phase) -> bool {
    matches!(
        (command, shown, phase),
        (Command::Run, Shown::Service, Phase::Idle)
            | (Command::Cancel, Shown::Local, Phase::Running)
            | (Command::Resume, Shown::Local, Phase::Done)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(text: &str) -> Url {
        Url::parse(text).unwrap()
    }

    #[test]
    fn only_exact_commands_are_read() {
        assert_eq!(command(&url("orag-bench://run")), Some(Command::Run));
        assert_eq!(command(&url("orag-bench://cancel")), Some(Command::Cancel));
        assert_eq!(command(&url("orag-bench://resume")), Some(Command::Resume));
        for bad in [
            "orag-bench://other",
            "orag-bench://run?x=1",
            "orag-bench://run/x",
            "orag-bench://run:80",
            "orag-bench://me@run",
            "orag-bench://run#x",
            "orag-bench:run",
            "orag-lang://run",
        ] {
            assert_eq!(command(&url(bad)), None, "{bad}");
        }
        assert!(is_bench(&url("orag-bench://other")));
    }

    #[test]
    fn each_command_comes_from_its_page_at_its_time() {
        use Command::*;
        use Phase::*;
        use Shown::*;
        assert!(allowed(Run, Service, Idle));
        assert!(!allowed(Run, Local, Idle), "a replay from the status page");
        assert!(!allowed(Run, Other, Idle), "any other page");
        assert!(!allowed(Run, Service, Running), "one bench at a time");
        assert!(!allowed(Run, Service, Done));
        assert!(allowed(Cancel, Local, Running));
        assert!(!allowed(Cancel, Service, Running));
        assert!(!allowed(Cancel, Local, Idle));
        assert!(allowed(Resume, Local, Done));
        assert!(!allowed(Resume, Local, Running));
        assert!(!allowed(Resume, Service, Done));
    }
}
