//! Desktop composition root: the only place in the app that names concrete adapters.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use lampo_application::control::{ControlDispatcher, ControlServices, instance_pipe_name};
use lampo_application::tasks::TaskRegistry;
use lampo_application::use_cases::*;
use lampo_domain::ports::*;
use lampo_infra_packages as packages;
use lampo_infra_services as services;
use lampo_infra_store as store;
use lampo_infra_web as web;
use tauri::AppHandle;

use crate::events::TauriEventPublisher;
use crate::state::AppState;

pub struct Built {
    pub state: AppState,
    pub control_server: Arc<dyn ControlChannelServer>,
    pub control_handler: Arc<dyn ControlRequestHandler>,
    /// Rotates the PHP error logs while the app runs (spawned by the app setup).
    pub log_rotation: Arc<PeriodicLogRotation>,
    /// Restarts the services that crash (spawned by the app setup with `crashes`).
    pub auto_restart: Arc<ServiceAutoRestart>,
    /// The crashes the runners report, for `auto_restart`.
    pub crashes: CrashReceiver,
}

/// Folder of the running executable. The helper executables (`lampo.exe`, `lampo-shim.exe`,
/// `lampo-hosts-helper.exe`) live there: next to `lampo-desktop.exe` in `target\<profile>` during
/// development, and installed next to it as Tauri external binaries in a bundle
/// (`tauri.bundle.conf.json`).
fn exe_dir() -> PathBuf {
    std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())).unwrap_or_default()
}

