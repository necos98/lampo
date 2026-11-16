//! Removing an installed package: the GUI "Pacchetti" page and `lampo package uninstall`.

use std::sync::Arc;

use lampo_domain::events::DomainEvent;
use lampo_domain::package::{InstalledPackage, PackageId, PackageKind};
use lampo_domain::php::PhpMinor;
use lampo_domain::ports::{EventPublisher, InstalledPackageRegistry, ProjectRepository, SettingsRepository, WorktreeRepository};
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
    pub projects: Arc<dyn ProjectRepository>,
    pub worktrees: Arc<dyn WorktreeRepository>,
    pub settings: Arc<dyn SettingsRepository>,
    pub events: Arc<dyn EventPublisher>,
    pub env: Arc<dyn EnvironmentSync>,
    pub services: Arc<ServiceCatalog>,
}

impl UninstallPackage {
    pub fn new(deps: UninstallPackageDeps) -> Self {
        let UninstallPackageDeps { installed, projects, worktrees, settings, events, env, services } = deps;
        Self { installed, projects, worktrees, settings, events, env, services }
    }

    /// `Conflict` if it is the last installed build satisfying something in use (a site's PHP
    /// minor, MySQL series, add-on…, the always-on MySQL or the default PHP). `NotFound` if not
    /// installed.
    ///
    /// Long-running (services restart, thousands of files are deleted): front-ends run it as a
    /// task. The environment is first synced without the package, so what used it (a vhost on
    /// `php-8.3.15\php-cgi.exe`, its FastCGI processes, a service) moves to the build that
    /// remains, or stops, before the files are deleted. That sync is best effort: its failure is
    /// logged and the removal still runs (files still in use then fail it with `Io`). A failed
    /// removal leaves the package installed, so the environment is synced back onto it.
    /// Publishes `PackagesChanged` once removed.
    pub async fn execute(&self, id: PackageIdDto, ctx: &CallContext) -> AppResult<()> {
        let id = package_id_from_dto(&id)?;
        self.ensure_removable(&id)?;
        if let Err(e) = self.env.sync_without(&id, ctx).await {
            tracing::warn!("environment sync before uninstalling {id} failed: {}", e.message);
        }
        if let Err(e) = self.installed.remove(&id).await {
            if let Err(sync) = self.env.sync(ctx).await {
                tracing::warn!("environment sync after failing to uninstall {id} failed: {}", sync.message);
            }
            return Err(e.into());
        }
        self.events.publish(DomainEvent::PackagesChanged);
        Ok(())
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
