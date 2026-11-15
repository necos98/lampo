//! Production implementations of the role traits in `ports.rs`: one-line delegations to the
//! application use cases and to the control client. No logic lives here.

use async_trait::async_trait;
use lampo_application::control::{APP_START_TIMEOUT, ControlClient, ControlRequest};
use lampo_application::dto::*;
use lampo_application::events::ProgressDto;
use lampo_application::use_cases::*;
use lampo_application::{AppResult, CallContext};

use crate::ports::*;

impl SiteLookup for SiteResolver {
    fn resolve(&self, request: &ResolveRequestDto) -> AppResult<ResolvedSiteDto> {
        self.resolve_dto(request)
    }
}

impl ProjectReader for ProjectQueries {
    fn summaries(&self) -> AppResult<Vec<ProjectSummaryDto>> {
        ProjectQueries::summaries(self)
    }
    fn get(&self, project_id: &str) -> AppResult<ProjectDto> {
        ProjectQueries::get(self, project_id)
    }
}

impl ProjectDetection for DetectProject {
    fn detect(&self, folder: &str) -> AppResult<DetectionDto> {
        self.execute(folder)
    }
}

impl WorktreeReader for WorktreeQueries {
    fn list(&self, project_id: Option<&str>) -> AppResult<Vec<WorktreeEnvironmentDto>> {
        WorktreeQueries::list(self, project_id)
    }
}

impl LogReader for LogQueries {
    fn sources(&self, site: Option<&str>) -> AppResult<Vec<LogSourceDto>> {
        LogQueries::sources(self, site)
    }
    fn tail(&self, request: &LogTailRequest) -> AppResult<LogLinesDto> {
        LogQueries::tail(self, request)
    }
    fn read_from(&self, source: &str, offset: u64) -> AppResult<LogLinesDto> {
        LogQueries::read_from(self, source, offset)
    }
}

#[async_trait]
impl PackageReader for PackageCatalog {
    async fn catalog(&self, kind: Option<&str>) -> AppResult<Vec<CatalogEntryDto>> {
        self.list(kind).await
    }
    fn installed(&self) -> AppResult<Vec<InstalledPackageDto>> {
        self.list_installed()
    }
}

impl PhpExtensionReader for PhpExtensionListing {
    fn list(&self, request: &ResolveRequestDto, php: Option<&str>) -> AppResult<PhpExtensionListDto> {
        self.execute(request, php)
    }
}

impl ToolchainReader for ToolchainQuery {
    fn env(&self, request: &ResolveRequestDto) -> AppResult<ToolchainEnvDto> {
        ToolchainQuery::env(self, request)
    }
    fn exec_plan(&self, request: &ResolveRequestDto, command: &[String]) -> AppResult<ExecPlanDto> {
        ToolchainQuery::exec_plan(self, request, command)
    }
}

#[async_trait]
impl DoctorRunner for Doctor {
    async fn run(&self) -> AppResult<DoctorReportDto> {
        Doctor::run(self).await
    }
}

impl AgentContextReader for AgentContextQuery {
    fn context(&self, request: &ResolveRequestDto) -> AppResult<AgentContextDto> {
        self.execute(request)
    }
}

impl ShimAdmin for ShimSetup {
    fn status(&self) -> AppResult<ShimStatusDto> {
        ShimSetup::status(self)
    }
    fn install(&self) -> AppResult<ShimStatusDto> {
        ShimSetup::install(self)
    }
}

#[async_trait]
impl OfflineStatus for StatusQuery {
    async fn status(&self) -> AppResult<StatusDto> {
        self.execute().await
    }
}

#[async_trait]
impl AppGateway for ControlClient {
    fn is_running(&self) -> bool {
        self.is_app_running()
    }

    async fn ensure_running(&self) -> AppResult<()> {
        self.ensure_app(APP_START_TIMEOUT).await
    }

    async fn send(&self, request: ControlRequest, context: CallContext, on_progress: &(dyn Fn(ProgressDto) + Send + Sync)) -> AppResult<serde_json::Value> {
        self.call(request, context, on_progress).await
    }
}

/// Hook JSON on stdin; nothing when stdin is a console (a person running the command).
pub struct StdinHookInput;

/// Hook payloads are small; never read more.
const HOOK_INPUT_LIMIT: u64 = 1024 * 1024;

impl HookInput for StdinHookInput {
    fn read(&self) -> Option<String> {
        use std::io::{IsTerminal, Read};
        let stdin = std::io::stdin();
        if stdin.is_terminal() {
            return None;
        }
        let mut text = String::new();
        stdin.lock().take(HOOK_INPUT_LIMIT).read_to_string(&mut text).ok()?;
        Some(text).filter(|t| !t.trim().is_empty())
    }
}

/// Ctrl+C as seen by the tokio runtime.
pub struct CtrlC;

#[async_trait]
impl Interrupt for CtrlC {
    async fn wait(&self) {
        // If the handler cannot be installed, never report an interrupt: the process still ends
        // on the default Ctrl+C behaviour.
        if tokio::signal::ctrl_c().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}
