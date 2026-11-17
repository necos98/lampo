//! Removing an installed package: the GUI "Pacchetti" page and `lampo package uninstall`.

use std::sync::Arc;

use lampo_domain::events::DomainEvent;
use lampo_domain::package::{InstalledPackage, PackageId, PackageKind};
use lampo_domain::php::PhpMinor;
use lampo_domain::ports::{EventPublisher, InstalledPackageRegistry, PackageSlotLock, ProjectRepository, SettingsRepository, WorktreeRepository};
use lampo_domain::progress::CancelFlag;
use lampo_domain::version::PackageVersion;

use super::environment::EnvironmentSync;
use super::packages::parse_series;
use super::requirements::requirements;
use super::service_catalog::ServiceCatalog;
use crate::context::CallContext;
use crate::dto::{PackageIdDto, UninstallPackageRequest};
use crate::error::{AppError, AppResult, ErrorCode};
use crate::mapping::{package_id_from_dto, package_id_to_dto};

pub struct UninstallPackage {
    installed: Arc<dyn InstalledPackageRegistry>,
    /// The lock installs hold: a removal never runs while an update replaces the slot.
    slots: Arc<dyn PackageSlotLock>,
    projects: Arc<dyn ProjectRepository>,
    worktrees: Arc<dyn WorktreeRepository>,
    settings: Arc<dyn SettingsRepository>,
    events: Arc<dyn EventPublisher>,
    /// Moves vhosts, php.ini files, shims and services off the package before its files go.
    env: Arc<dyn EnvironmentSync>,
    services: Arc<ServiceCatalog>,
}

pub struct UninstallPackageDeps {
    pub installed: Arc<dyn InstalledPackageRegistry>,
    /// The instance the install use case holds.
    pub slots: Arc<dyn PackageSlotLock>,
    pub projects: Arc<dyn ProjectRepository>,
    pub worktrees: Arc<dyn WorktreeRepository>,
    pub settings: Arc<dyn SettingsRepository>,
    pub events: Arc<dyn EventPublisher>,
    pub env: Arc<dyn EnvironmentSync>,
    pub services: Arc<ServiceCatalog>,
}

impl UninstallPackage {
    pub fn new(deps: UninstallPackageDeps) -> Self {
        let UninstallPackageDeps { installed, slots, projects, worktrees, settings, events, env, services } = deps;
        Self { installed, slots, projects, worktrees, settings, events, env, services }
    }

    /// `Conflict` if it is the last installed build satisfying something in use (a site's PHP
    /// minor, MySQL series, add-on…, the always-on MySQL or the default PHP). `NotFound` if not
    /// installed.
    ///
    /// Runs under the slot lock of the package, after any install into that slot: an update
    /// in progress (8.3.14 → 8.3.15 in `php-8.3`) finishes first, and 8.3.14 is then no longer
    /// installed (`NotFound`) instead of its slot being deleted under the new files.
    ///
    /// Long-running (services restart, thousands of files are deleted): front-ends run it as a
    /// task. The environment is first synced without the package, so what used it (a Redis
    /// that no site pins to a series, running from `redis-8.10`) moves to the build that
    /// remains, or stops, before the files are deleted. That sync is best effort: its failure
    /// is logged and the removal still runs; if the files are then still in use, the sync's
    /// error is returned (it says why). The use is checked again once the sync is done, since a
    /// site may have started to need the package meanwhile. A removal that fails or is refused
    /// leaves the package installed, so the environment is synced back onto it. Publishes
    /// `PackagesChanged` once removed.
    pub async fn execute(&self, id: PackageIdDto, ctx: &CallContext) -> AppResult<()> {
        let id = package_id_from_dto(&id)?;
        // Nothing cancels an uninstall: it waits for the install holding the slot.
        let _slot = self.slots.acquire(&id, &CancelFlag::new()).await?;
        self.ensure_removable(&id)?;
        let moved_off = self.env.sync_without(&id, ctx).await;
        if let Err(e) = &moved_off {
            tracing::warn!("environment sync before uninstalling {id} failed: {}", e.message);
        }
        if let Err(e) = self.ensure_removable(&id) {
            self.sync_back(&id, ctx).await;
            return Err(e);
        }
        if let Err(e) = self.installed.remove(&id).await {
            self.sync_back(&id, ctx).await;
            return Err(removal_error(&id, e.into(), moved_off.err()));
        }
        self.events.publish(DomainEvent::PackagesChanged);
        Ok(())
    }

