//! Ties the tool started by a shim or by `lampo exec` to the launcher's lifetime.
//!
//! An IDE, Node or a CI runner that stops a command kills the process it started: the shim (or
//! `lampo exec`), not the real tool. Without a tie, `php artisan serve` and the PHP server it
//! spawned would keep running, orphaned. The launcher therefore joins a Job Object with
//! `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` *before* starting the tool, so the tool and everything it
//! starts belong to the job from their first instruction; when the launcher dies, the OS closes
//! the job handle and kills the whole tree.
//!
//! The limit is armed only while the tool runs. When the tool exits on its own (or cannot be
//! started), the launcher disarms it before closing the handle: whatever the tool deliberately
//! left running (a detached editor, a daemon) survives, as it did without Lampo. The same holds
//! when the console goes away (window or tab closed, logoff, shutdown): Windows may end the
//! launcher with `ExitProcess`, which runs no destructor, while the tool is still running, so a
//! console handler disarms the job first. The tool, attached to the same console, still receives
//! the event and closes as usual.
//!
//! The tie is best effort: when no job can be created or joined (a restrictive parent job), the
//! tool runs untied rather than not at all.

use std::io;
use std::process::{Command, ExitStatus};

/// Runs `command` to completion inside a kill-on-close job and returns its exit status.
#[cfg(windows)]
pub fn run_tied(command: &mut Command) -> io::Result<ExitStatus> {
    let tie = win::Tie::new().ok();
    let launcher_joined = tie.as_ref().is_some_and(|t| t.join_current_process().is_ok());
    // Released (disarmed, then closed) when this returns, or earlier if the console goes away.
    let held = tie.map(win::Held::new);
    let mut child = command.spawn()?;
    if !launcher_joined {
        // Fallback when the launcher cannot join: only what the child starts from now on is tied.
        if let Some(held) = &held {
            let _untied = held.join_child(&child);
        }
    }
    child.wait()
}

#[cfg(not(windows))]
pub fn run_tied(command: &mut Command) -> io::Result<ExitStatus> {
    command.status()
}

#[cfg(windows)]
mod win {
    use std::io;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use std::process::Child;
    use std::sync::{Mutex, MutexGuard, Once, PoisonError};

    use windows_sys::Win32::Foundation::{BOOL, FALSE, HANDLE, TRUE};
    use windows_sys::Win32::System::Console::{CTRL_CLOSE_EVENT, CTRL_LOGOFF_EVENT, CTRL_SHUTDOWN_EVENT, SetConsoleCtrlHandler};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_BREAKAWAY_OK, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JobObjectExtendedLimitInformation, SetInformationJobObject,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    /// While the tool runs: closing the handle kills every process of the job. Programs that
    /// detach on purpose (`CREATE_BREAKAWAY_FROM_JOB`) may still leave, as they could before.
    const WHILE_RUNNING: u32 = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_BREAKAWAY_OK;
    /// Once the launcher is done: closing the handle kills nothing.
    const DISARMED: u32 = JOB_OBJECT_LIMIT_BREAKAWAY_OK;

    /// An armed job; dropping it disarms it, then closes it. A launcher killed by force never
    /// runs the drop, which is the point: the OS closes the armed handle.
    pub struct Tie(OwnedHandle);

    impl Tie {
        pub fn new() -> io::Result<Self> {
            // SAFETY: anonymous job with default security; the handle is checked below.
            let raw = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
            if raw.is_null() {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: a valid handle freshly returned to us; ownership is transferred.
            let tie = Self(unsafe { OwnedHandle::from_raw_handle(raw) });
            tie.set_limits(WHILE_RUNNING)?;
            Ok(tie)
        }

        /// Puts the launcher itself in the job: every process it starts from now on inherits it.
        pub fn join_current_process(&self) -> io::Result<()> {
            // SAFETY: the pseudo-handle of the current process needs no closing.
            self.assign(unsafe { GetCurrentProcess() })
        }

        pub fn join_child(&self, child: &Child) -> io::Result<()> {
            self.assign(child.as_raw_handle() as HANDLE)
        }

        fn assign(&self, process: HANDLE) -> io::Result<()> {
            // SAFETY: valid job and process handles.
            let ok = unsafe { AssignProcessToJobObject(self.raw(), process) };
            if ok == 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
        }

        fn set_limits(&self, flags: u32) -> io::Result<()> {
            // SAFETY: plain C struct, all zeroes is a valid value.
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
            info.BasicLimitInformation.LimitFlags = flags;
            let size = size_of_val(&info) as u32;
            // SAFETY: valid job handle and a correctly sized struct.
            let ok = unsafe { SetInformationJobObject(self.raw(), JobObjectExtendedLimitInformation, (&raw const info).cast(), size) };
            if ok == 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
        }

        fn raw(&self) -> HANDLE {
            self.0.as_raw_handle() as HANDLE
        }
    }

    impl Drop for Tie {
        fn drop(&mut self) {
            // Nothing to report to: should disarming fail, closing the handle kills what the
            // tool left running (and the launcher, which is in the job and only has to exit).
            let _disarm_failed = self.set_limits(DISARMED);
        }
    }

    /// The tie of the tool now running, where the console handler can reach it.
    static RUNNING: Mutex<Option<Tie>> = Mutex::new(None);

    fn running() -> MutexGuard<'static, Option<Tie>> {
        RUNNING.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Keeps the tie in [`RUNNING`] while the tool runs (one tool at a time per process) and
    /// releases it when dropped. The console handler releases it earlier if the console goes away.
    pub struct Held(());

