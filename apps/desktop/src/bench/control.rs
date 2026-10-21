//! Who may change the window while a bench runs (v0.6 step 88). Every
//! supervisor the app watches gets a generation; pausing the service for a
//! bench and resuming it each start a new one, so a late event of an older
//! supervisor (a `Restarting`, a `Ready` already queued) is dropped instead
//! of covering the bench page or leaving it. The app checks an event and
//! acts on it under the same lock that bench commands take, so the two
//! never interleave.

use crate::bench::channel::{Command, Phase, Shown, allowed};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Control {
    phase: Phase,
    generation: u64,
}

/// What the app does for a bench command it accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Show the bench page, then pause the service and run the bench.
    StartBench,
    /// Set the cancel flag.
    Cancel,
    /// Resume the paused policy (or start afresh) under this generation.
    Resume { generation: u64 },
}

impl Default for Control {
    fn default() -> Control {
        Control {
            phase: Phase::Idle,
            generation: 0,
        }
    }
}

impl Control {
    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// A new supervisor to watch (the first start): its generation.
    pub fn watch(&mut self) -> u64 {
        self.generation += 1;
        self.generation
    }

    /// Whether an event of the supervisor of `generation` may change the
    /// window: only the current one's, and never during a bench.
    pub fn current(&self, generation: u64) -> bool {
        self.phase == Phase::Idle && generation == self.generation
    }

    /// Accepts `command` from the page `shown`, or None.
    pub fn command(&mut self, command: Command, shown: Shown) -> Option<Step> {
        if !allowed(command, shown, self.phase) {
            return None;
        }
        Some(match command {
            Command::Run => {
                self.phase = Phase::Running;
                // The running supervisor's events are stale from now on.
                self.generation += 1;
                Step::StartBench
            }
            Command::Cancel => Step::Cancel,
            Command::Resume => {
                self.phase = Phase::Idle;
                Step::Resume {
                    generation: self.watch(),
                }
            }
        })
    }

    /// The bench thread ended (results written or not).
    pub fn finished(&mut self) {
        if self.phase == Phase::Running {
            self.phase = Phase::Done;
        }
    }
}

/// Runs `act` for an event of the supervisor of `generation` if that
/// supervisor is still current when the event's turn comes: the app queues
/// this onto the window's thread, where bench commands also run, so the
/// check and the window change happen in one step.
pub fn deliver(control: &std::sync::Mutex<Control>, generation: u64, act: impl FnOnce()) {
    let control = control.lock().unwrap_or_else(|e| e.into_inner());
    if control.current(generation) {
        act();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bench_makes_the_old_supervisors_events_stale_for_good() {
        let mut control = Control::default();
        let first = control.watch();
        assert!(control.current(first));
        assert_eq!(
            control.command(Command::Run, Shown::Service),
            Some(Step::StartBench)
        );
        assert!(
            !control.current(first),
            "a queued Restarting or Ready of the paused one"
        );
        assert_eq!(
            control.command(Command::Run, Shown::Service),
            None,
            "one at a time"
        );
        control.finished();
        assert_eq!(control.phase(), Phase::Done);
        assert!(!control.current(first));
        let Some(Step::Resume { generation }) = control.command(Command::Resume, Shown::Local)
        else {
            panic!("resume accepted");
        };
        assert!(control.current(generation));
        assert!(!control.current(first), "still stale after the resume");
    }

    #[test]
    fn commands_follow_the_phase_and_the_page() {
        let mut control = Control::default();
        control.watch();
        assert_eq!(control.command(Command::Cancel, Shown::Local), None);
        assert_eq!(control.command(Command::Resume, Shown::Local), None);
        assert_eq!(control.command(Command::Run, Shown::Local), None);
        assert_eq!(
            control.command(Command::Run, Shown::Service),
            Some(Step::StartBench)
        );
        // Cancel before the first progress: accepted while running.
        assert_eq!(
            control.command(Command::Cancel, Shown::Local),
            Some(Step::Cancel)
        );
        assert_eq!(
            control.command(Command::Resume, Shown::Local),
            None,
            "not done yet"
        );
        control.finished();
        assert_eq!(control.command(Command::Cancel, Shown::Local), None);
        assert!(matches!(
            control.command(Command::Resume, Shown::Local),
            Some(Step::Resume { .. })
        ));
        assert_eq!(control.command(Command::Resume, Shown::Local), None, "once");
    }

    /// The window's thread as a queue: an event queued before a bench
    /// command but run after it is dropped.
    #[test]
    fn an_event_queued_before_a_bench_starts_is_dropped_when_its_turn_comes() {
        use std::sync::Mutex;
        let control = Mutex::new(Control::default());
        let generation = control.lock().unwrap().watch();
        let acted = std::cell::Cell::new(Vec::new());
        let mut queue: std::collections::VecDeque<Box<dyn FnOnce() + '_>> = Default::default();
        queue.push_back(Box::new(|| {
            deliver(&control, generation, || {
                let mut seen = acted.take();
                seen.push("restarting");
                acted.set(seen);
            })
        }));
        queue.push_back(Box::new(|| {
            assert_eq!(
                control
                    .lock()
                    .unwrap()
                    .command(Command::Run, Shown::Service),
                Some(Step::StartBench)
            );
        }));
        queue.push_back(Box::new(|| {
            deliver(&control, generation, || {
                let mut seen = acted.take();
                seen.push("ready");
                acted.set(seen);
            })
        }));
        while let Some(job) = queue.pop_front() {
            job();
        }
        assert_eq!(
            acted.take(),
            vec!["restarting"],
            "the Ready after the bench start is dropped"
        );
    }
}