    /// Puts the environment back onto `id`, which stays installed. Best effort (logged).
    async fn sync_back(&self, id: &PackageId, ctx: &CallContext) {
        if let Err(sync) = self.env.sync(ctx).await {
            tracing::warn!("environment sync after failing to uninstall {id} failed: {}", sync.message);
        }
    }

    /// `NotFound` unless `id` is installed; `Conflict` naming who needs it if it is the last
    /// build satisfying a requirement.
    fn ensure_removable(&self, id: &PackageId) -> AppResult<()> {
        let installed = self.installed.list()?;
        if !installed.iter().any(|p| &p.id == id) {
            return Err(AppError::not_found("package", id));
        }
        let others: Vec<InstalledPackage> = installed.into_iter().filter(|p| &p.id != id).collect();
        let reqs = requirements(&self.projects.list()?, &self.worktrees.list_all()?, &self.settings.load()?, &self.services);
        let mut blocking: Vec<String> = reqs.iter().filter(|r| r.matches(id) && !r.satisfied_by(&others)).map(|r| r.required_by.clone()).collect();
        blocking.sort();
        blocking.dedup();
        if blocking.is_empty() {
            return Ok(());
        }
        Err(AppError::new(ErrorCode::Conflict, format!("{id} is in use by {}", blocking.join(", "))).with_details(serde_json::json!({ "package": id.to_string(), "usedBy": blocking })))
    }

    /// `lampo package uninstall`: the one installed build matching `request` (exact version,
    /// else series, else any; `phpVersion` narrows add-ons), removed with [`Self::execute`]'s
    /// rules. Returns its id. `NotFound` if nothing installed matches; `Validation` if several
    /// builds do (`details.candidates`, newest last). The in-use `Conflict` names what to change.
    pub async fn execute_matching(&self, request: UninstallPackageRequest, ctx: &CallContext) -> AppResult<PackageIdDto> {
        let kind = PackageKind::new(request.kind.trim())?;
        let php = request.php_version.as_deref().map(|p| PhpMinor::parse(p.trim())).transpose()?;
        let version = request.version.as_deref().map(|v| PackageVersion::parse(v.trim())).transpose()?;
        let series = request.series.as_deref().map(|s| parse_series(s.trim())).transpose()?;
        let mut matching: Vec<PackageId> = self
            .installed
            .list()?
            .into_iter()
            .map(|p| p.id)
            .filter(|id| id.kind == kind && php.is_none_or(|php| id.php == Some(php)) && version.as_ref().is_none_or(|v| &id.version == v) && series.is_none_or(|s| id.series() == s))
            .collect();
        matching.sort();
        let wanted = [Some(kind.to_string()), request.version.clone().or(request.series.clone()), php.map(|p| format!("for PHP {p}"))].into_iter().flatten().collect::<Vec<_>>().join(" ");
        match matching.as_slice() {
            [] => Err(AppError::new(ErrorCode::NotFound, format!("no installed package matches `{wanted}` (see `lampo package list --installed`)"))),
            [id] => {
                let dto = package_id_to_dto(id);
                self.execute(dto.clone(), ctx).await.map_err(|e| match e.code {
                    ErrorCode::Conflict => AppError {
                        message: format!(
                            "{}: move those sites to another build first (`lampo project set-php`, `lampo project set --no-addon/--no-service`) or keep it; `system` is the default PHP or an always-on MySQL, changed in the Lampo settings",
                            e.message
                        ),
                        ..e
                    },
                    _ => e,
                })?;
                Ok(dto)
            }
            many => {
                let names: Vec<String> = many.iter().map(ToString::to_string).collect();
                Err(AppError::new(
                    ErrorCode::Validation,
                    format!("{} installed packages match `{wanted}` ({}): name the exact version (and --php for add-ons)", many.len(), names.join(", ")),
                )
                .with_details(serde_json::json!({ "candidates": many.iter().map(package_id_to_dto).collect::<Vec<_>>() })))
            }
        }
    }
}

/// The error of a failed removal of `id`. Files still in use (`Io`) after moving what used them
/// off the package failed are explained by that failure: its error is returned (the removal's own
/// guess, "is a service using it?", is only logged).
fn removal_error(id: &PackageId, removal: AppError, move_off: Option<AppError>) -> AppError {
    match move_off {
        Some(sync) if removal.code == ErrorCode::Io => {
            tracing::warn!("removing the files of {id} failed: {}", removal.message);
            AppError { message: format!("cannot uninstall {id}: moving its sites and services off it failed first: {}", sync.message), ..sync }
        }
        _ => removal,
    }
}
