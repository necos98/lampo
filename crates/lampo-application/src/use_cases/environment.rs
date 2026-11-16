//! Reconciling the machine with the configuration: the heart of Lampo, and the single owner of
//! the service lifecycle (full syncs and single-service starts/restarts/stops share one lock).

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use async_trait::async_trait;
use lampo_domain::hosts::HostsBlock;
use lampo_domain::package::PackageId;
use lampo_domain::paths::LampoPaths;
use lampo_domain::ports::{
    EventPublisher, HostsFileReader, HostsFileWriter, InstalledPackageRegistry, PhpIniWriter, PortAvailability, PortProbe, ProjectRepository, ServiceRunner, SettingsRepository,
    ShimIndexWriter, WebServerConfigurator, WorktreeRepository,
};
use lampo_domain::service::{ServiceId, ServiceLaunch, ServiceState, plan_services};
use lampo_domain::tcp_port::Port;
use lampo_domain::web::{PhpIniSpec, WebServerConfig};
use lampo_domain::PortError;

use super::allocation::SiteSnapshot;
use super::generation::{ConfigSnapshot, php_ini_specs, runner_for, shim_index, web_config};
use super::service_catalog::ServiceCatalog;
use super::service_leases::{ServiceLease, ServiceLeases};
use super::support::sync_without_elevation;
use super::sync_outcome::Outcome;
use super::web_apply::apply_web_config;
use super::web_state::WebState;
use crate::blocking::in_place;
use crate::context::{CallContext, ElevationPolicy};
use crate::dto::SyncReportDto;
use crate::error::{AppError, AppResult, ErrorCode};

/// "Make the machine match the config." Called after every mutation by the use cases that
/// change projects, worktrees, settings or packages (dependency inversion: they do not know
/// [`ApplyEnvironment`]).
#[async_trait]
pub trait EnvironmentSync: Send + Sync {
    async fn sync(&self, ctx: &CallContext) -> AppResult<SyncReportDto>;

    /// [`Self::sync`] after a commit that handed on `sites`, the registry as it saved it: used
    /// instead of reading the projects, worktree environments and settings again while it is
    /// still the newest state ([`SiteSnapshot::is_current`]), read again otherwise.
    async fn sync_sites(&self, _sites: SiteSnapshot, ctx: &CallContext) -> AppResult<SyncReportDto> {
        self.sync(ctx).await
    }

    /// [`Self::sync`] as if `retiring` were no longer installed: vhosts, php.ini files, CLI
    /// shims and services using it move to the build that remains (restarted) or stop, so its
    /// files are no longer in use when the uninstall deletes them.
    async fn sync_without(&self, retiring: &PackageId, ctx: &CallContext) -> AppResult<SyncReportDto>;
}

/// One service outside a full sync (`lampo up/down/restart <service>`, database operations),
/// serialised with the syncs so that both agree on what runs and with which launch.
#[async_trait]
pub trait ServiceLifecycle: Send + Sync {
    /// Starts `service` unless it is up (its ports checked first: `PortInUse`). `Ok(true)` when
    /// it was started now.
    async fn ensure(&self, service: &ServiceId) -> AppResult<bool>;

    /// [`Self::ensure`], then a lease keeping `service` running as it is until dropped: syncs
    /// postpone stopping or restarting it, and apply that once the last lease is released.
    async fn lease(&self, service: &ServiceId) -> AppResult<ServiceLease>;

    /// Restarts `service` with its current configuration. `Conflict` while it is leased.
    async fn restart(&self, service: &ServiceId) -> AppResult<()>;

    /// Stops `service` unless it is stopped. `Ok(true)` when it was stopped now; `Conflict`
    /// while it is leased.
    async fn stop(&self, service: &ServiceId) -> AppResult<bool>;
}

