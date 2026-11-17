//! What the command handlers need from the outside world, as small role traits (interface
//! segregation). Production implementations are thin delegations to the application use cases
//! (`local.rs`) and to the control client; tests inject fakes.

use std::collections::BTreeMap;

use async_trait::async_trait;
use lampo_application::control::ControlRequest;
use lampo_application::dto::*;
use lampo_application::events::ProgressDto;
use lampo_application::use_cases::LogTarget;
use lampo_application::{AppResult, CallContext};

/// Site resolution (`--project` / `LAMPO_PROJECT` / cwd / git worktree / default).
pub trait SiteLookup: Send + Sync {
    fn resolve(&self, request: &ResolveRequestDto) -> AppResult<ResolvedSiteDto>;
}

pub trait ProjectReader: Send + Sync {
    /// The compact rows, plus the project files that do not load.
    fn summaries(&self) -> AppResult<ProjectSummaryListDto>;
    /// Every project in full (folder state included), plus the files that do not load.
    fn list(&self) -> AppResult<ProjectListDto>;
    fn get(&self, project_id: &str) -> AppResult<ProjectDto>;
}

/// Folder analysis that registers nothing (`project detect`).
pub trait ProjectDetection: Send + Sync {
    /// `folder` is absolute; `Validation` if it is not an existing directory.
    fn detect(&self, folder: &str) -> AppResult<DetectionDto>;
}

pub trait WorktreeReader: Send + Sync {
    fn list(&self, project_id: Option<&str>) -> AppResult<Vec<WorktreeEnvironmentDto>>;
}

pub trait LogReader: Send + Sync {
    fn sources(&self, site: Option<&str>) -> AppResult<Vec<LogSourceDto>>;
    fn tail(&self, request: &LogTailRequest) -> AppResult<LogLinesDto>;
    fn read_from(&self, source: &str, offset: u64) -> AppResult<LogLinesDto>;
}

/// Starts a desktop app of the user on a log's file or folder (`logs --open`); runs on this
/// machine, never in the app.
pub trait LogOpener: Send + Sync {
    /// `source`: `apache`, `php:shop`, ... (site shorthands already expanded).
    fn open(&self, source: &str, target: LogTarget) -> AppResult<()>;
}

#[async_trait]
pub trait PackageReader: Send + Sync {
    async fn catalog(&self, kind: Option<&str>) -> AppResult<Vec<CatalogEntryDto>>;
    fn installed(&self) -> AppResult<Vec<InstalledPackageDto>>;
}

/// Bundled extensions of an installed PHP build (`package extensions`), file-based.
pub trait PhpExtensionReader: Send + Sync {
    /// `php`: explicit minor, else the resolved site's PHP.
    fn list(&self, request: &ResolveRequestDto, php: Option<&str>) -> AppResult<PhpExtensionListDto>;
}

pub trait ToolchainReader: Send + Sync {
    fn env(&self, request: &ResolveRequestDto) -> AppResult<ToolchainEnvDto>;
    fn exec_plan(&self, request: &ResolveRequestDto, command: &[String]) -> AppResult<ExecPlanDto>;
}

/// What a shim would run from a folder, and why (`which`), file-based.
pub trait ShimToolReader: Send + Sync {
    /// `tool`: a shim name (`php`, `composer`, `mysql`, `mysqldump`, `redis-cli`).
    fn which(&self, request: &ResolveRequestDto, tool: &str) -> AppResult<WhichDto>;
}

/// Starts a desktop app of the user on a site (`open`); runs on this machine, never in the app.
pub trait SiteOpener: Send + Sync {
    /// `site`: `shop` or `shop@feature-x`.
    fn open(&self, site: &str, app: ExternalApp) -> AppResult<OpenedInDto>;
}

#[async_trait]
pub trait DoctorRunner: Send + Sync {
    async fn run(&self) -> AppResult<DoctorReportDto>;
}

pub trait AgentContextReader: Send + Sync {
    fn context(&self, request: &ResolveRequestDto) -> AppResult<AgentContextDto>;
}

/// What a Claude Code hook passes on stdin (a JSON object), read only by the hook mode.
pub trait HookInput: Send + Sync {
    /// `None` when stdin is a console or empty.
    fn read(&self) -> Option<String>;
}

pub trait ShimAdmin: Send + Sync {
    fn status(&self) -> AppResult<ShimStatusDto>;
    fn install(&self) -> AppResult<ShimStatusDto>;
    fn uninstall(&self) -> AppResult<ShimStatusDto>;
}

/// The stored global settings (`settings get`, file-based).
pub trait SettingsReader: Send + Sync {
    fn get(&self) -> AppResult<SettingsDto>;
}

/// `lampo status` when the app is not running.
#[async_trait]
pub trait OfflineStatus: Send + Sync {
    async fn status(&self) -> AppResult<StatusDto>;
}

/// The running app, reached through the control channel.
#[async_trait]
pub trait AppGateway: Send + Sync {
    fn is_running(&self) -> bool;
    /// Starts the app headless if needed and waits until it listens.
    async fn ensure_running(&self) -> AppResult<()>;
    /// Sends one request; `on_progress` receives progress frames. Returns the raw payload.
    async fn send(&self, request: ControlRequest, context: CallContext, on_progress: &(dyn Fn(ProgressDto) + Send + Sync)) -> AppResult<serde_json::Value>;
}

/// Completes when the user interrupts the process (Ctrl+C).
#[async_trait]
pub trait Interrupt: Send + Sync {
    async fn wait(&self);
}

/// What `lampo exec` hands to the process runner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnSpec {
    pub program: String,
    pub args: Vec<String>,
    /// Directories put in front of `PATH` (also used to find `program`).
    pub path_prepend: Vec<String>,
    pub vars: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpawnError {
    /// `program` was not found on the toolchain PATH.
    NotFound(String),
    Failed(String),
}

/// Runs a child process with inherited stdio and returns its exit code.
pub trait ProgramRunner: Send + Sync {
    fn run(&self, spec: &SpawnSpec) -> Result<i32, SpawnError>;
}
