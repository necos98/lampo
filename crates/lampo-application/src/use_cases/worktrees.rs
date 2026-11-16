//! Worktree environments: per-git-worktree sites for parallel coding agents. No hosts-file
//! changes ever (loopback port + `*.localhost`), so these never need elevation.

use std::sync::Arc;

use lampo_domain::events::DomainEvent;
use lampo_domain::mysql::DatabaseName;
use lampo_domain::ports::{
    Clock, DatabaseAdmin, EventPublisher, FileSystemInspector, GitInspector, InstalledPackageRegistry, ProgressSink, ProjectRepository, SettingsRepository, WorktreeRepository,
};
use lampo_domain::progress::{CancelFlag, Progress, ProgressStage};
use lampo_domain::project::{Project, ProjectRoot};
use lampo_domain::resolution::git;
use lampo_domain::settings::Settings;
use lampo_domain::site::SiteId;
use lampo_domain::worktree::{DatabaseMode, Occupancy, WorktreeDatabase, WorktreeEnvironment};
use lampo_domain::PortError;

use super::allocation::LoopbackPortAllocator;
use super::mysql_accounts::ConnectionPasswords;
use super::environment::EnvironmentSync;
use super::service_catalog::ServiceCatalog;
use super::service_leases::ServiceLease;
use super::services::ServiceEnsurer;
use super::support::{load_project, mysql_endpoint, sync_without_elevation, validation, worktree_dto};
use crate::context::CallContext;
use crate::dto::*;
use crate::error::{AppError, AppResult, ErrorCode};
use crate::mapping::database_mode_from_dto;

/// Read-only listing (works without the app). `stale` = folder missing.
pub struct WorktreeQueries {
    worktrees: Arc<dyn WorktreeRepository>,
    projects: Arc<dyn ProjectRepository>,
    settings: Arc<dyn SettingsRepository>,
    fs: Arc<dyn FileSystemInspector>,
    /// The passwords of the connection details shown.
    passwords: Arc<dyn ConnectionPasswords>,
}

impl WorktreeQueries {
    pub fn new(
        worktrees: Arc<dyn WorktreeRepository>,
        projects: Arc<dyn ProjectRepository>,
        settings: Arc<dyn SettingsRepository>,
        fs: Arc<dyn FileSystemInspector>,
        passwords: Arc<dyn ConnectionPasswords>,
    ) -> Self {
        Self { worktrees, projects, settings, fs, passwords }
    }

    /// All environments, or those of `project_id` (`NotFound` if unknown). Environments whose
    /// parent project no longer exists are omitted.
    pub fn list(&self, project_id: Option<&str>) -> AppResult<Vec<WorktreeEnvironmentDto>> {
        let settings = self.settings.load()?;
        let projects = self.projects.list()?;
        let envs = match project_id {
            Some(raw) => self.worktrees.list_for(&load_project(self.projects.as_ref(), raw)?.id)?,
            None => self.worktrees.list_all()?,
        };
        let mut out: Vec<WorktreeEnvironmentDto> = envs
            .iter()
            .filter_map(|w| projects.iter().find(|p| p.id == w.parent).map(|parent| worktree_dto(w, parent, &settings, Some(self.fs.as_ref()), self.passwords.as_ref())))
            .collect();
        out.sort_by(|a, b| a.site.cmp(&b.site));
        Ok(out)
    }

    /// The view of one environment of `parent`.
    pub(crate) fn present(&self, env: &WorktreeEnvironment, parent: &Project) -> AppResult<WorktreeEnvironmentDto> {
        Ok(self.present_in(env, parent, &self.settings.load()?))
    }

    /// [`Self::present`] with the `settings` at hand (a commit's, so nothing is read again).
    pub(crate) fn present_in(&self, env: &WorktreeEnvironment, parent: &Project, settings: &Settings) -> WorktreeEnvironmentDto {
        worktree_dto(env, parent, settings, Some(self.fs.as_ref()), self.passwords.as_ref())
    }
}

