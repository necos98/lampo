//! Control protocol between front-ends (the CLI) and the running app, over the domain's
//! control-channel ports (a named pipe in production).
//!
//! Wire format: newline-delimited JSON. The client sends one [`ControlEnvelope`]; the server
//! answers with zero or more [`ControlFrame::Progress`] frames and exactly one
//! [`ControlFrame::Response`]. Only operations that need the app (process ownership or
//! mutations) are requests here; read-only, file-based queries are executed by the CLI itself.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use lampo_domain::ports::{AppLauncher, Clock, ControlChannelClient, ControlFrameSink, ControlRequestHandler, ProgressSink, Timer};
use lampo_domain::progress::{CancelFlag, Progress};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::context::CallContext;
use crate::dto::*;
use crate::error::{AppError, AppResult, ErrorCode};
use crate::events::ProgressDto;
use crate::use_cases::*;

/// Bumped on any incompatible change to requests, frames or payload DTOs. Adding a request is
/// not one: older clients keep working, and an older app rejects the unknown request as a
/// malformed one (`ProtocolMismatch`, the same "update both" hint).
pub const PROTOCOL_VERSION: u32 = 1;

/// Per-user pipe name: `\\.\pipe\lampo-control-<user>`; `user` is the Windows user name,
/// sanitised to `[A-Za-z0-9_-]`.
pub fn pipe_name(user: &str) -> String {
    let safe: String = user.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    format!(r"\\.\pipe\lampo-control-{safe}")
}

/// The pipe of one Lampo instance: [`pipe_name`] for the normal data root; with an isolated
/// data root (`LAMPO_HOME`) a suffix derived from that folder, so a test instance and the real
/// app never talk to each other's CLI.
pub fn instance_pipe_name(user: &str, isolated_home: Option<&std::path::Path>) -> String {
    match isolated_home {
        None => pipe_name(user),
        Some(home) => {
            // FNV-1a over the case-folded path (Windows paths are case-insensitive).
            let key = home.display().to_string().replace('/', "\\").trim_end_matches('\\').to_lowercase();
            let hash = key.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3));
            format!("{}-{hash:016x}", pipe_name(user))
        }
    }
}

/// Every request the app serves. Each variant documents its success payload type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "params", rename_all = "kebab-case")]
pub enum ControlRequest {
    /// → [`AppInfoDto`]
    Ping,
    /// → [`StatusDto`]
    Status,
    /// → [`CreateProjectResultDto`] (a [`ProjectDto`] plus an optional `databaseSetup`)
    ProjectCreate(CreateProjectRequest),
    /// → [`ScaffoldProjectResultDto`] (progress frames carry the tools' output lines)
    ProjectScaffold(ScaffoldProjectRequest),
    /// → [`ProjectDto`]
    ProjectUpdate(UpdateProjectRequest),
    /// → [`ProjectDto`]
    ProjectSetActive(SetProjectActiveRequest),
    /// → [`ProjectDto`]
    ProjectSetPhp(SetProjectPhpRequest),
    /// Partial environment change (`lampo project set`). → [`ProjectDto`]
    ProjectPatch(ProjectPatchRequest),
    /// → `null`
    ProjectDelete { project_id: String },
    /// → [`SyncReportDto`]
    ServicesUp(ServicesRequest),
    /// → [`SyncReportDto`]
    ServicesDown(ServicesRequest),
    /// → [`SyncReportDto`]
    ServicesRestart(ServicesRequest),
    /// → [`SyncReportDto`]
    ApplyEnvironment,
    /// → [`InstalledPackageDto`] (progress frames while downloading)
    PackageInstall(InstallPackageRequest),
    /// → [`DatabaseListDto`]
    DbList(DatabaseListRequest),
    /// → `string` (database name)
    DbCreate(DatabaseRequest),
    /// → `string` (database name)
    DbDrop(DatabaseRequest),
    /// → `null` (progress frames)
    DbImport(DatabaseFileRequest),
    /// → `null` (progress frames)
    DbExport(DatabaseFileRequest),
    /// → [`WorktreeUpResultDto`] (progress frames when cloning a database)
    WorktreeUp(WorktreeUpRequest),
    /// → [`WorktreeDownResultDto`]
    WorktreeDown(WorktreeDownRequest),
    /// → [`PruneReportDto`]
    WorktreePrune(PruneWorktreesRequest),
    /// Stops services and exits the app. → `null`
    Quit,
}

