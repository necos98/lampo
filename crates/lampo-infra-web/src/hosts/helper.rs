//! Logic of `lampo-hosts-helper.exe`, the only Lampo code that runs elevated.
//!
//! Security design — the helper is started with administrator rights, so it must not be usable
//! as a general "write a file as admin" tool:
//! * **Fixed target.** Release builds write only `<GetSystemDirectoryW()>\drivers\etc\hosts`
//!   (not taken from environment variables, which the caller controls). `--hosts <path>` exists
//!   only in debug builds (`cfg(debug_assertions)`), for tests against a temp file.
//! * **Narrow request.** The only input is a JSON `HostsBlock` (≤ 1 MiB, ≤ 4096 entries) whose
//!   SHA-256 is on the command line ([`super::request`]), parsed with the domain's validating
//!   types (`Hostname`, `IpAddr`) and accepted only if every address is a loopback address and
//!   every name is under a local TLD (`.test`, `.localhost`, … or the one set in Lampo's
//!   settings): the helper cannot redirect a public name such as `windowsupdate.com`, and no
//!   free text reaches the file (each line is rendered by `HostsBlock::render`).
//! * **Only the block changes.** The new file is the old file with the block spliced in at byte
//!   level ([`super::merge`]); nothing else is touched. Unparseable or ambiguous files (UTF-16,
//!   unterminated block) are refused, not "repaired".
//! * **Atomic write.** A temp file in the `etc` directory (writable by administrators only, so
//!   no other user can plant a link there), flushed, then `ReplaceFileW` (keeps the hosts file's
//!   ACL and attributes) or `MoveFileExW` when the file did not exist, retried with backoff while
//!   another process (DNS client, antivirus, an editor) holds the file open.
//! * **Distinct outcomes.** Exit codes from [`super::helper_exit`]; no output is echoed (the
//!   process has no console and the request content is never printed).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use lampo_domain::hosts::HostsBlock;

use super::helper_exit;
use super::merge::{MergeError, merge_block};
use super::request::{HelperArgs, load_request};

/// Upper bound for the hosts file we are willing to rewrite.
const MAX_HOSTS_BYTES: u64 = 16 * 1024 * 1024;
/// Attempts at swapping the new file in while the hosts file is locked by another process.
const SWAP_ATTEMPTS: u32 = 5;
/// Wait before the second attempt; doubled at each further one (50 + 100 + 200 + 400 ms).
const SWAP_FIRST_BACKOFF: Duration = Duration::from_millis(50);

/// The system hosts file.
pub fn system_hosts_path() -> Option<PathBuf> {
    system_directory().map(|dir| dir.join("drivers").join("etc").join("hosts"))
}

#[cfg(windows)]
fn system_directory() -> Option<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;
    let mut buf = vec![0u16; 512];
    // SAFETY: the buffer pointer and its length in u16 units are passed together.
    let len = unsafe { GetSystemDirectoryW(buf.as_mut_ptr(), buf.len() as u32) } as usize;
    (len > 0 && len < buf.len()).then(|| PathBuf::from(std::ffi::OsString::from_wide(&buf[..len])))
}

#[cfg(not(windows))]
fn system_directory() -> Option<PathBuf> {
    None
}

/// Entire helper run; returns the process exit code.
pub fn run(args: &[String]) -> i32 {
    let Some(args) = HelperArgs::parse(args) else { return helper_exit::BAD_ARGUMENTS };
    let Some(block) = load_request(&args) else { return helper_exit::INVALID_REQUEST };
    let target = match args.hosts_override.or_else(system_hosts_path) {
        Some(t) => t,
        None => return helper_exit::READ_FAILED,
    };
    apply(&target, &block)
}

/// Applies `block` to the file at `target`.
pub fn apply(target: &Path, block: &HostsBlock) -> i32 {
    let original = match read_limited(target) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(_) => return helper_exit::READ_FAILED,
    };
    let updated = match merge_block(&original, block) {
        Ok(bytes) => bytes,
        Err(MergeError::Utf16 | MergeError::UnterminatedBlock) => return helper_exit::UNSUPPORTED_HOSTS_FILE,
    };
    if updated == original {
        return helper_exit::OK;
    }
    match replace_atomically(target, &updated, !original.is_empty() || target.exists()) {
        Ok(()) => helper_exit::OK,
        Err(_) => helper_exit::WRITE_FAILED,
    }
}

