//! The one guarded save path of the global settings: the GUI's full save
//! ([`super::settings::SettingsService`]), `lampo settings set` ([`super::settings_patch`]), the
//! first PHP installed becoming the default and the first-run wizard's completion.
//!
//! Every save runs under the allocator lock ([`LoopbackPortAllocator::reserve`]) and is built
//! from the settings stored at that moment, so it never reverts a concurrent edit. Its checks
//! against the sites' dedicated loopback ports cannot interleave with a project save or a port
//! allocation, which check the other direction under the same lock.

use std::sync::Arc;

use lampo_domain::DomainError;
use lampo_domain::PortError;
use lampo_domain::events::DomainEvent;
use lampo_domain::ports::{EventPublisher, InstalledPackageRegistry, PortAvailability, PortProbe, SettingsRepository};
use lampo_domain::project::Project;
use lampo_domain::service::ServiceId;
use lampo_domain::settings::Settings;
use lampo_domain::worktree::WorktreeEnvironment;

use super::allocation::{LoopbackPortAllocator, SiteSnapshot};
use super::service_catalog::ServiceCatalog;
use super::support::php_package;
use crate::error::{AppError, AppResult, ErrorCode};

/// What every settings-mutating use case needs. The composition roots share the allocator with
/// every other use case that takes it.
#[derive(Clone)]
pub struct SettingsWriter {
    pub repo: Arc<dyn SettingsRepository>,
    pub allocator: Arc<LoopbackPortAllocator>,
    pub port_probe: Arc<dyn PortProbe>,
    /// A new default PHP must be installed.
    pub installed: Arc<dyn InstalledPackageRegistry>,
    pub events: Arc<dyn EventPublisher>,
    pub services: Arc<ServiceCatalog>,
}

/// The outcome of [`SettingsWriter::modify`]: the settings before and after (equal when nothing
/// changed) and the sites seen under the lock.
pub(crate) struct SettingsCommit {
    pub before: Settings,
    pub after: Settings,
    pub projects: Vec<Project>,
    pub worktrees: Vec<WorktreeEnvironment>,
}

impl SettingsWriter {
    pub fn load(&self) -> AppResult<Settings> {
        Ok(self.repo.load()?)
    }

    /// Under the allocator lock: `change` turns the stored settings into the new ones (validated
    /// by `change` itself); when they differ they are checked ([`Self::check`]), saved and
    /// `SettingsChanged` is published. Nothing is saved on error.
    pub(crate) async fn modify(&self, change: impl FnOnce(&Settings) -> AppResult<Settings>) -> AppResult<SettingsCommit> {
        self.allocator
            .reserve(|snap| {
                let before = &snap.settings;
                let after = change(before)?;
                if &after != before {
                    self.check(before, &after, snap)?;
                    self.repo.save(&after)?;
                    self.events.publish(DomainEvent::SettingsChanged);
                }
                Ok(SettingsCommit { before: before.clone(), after, projects: snap.projects.clone(), worktrees: snap.worktrees.clone() })
            })
            .await
    }

    /// `PackageMissing` for a new default PHP that is not installed; `Conflict` for a port that is
    /// a site's dedicated loopback port; `PortInUse` for a new port another program listens on.
    fn check(&self, before: &Settings, after: &Settings, snap: &SiteSnapshot) -> AppResult<()> {
        self.check_default_php(before, after)?;
        let changed = changed_ports(before, after, &self.services);
        check_loopback_ports(&changed, &snap.projects, &snap.worktrees)?;
        self.check_free(&changed, before)
    }

    fn check_default_php(&self, before: &Settings, after: &Settings) -> AppResult<()> {
        let Some(php) = after.default_php.filter(|php| Some(*php) != before.default_php) else { return Ok(()) };
        match php_package(&self.installed.list()?, php) {
            Some(_) => Ok(()),
            None => Err(AppError::new(ErrorCode::PackageMissing, format!("PHP {php} is not installed: run `lampo package install php {php}` first"))),
        }
    }

    /// New ports must be free on the OS, unless a Lampo service holds them now (a swap between
    /// instances: the sync moves them).
    fn check_free(&self, changed: &[PortChange], before: &Settings) -> AppResult<()> {
        for c in changed {
            if before.service_on_port(c.port).is_some() {
                continue;
            }
            if let PortAvailability::InUse { owner } = self.port_probe.check(c.port)? {
                return Err(PortError::PortInUse { port: c.port, owner }.into());
            }
        }
        Ok(())
    }
}

/// One port that differs between two settings: a service's main port, or its web UI port.
pub(super) struct PortChange {
    pub id: ServiceId,
    pub ui: bool,
    pub port: u16,
}

impl PortChange {
    /// `apache.port`, `mysql-8.4.port`, `mailpit.ui-port`.
    pub fn setting(&self) -> String {
        format!("{}.{}", self.id, if self.ui { "ui-port" } else { "port" })
    }

    pub fn previous(&self, before: &Settings, catalog: &ServiceCatalog) -> Option<u16> {
        match self.ui {
            false => before.service_port(&self.id),
            true => catalog.ui_port(&self.id, before),
        }
    }
}

/// Every port (Apache's, services', web UIs') that differs between `before` and `after`, with
/// its new value. A web UI port is compared by its effective value (configured, else default).
pub(super) fn changed_ports(before: &Settings, after: &Settings, catalog: &ServiceCatalog) -> Vec<PortChange> {
    let mut out = Vec::new();
    if before.apache_port != after.apache_port {
        out.push(PortChange { id: ServiceId::apache(), ui: false, port: after.apache_port });
    }
    for s in &after.services {
        if before.service_port(&s.service) != Some(s.port) {
            out.push(PortChange { id: s.service.clone(), ui: false, port: s.port });
        }
        if let Some(ui) = catalog.ui_port(&s.service, after).filter(|ui| catalog.ui_port(&s.service, before) != Some(*ui)) {
            out.push(PortChange { id: s.service.clone(), ui: true, port: ui });
        }
    }
    out
}

/// A service port may not be a site's dedicated loopback port (Apache listens there for it).
fn check_loopback_ports(changed: &[PortChange], projects: &[Project], worktrees: &[WorktreeEnvironment]) -> AppResult<()> {
    for port in changed.iter().map(|c| c.port) {
        let project = projects.iter().find(|p| p.addresses.iter().any(|a| a.loopback() == Some(port))).map(|p| p.id.to_string());
        let site = project.or_else(|| worktrees.iter().find(|w| w.loopback_port == port).map(|w| w.site_id().to_string()));
        if let Some(site) = site {
            return Err(DomainError::LoopbackPortTaken { port, service: format!("site `{site}`") }.into());
        }
    }
    Ok(())
}
