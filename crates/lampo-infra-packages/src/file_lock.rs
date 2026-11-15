//! An exclusive lock on a file, shared by every Lampo process (the GUI and the CLI). The
//! operating system releases it when its holder exits, even after a crash, so a lock file is
//! never stale; the file itself stays (deleting it would race with the next locker).

use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;

/// Held until dropped.
#[derive(Debug)]
pub(crate) struct FileLock {
    file: File,
}

impl FileLock {
    /// Locks `path` (created if missing) without waiting: `Ok(None)` while another handle, in
    /// this process or another, holds it.
    pub(crate) fn try_acquire(path: &Path) -> io::Result<Option<Self>> {
        let file = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(path)?;
        Ok(imp::try_lock(&file)?.then_some(Self { file }))
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        imp::unlock(&self.file);
    }
}

#[cfg(windows)]
mod imp {
    use std::fs::File;
    use std::io;
    use std::os::windows::io::AsRawHandle;

    use windows_sys::Win32::Foundation::{ERROR_LOCK_VIOLATION, HANDLE};
    use windows_sys::Win32::Storage::FileSystem::{LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY, LockFileEx, UnlockFile};
    use windows_sys::Win32::System::IO::OVERLAPPED;

    /// The locked range: one byte at offset 0 (the file stays empty; the range may lie past
    /// its end).
    const LOCKED_BYTES: u32 = 1;

    pub(super) fn try_lock(file: &File) -> io::Result<bool> {
        // SAFETY: `file` owns a valid handle for the duration of the call; `overlapped` only
        // carries the offset (0) for a synchronous handle.
        let locked = unsafe {
            let mut overlapped: OVERLAPPED = std::mem::zeroed();
            LockFileEx(file.as_raw_handle() as HANDLE, LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY, 0, LOCKED_BYTES, 0, &mut overlapped)
        };
        if locked != 0 {
            return Ok(true);
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(ERROR_LOCK_VIOLATION as i32) { Ok(false) } else { Err(error) }
    }

    pub(super) fn unlock(file: &File) {
        // SAFETY: as above. Closing the handle would release the lock too, but not necessarily
        // at once.
        unsafe {
            UnlockFile(file.as_raw_handle() as HANDLE, 0, 0, LOCKED_BYTES, 0);
        }
    }
}

/// Lampo runs on Windows; elsewhere only the in-process part of a lock applies.
#[cfg(not(windows))]
mod imp {
    use std::fs::File;
    use std::io;

    pub(super) fn try_lock(_file: &File) -> io::Result<bool> {
        Ok(true)
    }

    pub(super) fn unlock(_file: &File) {}
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn a_second_handle_waits_until_the_first_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("php-8.3.lock");
        let first = FileLock::try_acquire(&path).unwrap().expect("free");
        assert!(FileLock::try_acquire(&path).unwrap().is_none(), "held by another handle");
        drop(first);
        assert!(FileLock::try_acquire(&path).unwrap().is_some(), "released on drop");
    }
}