/// The database side of worktree environments, shared by `up`, `down` and `prune`: locating the
/// parent's MySQL instance, creating/cloning an environment's own database, dropping it.
pub struct WorktreeDatabases {
    projects: Arc<dyn ProjectRepository>,
    settings: Arc<dyn SettingsRepository>,
    installed: Arc<dyn InstalledPackageRegistry>,
    database: Arc<dyn DatabaseAdmin>,
    services: Arc<dyn ServiceEnsurer>,
    catalog: Arc<ServiceCatalog>,
}

impl WorktreeDatabases {
    pub fn new(
        projects: Arc<dyn ProjectRepository>,
        settings: Arc<dyn SettingsRepository>,
        installed: Arc<dyn InstalledPackageRegistry>,
        database: Arc<dyn DatabaseAdmin>,
        services: Arc<dyn ServiceEnsurer>,
        catalog: Arc<ServiceCatalog>,
    ) -> Self {
        Self { projects, settings, installed, database, services, catalog }
    }

    /// Starts the parent's MySQL instance if needed and returns its endpoint, with the lease
    /// that keeps it running (no sync restarts it) while the caller uses it.
    async fn mysql_for(&self, parent: &Project) -> AppResult<(lampo_domain::ports::MysqlEndpoint, ServiceLease)> {
        let series = parent.environment.mysql().ok_or_else(|| validation(format!("project `{}` does not use MySQL", parent.id)))?;
        let endpoint = mysql_endpoint(self.installed.as_ref(), &self.settings.load()?, &self.catalog, series)?;
        let lease = self.services.ensure_running(series.service_id()).await?;
        Ok((endpoint, lease))
    }

    /// The databases already on the parent's instance, which a new environment's own database
    /// must not adopt (`WorktreeEnvironment::derive` picks another name). Empty for `Shared` and
    /// for a parent without MySQL (which `derive` rejects for the other modes).
    async fn existing_on_instance(&self, parent: &Project, mode: &DatabaseMode) -> AppResult<Vec<String>> {
        if *mode == DatabaseMode::Shared || parent.environment.mysql().is_none() {
            return Ok(Vec::new());
        }
        let (endpoint, _lease) = self.mysql_for(parent).await?;
        Ok(self.database.list(&endpoint).await?)
    }

    /// Creates the environment's own database. Its name was free when the environment was
    /// derived: one that exists by now was created by another client and is reported as
    /// `Conflict`, never adopted nor dropped. On any other failure the half-created database is
    /// dropped (best effort).
    async fn provision(&self, env: &WorktreeEnvironment, parent: &Project, progress: &dyn ProgressSink, cancel: &CancelFlag) -> AppResult<()> {
        let WorktreeDatabase::Own { name, cloned_from } = &env.database else { return Ok(()) };
        let (endpoint, _lease) = self.mysql_for(parent).await?;
        cancel.check()?;
        let subject = env.site_id().to_string();
        match cloned_from {
            None => Ok(self.database.create(&endpoint, name).await?),
            Some(source) => {
                if !self.database.list(&endpoint).await?.iter().any(|d| d == source.as_str()) {
                    return Err(AppError::not_found("database", source));
                }
                progress.report(Progress::new(ProgressStage::Configuring, 0, None).with_subject(subject));
                let result = self.database.clone_database(&endpoint, source, name, progress, cancel).await;
                let partial = result.as_ref().is_err_and(|e| !matches!(e, PortError::AlreadyExists { .. }));
                if partial {
                    if let Err(e) = self.database.drop_database(&endpoint, name).await {
                        tracing::warn!("could not drop the partial clone `{name}`: {e}");
                    }
                }
                Ok(result?)
            }
        }
    }

    /// Drops the environment's own database (never the parent's). `Ok(None)` if it has none.
    async fn drop_owned(&self, env: &WorktreeEnvironment) -> AppResult<Option<DatabaseName>> {
        let Some(name) = env.owned_database() else { return Ok(None) };
        let parent = load_project(self.projects.as_ref(), env.parent.as_str())?;
        let (endpoint, _lease) = self.mysql_for(&parent).await?;
        self.database.drop_database(&endpoint, name).await?;
        Ok(Some(name.clone()))
    }
}

