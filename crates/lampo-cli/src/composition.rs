//! CLI composition root: the only place in the CLI that names concrete adapters.
//!
//! The CLI builds only **read-only, file-based** use cases locally. Everything that mutates
//! state or needs a process goes through [`ControlClient`] to the running app (started headless
//! when needed), which is the single owner of service processes.

use std::sync::Arc;

use lampo_application::control::{ControlClient, instance_pipe_name};
use lampo_application::use_cases::*;
use lampo_application::AppResult;
use lampo_domain::ports::{
    ControlChannelClient, ExecutableLocator, FileSystemInspector, InstalledPackageRegistry, ProjectRepository, SettingsRepository, ShimInstaller, UserPathEditor, VcRuntimeDetector,
    WorktreeRepository,
};
use lampo_domain::resolution::ResolverChain;
use lampo_infra_services::{FileLogReader, HeadlessAppLauncher, NamedPipeControlClient, TcpPortProbe, TokioTimer};
use lampo_infra_store as store;

use crate::context::{CliContext, CliEnv};
use crate::local::CtrlC;
use crate::process::SystemRunner;

pub fn build() -> AppResult<CliContext> {
    let paths = store::paths::resolve_lampo_paths()?;

    let projects_repo: Arc<dyn ProjectRepository> = Arc::new(store::TomlProjectRepository::new(paths.clone()));
    let worktrees_repo: Arc<dyn WorktreeRepository> = Arc::new(store::TomlWorktreeRepository::new(paths.clone()));
    let settings_repo: Arc<dyn SettingsRepository> = Arc::new(store::TomlSettingsRepository::new(paths.clone()));
    let fs: Arc<dyn FileSystemInspector> = Arc::new(store::LocalFileSystem);
    let git = Arc::new(store::FsGitInspector);
    let index = Arc::new(store::JsonShimIndex::new(paths.shim_index_file()));
    let installed: Arc<dyn InstalledPackageRegistry> = Arc::new(lampo_infra_packages::FsInstalledPackageRegistry::new(paths.clone()));
    let vc: Arc<dyn VcRuntimeDetector> = Arc::new(lampo_infra_packages::RegistryVcRuntimeDetector);
    let locator: Arc<dyn ExecutableLocator> = Arc::new(store::PathExecutableLocator);
    let isolated_home = store::paths::home_override();
    // An isolated data root (`LAMPO_HOME`) never edits the real user PATH.
    let user_path: Arc<dyn UserPathEditor> = match isolated_home {
        Some(_) => Arc::new(store::ProcessOnlyUserPath),
        None => Arc::new(store::WindowsUserPath),
    };
    // The app and its helper executables: next to this executable, or recorded by the app when
    // this is the copy in the shim dir.
    let app_dir = store::app_location::resolve_app_dir(&current_exe_dir(), &paths.app_location_file());
    let shim_tools = lampo_domain::shim::tools::all();
    let shim_installer: Arc<dyn ShimInstaller> =
        Arc::new(store::ExeShimInstaller::new(app_dir.join("lampo-shim.exe"), shim_tools.iter().map(|t| t.name()).collect()).with_cli(app_dir.join("lampo.exe")));
    // The registered service kinds (the same list as the app's).
    let services = Arc::new(ServiceCatalog::new(lampo_domain::service::builtin::all()));

    let user = std::env::var("USERNAME").unwrap_or_else(|_| "user".into());
    let channel: Arc<dyn ControlChannelClient> = Arc::new(NamedPipeControlClient::new(instance_pipe_name(&user, isolated_home.as_deref())));
    let launcher = Arc::new(HeadlessAppLauncher::new(app_dir.join(store::app_location::APP_EXE_NAME)));

    let resolver = Arc::new(SiteResolver::new(index, ResolverChain::standard(git.clone())));
    let projects = Arc::new(ProjectQueries::new(projects_repo.clone(), worktrees_repo.clone(), settings_repo.clone(), fs.clone()));
    let app = Arc::new(ControlClient::new(channel.clone(), launcher, Arc::new(store::SystemClock), Arc::new(TokioTimer)));
    let running_channel = channel.clone();
    let shim_dir = paths.shim_dir();
    let checks: Vec<Arc<dyn DoctorCheck>> = vec![
        Arc::new(VcRuntimeCheck { vc: vc.clone() }),
        Arc::new(PortsCheck { probe: Arc::new(TcpPortProbe), services: services.clone() }),
        Arc::new(ShimsOnPathCheck { installer: shim_installer.clone(), user_path: user_path.clone(), shim_dir: shim_dir.clone() }),
        Arc::new(PathShadowingCheck { locator: locator.clone(), shim_dir, tools: shim_tools.clone() }),
        Arc::new(HostsBlockCheck { hosts: Arc::new(lampo_infra_web::ElevatedHostsFile::new(lampo_infra_web::hosts::HOSTS_FILE.into(), app_dir.join("lampo-hosts-helper.exe"))) }),
        Arc::new(PackagesCheck { services: services.clone() }),
        Arc::new(FoldersCheck { fs: fs.clone() }),
        Arc::new(StaleWorktreesCheck { fs: fs.clone() }),
        Arc::new(AppCheck),
    ];

    let settings = Arc::new(SettingsQuery::new(settings_repo.clone(), services.clone()));

    Ok(CliContext {
        env: CliEnv::from_process(),
        sites: resolver.clone(),
        projects: projects.clone(),
        // Every detector, as in the app: the same folder analysis as `project create`.
        detection: Arc::new(DetectProject::new(store::detectors::all(), projects_repo.clone(), fs.clone())),
        worktrees: Arc::new(WorktreeQueries::new(worktrees_repo.clone(), projects_repo.clone(), settings_repo.clone(), fs.clone())),
        agent: Arc::new(AgentContextQuery::new(AgentContextDeps {
            resolver: resolver.clone(),
            projects: projects_repo.clone(),
            worktrees: worktrees_repo.clone(),
            settings: settings_repo.clone(),
            git: git.clone(),
            fs: fs.clone(),
            control: channel.clone(),
            paths: paths.clone(),
            services: services.clone(),
        })),
        hook_input: Arc::new(crate::local::StdinHookInput),
        toolchain: Arc::new(ToolchainQuery::new(resolver, paths.clone())),
        logs: Arc::new(LogQueries::new(Arc::new(FileLogReader), projects_repo.clone(), settings_repo.clone(), services.clone(), paths.clone())),
        doctor: Arc::new(Doctor::new(DoctorDeps {
            projects: projects_repo.clone(),
            worktrees: worktrees_repo.clone(),
            settings: settings_repo.clone(),
            installed: installed.clone(),
            control: channel.clone(),
            checks,
        })),
        shims: Arc::new(ShimSetup::new(shim_installer, user_path, locator, paths.clone(), shim_tools)),
        packages: Arc::new(PackageCatalog::new(PackageCatalogDeps {
            // Offline listing: embedded catalog only; live data comes through the app.
            sources: vec![Arc::new(lampo_infra_packages::EmbeddedCatalog::new())],
            installed,
            vc,
            projects: projects_repo,
            worktrees: worktrees_repo,
            settings: settings_repo.clone(),
            services: services.clone(),
        })),
        offline_status: Arc::new(StatusQuery::new(projects, Arc::new(OfflineServiceStatus::new(services, settings_repo)), Arc::new(move || running_channel.is_server_running()))),
        app,
        runner: Arc::new(SystemRunner),
        interrupt: Arc::new(CtrlC),
        settings,
    })
}

fn current_exe_dir() -> std::path::PathBuf {
    std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())).unwrap_or_default()
}
