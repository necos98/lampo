//! Logic of `lampo-hosts-helper.exe`, the only Lampo code that runs elevated.
//!
//! Security design — the helper is started with administrator rights, so it must not be usable
//! as a general "write a file as admin" tool:
//! * **Fixed target.** Release builds write only `<GetSystemDirectoryW()>\drivers\etc\hosts`
//!   (not taken from environment variables, which the caller controls). `--hosts <path>` exists
//!   only in debug builds (`cfg(debug_assertions)`), for tests against a temp file.
//! * **Narrow request.** The only input is a JSON `HostsBlock` (≤ 1 MiB, ≤ 4096 entries), parsed
//!   with the domain's validating types (`Hostname`, `IpAddr`) and accepted only if every address
//!   is a loopback address: the helper cannot be used to redirect a name elsewhere, and no
//!   free text reaches the file (each line is rendered by `HostsBlock::render`).
//! * **Only the block changes.** The new file is the old file with the block spliced in at byte
//!   level ([`super::merge`]); nothing else is touched. Unparseable or ambiguous files (UTF-16,
//!   unterminated block) are refused, not "repaired".
//! * **Atomic write.** A temp file in the `etc` directory (writable by administrators only, so
//!   no other user can plant a link there), flushed, then `ReplaceFileW` (keeps the hosts file's
//!   ACL and attributes) or `MoveFileExW` when the file did not exist.
//! * **Distinct outcomes.** Exit codes from [`super::helper_exit`]; no output is echoed (the
//!   process has no console and the request content is never printed).

use std::io::Write;
use std::net::IpAddr;
use std::path::{Path, PathBuf};

use lampo_domain::hosts::HostsBlock;

use super::helper_exit;
use super::merge::{MergeError, merge_block};

/// Upper bound for the request file.
pub const MAX_REQUEST_BYTES: u64 = 1024 * 1024;
/// Upper bound for the number of block entries.
pub const MAX_ENTRIES: usize = 4096;
/// Upper bound for the hosts file we are willing to rewrite.
const MAX_HOSTS_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub struct HelperArgs {
    pub block_file: PathBuf,
    /// Debug builds only.
    pub hosts_override: Option<PathBuf>,
}

/// Parses `--block <file.json> [--hosts <path>]` (the latter only in debug builds).
pub fn parse_args(args: &[String]) -> Option<HelperArgs> {
    let mut block_file = None;
    let mut hosts_override = None;
    let mut it = args.iter();
    while let Some(flag) = it.next() {
        let value = it.next()?;
        match flag.as_str() {
            "--block" if block_file.is_none() => block_file = Some(PathBuf::from(value)),
            "--hosts" if cfg!(debug_assertions) && hosts_override.is_none() => hosts_override = Some(PathBuf::from(value)),
            _ => return None,
        }
    }
    Some(HelperArgs { block_file: block_file?, hosts_override })
}

/// Reads and validates the request. `None` if it is not an acceptable block.
pub fn load_request(path: &Path) -> Option<HostsBlock> {
    let file = std::fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() > MAX_REQUEST_BYTES {
        return None;
    }
    let block: HostsBlock = serde_json::from_reader(std::io::BufReader::new(file)).ok()?;
    validate_request(&block).then_some(block)
}

/// Only loopback addresses, bounded size.
pub fn validate_request(block: &HostsBlock) -> bool {
    block.entries().count() <= MAX_ENTRIES && block.entries().all(|e| is_loopback(e.ip))
}

fn is_loopback(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback(),
        IpAddr::V6(v6) => v6.is_loopback() || v6.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback()),
    }
}

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
    let Some(args) = parse_args(args) else { return helper_exit::BAD_ARGUMENTS };
    let Some(block) = load_request(&args.block_file) else { return helper_exit::INVALID_REQUEST };
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
        swap_in(target, &temp, exists)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
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
    use std::net::{Ipv4Addr, Ipv6Addr};

    use lampo_domain::hostname::Hostname;
    use lampo_domain::hosts::HostsEntry;

    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    fn entry(ip: IpAddr, host: &str) -> HostsEntry {
        HostsEntry { ip, hostname: Hostname::new(host).unwrap() }
    }

    #[test]
    fn argument_parsing_is_strict() {
        assert_eq!(parse_args(&s(&["--block", "a.json"])), Some(HelperArgs { block_file: "a.json".into(), hosts_override: None }));
        assert_eq!(parse_args(&s(&[])), None);
        assert_eq!(parse_args(&s(&["--block"])), None);
        assert_eq!(parse_args(&s(&["--block", "a", "--block", "b"])), None);
        assert_eq!(parse_args(&s(&["--block", "a", "--other", "b"])), None);
        assert_eq!(parse_args(&s(&["--hosts", "x"])), None, "a block is mandatory");
        // Tests run in debug builds, where the override exists.
        assert_eq!(parse_args(&s(&["--block", "a", "--hosts", "h"])).unwrap().hosts_override, Some(PathBuf::from("h")));
    }

    #[test]
    fn only_loopback_entries_are_accepted() {
        let ok = HostsBlock::new([entry(IpAddr::V4(Ipv4Addr::LOCALHOST), "a.test"), entry(IpAddr::V6(Ipv6Addr::LOCALHOST), "a.test"), entry("127.0.0.2".parse().unwrap(), "b.test")]);
        assert!(validate_request(&ok));
        let bad = HostsBlock::new([entry("10.0.0.1".parse().unwrap(), "bank.test")]);
        assert!(!validate_request(&bad));
        let bad = HostsBlock::new([entry("::ffff:8.8.8.8".parse().unwrap(), "bank.test")]);
        assert!(!validate_request(&bad));
    }

    #[test]
    fn request_files_are_validated() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("req.json");
        std::fs::write(&file, r#"{"entries":[{"ip":"127.0.0.1","hostname":"shop.test"}]}"#).unwrap();
        assert_eq!(load_request(&file).unwrap().entries().count(), 1);
        for bad in [
            r#"{"entries":[{"ip":"127.0.0.1","hostname":"shop.test\r\n1.2.3.4 evil.test"}]}"#,
            r#"{"entries":[{"ip":"1.2.3.4","hostname":"shop.test"}]}"#,
            r#"{"entries":[{"ip":"127.0.0.1","hostname":"x"}]}"#,
            "not json",
        ] {
            std::fs::write(&file, bad).unwrap();
            assert!(load_request(&file).is_none(), "{bad}");
        }
        std::fs::write(&file, vec![b' '; (MAX_REQUEST_BYTES + 1) as usize]).unwrap();
        assert!(load_request(&file).is_none());
        assert!(load_request(&dir.path().join("missing.json")).is_none());
    }

    #[test]
    fn system_hosts_path_is_under_system32() {
        if cfg!(windows) {
            let p = system_hosts_path().unwrap();
            assert!(p.to_string_lossy().to_ascii_lowercase().ends_with(r"system32\drivers\etc\hosts"));
        }
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