pub struct ApplyEnvironmentDeps {
    pub projects: Arc<dyn ProjectRepository>,
    pub worktrees: Arc<dyn WorktreeRepository>,
    pub settings: Arc<dyn SettingsRepository>,
    pub installed: Arc<dyn InstalledPackageRegistry>,
    pub web_config: Arc<dyn WebServerConfigurator>,
    pub php_ini: Arc<dyn PhpIniWriter>,
    pub hosts_reader: Arc<dyn HostsFileReader>,
    pub hosts_writer: Arc<dyn HostsFileWriter>,
    pub shim_index: Arc<dyn ShimIndexWriter>,
    /// The registered service kinds.
    pub services: Arc<ServiceCatalog>,
    /// One runner per service kind.
    pub runners: Vec<Arc<dyn ServiceRunner>>,
    pub port_probe: Arc<dyn PortProbe>,
    pub paths: LampoPaths,
    pub events: Arc<dyn EventPublisher>,
}

#[derive(Default)]
struct SyncState {
    web: Option<WebState>,
    /// Launch last used for each service this instance started or restarted.
    launches: HashMap<ServiceId, ServiceLaunch>,
}

/// `Running` or `Starting`.
pub(crate) fn is_up(state: ServiceState) -> bool {
    matches!(state, ServiceState::Running | ServiceState::Starting)
}

/// Steps, in order (each idempotent):
/// 1. load projects, worktree environments, settings, installed packages;
/// 2. write php.ini for every site and the default CLI ini, then regenerate the shim index;
/// 3. build `WebServerConfig` (vhosts for active projects + all worktree environments) and
///    `apply` it (staged + `httpd -t`). If Apache rejects it, the broken vhosts are isolated
///    (`web_apply`), left out and the rest is applied, so one broken project never takes the
///    others down; the rejection is still returned at the end. An unchanged rejected
///    configuration is not validated again, an apply that failed for another reason (I/O, the
///    validator not starting) is retried on every sync;
/// 4. hosts block: compute `HostsBlock::for_projects`; if different from `read_block`, write it —
///    only when `ctx.elevation == Allow`, otherwise remember the failure and continue;
/// 5. `plan_services`: stop unneeded services, start missing ones (port check first), restart
///    the ones whose configuration changed (Apache: config or php.ini). A leased service is
///    neither stopped nor restarted: a warning says so and the change waits for the release;
/// 6. return the report, or the first failure (`ConfigRejected`, `ElevationRequired`,
///    `ElevationDenied`, `PortInUse`, …) with the report in `details.report` and every failure
///    in `details.errors`.
///
/// Missing packages are warnings, not errors. Serialises concurrent calls, and the
/// [`ServiceLifecycle`] calls, internally.
pub struct ApplyEnvironment {
    deps: ApplyEnvironmentDeps,
    lock: tokio::sync::Mutex<SyncState>,
    leases: Arc<ServiceLeases>,
}

impl ApplyEnvironment {
    pub fn new(deps: ApplyEnvironmentDeps) -> Self {
        Self { deps, lock: tokio::sync::Mutex::new(SyncState::default()), leases: Arc::default() }
    }

    /// Applies, with a sync, the changes postponed because a lease held the service, each time
    /// the last such lease is released. Never returns: the composition root spawns it for the
    /// app's lifetime. Never prompts for elevation; failures are only logged.
    pub async fn apply_deferred_changes(&self) {
        loop {
            self.leases.deferred_released().await;
            if let Err(e) = sync_without_elevation(self, None, &CallContext::GUI).await {
                tracing::warn!("applying the service changes postponed by a database operation failed: {}", e.message);
            }
        }
    }

