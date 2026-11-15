use std::sync::Arc;

use async_trait::async_trait;
use lampo_domain::paths::LampoPaths;
use lampo_domain::ports::{InstalledPackageRegistry, PortAvailability, PortProbe, ProjectRepository, ServiceRunner, SettingsRepository, WorktreeRepository};
use lampo_domain::service::{ServiceId, ServiceState, plan_services};
use lampo_domain::PortError;

use super::environment::EnvironmentSync;
use super::generation::runner_for;
use super::service_catalog::ServiceCatalog;
use crate::context::CallContext;
use crate::dto::*;
use crate::error::{AppError, AppResult, ErrorCode};

/// Live service states. In the app: [`ServiceQueries`]. In the CLI: [`OfflineServiceStatus`]
/// (app not running) or the app over the control pipe.
#[async_trait]
pub trait ServiceStatusSource: Send + Sync {
    async fn statuses(&self) -> AppResult<Vec<ServiceStatusDto>>;
}

/// Makes sure one service is running (used by database operations, worktree DB cloning).
#[async_trait]
pub trait ServiceEnsurer: Send + Sync {
    async fn ensure_running(&self, service: ServiceId) -> AppResult<()>;
}

pub struct ServiceQueries {
    runners: Vec<Arc<dyn ServiceRunner>>,
    catalog: Arc<ServiceCatalog>,
    projects: Arc<dyn ProjectRepository>,
    worktrees: Arc<dyn WorktreeRepository>,
    settings: Arc<dyn SettingsRepository>,
}

impl ServiceQueries {
    pub fn new(
        runners: Vec<Arc<dyn ServiceRunner>>,
        catalog: Arc<ServiceCatalog>,
        projects: Arc<dyn ProjectRepository>,
        worktrees: Arc<dyn WorktreeRepository>,
        settings: Arc<dyn SettingsRepository>,
    ) -> Self {
        Self { runners, catalog, projects, worktrees, settings }
    }

    /// Every configured service plus any planned one, with its live state and requirers.
    pub fn list(&self) -> AppResult<Vec<ServiceStatusDto>> {
        let settings = self.settings.load()?;
        let plan = plan_services(&self.projects.list()?, &self.worktrees.list_all()?, &settings);
        let mut ids = self.catalog.configured(&settings);
        ids.extend(plan.keys().filter(|id| !ids.contains(id)).cloned().collect::<Vec<_>>());
        Ok(ids
            .into_iter()
            .map(|id| {
                let status = runner_for(&self.runners, id.kind()).map(|r| r.status(&id));
                let payload = status.as_ref().map(crate::events::service_status_payload);
                ServiceStatusDto {
                    service: id.to_string(),
                    kind: id.kind().to_string(),
                    label: self.catalog.label(&id),
                    state: payload.as_ref().map(|p| p.state).unwrap_or(ServiceStateDto::Unknown),
                    pid: status.as_ref().and_then(|s| s.pid),
                    port: status.as_ref().and_then(|s| s.port).or_else(|| self.catalog.port(&id, &settings)),
                    ui_port: self.catalog.ui_port(&id, &settings),
                    ui_url: self.catalog.ui_url(&id, &settings),
                    message: status.and_then(|s| s.message),
                    required: plan.contains_key(&id),
                    required_by: plan.get(&id).map(|sites| sites.iter().map(|s| s.to_string()).collect()).unwrap_or_default(),
                }
            })
            .collect())
    }
}

#[async_trait]
impl ServiceStatusSource for ServiceQueries {
    async fn statuses(&self) -> AppResult<Vec<ServiceStatusDto>> {
        self.list()
    }
}

/// Service states when the app is not running: every configured service is `unknown`.
pub struct OfflineServiceStatus {
    catalog: Arc<ServiceCatalog>,
    settings: Arc<dyn SettingsRepository>,
}

impl OfflineServiceStatus {
    pub fn new(catalog: Arc<ServiceCatalog>, settings: Arc<dyn SettingsRepository>) -> Self {
        Self { catalog, settings }
    }
}

#[async_trait]
impl ServiceStatusSource for OfflineServiceStatus {
    async fn statuses(&self) -> AppResult<Vec<ServiceStatusDto>> {
        let settings = self.settings.load()?;
        Ok(self
            .catalog
            .configured(&settings)
            .into_iter()
            .map(|id| ServiceStatusDto {
                service: id.to_string(),
                kind: id.kind().to_string(),
                label: self.catalog.label(&id),
                state: ServiceStateDto::Unknown,
                pid: None,
                port: self.catalog.port(&id, &settings),
                ui_port: self.catalog.ui_port(&id, &settings),
                ui_url: self.catalog.ui_url(&id, &settings),
                message: Some("the Lampo app is not running".into()),
                required: false,
                required_by: Vec::new(),
            })
            .collect())
    }
}