    impl Held {
        pub fn new(tie: Tie) -> Self {
            static HANDLER: Once = Once::new();
            HANDLER.call_once(|| {
                // SAFETY: plain kernel32 call with a valid `extern "system"` function pointer.
                // Should it fail, closing the console kills the tree, as a forced kill does.
                let _unregistered = unsafe { SetConsoleCtrlHandler(Some(release_on_session_end), TRUE) };
            });
            let mut slot = running();
            debug_assert!(slot.is_none(), "one tied tool at a time");
            *slot = Some(tie);
            Self(())
        }

        pub fn join_child(&self, child: &Child) -> io::Result<()> {
            running().as_ref().map_or(Ok(()), |tie| tie.join_child(child))
        }
    }

    impl Drop for Held {
        fn drop(&mut self) {
            release();
        }
    }

    /// Disarms and closes the running tie, if any.
    fn release() {
        drop(running().take());
    }

    /// Releases the tie on the events after which Windows ends the launcher with `ExitProcess`.
    /// Ctrl+C and Ctrl+Break leave it armed: the launcher survives them (see `console`).
    fn on_console_event(event: u32) {
        if matches!(event, CTRL_CLOSE_EVENT | CTRL_LOGOFF_EVENT | CTRL_SHUTDOWN_EVENT) {
            release();
        }
    }

    /// Console handler. It never handles the event: the next handler, or the default one that
    /// ends the launcher, still runs.
    unsafe extern "system" fn release_on_session_end(event: u32) -> BOOL {
        on_console_event(event);
        FALSE
    }

    #[cfg(test)]
    mod tests {
        use std::mem::ManuallyDrop;
        use std::process::{Command, Stdio};
        use std::time::{Duration, Instant};

        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Console::{CTRL_BREAK_EVENT, CTRL_C_EVENT};

        use super::*;

        /// Pings of a child that should outlive its job: about 3 seconds of work.
        const SHORT_WORK: &str = "4";
        /// Pings of a child that should be killed: about 30 seconds of work.
        const LONG_WORK: &str = "31";
        /// A child still alive this long after its job was closed was not killed with it (job
        /// termination is asynchronous: right after the close the child may still be running).
        const SURVIVED: Duration = Duration::from_secs(2);
        /// A child gone within this time was killed with its job.
        const KILLED: Duration = Duration::from_secs(10);

        /// Serializes the tests that use the process-wide [`RUNNING`] slot.
        static SLOT: Mutex<()> = Mutex::new(());

        fn spawn_ping(count: &str) -> Child {
            Command::new("ping").args(["-n", count, "127.0.0.1"]).stdout(Stdio::null()).spawn().unwrap()
        }

        /// Closes the armed handle without the drop's disarming: what the OS does when the
        /// launcher dies.
        fn close_armed(tie: Tie) {
            let tie = ManuallyDrop::new(tie);
            // SAFETY: the handle is closed once, here; ManuallyDrop prevents a second close.
            unsafe { CloseHandle(tie.raw()) };
        }

        /// Holds a fresh tie with `child` in it, as `run_tied` does.
        fn hold_with(child: &Child) -> Held {
            let held = Held::new(Tie::new().unwrap());
            held.join_child(child).unwrap();
            held
        }

        /// What `ExitProcess` does to the launcher: no destructor runs, the OS closes the handles.
        fn exit_process(held: Held) {
            std::mem::forget(held);
            if let Some(tie) = running().take() {
                close_armed(tie);
            }
        }

        fn assert_survives(mut child: Child, why: &str) {
            let closed = Instant::now();
            assert!(child.wait().unwrap().success(), "{why}");
            assert!(closed.elapsed() >= SURVIVED, "{why}");
        }

        fn assert_killed(mut child: Child, why: &str) {
            let closed = Instant::now();
            child.wait().unwrap();
            assert!(closed.elapsed() < KILLED, "{why}");
        }

        #[test]
        fn a_disarmed_job_lets_its_processes_live() {
            let tie = Tie::new().unwrap();
            let child = spawn_ping(SHORT_WORK);
            tie.join_child(&child).unwrap();
            drop(tie);
            assert_survives(child, "dropping the tie must not kill the child");
        }

        #[test]
        fn an_armed_job_kills_its_processes_when_closed() {
            let tie = Tie::new().unwrap();
            let child = spawn_ping(LONG_WORK);
            tie.join_child(&child).unwrap();
            close_armed(tie);
            assert_killed(child, "the child must be killed with the job");
        }

        #[test]
        fn a_tool_done_on_its_own_releases_the_held_tie_and_its_programs_live() {
            let _slot = SLOT.lock().unwrap_or_else(PoisonError::into_inner);
            let child = spawn_ping(SHORT_WORK);
            drop(hold_with(&child));
            assert!(running().is_none(), "the slot must be empty once the tool is done");
            assert_survives(child, "a tool that exits on its own leaves its programs alive");
        }

        #[test]
        fn the_end_of_the_console_lets_the_detached_programs_live() {
            let _slot = SLOT.lock().unwrap_or_else(PoisonError::into_inner);
            for event in [CTRL_CLOSE_EVENT, CTRL_LOGOFF_EVENT, CTRL_SHUTDOWN_EVENT] {
                let child = spawn_ping(SHORT_WORK);
                let held = hold_with(&child);
                on_console_event(event);
                exit_process(held);
                assert_survives(child, &format!("console event {event} must disarm the job"));
            }
        }

        #[test]
        fn ctrl_c_and_ctrl_break_leave_the_job_armed() {
            let _slot = SLOT.lock().unwrap_or_else(PoisonError::into_inner);
            for event in [CTRL_C_EVENT, CTRL_BREAK_EVENT] {
                let child = spawn_ping(LONG_WORK);
                let held = hold_with(&child);
                on_console_event(event);
                exit_process(held);
                assert_killed(child, &format!("console event {event} must not disarm the job"));
            }
        }
    }
}
