//! `lampo doctor` / GUI diagnostics. Read-only, works without the app.
//!
//! [`Doctor`] loads the configuration once ([`DoctorFacts`]) and runs every registered
//! [`DoctorCheck`] in order. Each check is one small type with only the ports it needs;
//! [`standard_checks`] is the one list both composition roots register (open-closed: a new
//! check is a new type added there).

mod config_files_check;
mod mysql_auth_check;
mod services_check;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use lampo_domain::address::SiteAddress;
use lampo_domain::package::InstalledPackage;
use lampo_domain::ports::{
    ControlChannelClient, ExecutableLocator, FileSystemInspector, HostsFileReader, InstalledPackageRegistry, PortAvailability, PortProbe, ProjectListing, ProjectRepository,
    SettingsRepository, ShimInstaller, UserPathEditor, VcRuntimeDetector, WorktreeRepository,
};
use lampo_domain::settings::Settings;
use lampo_domain::shim::ShimTool;
use lampo_domain::tcp_port::Port;
use lampo_domain::vc_runtime::VcRedistributable;
use lampo_domain::worktree::WorktreeEnvironment;

use super::hosts_sync::hosts_block_in_sync;
use super::requirements::{missing, requirements};
use super::service_catalog::ServiceCatalog;
use super::services::ServiceStatusSource;
use super::shims::shadowed_tools;
use super::support::all_sites;
use crate::dto::{CheckStatus, DoctorCheckDto, DoctorReportDto, PortConflictDto, ServiceStatusDto, VcRuntimeIssueDto};
use crate::error::AppResult;

pub use config_files_check::{ProjectFilesCheck, SettingsFileCheck};
pub use mysql_auth_check::MysqlAuthCheck;
pub use services_check::ServicesCheck;

/// What every check may look at, loaded once per run.
pub struct DoctorFacts {
    /// The projects that load, and the files that do not.
    pub project_files: ProjectListing,
    pub worktrees: Vec<WorktreeEnvironment>,
    /// The defaults when `settings.toml` does not load ([`Self::settings_problem`]), so the
    /// other checks still run.
    pub settings: Settings,
    /// Why `settings.toml` does not load (names the file).
    pub settings_problem: Option<String>,
    pub installed: Vec<InstalledPackage>,
    /// The app answers on the control pipe (its services then hold their own ports).
    pub app_running: bool,
    /// Live service states; `Err` when they cannot be read (in the CLI: the app is not running).
    pub services: AppResult<Vec<ServiceStatusDto>>,
}

/// One diagnostic. Returns its line of the report and may fill its own detail field of
/// `report` (`port_conflicts`, `stale_worktrees`, …). Must not fail: problems are findings.
pub trait DoctorCheck: Send + Sync {
    fn run(&self, facts: &DoctorFacts, report: &mut DoctorReportDto) -> DoctorCheckDto;
}

fn check(id: &str, status: CheckStatus, message: impl Into<String>, fix: Option<String>) -> DoctorCheckDto {
    DoctorCheckDto { id: id.into(), status, message: message.into(), fix }
}

pub struct DoctorDeps {
    pub projects: Arc<dyn ProjectRepository>,
    pub worktrees: Arc<dyn WorktreeRepository>,
    pub settings: Arc<dyn SettingsRepository>,
    pub installed: Arc<dyn InstalledPackageRegistry>,
    pub control: Arc<dyn ControlChannelClient>,
    /// The source of `lampo status`: the live services in the app, the app over the control
    /// pipe in the CLI.
    pub service_states: Arc<dyn ServiceStatusSource>,
    /// Run in this order.
    pub checks: Vec<Arc<dyn DoctorCheck>>,
}

pub struct Doctor {
    deps: DoctorDeps,
}

impl Doctor {
    pub fn new(deps: DoctorDeps) -> Self {
        Self { deps }
    }

