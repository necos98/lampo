use lampo_application::dto::*;
use lampo_application::{AppResult, CallContext};
use tauri::State;

use crate::state::AppState;

#[tauri::command]
pub fn list_projects(state: State<'_, AppState>) -> AppResult<ProjectListDto> {
    state.project_queries.list()
}

#[tauri::command]
pub fn get_project(state: State<'_, AppState>, project_id: String) -> AppResult<ProjectDto> {
    state.project_queries.get(&project_id)
}

#[tauri::command]
pub fn detect_project(state: State<'_, AppState>, folder: String) -> AppResult<DetectionDto> {
    state.detect_project.execute(&folder)
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
pub fn open_terminal(state: State<'_, AppState>, project_id: String) -> AppResult<()> {
    use std::os::windows::process::CommandExt;
    const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;

    let project = state.project_queries.get(&project_id)?;
    let lampo = std::env::current_exe().map_err(|e| lampo_application::AppError::internal(e.to_string()))?.with_file_name("lampo.exe");
    std::process::Command::new(lampo)
        .args(["-p", &project.id, "exec", "--", "powershell.exe", "-NoLogo", "-NoExit"])
        .current_dir(&project.root)
        .creation_flags(CREATE_NEW_CONSOLE)
        .spawn()
        .map_err(|e| lampo_application::AppError::internal(format!("impossibile aprire il terminale: {e}")))?;
    Ok(())
}

/// Opens the project folder in Windows Explorer.
#[tauri::command]
pub fn open_project_folder(state: State<'_, AppState>, project_id: String) -> AppResult<()> {
    let project = state.project_queries.get(&project_id)?;
    super::external_apps::open_folder(&project.root)
}

/// Opens the project folder in Visual Studio Code.
#[tauri::command]
pub fn open_in_vscode(state: State<'_, AppState>, project_id: String) -> AppResult<()> {
    let project = state.project_queries.get(&project_id)?;
    super::external_apps::open_in_vscode(&project.root)
}

#[tauri::command]
pub fn export_project(state: State<'_, AppState>, request: ExportProjectRequest) -> AppResult<()> {
    state.export_project.execute(request)
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
