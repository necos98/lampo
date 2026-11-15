//! In-memory fakes of every port, and a [`World`] wiring the real use cases to them.
//! Test-only: nothing here touches the file system, the registry, the network or processes.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use lampo_domain::address::SiteAddress;
use lampo_domain::detection::DetectionHint;
use lampo_domain::events::DomainEvent;
use lampo_domain::hostname::Tld;
use lampo_domain::hosts::HostsBlock;
use lampo_domain::logs::{LogChunk, LogLine, LogQuery};
use lampo_domain::mysql::{DatabaseName, MysqlSeries};
use lampo_domain::package::{CatalogChannel, CatalogEntry, DownloadUrl, InstalledPackage, PackageId, PackageKind, PackagePayload, Sha256Digest};
use lampo_domain::paths::LampoPaths;
use lampo_domain::php::PhpMinor;
use lampo_domain::ports::*;
use lampo_domain::preset::{Preset, PresetId, builtin};
use lampo_domain::progress::{CancelFlag, Progress, ProgressStage};
use lampo_domain::project::{DisplayName, Project, ProjectEnvironment, ProjectId, ProjectRoot};
use lampo_domain::resolution::{GitCheckout, ResolverChain};
use lampo_domain::service::{ServiceId, ServiceKind, ServiceLaunch, ServiceState, ServiceStatus};
use lampo_domain::settings::Settings;
use lampo_domain::shim::ShimIndex;
use lampo_domain::site::WorktreeSlug;
use lampo_domain::vc_runtime::{InstalledVcRuntime, VcRedistributable};
use lampo_domain::version::PackageVersion;
use lampo_domain::web::{PhpIniSpec, WebServerConfig};
use lampo_domain::worktree::WorktreeEnvironment;
use lampo_domain::{PortError, PortResult};

use crate::context::CallContext;
use crate::dto::SyncReportDto;
use crate::error::{AppError, AppResult};
use crate::use_cases::*;

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap()
}

fn key(p: &Path) -> String {
    p.display().to_string().replace('/', "\\").trim_end_matches('\\').to_lowercase()
}

// ---------------------------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------------------------

#[derive(Default)]
pub struct MemProjects(pub Mutex<BTreeMap<ProjectId, Project>>);
impl ProjectRepository for MemProjects {
    fn list(&self) -> PortResult<Vec<Project>> {
        Ok(lock(&self.0).values().cloned().collect())
    }
    fn get(&self, id: &ProjectId) -> PortResult<Option<Project>> {
        Ok(lock(&self.0).get(id).cloned())
    }
    fn save(&self, project: &Project) -> PortResult<()> {
        lock(&self.0).insert(project.id.clone(), project.clone());
        Ok(())
    }
    fn delete(&self, id: &ProjectId) -> PortResult<()> {
        lock(&self.0).remove(id);
        Ok(())
    }
}

#[derive(Default)]
pub struct MemWorktrees(pub Mutex<Vec<WorktreeEnvironment>>);
impl WorktreeRepository for MemWorktrees {
    fn list_all(&self) -> PortResult<Vec<WorktreeEnvironment>> {
        Ok(lock(&self.0).clone())
    }
    fn list_for(&self, parent: &ProjectId) -> PortResult<Vec<WorktreeEnvironment>> {
        Ok(lock(&self.0).iter().filter(|w| &w.parent == parent).cloned().collect())
    }
    fn get(&self, parent: &ProjectId, slug: &WorktreeSlug) -> PortResult<Option<WorktreeEnvironment>> {
        Ok(lock(&self.0).iter().find(|w| &w.parent == parent && &w.slug == slug).cloned())
    }
    fn find_by_root(&self, root: &ProjectRoot) -> PortResult<Option<WorktreeEnvironment>> {
        Ok(lock(&self.0).iter().find(|w| &w.root == root).cloned())
    }
    fn save(&self, env: &WorktreeEnvironment) -> PortResult<()> {
        let mut all = lock(&self.0);
        all.retain(|w| !(w.parent == env.parent && w.slug == env.slug));
        all.push(env.clone());
        Ok(())
    }
    fn delete(&self, parent: &ProjectId, slug: &WorktreeSlug) -> PortResult<()> {
        lock(&self.0).retain(|w| !(&w.parent == parent && &w.slug == slug));
        Ok(())
    }
    fn delete_all_for(&self, parent: &ProjectId) -> PortResult<()> {
        lock(&self.0).retain(|w| &w.parent != parent);
        Ok(())
    }
}

#[derive(Default)]
pub struct MemSettings(pub Mutex<Settings>);
impl SettingsRepository for MemSettings {
    fn load(&self) -> PortResult<Settings> {
        Ok(lock(&self.0).clone())
    }
    fn save(&self, settings: &Settings) -> PortResult<()> {
        *lock(&self.0) = settings.clone();
        Ok(())
    }
}

#[derive(Default)]
pub struct MemPresets(pub Mutex<Vec<Preset>>);
impl CustomPresetRepository for MemPresets {
    fn list(&self) -> PortResult<Vec<Preset>> {
        Ok(lock(&self.0).clone())
    }
    fn save(&self, preset: &Preset) -> PortResult<()> {
        let mut all = lock(&self.0);
        all.retain(|p| p.id != preset.id);
        all.push(preset.clone());
        Ok(())
    }
    fn delete(&self, id: &PresetId) -> PortResult<()> {
        lock(&self.0).retain(|p| &p.id != id);
        Ok(())
    }
}

