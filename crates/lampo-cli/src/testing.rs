//! Test doubles: one `Fake` implementing every port with canned data and recorded calls.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use lampo_application::control::ControlRequest;
use lampo_application::dto::*;
use lampo_application::events::ProgressDto;
use lampo_application::{AppError, AppResult, CallContext, ErrorCode};
use serde_json::{Value, json};

use crate::context::{CliContext, CliEnv};
use crate::inherited::InheritedEnv;
use crate::io::{Capture, Io, captured};
use crate::ports::*;

pub fn dto<T: serde::de::DeserializeOwned>(value: Value) -> T {
    serde_json::from_value(value).expect("test DTO matches the wire format")
}

pub fn project_json() -> Value {
    json!({
        "id": "shop", "name": "Shop", "root": "C:\\work\\shop", "folderExists": true,
        "addresses": [{"kind": "loopback-port", "port": 8101}],
        "url": "http://127.0.0.1:8101", "urls": ["http://127.0.0.1:8101", "http://shop.localhost:8101"],
        "documentRoot": "C:\\work\\shop\\public",
        "environment": {"phpVersion": "8.3", "extensions": ["intl"], "addons": [], "phpIni": {}, "publicDir": "public",
                        "services": [{"kind": "mysql", "version": "8.4"}], "apacheDirectives": "", "envVars": {}},
        "databaseName": "shop",
        "database": {"mysqlVersion": "8.4", "host": "127.0.0.1", "port": 3306, "user": "root", "password": "", "database": "shop"}, "mysqlCredentials": "root",
        "tls": "disabled", "active": true, "originPreset": "laravel", "worktrees": [], "notices": [], "warnings": []
    })
}

pub fn summary_json() -> Value {
    json!({"id": "shop", "name": "Shop", "root": "C:\\work\\shop", "active": true, "url": "http://127.0.0.1:8101", "phpVersion": "8.3", "mysqlVersion": "8.4", "worktreeCount": 0})
}

pub fn site_json(site: Option<&str>) -> Value {
    json!({"kind": if site.is_some() {"project"} else {"global-default"}, "via": "directory", "site": site,
           "projectId": site.map(|s| s.split('@').next().unwrap().to_string()), "root": null, "worktreeRoot": null, "mainRoot": null})
}

pub fn toolchain_json() -> Value {
    json!({"site": "shop", "pathPrepend": ["C:\\bin", "C:\\php\\8.3"], "vars": {"LAMPO_PROJECT": "shop", "PHPRC": "C:\\ini"}})
}

pub fn agent_context_json() -> Value {
    json!({
        "schemaVersion": 2, "appRunning": false, "resolution": site_json(Some("shop")),
        "projectId": "shop", "projectName": "Shop", "root": "C:\\work\\shop", "worktree": null,
        "url": "http://127.0.0.1:8101", "urls": ["http://127.0.0.1:8101"], "documentRoot": "C:\\work\\shop\\public",
        "php": {"version": "8.3", "binary": "C:\\php\\8.3\\php.exe", "iniDir": "C:\\ini", "composerPhar": "C:\\c\\composer.phar", "composerChannel": "stable"},
        "database": {"mysqlVersion": "8.4", "host": "127.0.0.1", "port": 3306, "user": "root", "password": "", "database": "shop"},
        "services": [{"service": "mysql-8.4", "kind": "mysql", "label": "MySQL 8.4", "host": "127.0.0.1", "port": 3306}],
        "logs": [{"id": "access:shop", "file": "C:\\logs\\access.log", "exists": false}],
        "commands": [{"name": "lampo status", "usage": "lampo status [--json]", "description": "Overview.", "mutating": false, "requiresApp": false}],
        "hints": ["no worktree environment is up: run lampo worktree up"]
    })
}

/// A Laravel folder with a Docker Compose file (MySQL, Redis, PostgreSQL), not registered.
pub fn detection_json(folder: &str) -> Value {
    json!({
        "folder": folder, "projectType": "laravel", "presetId": "laravel", "phpVersion": "8.2",
        "suggestedName": "shop", "suggestedId": "shop", "evidence": ["found artisan", "compose.yaml: php 8.2"],
        "services": ["mysql", "redis"], "warnings": ["PostgreSQL is not provided by Lampo"], "alreadyRegistered": null
    })
}

