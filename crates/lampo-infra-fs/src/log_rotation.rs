//! Size-bounded log files. A log is kept below [`MAX_LOG_BYTES`] by moving it to `<name>.1`,
//! the naming Apache's `rotatelogs -n` uses too (`<name>`, `<name>.1`, `<name>.2`, then
//! `<name>` again). The writers rotate; readers find the file being written with
//! [`current_log_file`], whichever member of the set that is.

use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Size past which a log is rotated (`rotatelogs` takes it in MiB, see [`MAX_LOG_MIB`]).
pub const MAX_LOG_BYTES: u64 = MAX_LOG_MIB * 1024 * 1024;
/// [`MAX_LOG_BYTES`] in MiB, the unit of the `rotatelogs` size argument (`10M`).
pub const MAX_LOG_MIB: u64 = 10;
/// Files in a rotation set, the current one included (`rotatelogs -n 3`). Lampo's own
/// rotation keeps one previous file (`<name>.1`), which falls within the set.
pub const LOG_SET_SIZE: u32 = 3;

/// `<path>.<n>`: the `n`-th member of `path`'s rotation set (`n` ≥ 1).
pub fn rotated_path(path: &Path, n: u32) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(format!(".{n}"));
    PathBuf::from(name)
}

/// Moves `path` to `<path>.1` (replacing the previous one) when it is larger than `max`.
/// `Ok(true)` when rotated; a missing file is not an error. A writer holding the file open
/// keeps writing to `<path>.1`, so only logs whose writer reopens them per write (PHP) or
/// that are not open yet (before a service starts) are rotated this way.
pub fn rotate_if_large(path: &Path, max: u64) -> io::Result<bool> {
    match std::fs::metadata(path) {
        Ok(meta) if meta.len() > max => std::fs::rename(path, rotated_path(path, 1)).map(|()| true),
        Ok(_) => Ok(false),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

/// The member of `path`'s rotation set written last: `path` itself unless a numbered sibling
/// is newer (after `rotatelogs` moved on to `<path>.1`, or right after a rotation, before the
/// writer created `path` again). `path` when none exists.
pub fn current_log_file(path: &Path) -> PathBuf {
    let members = std::iter::once(path.to_path_buf()).chain((1..LOG_SET_SIZE).map(|n| rotated_path(path, n)));
    let mut newest: Option<(SystemTime, PathBuf)> = None;
    for member in members {
        let Ok(modified) = std::fs::metadata(&member).and_then(|m| m.modified()) else { continue };
        // Strictly newer: on equal times the earlier member (the base file) wins.
        if newest.as_ref().is_none_or(|(time, _)| modified > *time) {
            newest = Some((modified, member));
        }
    }
    newest.map_or_else(|| path.to_path_buf(), |(_, member)| member)
}

#[cfg(test)]
mod tests {
    use std::fs::{File, FileTimes};
    use std::time::Duration;

    use super::*;

    fn write(path: &Path, bytes: usize, age_secs: u64) {
        let file = File::create(path).unwrap();
        file.set_len(bytes as u64).unwrap();
        let modified = SystemTime::now() - Duration::from_secs(age_secs);
        file.set_times(FileTimes::new().set_modified(modified)).unwrap();
    }

    #[test]
    fn rotated_names_follow_rotatelogs() {
        assert_eq!(rotated_path(Path::new(r"C:\logs\error.log"), 2), PathBuf::from(r"C:\logs\error.log.2"));
    }

    #[test]
    fn rotates_only_files_over_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("php_errors.log");
        assert!(!rotate_if_large(&log, 10).unwrap(), "a missing file is left alone");
        write(&log, 10, 0);
        assert!(!rotate_if_large(&log, 10).unwrap());
        assert!(log.is_file());
        write(&log, 11, 0);
        write(&rotated_path(&log, 1), 1, 0);
        assert!(rotate_if_large(&log, 10).unwrap());
        assert!(!log.exists());
        assert_eq!(std::fs::metadata(rotated_path(&log, 1)).unwrap().len(), 11, "the previous file is replaced");
    }

    #[test]
    fn the_current_file_is_the_newest_member_of_the_set() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("access.log");
        assert_eq!(current_log_file(&log), log, "nothing written yet");
        write(&log, 1, 300);
        assert_eq!(current_log_file(&log), log);
        // rotatelogs moved on to `.1`, then to `.2`.
        write(&rotated_path(&log, 1), 1, 200);
        assert_eq!(current_log_file(&log), rotated_path(&log, 1));
        write(&rotated_path(&log, 2), 1, 100);
        assert_eq!(current_log_file(&log), rotated_path(&log, 2));
        // ...and back to the base file.
        write(&log, 1, 0);
        assert_eq!(current_log_file(&log), log);
        // Members beyond the set are never picked.
        write(&rotated_path(&log, LOG_SET_SIZE), 1, 0);
        assert_eq!(current_log_file(&log), log);
    }

    #[test]
    fn right_after_a_rotation_the_previous_file_is_current() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("output.log");
        write(&log, 20, 0);
        rotate_if_large(&log, 10).unwrap();
        assert_eq!(current_log_file(&log), rotated_path(&log, 1));
    }
}
