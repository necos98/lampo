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
use lampo_domain::mysql::auth::MysqlAuthPlugin;
use lampo_domain::mysql::credentials::MysqlPassword;
use lampo_domain::mysql::{DatabaseName, MysqlSeries};
use lampo_domain::package::{CatalogChannel, CatalogEntry, DownloadUrl, DownloadedFile, InstalledPackage, PackageId, PackageKind, PackagePayload, Sha256Digest};
use lampo_domain::paths::LampoPaths;
use lampo_domain::php::PhpMinor;
use lampo_domain::ports::*;
use lampo_domain::preset::{Preset, PresetId, builtin};
use lampo_domain::progress::{CancelFlag, Progress, ProgressStage};
use lampo_domain::project::{DisplayName, Project, ProjectEnvironment, ProjectId, ProjectRoot};
use lampo_domain::project_import::{DroppedOnImport, ImportedProject};
use lampo_domain::resolution::{GitCheckout, ResolverChain};
use lampo_domain::service::{ServiceId, ServiceKind, ServiceLaunch, ServiceState, ServiceStatus};
use lampo_domain::settings::Settings;
use lampo_domain::shim::ShimIndex;
use lampo_domain::site::WorktreeSlug;
use lampo_domain::tcp_port::Port;
use lampo_domain::vc_runtime::{InstalledVcRuntime, VcRedistributable};
use lampo_domain::version::{PackageVersion, VersionSeries};
use lampo_domain::web::{PhpIniSpec, WebServerConfig};
use lampo_domain::worktree::WorktreeEnvironment;
use lampo_domain::{PortError, PortResult};

use crate::context::CallContext;
use crate::dto::{ServiceStatusDto, SyncReportDto};
use crate::error::{AppError, AppResult, ErrorCode};
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

/// How long a held read ([`Reads::hold_next`]) waits for its release before failing the test.
const HELD_READ_LIMIT: Duration = Duration::from_secs(5);

/// How many times a fake repository was read (each listing or lookup is one read of the store),
/// and the read to hold, if a test armed one.
#[derive(Default)]
pub struct Reads(AtomicUsize, Mutex<Option<HeldRead>>);
impl Reads {
    fn count(&self) {
        self.0.fetch_add(1, Ordering::SeqCst);
        let held = lock(&self.1).take();
        if let Some(held) = held {
            held.block();
        }
    }
    pub fn get(&self) -> usize {
        self.0.load(Ordering::SeqCst)
    }
    pub fn reset(&self) {
        self.0.store(0, Ordering::SeqCst);
    }

    /// The next read blocks its thread, as a store read waiting on a file Windows holds does,
    /// until the test releases it through the returned [`ReadHold`].
    pub fn hold_next(&self) -> ReadHold {
        let (entered, on_entered) = tokio::sync::oneshot::channel();
        let (release, released) = std::sync::mpsc::channel();
        *lock(&self.1) = Some(HeldRead { entered, released });
        ReadHold { entered: on_entered, release }
    }
}

/// The store's side of a held read.
struct HeldRead {
    entered: tokio::sync::oneshot::Sender<()>,
    released: std::sync::mpsc::Receiver<()>,
}
impl HeldRead {
    /// Panics when no release comes within [`HELD_READ_LIMIT`].
    fn block(self) {
        // The test may have stopped waiting: the read is held all the same.
        let _ = self.entered.send(());
        if self.released.recv_timeout(HELD_READ_LIMIT).is_err() {
            panic!("the held read was never released: it kept the runtime worker the release runs on");
        }
    }
}

/// The test's side of a held read: `entered` fires once the read blocks, `release` ends it.
pub struct ReadHold {
    pub entered: tokio::sync::oneshot::Receiver<()>,
    pub release: std::sync::mpsc::Sender<()>,
}

