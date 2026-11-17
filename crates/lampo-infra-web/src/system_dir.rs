//! The Windows system folder (`C:\Windows\System32` on a default install), from
//! `GetSystemDirectoryW`: never from environment variables, which the caller of an elevated
//! process controls.

use std::path::PathBuf;

#[cfg(windows)]
pub fn system_directory() -> Option<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;
    let mut buf = vec![0u16; 512];
    // SAFETY: the buffer pointer and its length in u16 units are passed together.
    let len = unsafe { GetSystemDirectoryW(buf.as_mut_ptr(), buf.len() as u32) } as usize;
    (len > 0 && len < buf.len()).then(|| PathBuf::from(std::ffi::OsString::from_wide(&buf[..len])))
}

#[cfg(not(windows))]
pub fn system_directory() -> Option<PathBuf> {
    None
}
