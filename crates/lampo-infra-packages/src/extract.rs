//! Zip extraction that is all-or-nothing: the archive is extracted into a staging directory
//! next to the destination, which then replaces the destination in one rename (see
//! [`crate::slot`]). A failure or cancellation at any point removes the staging directory: the
//! destination is either the complete old tree or the complete new one.
//!
//! Every entry is checked before anything is written:
//! * its name must be a safe relative Windows path ([`crate::entry_name`]: no zip-slip, no
//!   reserved device names, no trailing dot or space); symlink entries are refused;
//! * two files whose names differ only by case are refused: Windows would write one over the
//!   other;
//! * the archive may not expand beyond [`expansion_limit`] (a zip bomb), checked against the
//!   sizes it declares and again against the bytes actually decompressed.

use std::collections::HashSet;
use std::fs;
use std::io::{Read, Seek, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use lampo_domain::ports::{ArchiveExtractor, ProgressSink};
use lampo_domain::progress::{CancelFlag, Progress, ProgressStage};
use lampo_domain::{PortError, PortResult};

use crate::entry_name::{collision_key, safe_relative_path};
use crate::slot::{place_file_blocking, prepare_staging, replace_dir};

/// How many times its own size an archive may expand to. Package zips expand 2-4x (PHP 30 MB →
/// ~100 MB, MySQL 350 MB → ~1.3 GB), while zip bombs reach ratios of 1000x and more.
const EXPANSION_FACTOR: u64 = 6;

/// The expansion limit is never below this (2 GiB): every real package fits with room to spare,
/// so a small archive with a high but legitimate ratio is never refused.
const MIN_EXPANSION_LIMIT_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// The most bytes an archive of `archive_len` bytes may expand to: `max(len × 6, 2 GiB)`. The
/// archive length stands in for the catalog size: the downloader refuses anything larger than
/// 110% of it, and the length is known here without widening the port.
pub(crate) fn expansion_limit(archive_len: u64) -> u64 {
    archive_len.saturating_mul(EXPANSION_FACTOR).max(MIN_EXPANSION_LIMIT_BYTES)
}

/// Extracts into `<destination>.staging-*` then renames (no half-installed packages); rejects
/// unsafe entries and zip bombs. Places single-file payloads the same way.
#[derive(Debug, Default, Clone, Copy)]
pub struct ZipExtractor;

#[async_trait]
impl ArchiveExtractor for ZipExtractor {
    async fn extract_zip(&self, archive: &Path, destination: &Path, strip_prefix: Option<&str>, progress: &dyn ProgressSink, cancel: &CancelFlag) -> PortResult<()> {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Progress>();
        let archive = archive.to_path_buf();
        let destination = destination.to_path_buf();
        let strip_prefix = strip_prefix.map(str::to_string);
        let cancel = cancel.clone();
        let job = tokio::task::spawn_blocking(move || extract_blocking(&archive, &destination, strip_prefix.as_deref(), &|p| drop(tx.send(p)), &cancel));
        // The sender lives in the blocking job: the channel closes when it ends.
        while let Some(p) = rx.recv().await {
            progress.report(p);
        }
        job.await.map_err(|e| PortError::io("extracting an archive", e))?
    }

    async fn place_file(&self, file: &Path, destination: &Path, file_name: &str, cancel: &CancelFlag) -> PortResult<()> {
        let (file, destination, file_name, cancel) = (file.to_path_buf(), destination.to_path_buf(), file_name.to_string(), cancel.clone());
        tokio::task::spawn_blocking(move || place_file_blocking(&file, &destination, &file_name, &cancel)).await.map_err(|e| PortError::io("placing a package file", e))?
    }
}

/// Synchronous extraction with the same guarantees (also used by tests).
pub fn extract_blocking(archive: &Path, destination: &Path, strip_prefix: Option<&str>, report: &dyn Fn(Progress), cancel: &CancelFlag) -> PortResult<()> {
    extract_with_limit(archive, destination, strip_prefix, expansion_limit, report, cancel)
}

/// [`extract_blocking`] with the expansion limit computed by `limit` from the archive length.
fn extract_with_limit(archive: &Path, destination: &Path, strip_prefix: Option<&str>, limit: fn(u64) -> u64, report: &dyn Fn(Progress), cancel: &CancelFlag) -> PortResult<()> {
    let staging = prepare_staging(destination)?;
    let result = extract_into(archive, &staging, strip_prefix, limit, report, cancel).and_then(|()| {
        cancel.check()?;
        replace_dir(&staging, destination)
    });
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

/// One entry to extract: its index in the archive and its path under the destination.
struct PlannedEntry {
    index: usize,
    relative: PathBuf,
    is_dir: bool,
}

fn extract_into(archive: &Path, staging: &Path, strip_prefix: Option<&str>, limit: fn(u64) -> u64, report: &dyn Fn(Progress), cancel: &CancelFlag) -> PortResult<()> {
    let source = archive.display().to_string();
    let bad = |m: String| PortError::invalid_data(source.clone(), m);
    let file = fs::File::open(archive).map_err(|e| PortError::io(format!("opening {source}"), e))?;
    let limit = limit(file.metadata().map_err(|e| PortError::io(format!("reading {source}"), e))?.len());
    let mut zip = zip::ZipArchive::new(std::io::BufReader::new(file)).map_err(|e| bad(format!("not a valid zip archive: {e}")))?;
    let (plan, total) = plan_entries(&mut zip, strip_prefix, &bad)?;
    if total > limit {
        return Err(bad(format!("the archive expands to {total} bytes, more than the {limit} allowed")));
    }

    let mut done = 0u64;
    let mut last_report = Instant::now() - Duration::from_secs(1);
    let mut buf = vec![0u8; 256 * 1024];
    for PlannedEntry { index, relative, is_dir } in plan {
        cancel.check()?;
        let target = staging.join(&relative);
        debug_assert!(target.starts_with(staging));
        if is_dir {
            fs::create_dir_all(&target).map_err(|e| PortError::io(format!("creating {}", target.display()), e))?;
            continue;
        }
        if let Some(dir) = target.parent() {
            fs::create_dir_all(dir).map_err(|e| PortError::io(format!("creating {}", dir.display()), e))?;
        }
        let mut entry = zip.by_index(index).map_err(|e| bad(e.to_string()))?;
        let io = |e: std::io::Error| PortError::io(format!("extracting {}", relative.display()), e);
        let mut out = fs::File::create(&target).map_err(io)?;
        loop {
            let n = entry.read(&mut buf).map_err(|e| bad(format!("corrupt entry `{}`: {e}", relative.display())))?;
            if n == 0 {
                break;
            }
            // Declared sizes can lie: count what is really decompressed.
            if done + n as u64 > limit {
                return Err(bad(format!("the archive expands to more than the {limit} bytes allowed")));
            }
            out.write_all(&buf[..n]).map_err(io)?;
            done += n as u64;
            if last_report.elapsed() >= Duration::from_millis(100) {
                cancel.check()?;
                report(Progress::new(ProgressStage::Extracting, done, Some(total)));
                last_report = Instant::now();
            }
        }
    }
    report(Progress::new(ProgressStage::Extracting, done, Some(total)));
    Ok(())
}

/// Validates every entry before anything is written; returns what to extract and the total size
/// the archive declares for it.
fn plan_entries<R: Read + Seek>(zip: &mut zip::ZipArchive<R>, strip_prefix: Option<&str>, bad: &dyn Fn(String) -> PortError) -> PortResult<(Vec<PlannedEntry>, u64)> {
    let mut plan = Vec::with_capacity(zip.len());
    let mut files = HashSet::new();
    let mut total = 0u64;
    for index in 0..zip.len() {
        let entry = zip.by_index_raw(index).map_err(|e| bad(e.to_string()))?;
        let raw_name = entry.name().to_string();
        if entry.is_symlink() {
            return Err(bad(format!("symbolic link entries are not allowed: `{raw_name}`")));
        }
        let relative = safe_relative_path(&raw_name).ok_or_else(|| bad(format!("unsafe path in archive: `{raw_name}`")))?;
        let Some(relative) = strip(&relative, strip_prefix) else { continue };
        if relative.as_os_str().is_empty() {
            continue;
        }
        if !entry.is_dir() {
            if !files.insert(collision_key(&relative)) {
                return Err(bad(format!("the archive has two files named `{raw_name}` (names are case-insensitive on Windows)")));
            }
            total = total.saturating_add(entry.size());
        }
        plan.push(PlannedEntry { index, relative, is_dir: entry.is_dir() });
    }
    if let Some(prefix) = strip_prefix {
        if plan.is_empty() {
            return Err(bad(format!("the archive has no `{prefix}/` folder")));
        }
    }
    Ok((plan, total))
}

/// Removes the `prefix` top-level folder; `None` = entry outside it (skipped, e.g. the
/// `ReadMe.txt` Apache Lounge ships next to `Apache24/`).
fn strip(path: &Path, prefix: Option<&str>) -> Option<PathBuf> {
    let Some(prefix) = prefix else { return Some(path.to_path_buf()) };
    let mut comps = path.components();
    match comps.next() {
        Some(Component::Normal(first)) if first.to_string_lossy().eq_ignore_ascii_case(prefix.trim_matches('/')) => Some(comps.as_path().to_path_buf()),
        _ => None,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::io::Write;

    use super::*;

    /// Builds a zip in memory: `(name, Some(content))` = file, `(name, None)` = directory.
    pub(crate) fn make_zip(path: &Path, entries: &[(&str, Option<&[u8]>)]) {
        let file = fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, content) in entries {
            match content {
                Some(c) => {
                    zip.start_file(*name, opts).unwrap();
                    zip.write_all(c).unwrap();
                }
                None => zip.add_directory(*name, opts).unwrap(),
            }
        }
        zip.finish().unwrap();
    }

    fn run(archive: &Path, dest: &Path, prefix: Option<&str>) -> PortResult<()> {
        extract_blocking(archive, dest, prefix, &|_| {}, &CancelFlag::new())
    }

    /// Expects `entries` to be refused as invalid data, with nothing left next to the destination.
    fn assert_refused(entries: &[(&str, Option<&[u8]>)]) {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("a.zip");
        make_zip(&archive, entries);
        let dest = dir.path().join("inner").join("out");
        let err = run(&archive, &dest, None).unwrap_err();
        assert!(matches!(err, PortError::InvalidData { .. }), "{entries:?}: {err:?}");
        assert_eq!(fs::read_dir(dir.path().join("inner")).unwrap().count(), 0, "{entries:?}: nothing written, staging cleaned");
    }

    #[test]
    fn extracts_with_strip_prefix_and_skips_outside_entries() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("a.zip");
        make_zip(&archive, &[("ReadMe.txt", Some(b"r")), ("Apache24/", None), ("Apache24/bin/httpd.exe", Some(b"exe")), ("Apache24/conf/httpd.conf", Some(b"conf"))]);
        let dest = dir.path().join("packages").join("apache-2.4");
        run(&archive, &dest, Some("Apache24")).unwrap();
        assert_eq!(fs::read(dest.join("bin/httpd.exe")).unwrap(), b"exe");
        assert!(dest.join("conf/httpd.conf").exists());
        assert!(!dest.join("ReadMe.txt").exists());
        assert_eq!(fs::read_dir(dest.parent().unwrap()).unwrap().count(), 1, "no staging leftovers");
    }

    #[test]
    fn missing_prefix_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("a.zip");
        make_zip(&archive, &[("other/x", Some(b"x"))]);
        let err = run(&archive, &dir.path().join("out"), Some("Apache24")).unwrap_err();
        assert!(matches!(err, PortError::InvalidData { .. }));
        assert!(!dir.path().join("out").exists());
    }

    #[test]
    fn rejects_zip_slip_entries_and_writes_nothing() {
        for evil in ["../evil.txt", "a/../../evil.txt", "/abs.txt", "C:/Windows/evil.txt", "C:evil.txt", "a\\..\\..\\evil.txt", "file.txt:stream"] {
            assert_refused(&[("ok.txt", Some(b"ok")), (evil, Some(b"evil"))]);
        }
    }

    #[test]
    fn rejects_names_windows_would_not_create_as_named() {
        for evil in ["CON", "sub/aux.txt", "bin/nul", "COM1.dll", "trail.", "dir./x.txt", "trail ", "a?b"] {
            assert_refused(&[("ok.txt", Some(b"ok")), (evil, Some(b"evil"))]);
        }
    }

    #[test]
    fn rejects_files_whose_names_differ_only_by_case() {
        assert_refused(&[("ext/php_curl.dll", Some(b"a")), ("EXT/PHP_curl.DLL", Some(b"b"))]);
    }

    #[test]
    fn directories_differing_by_case_are_merged() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("a.zip");
        make_zip(&archive, &[("ext/", None), ("EXT/", None), ("Ext/a.dll", Some(b"a")), ("ext/b.dll", Some(b"b"))]);
        let dest = dir.path().join("out");
        run(&archive, &dest, None).unwrap();
        assert!(dest.join("ext").join("a.dll").exists() && dest.join("ext").join("b.dll").exists());
    }

    #[test]
    fn an_archive_expanding_beyond_the_limit_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("bomb.zip");
        make_zip(&archive, &[("a.bin", Some(&[0u8; 600])), ("b.bin", Some(&[0u8; 600]))]);
        let dest = dir.path().join("out");
        let err = extract_with_limit(&archive, &dest, None, |_| 1000, &|_| {}, &CancelFlag::new()).unwrap_err();
        assert!(matches!(&err, PortError::InvalidData { message, .. } if message.contains("1200 bytes")), "{err:?}");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1, "nothing extracted");
        extract_with_limit(&archive, &dest, None, |_| 1200, &|_| {}, &CancelFlag::new()).unwrap();
    }

    #[test]
    fn the_expansion_limit_is_six_times_the_archive_with_a_two_gib_floor() {
        const GIB: u64 = 1024 * 1024 * 1024;
        assert_eq!(expansion_limit(0), 2 * GIB);
        assert_eq!(expansion_limit(30_000_000), 2 * GIB, "a PHP zip");
        assert_eq!(expansion_limit(GIB), 6 * GIB);
        assert_eq!(expansion_limit(u64::MAX), u64::MAX);
    }

    #[test]
    fn replaces_an_existing_install_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("php-8.3");
        let v1 = dir.path().join("v1.zip");
        make_zip(&v1, &[("php.exe", Some(b"8.3.14")), ("old-only.dll", Some(b"x"))]);
        run(&v1, &dest, None).unwrap();
        let v2 = dir.path().join("v2.zip");
        make_zip(&v2, &[("php.exe", Some(b"8.3.15"))]);
        run(&v2, &dest, None).unwrap();
        assert_eq!(fs::read(dest.join("php.exe")).unwrap(), b"8.3.15");
        assert!(!dest.join("old-only.dll").exists(), "the new version replaces the old tree entirely");
        let names: Vec<_> = fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names.len(), 3, "{names:?}");
    }

    #[test]
    fn corrupt_archive_keeps_the_previous_version() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("php-8.3");
        let good = dir.path().join("good.zip");
        make_zip(&good, &[("php.exe", Some(b"good"))]);
        run(&good, &dest, None).unwrap();
        let broken = dir.path().join("broken.zip");
        fs::write(&broken, b"PK\x03\x04 this is not a zip").unwrap();
        assert!(run(&broken, &dest, None).is_err());
        assert_eq!(fs::read(dest.join("php.exe")).unwrap(), b"good");
    }

    #[test]
    fn cancellation_leaves_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("a.zip");
        make_zip(&archive, &[("a.txt", Some(b"a")), ("b.txt", Some(b"b"))]);
        let cancel = CancelFlag::new();
        cancel.cancel();
        let dest = dir.path().join("out");
        assert_eq!(extract_blocking(&archive, &dest, None, &|_| {}, &cancel), Err(PortError::Cancelled));
        assert!(!dest.exists());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn leftovers_of_a_crash_are_cleaned() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("redis-8.10");
        fs::create_dir_all(dir.path().join("redis-8.10.staging-1-2")).unwrap();
        fs::create_dir_all(dir.path().join("redis-8.10.old-1-2")).unwrap();
        let archive = dir.path().join("a.zip");
        make_zip(&archive, &[("redis-server.exe", Some(b"x"))]);
        run(&archive, &dest, None).unwrap();
        assert!(!dir.path().join("redis-8.10.staging-1-2").exists());
        assert!(!dir.path().join("redis-8.10.old-1-2").exists());
    }

    #[tokio::test]
    async fn async_port_reports_progress() {
        use std::sync::Mutex;
        #[derive(Default)]
        struct Rec(Mutex<Vec<Progress>>);
        impl ProgressSink for Rec {
            fn report(&self, p: Progress) {
                self.0.lock().unwrap().push(p);
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("a.zip");
        make_zip(&archive, &[("x/a.txt", Some(b"hello")), ("x/b.txt", Some(b"world!"))]);
        let rec = Rec::default();
        ZipExtractor.extract_zip(&archive, &dir.path().join("out"), Some("x"), &rec, &CancelFlag::new()).await.unwrap();
        let last = rec.0.lock().unwrap().last().cloned().unwrap();
        assert_eq!((last.stage, last.done, last.total), (ProgressStage::Extracting, 11, Some(11)));
    }

    #[tokio::test]
    async fn async_port_places_single_files() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("download.phar");
        fs::write(&file, b"phar").unwrap();
        let slot = dir.path().join("composer-2.8");
        ZipExtractor.place_file(&file, &slot, "composer.phar", &CancelFlag::new()).await.unwrap();
        assert_eq!(fs::read(slot.join("composer.phar")).unwrap(), b"phar");
    }
}
