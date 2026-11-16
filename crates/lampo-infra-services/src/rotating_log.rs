//! The captured output of a supervised service (`<log_dir>\output.log`), kept below
//! [`MAX_LOG_BYTES`] while the service runs: Lampo is its only writer, so it closes the file,
//! moves it to `output.log.1` and starts a new one whenever the next line would cross the limit.
//! Logs other programs write are rotated with [`SizeLogRotator`].

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use lampo_domain::ports::LogRotator;
use lampo_domain::{PortError, PortResult};
use lampo_infra_fs::log_rotation::{MAX_LOG_BYTES, rotate_if_large, rotated_path};

/// Rotates a log nobody holds open yet (before its service starts). Best effort: a log that
/// cannot be moved aside keeps growing, which is no reason to refuse the start.
pub(crate) fn rotate_closed_log(path: &Path) {
    rotate_or_warn(path, MAX_LOG_BYTES);
}

fn rotate_or_warn(path: &Path, max: u64) {
    if let Err(e) = rotate_if_large(path, max) {
        warn_not_rotated(path, &e);
    }
}

fn warn_not_rotated(path: &Path, error: &io::Error) {
    tracing::warn!("cannot rotate {}: {error}", path.display());
}

/// [`LogRotator`] with Lampo's size limit, for logs written by others that reopen them per entry.
#[derive(Debug, Default, Clone, Copy)]
pub struct SizeLogRotator;

impl LogRotator for SizeLogRotator {
    fn rotate_if_large(&self, file: &Path) -> PortResult<()> {
        rotate_if_large(file, MAX_LOG_BYTES).map(|_| ()).map_err(|e| PortError::io(format!("rotating {}", file.display()), e))
    }
}

/// An append-only log file rotated by size as it is written.
pub(crate) struct RotatingLog {
    path: PathBuf,
    file: File,
    /// Size of the current file as this writer knows it; reset by every rotation attempt.
    written: u64,
    max: u64,
}

impl RotatingLog {
    /// Opens `path` for appending, rotating it first when it is already over the limit.
    pub(crate) fn open(path: &Path) -> PortResult<Self> {
        Self::with_limit(path, MAX_LOG_BYTES)
    }

    fn with_limit(path: &Path, max: u64) -> PortResult<Self> {
        rotate_or_warn(path, max);
        let file = append(path).map_err(|e| PortError::io(format!("opening {}", path.display()), e))?;
        let written = file.metadata().map(|m| m.len()).unwrap_or(0);
        Ok(Self { path: path.to_path_buf(), file, written, max })
    }

    /// Appends `line` (already terminated), first starting a new file when it would push the
    /// current one past the limit. After a failed rotation (logged) the line still goes to the
    /// current file, and the rotation is retried only after another `max` bytes, not on every
    /// line. The error is the failure to write `line`.
    pub(crate) fn write(&mut self, line: &[u8]) -> io::Result<()> {
        let len = line.len() as u64;
        if self.written > 0 && self.written + len > self.max {
            self.written = 0;
            if let Err(e) = self.rotate() {
                warn_not_rotated(&self.path, &e);
            }
        }
        self.file.write_all(line)?;
        self.written += len;
        Ok(())
    }

    fn rotate(&mut self) -> io::Result<()> {
        // The handle is only replaced once the new file is open: after a failure the log goes on
        // in the file it was writing.
        std::fs::rename(&self.path, rotated_path(&self.path, 1))?;
        self.file = append(&self.path)?;
        Ok(())
    }
}

fn append(path: &Path) -> io::Result<File> {
    OpenOptions::new().create(true).append(true).open(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap()
    }

    #[test]
    fn starts_a_new_file_when_the_next_line_would_cross_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("output.log");
        let mut log = RotatingLog::with_limit(&path, 11).unwrap();
        log.write(b"12345\n").unwrap();
        log.write(b"1234\n").unwrap();
        assert_eq!(read(&path), "12345\n1234\n");
        log.write(b"next\n").unwrap();
        assert_eq!(read(&path), "next\n");
        assert_eq!(read(&rotated_path(&path, 1)), "12345\n1234\n");
        log.write(b"again\n").unwrap();
        assert_eq!(read(&path), "next\nagain\n");
        log.write(b"third\n").unwrap();
        assert_eq!(read(&path), "third\n");
        assert_eq!(read(&rotated_path(&path, 1)), "next\nagain\n", "one previous file is kept");
    }

    #[test]
    fn a_line_longer_than_the_limit_still_gets_written() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("output.log");
        let mut log = RotatingLog::with_limit(&path, 4).unwrap();
        log.write(b"a very long line\n").unwrap();
        assert_eq!(read(&path), "a very long line\n");
        assert!(!rotated_path(&path, 1).exists(), "an empty file is never rotated");
    }

    #[test]
    fn an_oversized_log_is_rotated_when_opened_and_appended_to_otherwise() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("output.log");
        std::fs::write(&path, "old\n").unwrap();
        let mut log = RotatingLog::with_limit(&path, 10).unwrap();
        log.write(b"new\n").unwrap();
        assert_eq!(read(&path), "old\nnew\n", "under the limit: appended");
        drop(log);
        let mut log = RotatingLog::with_limit(&path, 4).unwrap();
        log.write(b"x\n").unwrap();
        assert_eq!(read(&path), "x\n");
        assert_eq!(read(&rotated_path(&path, 1)), "old\nnew\n");
    }

    #[test]
    fn the_rotator_moves_aside_only_logs_over_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("php_errors.log");
        SizeLogRotator.rotate_if_large(&path).unwrap();
        std::fs::File::create(&path).unwrap().set_len(MAX_LOG_BYTES).unwrap();
        SizeLogRotator.rotate_if_large(&path).unwrap();
        assert!(path.is_file());
        std::fs::File::options().write(true).open(&path).unwrap().set_len(MAX_LOG_BYTES + 1).unwrap();
        SizeLogRotator.rotate_if_large(&path).unwrap();
        assert!(!path.exists());
        assert!(rotated_path(&path, 1).is_file());
    }

    #[test]
    fn rotate_closed_log_tolerates_a_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        rotate_closed_log(&dir.path().join("error.log"));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}