pub struct WorktreeUpDeps {
    pub projects: Arc<dyn ProjectRepository>,
    pub worktrees: Arc<dyn WorktreeRepository>,
    pub git: Arc<dyn GitInspector>,
    pub clock: Arc<dyn Clock>,
    /// Shared with `CreateProject`: picks the port and saves the environment atomically.
    pub allocator: Arc<LoopbackPortAllocator>,
    pub databases: Arc<WorktreeDatabases>,
    pub queries: Arc<WorktreeQueries>,
    pub env: Arc<dyn EnvironmentSync>,
    pub events: Arc<dyn EventPublisher>,
}

/// Where a worktree environment goes: its parent project, its folder and branch.
struct Placement {
    parent: Project,
    root: ProjectRoot,
    branch: Option<String>,
}

/// `lampo worktree up [path]` — idempotent.
pub struct WorktreeUp {
    deps: WorktreeUpDeps,
}

impl WorktreeUp {
    pub fn new(deps: WorktreeUpDeps) -> Self {
        Self { deps }
    }

    /// Finds the parent and the environment folder. In a monorepo (the project is a subfolder
    /// of the main repository) the environment folder is the same subfolder of the worktree.
    fn place(&self, request: &WorktreeUpRequest) -> AppResult<Placement> {
        let path = request.path.trim();
        let checkout = self.deps.git.inspect(path)?;
        let explicit = request.project_id.as_deref().map(str::trim).filter(|s| !s.is_empty());
        let linked = checkout.as_ref().and_then(|c| c.main_root.clone().map(|main| (c.worktree_root.clone(), main)));

        let Some((worktree_root, main_root)) = linked else {
            let Some(raw) = explicit else {
                return Err(validation(format!("`{path}` is not inside a linked git worktree: pass the parent project explicitly")));
            };
            let parent = load_project(self.deps.projects.as_ref(), raw)?;
            let root = match &checkout {
                Some(c) => c.worktree_root.clone(),
                None => ProjectRoot::new(path)?,
            };
            return Ok(Placement { parent, root, branch: checkout.and_then(|c| c.branch) });
        };

        let mapped = git::map_into(path, &worktree_root, &main_root).unwrap_or_else(|| main_root.to_string());
        let projects = self.deps.projects.list()?;
        let parent = match explicit {
            Some(raw) => load_project(self.deps.projects.as_ref(), raw)?,
            None => projects
                .iter()
                .filter(|p| p.root.contains(&mapped) || p.root.contains(main_root.as_str()))
                .max_by_key(|p| p.root.depth())
                .cloned()
                .ok_or_else(|| AppError::new(ErrorCode::ProjectNotResolved, format!("no registered project contains the main repository `{main_root}`")))?,
        };
        // Rebase the project's position inside the main repository onto the worktree.
        let root = match main_root.contains(parent.root.as_str()) {
            true => git::map_into(parent.root.as_str(), &main_root, &worktree_root).map(ProjectRoot::new).transpose()?.unwrap_or(worktree_root.clone()),
            false => worktree_root.clone(),
        };
        Ok(Placement { parent, root, branch: checkout.and_then(|c| c.branch) })
    }