/// Manual service control (GUI buttons, `lampo up/down/restart`).
pub struct ServiceControl {
    runners: Vec<Arc<dyn ServiceRunner>>,
    catalog: Arc<ServiceCatalog>,
    installed: Arc<dyn InstalledPackageRegistry>,
    settings: Arc<dyn SettingsRepository>,
    port_probe: Arc<dyn PortProbe>,
    paths: LampoPaths,
    env: Arc<dyn EnvironmentSync>,
}

pub struct ServiceControlDeps {
    pub runners: Vec<Arc<dyn ServiceRunner>>,
    pub catalog: Arc<ServiceCatalog>,
    pub installed: Arc<dyn InstalledPackageRegistry>,
    pub settings: Arc<dyn SettingsRepository>,
    pub port_probe: Arc<dyn PortProbe>,
    pub paths: LampoPaths,
    pub env: Arc<dyn EnvironmentSync>,
}

fn is_up(state: ServiceState) -> bool {
    matches!(state, ServiceState::Running | ServiceState::Starting)
}

impl ServiceControl {
    pub fn new(deps: ServiceControlDeps) -> Self {
        let ServiceControlDeps { runners, catalog, installed, settings, port_probe, paths, env } = deps;
        Self { runners, catalog, installed, settings, port_probe, paths, env }
    }

    fn runner(&self, id: &ServiceId) -> AppResult<&Arc<dyn ServiceRunner>> {
        runner_for(&self.runners, id.kind()).ok_or_else(|| AppError::new(ErrorCode::Unsupported, format!("no runner is registered for {id}")))
    }

    /// Service ids of registered kinds (`Validation` otherwise).
    fn parse(&self, request: &ServicesRequest) -> AppResult<Vec<ServiceId>> {
        request
            .services
            .iter()
            .map(|s| {
                let id = s.trim().parse::<ServiceId>()?;
                match self.catalog.descriptor(id.kind()) {
                    Some(_) => Ok(id),
                    None => Err(AppError::new(ErrorCode::Validation, format!("unknown service `{id}`"))),
                }
            })
            .collect()
    }

    /// Starts `id` if it is not running. `Ok(true)` when it was started.
    async fn start_one(&self, id: &ServiceId) -> AppResult<bool> {
        let runner = self.runner(id)?;
        if is_up(runner.status(id).state) {
            return Ok(false);
        }
        let launch = self.catalog.launch(id, &self.settings.load()?, &self.installed.list()?, &self.paths)?;
        for port in launch.ports() {
            if let PortAvailability::InUse { owner } = self.port_probe.check(port)? {
                return Err(PortError::PortInUse { port, owner }.into());
            }
        }
        runner.prepare(&launch).await?;
        runner.start(&launch).await?;
        Ok(true)
    }

    /// `lampo up`: with no services, a full environment sync; otherwise starts the given ones
    /// (`prepare` + `start`, `PortInUse`/`PackageMissing` on failure).
    pub async fn up(&self, request: ServicesRequest, ctx: &CallContext) -> AppResult<SyncReportDto> {
        if request.services.is_empty() {
            return self.env.sync(ctx).await;
        }
        let mut report = SyncReportDto::default();
        for id in self.parse(&request)? {
            if self.start_one(&id).await? {
                report.started.push(id.to_string());
            }
        }
        Ok(report)
    }

    /// `lampo down`: stops the given services (all configured ones when empty). The always-on
    /// MySQL stays down until the next `up`/sync.
    pub async fn down(&self, request: ServicesRequest) -> AppResult<SyncReportDto> {
        let ids = if request.services.is_empty() { self.catalog.configured(&self.settings.load()?) } else { self.parse(&request)? };
        let mut report = SyncReportDto::default();
        for id in ids {
            let runner = self.runner(&id)?;
            if runner.status(&id).state != ServiceState::Stopped {
                runner.stop(&id).await?;
                report.stopped.push(id.to_string());
            }
        }
        Ok(report)
    }

    /// Restarts the given services (every running one when empty), re-rendering their config.
    pub async fn restart(&self, request: ServicesRequest) -> AppResult<SyncReportDto> {
        let settings = self.settings.load()?;
        let ids = if request.services.is_empty() {
            self.catalog.configured(&settings).into_iter().filter(|id| self.runner(id).is_ok_and(|r| is_up(r.status(id).state))).collect()
        } else {
            self.parse(&request)?
        };
        let installed = self.installed.list()?;
        let mut report = SyncReportDto::default();
        for id in ids {
            let runner = self.runner(&id)?;
            let launch = self.catalog.launch(&id, &settings, &installed, &self.paths)?;
            runner.prepare(&launch).await?;
            runner.restart(&launch).await?;
            report.restarted.push(id.to_string());
        }
        Ok(report)
    }
}

#[async_trait]
impl ServiceEnsurer for ServiceControl {
    async fn ensure_running(&self, service: ServiceId) -> AppResult<()> {
        self.start_one(&service).await.map(|_| ())
    }
}
