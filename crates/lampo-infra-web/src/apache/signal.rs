//! Graceful control of a console (non-service) httpd on Windows.
//!
//! `httpd -k restart|shutdown` only talks to the Windows service manager. A console httpd
//! instead waits on two named events created by its parent process, `ap<parent-pid>_restart` and
//! `ap<parent-pid>_shutdown` (see `server/mpm/winnt/mpm_winnt.c`): setting the first is exactly
//! what Ctrl+Break does (graceful restart: re-read the configuration, let running requests
//! finish), setting the second is Ctrl+C (clean shutdown, children and php-cgi included).

use std::io;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpdSignal {
    Restart,
    Shutdown,
}

impl HttpdSignal {
    /// Name of the event the httpd parent with `pid` waits on.
    pub fn event_name(self, pid: u32) -> String {
        match self {
            Self::Restart => format!("ap{pid}_restart"),
            Self::Shutdown => format!("ap{pid}_shutdown"),
        }
    }
}

/// Sends a control signal to a running httpd parent process.
pub trait HttpdSignals: Send + Sync {
    fn send(&self, pid: u32, signal: HttpdSignal) -> io::Result<()>;
}

/// Sets the named event (same user and session as the httpd we started).
#[derive(Debug, Default, Clone, Copy)]
pub struct NamedEventSignals;

#[cfg(windows)]
impl HttpdSignals for NamedEventSignals {
    fn send(&self, pid: u32, signal: HttpdSignal) -> io::Result<()> {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::{EVENT_MODIFY_STATE, OpenEventW, SetEvent};

        let name: Vec<u16> = signal.event_name(pid).encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: `name` is a NUL-terminated UTF-16 string that outlives the call; the handle is
        // checked for null and closed exactly once.
        unsafe {
            let event = OpenEventW(EVENT_MODIFY_STATE, 0, name.as_ptr());
            if event.is_null() {
                return Err(io::Error::last_os_error());
            }
            let ok = SetEvent(event);
            let err = io::Error::last_os_error();
            CloseHandle(event);
            if ok == 0 { Err(err) } else { Ok(()) }
        }
    }
}

#[cfg(not(windows))]
impl HttpdSignals for NamedEventSignals {
    fn send(&self, _pid: u32, _signal: HttpdSignal) -> io::Result<()> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "httpd named events exist only on Windows"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_names_match_mpm_winnt() {
        assert_eq!(HttpdSignal::Restart.event_name(4242), "ap4242_restart");
        assert_eq!(HttpdSignal::Shutdown.event_name(7), "ap7_shutdown");
    }

    #[cfg(windows)]
    #[test]
    fn missing_event_is_an_error() {
        assert!(NamedEventSignals.send(u32::MAX - 1, HttpdSignal::Restart).is_err());
    }
}
