use lampo_application::dto::*;
use lampo_application::AppResult;
use tauri::State;

use crate::state::AppState;

#[tauri::command]
pub fn list_log_sources(state: State<'_, AppState>, site: Option<String>) -> AppResult<Vec<LogSourceDto>> {
    state.logs.sources(site.as_deref())
}

#[tauri::command]
pub fn tail_log(state: State<'_, AppState>, request: LogTailRequest) -> AppResult<LogLinesDto> {
    state.logs.tail(&request)
}

#[tauri::command]
pub fn read_log_from(state: State<'_, AppState>, source: String, offset: u64) -> AppResult<LogLinesDto> {
    state.logs.read_from(&source, offset)
}
