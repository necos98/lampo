//! `lampo doctor` / GUI diagnostics. Read-only, works without the app.
//!
//! [`Doctor`] loads the configuration once ([`DoctorFacts`]) and runs every registered
//! [`DoctorCheck`] in order. Each check is one small type with only the ports it needs; the
//! composition roots decide which checks run (open-closed: a new check is a new type).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use lampo_domain::address::SiteAddress;
use lampo_domain::hosts::HostsBlock;
use lampo_domain::package::InstalledPackage;
use lampo_domain::ports::{
    ControlChannelClient, ExecutableLocator, FileSystemInspector, HostsFileReader, InstalledPackageRegistry, PortAvailability, PortProbe, ProjectRepository, SettingsRepository,
    ShimInstaller, UserPathEditor, VcRuntimeDetector, WorktreeRepository,
};
use lampo_domain::project::Project;
use lampo_domain::settings::Settings;
use lampo_domain::shim::ShimTool;
use lampo_domain::tcp_port::Port;
use lampo_domain::vc_runtime::VcRedistributable;
use lampo_domain::worktree::WorktreeEnvironment;

use super::requirements::{missing, requirements};
use super::service_catalog::ServiceCatalog;
use super::shims::shadowed_tools;
use super::support::all_sites;
use crate::dto::{CheckStatus, DoctorCheckDto, DoctorReportDto, PortConflictDto, VcRuntimeIssueDto};
use crate::error::AppResult;

/// What every check may look at, loaded once per run.
pub struct DoctorFacts {
    pub projects: Vec<Project>,
    pub worktrees: Vec<WorktreeEnvironment>,
    pub settings: Settings,
    pub installed: Vec<InstalledPackage>,
    /// The app answers on the control pipe (its services then hold their own ports).
    pub app_running: bool,
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

    pub async fn run(&self) -> AppResult<DoctorReportDto> {
        let d = &self.deps;
        let facts = DoctorFacts {
            projects: d.projects.list()?,
            worktrees: d.worktrees.list_all()?,
            settings: d.settings.load()?,
            installed: d.installed.list()?,
            app_running: d.control.is_server_running(),
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
            for (id, port) in self.services.planned_ports(&facts.projects, &facts.worktrees, &facts.settings) {
                wanted.entry(port).or_insert(id);
            }
            for site in all_sites(&facts.projects, &facts.worktrees).iter().filter(|s| s.served()) {
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
        let desired = HostsBlock::for_projects(&facts.projects);
        report.hosts_file_in_sync = match self.hosts.read_block() {
            Ok(current) => current.unwrap_or_default() == desired,
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
        report.missing_packages = missing(&requirements(&facts.projects, &facts.worktrees, &facts.settings, &self.services), &facts.installed);
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
        let missing: Vec<String> = facts.projects.iter().filter(|p| !self.fs.is_dir(p.root.as_path())).map(|p| format!("{} ({})", p.id, p.root)).collect();
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
        report.stale_worktrees = facts.worktrees.iter().filter(|w| w.is_stale(&facts.projects, |root| self.fs.is_dir(root.as_path()))).map(|w| w.site_id().to_string()).collect();
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
