use lampo_application::dto::*;
use lampo_application::use_cases::EnvironmentSync;
use lampo_application::{AppResult, CallContext};
use tauri::State;

use crate::state::AppState;

#[tauri::command]
pub fn list_services(state: State<'_, AppState>) -> AppResult<Vec<ServiceStatusDto>> {
    state.service_queries.list()
}

#[tauri::command]
pub async fn start_services(state: State<'_, AppState>, request: ServicesRequest) -> AppResult<SyncReportDto> {
    state.service_control.up(request, &CallContext::GUI).await
}

#[tauri::command]
pub async fn stop_services(state: State<'_, AppState>, request: ServicesRequest) -> AppResult<SyncReportDto> {
    state.service_control.down(request).await
}

#[tauri::command]
pub async fn restart_services(state: State<'_, AppState>, request: ServicesRequest) -> AppResult<SyncReportDto> {
    state.service_control.restart(request).await
}

/// Re-applies the whole configuration (e.g. after refusing a UAC prompt).
#[tauri::command]
pub async fn apply_environment(state: State<'_, AppState>) -> AppResult<SyncReportDto> {
    state.environment.sync(&CallContext::GUI).await
}
