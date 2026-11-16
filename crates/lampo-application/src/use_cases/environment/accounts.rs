//! The MySQL accounts of the running instances, kept by [`ApplyEnvironment`] under its lock:
//! each running MySQL instance is brought to [`InstanceAccounts::plan`] after every sync and
//! after a start outside a sync; the plan applied is remembered per instance
//! ([`SyncState::accounts`]) until it stops or starts again, so an unchanged plan costs nothing.

use lampo_domain::mysql::MysqlSeries;
use lampo_domain::mysql::accounts::InstanceAccounts;
use lampo_domain::package::InstalledPackage;
use lampo_domain::project::Project;
use lampo_domain::service::{ServiceId, ServiceKind, ServiceState};
use lampo_domain::settings::Settings;
use lampo_domain::worktree::WorktreeEnvironment;

use super::{ApplyEnvironment, SyncState};
use crate::error::AppResult;
use crate::use_cases::generation::ConfigSnapshot;
use crate::use_cases::support::mysql_endpoint_in;

/// What a plan is computed from.
struct Registry<'a> {
    projects: &'a [Project],
    worktrees: &'a [WorktreeEnvironment],
    settings: &'a Settings,
    installed: &'a [InstalledPackage],
}

impl ApplyEnvironment {
    /// Brings every running MySQL instance to the account plan of the registered sites, unless it
    /// already has it. A failure is a warning: the sync goes on, and the next one tries again.
    pub(super) async fn sync_mysql_accounts(&self, snap: &ConfigSnapshot, state: &mut SyncState, warnings: &mut Vec<String>) {
        let registry = Registry { projects: &snap.projects, worktrees: &snap.worktrees, settings: &snap.settings, installed: &snap.installed };
        let running: Vec<ServiceId> = state.launches.keys().filter(|id| self.is_running_mysql(id)).cloned().collect();
        for id in running {
            if let Err(e) = self.provision_accounts(&id, &registry, state).await {
                warnings.push(format!("the MySQL accounts of {id} were not updated: {}", e.message));
            }
        }
    }

    /// [`Self::sync_mysql_accounts`] for `id` just started outside a sync, from the stores.
    /// Failures are logged: the start itself succeeded.
    pub(super) async fn provision_started_mysql(&self, state: &mut SyncState, id: &ServiceId) {
        if !self.is_running_mysql(id) {
            return;
        }
        if let Err(e) = self.provision_from_stores(state, id).await {
            tracing::warn!(service = %id, "the MySQL accounts were not updated: {}", e.message);
        }
    }

    async fn provision_from_stores(&self, state: &mut SyncState, id: &ServiceId) -> AppResult<()> {
        let (projects, worktrees, settings, installed) = (self.deps.projects.list()?, self.deps.worktrees.list_all()?, self.deps.settings.load()?, self.deps.installed.list()?);
        self.provision_accounts(id, &Registry { projects: &projects, worktrees: &worktrees, settings: &settings, installed: &installed }, state).await
    }

    fn is_running_mysql(&self, id: &ServiceId) -> bool {
        id.kind() == &ServiceKind::MYSQL && self.runner(id).is_ok_and(|r| r.status(id).state == ServiceState::Running)
    }

    async fn provision_accounts(&self, id: &ServiceId, registry: &Registry<'_>, state: &mut SyncState) -> AppResult<()> {
        let Some(series) = id.version().and_then(|v| MysqlSeries::try_from(v).ok()) else { return Ok(()) };
        let plan = InstanceAccounts::plan(series, registry.projects, registry.worktrees);
        if state.accounts.get(id) == Some(&plan) {
            return Ok(());
        }
        let endpoint = mysql_endpoint_in(registry.installed, registry.settings, &self.deps.services, series)?;
        let done = self.deps.mysql_accounts.provision(&endpoint, &plan).await?;
        if done.root_secured {
            tracing::info!(service = %id, "root now has a random password, stored encrypted for the current user");
        }
        state.accounts.insert(id.clone(), plan);
        Ok(())
    }
}