    /// 1. `git.inspect(path)` → worktree root, main repo, branch (`Validation` if not a linked
    ///    worktree and no `project_id` given);
    /// 2. parent = `project_id` or the project containing the main repo (`ProjectNotResolved`);
    /// 3. existing environment for that folder → return it (`created: false`), re-syncing;
    /// 4. `Fresh`/`Clone`: ensure MySQL running and list its databases (never adopted);
    /// 5. allocate a loopback port and save the derived environment, atomically
    ///    ([`LoopbackPortAllocator::allocate`]): `Conflict` if the folder is a registered
    ///    project's; slug, alias and database name are free across every site;
    /// 6. database: `Fresh` → create; `Clone` → `clone_database` (progress reported); on failure
    ///    (or cancellation) the environment is removed again;
    /// 7. publish `WorktreesChanged`, sync (never needs elevation).
    pub async fn execute(&self, request: WorktreeUpRequest, ctx: &CallContext, progress: &dyn ProgressSink, cancel: &CancelFlag) -> AppResult<WorktreeUpResultDto> {
        let d = &self.deps;
        let placement = self.place(&request)?;
        let mode = database_mode_from_dto(&request.database)?;

        // `sites`: the registry after the commit, when there was one.
        let (env, created, sites) = match d.worktrees.find_by_root(&placement.root)? {
            Some(existing) => (existing, false, None),
            None => {
                let server_databases = d.databases.existing_on_instance(&placement.parent, &mode).await?;
                d.allocator
                    .allocate(|port, snap| {
                        // Re-checked under the lock: a concurrent `up` may have created it.
                        if let Some(existing) = snap.worktrees.iter().find(|w| w.root == placement.root) {
                            return Ok((existing.clone(), false, Some(snap.unchanged())));
                        }
                        let taken = Occupancy { projects: &snap.projects, worktrees: &snap.worktrees, server_databases: &server_databases };
                        let env = WorktreeEnvironment::derive(&placement.parent, &taken, placement.root.clone(), placement.branch.clone(), port, &mode, d.clock.now_unix_ms())?;
                        d.worktrees.save(&env)?;
                        let sites = snap.with_worktree(&env);
                        Ok((env, true, Some(sites)))
                    })
                    .await?
            }
        };
        let parent = if env.parent == placement.parent.id { placement.parent } else { load_project(d.projects.as_ref(), env.parent.as_str())? };

        if created {
            if let Err(e) = d.databases.provision(&env, &parent, progress, cancel).await {
                if let Err(cleanup) = d.allocator.write(|| Ok(d.worktrees.delete(&env.parent, &env.slug)?)).await {
                    tracing::warn!("could not remove the failed environment {}: {}", env.site_id(), cleanup.message);
                }
                return Err(e);
            }
            d.events.publish(DomainEvent::WorktreesChanged { project: env.parent.clone() });
        }
        let environment = match &sites {
            Some(sites) => d.queries.present_in(&env, &parent, &sites.settings),
            None => d.queries.present(&env, &parent)?,
        };
        sync_without_elevation(d.env.as_ref(), sites, ctx).await?;
        Ok(WorktreeUpResultDto { environment, created })
    }
}

/// Finds an environment by site id (`shop@fix-1`), by its folder, or by a folder inside it.
fn find_environment(worktrees: &dyn WorktreeRepository, target: &str) -> AppResult<WorktreeEnvironment> {
    let target = target.trim();
    if let Ok(SiteId::Worktree { parent, slug }) = target.parse::<SiteId>() {
        return worktrees.get(&parent, &slug)?.ok_or_else(|| AppError::not_found("worktree environment", target));
    }
    let root = ProjectRoot::new(target).map_err(|_| AppError::not_found("worktree environment", target))?;
    if let Some(env) = worktrees.find_by_root(&root)? {
        return Ok(env);
    }
    worktrees.list_all()?.into_iter().filter(|w| w.root.contains(root.as_str())).max_by_key(|w| w.root.depth()).ok_or_else(|| AppError::not_found("worktree environment", target))
}

/// `lampo worktree down <path|site>`.
pub struct WorktreeDown {
    worktrees: Arc<dyn WorktreeRepository>,
    /// Unregisters under its lock, like every registry write.
    allocator: Arc<LoopbackPortAllocator>,
    databases: Arc<WorktreeDatabases>,
    env: Arc<dyn EnvironmentSync>,
    events: Arc<dyn EventPublisher>,
}

impl WorktreeDown {
    pub fn new(
        worktrees: Arc<dyn WorktreeRepository>,
        allocator: Arc<LoopbackPortAllocator>,
        databases: Arc<WorktreeDatabases>,
        env: Arc<dyn EnvironmentSync>,
        events: Arc<dyn EventPublisher>,
    ) -> Self {
        Self { worktrees, allocator, databases, env, events }
    }

    /// Removes the environment; drops its *own* database only if `drop_database` (first, so a
    /// failed drop leaves the environment in place for a retry).
    pub async fn execute(&self, request: WorktreeDownRequest, ctx: &CallContext) -> AppResult<WorktreeDownResultDto> {
        let env = find_environment(self.worktrees.as_ref(), &request.target)?;
        let dropped = if request.drop_database { self.databases.drop_owned(&env).await? } else { None };
        self.allocator.write(|| Ok(self.worktrees.delete(&env.parent, &env.slug)?)).await?;
        self.events.publish(DomainEvent::WorktreesChanged { project: env.parent.clone() });
        sync_without_elevation(self.env.as_ref(), None, ctx).await?;
        Ok(WorktreeDownResultDto { site: env.site_id().to_string(), database_dropped: dropped.map(|n| n.to_string()) })
    }
}