    /// A configuration file that does not load is a finding (`settings`, `project-files`), not a
    /// failure of the run.
    pub async fn run(&self) -> AppResult<DoctorReportDto> {
        let d = &self.deps;
        let (settings, settings_problem) = match d.settings.load() {
            Ok(settings) => (settings, None),
            Err(e) => (Settings::default(), Some(e.to_string())),
        };
        let facts = DoctorFacts {
            project_files: d.projects.scan()?,
            worktrees: d.worktrees.list_all()?,
            settings,
            settings_problem,
            installed: d.installed.list()?,
            app_running: d.control.is_server_running(),
            services: d.service_states.statuses().await,
        };
        let mut report = DoctorReportDto::default();
        for c in &d.checks {
            let line = c.run(&facts, &mut report);
            report.checks.push(line);
        }
        report.ok = report.checks.iter().all(|c| c.status != CheckStatus::Error);
        Ok(report)
    }
}

/// The ports the [`standard_checks`] need, supplied by each composition root.
pub struct StandardChecksDeps {
    pub vc: Arc<dyn VcRuntimeDetector>,
    pub port_probe: Arc<dyn PortProbe>,
    pub services: Arc<ServiceCatalog>,
    pub shim_installer: Arc<dyn ShimInstaller>,
    pub user_path: Arc<dyn UserPathEditor>,
    pub locator: Arc<dyn ExecutableLocator>,
    pub shim_dir: PathBuf,
    pub shim_tools: Vec<Arc<dyn ShimTool>>,
    pub hosts: Arc<dyn HostsFileReader>,
    pub fs: Arc<dyn FileSystemInspector>,
}

/// The checks of `lampo doctor` and of the GUI diagnostics, in report order. The single list
/// both composition roots register, so CLI and GUI always run the same diagnostics.
pub fn standard_checks(deps: StandardChecksDeps) -> Vec<Arc<dyn DoctorCheck>> {
    vec![
        Arc::new(SettingsFileCheck),
        Arc::new(ProjectFilesCheck),
        Arc::new(VcRuntimeCheck { vc: deps.vc }),
        Arc::new(PortsCheck { probe: deps.port_probe, services: deps.services.clone() }),
        Arc::new(ShimsOnPathCheck { installer: deps.shim_installer, user_path: deps.user_path, shim_dir: deps.shim_dir.clone() }),
        Arc::new(PathShadowingCheck { locator: deps.locator, shim_dir: deps.shim_dir, tools: deps.shim_tools }),
        Arc::new(HostsBlockCheck { hosts: deps.hosts }),
        Arc::new(PackagesCheck { services: deps.services }),
        Arc::new(ServicesCheck),
        Arc::new(MysqlAuthCheck),
        Arc::new(FoldersCheck { fs: deps.fs.clone() }),
        Arc::new(StaleWorktreesCheck { fs: deps.fs }),
        Arc::new(AppCheck),
    ]
}

/// `vc-runtime`: Visual C++ runtimes needed by installed builds (`required_vc_runtime`).
pub struct VcRuntimeCheck {
    pub vc: Arc<dyn VcRuntimeDetector>,
}

impl DoctorCheck for VcRuntimeCheck {
    fn run(&self, facts: &DoctorFacts, report: &mut DoctorReportDto) -> DoctorCheckDto {
        let runtimes = match self.vc.installed() {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("cannot read installed VC++ runtimes: {e}");
                return check("vc-runtime", CheckStatus::Warning, format!("the installed Visual C++ runtimes cannot be read: {e}"), None);
            }
        };
        let mut by_redist: BTreeMap<VcRedistributable, Vec<String>> = BTreeMap::new();
        for p in &facts.installed {
            let Some(runtime) = p.required_vc_runtime() else { continue };
            if !runtime.is_satisfied_by(&runtimes) {
                by_redist.entry(runtime.redistributable()).or_default().push(p.id.key());
            }
        }
        report.missing_vc_runtimes =
            by_redist.into_iter().map(|(r, required_by)| VcRuntimeIssueDto { redistributable: r.to_string(), download_url: r.download_url().to_string(), required_by }).collect();
        let issues = &report.missing_vc_runtimes;
        match issues.is_empty() {
            true => check("vc-runtime", CheckStatus::Ok, "the Visual C++ runtimes of the installed packages are present", None),
            false => check(
                "vc-runtime",
                CheckStatus::Error,
                format!("missing Visual C++ runtime: {}", issues.iter().map(|i| i.redistributable.as_str()).collect::<Vec<_>>().join(", ")),
                Some(issues.iter().map(|i| i.download_url.as_str()).collect::<Vec<_>>().join(" ")),
            ),
        }
    }
}