    /// The configuration and the installed packages, leaving out `retiring` (being uninstalled).
    /// `sites` (handed on by a commit) stands in for the projects, worktree environments and
    /// settings while it is the newest state of the registry; they are read otherwise.
    fn snapshot(&self, retiring: Option<&PackageId>, sites: Option<SiteSnapshot>) -> AppResult<ConfigSnapshot> {
        let mut installed = self.deps.installed.list()?;
        if let Some(retiring) = retiring {
            installed.retain(|p| &p.id != retiring);
        }
        let (projects, worktrees, settings) = match sites.filter(SiteSnapshot::is_current) {
            Some(sites) => (sites.projects, sites.worktrees, sites.settings),
            None => (self.deps.projects.list()?, self.deps.worktrees.list_all()?, self.deps.settings.load()?),
        };
        Ok(ConfigSnapshot { projects, worktrees, settings, installed })
    }

    /// Writes the php.ini files and the shim index; returns the specs written.
    fn write_files(&self, snap: &ConfigSnapshot, warnings: &mut Vec<String>) -> AppResult<Vec<PhpIniSpec>> {
        let specs = php_ini_specs(snap, &self.deps.services, &self.deps.paths, warnings);
        for spec in &specs {
            if let Err(e) = self.deps.php_ini.write(spec) {
                warnings.push(format!("php.ini of `{}` not written: {e}", spec.site));
            }
        }
        self.deps.shim_index.write(&shim_index(snap, &self.deps.services, &self.deps.paths))?;
        Ok(specs)
    }

    /// Only steps 1–2 (no processes, no elevation): used by the CLI fallback and at startup
    /// before services come up.
    pub fn write_generated_files(&self) -> AppResult<()> {
        let snap = self.snapshot(None, None)?;
        let mut warnings = Vec::new();
        self.write_files(&snap, &mut warnings)?;
        for w in warnings {
            tracing::warn!("{w}");
        }
        Ok(())
    }

    async fn apply_web(&self, desired: &WebServerConfig, ini: &[PhpIniSpec]) -> WebState {
        WebState { desired: desired.clone(), ini: ini.to_vec(), outcome: apply_web_config(self.deps.web_config.as_ref(), desired).await }
    }

    async fn sync_hosts(&self, snap: &ConfigSnapshot, ctx: &CallContext, out: &mut Outcome) {
        let desired = HostsBlock::for_projects(&snap.projects);
        let current = match self.deps.hosts_reader.read_block() {
            Ok(block) => block.unwrap_or_default(),
            Err(e) => {
                out.report.warnings.push(format!("hosts file not readable: {e}"));
                return;
            }
        };
        if desired == current {
            return;
        }
        match ctx.elevation {
            ElevationPolicy::Deny => {
                out.fail(AppError::new(ErrorCode::ElevationRequired, "the hosts file must be updated, which needs administrator permission: re-run with --allow-elevation"))
            }
            ElevationPolicy::Allow => match self.deps.hosts_writer.write_block(&desired, &snap.settings.tld).await {
                Ok(()) => out.report.hosts_file_updated = true,
                Err(e) => out.fail(e.into()),
            },
        }
    }

    fn port_busy(&self, ports: &[Port]) -> Option<AppError> {
        ports.iter().find_map(|&port| match self.deps.port_probe.check(port) {
            Ok(PortAvailability::InUse { owner }) => Some(PortError::PortInUse { port: port.get(), owner }.into()),
            Ok(PortAvailability::Free) => None,
            Err(e) => {
                tracing::warn!("cannot probe port {port}: {e}");
                None
            }
        })
    }

    fn runner(&self, id: &ServiceId) -> AppResult<&Arc<dyn ServiceRunner>> {
        runner_for(&self.deps.runners, id.kind()).ok_or_else(|| AppError::new(ErrorCode::Unsupported, format!("no runner is registered for {id}")))
    }

    /// The launch of `id` for the current configuration.
    fn current_launch(&self, id: &ServiceId) -> AppResult<ServiceLaunch> {
        self.deps.services.launch(id, &self.deps.settings.load()?, &self.deps.installed.list()?, &self.deps.paths)
    }

