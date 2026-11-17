//! Loopback-port allocation: the single owner of "pick a free `127.0.0.1:<port>` and persist it".
//!
//! Projects with a `LoopbackPort` address and worktree environments draw from the same range
//! (`Settings::loopback_ports`). Several front-ends call into the app concurrently (the GUI plus
//! any number of agents on the control pipe), so choosing a port and saving the site that owns it
//! must be one atomic step. [`LoopbackPortAllocator`] holds that lock; every use case that saves
//! loopback addresses goes through it, and the composition roots share **one** instance.
//!
//! Every write to the registry (projects, worktree environments, settings) runs under that lock
//! and is numbered ([`RegistryRevision`]), so the snapshot a commit hands on can be told apart
//! from a stale one (see [`super::site_snapshot`]).

use std::sync::Arc;

use lampo_domain::ports::{PortAvailability, PortProbe, ProjectRepository, SettingsRepository, WorktreeRepository};
use lampo_domain::tcp_port::Port;
use lampo_domain::worktree::{allocate_loopback_port, used_loopback_ports};

pub use super::site_snapshot::SiteSnapshot;
use super::site_snapshot::RegistryRevision;
use crate::blocking::{in_place, unblocked};
use crate::error::AppResult;

pub struct LoopbackPortAllocator {
    projects: Arc<dyn ProjectRepository>,
    worktrees: Arc<dyn WorktreeRepository>,
    settings: Arc<dyn SettingsRepository>,
    probe: Arc<dyn PortProbe>,
    lock: tokio::sync::Mutex<()>,
    revision: Arc<RegistryRevision>,
}

impl LoopbackPortAllocator {
    pub fn new(projects: Arc<dyn ProjectRepository>, worktrees: Arc<dyn WorktreeRepository>, settings: Arc<dyn SettingsRepository>, probe: Arc<dyn PortProbe>) -> Self {
        Self { projects, worktrees, settings, probe, lock: tokio::sync::Mutex::new(()), revision: Arc::default() }
    }

    /// The registry, read on the blocking pool; stamped with `commit`.
    async fn snapshot(&self, commit: u64) -> AppResult<SiteSnapshot> {
        let (projects, worktrees, settings, revision) = (self.projects.clone(), self.worktrees.clone(), self.settings.clone(), self.revision.clone());
        unblocked("reading the projects, worktree environments and settings", move || Ok(SiteSnapshot::loaded(projects.scan()?, worktrees.list_all()?, settings.load()?, revision, commit)))
            .await
    }

    /// Lowest port of the configured range that is neither assigned to a site, nor named by an
    /// unreadable project file, nor a service port, nor busy on the OS.
    fn free_port(&self, snap: &SiteSnapshot) -> AppResult<Port> {
        let mut used = used_loopback_ports(&snap.projects, &snap.worktrees);
        used.extend(&snap.reserved_ports);
        loop {
            let port = allocate_loopback_port(&used, &snap.settings)?;
            match self.probe.check(port)? {
                PortAvailability::Free => return Ok(port),
                PortAvailability::InUse { .. } => {
                    used.insert(port);
                }
            }
        }
    }

    /// Allocate-and-reserve: under the lock, picks a free port and runs `commit(port, snapshot)`,
    /// which must persist the site owning the port before returning. No other allocation or
    /// reservation can observe the registry in between.
    pub async fn allocate<T>(&self, commit: impl FnOnce(Port, &SiteSnapshot) -> AppResult<T>) -> AppResult<T> {
        let _guard = self.lock.lock().await;
        let snap = self.snapshot(self.revision.begin_commit()).await?;
        in_place(|| {
            let port = self.free_port(&snap)?;
            commit(port, &snap)
        })
    }

    /// For saves whose addresses were chosen by the caller (explicit ports, updates, relinks,
    /// imports): runs `commit(snapshot)` under the same lock, so its conflict check
    /// (`project::check_conflicts`) and its save cannot interleave with an allocation.
    pub async fn reserve<T>(&self, commit: impl FnOnce(&SiteSnapshot) -> AppResult<T>) -> AppResult<T> {
        let _guard = self.lock.lock().await;
        let snap = self.snapshot(self.revision.begin_commit()).await?;
        in_place(|| commit(&snap))
    }

    /// For registry writes that check nothing against the other sites (unregistering a worktree
    /// environment): runs `write` under the same lock, numbered like every commit, so a sync
    /// never applies a snapshot older than it.
    pub async fn write<T>(&self, write: impl FnOnce() -> AppResult<T>) -> AppResult<T> {
        let _guard = self.lock.lock().await;
        self.revision.begin_commit();
        in_place(write)
    }

    /// The port [`Self::allocate`] would pick right now, without reserving it (address
    /// suggestions shown before a project is created).
    pub async fn peek(&self) -> AppResult<Port> {
        let _guard = self.lock.lock().await;
        // Commit 0: never current, as nothing is committed with it.
        let snap = self.snapshot(0).await?;
        in_place(|| self.free_port(&snap))
    }
}