/// The projects that load, plus the files that do not (`unreadable`, set by tests).
#[derive(Default)]
pub struct MemProjects(pub Mutex<BTreeMap<ProjectId, Project>>, pub Mutex<Vec<UnreadableProject>>, pub Reads);
impl ProjectRepository for MemProjects {
    fn scan(&self) -> PortResult<ProjectListing> {
        self.2.count();
        Ok(ProjectListing { projects: lock(&self.0).values().cloned().collect(), unreadable: lock(&self.1).clone() })
    }
    fn get(&self, id: &ProjectId) -> PortResult<Option<Project>> {
        self.2.count();
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
pub struct MemWorktrees(pub Mutex<Vec<WorktreeEnvironment>>, pub Reads);
impl WorktreeRepository for MemWorktrees {
    fn list_all(&self) -> PortResult<Vec<WorktreeEnvironment>> {
        self.1.count();
        Ok(lock(&self.0).clone())
    }
    fn list_for(&self, parent: &ProjectId) -> PortResult<Vec<WorktreeEnvironment>> {
        self.1.count();
        Ok(lock(&self.0).iter().filter(|w| &w.parent == parent).cloned().collect())
    }
    fn get(&self, parent: &ProjectId, slug: &WorktreeSlug) -> PortResult<Option<WorktreeEnvironment>> {
        self.1.count();
        Ok(lock(&self.0).iter().find(|w| &w.parent == parent && &w.slug == slug).cloned())
    }
    fn find_by_root(&self, root: &ProjectRoot) -> PortResult<Option<WorktreeEnvironment>> {
        self.1.count();
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

/// The stored settings; saves fail (a locked or read-only file) while the flag is set, loads
/// fail with the reason in `.3` (a file edited into invalid settings) while it is set.
#[derive(Default)]
pub struct MemSettings(pub Mutex<Settings>, pub AtomicBool, pub Reads, pub Mutex<Option<String>>);
impl MemSettings {
    pub fn fail_saves(&self, fail: bool) {
        self.1.store(fail, Ordering::SeqCst);
    }

    /// Loads fail as a `settings.toml` with invalid data does, naming the file and `reason`.
    pub fn fail_loads(&self, reason: &str) {
        *lock(&self.3) = Some(reason.into());
    }
}
impl SettingsRepository for MemSettings {
    fn load(&self) -> PortResult<Settings> {
        self.2.count();
        if let Some(reason) = lock(&self.3).clone() {
            return Err(PortError::invalid_data("settings.toml", reason));
        }
        Ok(lock(&self.0).clone())
    }
    fn save(&self, settings: &Settings) -> PortResult<()> {
        if self.1.load(Ordering::SeqCst) {
            return Err(PortError::io("writing settings.json", "the file is locked by another process"));
        }
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

/// Like the real registry: one directory and one record per slot (kind, series, PHP minor), so a
/// patch update replaces the record of the patch it updates.
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
#[async_trait]
impl InstalledPackageRegistry for MemInstalled {
    fn list(&self) -> PortResult<Vec<InstalledPackage>> {
        Ok(lock(&self.packages).clone())
    }
    fn install_dir(&self, id: &PackageId) -> PathBuf {
        PathBuf::from("C:\\lampo\\packages").join(slot_dir_name(id))
    }
    fn record(&self, package: &InstalledPackage) -> PortResult<()> {
        let mut all = lock(&self.packages);
        all.retain(|p| slot_of(&p.id) != slot_of(&package.id));
        all.push(fake_layout(package.clone()));
        Ok(())
    }
    /// `NotFound` unless exactly `id` is recorded (its slot may hold another patch by now).
    async fn remove(&self, id: &PackageId) -> PortResult<()> {
        let mut all = lock(&self.packages);
        let before = all.len();
        all.retain(|p| &p.id != id);
        if all.len() == before {
            return Err(PortError::not_found("package", id.to_string()));
        }
        lock(&self.removed).push(id.clone());
        Ok(())
    }
}

/// Folders (`add`) and files (`add_file`) that exist; nothing else does.
#[derive(Default)]
pub struct FakeFs {
    dirs: Mutex<BTreeSet<String>>,
    files: Mutex<BTreeSet<String>>,
}
impl FakeFs {
    pub fn add(&self, path: &str) {
        lock(&self.dirs).insert(key(Path::new(path)));
    }
    pub fn add_file(&self, path: &Path) {
        lock(&self.files).insert(key(path));
    }
    pub fn remove(&self, path: &str) {
        lock(&self.dirs).remove(&key(Path::new(path)));
    }
}
impl FileSystemInspector for FakeFs {
    fn is_dir(&self, path: &Path) -> bool {
        lock(&self.dirs).contains(&key(path))
    }
    fn is_file(&self, path: &Path) -> bool {
        lock(&self.files).contains(&key(path))
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

/// A file system without aliases: every folder's final path is the folder itself.
pub struct SameNamePaths;
impl PathCanonicalizer for SameNamePaths {
    fn canonicalize(&self, folder: &str) -> Option<String> {
        Some(folder.to_string())
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
    /// What the import reports as left out of the file.
    pub dropped: Mutex<DroppedOnImport>,
}
impl ProjectTransfer for FakeTransfer {
    fn export_to(&self, project: &Project, file: &Path) -> PortResult<()> {
        lock(&self.exported).push((project.id.clone(), file.to_path_buf()));
        Ok(())
    }
    fn import_from(&self, file: &Path) -> PortResult<ImportedProject> {
        let project = lock(&self.to_import).clone().ok_or_else(|| PortError::not_found("file", file.display().to_string()))?;
        Ok(ImportedProject { project, dropped: lock(&self.dropped).clone() })
    }
}

/// `.0`: installed; `.1`: how many refreshes changed something (only when installed); `.2`:
/// uninstall fails (a copy is running).
#[derive(Default)]
pub struct FakeShimInstaller(pub AtomicBool, pub AtomicUsize, pub AtomicBool);
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
    fn uninstall(&self, shim_dir: &Path) -> PortResult<()> {
        if self.2.load(Ordering::SeqCst) {
            return Err(PortError::io(format!("deleting {}", shim_dir.display()), "access denied"));
        }
        self.0.store(false, Ordering::SeqCst);
        Ok(())
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
    fn remove(&self, dir: &Path) -> PortResult<()> {
        lock(&self.0).retain(|d| key(d) != key(dir));
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
/// configuration when `reject_all` is set; fails without validating anything (a file locked by
/// an antivirus, `httpd -t` not starting) when `fail_io` is set. Records what was applied
/// (validated + swapped).
#[derive(Default)]
pub struct FakeWeb {
    pub applied: Mutex<Vec<WebServerConfig>>,
    pub attempts: AtomicUsize,
    pub reject_all: AtomicBool,
    pub fail_io: AtomicBool,
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
        if self.fail_io.load(Ordering::SeqCst) {
            return Err(PortError::io("swapping httpd.conf", "the file is locked by another process"));
        }
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
    /// Reads fail like a hosts file locked by an antivirus.
    pub unreadable: AtomicBool,
}
impl HostsFileReader for FakeHosts {
    fn read_block(&self) -> PortResult<Option<HostsBlock>> {
        if self.unreadable.load(Ordering::SeqCst) {
            return Err(PortError::Io { context: "reading the hosts file".into(), message: "access denied".into() });
        }
        Ok(lock(&self.block).clone())
    }
}
#[async_trait]
impl HostsFileWriter for FakeHosts {
    async fn write_block(&self, block: &HostsBlock, _tld: &Tld) -> PortResult<()> {
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
    /// While set, `start` stays `Starting` until a `stop` makes it give up (`Cancelled`), like
    /// a long MySQL recovery interrupted by the user or by the shutdown.
    pub hold_starts: AtomicBool,
    give_up_start: tokio::sync::Notify,
}
impl FakeRunner {
    pub fn new(kind: ServiceKind) -> Arc<Self> {
        Arc::new(Self { kind, states: Mutex::default(), calls: Mutex::default(), launches: Mutex::default(), hold_starts: AtomicBool::new(false), give_up_start: tokio::sync::Notify::new() })
    }
    /// Completes once `id` is starting (a held start reached the runner).
    pub async fn until_starting(&self, id: &ServiceId) {
        while self.state(id) != ServiceState::Starting {
            tokio::task::yield_now().await;
        }
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
        if self.hold_starts.load(Ordering::SeqCst) {
            self.set(&launch.service, ServiceState::Starting);
            self.give_up_start.notified().await;
            return Err(PortError::Cancelled);
        }
        self.set(&launch.service, ServiceState::Running);
        Ok(())
    }
    async fn stop(&self, service: &ServiceId) -> PortResult<()> {
        lock(&self.calls).push(format!("stop:{service}"));
        if self.state(service) == ServiceState::Starting {
            self.give_up_start.notify_one();
        }
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
pub struct FakePorts(pub Mutex<BTreeMap<Port, Option<String>>>, pub std::sync::atomic::AtomicU64);
impl FakePorts {
    pub fn busy(&self, port: u16, owner: &str) {
        lock(&self.0).insert(Port::of(port), Some(owner.into()));
    }
    /// Makes every probe take `ms` milliseconds (widens race windows in concurrency tests).
    pub fn slow(&self, ms: u64) {
        self.1.store(ms, Ordering::SeqCst);
    }
}
impl PortProbe for FakePorts {
    fn check(&self, port: Port) -> PortResult<PortAvailability> {
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
    /// `drop_database` fails (MySQL error) for these names, dropping nothing.
    pub fail_drop_of: Mutex<BTreeSet<String>>,
    /// Present but missing from `list`: created by another client after the listing.
    pub unlisted: Mutex<BTreeSet<String>>,
    /// While set, `import` announces itself on `import_started` and waits for `import_resumed`
    /// (to act while an import runs).
    pub hold_import: AtomicBool,
    pub import_started: tokio::sync::Notify,
    pub import_resumed: tokio::sync::Notify,
    /// Returned by the next `import`, once it reached the server.
    pub import_error: Mutex<Option<PortError>>,
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
        let unlisted = lock(&self.unlisted);
        Ok(lock(&self.dbs).iter().filter(|d| !unlisted.contains(*d)).cloned().collect())
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
        if lock(&self.fail_drop_of).contains(name.as_str()) {
            return Err(PortError::process("mysql", "ERROR 1010: error dropping database"));
        }
        lock(&self.dbs).remove(name.as_str());
        Ok(())
    }
    async fn import(&self, _e: &MysqlEndpoint, name: &DatabaseName, file: &Path, options: DumpImportOptions, progress: &dyn ProgressSink, cancel: &CancelFlag) -> PortResult<()> {
        if self.hold_import.load(Ordering::SeqCst) {
            self.import_started.notify_one();
            self.import_resumed.notified().await;
        }
        cancel.check()?;
        progress.report(Progress::new(ProgressStage::Configuring, 1, Some(1)));
        let allowed = if options.allow_database_switch { ":allow-use" } else { "" };
        let compat = options.compat.map(|c| format!(":compat={c}")).unwrap_or_default();
        lock(&self.calls).push(format!("import:{name}:{}{allowed}{compat}", file.display()));
        lock(&self.import_error).take().map_or(Ok(()), Err)
    }
    async fn export(&self, _e: &MysqlEndpoint, name: &DatabaseName, file: &Path, _p: &dyn ProgressSink, cancel: &CancelFlag) -> PortResult<()> {
        cancel.check()?;
        lock(&self.calls).push(format!("export:{name}:{}", file.display()));
        Ok(())
    }
    async fn clone_database(&self, _e: &MysqlEndpoint, source: &DatabaseName, target: &DatabaseName, progress: &dyn ProgressSink, cancel: &CancelFlag) -> PortResult<()> {
        lock(&self.calls).push(format!("clone:{source}->{target}"));
        // Like the adapter, which creates `target` before restoring into it.
        if !lock(&self.dbs).insert(target.to_string()) {
            return Err(PortError::AlreadyExists { entity: "database", id: target.to_string() });
        }
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

/// How long a `slow` fake download waits between its two chunks: long enough for a concurrent
/// task to run meanwhile.
const SLOW_DOWNLOAD_PAUSE: Duration = Duration::from_millis(50);

/// Reports two progress chunks; cancels itself between them when `cancel_midway` is set, and
/// yields for [`SLOW_DOWNLOAD_PAUSE`] between them when `slow` is set.
/// The "downloaded" file hashes to `digest` (that of every `catalog_entry` by default).
pub struct FakeDownloader {
    pub downloads: Mutex<Vec<PathBuf>>,
    /// The size limit of each download.
    pub limits: Mutex<Vec<Option<u64>>>,
    pub discarded: Mutex<Vec<PathBuf>>,
    pub cancel_midway: AtomicBool,
    pub slow: AtomicBool,
    pub digest: Mutex<Sha256Digest>,
    /// Downloads running right now, and the most that ever ran at once.
    pub running: AtomicUsize,
    pub peak_running: AtomicUsize,
}
impl Default for FakeDownloader {
    fn default() -> Self {
        Self {
            downloads: Mutex::default(),
            limits: Mutex::default(),
            discarded: Mutex::default(),
            cancel_midway: AtomicBool::default(),
            slow: AtomicBool::default(),
            digest: Mutex::new(digest(7)),
            running: AtomicUsize::default(),
            peak_running: AtomicUsize::default(),
        }
    }
}
#[async_trait]
impl PackageDownloader for FakeDownloader {
    async fn download(&self, _url: &DownloadUrl, destination: &Path, max_bytes: Option<u64>, progress: &dyn ProgressSink, cancel: &CancelFlag) -> PortResult<DownloadedFile> {
        lock(&self.downloads).push(destination.to_path_buf());
        lock(&self.limits).push(max_bytes);
        progress.report(Progress::new(ProgressStage::Downloading, 500, Some(1000)));
        if self.cancel_midway.load(Ordering::SeqCst) {
            cancel.cancel();
        }
        if self.slow.load(Ordering::SeqCst) {
            let running = self.running.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak_running.fetch_max(running, Ordering::SeqCst);
            tokio::time::sleep(SLOW_DOWNLOAD_PAUSE).await;
            self.running.fetch_sub(1, Ordering::SeqCst);
        }
        cancel.check()?;
        progress.report(Progress::new(ProgressStage::Downloading, 1000, Some(1000)));
        Ok(DownloadedFile { bytes: 1000, sha256: *lock(&self.digest) })
    }
    async fn discard(&self, destination: &Path) -> PortResult<()> {
        lock(&self.discarded).push(destination.to_path_buf());
        Ok(())
    }
}

/// Records extractions (archive, destination) and single-file placements (file, destination,
/// name), and what became of each replacement. Every replacement claims a previous version kept
/// aside at `<destination>.previous`.
#[derive(Default)]
pub struct FakeExtractor {
    pub extracted: Mutex<Vec<(PathBuf, PathBuf)>>,
    pub placed: Mutex<Vec<(PathBuf, PathBuf, String)>>,
    pub committed: Mutex<Vec<SlotReplacement>>,
    pub rolled_back: Mutex<Vec<SlotReplacement>>,
}
impl FakeExtractor {
    fn replacement(destination: &Path) -> SlotReplacement {
        SlotReplacement { destination: destination.to_path_buf(), previous: Some(destination.with_extension("previous")) }
    }
}
#[async_trait]
impl ArchiveExtractor for FakeExtractor {
    async fn extract_zip(&self, archive: &Path, destination: &Path, _strip: Option<&str>, progress: &dyn ProgressSink, cancel: &CancelFlag) -> PortResult<SlotReplacement> {
        cancel.check()?;
        progress.report(Progress::new(ProgressStage::Extracting, 1, Some(1)));
        lock(&self.extracted).push((archive.to_path_buf(), destination.to_path_buf()));
        Ok(Self::replacement(destination))
    }
    async fn place_file(&self, file: &Path, destination: &Path, file_name: &str, cancel: &CancelFlag) -> PortResult<SlotReplacement> {
        cancel.check()?;
        lock(&self.placed).push((file.to_path_buf(), destination.to_path_buf(), file_name.to_string()));
        Ok(Self::replacement(destination))
    }
    async fn commit(&self, replacement: SlotReplacement) {
        lock(&self.committed).push(replacement);
    }
    async fn roll_back(&self, replacement: SlotReplacement) -> PortResult<()> {
        lock(&self.rolled_back).push(replacement);
        Ok(())
    }
}

/// A package slot: kind, series, PHP minor.
type SlotKey = (PackageKind, VersionSeries, Option<PhpMinor>);

fn slot_of(id: &PackageId) -> SlotKey {
    (id.kind.clone(), id.series(), id.php)
}

/// The directory of the slot of `id`, named as the real registry names it (`php-8.3`,
/// `xdebug-3.3-php8.3`).
fn slot_dir_name(id: &PackageId) -> String {
    match id.php {
        Some(php) => format!("{}-{}-php{php}", id.kind, id.series()),
        None => format!("{}-{}", id.kind, id.series()),
    }
}

/// One async mutex per slot, like the real lock within a process.
#[derive(Default)]
pub struct MemSlotLock(Mutex<BTreeMap<SlotKey, Arc<tokio::sync::Mutex<()>>>>);
#[async_trait]
impl PackageSlotLock for MemSlotLock {
    async fn acquire(&self, id: &PackageId, cancel: &CancelFlag) -> PortResult<SlotGuard> {
        cancel.check()?;
        let slot = lock(&self.0).entry(slot_of(id)).or_default().clone();
        Ok(SlotGuard::new(slot.lock_owned().await))
    }
}

/// Records each call; fails with an IO error when `fails` is set.
pub struct FakeHook {
    pub kind: PackageKind,
    pub calls: Mutex<Vec<PackageId>>,
    pub fails: AtomicBool,
}
impl FakeHook {
    pub fn new(kind: PackageKind) -> Self {
        Self { kind, calls: Mutex::default(), fails: AtomicBool::default() }
    }
}
#[async_trait]
impl PackageInstallHook for FakeHook {
    fn kind(&self) -> PackageKind {
        self.kind.clone()
    }
    async fn after_install(&self, package: &InstalledPackage) -> PortResult<()> {
        lock(&self.calls).push(package.id.clone());
        if self.fails.load(Ordering::SeqCst) {
            return Err(PortError::io("seeding php.ini", "access denied"));
        }
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

    async fn sync_without(&self, _retiring: &PackageId, ctx: &CallContext) -> AppResult<SyncReportDto> {
        self.sync(ctx).await
    }
}

// ---------------------------------------------------------------------------------------------
// World
// ---------------------------------------------------------------------------------------------

pub fn php(s: &str) -> PhpMinor {
    PhpMinor::parse(s).unwrap()
}

/// Appends `address` to `project`'s addresses, through the aggregate's validation.
pub fn add_address(project: &mut Project, address: SiteAddress) {
    let mut list = project.addresses().to_vec();
    list.push(address);
    project.set_addresses(list).unwrap();
}

/// What the fake scanner finds in every PHP build's `ext` dir.
pub const FAKE_BUNDLED_EXTENSIONS: &[&str] = &["gd", "intl", "mbstring", "opcache", "pdo_mysql", "zip"];

/// What the fake scanner finds compiled into every PHP build.
pub const FAKE_BUILTIN_EXTENSIONS: &[&str] = &["bcmath", "ctype", "json"];

/// A `PhpExtensionScanner` returning fixed lists, each name as the build's PHP minor names it
/// (GD is `gd2` before PHP 8.0; the minor comes from the fake install dir `php-<version>`).
/// Counts how often the compiled-in modules are asked for (a real build runs `php -m`).
pub struct FakeExtensionScanner {
    pub bundled: &'static [&'static str],
    pub builtin: &'static [&'static str],
    pub builtin_calls: AtomicUsize,
}

impl FakeExtensionScanner {
    pub fn new(bundled: &'static [&'static str]) -> Self {
        Self { bundled, builtin: FAKE_BUILTIN_EXTENSIONS, builtin_calls: AtomicUsize::new(0) }
    }

    fn named_for(php_dir: &Path, list: &[&str]) -> Vec<lampo_domain::php::ExtensionName> {
        let version = php_dir.file_name().and_then(|n| n.to_str()).and_then(|n| n.strip_prefix("php-")).and_then(|v| PackageVersion::parse(v).ok());
        let php = version.and_then(|v| PhpMinor::try_from(v.series()).ok());
        let mut names: Vec<_> = list.iter().map(|n| lampo_domain::php::ExtensionName::new(n).unwrap()).map(|n| php.map_or(n.clone(), |php| n.for_php(php))).collect();
        names.sort();
        names
    }
}

impl PhpExtensionScanner for FakeExtensionScanner {
    fn bundled_extensions(&self, php_dir: &Path) -> PortResult<Vec<lampo_domain::php::ExtensionName>> {
        Ok(Self::named_for(php_dir, self.bundled))
    }

    fn builtin_extensions(&self, php_dir: &Path) -> PortResult<Vec<lampo_domain::php::ExtensionName>> {
        self.builtin_calls.fetch_add(1, Ordering::SeqCst);
        Ok(Self::named_for(php_dir, self.builtin))
    }
}

// ---------------------------------------------------------------------------------------------
// MySQL accounts
// ---------------------------------------------------------------------------------------------

/// Stored root and project passwords, in memory.
#[derive(Default)]
pub struct MemMysqlPasswords {
    pub root: Mutex<BTreeMap<ServiceId, MysqlPassword>>,
    pub projects: Mutex<BTreeMap<ProjectId, MysqlPassword>>,
}
impl MysqlRootPasswordReader for MemMysqlPasswords {
    fn root_password(&self, instance: &ServiceId) -> PortResult<Option<MysqlPassword>> {
        Ok(lock(&self.root).get(instance).cloned())
    }
}
impl MysqlRootPasswordWriter for MemMysqlPasswords {
    fn save_root_password(&self, instance: &ServiceId, password: &MysqlPassword) -> PortResult<()> {
        lock(&self.root).insert(instance.clone(), password.clone());
        Ok(())
    }
}
impl MysqlProjectPasswordReader for MemMysqlPasswords {
    fn project_password(&self, project: &ProjectId) -> PortResult<Option<MysqlPassword>> {
        Ok(lock(&self.projects).get(project).cloned())
    }
}
impl MysqlProjectPasswordWriter for MemMysqlPasswords {
    fn save_project_password_if_absent(&self, project: &ProjectId, password: &MysqlPassword) -> PortResult<MysqlPassword> {
        Ok(lock(&self.projects).entry(project.clone()).or_insert_with(|| password.clone()).clone())
    }
}

/// `generated-1`, `generated-2`, …
#[derive(Default)]
pub struct CountingPasswords(pub AtomicUsize);
impl MysqlPasswordGenerator for CountingPasswords {
    fn generate(&self) -> PortResult<MysqlPassword> {
        let n = self.0.fetch_add(1, Ordering::SeqCst) + 1;
        Ok(MysqlPassword::new(format!("generated-{n}")).unwrap())
    }
}

/// Records `secure-root:<port>:<password>`, `native-root:<port>` and
/// `accounts:<port>:<user>[@native]=<db>+<db>;…` calls (`@native`: on `mysql_native_password`);
/// root starts without a password and keeps the one it is given.
#[derive(Default)]
pub struct FakeMysqlAccounts {
    pub calls: Mutex<Vec<String>>,
    /// Root's password on the server, per port.
    pub root: Mutex<BTreeMap<u16, MysqlPassword>>,
    /// The next call fails (MySQL error) without changing anything.
    pub fail_next: AtomicBool,
}
impl FakeMysqlAccounts {
    pub fn calls(&self) -> Vec<String> {
        lock(&self.calls).clone()
    }
    fn fail(&self) -> PortResult<()> {
        if self.fail_next.swap(false, Ordering::SeqCst) { Err(PortError::process("mysql", "ERROR 2013: Lost connection")) } else { Ok(()) }
    }
}
#[async_trait]
impl MysqlAccountAdmin for FakeMysqlAccounts {
    async fn secure_root(&self, endpoint: &MysqlEndpoint, password: &MysqlPassword) -> PortResult<RootPasswordChange> {
        self.fail()?;
        lock(&self.calls).push(format!("secure-root:{}:{}", endpoint.port, password.expose()));
        let mut root = lock(&self.root);
        match root.get(&endpoint.port.get()) {
            Some(current) if current == password => Ok(RootPasswordChange::AlreadySet),
            Some(_) => Err(PortError::process("mysql", "root accepts neither the stored password nor an empty one")),
            None => {
                root.insert(endpoint.port.get(), password.clone());
                Ok(RootPasswordChange::Applied)
            }
        }
    }
    async fn sync_project_accounts(&self, endpoint: &MysqlEndpoint, accounts: &[ProjectAccountGrant]) -> PortResult<()> {
        self.fail()?;
        let listed: Vec<String> = accounts
            .iter()
            .map(|a| {
                let native = if a.auth == Some(MysqlAuthPlugin::Native) { "@native" } else { "" };
                format!("{}{native}={}", a.user, a.databases.iter().map(DatabaseName::as_str).collect::<Vec<_>>().join("+"))
            })
            .collect();
        lock(&self.calls).push(format!("accounts:{}:{}", endpoint.port, listed.join(";")));
        Ok(())
    }
    async fn use_native_password_for_root(&self, endpoint: &MysqlEndpoint) -> PortResult<bool> {
        self.fail()?;
        lock(&self.calls).push(format!("native-root:{}", endpoint.port));
        Ok(true)
    }
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
    /// The package slot locks installs and uninstalls share (one per process, as in the
    /// composition roots).
    pub package_slots: Arc<MemSlotLock>,
    pub web: Arc<FakeWeb>,
    pub ini: Arc<FakeIni>,
    pub hosts: Arc<FakeHosts>,
    pub apache: Arc<FakeRunner>,
    pub mysql: Arc<FakeRunner>,
    pub redis: Arc<FakeRunner>,
    pub mailpit: Arc<FakeRunner>,
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
    /// What every installed PHP build offers ([`FAKE_BUNDLED_EXTENSIONS`], [`FAKE_BUILTIN_EXTENSIONS`]).
    pub scanner: Arc<FakeExtensionScanner>,
    /// The MySQL accounts on the (fake) servers.
    pub mysql_accounts: Arc<FakeMysqlAccounts>,
    /// The stored MySQL passwords.
    pub secrets: Arc<MemMysqlPasswords>,
    /// The passwords over `secrets` (generated as `generated-<n>`).
    pub passwords: Arc<MysqlPasswords>,
    /// The shutdown gate `env` checks, closed by [`Self::shutdown`].
    pub gate: Arc<ShutdownGate>,
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
        let mailpit = FakeRunner::new(ServiceKind::MAILPIT);
        let mut kinds = lampo_domain::service::builtin::all();
        kinds.extend(extra_kinds);
        let services = Arc::new(ServiceCatalog::new(kinds));
        let ports = Arc::new(FakePorts::default());
        let events = Arc::new(RecordingEvents::default());
        let scanner = Arc::new(FakeExtensionScanner::new(FAKE_BUNDLED_EXTENSIONS));
        let mysql_accounts = Arc::new(FakeMysqlAccounts::default());
        let secrets = Arc::new(MemMysqlPasswords::default());
        let passwords = Arc::new(MysqlPasswords::with_store(secrets.clone(), Arc::new(CountingPasswords::default())));
        let gate = Arc::new(ShutdownGate::default());
        let mut runners: Vec<Arc<dyn ServiceRunner>> = vec![apache.clone(), mysql.clone(), redis.clone(), mailpit.clone()];
        runners.extend(extra_runners);
        let env = Arc::new(ApplyEnvironment::new(ApplyEnvironmentDeps {
            projects: projects.clone(),
            worktrees: worktrees.clone(),
            settings: settings.clone(),
            installed: installed.clone(),
            web_config: web.clone(),
            php_ini: ini.clone(),
            extensions: Arc::new(PhpExtensionsQuery::new(installed.clone(), scanner.clone())),
            hosts_reader: hosts.clone(),
            hosts_writer: hosts.clone(),
            shim_index: shim_index.clone(),
            services: services.clone(),
            runners: runners.clone(),
            port_probe: ports.clone(),
            paths: paths.clone(),
            events: events.clone(),
            mysql_accounts: Arc::new(MysqlAccounts::new(mysql_accounts.clone(), passwords.clone())),
            gate: gate.clone(),
        }));
        let service_control = Arc::new(ServiceControl::new(ServiceControlDeps { runners, catalog: services.clone(), settings: settings.clone(), lifecycle: env.clone(), env: env.clone() }));
        let allocator = Arc::new(LoopbackPortAllocator::new(projects.clone(), worktrees.clone(), settings.clone(), ports.clone()));
        Self {
            paths,
            projects,
            worktrees,
            settings,
            presets: Arc::new(MemPresets::default()),
            shim_index,
            installed,
            package_slots: Arc::new(MemSlotLock::default()),
            web,
            ini,
            hosts,
            apache,
            mysql,
            redis,
            mailpit,
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
            scanner,
            mysql_accounts,
            secrets,
            passwords,
            gate,
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

    /// Registers a project file that does not load, naming `loopback_ports`.
    pub fn unreadable_project(&self, file: &str, loopback_ports: &[Port]) {
        let reason = format!("invalid data in {file}: a project needs at least one address");
        lock(&self.projects.1).push(UnreadableProject { file: PathBuf::from(file), reason, loopback_ports: loopback_ports.iter().copied().collect() });
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
        Arc::new(WorktreeDatabases::new(self.projects.clone(), self.settings.clone(), self.installed.clone(), self.db.clone(), self.service_control.clone(), self.services.clone()))
    }

    pub fn worktree_queries(&self) -> Arc<WorktreeQueries> {
        Arc::new(WorktreeQueries::new(self.worktrees.clone(), self.projects.clone(), self.settings.clone(), self.fs.clone(), self.passwords.clone()))
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
        WorktreeDown::new(self.worktrees.clone(), self.allocator.clone(), self.worktree_databases(), self.env.clone(), self.events.clone())
    }

    pub fn prune_worktrees(&self) -> PruneWorktrees {
        PruneWorktrees::new(self.projects.clone(), self.worktrees.clone(), self.allocator.clone(), self.fs.clone(), self.worktree_databases(), self.env.clone(), self.events.clone())
    }

    pub fn database_service(&self) -> Arc<DatabaseService> {
        Arc::new(DatabaseService::new(
            self.db.clone(),
            self.projects.clone(),
            self.worktrees.clone(),
            self.settings.clone(),
            self.installed.clone(),
            self.service_control.clone(),
            self.services.clone(),
        ))
    }

    pub fn presets(&self) -> Arc<PresetQueries> {
        Arc::new(PresetQueries::new(builtin::all(), self.presets.clone(), self.settings.clone(), self.installed.clone()))
    }

    pub fn project_queries(&self) -> Arc<ProjectQueries> {
        Arc::new(ProjectQueries::new(self.projects.clone(), self.worktrees.clone(), self.settings.clone(), self.fs.clone(), self.passwords.clone()))
    }

    /// The rules of every project save; every installed PHP ships [`FAKE_BUNDLED_EXTENSIONS`].
    pub fn project_policy(&self) -> Arc<ProjectEnvironmentPolicy> {
        Arc::new(ProjectEnvironmentPolicy::new(self.services.clone(), self.installed.clone(), Arc::new(PhpExtensionsQuery::new(self.installed.clone(), self.scanner.clone()))))
    }

    /// The writer every project-mutating use case takes, syncing through `env`.
    pub fn project_writer_with(&self, env: Arc<dyn EnvironmentSync>) -> ProjectWriter {
        ProjectWriter::new(ProjectWriterDeps {
            projects: self.projects.clone(),
            allocator: self.allocator.clone(),
            env,
            events: self.events.clone(),
            queries: self.project_queries(),
            policy: self.project_policy(),
        })
    }

    pub fn project_writer(&self) -> ProjectWriter {
        self.project_writer_with(self.env.clone())
    }

    /// `delete_project`, syncing through `env`.
    pub fn delete_project_with(&self, env: Arc<dyn EnvironmentSync>) -> DeleteProject {
        DeleteProject::new(self.project_writer_with(env), self.worktrees.clone(), self.events.clone())
    }

    pub fn import_project(&self, transfer: Arc<FakeTransfer>) -> ImportProject {
        ImportProject::new(self.project_writer(), transfer, self.fs.clone())
    }

    pub fn relink_project(&self) -> RelinkProject {
        RelinkProject::new(self.project_writer(), self.fs.clone())
    }

    /// `lampo project set`, syncing through `env`.
    pub fn patch_project_with(&self, env: Arc<dyn EnvironmentSync>) -> PatchProjectEnvironment {
        PatchProjectEnvironment::new(self.project_writer_with(env), self.installed.clone())
    }

    pub fn patch_project(&self) -> PatchProjectEnvironment {
        self.patch_project_with(self.env.clone())
    }

    /// The guarded settings save every settings-mutating use case takes, sharing this world's
    /// allocator.
    pub fn settings_writer(&self) -> SettingsWriter {
        SettingsWriter {
            repo: self.settings.clone(),
            allocator: self.allocator.clone(),
            port_probe: self.ports.clone(),
            installed: self.installed.clone(),
            events: self.events.clone(),
            services: self.services.clone(),
        }
    }

    /// `lampo settings set`, syncing through `env`.
    pub fn patch_settings(&self) -> PatchSettings {
        PatchSettings::new(self.settings_writer(), self.env.clone())
    }

    /// The GUI settings page, syncing through `env`.
    pub fn settings_service(&self) -> SettingsService {
        SettingsService::new(self.settings_writer(), self.env.clone())
    }

    pub fn resolver(&self) -> Arc<SiteResolver> {
        Arc::new(SiteResolver::new(self.shim_index.clone(), ResolverChain::standard(self.git.clone(), Arc::new(SameNamePaths))))
    }

    pub fn runners(&self) -> Vec<Arc<dyn ServiceRunner>> {
        vec![self.apache.clone(), self.mysql.clone(), self.redis.clone(), self.mailpit.clone()]
    }

    /// The app exit of this world, closing the gate its `env` checks.
    pub fn shutdown(&self) -> Shutdown {
        Shutdown::new(self.runners(), self.services.clone(), self.settings.clone(), self.gate.clone())
    }

    pub fn all_calls(&self) -> Vec<String> {
        [self.apache.calls(), self.mysql.calls(), self.redis.calls(), self.mailpit.calls()].concat()
    }

    /// Reads of the projects, worktree environments and settings: (projects, worktrees, settings).
    pub fn store_reads(&self) -> (usize, usize, usize) {
        (self.projects.2.get(), self.worktrees.1.get(), self.settings.2.get())
    }

    pub fn reset_store_reads(&self) {
        self.projects.2.reset();
        self.worktrees.1.reset();
        self.settings.2.reset();
    }

    pub fn clear_calls(&self) {
        self.apache.clear();
        self.mysql.clear();
        self.redis.clear();
        self.mailpit.clear();
    }
}

impl World {
    /// `UninstallPackage` syncing through this world's `ApplyEnvironment`.
    pub fn uninstall_package(&self) -> UninstallPackage {
        self.uninstall_package_with(self.installed.clone(), self.env.clone())
    }

    /// [`Self::uninstall_package`] removing through `installed` and syncing through `env`
    /// (wrappers of this world's registry and environment).
    pub fn uninstall_package_with(&self, installed: Arc<dyn InstalledPackageRegistry>, env: Arc<dyn EnvironmentSync>) -> UninstallPackage {
        UninstallPackage::new(UninstallPackageDeps {
            installed,
            slots: self.package_slots.clone(),
            projects: self.projects.clone(),
            worktrees: self.worktrees.clone(),
            settings: self.settings.clone(),
            events: self.events.clone(),
            env,
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
        "apache" => vec![(EntryRole::Server, dir.join("bin").join("httpd.exe"))],
        "mod-fcgid" => vec![(EntryRole::ApacheModule, dir.join("mod_fcgid.so"))],
        "mysql" => vec![(EntryRole::ClientDir, dir.join("bin"))],
        "redis" => vec![(EntryRole::Cli, dir.join("redis-cli.exe")), (EntryRole::ClientDir, dir.clone())],
        "mailpit" => vec![(EntryRole::Cli, dir.join("mailpit.exe")), (EntryRole::ClientDir, dir.clone())],
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
        let checks = standard_checks(StandardChecksDeps {
            vc: Arc::new(FakeVc::modern()),
            port_probe: self.ports.clone(),
            services: self.services.clone(),
            shim_installer: Arc::new(FakeShimInstaller::default()),
            user_path: Arc::new(FakeUserPath::default()),
            locator,
            shim_dir: self.paths.shim_dir(),
            shim_tools: lampo_domain::shim::tools::all(),
            hosts: self.hosts.clone(),
            fs: self.fs.clone(),
        });
        DoctorDeps {
            projects: self.projects.clone(),
            worktrees: self.worktrees.clone(),
            settings: self.settings.clone(),
            installed: self.installed.clone(),
            control: self.control.clone(),
            service_states: Arc::new(ServiceQueries::new(self.runners(), self.services.clone(), self.projects.clone(), self.worktrees.clone(), self.settings.clone())),
            checks,
        }
    }
}

/// Service states nobody can read (the CLI with the app gone): every call fails with `AppUnavailable`.
pub struct UnreadableServiceStates;

#[async_trait]
impl ServiceStatusSource for UnreadableServiceStates {
    async fn statuses(&self) -> AppResult<Vec<ServiceStatusDto>> {
        Err(AppError::new(ErrorCode::AppUnavailable, "the Lampo app is not running"))
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