    /// `PortInUse` if one of `ports` is taken, otherwise `prepare` + `start`.
    async fn start_with(&self, runner: &dyn ServiceRunner, launch: &ServiceLaunch, ports: &[Port]) -> AppResult<()> {
        if let Some(busy) = self.port_busy(ports) {
            return Err(busy);
        }
        runner.prepare(launch).await?;
        Ok(runner.start(launch).await?)
    }

    async fn restart_with(runner: &dyn ServiceRunner, launch: &ServiceLaunch) -> AppResult<()> {
        runner.prepare(launch).await?;
        Ok(runner.restart(launch).await?)
    }

    /// `Conflict` while `id` is leased.
    fn refuse_if_leased(&self, id: &ServiceId, action: &str) -> AppResult<()> {
        if !self.leases.is_leased(id) {
            return Ok(());
        }
        Err(AppError::new(ErrorCode::Conflict, format!("{id} is in use by a running database operation (import, export, clone): {action} it when the operation finishes")))
    }

    /// [`ServiceLifecycle::ensure`], with the lock held.
    async fn ensure_locked(&self, state: &mut SyncState, id: &ServiceId) -> AppResult<bool> {
        let runner = self.runner(id)?;
        if is_up(runner.status(id).state) {
            return Ok(false);
        }
        let launch = self.current_launch(id)?;
        self.start_with(runner.as_ref(), &launch, &launch.ports()).await?;
        state.launches.insert(id.clone(), launch);
        Ok(true)
    }

    async fn sync_services(&self, snap: &ConfigSnapshot, state: &mut SyncState, web_changed: bool, out: &mut Outcome) {
        let plan = plan_services(&snap.projects, &snap.worktrees, &snap.settings);
        let runners = &self.deps.runners;
        let catalog = &self.deps.services;

        // Stop what is no longer needed first (frees ports).
        let candidates: BTreeSet<ServiceId> = catalog.configured(&snap.settings).into_iter().chain(state.launches.keys().cloned()).collect();
        for id in candidates.iter().filter(|id| !plan.contains_key(id)) {
            let Some(runner) = runner_for(runners, id.kind()) else { continue };
            if is_up(runner.status(id).state) {
                if self.leases.defer(id) {
                    out.report.warnings.push(format!("{id} is no longer needed but a database operation is using it: it stops when the operation finishes"));
                    continue;
                }
                match runner.stop(id).await {
                    Ok(()) => out.report.stopped.push(id.to_string()),
                    Err(e) => out.fail(e.into()),
                }
            }
            state.launches.remove(id);
        }

        // Then start/restart what is needed, Apache last (after its databases).
        let mut needed: Vec<ServiceId> = plan.keys().filter(|id| !id.is_apache()).cloned().collect();
        if plan.contains_key(&ServiceId::apache()) {
            needed.push(ServiceId::apache());
        }
        for id in needed {
            let launch = match catalog.launch(&id, &snap.settings, &snap.installed, &self.deps.paths) {
                Ok(l) => l,
                Err(e) if e.code == ErrorCode::PackageMissing => {
                    out.report.warnings.push(format!("{id} cannot start: {}", e.message));
                    continue;
                }
                Err(e) => {
                    out.fail(e);
                    continue;
                }
            };
            let Some(runner) = runner_for(runners, id.kind()) else {
                out.report.warnings.push(format!("no runner is registered for {id}"));
                continue;
            };
            let web = id.is_apache().then_some(state.web.as_ref()).flatten();
            if id.is_apache() && web.is_none_or(|w| w.outcome.blocks_apache()) {
                continue;
            }
            if is_up(runner.status(&id).state) {
                let changed = match web {
                    Some(_) => web_changed,
                    None => state.launches.get(&id).is_some_and(|previous| previous != &launch),
                };
                if changed {
                    if self.leases.defer(&id) {
                        out.report.warnings.push(format!("{id} keeps its previous configuration while a database operation is using it: it restarts when the operation finishes"));
                        continue;
                    }
                    match Self::restart_with(runner.as_ref(), &launch).await {
                        Ok(()) => out.report.restarted.push(id.to_string()),
                        Err(e) => out.fail(e),
                    }
                }
            } else {
                let ports = match web {
                    Some(w) => w.desired.listen_ports(),
                    None => launch.ports(),
                };
                if let Err(e) = self.start_with(runner.as_ref(), &launch, &ports).await {
                    out.fail(e);
                    continue;
                }
                out.report.started.push(id.to_string());
            }
            state.launches.insert(id, launch);
        }
    }

