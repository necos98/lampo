//! [`FsLeftoverSweeper`]: the app-start cleanup of what interrupted package operations left.
//!
//! * In `packages\`, every slot with leftovers next to it is handled as `leftovers`
//!   describes (an interrupted replacement restored, the copies of processes that are gone
//!   deleted), while holding that slot's lock: a slot an install is working on is skipped.
//! * In `downloads\`, a partial download (`*.part`, with its `*.part.json` metadata) untouched
//!   for [`STALE_PARTIAL_DOWNLOAD_AGE`] is deleted: a download in progress writes to it all the
//!   time, and resuming one that old is not worth the disk space.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use lampo_domain::paths::LampoPaths;
use lampo_domain::ports::PackageLeftoverSweeper;

use crate::download::{PART_META_SUFFIX, PART_SUFFIX};
use crate::leftovers;
use crate::slot_lock::FsPackageSlotLock;

/// Partial downloads older than this are deleted (a week).
pub const STALE_PARTIAL_DOWNLOAD_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);

#[derive(Clone)]
pub struct FsLeftoverSweeper {
    packages_dir: PathBuf,
    downloads_dir: PathBuf,
    slots: Arc<FsPackageSlotLock>,
}

impl FsLeftoverSweeper {
    /// `slots` must be the lock instance the install use case holds.
    pub fn new(paths: &LampoPaths, slots: Arc<FsPackageSlotLock>) -> Self {
        Self { packages_dir: paths.packages_dir(), downloads_dir: paths.downloads_dir(), slots }
    }

    fn sweep_blocking(&self) {
        self.sweep_slots();
        sweep_partial_downloads(&self.downloads_dir, SystemTime::now(), STALE_PARTIAL_DOWNLOAD_AGE);
    }

    fn sweep_slots(&self) {
        let slots = match leftovers::slots_with_leftovers(&self.packages_dir) {
            Ok(slots) => slots,
            Err(e) => return tracing::warn!(dir = %self.packages_dir.display(), error = %e, "cannot look for leftovers of interrupted package operations"),
        };
        for slot in slots {
            let Some(name) = slot.file_name().and_then(|n| n.to_str()) else { continue };
            match self.slots.try_hold(name) {
                Ok(Some(_held)) => leftovers::repair(&slot),
                Ok(None) => tracing::debug!(slot = name, "an install is using this package slot; its leftovers wait for the next start"),
                Err(e) => tracing::warn!(slot = name, error = %e, "cannot lock a package slot to clean it"),
            }
        }
    }
}

#[async_trait]
impl PackageLeftoverSweeper for FsLeftoverSweeper {
    async fn sweep(&self) {
        let sweeper = self.clone();
        if let Err(e) = tokio::task::spawn_blocking(move || sweeper.sweep_blocking()).await {
            tracing::warn!(error = %e, "the cleanup of interrupted package operations stopped early");
        }
    }
}

/// Deletes the partial downloads of `dir` last written more than `max_age` before `now`.
fn sweep_partial_downloads(dir: &Path, now: SystemTime, max_age: Duration) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
        Err(e) => return tracing::warn!(dir = %dir.display(), error = %e, "cannot look for stale partial downloads"),
    };
    for entry in entries.flatten() {
        let Some(download) = entry.file_name().to_str().and_then(|n| n.strip_suffix(PART_SUFFIX)).map(str::to_string) else { continue };
        let Ok(metadata) = entry.metadata() else { continue };
        let Ok(modified) = metadata.modified() else { continue };
        if metadata.is_file() && now.duration_since(modified).is_ok_and(|age| age > max_age) {
            remove_partial(dir, &download);
        }
    }
}

