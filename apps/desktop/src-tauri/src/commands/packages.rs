use lampo_application::dto::*;
use lampo_application::{AppResult, CallContext};
use tauri::State;

use super::tasks::spawn_task;
use crate::state::AppState;

#[tauri::command]
pub async fn list_catalog(state: State<'_, AppState>, kind: Option<String>) -> AppResult<Vec<CatalogEntryDto>> {
    state.package_catalog.list(kind.as_deref()).await
}

#[tauri::command]
pub fn list_installed_packages(state: State<'_, AppState>) -> AppResult<Vec<InstalledPackageDto>> {
    state.package_catalog.list_installed()
}

#[tauri::command]
pub async fn check_package_updates(state: State<'_, AppState>) -> AppResult<Vec<UpdateAvailableDto>> {
    state.package_catalog.check_updates().await
}

/// Background task; listen to `lampo://task-progress` / `lampo://task-finished`.
#[tauri::command]
pub fn install_package(state: State<'_, AppState>, request: InstallPackageRequest) -> TaskStartedDto {
    let install = state.install_package.clone();
    let operation = TaskOperationDto::InstallPackage { request: request.clone() };
    spawn_task(&state, operation, move |progress, cancel| async move { install.execute(request, &CallContext::GUI, progress.as_ref(), &cancel).await })
}

/// Extensions the installed PHP `php_version` (`"8.3"`) can load.
#[tauri::command]
pub fn list_php_extensions(state: State<'_, AppState>, php_version: String) -> AppResult<Vec<PhpExtensionDto>> {
    state.php_extensions.list(&php_version)
}

#[tauri::command]
pub fn uninstall_package(state: State<'_, AppState>, id: PackageIdDto) -> AppResult<()> {
    state.uninstall_package.execute(id)
}

#[tauri::command]
pub fn first_run_status(state: State<'_, AppState>) -> AppResult<FirstRunStatusDto> {
    state.first_run.status()
}

/// Background task (first-run wizard downloads).
#[tauri::command]
pub fn run_first_run(state: State<'_, AppState>) -> TaskStartedDto {
    let first_run = state.first_run.clone();
    spawn_task(&state, TaskOperationDto::FirstRun, move |progress, cancel| async move { first_run.run(&CallContext::GUI, progress.as_ref(), &cancel).await })
}
