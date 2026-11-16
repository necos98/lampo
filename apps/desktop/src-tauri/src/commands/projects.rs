use lampo_application::dto::*;
use lampo_application::use_cases::ExternalApp;
use lampo_application::{AppResult, CallContext};
use tauri::State;

use super::blocking::run_blocking;
use crate::state::AppState;

#[tauri::command]
pub async fn list_projects(state: State<'_, AppState>) -> AppResult<ProjectListDto> {
    let projects = state.project_queries.clone();
    run_blocking(move || projects.list()).await
}

#[tauri::command]
pub async fn get_project(state: State<'_, AppState>, project_id: String) -> AppResult<ProjectDto> {
    let projects = state.project_queries.clone();
    run_blocking(move || projects.get(&project_id)).await
}

#[tauri::command]
pub async fn detect_project(state: State<'_, AppState>, folder: String) -> AppResult<DetectionDto> {
    let detect = state.detect_project.clone();
    run_blocking(move || detect.execute(&folder)).await
}

#[tauri::command]
pub async fn create_project(state: State<'_, AppState>, request: CreateProjectRequest) -> AppResult<CreateProjectResultDto> {
    state.create_project.execute(request, &CallContext::GUI).await
}

#[tauri::command]
pub async fn update_project(state: State<'_, AppState>, request: UpdateProjectRequest) -> AppResult<ProjectDto> {
    state.update_project.execute(request, &CallContext::GUI).await
}

#[tauri::command]
pub async fn delete_project(state: State<'_, AppState>, project_id: String) -> AppResult<DeleteProjectResultDto> {
    state.delete_project.execute(&project_id, &CallContext::GUI).await
}

#[tauri::command]
pub async fn set_project_active(state: State<'_, AppState>, request: SetProjectActiveRequest) -> AppResult<ProjectDto> {
    state.set_project_active.execute(request, &CallContext::GUI).await
}

#[tauri::command]
pub async fn set_project_php(state: State<'_, AppState>, request: SetProjectPhpRequest) -> AppResult<ProjectDto> {
    state.set_project_php.execute(request, &CallContext::GUI).await
}

#[tauri::command]
pub async fn relink_project(state: State<'_, AppState>, request: RelinkProjectRequest) -> AppResult<ProjectDto> {
    state.relink_project.execute(request, &CallContext::GUI).await
}

/// Opens a PowerShell console in the project folder with the project's toolchain on PATH
/// (`lampo -p <id> exec -- powershell`; `lampo.exe` sits next to the app executable).
#[tauri::command]
pub async fn open_terminal(state: State<'_, AppState>, project_id: String) -> AppResult<()> {
    open_project_in(&state, project_id, ExternalApp::Terminal).await
}

/// Opens the project folder in Windows Explorer.
#[tauri::command]
pub async fn open_project_folder(state: State<'_, AppState>, project_id: String) -> AppResult<()> {
    open_project_in(&state, project_id, ExternalApp::Folder).await
}

/// Opens the project folder in Visual Studio Code.
#[tauri::command]
pub async fn open_in_vscode(state: State<'_, AppState>, project_id: String) -> AppResult<()> {
    open_project_in(&state, project_id, ExternalApp::VsCode).await
}

async fn open_project_in(state: &AppState, project_id: String, app: ExternalApp) -> AppResult<()> {
    let open = state.open_project_in.clone();
    run_blocking(move || open.execute(&project_id, app)).await
}

#[tauri::command]
pub async fn export_project(state: State<'_, AppState>, request: ExportProjectRequest) -> AppResult<()> {
    let export = state.export_project.clone();
    run_blocking(move || export.execute(request)).await
}

#[tauri::command]
pub async fn import_project(state: State<'_, AppState>, request: ImportProjectRequest) -> AppResult<ImportProjectResultDto> {
    state.import_project.execute(request, &CallContext::GUI).await
}

#[tauri::command]
pub async fn suggest_addresses(state: State<'_, AppState>, project_id: String) -> AppResult<AddressSuggestionsDto> {
    state.suggest_addresses.execute(&project_id).await
}

/// Background task: creates the folder of a new project (git clone / Laravel Installer),
/// streaming the tools' output as `lampo://task-progress` messages. The front-end then detects
/// and creates the project on `<parentDir>\<folderName>`.
#[tauri::command]
pub fn scaffold_project(state: State<'_, AppState>, request: ScaffoldProjectRequest) -> TaskStartedDto {
    let scaffold = state.scaffold_project.clone();
    let operation = TaskOperationDto::ScaffoldProject { request: request.clone() };
    super::tasks::spawn_task(&state, operation, move |progress, cancel| async move { scaffold.execute(request, progress.as_ref(), &cancel).await })
}
