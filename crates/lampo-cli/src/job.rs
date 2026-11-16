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
//! left running (a detached editor, a daemon) survives, as it did without Lampo.
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
    let mut child = command.spawn()?;
    if !launcher_joined {
        // Fallback when the launcher cannot join: only what the child starts from now on is tied.
        if let Some(tie) = &tie {
            let _untied = tie.join_child(&child);
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

    use windows_sys::Win32::Foundation::HANDLE;
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

    #[cfg(test)]
    mod tests {
        use std::mem::ManuallyDrop;
        use std::process::{Command, Stdio};
        use std::time::{Duration, Instant};

        use windows_sys::Win32::Foundation::CloseHandle;

        use super::*;

        #[test]
        fn a_disarmed_job_lets_its_processes_live() {
            let tie = Tie::new().unwrap();
            let mut child = Command::new("ping").args(["-n", "3", "127.0.0.1"]).stdout(Stdio::null()).spawn().unwrap();
            tie.join_child(&child).unwrap();
            drop(tie);
            assert!(child.try_wait().unwrap().is_none(), "dropping the tie must not kill the child");
            assert!(child.wait().unwrap().success());
        }

        #[test]
        fn an_armed_job_kills_its_processes_when_closed() {
            let tie = Tie::new().unwrap();
            // About 30 seconds of work unless killed.
            let mut child = Command::new("ping").args(["-n", "31", "127.0.0.1"]).stdout(Stdio::null()).spawn().unwrap();
            tie.join_child(&child).unwrap();
            let started = Instant::now();
            // Close the armed handle without the drop's disarming: what the OS does when the
            // launcher dies.
            let tie = ManuallyDrop::new(tie);
            // SAFETY: the handle is closed once, here; ManuallyDrop prevents a second close.
            unsafe { CloseHandle(tie.raw()) };
            child.wait().unwrap();
            assert!(started.elapsed() < Duration::from_secs(10), "the child must be killed with the job");
        }
    }
}
