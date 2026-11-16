//! Console control events. When the user presses Ctrl+C in a terminal, Windows delivers the
//! event to every process attached to the console: the wrapper (`lampo exec`, a shim) and its
//! child. The wrapper must not die first, or it would orphan the child and lose its exit code;
//! it keeps waiting and forwards the child's exit code instead.

/// Makes this process survive Ctrl+C / Ctrl+Break (other control events, such as closing the
/// console window, keep their default behaviour; `job` disarms the tie before it). Not inherited
/// by child processes: a handler routine, unlike `SetConsoleCtrlHandler(NULL, TRUE)`, is per
/// process.
#[cfg(windows)]
pub fn survive_ctrl_c() {
    use windows_sys::Win32::Foundation::{BOOL, TRUE};
    use windows_sys::Win32::System::Console::{CTRL_BREAK_EVENT, CTRL_C_EVENT, SetConsoleCtrlHandler};

    unsafe extern "system" fn handler(event: u32) -> BOOL {
        BOOL::from(event == CTRL_C_EVENT || event == CTRL_BREAK_EVENT)
    }

    // SAFETY: plain kernel32 call with a valid `extern "system"` function pointer.
    unsafe {
        SetConsoleCtrlHandler(Some(handler), TRUE);
    }
}

#[cfg(not(windows))]
pub fn survive_ctrl_c() {}