/// `ports`: ports the planned services and served sites need, busy on the OS. With the app
/// running, they are presumed held by Lampo itself.
pub struct PortsCheck {
    pub probe: Arc<dyn PortProbe>,
    pub services: Arc<ServiceCatalog>,
}

impl DoctorCheck for PortsCheck {
    fn run(&self, facts: &DoctorFacts, report: &mut DoctorReportDto) -> DoctorCheckDto {
        if !facts.app_running {
            let mut wanted: BTreeMap<Port, String> = BTreeMap::new();
            for (id, port) in self.services.planned_ports(&facts.project_files.projects, &facts.worktrees, &facts.settings) {
                wanted.entry(port).or_insert(id);
            }
            for site in all_sites(&facts.project_files.projects, &facts.worktrees).iter().filter(|s| s.served()) {
                for port in site.addresses().iter().filter_map(SiteAddress::loopback) {
                    wanted.entry(port).or_insert_with(|| site.id.to_string());
                }
            }
            report.port_conflicts = wanted
                .into_iter()
                .filter_map(|(port, wanted_by)| match self.probe.check(port) {
                    Ok(PortAvailability::InUse { owner }) => Some(PortConflictDto { port: port.get(), wanted_by, owner }),
                    Ok(PortAvailability::Free) => None,
                    Err(e) => {
                        tracing::warn!("cannot probe port {port}: {e}");
                        None
                    }
                })
                .collect();
        }
        let conflicts = &report.port_conflicts;
        match conflicts.is_empty() {
            true => check("ports", CheckStatus::Ok, "the ports Lampo needs are free", None),
            false => check(
                "ports",
                CheckStatus::Error,
                conflicts
                    .iter()
                    .map(|c| format!("port {} (wanted by {}) is in use{}", c.port, c.wanted_by, c.owner.as_ref().map(|o| format!(" by {o}")).unwrap_or_default()))
                    .collect::<Vec<_>>()
                    .join("; "),
                Some("stop the program holding the port or change the port in the settings".into()),
            ),
        }
    }
}

/// `shims-on-path`: the shims are installed and their dir is on the user PATH.
pub struct ShimsOnPathCheck {
    pub installer: Arc<dyn ShimInstaller>,
    pub user_path: Arc<dyn UserPathEditor>,
    pub shim_dir: PathBuf,
}

impl DoctorCheck for ShimsOnPathCheck {
    fn run(&self, _facts: &DoctorFacts, _report: &mut DoctorReportDto) -> DoctorCheckDto {
        let installed = self.installer.is_installed(&self.shim_dir).unwrap_or(false);
        let on_path = self.user_path.contains(&self.shim_dir).unwrap_or(false);
        match (installed, on_path) {
            (true, true) => check("shims-on-path", CheckStatus::Ok, format!("the shims in {} are on PATH", self.shim_dir.display()), None),
            _ => check(
                "shims-on-path",
                CheckStatus::Warning,
                format!("the shims are {}installed and {}on the user PATH", if installed { "" } else { "not " }, if on_path { "" } else { "not " }),
                Some("reinstall the shims from the Lampo settings, then open a new terminal".into()),
            ),
        }
    }
}

/// `path-shadowing`: another `php.exe`/`composer`/`mysql` earlier on PATH than the shims.
pub struct PathShadowingCheck {
    pub locator: Arc<dyn ExecutableLocator>,
    pub shim_dir: PathBuf,
    pub tools: Vec<Arc<dyn ShimTool>>,
}

impl DoctorCheck for PathShadowingCheck {
    fn run(&self, _facts: &DoctorFacts, report: &mut DoctorReportDto) -> DoctorCheckDto {
        report.shadowed_tools = shadowed_tools(self.locator.as_ref(), &self.shim_dir, &self.tools);
        let shadowed = &report.shadowed_tools;
        match shadowed.is_empty() {
            true => check("path-shadowing", CheckStatus::Ok, "no other executable shadows the Lampo shims", None),
            false => check(
                "path-shadowing",
                CheckStatus::Warning,
                shadowed.iter().map(|s| format!("`{}` resolves to {}", s.tool, s.shadowed_by)).collect::<Vec<_>>().join("; "),
                Some(format!("move {} before the other directories in PATH, or remove the other installation", self.shim_dir.display())),
            ),
        }
    }
}

