use lampo_application::dto::*;
use lampo_application::use_cases::LogTarget;
use lampo_application::AppResult;
use tauri::State;

use super::blocking::run_blocking;
use crate::state::AppState;

#[tauri::command]
pub async fn list_log_sources(state: State<'_, AppState>, site: Option<String>) -> AppResult<Vec<LogSourceDto>> {
    let logs = state.logs.clone();
    run_blocking(move || logs.sources(site.as_deref())).await
}

#[tauri::command]
pub async fn tail_log(state: State<'_, AppState>, request: LogTailRequest) -> AppResult<LogLinesDto> {
    let logs = state.logs.clone();
    run_blocking(move || logs.tail(&request)).await
}

#[tauri::command]
pub async fn read_log_from(state: State<'_, AppState>, source: String, offset: u64) -> AppResult<LogLinesDto> {
    let logs = state.logs.clone();
    run_blocking(move || logs.read_from(&source, offset)).await
}

/// Opens the file of a log source in the app Windows associates with it.
#[tauri::command]
pub async fn open_log_file(state: State<'_, AppState>, source: String) -> AppResult<()> {
    open_log(&state, source, LogTarget::File).await
}

/// Opens the folder of a log source in Windows Explorer.
#[tauri::command]
pub async fn open_log_folder(state: State<'_, AppState>, source: String) -> AppResult<()> {
    open_log(&state, source, LogTarget::Folder).await
}

async fn open_log(state: &AppState, source: String, target: LogTarget) -> AppResult<()> {
    let open = state.open_log.clone();
    run_blocking(move || open.execute(&source, target)).await
}

/// Saves the lines the viewer shows to a file the user picked.
#[tauri::command]
pub async fn export_log_lines(state: State<'_, AppState>, request: LogExportRequest) -> AppResult<()> {
    let export = state.export_log_lines.clone();
    run_blocking(move || export.execute(&request)).await
}
