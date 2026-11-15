use std::path::PathBuf;
use std::ptr::null;

use lampo_domain::ports::AppLauncher;
use lampo_domain::{PortError, PortResult};
use windows_sys::Win32::Foundation::ERROR_ACCESS_DENIED;
use windows_sys::Win32::System::Threading::{
    CREATE_BREAKAWAY_FROM_JOB, CREATE_NEW_PROCESS_GROUP, CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DETACHED_PROCESS, PROCESS_INFORMATION, STARTUPINFOW,
};

use crate::process::push_arg;
use crate::win::{self, wide};

/// The flag the desktop app recognises to start in the tray without a webview window.
pub const HEADLESS_FLAG: &str = "--headless";

/// Starts `<app_exe> --headless` detached and returns immediately (the caller waits for the
/// pipe, see `ControlClient::ensure_app`). The app must outlive the CLI and must not hold
/// anything of the caller, so it is created with:
///
/// * `CREATE_BREAKAWAY_FROM_JOB`: out of the caller's Job Object (terminals and agent
///   sandboxes often kill their job on exit); retried without it when the job forbids it;
/// * no inherited handles at all: `std::process::Command` would pass every inheritable handle,
///   including the CLI's redirected stdout, and a coding agent waiting for the CLI's output to
///   reach EOF would then hang for as long as the app runs;
/// * the app's own folder as working directory, so it never locks the caller's (e.g. a git
///   worktree about to be deleted);
/// * `DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP`: no console, not hit by the caller's Ctrl-C.
pub struct HeadlessAppLauncher {
    app_exe: PathBuf,
}

impl HeadlessAppLauncher {
    /// `app_exe`: `lampo-desktop.exe`, located by the CLI next to itself or, for the CLI copy
    /// in the shim dir, in the folder the app recorded (`lampo_infra_store::app_location`).
    pub fn new(app_exe: PathBuf) -> Self {
        Self { app_exe }
    }

    fn create(&self, breakaway: bool) -> std::io::Result<()> {
        let mut cmd = Vec::new();
        push_arg(&mut cmd, self.app_exe.as_os_str(), true);
        push_arg(&mut cmd, HEADLESS_FLAG.as_ref(), false);
        cmd.push(0);
        let app = wide(&self.app_exe);
        let cwd = self.app_exe.parent().map(wide);
        // SAFETY: plain C structs.
        let mut si: STARTUPINFOW = unsafe { std::mem::zeroed() };
        si.cb = size_of::<STARTUPINFOW>() as u32;
        let mut pi: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
        let mut flags = DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_UNICODE_ENVIRONMENT;
        if breakaway {
            flags |= CREATE_BREAKAWAY_FROM_JOB;
        }
        // SAFETY: all pointers valid for the call; bInheritHandles = FALSE.
        let ok = unsafe { CreateProcessW(app.as_ptr(), cmd.as_mut_ptr(), null(), null(), 0, flags, null(), cwd.as_ref().map_or(null(), |c| c.as_ptr()), &si, &mut pi) };
        if ok == 0 {
            return Err(std::io::Error::last_os_error());
        }
        win::close(pi.hThread);
        win::close(pi.hProcess);
        Ok(())
    }
}

impl AppLauncher for HeadlessAppLauncher {
    fn launch_headless(&self) -> PortResult<()> {
        if !self.app_exe.is_file() {
            return Err(PortError::not_found("app executable", self.app_exe.display().to_string()));
        }
        let result = match self.create(true) {
            Err(e) if e.raw_os_error() == Some(ERROR_ACCESS_DENIED as i32) => self.create(false),
            other => other,
        };
        result.map_err(|e| PortError::process(self.app_exe.display().to_string(), e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_app_is_not_found() {
        let l = HeadlessAppLauncher::new(r"C:\definitely\not\here\lampo-desktop.exe".into());
        assert!(matches!(l.launch_headless(), Err(PortError::NotFound { .. })));
    }

    #[test]
    fn launches_detached_and_returns_immediately() {
        // `whoami --headless` rejects the flag and exits at once: proves the launch path only.
        let whoami = PathBuf::from(std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into())).join(r"System32\whoami.exe");
        let started = std::time::Instant::now();
        HeadlessAppLauncher::new(whoami).launch_headless().unwrap();
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
    }
}