/// `hosts-block`: the Lampo block of the hosts file lists every project hostname.
pub struct HostsBlockCheck {
    pub hosts: Arc<dyn HostsFileReader>,
}

impl DoctorCheck for HostsBlockCheck {
    fn run(&self, facts: &DoctorFacts, report: &mut DoctorReportDto) -> DoctorCheckDto {
        report.hosts_file_in_sync = match hosts_block_in_sync(&facts.project_files.projects, self.hosts.as_ref()) {
            Ok(in_sync) => in_sync,
            Err(e) => {
                tracing::warn!("hosts file unreadable: {e}");
                false
            }
        };
        match report.hosts_file_in_sync {
            true => check("hosts-block", CheckStatus::Ok, "the hosts file lists every project hostname", None),
            false => check("hosts-block", CheckStatus::Warning, "the Lampo block of the hosts file is out of date", Some("lampo up --allow-elevation".into())),
        }
    }
}

/// `packages`: packages required by the configuration are installed.
pub struct PackagesCheck {
    pub services: Arc<ServiceCatalog>,
}

impl DoctorCheck for PackagesCheck {
    fn run(&self, facts: &DoctorFacts, report: &mut DoctorReportDto) -> DoctorCheckDto {
        report.missing_packages = missing(&requirements(&facts.project_files.projects, &facts.worktrees, &facts.settings, &self.services), &facts.installed);
        match report.missing_packages.is_empty() {
            true => check("packages", CheckStatus::Ok, "every package the projects need is installed", None),
            false => check("packages", CheckStatus::Error, format!("missing packages: {}", report.missing_packages.join(", ")), Some("lampo package install <kind> <series>".into())),
        }
    }
}

/// `folders`: every project folder exists.
pub struct FoldersCheck {
    pub fs: Arc<dyn FileSystemInspector>,
}

impl DoctorCheck for FoldersCheck {
    fn run(&self, facts: &DoctorFacts, _report: &mut DoctorReportDto) -> DoctorCheckDto {
        let missing: Vec<String> = facts.project_files.projects.iter().filter(|p| !self.fs.is_dir(p.root.as_path())).map(|p| format!("{} ({})", p.id, p.root)).collect();
        match missing.is_empty() {
            true => check("folders", CheckStatus::Ok, "every project folder exists", None),
            false => check("folders", CheckStatus::Warning, format!("project folders not found: {}", missing.join(", ")), Some("relink the project to its new folder, or delete it".into())),
        }
    }
}

/// `stale-worktrees`: environments `prune` would remove (`WorktreeEnvironment::is_stale`).
pub struct StaleWorktreesCheck {
    pub fs: Arc<dyn FileSystemInspector>,
}

impl DoctorCheck for StaleWorktreesCheck {
    fn run(&self, facts: &DoctorFacts, report: &mut DoctorReportDto) -> DoctorCheckDto {
        report.stale_worktrees =
            facts.worktrees.iter().filter(|w| w.is_stale(|id| facts.project_files.is_registered(id), |root| self.fs.is_dir(root.as_path()))).map(|w| w.site_id().to_string()).collect();
        match report.stale_worktrees.is_empty() {
            true => check("stale-worktrees", CheckStatus::Ok, "no stale worktree environment", None),
            false => check(
                "stale-worktrees",
                CheckStatus::Warning,
                format!("worktree environments whose folder or project is gone: {}", report.stale_worktrees.join(", ")),
                Some("lampo worktree prune".into()),
            ),
        }
    }
}

/// `app`: the Lampo app answers on the control pipe.
pub struct AppCheck;

impl DoctorCheck for AppCheck {
    fn run(&self, facts: &DoctorFacts, _report: &mut DoctorReportDto) -> DoctorCheckDto {
        match facts.app_running {
            true => check("app", CheckStatus::Ok, "the Lampo app is running", None),
            false => check("app", CheckStatus::Warning, "the Lampo app is not running (mutating commands start it)", None),
        }
    }
}
