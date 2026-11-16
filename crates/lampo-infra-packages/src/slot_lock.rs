//! [`FsPackageSlotLock`]: one lock per package slot, held for a whole install (download,
//! extraction, hook, record).
//!
//! Two layers, both taken without waiting and retried every [`POLL_INTERVAL`] (so a waiter
//! notices cancellation): a `tokio::sync::Mutex` per slot within the process, then the file lock
//! `packages\<slot>.lock` (see `file_lock`) that the GUI and the CLI share.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use lampo_domain::package::PackageId;
use lampo_domain::paths::LampoPaths;
use lampo_domain::ports::{PackageSlotLock, SlotGuard};
use lampo_domain::progress::CancelFlag;
use lampo_domain::{PortError, PortResult};
use lampo_infra_fs::ensure_dir;
use tokio::sync::OwnedMutexGuard;

use crate::file_lock::FileLock;
use crate::registry::slot_key;

/// How often a waiter retries a slot held by someone else.
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Extension of the lock file next to each slot directory.
const LOCK_EXTENSION: &str = "lock";

/// Both layers of a held slot; released when dropped.
pub(crate) struct HeldSlot {
    _file: FileLock,
    _local: OwnedMutexGuard<()>,
}

/// Slot locks of one packages directory. Share one instance per process (the install use case
/// and the leftover sweep), so its in-process layer sees every holder.
pub struct FsPackageSlotLock {
    packages_dir: PathBuf,
    local: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl FsPackageSlotLock {
    pub fn new(paths: &LampoPaths) -> Self {
        Self { packages_dir: paths.packages_dir(), local: Mutex::default() }
    }

    fn local_lock(&self, slot: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = self.local.lock().unwrap_or_else(|p| p.into_inner());
        locks.entry(slot.to_string()).or_default().clone()
    }

    fn lock_file(&self, slot: &str) -> PathBuf {
        self.packages_dir.join(format!("{slot}.{LOCK_EXTENSION}"))
    }

    /// Holds the slot directory named `slot` if nobody else does (`Ok(None)` otherwise).
    /// Blocking (opens the lock file).
    pub(crate) fn try_hold(&self, slot: &str) -> PortResult<Option<HeldSlot>> {
        try_hold(self.local_lock(slot), &self.packages_dir, &self.lock_file(slot))
    }
}

fn try_hold(local: Arc<tokio::sync::Mutex<()>>, packages_dir: &Path, lock_file: &Path) -> PortResult<Option<HeldSlot>> {
    let Ok(local) = local.try_lock_owned() else { return Ok(None) };
    ensure_dir(packages_dir)?;
    let file = FileLock::try_acquire(lock_file).map_err(|e| PortError::io(format!("locking {}", lock_file.display()), e))?;
    Ok(file.map(|file| HeldSlot { _file: file, _local: local }))
}

#[async_trait]
impl PackageSlotLock for FsPackageSlotLock {
    async fn acquire(&self, id: &PackageId, cancel: &CancelFlag) -> PortResult<SlotGuard> {
        let slot = slot_key(id);
        let (local, lock_file) = (self.local_lock(&slot), self.lock_file(&slot));
        let mut waiting = false;
        loop {
            cancel.check()?;
            let (local, packages_dir, lock_file) = (local.clone(), self.packages_dir.clone(), lock_file.clone());
            let held = tokio::task::spawn_blocking(move || try_hold(local, &packages_dir, &lock_file)).await.map_err(|e| PortError::io(format!("locking the package slot {slot}"), e))??;
            if let Some(held) = held {
                return Ok(SlotGuard::new(held));
            }
            if !waiting {
                tracing::info!(slot, "another install into this package slot is running; waiting for it to finish");
                waiting = true;
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use lampo_domain::package::PackageKind;
    use lampo_domain::php::PhpMinor;
    use lampo_domain::version::PackageVersion;

    use super::*;

    fn id(kind: PackageKind, version: &str, php: Option<&str>) -> PackageId {
        PackageId { kind, version: PackageVersion::parse(version).unwrap(), php: php.map(|p| PhpMinor::parse(p).unwrap()) }
    }

    fn paths(dir: &tempfile::TempDir) -> LampoPaths {
        LampoPaths::new(dir.path().join("cfg"), dir.path().join("data"))
    }

    #[tokio::test]
    async fn a_second_install_into_the_slot_waits_for_the_first() {
        let dir = tempfile::tempdir().unwrap();
        let locks = Arc::new(FsPackageSlotLock::new(&paths(&dir)));
        let first = locks.acquire(&id(PackageKind::PHP, "8.3.14", None), &CancelFlag::new()).await.unwrap();
        // Another version of the same series shares the slot.
        let waiter = tokio::spawn({
            let locks = locks.clone();
            async move { locks.acquire(&id(PackageKind::PHP, "8.3.15", None), &CancelFlag::new()).await.map(drop) }
        });
        tokio::time::sleep(POLL_INTERVAL * 3).await;
        assert!(!waiter.is_finished(), "the slot is still held");
        drop(first);
        waiter.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn other_slots_are_independent() {
        let dir = tempfile::tempdir().unwrap();
        let locks = FsPackageSlotLock::new(&paths(&dir));
        let cancel = CancelFlag::new();
        let _php83 = locks.acquire(&id(PackageKind::PHP, "8.3.14", None), &cancel).await.unwrap();
        let _php84 = locks.acquire(&id(PackageKind::PHP, "8.4.1", None), &cancel).await.unwrap();
        let _xdebug = locks.acquire(&id(PackageKind::XDEBUG, "3.5.0", Some("8.3")), &cancel).await.unwrap();
    }

    #[tokio::test]
    async fn waiting_is_cancellable() {
        let dir = tempfile::tempdir().unwrap();
        let locks = FsPackageSlotLock::new(&paths(&dir));
        let php = id(PackageKind::MYSQL, "8.4.3", None);
        let _held = locks.acquire(&php, &CancelFlag::new()).await.unwrap();
        let cancel = CancelFlag::new();
        cancel.cancel();
        assert_eq!(locks.acquire(&php, &cancel).await.unwrap_err(), PortError::Cancelled);
    }

    /// Two instances share nothing in memory, like the GUI and the CLI: the lock file alone
    /// keeps them apart.
    #[cfg(windows)]
    #[tokio::test]
    async fn separate_processes_exclude_each_other_through_the_lock_file() {
        let dir = tempfile::tempdir().unwrap();
        let (gui, cli) = (FsPackageSlotLock::new(&paths(&dir)), FsPackageSlotLock::new(&paths(&dir)));
        let redis = id(PackageKind::REDIS, "8.10.3", None);
        let held = gui.acquire(&redis, &CancelFlag::new()).await.unwrap();
        assert!(cli.try_hold(&slot_key(&redis)).unwrap().is_none());
        drop(held);
        assert!(cli.try_hold(&slot_key(&redis)).unwrap().is_some());
    }
}
