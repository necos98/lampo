//! Loopback-port allocation: the single owner of "pick a free `127.0.0.1:<port>` and persist it".
//!
//! Projects with a `LoopbackPort` address and worktree environments draw from the same range
//! (`Settings::loopback_ports`). Several front-ends call into the app concurrently (the GUI plus
//! any number of agents on the control pipe), so choosing a port and saving the site that owns it
//! must be one atomic step. [`LoopbackPortAllocator`] holds that lock; every use case that saves
//! loopback addresses goes through it, and the composition roots share **one** instance.

use std::collections::BTreeSet;
use std::sync::Arc;

use lampo_domain::ports::{PortAvailability, PortProbe, ProjectRepository, SettingsRepository, WorktreeRepository};
use lampo_domain::project::Project;
use lampo_domain::settings::Settings;
use lampo_domain::tcp_port::Port;
use lampo_domain::worktree::{WorktreeEnvironment, allocate_loopback_port, used_loopback_ports};

use crate::error::AppResult;

/// The registry as seen under the allocation lock: what a commit validates against.
#[derive(Debug, Clone)]
pub struct SiteSnapshot {
    pub projects: Vec<Project>,
    /// Loopback ports named by project files that do not load: never handed out, so a repaired
    /// project does not find its listener taken.
    pub reserved_ports: BTreeSet<Port>,
    pub worktrees: Vec<WorktreeEnvironment>,
    pub settings: Settings,
}

pub struct LoopbackPortAllocator {
    projects: Arc<dyn ProjectRepository>,
    worktrees: Arc<dyn WorktreeRepository>,
    settings: Arc<dyn SettingsRepository>,
    probe: Arc<dyn PortProbe>,
    lock: tokio::sync::Mutex<()>,
}

impl LoopbackPortAllocator {
    pub fn new(projects: Arc<dyn ProjectRepository>, worktrees: Arc<dyn WorktreeRepository>, settings: Arc<dyn SettingsRepository>, probe: Arc<dyn PortProbe>) -> Self {
        Self { projects, worktrees, settings, probe, lock: tokio::sync::Mutex::new(()) }
    }

    fn snapshot(&self) -> AppResult<SiteSnapshot> {
        let listing = self.projects.scan()?;
        let reserved_ports = listing.unreadable.iter().flat_map(|u| u.loopback_ports.iter().copied()).collect();
        Ok(SiteSnapshot { projects: listing.projects, reserved_ports, worktrees: self.worktrees.list_all()?, settings: self.settings.load()? })
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
        let snap = self.snapshot()?;
        let port = self.free_port(&snap)?;
        commit(port, &snap)
    }

    /// For saves whose addresses were chosen by the caller (explicit ports, updates, relinks,
    /// imports): runs `commit(snapshot)` under the same lock, so its conflict check
    /// (`project::check_conflicts`) and its save cannot interleave with an allocation.
    pub async fn reserve<T>(&self, commit: impl FnOnce(&SiteSnapshot) -> AppResult<T>) -> AppResult<T> {
        let _guard = self.lock.lock().await;
        let snap = self.snapshot()?;
        commit(&snap)
    }

    /// The port [`Self::allocate`] would pick right now, without reserving it (address
    /// suggestions shown before a project is created).
    pub async fn peek(&self) -> AppResult<Port> {
        let _guard = self.lock.lock().await;
        let snap = self.snapshot()?;
        self.free_port(&snap)
    }
}
