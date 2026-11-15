//! # lampo-infra-fs
//!
//! Filesystem primitives every infrastructure adapter that persists files shares, so that the
//! store (`lampo-infra-store`) and the package registry (`lampo-infra-packages`) write and back
//! up their files the same way without depending on each other.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use lampo_domain::{PortError, PortResult};

/// Prefix of the temp files of in-flight atomic writes, so listings can never mistake them for
/// data files.
pub const TEMP_PREFIX: &str = ".lampo-";
/// Suffix of the temp files of in-flight atomic writes.
pub const TEMP_SUFFIX: &str = ".tmp";

/// Marker between a file name and the timestamp of its backup copy (`shop.toml.corrupt-…`).
/// The copy no longer ends in the original extension, so listings never read it back as data.
pub const CORRUPT_BACKUP_MARKER: &str = ".corrupt-";

/// How many times replacing the target is retried while another process holds it.
const PERSIST_RETRIES: u32 = 40;
/// Base pause between two replacement attempts (grows linearly up to ten times this).
const PERSIST_RETRY_STEP: Duration = Duration::from_millis(5);
/// The pause stops growing after this many attempts.
const PERSIST_RETRY_GROWTH_CAP: u32 = 10;

/// Writes `contents` to `path` atomically and durably, creating parent directories. A concurrent
/// reader sees either the old or the new file, never a partial one; a crash leaves the old file
/// intact, since the data is flushed to disk before the rename.
pub fn write_atomic(path: &Path, contents: &[u8]) -> PortResult<()> {
    let context = || format!("writing {}", path.display());
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent).map_err(|e| PortError::io(context(), e))?;
    let mut tmp = tempfile::Builder::new().prefix(TEMP_PREFIX).suffix(TEMP_SUFFIX).tempfile_in(parent).map_err(|e| PortError::io(context(), e))?;
    tmp.write_all(contents).map_err(|e| PortError::io(context(), e))?;
    tmp.as_file().sync_all().map_err(|e| PortError::io(context(), e))?;
    // Replacing can transiently fail on Windows while a reader or antivirus holds the target.
    let mut pending = tmp;
    let mut attempt = 0;
    loop {
        match pending.persist(path) {
            Ok(_) => return Ok(()),
            Err(e) if attempt < PERSIST_RETRIES => {
                attempt += 1;
                pending = e.file;
                std::thread::sleep(PERSIST_RETRY_STEP * attempt.min(PERSIST_RETRY_GROWTH_CAP));
            }
            Err(e) => return Err(PortError::io(context(), e.error)),
        }
    }
}

/// Where a backup of the unreadable `path` goes: `<name>.corrupt-<unix ms>` next to it.
pub fn corrupt_backup_path(path: &Path) -> PathBuf {
    let file_name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let unix_ms = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    path.with_file_name(format!("{file_name}{CORRUPT_BACKUP_MARKER}{unix_ms}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_replaces_and_leaves_no_temp_files() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("sub").join("f.txt");
        write_atomic(&file, b"one").unwrap();
        write_atomic(&file, b"two").unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"two");
        assert_eq!(std::fs::read_dir(file.parent().unwrap()).unwrap().count(), 1);
    }

    #[test]
    fn backups_sit_next_to_the_file_under_a_name_that_is_no_longer_data() {
        let backup = corrupt_backup_path(&Path::new("packages").join("installed.toml"));
        assert_eq!(backup.parent(), Some(Path::new("packages")));
        let name = backup.file_name().unwrap().to_string_lossy().into_owned();
        let stamp = name.strip_prefix("installed.toml.corrupt-").unwrap();
        assert!(!stamp.is_empty() && stamp.bytes().all(|b| b.is_ascii_digit()), "{name}");
    }
}