/// Bundled extensions of `php`, flagged for `shop` (which enables `intl`).
pub fn extensions_json(php: &str) -> Value {
    json!({
        "phpVersion": php, "projectId": "shop",
        "extensions": [{"name": "intl", "zend": false, "enabled": true}, {"name": "opcache", "zend": true, "enabled": false}, {"name": "pdo_mysql", "zend": false, "enabled": false}]
    })
}

/// Every port in one struct. Public fields are the knobs; `Mutex`es hold recorded calls.
pub struct Fake {
    pub site: Mutex<AppResult<ResolvedSiteDto>>,
    pub app_running: AtomicBool,
    pub ensure_calls: AtomicUsize,
    pub ensure_result: Mutex<AppResult<()>>,
    pub sent: Mutex<Vec<(ControlRequest, CallContext)>>,
    pub app_replies: Mutex<VecDeque<AppResult<Value>>>,
    pub progress_to_send: Mutex<Vec<ProgressDto>>,
    pub tail: Mutex<AppResult<LogLinesDto>>,
    pub tail_requests: Mutex<Vec<LogTailRequest>>,
    pub follow_reads: Mutex<VecDeque<LogLinesDto>>,
    pub follow_offsets: Mutex<Vec<u64>>,
    pub doctor_ok: AtomicBool,
    pub spawned: Mutex<Vec<SpawnSpec>>,
    pub spawn_result: Mutex<Result<i32, SpawnError>>,
    pub shim_installed: AtomicBool,
    /// What the hook mode reads from stdin.
    pub hook_payload: Mutex<Option<String>>,
    /// Replaces the canned agent context (`None`: [`agent_context_json`]).
    pub agent_result: Mutex<Option<AppResult<Value>>>,
    pub agent_requests: Mutex<Vec<ResolveRequestDto>>,
    /// Replaces the canned detection (`None`: [`detection_json`] for the requested folder).
    pub detect_result: Mutex<Option<AppResult<Value>>>,
    pub detect_folders: Mutex<Vec<String>>,
    /// Replaces the canned extension list (`None`: [`extensions_json`] for the requested PHP).
    pub extensions_result: Mutex<Option<AppResult<Value>>>,
    /// The `php` argument of every extension listing.
    pub extensions_php: Mutex<Vec<Option<String>>>,
    /// What `env`/`exec` resolve to (default: [`toolchain_json`]).
    pub toolchain: Mutex<Value>,
    /// What `settings get` (and the default addressing) read locally (default: [`settings_json`]).
    pub settings: Mutex<Value>,
    /// `LAMPO_QUIET` of the context.
    pub quiet: AtomicBool,
    /// The request and tool of every `which`.
    pub which_requests: Mutex<Vec<(ResolveRequestDto, String)>>,
    /// The site and app of every `open`.
    pub opened: Mutex<Vec<(String, ExternalApp)>>,
    /// Replaces the canned `open` answer (`None`: [`opened_json`], served).
    pub open_result: Mutex<Option<AppResult<Value>>>,
    /// The project files that do not load, in the listings (default: none).
    pub unreadable_projects: Mutex<Vec<UnreadableProjectDto>>,
}

