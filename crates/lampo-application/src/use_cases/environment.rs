//! Reconciling the machine with the configuration: the heart of Lampo.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use async_trait::async_trait;
use lampo_domain::hosts::HostsBlock;
use lampo_domain::paths::LampoPaths;
use lampo_domain::ports::{
    EventPublisher, HostsFileReader, HostsFileWriter, InstalledPackageRegistry, PhpIniWriter, PortAvailability, PortProbe, ProjectRepository, ServiceRunner, SettingsRepository,
    ShimIndexWriter, WebServerConfigurator, WorktreeRepository,
};
use lampo_domain::service::{ServiceId, ServiceLaunch, ServiceState, plan_services};
use lampo_domain::web::{PhpIniSpec, WebServerConfig};
use lampo_domain::PortError;

use super::generation::{ConfigSnapshot, php_ini_specs, runner_for, shim_index, web_config};
use super::service_catalog::ServiceCatalog;
use super::web_state::{WebOutcome, WebState};
use crate::context::{CallContext, ElevationPolicy};
use crate::dto::SyncReportDto;
use crate::error::{AppError, AppResult, ErrorCode};

/// "Make the machine match the config." Called after every mutation by the use cases that
/// change projects, worktrees, settings or packages (dependency inversion: they do not know
/// [`ApplyEnvironment`]).
#[async_trait]
pub trait EnvironmentSync: Send + Sync {
    async fn sync(&self, ctx: &CallContext) -> AppResult<SyncReportDto>;
}

/// Regenerates the file-only artefacts (php.ini files, shim index) without touching processes
/// or the hosts file. Synchronous, so non-async use cases (package uninstall) can refresh the
/// shim index too. Implemented by [`ApplyEnvironment`].
pub trait GeneratedFilesWriter: Send + Sync {
    fn write_generated_files(&self) -> AppResult<()>;
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
    /// Launch last used for each service this instance started or adopted.
    launches: HashMap<ServiceId, ServiceLaunch>,
}

/// Collects the outcome of one sync. Failures that do not prevent the other steps are kept as
/// the *pending* error (the first one wins, later ones become warnings) and returned at the
/// end, after everything else was applied.
#[derive(Default)]
struct Outcome {
    report: SyncReportDto,
    pending: Option<AppError>,
}

impl Outcome {
    fn fail(&mut self, error: AppError) {
        match &self.pending {
            None => self.pending = Some(error),
            Some(_) => self.report.warnings.push(error.message),
        }
    }

    fn finish(self) -> AppResult<SyncReportDto> {
        match self.pending {
            None => Ok(self.report),
            Some(mut e) => {
                let report = serde_json::to_value(&self.report).unwrap_or_default();
                let mut details = match e.details.take() {
                    Some(serde_json::Value::Object(map)) => map,
                    _ => serde_json::Map::new(),
                };
                details.insert("report".into(), report);
                Err(e.with_details(serde_json::Value::Object(details)))
            }
        }
    }
}

/// Steps, in order (each idempotent):
/// 1. load projects, worktree environments, settings, installed packages;
/// 2. write php.ini for every site and the default CLI ini, then regenerate the shim index;
/// 3. build `WebServerConfig` (vhosts for active projects + all worktree environments) and
///    `apply` it (staged + `httpd -t`). If Apache rejects it, every vhost is validated on its
///    own, the broken ones are left out and the rest is applied, so one broken project never
///    takes the others down; the rejection is still returned at the end. An unchanged rejected
///    configuration is not validated again, an apply that failed for another reason (I/O, the
///    validator not starting) is retried on every sync;
/// 4. hosts block: compute `HostsBlock::for_projects`; if different from `read_block`, write it —
///    only when `ctx.elevation == Allow`, otherwise remember the failure and continue;
/// 5. `plan_services`: stop unneeded services, start missing ones (port check first), restart
///    the ones whose configuration changed (Apache: config or php.ini);
/// 6. return the report, or the first pending error (`ConfigRejected`, `ElevationRequired`,
///    `ElevationDenied`, `PortInUse`, …) with the report in `details.report`.
///
/// Missing packages are warnings, not errors. Serialises concurrent calls internally.
pub struct ApplyEnvironment {
    deps: ApplyEnvironmentDeps,
    lock: tokio::sync::Mutex<SyncState>,
}

impl ApplyEnvironment {
    pub fn new(deps: ApplyEnvironmentDeps) -> Self {
        Self { deps, lock: tokio::sync::Mutex::new(SyncState::default()) }
    }

    fn snapshot(&self) -> AppResult<ConfigSnapshot> {
        Ok(ConfigSnapshot { projects: self.deps.projects.list()?, worktrees: self.deps.worktrees.list_all()?, settings: self.deps.settings.load()?, installed: self.deps.installed.list()? })
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
        let snap = self.snapshot()?;
        let mut warnings = Vec::new();
        self.write_files(&snap, &mut warnings)?;
        for w in warnings {
            tracing::warn!("{w}");
        }
        Ok(())
    }

    /// Applies `desired`. Sites whose custom directives break the policy (stored before a rule
    /// existed) are left out up front; on an Apache rejection the broken vhosts are isolated.
    /// The rest is applied.
    async fn apply_web(&self, desired: &WebServerConfig, ini: &[PhpIniSpec]) -> WebState {
        WebState { desired: desired.clone(), ini: ini.to_vec(), outcome: self.web_outcome(desired).await }
    }