/// `lampo worktree prune`: removes stale environments (`WorktreeEnvironment::is_stale`: folder
/// gone, or parent project gone, which can never be served).
pub struct PruneWorktrees {
    projects: Arc<dyn ProjectRepository>,
    worktrees: Arc<dyn WorktreeRepository>,
    /// Unregisters under its lock, like every registry write.
    allocator: Arc<LoopbackPortAllocator>,
    fs: Arc<dyn FileSystemInspector>,
    databases: Arc<WorktreeDatabases>,
    env: Arc<dyn EnvironmentSync>,
    events: Arc<dyn EventPublisher>,
}

impl PruneWorktrees {
    pub fn new(
        projects: Arc<dyn ProjectRepository>,
        worktrees: Arc<dyn WorktreeRepository>,
        allocator: Arc<LoopbackPortAllocator>,
        fs: Arc<dyn FileSystemInspector>,
        databases: Arc<WorktreeDatabases>,
        env: Arc<dyn EnvironmentSync>,
        events: Arc<dyn EventPublisher>,
    ) -> Self {
        Self { projects, worktrees, allocator, fs, databases, env, events }
    }

    /// Environments that `prune` would remove.
    fn stale(&self, project_id: Option<&str>) -> AppResult<Vec<WorktreeEnvironment>> {
        let projects = self.projects.list()?;
        let envs = match project_id.map(str::trim).filter(|s| !s.is_empty()) {
            Some(raw) => self.worktrees.list_for(&load_project(self.projects.as_ref(), raw)?.id)?,
            None => self.worktrees.list_all()?,
        };
        Ok(envs.into_iter().filter(|w| w.is_stale(&projects, |root| self.fs.is_dir(root.as_path()))).collect())
    }

    /// Removes every stale environment it can; one that fails (its database cannot be dropped,
    /// its config cannot be deleted) is reported in `failed` and stays registered for a retry,
    /// while the others are removed, published and synced all the same.
    pub async fn execute(&self, request: PruneWorktreesRequest, ctx: &CallContext) -> AppResult<PruneReportDto> {
        let stale = self.stale(request.project_id.as_deref())?;
        let mut report = PruneReportDto { dry_run: request.dry_run, removed: Vec::new(), dropped_databases: Vec::new(), failed: Vec::new() };
        if request.dry_run {
            report.removed = stale.iter().map(|w| w.site_id().to_string()).collect();
            report.dropped_databases = if request.drop_databases { stale.iter().filter_map(|w| w.owned_database()).map(|n| n.to_string()).collect() } else { Vec::new() };
            return Ok(report);
        }
        let mut parents = std::collections::BTreeSet::new();
        for env in &stale {
            match self.prune_one(env, request.drop_databases, &mut report).await {
                Ok(()) => {
                    parents.insert(env.parent.clone());
                }
                Err(e) => {
                    tracing::warn!("could not prune {}: {e}", env.site_id());
                    report.failed.push(PruneFailureDto { site: env.site_id().to_string(), code: e.code, message: e.message });
                }
            }
        }
        for project in parents {
            self.events.publish(DomainEvent::WorktreesChanged { project });
        }
        if !report.removed.is_empty() {
            sync_without_elevation(self.env.as_ref(), None, ctx).await?;
        }
        Ok(report)
    }

    /// Drops the environment's own database (if asked), then unregisters it, recording both in
    /// `report`. A failed drop leaves it registered.
    async fn prune_one(&self, env: &WorktreeEnvironment, drop_database: bool, report: &mut PruneReportDto) -> AppResult<()> {
        // An orphan's MySQL series is unknown: its database cannot be located.
        if drop_database && self.projects.get(&env.parent)?.is_some() {
            if let Some(name) = self.databases.drop_owned(env).await? {
                report.dropped_databases.push(name.to_string());
            }
        }
        self.allocator.write(|| Ok(self.worktrees.delete(&env.parent, &env.slug)?)).await?;
        report.removed.push(env.site_id().to_string());
        Ok(())
    }
}