impl Fake {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            site: Mutex::new(Ok(dto(site_json(Some("shop"))))),
            app_running: AtomicBool::new(true),
            ensure_calls: AtomicUsize::new(0),
            ensure_result: Mutex::new(Ok(())),
            sent: Mutex::new(Vec::new()),
            app_replies: Mutex::new(VecDeque::new()),
            progress_to_send: Mutex::new(Vec::new()),
            tail: Mutex::new(Ok(dto(
                json!({"source": "apache", "file": "C:\\l\\apache.log", "lines": [{"text": "one\r\n", "atUnixMs": 1}, {"text": "two", "atUnixMs": null}], "nextOffset": 8}),
            ))),
            tail_requests: Mutex::new(Vec::new()),
            follow_reads: Mutex::new(VecDeque::new()),
            follow_offsets: Mutex::new(Vec::new()),
            doctor_ok: AtomicBool::new(true),
            spawned: Mutex::new(Vec::new()),
            spawn_result: Mutex::new(Ok(0)),
            shim_installed: AtomicBool::new(false),
            hook_payload: Mutex::new(None),
            agent_result: Mutex::new(None),
            agent_requests: Mutex::new(Vec::new()),
            detect_result: Mutex::new(None),
            detect_folders: Mutex::new(Vec::new()),
            extensions_result: Mutex::new(None),
            extensions_php: Mutex::new(Vec::new()),
            toolchain: Mutex::new(toolchain_json()),
            settings: Mutex::new(settings_json()),
            quiet: AtomicBool::new(false),
            which_requests: Mutex::new(Vec::new()),
            opened: Mutex::new(Vec::new()),
            open_result: Mutex::new(None),
            unreadable_projects: Mutex::new(Vec::new()),
        })
    }

    pub fn reply(&self, reply: AppResult<Value>) {
        self.app_replies.lock().unwrap().push_back(reply);
    }

    pub fn resolves_to(&self, site: Option<&str>) {
        *self.site.lock().unwrap() = Ok(dto(site_json(site)));
    }

    pub fn last_request(&self) -> (ControlRequest, CallContext) {
        self.sent.lock().unwrap().last().cloned().expect("a control request was sent")
    }

    /// A context whose every port is this fake; `cwd` is `C:\work\shop`.
    pub fn context(self: &Arc<Self>) -> CliContext {
        CliContext {
            env: CliEnv {
                cwd: r"C:\work\shop".into(),
                inherited: InheritedEnv { quiet: self.quiet.load(Ordering::SeqCst), ..InheritedEnv::default() },
                now_unix_ms: 1_000_000,
                follow_interval: Duration::from_millis(1),
            },
            sites: self.clone(),
            projects: self.clone(),
            detection: self.clone(),
            worktrees: self.clone(),
            logs: self.clone(),
            packages: self.clone(),
            php_extensions: self.clone(),
            toolchain: self.clone(),
            shim_tools: self.clone(),
            opener: self.clone(),
            doctor: self.clone(),
            agent: self.clone(),
            hook_input: self.clone(),
            shims: self.clone(),
            offline_status: self.clone(),
            app: self.clone(),
            runner: self.clone(),
            interrupt: self.clone(),
            settings: self.clone(),
        }
    }
}

impl SiteLookup for Fake {
    fn resolve(&self, _: &ResolveRequestDto) -> AppResult<ResolvedSiteDto> {
        self.site.lock().unwrap().clone()
    }
}

impl ProjectReader for Fake {
    fn summaries(&self) -> AppResult<ProjectSummaryListDto> {
        Ok(ProjectSummaryListDto { projects: vec![dto(summary_json())], unreadable: self.unreadable_projects.lock().unwrap().clone() })
    }
    fn get(&self, id: &str) -> AppResult<ProjectDto> {
        if id == "shop" { Ok(dto(project_json())) } else { Err(AppError::not_found("project", id)) }
    }
}

impl ProjectDetection for Fake {
    fn detect(&self, folder: &str) -> AppResult<DetectionDto> {
        self.detect_folders.lock().unwrap().push(folder.to_string());
        match self.detect_result.lock().unwrap().clone() {
            Some(result) => result.map(dto),
            None => Ok(dto(detection_json(folder))),
        }
    }
}

impl WorktreeReader for Fake {
    fn list(&self, _: Option<&str>) -> AppResult<Vec<WorktreeEnvironmentDto>> {
        Ok(Vec::new())
    }
}

impl LogReader for Fake {
    fn sources(&self, site: Option<&str>) -> AppResult<Vec<LogSourceDto>> {
        Ok(vec![dto(json!({"id": site.map(|s| format!("access:{s}")).unwrap_or("apache".into()), "file": "C:\\l\\a.log", "exists": true}))])
    }
    fn tail(&self, request: &LogTailRequest) -> AppResult<LogLinesDto> {
        self.tail_requests.lock().unwrap().push(request.clone());
        self.tail.lock().unwrap().clone()
    }
    fn read_from(&self, _: &str, offset: u64) -> AppResult<LogLinesDto> {
        self.follow_offsets.lock().unwrap().push(offset);
        Ok(self.follow_reads.lock().unwrap().pop_front().unwrap_or_else(|| dto(json!({"source": "apache", "file": "f", "lines": [], "nextOffset": offset}))))
    }
}

#[async_trait]
impl PackageReader for Fake {
    async fn catalog(&self, _: Option<&str>) -> AppResult<Vec<CatalogEntryDto>> {
        Ok(vec![dto(
            json!({"id": {"kind": "php", "version": "8.3.12", "phpVersion": null}, "channel": "live", "endOfLife": false, "sizeBytes": 1, "vcRuntime": "vs16", "vcRuntimeInstalled": true, "installed": false}),
        )])
    }
    fn installed(&self) -> AppResult<Vec<InstalledPackageDto>> {
        Ok(vec![dto(json!({"id": {"kind": "php", "version": "8.3.12", "phpVersion": null}, "path": "C:\\p", "usedBy": ["shop"]}))])
    }
}