fn read_limited(path: &Path) -> std::io::Result<Vec<u8>> {
    let meta = std::fs::metadata(path)?;
    if meta.len() > MAX_HOSTS_BYTES {
        return Err(std::io::Error::other("hosts file too large"));
    }
    std::fs::read(path)
}

fn replace_atomically(target: &Path, contents: &[u8], exists: bool) -> std::io::Result<()> {
    let dir = target.parent().ok_or_else(|| std::io::Error::other("no parent directory"))?;
    let name = target.file_name().ok_or_else(|| std::io::Error::other("no file name"))?.to_string_lossy();
    let temp = dir.join(format!("{name}.lampo-{}.tmp", std::process::id()));
    let result = (|| {
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(&temp)?;
        f.write_all(contents)?;
        f.sync_all()?;
        drop(f);
        retry_while(SWAP_ATTEMPTS, SWAP_FIRST_BACKOFF, is_sharing_violation, || swap_in(target, &temp, exists))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

/// Runs `op` up to `attempts` times while it fails with an error `is_transient` accepts,
/// sleeping `first_backoff`, then twice as long at each further attempt.
fn retry_while<T>(attempts: u32, first_backoff: Duration, is_transient: impl Fn(&std::io::Error) -> bool, mut op: impl FnMut() -> std::io::Result<T>) -> std::io::Result<T> {
    let mut backoff = first_backoff;
    for _ in 1..attempts {
        match op() {
            Err(e) if is_transient(&e) => {
                std::thread::sleep(backoff);
                backoff = backoff.saturating_mul(2);
            }
            result => return result,
        }
    }
    op()
}

/// Another process has the file open without sharing it. `ERROR_UNABLE_TO_REMOVE_REPLACED` is
/// `ReplaceFileW`'s form of the same thing (the replacement is still in place under its own
/// name, so the call can be repeated as is).
#[cfg(windows)]
fn is_sharing_violation(e: &std::io::Error) -> bool {
    use windows_sys::Win32::Foundation::{ERROR_LOCK_VIOLATION, ERROR_SHARING_VIOLATION, ERROR_UNABLE_TO_REMOVE_REPLACED};
    e.raw_os_error().is_some_and(|code| [ERROR_SHARING_VIOLATION, ERROR_LOCK_VIOLATION, ERROR_UNABLE_TO_REMOVE_REPLACED].map(|c| c as i32).contains(&code))
}

#[cfg(not(windows))]
fn is_sharing_violation(_e: &std::io::Error) -> bool {
    false
}

#[cfg(windows)]
fn swap_in(target: &Path, temp: &Path, exists: bool) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW, REPLACEFILE_IGNORE_MERGE_ERRORS, ReplaceFileW};
    let wide = |p: &Path| p.as_os_str().encode_wide().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let (t, r) = (wide(target), wide(temp));
    // SAFETY: both strings are NUL-terminated and live across the calls; optional arguments null.
    let ok = unsafe {
        if exists {
            ReplaceFileW(t.as_ptr(), r.as_ptr(), std::ptr::null(), REPLACEFILE_IGNORE_MERGE_ERRORS, std::ptr::null(), std::ptr::null())
        } else {
            MoveFileExW(r.as_ptr(), t.as_ptr(), MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)
        }
    };
    if ok == 0 { Err(std::io::Error::last_os_error()) } else { Ok(()) }
}

#[cfg(not(windows))]
fn swap_in(target: &Path, temp: &Path, _exists: bool) -> std::io::Result<()> {
    std::fs::rename(temp, target)
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::net::{IpAddr, Ipv4Addr};

    use lampo_domain::hostname::Hostname;
    use lampo_domain::hosts::HostsEntry;

    use super::*;

    fn entry(ip: IpAddr, host: &str) -> HostsEntry {
        HostsEntry { ip, hostname: Hostname::new(host).unwrap() }
    }

    fn locked() -> std::io::Error {
        std::io::Error::new(std::io::ErrorKind::ResourceBusy, "locked")
    }

    fn is_busy(e: &std::io::Error) -> bool {
        e.kind() == std::io::ErrorKind::ResourceBusy
    }

    #[test]
    fn system_hosts_path_is_under_system32() {
        if cfg!(windows) {
            let p = system_hosts_path().unwrap();
            assert!(p.to_string_lossy().to_ascii_lowercase().ends_with(r"system32\drivers\etc\hosts"));
        }
    }

    #[test]
    fn retries_transient_failures_until_success() {
        let calls = Cell::new(0);
        let result = retry_while(SWAP_ATTEMPTS, Duration::ZERO, is_busy, || {
            calls.set(calls.get() + 1);
            if calls.get() < 3 { Err(locked()) } else { Ok("swapped") }
        });
        assert_eq!(result.unwrap(), "swapped");
        assert_eq!(calls.get(), 3);
    }

    #[test]
    fn gives_up_after_the_last_attempt() {
        let calls = Cell::new(0);
        let result: std::io::Result<()> = retry_while(SWAP_ATTEMPTS, Duration::ZERO, is_busy, || {
            calls.set(calls.get() + 1);
            Err(locked())
        });
        assert!(is_busy(&result.unwrap_err()));
        assert_eq!(calls.get(), SWAP_ATTEMPTS);
    }

    #[test]
    fn other_errors_are_not_retried() {
        let calls = Cell::new(0);
        let result: std::io::Result<()> = retry_while(SWAP_ATTEMPTS, Duration::ZERO, is_busy, || {
            calls.set(calls.get() + 1);
            Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
        });
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::PermissionDenied);
        assert_eq!(calls.get(), 1);
    }

    #[cfg(windows)]
    #[test]
    fn sharing_violations_are_transient() {
        use windows_sys::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_SHARING_VIOLATION, ERROR_UNABLE_TO_REMOVE_REPLACED};
        assert!(is_sharing_violation(&std::io::Error::from_raw_os_error(ERROR_SHARING_VIOLATION as i32)));
        assert!(is_sharing_violation(&std::io::Error::from_raw_os_error(ERROR_UNABLE_TO_REMOVE_REPLACED as i32)));
        assert!(!is_sharing_violation(&std::io::Error::from_raw_os_error(ERROR_ACCESS_DENIED as i32)));
    }

    #[test]
    fn apply_writes_atomically_and_skips_no_ops() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("hosts");
        std::fs::write(&target, b"127.0.0.1 localhost\r\n").unwrap();
        let block = HostsBlock::new([entry(IpAddr::V4(Ipv4Addr::LOCALHOST), "shop.test")]);
        assert_eq!(apply(&target, &block), helper_exit::OK);
        let written = std::fs::read(&target).unwrap();
        assert!(written.starts_with(b"127.0.0.1 localhost\r\n"));
        assert!(String::from_utf8_lossy(&written).contains("127.0.0.1\tshop.test"));
        let modified = std::fs::metadata(&target).unwrap().modified().unwrap();
        assert_eq!(apply(&target, &block), helper_exit::OK);
        assert_eq!(std::fs::metadata(&target).unwrap().modified().unwrap(), modified, "unchanged block: no write");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1, "no temp file left behind");
        // A missing file is created.
        let fresh = dir.path().join("sub-hosts");
        assert_eq!(apply(&fresh, &block), helper_exit::OK);
        assert!(fresh.is_file());
    }

    #[test]
    fn apply_refuses_ambiguous_files() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("hosts");
        std::fs::write(&target, format!("{}\r\n127.0.0.1 a.test\r\n", lampo_domain::hosts::BEGIN_MARKER)).unwrap();
        let block = HostsBlock::new([entry(IpAddr::V4(Ipv4Addr::LOCALHOST), "shop.test")]);
        assert_eq!(apply(&target, &block), helper_exit::UNSUPPORTED_HOSTS_FILE);
    }
}