impl ControlRequest {
    /// Whether the request changes state (used by the CLI for help/agent catalog).
    pub fn is_mutating(&self) -> bool {
        !matches!(self, Self::Ping | Self::Status | Self::DbList(_) | Self::DbExport(_))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlEnvelope {
    pub protocol: u32,
    pub context: CallContext,
    pub request: ControlRequest,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum ControlReply {
    Ok { payload: serde_json::Value },
    Error { error: AppError },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "frame", rename_all = "kebab-case")]
pub enum ControlFrame {
    Progress { progress: ProgressDto },
    Response { reply: ControlReply },
}

/// Use cases the dispatcher routes to (all owned by the app's composition root).
pub struct ControlServices {
    pub app_info: Arc<AppInfoQuery>,
    pub status: Arc<StatusQuery>,
    pub create_project: Arc<CreateProject>,
    pub scaffold_project: Arc<ScaffoldProject>,
    pub update_project: Arc<UpdateProject>,
    pub set_project_active: Arc<SetProjectActive>,
    pub set_project_php: Arc<SetProjectPhp>,
    pub patch_project: Arc<PatchProjectEnvironment>,
    pub delete_project: Arc<DeleteProject>,
    pub service_control: Arc<ServiceControl>,
    pub environment: Arc<dyn EnvironmentSync>,
    pub install_package: Arc<InstallPackage>,
    pub database: Arc<DatabaseService>,
    pub worktree_up: Arc<WorktreeUp>,
    pub worktree_down: Arc<WorktreeDown>,
    pub prune_worktrees: Arc<PruneWorktrees>,
    /// Called for [`ControlRequest::Quit`] after the reply is produced.
    pub quit: Arc<dyn Fn() + Send + Sync>,
    /// Closed while the app shuts down or prepares an update: mutating requests are refused.
    pub gate: Arc<ShutdownGate>,
}

/// Server-side handler: decodes the envelope, checks the protocol, calls the use case,
/// encodes the reply. No business logic.
pub struct ControlDispatcher {
    services: ControlServices,
}

impl ControlDispatcher {
    pub fn new(services: ControlServices) -> Self {
        Self { services }
    }

    async fn dispatch(&self, envelope: ControlEnvelope, progress: &dyn ProgressSink, cancel: &CancelFlag) -> AppResult<serde_json::Value> {
        if envelope.protocol != PROTOCOL_VERSION {
            return Err(AppError::new(ErrorCode::ProtocolMismatch, format!("client speaks protocol {}, app speaks {PROTOCOL_VERSION}", envelope.protocol)));
        }
        let ctx = &envelope.context;
        let s = &self.services;
        // Quit stays allowed (it is what is happening anyway); anything else could restart a
        // service that was just stopped.
        if s.gate.is_closed() && envelope.request.is_mutating() && envelope.request != ControlRequest::Quit {
            return Err(AppError::new(ErrorCode::AppUnavailable, "the Lampo app is shutting down (quit or update in progress)"));
        }
        match envelope.request {
            ControlRequest::Ping => to_json(s.app_info.execute()),
            ControlRequest::Status => to_json(s.status.execute().await?),
            ControlRequest::ProjectCreate(r) => to_json(s.create_project.execute(r, ctx).await?),
            ControlRequest::ProjectScaffold(r) => to_json(s.scaffold_project.execute(r, progress, cancel).await?),
            ControlRequest::ProjectUpdate(r) => to_json(s.update_project.execute(r, ctx).await?),
            ControlRequest::ProjectSetActive(r) => to_json(s.set_project_active.execute(r, ctx).await?),
            ControlRequest::ProjectSetPhp(r) => to_json(s.set_project_php.execute(r, ctx).await?),
            ControlRequest::ProjectPatch(r) => to_json(s.patch_project.execute(r, ctx).await?),
            ControlRequest::ProjectDelete { project_id } => to_json(s.delete_project.execute(&project_id, ctx).await?),
            ControlRequest::ServicesUp(r) => to_json(s.service_control.up(r, ctx).await?),
            ControlRequest::ServicesDown(r) => to_json(s.service_control.down(r).await?),
            ControlRequest::ServicesRestart(r) => to_json(s.service_control.restart(r).await?),
            ControlRequest::ApplyEnvironment => to_json(s.environment.sync(ctx).await?),
            ControlRequest::PackageInstall(r) => to_json(s.install_package.execute(r, ctx, progress, cancel).await?),
            ControlRequest::DbList(r) => to_json(s.database.list(&r).await?),
            ControlRequest::DbCreate(r) => to_json(s.database.create(&r).await?),
            ControlRequest::DbDrop(r) => to_json(s.database.drop_database(&r).await?),
            ControlRequest::DbImport(r) => to_json(s.database.import(&r, progress, cancel).await?),
            ControlRequest::DbExport(r) => to_json(s.database.export(&r, progress, cancel).await?),
            ControlRequest::WorktreeUp(r) => to_json(s.worktree_up.execute(r, ctx, progress, cancel).await?),
            ControlRequest::WorktreeDown(r) => to_json(s.worktree_down.execute(r, ctx).await?),
            ControlRequest::WorktreePrune(r) => to_json(s.prune_worktrees.execute(r, ctx).await?),
            ControlRequest::Quit => {
                (s.quit)();
                Ok(serde_json::Value::Null)
            }
        }
    }
}

fn to_json<T: Serialize>(value: T) -> AppResult<serde_json::Value> {
    serde_json::to_value(value).map_err(|e| AppError::internal(format!("serialising reply: {e}")))
}

fn encode(frame: &ControlFrame) -> String {
    serde_json::to_string(frame).unwrap_or_else(|e| format!(r#"{{"frame":"response","reply":{{"status":"error","error":{{"code":"internal","message":"{e}"}}}}}}"#))
}

/// Turns progress reports into `Progress` frames.
struct FrameProgress<'a>(&'a dyn ControlFrameSink);

impl ProgressSink for FrameProgress<'_> {
    fn report(&self, progress: Progress) {
        self.0.frame(encode(&ControlFrame::Progress { progress: progress.into() }));
    }
}

#[async_trait]
impl ControlRequestHandler for ControlDispatcher {
    async fn handle(&self, request_json: String, events: &dyn ControlFrameSink, cancel: &CancelFlag) -> String {
        let result = match serde_json::from_str::<ControlEnvelope>(&request_json) {
            Ok(envelope) => self.dispatch(envelope, &FrameProgress(events), cancel).await,
            Err(e) => Err(AppError::new(ErrorCode::ProtocolMismatch, format!("malformed request: {e}"))),
        };
        let reply = match result {
            Ok(payload) => ControlReply::Ok { payload },
            Err(error) => ControlReply::Error { error },
        };
        encode(&ControlFrame::Response { reply })
    }
}

/// Typed client used by the CLI. Starts the app headless when needed.
pub struct ControlClient {
    channel: Arc<dyn ControlChannelClient>,
    launcher: Arc<dyn AppLauncher>,
    clock: Arc<dyn Clock>,
    timer: Arc<dyn Timer>,
}

/// How often [`ControlClient::ensure_app`] looks for the pipe while the app starts.
pub const APP_START_POLL: Duration = Duration::from_millis(200);

/// How long a single exchange may take (package downloads, DB imports are long).
pub const DEFAULT_EXCHANGE_TIMEOUT: Duration = Duration::from_secs(60 * 60);
/// How long to wait for a headless app to come up.
pub const APP_START_TIMEOUT: Duration = Duration::from_secs(20);

impl ControlClient {
    pub fn new(channel: Arc<dyn ControlChannelClient>, launcher: Arc<dyn AppLauncher>, clock: Arc<dyn Clock>, timer: Arc<dyn Timer>) -> Self {
        Self { channel, launcher, clock, timer }
    }

    pub fn is_app_running(&self) -> bool {
        self.channel.is_server_running()
    }

    /// Starts the app headless if it is not running and waits for the pipe (`AppUnavailable`
    /// on timeout).
    pub async fn ensure_app(&self, timeout: Duration) -> AppResult<()> {
        if self.channel.is_server_running() {
            return Ok(());
        }
        self.launcher.launch_headless().map_err(|e| AppError::new(ErrorCode::AppUnavailable, e.to_string()))?;
        let deadline = self.clock.now_unix_ms().saturating_add(timeout.as_millis() as u64);
        while self.clock.now_unix_ms() < deadline {
            if self.channel.is_server_running() {
                return Ok(());
            }
            self.timer.sleep(APP_START_POLL).await;
        }
        Err(AppError::new(ErrorCode::AppUnavailable, "the Lampo app did not start in time"))
    }

    /// Sends `request` and decodes the success payload as `T`. `on_progress` receives progress
    /// frames. Does not start the app (call [`ControlClient::ensure_app`] first).
    pub async fn call<T: DeserializeOwned>(&self, request: ControlRequest, ctx: CallContext, on_progress: &(dyn Fn(ProgressDto) + Send + Sync)) -> AppResult<T> {
        let envelope = ControlEnvelope { protocol: PROTOCOL_VERSION, context: ctx, request };
        let json = serde_json::to_string(&envelope).map_err(|e| AppError::internal(e.to_string()))?;
        struct Forward<'a>(&'a (dyn Fn(ProgressDto) + Send + Sync));
        impl ControlFrameSink for Forward<'_> {
            fn frame(&self, json: String) {
                if let Ok(ControlFrame::Progress { progress }) = serde_json::from_str(&json) {
                    (self.0)(progress);
                }
            }
        }
        let final_frame = self.channel.exchange(json, &Forward(on_progress), DEFAULT_EXCHANGE_TIMEOUT).await?;
        match serde_json::from_str::<ControlFrame>(&final_frame) {
            Ok(ControlFrame::Response { reply: ControlReply::Ok { payload } }) => {
                serde_json::from_value(payload).map_err(|e| AppError::new(ErrorCode::ProtocolMismatch, format!("unexpected reply payload: {e}")))
            }
            Ok(ControlFrame::Response { reply: ControlReply::Error { error } }) => Err(error),
            Ok(ControlFrame::Progress { .. }) | Err(_) => Err(AppError::new(ErrorCode::ProtocolMismatch, "malformed reply from the app")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::CallContext;

    #[test]
    fn envelope_wire_format() {
        let env = ControlEnvelope {
            protocol: PROTOCOL_VERSION,
            context: CallContext::cli(false),
            request: ControlRequest::WorktreeUp(WorktreeUpRequest { path: "D:\\wt\\x".into(), ..Default::default() }),
        };
        let json = serde_json::to_value(&env).unwrap();
        assert_eq!(json["request"]["type"], "worktree-up");
        assert_eq!(json["request"]["params"]["database"]["mode"], "shared");
        assert_eq!(json["context"]["elevation"], "deny");
        let back: ControlEnvelope = serde_json::from_value(json).unwrap();
        assert_eq!(back, env);
        let ping: ControlRequest = serde_json::from_str(r#"{"type":"ping"}"#).unwrap();
        assert_eq!(ping, ControlRequest::Ping);
    }

    #[test]
    fn project_patch_wire_format() {
        let request = ControlRequest::ProjectPatch(ProjectPatchRequest {
            project_id: "shop".into(),
            extensions_add: vec!["intl".into()],
            ini_set: [("memory_limit".to_string(), "512M".to_string())].into(),
            services_add: vec![ServiceUseDto { kind: "mysql".into(), version: Some("5.7".into()) }],
            name: Some("Shop Legacy".into()),
            addresses_add: vec!["loopback".into()],
            addresses_remove: vec!["shop.test".into()],
            ..Default::default()
        });
        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["type"], "project-patch");
        assert_eq!((&json["params"]["name"], &json["params"]["addressesAdd"][0], &json["params"]["addressesRemove"][0]), (&"Shop Legacy".into(), &"loopback".into(), &"shop.test".into()));
        assert_eq!(json["params"]["extensionsAdd"][0], "intl");
        assert_eq!(json["params"]["iniSet"]["memory_limit"], "512M");
        assert_eq!(json["params"]["servicesAdd"][0]["version"], "5.7");
        assert_eq!(serde_json::from_value::<ControlRequest>(json).unwrap(), request);
        // Omitted fields default: a client sends only what it changes.
        let minimal: ControlRequest = serde_json::from_str(r#"{"type":"project-patch","params":{"projectId":"shop","addonsRemove":["xdebug"]}}"#).unwrap();
        let ControlRequest::ProjectPatch(p) = &minimal else { panic!("wrong variant") };
        assert_eq!(p.addons_remove, ["xdebug"]);
        assert!(p.extensions_add.is_empty() && p.database_name.is_none() && !p.is_empty());
        assert!(minimal.is_mutating());
    }

    #[test]
    fn pipe_name_is_sanitised() {
        assert_eq!(pipe_name("Mario Rossi"), r"\\.\pipe\lampo-control-Mario_Rossi");
    }

    #[test]
    fn isolated_instances_get_their_own_pipe() {
        assert_eq!(instance_pipe_name("me", None), pipe_name("me"));
        let a = instance_pipe_name("me", Some(std::path::Path::new(r"C:\Temp\Lampo-A")));
        assert_ne!(a, pipe_name("me"));
        assert!(a.starts_with(&pipe_name("me")));
        assert_eq!(a, instance_pipe_name("me", Some(std::path::Path::new(r"c:\temp\lampo-a\"))));
        assert_ne!(a, instance_pipe_name("me", Some(std::path::Path::new(r"C:\Temp\Lampo-B"))));
    }
}