impl PhpExtensionReader for Fake {
    fn list(&self, _: &ResolveRequestDto, php: Option<&str>) -> AppResult<PhpExtensionListDto> {
        self.extensions_php.lock().unwrap().push(php.map(str::to_string));
        match self.extensions_result.lock().unwrap().clone() {
            Some(result) => result.map(dto),
            None => Ok(dto(extensions_json(php.unwrap_or("8.3")))),
        }
    }
}

impl ToolchainReader for Fake {
    fn env(&self, _: &ResolveRequestDto) -> AppResult<ToolchainEnvDto> {
        Ok(dto(self.toolchain.lock().unwrap().clone()))
    }
    fn exec_plan(&self, _: &ResolveRequestDto, command: &[String]) -> AppResult<ExecPlanDto> {
        Ok(ExecPlanDto { program: command[0].clone(), args: command[1..].to_vec(), env: dto(self.toolchain.lock().unwrap().clone()) })
    }
}

/// `php` or `mysql` (with the site's database) of `shop`, resolved from the folder.
pub fn which_json(tool: &str) -> Value {
    let (binary, args): (&str, Vec<&str>) = match tool {
        "php" => (r"C:\php\8.3\php.exe", vec![]),
        "mysql" => (r"C:\mysql\8.4\bin\mysql.exe", vec!["--host=127.0.0.1", "--port=3306", "--user=root", "--default-character-set=utf8mb4", "--database=shop"]),
        _ => unreachable!("no canned `which` for {tool}"),
    };
    json!({"tool": tool, "resolution": site_json(Some("shop")), "reason": "directory", "phpVersion": "8.3", "binary": binary, "defaultArgs": args, "notices": []})
}

impl ShimToolReader for Fake {
    fn which(&self, request: &ResolveRequestDto, tool: &str) -> AppResult<WhichDto> {
        self.which_requests.lock().unwrap().push((request.clone(), tool.to_string()));
        match tool {
            "php" | "mysql" => Ok(dto(which_json(tool))),
            other => Err(AppError::new(ErrorCode::PackageMissing, format!("{other} is not available: not installed for this project"))),
        }
    }
}

/// `site` opened in `app`: its folder, or its URL for the browser.
pub fn opened_json(site: &str, app: ExternalApp) -> Value {
    let target = if app == ExternalApp::Browser { "http://127.0.0.1:8101" } else { r"C:\work\shop" };
    json!({"site": site, "app": app, "target": target, "served": true})
}

impl SiteOpener for Fake {
    fn open(&self, site: &str, app: ExternalApp) -> AppResult<OpenedInDto> {
        self.opened.lock().unwrap().push((site.to_string(), app));
        match self.open_result.lock().unwrap().clone() {
            Some(result) => result.map(dto),
            None => Ok(dto(opened_json(site, app))),
        }
    }
}

#[async_trait]
impl DoctorRunner for Fake {
    async fn run(&self) -> AppResult<DoctorReportDto> {
        let ok = self.doctor_ok.load(Ordering::SeqCst);
        Ok(dto(json!({
            "ok": ok,
            "checks": [{"id": "ports", "status": if ok {"ok"} else {"error"}, "message": "port 80", "fix": if ok {Value::Null} else {json!("stop IIS")}}],
            "missingVcRuntimes": [], "portConflicts": [], "shadowedTools": [], "staleWorktrees": [], "missingPackages": [], "servicesDown": [], "hostsFileInSync": true
        })))
    }
}

impl AgentContextReader for Fake {
    fn context(&self, request: &ResolveRequestDto) -> AppResult<AgentContextDto> {
        self.agent_requests.lock().unwrap().push(request.clone());
        match self.agent_result.lock().unwrap().clone() {
            Some(result) => result.map(dto),
            None => Ok(dto(agent_context_json())),
        }
    }
}

impl HookInput for Fake {
    fn read(&self) -> Option<String> {
        self.hook_payload.lock().unwrap().clone()
    }
}

