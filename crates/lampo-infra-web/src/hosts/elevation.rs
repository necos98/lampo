//! Starting `lampo-hosts-helper.exe`. The production launcher goes through UAC
//! (`ShellExecuteExW` with the `runas` verb); tests plug in a launcher that runs the helper
//! directly against a temp file.

use std::path::Path;
use std::time::Duration;

use lampo_domain::{PortError, PortResult};

/// How a helper run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchOutcome {
    Exited(i32),
    /// The user refused the UAC prompt.
    Denied,
}

/// Runs the helper with `args` and waits for it. Blocking.
pub trait HelperLauncher: Send + Sync {
    fn run(&self, helper: &Path, args: &[String]) -> PortResult<LaunchOutcome>;
}

/// Elevated launch through the UAC consent prompt.
#[derive(Debug, Clone, Copy)]
pub struct RunAsLauncher {
    pub timeout: Duration,
}

impl Default for RunAsLauncher {
    fn default() -> Self {
        // Covers the time the user takes to read the prompt.
        Self { timeout: Duration::from_secs(300) }
    }
}

/// Command-line quoting for arguments that contain no `"` (temp paths, flags).
pub fn command_line(args: &[String]) -> String {
    args.iter()
        .map(|a| {
            if a.is_empty() || a.contains([' ', '\t']) {
                // Backslashes before the closing quote must be doubled (MSVC argv rules).
                let trailing = a.len() - a.trim_end_matches('\\').len();
                format!("\"{a}{}\"", "\\".repeat(trailing))
            } else {
                a.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(windows)]
impl HelperLauncher for RunAsLauncher {
    fn run(&self, helper: &Path, args: &[String]) -> PortResult<LaunchOutcome> {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Foundation::{CloseHandle, ERROR_CANCELLED, WAIT_OBJECT_0};
        use windows_sys::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject};
        use windows_sys::Win32::UI::Shell::{SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW};
        use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, SW_HIDE};

        if args.iter().any(|a| a.contains('"')) {
            return Err(PortError::invalid_data("hosts helper arguments", "quotes are not allowed"));
        }
        let wide = |s: &std::ffi::OsStr| s.encode_wide().chain(std::iter::once(0)).collect::<Vec<u16>>();
        let verb = wide("runas".as_ref());
        let file = wide(helper.as_os_str());
        let params = wide(command_line(args).as_ref());
        // SAFETY: SHELLEXECUTEINFOW is plain data; every pointer stored in it refers to a
        // NUL-terminated buffer that outlives the call; the returned process handle is closed once.
        unsafe {
            let mut info: SHELLEXECUTEINFOW = std::mem::zeroed();
            info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
            info.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC;
            info.hwnd = GetForegroundWindow();
            info.lpVerb = verb.as_ptr();
            info.lpFile = file.as_ptr();
            info.lpParameters = params.as_ptr();
            info.nShow = SW_HIDE;
            if ShellExecuteExW(&mut info) == 0 {
                let err = std::io::Error::last_os_error();
                if err.raw_os_error() == Some(ERROR_CANCELLED as i32) {
                    return Ok(LaunchOutcome::Denied);
                }
                return Err(PortError::process("lampo-hosts-helper", format!("cannot start the elevated helper: {err}")));
            }
            if info.hProcess.is_null() {
                return Err(PortError::process("lampo-hosts-helper", "no process handle returned"));
            }
            let wait = WaitForSingleObject(info.hProcess, self.timeout.as_millis().min(u32::MAX as u128 - 1) as u32);
            let mut code = 0u32;
            let got = GetExitCodeProcess(info.hProcess, &mut code);
            CloseHandle(info.hProcess);
            if wait != WAIT_OBJECT_0 {
                return Err(PortError::process("lampo-hosts-helper", "the helper did not finish in time"));
            }
            if got == 0 {
                return Err(PortError::process("lampo-hosts-helper", std::io::Error::last_os_error()));
            }
            Ok(LaunchOutcome::Exited(code as i32))
        }
    }
}

#[cfg(not(windows))]
impl HelperLauncher for RunAsLauncher {
    fn run(&self, _helper: &Path, _args: &[String]) -> PortResult<LaunchOutcome> {
        Err(PortError::Unsupported("the hosts helper needs Windows".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_paths_with_spaces() {
        let args = vec!["--block".to_string(), r"C:\Users\John Smith\AppData\Local\Temp\lampo-hosts.json".to_string()];
        assert_eq!(command_line(&args), r#"--block "C:\Users\John Smith\AppData\Local\Temp\lampo-hosts.json""#);
        assert_eq!(command_line(&["a".into(), String::new()]), "a \"\"");
    }
}
