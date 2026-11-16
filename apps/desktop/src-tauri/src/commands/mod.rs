//! The Tauri command surface. Every command delegates to one use case and returns DTOs or
//! `AppError` (serialised as `{ code, message, details? }`). No business logic here.
//!
//! Naming: snake_case Rust names = the `invoke` command names; arguments are camelCase on the
//! JS side (Tauri converts). Mirrored 1:1 in `src/lib/api/commands.ts`.
//!
//! Long operations return `TaskStartedDto` immediately and report through
//! `lampo://task-progress` / `lampo://task-finished`; `cancel_task` cancels them.

pub mod app;
mod blocking;
pub mod diagnostics;
pub mod integrations;
pub mod logs;
pub mod packages;
pub mod presets;
pub mod projects;
pub mod services;
pub mod settings;
pub mod tasks;
pub mod worktrees;

/// Registers every command. Keep in sync with `src/lib/api/commands.ts`.
#[macro_export]
macro_rules! lampo_command_handler {
    () => {
        tauri::generate_handler![
            $crate::commands::app::app_info,
            $crate::commands::app::get_status,
            $crate::commands::app::quit_app,
            $crate::commands::app::prepare_update,
            $crate::commands::app::cancel_update,
            $crate::commands::projects::list_projects,
            $crate::commands::projects::get_project,
            $crate::commands::projects::detect_project,
            $crate::commands::projects::suggest_addresses,
            $crate::commands::projects::create_project,
            $crate::commands::projects::scaffold_project,
            $crate::commands::projects::scaffold_parent_exists,
            $crate::commands::projects::update_project,
            $crate::commands::projects::delete_project,
            $crate::commands::projects::set_project_active,
            $crate::commands::projects::set_project_php,
            $crate::commands::projects::relink_project,
            $crate::commands::projects::export_project,
            $crate::commands::projects::open_terminal,
            $crate::commands::projects::open_project_folder,
            $crate::commands::projects::open_in_vscode,
            $crate::commands::projects::import_project,
            $crate::commands::presets::list_presets,
            $crate::commands::presets::save_preset,
            $crate::commands::presets::delete_preset,
            $crate::commands::packages::list_catalog,
            $crate::commands::packages::list_installed_packages,
            $crate::commands::packages::check_package_updates,
            $crate::commands::packages::install_package,
            $crate::commands::packages::list_php_extensions,
            $crate::commands::packages::uninstall_package,
            $crate::commands::packages::first_run_status,
            $crate::commands::packages::run_first_run,
            $crate::commands::tasks::cancel_task,
            $crate::commands::tasks::list_running_tasks,
            $crate::commands::services::list_services,
            $crate::commands::services::start_services,
            $crate::commands::services::stop_services,
            $crate::commands::services::restart_services,
            $crate::commands::services::apply_environment,
            $crate::commands::worktrees::list_worktrees,
            $crate::commands::worktrees::create_worktree,
            $crate::commands::worktrees::remove_worktree,
            $crate::commands::worktrees::prune_worktrees,
            $crate::commands::logs::list_log_sources,
            $crate::commands::logs::tail_log,
            $crate::commands::logs::read_log_from,
            $crate::commands::logs::open_log_file,
            $crate::commands::logs::open_log_folder,
            $crate::commands::logs::export_log_lines,
            $crate::commands::settings::get_settings,
            $crate::commands::settings::list_service_kinds,
            $crate::commands::settings::update_settings,
            $crate::commands::diagnostics::run_doctor,
            $crate::commands::diagnostics::hosts_in_sync,
            $crate::commands::diagnostics::shim_status,
            $crate::commands::diagnostics::install_shims,
            $crate::commands::diagnostics::uninstall_shims,
            $crate::commands::integrations::claude_code_status,
            $crate::commands::integrations::install_claude_code,
            $crate::commands::integrations::uninstall_claude_code,
        ]
    };
}