    async fn web_outcome(&self, desired: &WebServerConfig) -> WebOutcome {
        let (allowed, mut excluded) = leave_out_refused_directives(desired);
        let output = match self.deps.web_config.apply(&allowed).await {
            Ok(()) => return WebOutcome::Applied { exclusion: exclusion_error(&excluded) },
            Err(PortError::ConfigRejected { output }) => output,
            Err(e) => return WebOutcome::Failed(e.into()),
        };
        let refused = excluded.len();
        let mut good = allowed.clone();
        good.vhosts.clear();
        for vhost in &allowed.vhosts {
            let mut probe = good.clone();
            probe.vhosts = vec![vhost.clone()];
            match self.deps.web_config.apply(&probe).await {
                Ok(()) => good.vhosts.push(vhost.clone()),
                Err(PortError::ConfigRejected { output }) => excluded.push((vhost.site.to_string(), output)),
                Err(e) => return WebOutcome::Failed(e.into()),
            }
        }
        if excluded.len() == refused {
            // Every vhost is fine alone: the problem is global or a combination.
            return WebOutcome::Rejected(PortError::ConfigRejected { output }.into());
        }
        match self.deps.web_config.apply(&good).await {
            Ok(()) => WebOutcome::Applied { exclusion: exclusion_error(&excluded) },
            Err(e) => WebOutcome::from_port_error(e),
        }
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

    fn port_busy(&self, ports: &[u16]) -> Option<AppError> {
        ports.iter().find_map(|&port| match self.deps.port_probe.check(port) {
            Ok(PortAvailability::InUse { owner }) => Some(PortError::PortInUse { port, owner }.into()),
            Ok(PortAvailability::Free) => None,
            Err(e) => {
                tracing::warn!("cannot probe port {port}: {e}");
                None
            }
        })
    }

    async fn sync_services(&self, snap: &ConfigSnapshot, state: &mut SyncState, web_changed: bool, out: &mut Outcome) {
        let plan = plan_services(&snap.projects, &snap.worktrees, &snap.settings);
        let runners = &self.deps.runners;
        let catalog = &self.deps.services;

        // Stop what is no longer needed first (frees ports).
        let candidates: BTreeSet<ServiceId> = catalog.configured(&snap.settings).into_iter().chain(state.launches.keys().cloned()).collect();
        for id in candidates.iter().filter(|id| !plan.contains_key(id)) {
            let Some(runner) = runner_for(runners, id.kind()) else { continue };
            if matches!(runner.status(id).state, ServiceState::Running | ServiceState::Starting) {
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
            let running = matches!(runner.status(&id).state, ServiceState::Running | ServiceState::Starting);
            if running {
                let changed = match web {
                    Some(_) => web_changed,
                    None => state.launches.get(&id).is_some_and(|previous| previous != &launch),
                };
                if changed {
                    let result = match runner.prepare(&launch).await {
                        Ok(()) => runner.restart(&launch).await,
                        Err(e) => Err(e),
                    };
                    match result {
                        Ok(()) => out.report.restarted.push(id.to_string()),
                        Err(e) => out.fail(e.into()),
                    }
                }
            } else {
                let ports = match web {
                    Some(w) => w.desired.listen_ports(),
                    None => launch.ports(),
                };
                if let Some(busy) = self.port_busy(&ports) {
                    out.fail(busy);
                    continue;
                }
                let result = match runner.prepare(&launch).await {
                    Ok(()) => runner.start(&launch).await,
                    Err(e) => Err(e),
                };
                match result {
                    Ok(()) => out.report.started.push(id.to_string()),
                    Err(e) => {
                        out.fail(e.into());
                        continue;
                    }
                }
            }
            state.launches.insert(id, launch);
        }
    }
}

/// Splits `desired` into the config to apply and the `(site, reason)` of the vhosts whose custom
/// directives the policy refuses (`ApacheDirectives::validate`).
fn leave_out_refused_directives(desired: &WebServerConfig) -> (WebServerConfig, Vec<(String, String)>) {
    let mut allowed = desired.clone();
    let mut refused = Vec::new();
    allowed.vhosts.retain(|vhost| match vhost.custom_directives.validate() {
        Ok(()) => true,
        Err(e) => {
            refused.push((vhost.site.to_string(), e.to_string()));
            false
        }
    });
    (allowed, refused)
}

/// The `ConfigRejected` error naming the left-out sites, `None` when every site is served.
fn exclusion_error(excluded: &[(String, String)]) -> Option<AppError> {
    let (_, first) = excluded.first()?;
    let sites: Vec<&str> = excluded.iter().map(|(s, _)| s.as_str()).collect();
    let message = format!("the configuration of {} was rejected and is not served; the other sites are still served", sites.join(", "));
    Some(AppError::new(ErrorCode::ConfigRejected, message).with_details(serde_json::json!({
        "output": first,
        "excludedSites": sites,
        "rejections": excluded.iter().map(|(s, o)| serde_json::json!({ "site": s, "output": o })).collect::<Vec<_>>(),
    })))
}

impl GeneratedFilesWriter for ApplyEnvironment {
    fn write_generated_files(&self) -> AppResult<()> {
        ApplyEnvironment::write_generated_files(self)
    }
}

#[async_trait]
impl EnvironmentSync for ApplyEnvironment {
    async fn sync(&self, ctx: &CallContext) -> AppResult<SyncReportDto> {
        let mut state = self.lock.lock().await;
        let mut out = Outcome::default();
        let snap = self.snapshot()?;

        let ini = match self.write_files(&snap, &mut out.report.warnings) {
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
