//! Last-resort clean MySQL shutdown, for when the app is going away without the graceful stop of
//! every service (a panic of the main thread, a session end that ran out of time). The app's Job
//! Object kills every child the moment the process ends: InnoDB then needs crash recovery and the
//! MyISAM `mysql.*` tables of 5.7 can be left corrupted.
//!
//! Synchronous and stateless (a process snapshot of the app's descendants, the
//! `MYSQLShutdown<pid>` event, blocking waits), so it is safe inside a panic hook or a window
//! procedure, where the async runtime and the app state may be unusable.

use std::os::windows::io::OwnedHandle;
use std::time::{Duration, Instant};

use crate::mysql::signal_shutdown_event;
use crate::win;

/// mysqld's executable (also the name of the 8.0 monitor process that runs the server).
const MYSQLD_EXE: &str = "mysqld.exe";

/// What [`stop_mysql_servers`] did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct LastResortStop {
    /// mysqld processes of this app that were running.
    pub found: usize,
    /// Those that accepted the shutdown request.
    pub signalled: usize,
    /// Those still running when the wait ended (the Job Object kills them with the app).
    pub still_running: usize,
}

/// Asks every mysqld this app started for a clean shutdown and waits up to `timeout` for all
/// of them to exit. Does nothing when no mysqld is running.
pub fn stop_mysql_servers(timeout: Duration) -> LastResortStop {
    stop_processes(win::descendants_named(std::process::id(), MYSQLD_EXE), signal_shutdown_event, timeout)
}

/// Sends `request_shutdown` to each of `pids`, then waits up to `timeout` for all of them.
/// Nothing accepted the request: no wait (the processes would not exit on their own).
fn stop_processes(pids: Vec<u32>, request_shutdown: impl Fn(u32) -> bool, timeout: Duration) -> LastResortStop {
    // Opened before signalling: a handle keeps the pid from being reused while we wait.
    let processes: Vec<(u32, OwnedHandle)> = pids.into_iter().filter_map(|pid| win::open_for_wait(pid).map(|handle| (pid, handle))).collect();
    let found = processes.len();
    let signalled = processes.iter().filter(|(pid, _)| request_shutdown(*pid)).count();
    if signalled == 0 {
        return LastResortStop { found, signalled, still_running: found };
    }
    let deadline = Instant::now() + timeout;
    let still_running = processes.iter().filter(|(_, handle)| !win::wait_for_exit_blocking(handle, deadline.saturating_duration_since(Instant::now()))).count();
    LastResortStop { found, signalled, still_running }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::win::test_support::Sleeper;

    #[test]
    fn a_process_that_honours_the_request_is_waited_for() {
        let sleeper = Mutex::new(Sleeper::spawn());
        let pid = sleeper.lock().unwrap().pid();
        let request = |p: u32| {
            sleeper.lock().unwrap().kill();
            p == pid
        };
        let stop = stop_processes(vec![pid], request, Duration::from_secs(10));
        assert_eq!(stop, LastResortStop { found: 1, signalled: 1, still_running: 0 });
    }

    #[test]
    fn a_process_that_ignores_the_request_is_reported_after_the_timeout() {
        let sleeper = Sleeper::spawn();
        let started = Instant::now();
        let stop = stop_processes(vec![sleeper.pid()], |_| true, Duration::from_millis(200));
        assert_eq!(stop, LastResortStop { found: 1, signalled: 1, still_running: 1 });
        // WaitForSingleObject times out on the system timer tick (~15.6 ms) and can return a tick
        // earlier than Instant measures: the margin keeps the check independent of that granularity.
        assert!(started.elapsed() >= Duration::from_millis(150));
    }

    #[test]
    fn nothing_signalled_means_no_wait() {
        let sleeper = Sleeper::spawn();
        let started = Instant::now();
        let stop = stop_processes(vec![sleeper.pid()], |_| false, Duration::from_secs(30));
        assert_eq!(stop, LastResortStop { found: 1, signalled: 0, still_running: 1 });
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn processes_that_cannot_be_opened_are_skipped() {
        // Pid 0 (the idle process) can never be opened, like a process that is already gone.
        let stop = stop_processes(vec![0], |_| panic!("an unopenable process is never signalled"), Duration::from_secs(1));
        assert_eq!(stop, LastResortStop::default());
    }

    #[test]
    fn without_mysqld_children_nothing_happens() {
        assert_eq!(stop_mysql_servers(Duration::from_secs(1)), LastResortStop::default());
    }
}
