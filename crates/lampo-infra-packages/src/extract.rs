//! Zip extraction that is all-or-nothing.
//!
//! 1. Extract into a staging directory next to the destination (`<dest>.staging-<n>`), same volume.
//! 2. If the destination exists (a patch update of the same slot, e.g. PHP 8.3.14 → 8.3.15),
//!    rename it aside to `<dest>.old-<n>`; this fails cleanly if a running process (php-cgi,
//!    httpd, mysqld) holds files in it, and then nothing has changed.
//! 3. Rename staging → destination (restoring the old directory if that fails), delete the old one.
//!
//! A failure or cancellation at any point removes the staging directory: the destination is
//! either the complete old tree or the complete new one. Leftovers of a crash are removed the next
//! time the same destination is extracted.
//!
//! Zip-slip: entry names must be relative, without `..`, drive prefixes, roots or `:` (NTFS
//! alternate streams); symlink entries are refused.

use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use lampo_domain::ports::{ArchiveExtractor, ProgressSink};
use lampo_domain::progress::{CancelFlag, Progress, ProgressStage};
use lampo_domain::{PortError, PortResult};

/// Extracts into `<destination>.staging-*` then renames (no half-installed packages); rejects
/// zip-slip entries.
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
}

/// Synchronous extraction with the same guarantees (also used by tests).
pub fn extract_blocking(archive: &Path, destination: &Path, strip_prefix: Option<&str>, report: &dyn Fn(Progress), cancel: &CancelFlag) -> PortResult<()> {
    let parent = destination.parent().filter(|p| !p.as_os_str().is_empty()).ok_or_else(|| PortError::io("extracting", format!("{} has no parent directory", destination.display())))?;
    fs::create_dir_all(parent).map_err(|e| PortError::io(format!("creating {}", parent.display()), e))?;
    clean_leftovers(destination);
    let staging = unique_sibling(destination, "staging");
    let result = extract_into(archive, &staging, strip_prefix, report, cancel).and_then(|()| {
        cancel.check()?;
        replace_dir(&staging, destination)
    });
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

fn extract_into(archive: &Path, staging: &Path, strip_prefix: Option<&str>, report: &dyn Fn(Progress), cancel: &CancelFlag) -> PortResult<()> {
    let source = archive.display().to_string();
    let bad = |m: String| PortError::invalid_data(source.clone(), m);
    let file = fs::File::open(archive).map_err(|e| PortError::io(format!("opening {source}"), e))?;
    let mut zip = zip::ZipArchive::new(std::io::BufReader::new(file)).map_err(|e| bad(format!("not a valid zip archive: {e}")))?;

    // First pass: validate every name and compute the total size, before writing anything.
    let mut plan = Vec::with_capacity(zip.len());
    let mut total = 0u64;
    for i in 0..zip.len() {
        let entry = zip.by_index_raw(i).map_err(|e| bad(e.to_string()))?;
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
            total += entry.size();
        }
        plan.push((i, relative, entry.is_dir()));
    }
    if let Some(prefix) = strip_prefix {
        if plan.is_empty() {
            return Err(bad(format!("the archive has no `{prefix}/` folder")));
        }
    }

    fs::create_dir_all(staging).map_err(|e| PortError::io(format!("creating {}", staging.display()), e))?;
    let mut done = 0u64;
    let mut last_report = Instant::now() - Duration::from_secs(1);
    let mut buf = vec![0u8; 256 * 1024];
    for (index, relative, is_dir) in plan {
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

/// A zip entry name as a safe relative path, or `None` if it could escape the destination.
fn safe_relative_path(name: &str) -> Option<PathBuf> {
    let normalized = name.replace('\\', "/");
    if normalized.starts_with('/') || normalized.contains(':') || normalized.contains('\0') {
        return None;
    }
    let mut out = PathBuf::new();
    for part in normalized.split('/') {
        match part {
            "" | "." => continue,
            ".." => return None,
            p => {
                // Each part must be a single normal component (rejects prefixes such as `C:`).
                let mut comps = Path::new(p).components();
                match (comps.next(), comps.next()) {
                    (Some(Component::Normal(_)), None) => out.push(p),
                    _ => return None,
                }
            }
        }
    }
    Some(out)
}

/// Removes the `prefix` top-level folder; `None` = entry outside it (skipped, e.g. Apache
/// Lounge's `ReadMe.txt` next to `Apache24/`).
fn strip(path: &Path, prefix: Option<&str>) -> Option<PathBuf> {
    let Some(prefix) = prefix else { return Some(path.to_path_buf()) };
    let mut comps = path.components();
    match comps.next() {
        Some(Component::Normal(first)) if first.to_string_lossy().eq_ignore_ascii_case(prefix.trim_matches('/')) => Some(comps.as_path().to_path_buf()),
        _ => None,
    }
}

/// Atomically (per rename) replaces `destination` with `staging`.
pub(crate) fn replace_dir(staging: &Path, destination: &Path) -> PortResult<()> {
    let old = if destination.exists() {
        let old = unique_sibling(destination, "old");
        fs::rename(destination, &old).map_err(|e| PortError::io(format!("replacing {} (is a service using it? stop it and retry)", destination.display()), e))?;
        Some(old)
    } else {
        None
    };
    if let Err(e) = fs::rename(staging, destination) {
        if let Some(old) = &old {
            let _ = fs::rename(old, destination);
        }
        return Err(PortError::io(format!("moving files into {}", destination.display()), e));
    }
    if let Some(old) = old {
        if let Err(e) = fs::remove_dir_all(&old) {
            tracing::warn!(dir = %old.display(), error = %e, "could not delete the previous version; it will be removed later");
        }
    }
    Ok(())
}

/// `<dest>.<tag>-<pid>-<nanos>`
pub(crate) fn unique_sibling(path: &Path, tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let mut name = path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(format!(".{tag}-{}-{nanos}", std::process::id()));
    path.with_file_name(name)
}

/// Deletes `<dest>.staging-*` / `<dest>.old-*` / `<dest>.trash-*` left by an interrupted run.
pub(crate) fn clean_leftovers(destination: &Path) {
    let (Some(parent), Some(name)) = (destination.parent(), destination.file_name()) else { return };
    let name = name.to_string_lossy().to_string();
    let Ok(entries) = fs::read_dir(parent) else { return };
    for entry in entries.flatten() {
        let n = entry.file_name().to_string_lossy().to_string();
        let leftover = ["staging", "old", "trash"].iter().any(|tag| n.starts_with(&format!("{name}.{tag}-")));
        if leftover {
            let _ = fs::remove_dir_all(entry.path());
        }
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
            let dir = tempfile::tempdir().unwrap();
            let archive = dir.path().join("a.zip");
            make_zip(&archive, &[("ok.txt", Some(b"ok")), (evil, Some(b"evil"))]);
            let dest = dir.path().join("inner").join("out");
            let err = run(&archive, &dest, None).unwrap_err();
            assert!(matches!(err, PortError::InvalidData { .. }), "{evil}: {err:?}");
            assert!(!dest.exists(), "{evil}");
            assert!(!dir.path().join("evil.txt").exists());
            assert_eq!(fs::read_dir(dir.path().join("inner")).unwrap().count(), 0, "{evil}: staging cleaned");
        }
    }

    #[test]
    fn safe_paths() {
        assert_eq!(safe_relative_path("a/b/c.txt").unwrap(), Path::new("a").join("b").join("c.txt"));
        assert_eq!(safe_relative_path("./a//b").unwrap(), Path::new("a").join("b"));
        assert!(safe_relative_path("..").is_none());
        assert!(safe_relative_path("\\\\server\\share\\x").is_none());
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
        fs::create_dir_all(dir.path().join("redis-8.100")).unwrap();
        let archive = dir.path().join("a.zip");
        make_zip(&archive, &[("redis-server.exe", Some(b"x"))]);
        run(&archive, &dest, None).unwrap();
        assert!(!dir.path().join("redis-8.10.staging-1-2").exists());
        assert!(!dir.path().join("redis-8.10.old-1-2").exists());
        assert!(dir.path().join("redis-8.100").exists(), "unrelated directories are untouched");
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
}
