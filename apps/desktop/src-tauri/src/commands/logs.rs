use lampo_application::dto::*;
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
