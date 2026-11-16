//! Everything Tauri commands need, managed as Tauri state. Built by `composition::build`.

use std::sync::Arc;

use lampo_application::tasks::TaskRegistry;
use lampo_application::use_cases::*;
use lampo_domain::ports::EventPublisher;

pub struct AppState {
    pub events: Arc<dyn EventPublisher>,
    pub tasks: Arc<TaskRegistry>,

    pub app_info: Arc<AppInfoQuery>,
    pub status: Arc<StatusQuery>,

    pub project_queries: Arc<ProjectQueries>,
    pub detect_project: Arc<DetectProject>,
    pub suggest_addresses: Arc<SuggestAddresses>,
    pub create_project: Arc<CreateProject>,
    pub scaffold_project: Arc<ScaffoldProject>,
    pub update_project: Arc<UpdateProject>,
    pub delete_project: Arc<DeleteProject>,
    pub set_project_active: Arc<SetProjectActive>,
    pub set_project_php: Arc<SetProjectPhp>,
    pub relink_project: Arc<RelinkProject>,
    pub export_project: Arc<ExportProject>,
    pub import_project: Arc<ImportProject>,
    pub open_project_in: Arc<OpenProjectIn>,

    pub preset_queries: Arc<PresetQueries>,
    pub save_preset: Arc<SavePreset>,
    pub delete_preset: Arc<DeletePreset>,

    pub package_catalog: Arc<PackageCatalog>,
    pub php_extensions: Arc<PhpExtensionsQuery>,
    pub install_package: Arc<InstallPackage>,
    pub uninstall_package: Arc<UninstallPackage>,
    pub first_run: Arc<FirstRunSetup>,

    pub service_queries: Arc<ServiceQueries>,
    pub service_control: Arc<ServiceControl>,
    pub environment: Arc<ApplyEnvironment>,

    pub worktree_queries: Arc<WorktreeQueries>,
    pub worktree_up: Arc<WorktreeUp>,
    pub worktree_down: Arc<WorktreeDown>,
    pub prune_worktrees: Arc<PruneWorktrees>,

    pub logs: Arc<LogQueries>,
    pub open_log: Arc<OpenLog>,
    pub export_log_lines: Arc<ExportLogLines>,
    pub settings: Arc<SettingsService>,
    pub doctor: Arc<Doctor>,
    pub hosts_sync: Arc<HostsSyncQuery>,
    pub shims: Arc<ShimSetup>,
    pub claude_code: Arc<ClaudeCodeSetup>,

    pub startup: Arc<Startup>,
    pub shutdown: Arc<Shutdown>,
}