/// Deletes the partial file of the download `download` (a file name in `dir`) and its resume
/// metadata.
fn remove_partial(dir: &Path, download: &str) {
    for file in [dir.join(format!("{download}{PART_SUFFIX}")), dir.join(format!("{download}{PART_META_SUFFIX}"))] {
        match std::fs::remove_file(&file) {
            Ok(()) => tracing::info!(file = %file.display(), "deleted a stale partial download"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => tracing::warn!(file = %file.display(), error = %e, "cannot delete a stale partial download"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        names
    }

    #[test]
    fn partial_downloads_older_than_the_limit_are_deleted_with_their_metadata() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["php-8.3.15-php.zip.part", "php-8.3.15-php.zip.part.json", "mysql-8.4.3-mysql.zip", "redis.part.txt"] {
            std::fs::write(dir.path().join(name), b"x").unwrap();
        }
        // Just written: not stale yet.
        sweep_partial_downloads(dir.path(), SystemTime::now(), STALE_PARTIAL_DOWNLOAD_AGE);
        assert_eq!(names(dir.path()).len(), 4);
        // A week and a bit later.
        sweep_partial_downloads(dir.path(), SystemTime::now() + STALE_PARTIAL_DOWNLOAD_AGE + Duration::from_secs(60), STALE_PARTIAL_DOWNLOAD_AGE);
        assert_eq!(names(dir.path()), ["mysql-8.4.3-mysql.zip", "redis.part.txt"], "only partial downloads and their metadata go");
        sweep_partial_downloads(&dir.path().join("missing"), SystemTime::now(), STALE_PARTIAL_DOWNLOAD_AGE);
    }

    #[cfg(windows)]
    mod slots {
        use lampo_domain::package::{PackageId, PackageKind};
        use lampo_domain::ports::PackageSlotLock;
        use lampo_domain::progress::CancelFlag;
        use lampo_domain::version::PackageVersion;

        use super::*;
        use crate::pid::exited_process_id;

        fn setup() -> (tempfile::TempDir, LampoPaths, Arc<FsPackageSlotLock>) {
            let dir = tempfile::tempdir().unwrap();
            let paths = LampoPaths::new(dir.path().join("cfg"), dir.path().join("data"));
            std::fs::create_dir_all(paths.packages_dir()).unwrap();
            let slots = Arc::new(FsPackageSlotLock::new(&paths));
            (dir, paths, slots)
        }

        fn previous_copy(paths: &LampoPaths, slot: &str, content: &[u8]) -> PathBuf {
            let old = paths.packages_dir().join(format!("{slot}.old-{}-1", exited_process_id()));
            std::fs::create_dir_all(&old).unwrap();
            std::fs::write(old.join("bin.exe"), content).unwrap();
            old
        }

        #[tokio::test]
        async fn interrupted_replacements_are_restored_and_dead_leftovers_deleted() {
            let (_d, paths, slots) = setup();
            previous_copy(&paths, "php-8.3", b"8.3.14");
            std::fs::create_dir_all(paths.packages_dir().join("redis-8.10")).unwrap();
            previous_copy(&paths, "redis-8.10", b"old redis");
            std::fs::create_dir_all(paths.packages_dir().join(format!("mysql-8.4.trash-{}-9", exited_process_id()))).unwrap();
            let alive = paths.packages_dir().join(format!("apache-2.4.staging-{}-1", std::process::id()));
            std::fs::create_dir_all(&alive).unwrap();

            FsLeftoverSweeper::new(&paths, slots).sweep().await;

            assert_eq!(std::fs::read(paths.packages_dir().join("php-8.3").join("bin.exe")).unwrap(), b"8.3.14");
            let mut left = names(&paths.packages_dir());
            left.retain(|n| !n.ends_with(".lock"));
            assert_eq!(left, [alive.file_name().unwrap().to_string_lossy().into_owned(), "php-8.3".into(), "redis-8.10".into()]);
        }

        #[tokio::test]
        async fn a_slot_an_install_holds_is_left_alone() {
            let (_d, paths, slots) = setup();
            let old = previous_copy(&paths, "php-8.3", b"8.3.14");
            let id = PackageId::new(PackageKind::PHP, PackageVersion::parse("8.3.15").unwrap());
            let _installing = slots.acquire(&id, &CancelFlag::new()).await.unwrap();
            FsLeftoverSweeper::new(&paths, slots.clone()).sweep().await;
            assert!(old.exists() && !paths.packages_dir().join("php-8.3").exists(), "the install owns the slot");
        }
    }
}
