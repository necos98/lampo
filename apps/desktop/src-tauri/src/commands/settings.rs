use lampo_application::dto::{ServiceKindDto, SettingsDto};
use lampo_application::{AppResult, CallContext};
use tauri::State;

use super::blocking::run_blocking;
use crate::state::AppState;

#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> AppResult<SettingsDto> {
    let settings = state.settings.clone();
    run_blocking(move || settings.get()).await
}

#[tauri::command]
pub async fn update_settings(state: State<'_, AppState>, settings: SettingsDto) -> AppResult<SettingsDto> {
    state.settings.update(settings, &CallContext::GUI).await
}

#[tauri::command]
pub async fn list_service_kinds(state: State<'_, AppState>) -> AppResult<Vec<ServiceKindDto>> {
    let settings = state.settings.clone();
    run_blocking(move || settings.service_kinds()).await
}