    /// The steps of the type docs, against the installed packages minus `retiring` and, while it
    /// is current, the registry `sites` handed on by a commit. The store is read and the files
    /// are written off the runtime's worker ([`in_place`]).
    async fn reconcile(&self, retiring: Option<&PackageId>, sites: Option<SiteSnapshot>, ctx: &CallContext) -> AppResult<SyncReportDto> {
        let mut state = self.lock.lock().await;
        let mut out = Outcome::default();
        let snap = in_place(|| self.snapshot(retiring, sites))?;

        let ini = match in_place(|| self.write_files(&snap, &mut out.report.warnings)) {
            Ok(specs) => specs,
            Err(e) => {
                out.fail(e);
                Vec::new()
            }
        };

        let mut web_changed = false;
        match web_config(&snap, &self.deps.paths, &mut out.report.warnings) {
            Some(desired) => {
                let current = state.web.as_ref().is_some_and(|w| w.is_current(&desired, &ini));
                if !current {
                    state.web = Some(self.apply_web(&desired, &ini).await);
                    web_changed = true;
                }
                if let Some(error) = state.web.as_ref().and_then(|w| w.outcome.error()) {
                    out.fail(error.clone());
                }
            }
            None => state.web = None,
        }

        self.sync_hosts(&snap, ctx, &mut out).await;
        self.sync_services(&snap, &mut state, web_changed, &mut out).await;
        out.finish()
    }
}

#[async_trait]
impl EnvironmentSync for ApplyEnvironment {
    async fn sync(&self, ctx: &CallContext) -> AppResult<SyncReportDto> {
        self.reconcile(None, None, ctx).await
    }

    async fn sync_sites(&self, sites: SiteSnapshot, ctx: &CallContext) -> AppResult<SyncReportDto> {
        self.reconcile(None, Some(sites), ctx).await
    }

    async fn sync_without(&self, retiring: &PackageId, ctx: &CallContext) -> AppResult<SyncReportDto> {
        self.reconcile(Some(retiring), None, ctx).await
    }
}

#[async_trait]
impl ServiceLifecycle for ApplyEnvironment {
    async fn ensure(&self, service: &ServiceId) -> AppResult<bool> {
        let mut state = self.lock.lock().await;
        self.ensure_locked(&mut state, service).await
    }

    async fn lease(&self, service: &ServiceId) -> AppResult<ServiceLease> {
        let mut state = self.lock.lock().await;
        self.ensure_locked(&mut state, service).await?;
        // Taken before the lock is released: no sync can slip in between.
        Ok(self.leases.acquire(service.clone()))
    }

    async fn restart(&self, service: &ServiceId) -> AppResult<()> {
        let mut state = self.lock.lock().await;
        self.refuse_if_leased(service, "restart")?;
        let runner = self.runner(service)?;
        let launch = self.current_launch(service)?;
        Self::restart_with(runner.as_ref(), &launch).await?;
        state.launches.insert(service.clone(), launch);
        Ok(())
    }

    async fn stop(&self, service: &ServiceId) -> AppResult<bool> {
        let mut state = self.lock.lock().await;
        self.refuse_if_leased(service, "stop")?;
        let runner = self.runner(service)?;
        let stopped = runner.status(service).state != ServiceState::Stopped;
        if stopped {
            runner.stop(service).await?;
        }
        state.launches.remove(service);
        Ok(stopped)
    }
}
