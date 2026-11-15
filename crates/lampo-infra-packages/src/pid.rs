//! Whether the process that left a file behind is still running: leftovers of a live process
//! belong to an operation in progress and must not be touched.

/// `true` while process `pid` runs. Lampo runs on Windows; elsewhere every process counts as
/// alive, so nothing that might be in use is ever deleted.
pub(crate) fn is_alive(pid: u32) -> bool {
    imp::is_alive(pid)
}

#[cfg(windows)]
mod imp {
    use windows_sys::Win32::Foundation::{CloseHandle, ERROR_ACCESS_DENIED, GetLastError, WAIT_TIMEOUT};
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject};

    pub(super) fn is_alive(pid: u32) -> bool {
        // SAFETY: plain Win32 calls; the handle is closed before returning.
        unsafe {
            let handle = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
            if handle.is_null() {
                // A process we may not open (another user's, a protected one) still exists.
                return GetLastError() == ERROR_ACCESS_DENIED;
            }
            let running = WaitForSingleObject(handle, 0) == WAIT_TIMEOUT;
            CloseHandle(handle);
            running
        }
    }
}

#[cfg(not(windows))]
mod imp {
    pub(super) fn is_alive(_pid: u32) -> bool {
        true
    }
}

/// The id of a process that has already exited (a finished `cmd /C exit`).
#[cfg(all(test, windows))]
pub(crate) fn exited_process_id() -> u32 {
    let mut child = std::process::Command::new("cmd").args(["/C", "exit"]).spawn().expect("starting cmd");
    let pid = child.id();
    child.wait().expect("waiting for cmd");
    pid
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn this_process_is_alive_and_an_exited_one_is_not() {
        assert!(is_alive(std::process::id()));
        assert!(!is_alive(exited_process_id()));
    }
}