#[derive(Default)]
pub struct MemShimIndex {
    pub index: Mutex<Option<ShimIndex>>,
    pub writes: AtomicUsize,
}
impl MemShimIndex {
    pub fn current(&self) -> ShimIndex {
        lock(&self.index).clone().unwrap_or_default()
    }
}
impl ShimIndexWriter for MemShimIndex {
    fn write(&self, index: &ShimIndex) -> PortResult<()> {
        *lock(&self.index) = Some(index.clone());
        self.writes.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}
impl ShimIndexReader for MemShimIndex {
    fn read(&self) -> PortResult<ShimIndex> {
        Ok(self.current())
    }
}

#[derive(Default)]
pub struct MemInstalled {
    pub packages: Mutex<Vec<InstalledPackage>>,
    pub removed: Mutex<Vec<PackageId>>,
}
impl MemInstalled {
    pub fn add(&self, kind: PackageKind, version: &str, php: Option<&str>) -> InstalledPackage {
        let id = PackageId { kind, version: PackageVersion::parse(version).unwrap(), php: php.map(|p| PhpMinor::parse(p).unwrap()) };
        let pkg = fake_layout(InstalledPackage::new(id.clone(), self.install_dir(&id), None));
        self.record(&pkg).unwrap();
        pkg
    }
    pub fn has(&self, kind: &PackageKind, version: &str) -> bool {
        lock(&self.packages).iter().any(|p| &p.id.kind == kind && p.id.version.as_str() == version)
    }
}
impl InstalledPackageRegistry for MemInstalled {
    fn list(&self) -> PortResult<Vec<InstalledPackage>> {
        Ok(lock(&self.packages).clone())
    }
    fn install_dir(&self, id: &PackageId) -> PathBuf {
        PathBuf::from("C:\\lampo\\packages").join(id.key())
    }
    fn record(&self, package: &InstalledPackage) -> PortResult<()> {
        let mut all = lock(&self.packages);
        all.retain(|p| p.id != package.id);
        all.push(fake_layout(package.clone()));
        Ok(())
    }
    fn remove(&self, id: &PackageId) -> PortResult<()> {
        lock(&self.packages).retain(|p| &p.id != id);
        lock(&self.removed).push(id.clone());
        Ok(())
    }
}

#[derive(Default)]
pub struct FakeFs(pub Mutex<BTreeSet<String>>);
impl FakeFs {
    pub fn add(&self, path: &str) {
        lock(&self.0).insert(key(Path::new(path)));
    }
    pub fn remove(&self, path: &str) {
        lock(&self.0).remove(&key(Path::new(path)));
    }
}
impl FileSystemInspector for FakeFs {
    fn is_dir(&self, path: &Path) -> bool {
        lock(&self.0).contains(&key(path))
    }
}

/// Answers with the deepest registered checkout containing the folder.
#[derive(Default)]
pub struct FakeGit(pub Mutex<Vec<GitCheckout>>);
impl FakeGit {
    pub fn linked(&self, worktree: &str, main: &str, branch: Option<&str>) {
        lock(&self.0).push(GitCheckout { worktree_root: ProjectRoot::new(worktree).unwrap(), main_root: Some(ProjectRoot::new(main).unwrap()), branch: branch.map(str::to_string) });
    }
}
impl GitInspector for FakeGit {
    fn inspect(&self, folder: &str) -> PortResult<Option<GitCheckout>> {
        Ok(lock(&self.0).iter().filter(|c| c.worktree_root.contains(folder)).max_by_key(|c| c.worktree_root.depth()).cloned())
    }
}

#[derive(Default)]
pub struct FakeDetector(pub Option<DetectionHint>, pub bool);
impl ProjectDetector for FakeDetector {
    fn name(&self) -> &str {
        "fake"
    }
    fn detect(&self, _folder: &Path) -> PortResult<Option<DetectionHint>> {
        if self.1 {
            return Err(PortError::io("reading composer.json", "boom"));
        }
        Ok(self.0.clone())
    }
}

#[derive(Default)]
pub struct FakeTransfer {
    pub exported: Mutex<Vec<(ProjectId, PathBuf)>>,
    pub to_import: Mutex<Option<Project>>,
}
impl ProjectTransfer for FakeTransfer {
    fn export_to(&self, project: &Project, file: &Path) -> PortResult<()> {
        lock(&self.exported).push((project.id.clone(), file.to_path_buf()));
        Ok(())
    }
    fn import_from(&self, file: &Path) -> PortResult<Project> {
        lock(&self.to_import).clone().ok_or_else(|| PortError::not_found("file", file.display().to_string()))
    }
}

/// `.0`: installed; `.1`: how many refreshes changed something (only when installed).
#[derive(Default)]
pub struct FakeShimInstaller(pub AtomicBool, pub AtomicUsize);
impl ShimInstaller for FakeShimInstaller {
    fn refresh(&self, _shim_dir: &Path) -> PortResult<bool> {
        let installed = self.0.load(Ordering::SeqCst);
        if installed {
            self.1.fetch_add(1, Ordering::SeqCst);
        }
        Ok(installed)
    }
    fn install(&self, _shim_dir: &Path) -> PortResult<()> {
        self.0.store(true, Ordering::SeqCst);
        Ok(())
    }
    fn is_installed(&self, _shim_dir: &Path) -> PortResult<bool> {
        Ok(self.0.load(Ordering::SeqCst))
    }
}

/// Counts the recordings of the app location.
#[derive(Default)]
pub struct FakeAppLocation(pub AtomicUsize);
impl AppLocationRecorder for FakeAppLocation {
    fn record_running_app(&self) -> PortResult<()> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[derive(Default)]
pub struct FakeUserPath(pub Mutex<Vec<PathBuf>>);
impl UserPathEditor for FakeUserPath {
    fn contains(&self, dir: &Path) -> PortResult<bool> {
        Ok(lock(&self.0).iter().any(|d| key(d) == key(dir)))
    }
    fn append(&self, dir: &Path) -> PortResult<()> {
        lock(&self.0).push(dir.to_path_buf());
        Ok(())
    }
}

#[derive(Default)]
pub struct FakeLocator(pub Mutex<BTreeMap<String, Vec<PathBuf>>>);
impl ExecutableLocator for FakeLocator {
    fn find_all(&self, name: &str) -> PortResult<Vec<PathBuf>> {
        Ok(lock(&self.0).get(name).cloned().unwrap_or_default())
    }
}

pub struct FixedClock(pub u64);
impl Clock for FixedClock {
    fn now_unix_ms(&self) -> u64 {
        self.0
    }
}

// ---------------------------------------------------------------------------------------------
// Web
// ---------------------------------------------------------------------------------------------

/// Rejects any configuration containing a vhost whose directives mention `Broken`, or every
/// configuration when `reject_all` is set. Records what was applied (validated + swapped).
#[derive(Default)]
pub struct FakeWeb {
    pub applied: Mutex<Vec<WebServerConfig>>,
    pub attempts: AtomicUsize,
    pub reject_all: AtomicBool,
}
impl FakeWeb {
    pub fn last(&self) -> Option<WebServerConfig> {
        lock(&self.applied).last().cloned()
    }
}
#[async_trait]
impl WebServerConfigurator for FakeWeb {
    async fn apply(&self, config: &WebServerConfig) -> PortResult<()> {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        if self.reject_all.load(Ordering::SeqCst) {
            return Err(PortError::ConfigRejected { output: "Syntax error in httpd.conf".into() });
        }
        if let Some(v) = config.vhosts.iter().find(|v| v.custom_directives.as_str().contains("Broken")) {
            return Err(PortError::ConfigRejected { output: format!("Invalid command 'Broken' in {}", v.site) });
        }
        lock(&self.applied).push(config.clone());
        Ok(())
    }
}

#[derive(Default)]
pub struct FakeIni(pub Mutex<Vec<PhpIniSpec>>);
impl PhpIniWriter for FakeIni {
    fn write(&self, spec: &PhpIniSpec) -> PortResult<()> {
        let mut all = lock(&self.0);
        all.retain(|s| s.output_dir != spec.output_dir);
        all.push(spec.clone());
        Ok(())
    }
}

#[derive(Default)]
pub struct FakeHosts {
    pub block: Mutex<Option<HostsBlock>>,
    pub writes: AtomicUsize,
    pub deny: AtomicBool,
}
impl HostsFileReader for FakeHosts {
    fn read_block(&self) -> PortResult<Option<HostsBlock>> {
        Ok(lock(&self.block).clone())
    }
}
#[async_trait]
impl HostsFileWriter for FakeHosts {
    async fn write_block(&self, block: &HostsBlock) -> PortResult<()> {
        if self.deny.load(Ordering::SeqCst) {
            return Err(PortError::ElevationDenied);
        }
        self.writes.fetch_add(1, Ordering::SeqCst);
        *lock(&self.block) = Some(block.clone());
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------
// Services
// ---------------------------------------------------------------------------------------------

/// A runner keeping states in memory and recording `prepare:<id>`, `start:<id>`, … calls.
pub struct FakeRunner {
    pub kind: ServiceKind,
    pub states: Mutex<HashMap<ServiceId, ServiceState>>,
    pub calls: Mutex<Vec<String>>,
    pub launches: Mutex<Vec<ServiceLaunch>>,
}
impl FakeRunner {
    pub fn new(kind: ServiceKind) -> Arc<Self> {
        Arc::new(Self { kind, states: Mutex::default(), calls: Mutex::default(), launches: Mutex::default() })
    }
    pub fn calls(&self) -> Vec<String> {
        lock(&self.calls).clone()
    }
    pub fn clear(&self) {
        lock(&self.calls).clear();
    }
    pub fn state(&self, id: &ServiceId) -> ServiceState {
        lock(&self.states).get(id).copied().unwrap_or(ServiceState::Stopped)
    }
    pub fn set(&self, id: &ServiceId, state: ServiceState) {
        lock(&self.states).insert(id.clone(), state);
    }
}
#[async_trait]
impl ServiceRunner for FakeRunner {
    fn kind(&self) -> ServiceKind {
        self.kind.clone()
    }
    async fn prepare(&self, launch: &ServiceLaunch) -> PortResult<()> {
        lock(&self.calls).push(format!("prepare:{}", launch.service));
        Ok(())
    }
    async fn start(&self, launch: &ServiceLaunch) -> PortResult<()> {
        lock(&self.calls).push(format!("start:{}", launch.service));
        lock(&self.launches).push(launch.clone());
        self.set(&launch.service, ServiceState::Running);
        Ok(())
    }
    async fn stop(&self, service: &ServiceId) -> PortResult<()> {
        lock(&self.calls).push(format!("stop:{service}"));
        self.set(service, ServiceState::Stopped);
        Ok(())
    }
    async fn restart(&self, launch: &ServiceLaunch) -> PortResult<()> {
        lock(&self.calls).push(format!("restart:{}", launch.service));
        lock(&self.launches).push(launch.clone());
        self.set(&launch.service, ServiceState::Running);
        Ok(())
    }
    fn status(&self, service: &ServiceId) -> ServiceStatus {
        ServiceStatus { state: self.state(service), ..ServiceStatus::stopped(service.clone()) }
    }
}

#[derive(Default)]
pub struct FakePorts(pub Mutex<BTreeMap<u16, Option<String>>>, pub std::sync::atomic::AtomicU64);
impl FakePorts {
    pub fn busy(&self, port: u16, owner: &str) {
        lock(&self.0).insert(port, Some(owner.into()));
    }
    /// Makes every probe take `ms` milliseconds (widens race windows in concurrency tests).
    pub fn slow(&self, ms: u64) {
        self.1.store(ms, Ordering::SeqCst);
    }
}
impl PortProbe for FakePorts {
    fn check(&self, port: u16) -> PortResult<PortAvailability> {
        let delay = self.1.load(Ordering::SeqCst);
        if delay > 0 {
            std::thread::sleep(Duration::from_millis(delay));
        }
        Ok(match lock(&self.0).get(&port) {
            Some(owner) => PortAvailability::InUse { owner: owner.clone() },
            None => PortAvailability::Free,
        })
    }
}

#[derive(Default)]
pub struct FakeDb {
    pub dbs: Mutex<BTreeSet<String>>,
    pub calls: Mutex<Vec<String>>,
    pub fail_clone: AtomicBool,
    /// `create` fails (MySQL error) without creating anything.
    pub fail_create: AtomicBool,
}
impl FakeDb {
    pub fn has(&self, name: &str) -> bool {
        lock(&self.dbs).contains(name)
    }
    pub fn calls(&self) -> Vec<String> {
        lock(&self.calls).clone()
    }
}
#[async_trait]
impl DatabaseAdmin for FakeDb {
    async fn list(&self, _e: &MysqlEndpoint) -> PortResult<Vec<String>> {
        Ok(lock(&self.dbs).iter().cloned().collect())
    }
    async fn create(&self, e: &MysqlEndpoint, name: &DatabaseName) -> PortResult<()> {
        lock(&self.calls).push(format!("create:{}:{name}", e.port));
        if self.fail_create.load(Ordering::SeqCst) {
            return Err(PortError::process("mysql", "ERROR 1044: access denied"));
        }
        if !lock(&self.dbs).insert(name.to_string()) {
            return Err(PortError::AlreadyExists { entity: "database", id: name.to_string() });
        }
        Ok(())
    }
    async fn drop_database(&self, e: &MysqlEndpoint, name: &DatabaseName) -> PortResult<()> {
        lock(&self.calls).push(format!("drop:{}:{name}", e.port));
        lock(&self.dbs).remove(name.as_str());
        Ok(())
    }
    async fn import(&self, _e: &MysqlEndpoint, name: &DatabaseName, file: &Path, progress: &dyn ProgressSink, cancel: &CancelFlag) -> PortResult<()> {
        cancel.check()?;
        progress.report(Progress::new(ProgressStage::Configuring, 1, Some(1)));
        lock(&self.calls).push(format!("import:{name}:{}", file.display()));
        Ok(())
    }
    async fn export(&self, _e: &MysqlEndpoint, name: &DatabaseName, file: &Path, _p: &dyn ProgressSink, cancel: &CancelFlag) -> PortResult<()> {
        cancel.check()?;
        lock(&self.calls).push(format!("export:{name}:{}", file.display()));
        Ok(())
    }
    async fn clone_database(&self, _e: &MysqlEndpoint, source: &DatabaseName, target: &DatabaseName, progress: &dyn ProgressSink, cancel: &CancelFlag) -> PortResult<()> {
        lock(&self.calls).push(format!("clone:{source}->{target}"));
        lock(&self.dbs).insert(target.to_string());
        progress.report(Progress::new(ProgressStage::Configuring, 50, Some(100)));
        cancel.check()?;
        if self.fail_clone.load(Ordering::SeqCst) {
            return Err(PortError::process("mysqldump", "exit code 2"));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------
// Packages
// ---------------------------------------------------------------------------------------------

pub fn digest(byte: u8) -> Sha256Digest {
    Sha256Digest::from_bytes([byte; 32])
}

pub fn catalog_entry(kind: PackageKind, version: &str, php: Option<&str>) -> CatalogEntry {
    let id = PackageId { kind, version: PackageVersion::parse(version).unwrap(), php: php.map(|p| PhpMinor::parse(p).unwrap()) };
    CatalogEntry {
        url: DownloadUrl::new(format!("https://example.invalid/{}.zip", id.key())).unwrap(),
        id,
        sha256: Some(digest(7)),
        size_bytes: Some(1000),
        vc_runtime: None,
        channel: CatalogChannel::Frozen,
        end_of_life: false,
        payload: PackagePayload::Zip { strip_prefix: None },
    }
}

pub struct FakeSource {
    pub name: &'static str,
    pub entries: Mutex<Vec<CatalogEntry>>,
    pub fail: AtomicBool,
}
impl FakeSource {
    pub fn new(name: &'static str, entries: Vec<CatalogEntry>) -> Arc<Self> {
        Arc::new(Self { name, entries: Mutex::new(entries), fail: AtomicBool::new(false) })
    }
}
#[async_trait]
impl PackageCatalogSource for FakeSource {
    fn name(&self) -> &str {
        self.name
    }
    fn kinds(&self) -> Vec<PackageKind> {
        let mut kinds: Vec<PackageKind> = lock(&self.entries).iter().map(|e| e.id.kind.clone()).collect();
        kinds.sort();
        kinds.dedup();
        kinds
    }
    async fn fetch(&self, kind: &PackageKind) -> PortResult<Vec<CatalogEntry>> {
        if self.fail.load(Ordering::SeqCst) {
            return Err(PortError::Network("offline".into()));
        }
        Ok(lock(&self.entries).iter().filter(|e| &e.id.kind == kind).cloned().collect())
    }
}

/// Reports two progress chunks; cancels itself between them when `cancel_midway` is set.
#[derive(Default)]
pub struct FakeDownloader {
    pub downloads: Mutex<Vec<PathBuf>>,
    pub discarded: Mutex<Vec<PathBuf>>,
    pub cancel_midway: AtomicBool,
}
#[async_trait]
impl PackageDownloader for FakeDownloader {
    async fn download(&self, _url: &DownloadUrl, destination: &Path, progress: &dyn ProgressSink, cancel: &CancelFlag) -> PortResult<u64> {
        lock(&self.downloads).push(destination.to_path_buf());
        progress.report(Progress::new(ProgressStage::Downloading, 500, Some(1000)));
        if self.cancel_midway.load(Ordering::SeqCst) {
            cancel.cancel();
        }
        cancel.check()?;
        progress.report(Progress::new(ProgressStage::Downloading, 1000, Some(1000)));
        Ok(1000)
    }
    async fn discard(&self, destination: &Path) -> PortResult<()> {
        lock(&self.discarded).push(destination.to_path_buf());
        Ok(())
    }
}

pub struct FakeChecksum(pub Mutex<Sha256Digest>);
#[async_trait]
impl ChecksumCalculator for FakeChecksum {
    async fn sha256(&self, _file: &Path) -> PortResult<Sha256Digest> {
        Ok(*lock(&self.0))
    }
}

#[derive(Default)]
pub struct FakeExtractor(pub Mutex<Vec<(PathBuf, PathBuf)>>);
#[async_trait]
impl ArchiveExtractor for FakeExtractor {
    async fn extract_zip(&self, archive: &Path, destination: &Path, _strip: Option<&str>, progress: &dyn ProgressSink, cancel: &CancelFlag) -> PortResult<()> {
        cancel.check()?;
        progress.report(Progress::new(ProgressStage::Extracting, 1, Some(1)));
        lock(&self.0).push((archive.to_path_buf(), destination.to_path_buf()));
        Ok(())
    }
}

pub struct FakeHook {
    pub kind: PackageKind,
    pub calls: Mutex<Vec<PackageId>>,
}
#[async_trait]
impl PackageInstallHook for FakeHook {
    fn kind(&self) -> PackageKind {
        self.kind.clone()
    }
    async fn after_install(&self, package: &InstalledPackage) -> PortResult<()> {
        lock(&self.calls).push(package.id.clone());
        Ok(())
    }
}

pub struct FakeVc(pub Mutex<Vec<InstalledVcRuntime>>);
impl FakeVc {
    pub fn modern() -> Self {
        Self(Mutex::new(vec![InstalledVcRuntime { redistributable: VcRedistributable::Vs2015To2022X64, version: (14, 40, 33810) }]))
    }
}
impl VcRuntimeDetector for FakeVc {
    fn installed(&self) -> PortResult<Vec<InstalledVcRuntime>> {
        Ok(lock(&self.0).clone())
    }
}

// ---------------------------------------------------------------------------------------------
// Control, logs, events, progress
// ---------------------------------------------------------------------------------------------

pub struct FakeControl(pub AtomicBool);
#[async_trait]
impl ControlChannelClient for FakeControl {
    fn is_server_running(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
    async fn exchange(&self, _request: String, _events: &dyn ControlFrameSink, _timeout: Duration) -> PortResult<String> {
        Err(PortError::not_found("app", "pipe"))
    }
}

/// Files are keys of the map; a missing file yields an empty chunk.
#[derive(Default)]
pub struct FakeLogs(pub Mutex<BTreeMap<String, Vec<String>>>);
impl FakeLogs {
    fn chunk(&self, file: &Path, skip: usize) -> LogChunk {
        let files = lock(&self.0);
        let Some(lines) = files.get(&key(file)) else { return LogChunk::default() };
        let size: usize = lines.iter().map(|l| l.len() + 1).sum();
        LogChunk { lines: lines.iter().skip(skip).map(|t| LogLine { text: t.clone(), at_unix_ms: None }).collect(), next_offset: size as u64 }
    }
}
impl LogFileReader for FakeLogs {
    fn tail(&self, file: &Path, query: &LogQuery) -> PortResult<LogChunk> {
        let mut c = self.chunk(file, 0);
        let n = c.lines.len().saturating_sub(query.max_lines);
        c.lines.drain(..n);
        Ok(c)
    }
    fn read_from(&self, file: &Path, offset: u64) -> PortResult<LogChunk> {
        let full = self.chunk(file, 0);
        let mut pos = 0u64;
        let lines = full
            .lines
            .into_iter()
            .filter(|l| {
                let keep = pos >= offset;
                pos += l.text.len() as u64 + 1;
                keep
            })
            .collect();
        Ok(LogChunk { lines, next_offset: full.next_offset })
    }
}

#[derive(Default)]
pub struct RecordingEvents(pub Mutex<Vec<DomainEvent>>);
impl RecordingEvents {
    pub fn all(&self) -> Vec<DomainEvent> {
        lock(&self.0).clone()
    }
}
impl EventPublisher for RecordingEvents {
    fn publish(&self, event: DomainEvent) {
        lock(&self.0).push(event);
    }
}

#[derive(Default)]
pub struct RecordingProgress(pub Mutex<Vec<Progress>>);
impl RecordingProgress {
    pub fn stages(&self) -> Vec<ProgressStage> {
        let mut s: Vec<ProgressStage> = lock(&self.0).iter().map(|p| p.stage).collect();
        s.dedup();
        s
    }
}
impl ProgressSink for RecordingProgress {
    fn report(&self, progress: Progress) {
        lock(&self.0).push(progress);
    }
}

/// An `EnvironmentSync` that counts calls and returns a configurable error.
#[derive(Default)]
pub struct CountingSync {
    pub calls: AtomicUsize,
    pub fail_with: Mutex<Option<AppError>>,
}
#[async_trait]
impl EnvironmentSync for CountingSync {
    async fn sync(&self, _ctx: &CallContext) -> AppResult<SyncReportDto> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        match lock(&self.fail_with).clone() {
            Some(e) => Err(e),
            None => Ok(SyncReportDto::default()),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// World
// ---------------------------------------------------------------------------------------------

pub fn php(s: &str) -> PhpMinor {
    PhpMinor::parse(s).unwrap()
}

/// Every fake, plus the real `ApplyEnvironment` / `ServiceControl` wired to them.
pub struct World {
    pub paths: LampoPaths,
    pub projects: Arc<MemProjects>,
    pub worktrees: Arc<MemWorktrees>,
    pub settings: Arc<MemSettings>,
    pub presets: Arc<MemPresets>,
    pub shim_index: Arc<MemShimIndex>,
    pub installed: Arc<MemInstalled>,
    pub web: Arc<FakeWeb>,
    pub ini: Arc<FakeIni>,
    pub hosts: Arc<FakeHosts>,
    pub apache: Arc<FakeRunner>,
    pub mysql: Arc<FakeRunner>,
    pub redis: Arc<FakeRunner>,
    pub ports: Arc<FakePorts>,
    pub fs: Arc<FakeFs>,
    pub git: Arc<FakeGit>,
    pub db: Arc<FakeDb>,
    pub events: Arc<RecordingEvents>,
    pub control: Arc<FakeControl>,
    pub env: Arc<ApplyEnvironment>,
    pub service_control: Arc<ServiceControl>,
    /// The one allocator shared by every use case of this world (as in the composition roots).
    pub allocator: Arc<LoopbackPortAllocator>,
    /// The registered service kinds (the built-ins).
    pub services: Arc<ServiceCatalog>,
}

impl World {
    pub fn new() -> Self {
        Self::with_kinds(Vec::new(), Vec::new())
    }

    /// The built-in service kinds plus `extra_kinds`, run by the three fake runners plus
    /// `extra_runners` (open-closed: a new kind is a descriptor and a runner).
    pub fn with_kinds(extra_kinds: Vec<Arc<dyn lampo_domain::service::ServiceDescriptor>>, extra_runners: Vec<Arc<dyn ServiceRunner>>) -> Self {
        let paths = LampoPaths::new("C:\\cfg", "C:\\data");
        let projects = Arc::new(MemProjects::default());
        let worktrees = Arc::new(MemWorktrees::default());
        let settings = Arc::new(MemSettings::default());
        let installed = Arc::new(MemInstalled::default());
        let web = Arc::new(FakeWeb::default());
        let ini = Arc::new(FakeIni::default());
        let hosts = Arc::new(FakeHosts::default());
        let shim_index = Arc::new(MemShimIndex::default());
        let apache = FakeRunner::new(ServiceKind::APACHE);
        let mysql = FakeRunner::new(ServiceKind::MYSQL);
        let redis = FakeRunner::new(ServiceKind::REDIS);
        let mut kinds = lampo_domain::service::builtin::all();
        kinds.extend(extra_kinds);
        let services = Arc::new(ServiceCatalog::new(kinds));
        let ports = Arc::new(FakePorts::default());
        let events = Arc::new(RecordingEvents::default());
        let mut runners: Vec<Arc<dyn ServiceRunner>> = vec![apache.clone(), mysql.clone(), redis.clone()];
        runners.extend(extra_runners);
        let env = Arc::new(ApplyEnvironment::new(ApplyEnvironmentDeps {
            projects: projects.clone(),
            worktrees: worktrees.clone(),
            settings: settings.clone(),
            installed: installed.clone(),
            web_config: web.clone(),
            php_ini: ini.clone(),
            hosts_reader: hosts.clone(),
            hosts_writer: hosts.clone(),
            shim_index: shim_index.clone(),
            services: services.clone(),
            runners: runners.clone(),
            port_probe: ports.clone(),
            paths: paths.clone(),
            events: events.clone(),
        }));
        let service_control = Arc::new(ServiceControl::new(ServiceControlDeps {
            runners,
            catalog: services.clone(),
            installed: installed.clone(),
            settings: settings.clone(),
            port_probe: ports.clone(),
            paths: paths.clone(),
            env: env.clone(),
        }));
        let allocator = Arc::new(LoopbackPortAllocator::new(projects.clone(), worktrees.clone(), settings.clone(), ports.clone()));
        Self {
            paths,
            projects,
            worktrees,
            settings,
            presets: Arc::new(MemPresets::default()),
            shim_index,
            installed,
            web,
            ini,
            hosts,
            apache,
            mysql,
            redis,
            ports,
            fs: Arc::new(FakeFs::default()),
            git: Arc::new(FakeGit::default()),
            db: Arc::new(FakeDb::default()),
            events,
            control: Arc::new(FakeControl(AtomicBool::new(true))),
            env,
            service_control,
            allocator,
            services,
        }
    }

    /// Apache, mod_fcgid, PHP 8.3 and 8.4, MySQL 8.4 and 5.7, Redis, Composer stable; first run done.
    pub fn with_basics() -> Self {
        let w = Self::new();
        for (kind, v) in [
            (PackageKind::APACHE, "2.4.62"),
            (PackageKind::MOD_FCGID, "2.3.10"),
            (PackageKind::PHP, "8.3.14"),
            (PackageKind::PHP, "8.4.1"),
            (PackageKind::MYSQL, "8.4.3"),
            (PackageKind::MYSQL, "5.7.44"),
            (PackageKind::REDIS, "5.0.14.1"),
            (PackageKind::COMPOSER, "2.8.4"),
        ] {
            w.installed.add(kind, v, None);
        }
        w.settings.0.lock().unwrap().default_php = Some(php("8.4"));
        w.settings.0.lock().unwrap().first_run_completed = true;
        w
    }

    /// Saves a project (`<id>.test` + `<id>.localhost`, PHP 8.3, MySQL 8.4) and creates its folder.
    pub fn project(&self, id: &str, root: &str, active: bool) -> Project {
        let pid = ProjectId::new(id).unwrap();
        let addresses = SiteAddress::defaults_for(id, &Tld::default()).unwrap();
        let mut env = ProjectEnvironment::minimal(php("8.3"));
        env.set_mysql(Some(MysqlSeries::DEFAULT));
        let mut p = Project::new(pid, DisplayName::new(id).unwrap(), ProjectRoot::new(root).unwrap(), addresses, env).unwrap();
        p.active = active;
        self.projects.save(&p).unwrap();
        self.fs.add(root);
        p
    }

    pub fn update_project(&self, id: &str, f: impl FnOnce(&mut Project)) {
        let mut p = self.projects.get(&ProjectId::new(id).unwrap()).unwrap().unwrap();
        f(&mut p);
        self.projects.save(&p).unwrap();
    }

    /// Hosts file already matching the projects (so syncs do not need elevation).
    pub fn hosts_in_sync(&self) {
        *self.hosts.block.lock().unwrap() = Some(HostsBlock::for_projects(&self.projects.list().unwrap()));
    }

    pub fn worktree_databases(&self) -> Arc<WorktreeDatabases> {
        Arc::new(WorktreeDatabases::new(self.projects.clone(), self.settings.clone(), self.installed.clone(), self.db.clone(), self.service_control.clone()))
    }

    pub fn worktree_queries(&self) -> Arc<WorktreeQueries> {
        Arc::new(WorktreeQueries::new(self.worktrees.clone(), self.projects.clone(), self.settings.clone(), self.fs.clone()))
    }

    pub fn worktree_up(&self) -> WorktreeUp {
        WorktreeUp::new(WorktreeUpDeps {
            projects: self.projects.clone(),
            worktrees: self.worktrees.clone(),
            git: self.git.clone(),
            clock: Arc::new(FixedClock(1_700_000_000_000)),
            allocator: self.allocator.clone(),
            databases: self.worktree_databases(),
            queries: self.worktree_queries(),
            env: self.env.clone(),
            events: self.events.clone(),
        })
    }

    pub fn worktree_down(&self) -> WorktreeDown {
        WorktreeDown::new(self.worktrees.clone(), self.worktree_databases(), self.env.clone(), self.events.clone())
    }

    pub fn prune_worktrees(&self) -> PruneWorktrees {
        PruneWorktrees::new(self.projects.clone(), self.worktrees.clone(), self.fs.clone(), self.worktree_databases(), self.env.clone(), self.events.clone())
    }

    pub fn database_service(&self) -> Arc<DatabaseService> {
        Arc::new(DatabaseService::new(self.db.clone(), self.projects.clone(), self.worktrees.clone(), self.settings.clone(), self.installed.clone(), self.service_control.clone()))
    }

    pub fn presets(&self) -> Arc<PresetQueries> {
        Arc::new(PresetQueries::new(builtin::all(), self.presets.clone(), self.settings.clone(), self.installed.clone()))
    }

    pub fn project_queries(&self) -> Arc<ProjectQueries> {
        Arc::new(ProjectQueries::new(self.projects.clone(), self.worktrees.clone(), self.settings.clone(), self.fs.clone()))
    }

    /// The writer every project-mutating use case takes, syncing through `env`.
    pub fn project_writer_with(&self, env: Arc<dyn EnvironmentSync>) -> ProjectWriter {
        ProjectWriter {
            projects: self.projects.clone(),
            allocator: self.allocator.clone(),
            env,
            events: self.events.clone(),
            queries: self.project_queries(),
            services: self.services.clone(),
        }
    }

    pub fn project_writer(&self) -> ProjectWriter {
        self.project_writer_with(self.env.clone())
    }

    pub fn import_project(&self, transfer: Arc<FakeTransfer>) -> ImportProject {
        ImportProject::new(self.project_writer(), transfer, self.fs.clone())
    }

    pub fn relink_project(&self) -> RelinkProject {
        RelinkProject::new(self.project_writer(), self.fs.clone())
    }

    pub fn resolver(&self) -> Arc<SiteResolver> {
        Arc::new(SiteResolver::new(self.shim_index.clone(), ResolverChain::standard(self.git.clone())))
    }

    pub fn runners(&self) -> Vec<Arc<dyn ServiceRunner>> {
        vec![self.apache.clone(), self.mysql.clone(), self.redis.clone()]
    }

    pub fn all_calls(&self) -> Vec<String> {
        [self.apache.calls(), self.mysql.calls(), self.redis.calls()].concat()
    }

    pub fn clear_calls(&self) {
        self.apache.clear();
        self.mysql.clear();
        self.redis.clear();
    }
}

impl World {
    /// `UninstallPackage` regenerating files through this world's `ApplyEnvironment`.
    pub fn uninstall_package(&self) -> UninstallPackage {
        UninstallPackage::new(UninstallPackageDeps {
            installed: self.installed.clone(),
            projects: self.projects.clone(),
            worktrees: self.worktrees.clone(),
            settings: self.settings.clone(),
            events: self.events.clone(),
            files: self.env.clone(),
            services: self.services.clone(),
        })
    }

    pub fn shim_setup(&self) -> Arc<ShimSetup> {
        Arc::new(ShimSetup::new(
            Arc::new(FakeShimInstaller::default()),
            Arc::new(FakeUserPath::default()),
            Arc::new(FakeLocator::default()),
            self.paths.clone(),
            lampo_domain::shim::tools::all(),
        ))
    }
}

/// A clock that only moves when someone sleeps on it (`Clock` + `Timer`), so polling loops with
/// timeouts run instantly and deterministically.
#[derive(Default)]
pub struct ManualClock {
    pub now_ms: std::sync::atomic::AtomicU64,
    pub sleeps: AtomicUsize,
}
impl Clock for ManualClock {
    fn now_unix_ms(&self) -> u64 {
        self.now_ms.load(Ordering::SeqCst)
    }
}
#[async_trait]
impl Timer for ManualClock {
    async fn sleep(&self, duration: Duration) {
        self.sleeps.fetch_add(1, Ordering::SeqCst);
        self.now_ms.fetch_add(duration.as_millis() as u64, Ordering::SeqCst);
    }
}

/// Records headless launches.
pub struct FakeLauncher {
    pub launches: AtomicUsize,
}
impl AppLauncher for FakeLauncher {
    fn launch_headless(&self) -> PortResult<()> {
        self.launches.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

/// What the real registry adapter does when listing (`lampo_infra_packages::layout`), for the
/// kinds the tests install. Fakes model adapters; the application itself never joins file names.
pub fn fake_layout(mut p: InstalledPackage) -> InstalledPackage {
    use lampo_domain::package::EntryRole;
    let dir = p.install_dir.clone();
    let entries: Vec<(EntryRole, PathBuf)> = match p.id.kind.as_str() {
        "php" => vec![(EntryRole::Cli, dir.join("php.exe")), (EntryRole::FastCgi, dir.join("php-cgi.exe"))],
        "mod-fcgid" => vec![(EntryRole::ApacheModule, dir.join("mod_fcgid.so"))],
        "mysql" => vec![(EntryRole::ClientDir, dir.join("bin"))],
        "redis" => vec![(EntryRole::Cli, dir.join("redis-cli.exe")), (EntryRole::ClientDir, dir.clone())],
        "composer" => vec![(EntryRole::Phar, dir.join("composer.phar"))],
        "xdebug" | "php-redis" => {
            let name = if p.id.kind.as_str() == "xdebug" { "xdebug" } else { "redis" };
            p.php_extension = Some(lampo_domain::php::ExtensionName::new(name).unwrap());
            vec![(EntryRole::PhpExtension, dir.join(format!("php_{name}.dll")))]
        }
        _ => vec![],
    };
    p.entry_points = entries.into_iter().collect();
    p
}

/// `mysql-<series>` (`mysql("8.4")`).
pub fn mysql_id(series: &str) -> ServiceId {
    format!("mysql-{series}").parse().unwrap()
}

/// `redis`.
pub fn redis_id() -> ServiceId {
    ServiceId::new(ServiceKind::REDIS, None)
}

impl World {
    /// The doctor with every standard check, as the composition roots register them.
    pub fn doctor_deps(&self, locator: Arc<FakeLocator>) -> DoctorDeps {
        let shim_dir = self.paths.shim_dir();
        let checks: Vec<Arc<dyn DoctorCheck>> = vec![
            Arc::new(VcRuntimeCheck { vc: Arc::new(FakeVc::modern()) }),
            Arc::new(PortsCheck { probe: self.ports.clone(), services: self.services.clone() }),
            Arc::new(ShimsOnPathCheck { installer: Arc::new(FakeShimInstaller::default()), user_path: Arc::new(FakeUserPath::default()), shim_dir: shim_dir.clone() }),
            Arc::new(PathShadowingCheck { locator, shim_dir, tools: lampo_domain::shim::tools::all() }),
            Arc::new(HostsBlockCheck { hosts: self.hosts.clone() }),
            Arc::new(PackagesCheck { services: self.services.clone() }),
            Arc::new(FoldersCheck { fs: self.fs.clone() }),
            Arc::new(StaleWorktreesCheck { fs: self.fs.clone() }),
            Arc::new(AppCheck),
        ];
        DoctorDeps {
            projects: self.projects.clone(),
            worktrees: self.worktrees.clone(),
            settings: self.settings.clone(),
            installed: self.installed.clone(),
            control: self.control.clone(),
            checks,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Scaffolding
// ---------------------------------------------------------------------------------------------

/// Folders and files of a scaffold, in memory (case-insensitive paths).
#[derive(Default)]
pub struct FakeScaffoldFiles {
    pub dirs: Mutex<BTreeSet<String>>,
    pub files: Mutex<BTreeMap<String, String>>,
    pub removed: Mutex<Vec<String>>,
}
impl FakeScaffoldFiles {
    pub fn dir(&self, path: &str) {
        lock(&self.dirs).insert(key(Path::new(path)));
    }
    pub fn file(&self, path: &str, contents: &str) {
        lock(&self.files).insert(key(Path::new(path)), contents.into());
    }
    pub fn contents(&self, path: &str) -> Option<String> {
        lock(&self.files).get(&key(Path::new(path))).cloned()
    }
}
impl ScaffoldFiles for FakeScaffoldFiles {
    fn path_state(&self, path: &Path) -> lampo_domain::scaffold::PathState {
        use lampo_domain::scaffold::PathState;
        let k = key(path);
        if lock(&self.files).contains_key(&k) {
            return PathState::File;
        }
        if !lock(&self.dirs).contains(&k) {
            return PathState::Missing;
        }
        let prefix = format!("{k}\\");
        let nested = lock(&self.dirs).iter().any(|d| d.starts_with(&prefix)) || lock(&self.files).keys().any(|f| f.starts_with(&prefix));
        if nested { PathState::NonEmptyDir } else { PathState::EmptyDir }
    }
    fn is_file(&self, path: &Path) -> bool {
        lock(&self.files).contains_key(&key(path))
    }
    fn create_dir_all(&self, path: &Path) -> PortResult<()> {
        lock(&self.dirs).insert(key(path));
        Ok(())
    }
    fn write_file(&self, path: &Path, contents: &str) -> PortResult<()> {
        lock(&self.files).insert(key(path), contents.into());
        Ok(())
    }
    fn remove_dir_all(&self, path: &Path) -> PortResult<()> {
        let k = key(path);
        let prefix = format!("{k}\\");
        lock(&self.dirs).retain(|d| d != &k && !d.starts_with(&prefix));
        lock(&self.files).retain(|f, _| f != &k && !f.starts_with(&prefix));
        lock(&self.removed).push(k);
        Ok(())
    }
}

/// What a scripted command does: matched by a substring of `program args...`.
#[derive(Default, Clone)]
pub struct ScriptedCommand {
    pub matches: String,
    pub lines: Vec<String>,
    pub exit: Option<i32>,
    /// Folders / files created in the fake file system before the lines are emitted.
    pub creates_dirs: Vec<String>,
    pub creates_files: Vec<String>,
    /// Waits for the cancel flag and returns `Cancelled` (after creating and emitting).
    pub until_cancelled: bool,
}

/// Records every command; unscripted commands succeed silently.
pub struct FakeStreamingRunner {
    pub fs: Arc<FakeScaffoldFiles>,
    pub script: Mutex<Vec<ScriptedCommand>>,
    pub calls: Mutex<Vec<CommandSpec>>,
}
impl FakeStreamingRunner {
    pub fn new(fs: Arc<FakeScaffoldFiles>) -> Arc<Self> {
        Arc::new(Self { fs, script: Mutex::default(), calls: Mutex::default() })
    }
    pub fn on(&self, command: ScriptedCommand) {
        lock(&self.script).push(command);
    }
    /// `program args...` of every call.
    pub fn command_lines(&self) -> Vec<String> {
        lock(&self.calls).iter().map(|s| std::iter::once(s.program.display().to_string()).chain(s.args.iter().cloned()).collect::<Vec<_>>().join(" ")).collect()
    }
}
#[async_trait]
impl StreamingCommandRunner for FakeStreamingRunner {
    async fn run_streaming(&self, spec: CommandSpec, on_line: &(dyn for<'l> Fn(lampo_domain::service::LogStream, &'l str) + Send + Sync), cancel: &CancelFlag) -> PortResult<Option<i32>> {
        let line = std::iter::once(spec.program.display().to_string()).chain(spec.args.iter().cloned()).collect::<Vec<_>>().join(" ");
        lock(&self.calls).push(spec);
        let scripted = lock(&self.script).iter().find(|s| line.contains(&s.matches)).cloned().unwrap_or(ScriptedCommand { exit: Some(0), ..Default::default() });
        for d in &scripted.creates_dirs {
            self.fs.dir(d);
        }
        for f in &scripted.creates_files {
            self.fs.file(f, "");
        }
        for l in &scripted.lines {
            on_line(lampo_domain::service::LogStream::Stderr, l);
        }
        if scripted.until_cancelled {
            while !cancel.is_cancelled() {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
            return Err(PortError::Cancelled);
        }
        Ok(scripted.exit)
    }
}