/// Wires every adapter into every use case. Does no IO besides resolving the app roots
/// (adapters are lazy), so it cannot fail on a fresh machine.
pub fn build(handle: AppHandle) -> Result<Built, lampo_application::AppError> {
    let paths = store::paths::resolve_lampo_paths()?;
    let isolated_home = store::paths::home_override();
    // Every event goes to the webview; the crashes of running services also to the automatic restart.
    let (crash_relay, crashes) = CrashRelay::new(Arc::new(TauriEventPublisher::new(handle.clone())));
    let events: Arc<dyn EventPublisher> = Arc::new(crash_relay);
    let tasks = Arc::new(TaskRegistry::new());

    // --- store ---
    let projects: Arc<dyn ProjectRepository> = Arc::new(store::TomlProjectRepository::new(paths.clone()));
    let worktrees: Arc<dyn WorktreeRepository> = Arc::new(store::TomlWorktreeRepository::new(paths.clone()));
    let settings: Arc<dyn SettingsRepository> = Arc::new(store::TomlSettingsRepository::new(paths.clone()));
    let custom_presets: Arc<dyn CustomPresetRepository> = Arc::new(store::TomlPresetRepository::new(paths.clone()));
    let transfer: Arc<dyn ProjectTransfer> = Arc::new(store::TomlProjectTransfer);
    let shim_index = Arc::new(store::JsonShimIndex::new(paths.shim_index_file()));
    let fs: Arc<dyn FileSystemInspector> = Arc::new(store::LocalFileSystem);
    let git: Arc<dyn GitInspector> = Arc::new(store::FsGitInspector);
    let clock: Arc<dyn Clock> = Arc::new(store::SystemClock);
    // An isolated data root (`LAMPO_HOME`) never edits the real user PATH.
    let user_path: Arc<dyn UserPathEditor> = match isolated_home {
        Some(_) => Arc::new(store::ProcessOnlyUserPath),
        None => Arc::new(store::WindowsUserPath),
    };
    let locator: Arc<dyn ExecutableLocator> = Arc::new(store::PathExecutableLocator);
    let shim_tools = lampo_domain::shim::tools::all();
    // The shim dir also gets a copy of the CLI, which finds this app through `app_location`.
    let shim_installer: Arc<dyn ShimInstaller> =
        Arc::new(store::ExeShimInstaller::new(exe_dir().join("lampo-shim.exe"), shim_tools.iter().map(|t| t.name()).collect()).with_cli(exe_dir().join("lampo.exe")));
    let app_location: Arc<dyn AppLocationRecorder> = Arc::new(store::AppLocationFile::new(paths.app_location_file(), exe_dir()));
    let detectors = store::detectors::all();

    // --- packages ---
    let http = packages::http::build_client()?;
    // Live sources first (on duplicate ids the first source wins), the embedded catalog last.
    let sources: Vec<Arc<dyn PackageCatalogSource>> = packages::catalog::default_sources(http.clone(), &paths);
    let installed: Arc<dyn InstalledPackageRegistry> = Arc::new(packages::FsInstalledPackageRegistry::new(paths.clone()));
    // One slot lock for the whole app: installs and the startup leftover sweep share it.
    let package_slots = Arc::new(packages::FsPackageSlotLock::new(&paths));
    let vc: Arc<dyn VcRuntimeDetector> = Arc::new(packages::RegistryVcRuntimeDetector);

    // --- services ---
    let supervisor = Arc::new(services::JobObjectSupervisor::new());
    let process_supervisor: Arc<dyn ProcessSupervisor> = supervisor.clone();
    let commands: Arc<dyn CommandRunner> = supervisor.clone();
    let pipes: Arc<dyn PipedCommandRunner> = supervisor.clone();
    let streaming: Arc<dyn StreamingCommandRunner> = supervisor;
    let port_probe: Arc<dyn PortProbe> = Arc::new(services::TcpPortProbe);
    // The stored MySQL passwords (DPAPI): root's, which Lampo's own clients authenticate with,
    // and each project account's.
    let password_store = Arc::new(services::DpapiMysqlPasswords::new(paths.secrets_dir()));
    let mysql_passwords: Arc<dyn MysqlRootPasswordReader> = password_store.clone();
    let mysql_admin = Arc::new(services::MysqlCliAdmin::new(commands.clone(), pipes, mysql_passwords.clone()));
    let db_admin: Arc<dyn DatabaseAdmin> = mysql_admin.clone();
    let passwords = Arc::new(MysqlPasswords::with_store(password_store, Arc::new(services::SystemRandomPasswords)));
    let connection_passwords: Arc<dyn ConnectionPasswords> = passwords.clone();
    // The registered service kinds and one runner per kind.
    let service_catalog = Arc::new(ServiceCatalog::new(lampo_domain::service::builtin::all()));
    let runners: Vec<Arc<dyn ServiceRunner>> = vec![
        Arc::new(web::ApacheRunner::new(process_supervisor.clone(), events.clone())),
        Arc::new(services::MysqlRunner::new(process_supervisor.clone(), commands.clone(), mysql_passwords, events.clone())),
        Arc::new(services::RedisRunner::new(process_supervisor.clone(), events.clone())),
        Arc::new(services::MailpitRunner::new(process_supervisor, events.clone())),
    ];
    let user = std::env::var("USERNAME").unwrap_or_else(|_| "user".into());
    let pipe = instance_pipe_name(&user, isolated_home.as_deref());
    let control_server: Arc<dyn ControlChannelServer> = Arc::new(services::NamedPipeControlServer::new(pipe.clone()));

    // --- web ---
    // UAC prompts for the hosts file belong to the app window (when shown).
    let owner_handle = handle.clone();
    let hosts_launcher = web::hosts::elevation::RunAsLauncher::default().with_owner(Arc::new(move || crate::tray::main_window_handle(&owner_handle)));
    let hosts = Arc::new(web::ElevatedHostsFile::system(exe_dir().join("lampo-hosts-helper.exe"), Arc::new(hosts_launcher))?);

    let php_ini: Arc<dyn PhpIniWriter> = Arc::new(web::PhpIniRenderer);

    // --- use cases ---
    // One scanner for the whole app: it caches the modules compiled into each PHP build.
    let php_extensions = Arc::new(PhpExtensionsQuery::new(installed.clone(), Arc::new(packages::FsPhpExtensionScanner::default())));
    // Closed by `Shutdown`: the control pipe, the crash watchdog and the environment sync check it.
    let shutdown_gate = Arc::new(ShutdownGate::default());
    let environment = Arc::new(ApplyEnvironment::new(ApplyEnvironmentDeps {
        projects: projects.clone(),
        worktrees: worktrees.clone(),
        settings: settings.clone(),
        installed: installed.clone(),
        web_config: Arc::new(web::ApacheConfigurator::new(commands.clone())),
        php_ini: php_ini.clone(),
        extensions: php_extensions.clone(),
        hosts_reader: hosts.clone(),
        hosts_writer: hosts.clone(),
        shim_index: shim_index.clone(),
        services: service_catalog.clone(),
        runners: runners.clone(),
        port_probe: port_probe.clone(),
        paths: paths.clone(),
        events: events.clone(),
        mysql_accounts: Arc::new(MysqlAccounts::new(mysql_admin, passwords)),
        gate: shutdown_gate.clone(),
    }));
    let env: Arc<dyn EnvironmentSync> = environment.clone();

    let builtin_presets = lampo_domain::preset::builtin::all();
    let preset_queries = Arc::new(PresetQueries::new(builtin_presets.clone(), custom_presets.clone(), settings.clone(), installed.clone()));
    let detect_project = Arc::new(DetectProject::new(detectors, projects.clone(), fs.clone()));
    let project_queries = Arc::new(ProjectQueries::new(projects.clone(), worktrees.clone(), settings.clone(), fs.clone(), connection_passwords.clone()));
    // One allocator for the whole app: projects and worktree environments share its lock.
    let allocator = Arc::new(LoopbackPortAllocator::new(projects.clone(), worktrees.clone(), settings.clone(), port_probe.clone()));
    // Every project save and removal (GUI, CLI patch, create, import, PHP switch, delete) goes
    // through this writer: the allocator lock plus the environment policy and the conflict checks.
    let project_writer = ProjectWriter::new(ProjectWriterDeps {
        projects: projects.clone(),
        allocator: allocator.clone(),
        env: env.clone(),
        events: events.clone(),
        queries: project_queries.clone(),
        policy: Arc::new(ProjectEnvironmentPolicy::new(service_catalog.clone(), installed.clone(), php_extensions.clone())),
    });
    let service_queries = Arc::new(ServiceQueries::new(runners.clone(), service_catalog.clone(), projects.clone(), worktrees.clone(), settings.clone()));
    let service_control = Arc::new(ServiceControl::new(ServiceControlDeps {
        runners: runners.clone(),
        catalog: service_catalog.clone(),
        settings: settings.clone(),
        lifecycle: environment.clone(),
        env: env.clone(),
    }));
    // Every settings save (GUI page, `lampo settings set`, default PHP adoption, first-run
    // wizard) goes through this writer: the allocator lock plus the port and default-PHP checks.
    let settings_writer = SettingsWriter {
        repo: settings.clone(),
        allocator: allocator.clone(),
        port_probe: port_probe.clone(),
        installed: installed.clone(),
        events: events.clone(),
        services: service_catalog.clone(),
    };
    let install_package = Arc::new(InstallPackage::new(InstallPackageDeps {
        sources: sources.clone(),
        downloader: Arc::new(packages::HttpDownloader::new(http)),
        extractor: Arc::new(packages::ZipExtractor),
        installed: installed.clone(),
        slots: package_slots.clone(),
        hooks: packages::hooks::all(),
        vc: vc.clone(),
        settings: settings_writer.clone(),
        paths: paths.clone(),
        env: env.clone(),
        events: events.clone(),
    }));
    // The GUI "Pacchetti" page and `lampo package uninstall` (via the control pipe).
    let uninstall_package = Arc::new(UninstallPackage::new(UninstallPackageDeps {
        installed: installed.clone(),
        projects: projects.clone(),
        worktrees: worktrees.clone(),
        settings: settings.clone(),
        events: events.clone(),
        env: env.clone(),
        services: service_catalog.clone(),
    }));
    let worktree_queries = Arc::new(WorktreeQueries::new(worktrees.clone(), projects.clone(), settings.clone(), fs.clone(), connection_passwords.clone()));
    let worktree_databases = Arc::new(WorktreeDatabases::new(projects.clone(), settings.clone(), installed.clone(), db_admin.clone(), service_control.clone(), service_catalog.clone()));
    let status = Arc::new(StatusQuery::new(project_queries.clone(), service_queries.clone(), Arc::new(|| true)));
    let app_info = Arc::new(AppInfoQuery::new(env!("CARGO_PKG_VERSION"), paths.clone()));
    let database = Arc::new(DatabaseService::new(db_admin, projects.clone(), worktrees.clone(), settings.clone(), installed.clone(), service_control.clone(), service_catalog.clone()));
    let create_project = Arc::new(CreateProject::new(project_writer.clone(), preset_queries.clone(), detect_project.clone(), fs.clone(), database.clone()));
    let scaffold_project = Arc::new(ScaffoldProject::new(ScaffoldProjectDeps {
        files: Arc::new(store::LocalFileSystem),
        runner: streaming,
        locator: locator.clone(),
        installed: installed.clone(),
        php_ini,
        paths: paths.clone(),
    }));
    let update_project = Arc::new(UpdateProject::new(project_writer.clone()));
    let delete_project = Arc::new(DeleteProject::new(project_writer.clone(), worktrees.clone(), events.clone()));
    let import_project = Arc::new(ImportProject::new(project_writer.clone(), transfer.clone(), fs.clone()));
    let set_project_active = Arc::new(SetProjectActive::new(project_writer.clone()));
    let set_project_php = Arc::new(SetProjectPhp::new(project_writer.clone(), installed.clone()));
    // CLI-only (`lampo project set`, via the control pipe): the GUI edits the full project.
    let patch_project = Arc::new(PatchProjectEnvironment::new(project_writer.clone(), installed.clone()));
    // CLI-only (`lampo settings set`, via the control pipe): the GUI saves the full settings.
    let patch_settings = Arc::new(PatchSettings::new(settings_writer.clone(), env.clone()));
    let worktree_up = Arc::new(WorktreeUp::new(WorktreeUpDeps {
        projects: projects.clone(),
        worktrees: worktrees.clone(),
        git,
        clock: clock.clone(),
        allocator: allocator.clone(),
        databases: worktree_databases.clone(),
        queries: worktree_queries.clone(),
        env: env.clone(),
        events: events.clone(),
    }));
    let worktree_down = Arc::new(WorktreeDown::new(worktrees.clone(), allocator.clone(), worktree_databases.clone(), env.clone(), events.clone()));
    let prune_worktrees = Arc::new(PruneWorktrees::new(projects.clone(), worktrees.clone(), allocator.clone(), fs.clone(), worktree_databases, env.clone(), events.clone()));
    let shutdown = Arc::new(Shutdown::new(runners, service_catalog.clone(), settings.clone(), shutdown_gate));
    let shims = Arc::new(ShimSetup::new(shim_installer.clone(), user_path.clone(), locator.clone(), paths.clone(), shim_tools.clone()));
    // Claude Code: user scope only, never under an isolated data root (`LAMPO_HOME`).
    let claude_code = Arc::new(ClaudeCodeSetup::new(ClaudeCodeSetupDeps {
        cli: Arc::new(services::ClaudeCliRunner::new(commands.clone(), locator.clone(), std::env::var_os("USERPROFILE").map(PathBuf::from))),
        files: Arc::new(store::LocalFileSystem),
        paths: paths.clone(),
        app_version: env!("CARGO_PKG_VERSION").into(),
        skills: embedded_skills(),
        icon: Some(embedded_icon().to_vec()),
        isolated: isolated_home.is_some(),
    }));

    let quit_handle = handle.clone();
    let control_handler: Arc<dyn ControlRequestHandler> = Arc::new(ControlDispatcher::new(ControlServices {
        app_info: app_info.clone(),
        status: status.clone(),
        create_project: create_project.clone(),
        scaffold_project: scaffold_project.clone(),
        update_project: update_project.clone(),
        set_project_active: set_project_active.clone(),
        set_project_php: set_project_php.clone(),
        patch_project,
        delete_project: delete_project.clone(),
        import_project: import_project.clone(),
        service_control: service_control.clone(),
        environment: env.clone(),
        install_package: install_package.clone(),
        uninstall_package: uninstall_package.clone(),
        // Database administration is CLI-only (via the control pipe) for now.
        database,
        worktree_up: worktree_up.clone(),
        worktree_down: worktree_down.clone(),
        prune_worktrees: prune_worktrees.clone(),
        patch_settings,
        quit: Arc::new(move || crate::tray::quit(&quit_handle)),
        gate: shutdown.gate(),
    }));

    let auto_restart = Arc::new(ServiceAutoRestart::new(ServiceAutoRestartDeps {
        recovery: environment.clone(),
        settings: settings.clone(),
        events: events.clone(),
        timer: Arc::new(services::TokioTimer),
        clock,
        gate: shutdown.gate(),
    }));
    let log_rotation = Arc::new(PeriodicLogRotation::new(Arc::new(services::SizeLogRotator), Arc::new(services::TokioTimer), projects.clone(), worktrees.clone(), paths.clone()));
    let launcher: Arc<dyn ExternalAppLauncher> = Arc::new(services::WindowsExternalAppLauncher);
    let logs = Arc::new(LogQueries::new(Arc::new(services::FileLogReader), projects.clone(), settings.clone(), service_catalog.clone(), paths.clone()));
    let state = AppState {
        events: events.clone(),
        tasks,
        app_info,
        status,
        project_queries: project_queries.clone(),
        detect_project,
        suggest_addresses: Arc::new(SuggestAddresses::new(allocator.clone(), settings.clone())),
        create_project,
        scaffold_project,
        update_project,
        delete_project,
        set_project_active,
        set_project_php,
        relink_project: Arc::new(RelinkProject::new(project_writer.clone(), fs.clone())),
        export_project: Arc::new(ExportProject::new(projects.clone(), transfer.clone())),
        import_project,
        open_project_in: Arc::new(OpenProjectIn::new(OpenProjectInDeps {
            projects: projects.clone(),
            worktrees: worktrees.clone(),
            settings: settings.clone(),
            fs: fs.clone(),
            launcher: launcher.clone(),
        })),
        preset_queries,
        save_preset: Arc::new(SavePreset::new(projects.clone(), custom_presets.clone(), builtin_presets.clone())),
        delete_preset: Arc::new(DeletePreset::new(custom_presets, builtin_presets)),
        package_catalog: Arc::new(PackageCatalog::new(PackageCatalogDeps {
            sources: sources.clone(),
            installed: installed.clone(),
            vc: vc.clone(),
            projects: projects.clone(),
            worktrees: worktrees.clone(),
            settings: settings.clone(),
            services: service_catalog.clone(),
        })),
        php_extensions,
        install_package: install_package.clone(),
        uninstall_package,
        first_run: Arc::new(FirstRunSetup::new(FirstRunDeps {
            settings: settings_writer.clone(),
            sources,
            installed: installed.clone(),
            install: install_package,
            env: env.clone(),
            shims: shims.clone(),
        })),
        service_queries: service_queries.clone(),
        service_control,
        environment,
        worktree_queries,
        worktree_up,
        worktree_down,
        prune_worktrees,
        open_log: Arc::new(OpenLog::new(logs.clone(), fs.clone(), launcher)),
        export_log_lines: Arc::new(ExportLogLines::new(Arc::new(services::FileLogExportWriter))),
        logs,
        settings: Arc::new(SettingsService::new(settings_writer, env.clone())),
        hosts_sync: Arc::new(HostsSyncQuery::new(projects.clone(), hosts.clone())),
        doctor: Arc::new(Doctor::new(DoctorDeps {
            projects,
            worktrees,
            settings: settings.clone(),
            installed,
            control: Arc::new(services::NamedPipeControlClient::new(pipe)),
            service_states: service_queries,
            checks: standard_checks(StandardChecksDeps {
                vc,
                port_probe,
                services: service_catalog,
                shim_installer: shim_installer.clone(),
                user_path: user_path.clone(),
                locator: locator.clone(),
                shim_dir: paths.shim_dir(),
                shim_tools,
                hosts,
                fs,
            }),
        })),
        shims: shims.clone(),
        claude_code: claude_code.clone(),
        startup: Arc::new(
            Startup::new(env, settings, shims, app_location).with_claude_code(claude_code).with_package_sweeper(Arc::new(packages::FsLeftoverSweeper::new(&paths, package_slots))),
        ),
        shutdown,
    };

    Ok(Built { state, control_server, control_handler, log_rotation, auto_restart, crashes })
}

/// Last-resort clean shutdown of the MySQL servers this app started, waiting up to `timeout`
/// (synchronous and stateless, for a panic hook or the end of the Windows session).
pub fn stop_mysql_last_resort(timeout: Duration) -> services::LastResortStop {
    services::stop_mysql_servers(timeout)
}