impl ShimAdmin for Fake {
    fn status(&self) -> AppResult<ShimStatusDto> {
        Ok(ShimStatusDto { shim_dir: r"C:\bin".into(), installed: self.shim_installed.load(Ordering::SeqCst), on_path: false, shadowed: vec![] })
    }
    fn install(&self) -> AppResult<ShimStatusDto> {
        self.shim_installed.store(true, Ordering::SeqCst);
        ShimAdmin::status(self)
    }
    fn uninstall(&self) -> AppResult<ShimStatusDto> {
        self.shim_installed.store(false, Ordering::SeqCst);
        ShimAdmin::status(self)
    }
}

#[async_trait]
impl OfflineStatus for Fake {
    async fn status(&self) -> AppResult<StatusDto> {
        Ok(dto(
            json!({"appRunning": false, "services": [{"service": "apache", "kind": "apache", "label": "Apache", "state": "unknown", "pid": null, "port": 80, "message": null, "required": false, "requiredBy": []}],
                      "projects": [summary_json()], "worktreeCount": 0, "unreadableProjects": *self.unreadable_projects.lock().unwrap()}),
        ))
    }
}

#[async_trait]
impl AppGateway for Fake {
    fn is_running(&self) -> bool {
        self.app_running.load(Ordering::SeqCst)
    }
    async fn ensure_running(&self) -> AppResult<()> {
        self.ensure_calls.fetch_add(1, Ordering::SeqCst);
        let result = self.ensure_result.lock().unwrap().clone();
        if result.is_ok() {
            self.app_running.store(true, Ordering::SeqCst);
        }
        result
    }
    async fn send(&self, request: ControlRequest, context: CallContext, on_progress: &(dyn Fn(ProgressDto) + Send + Sync)) -> AppResult<Value> {
        for p in self.progress_to_send.lock().unwrap().iter() {
            on_progress(p.clone());
        }
        self.sent.lock().unwrap().push((request, context));
        self.app_replies.lock().unwrap().pop_front().unwrap_or_else(|| Err(AppError::new(ErrorCode::Internal, "no canned reply")))
    }
}

#[async_trait]
impl Interrupt for Fake {
    /// Fires once the queued follow reads are consumed.
    async fn wait(&self) {
        loop {
            if self.follow_reads.lock().unwrap().is_empty() && !self.follow_offsets.lock().unwrap().is_empty() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    }
}

/// The default settings with PHP 8.4 as the default.
pub fn settings_json() -> Value {
    json!({
        "defaultPhpVersion": "8.4", "tld": "test", "apachePort": 80,
        "services": [
            {"service": "mysql-8.4", "label": "MySQL 8.4", "port": 3306, "alwaysOn": true},
            {"service": "mysql-5.7", "label": "MySQL 5.7", "port": 3307, "alwaysOn": false},
            {"service": "redis", "label": "Redis", "port": 6379, "alwaysOn": false}
        ],
        "loopbackPortRange": {"start": 8100, "end": 8199}, "closeToTray": true, "autoRestartServices": true, "mysqlProjectCredentials": true, "firstRunCompleted": true,
        "defaultAddressing": "no-elevation"
    })
}

impl SettingsReader for Fake {
    fn get(&self) -> AppResult<SettingsDto> {
        Ok(dto(self.settings.lock().unwrap().clone()))
    }
}

impl ProgramRunner for Fake {
    fn run(&self, spec: &SpawnSpec) -> Result<i32, SpawnError> {
        self.spawned.lock().unwrap().push(spec.clone());
        self.spawn_result.lock().unwrap().clone()
    }
}

/// Runs the real argument parser and dispatcher against the fake; returns `(exit code, stdout, stderr)`.
pub async fn run_cli(fake: &Arc<Fake>, args: &[&str]) -> (i32, String, String) {
    use clap::FromArgMatches;
    let argv: Vec<std::ffi::OsString> = std::iter::once("lampo").chain(args.iter().copied()).map(Into::into).collect();
    let json = crate::wants_json(&argv);
    let (io, out, err): (Io, Capture, Capture) = captured(json);
    let matches = crate::cli::build_command().try_get_matches_from(&argv).expect("valid command line");
    let cli = crate::cli::Cli::from_arg_matches(&matches).unwrap();
    let ctx = fake.context();
    let code = match crate::commands::run(cli, &ctx, &io).await {
        Ok(code) => code,
        Err(e) => crate::error_doc::report(&io, &e),
    };
    (code, out.text(), err.text())
}
