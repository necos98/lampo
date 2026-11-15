use lampo_application::dto::{ServiceKindDto, SettingsDto};
use lampo_application::{AppResult, CallContext};
use tauri::State;

use crate::state::AppState;

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> AppResult<SettingsDto> {
    state.settings.get()
}

#[tauri::command]
pub async fn update_settings(state: State<'_, AppState>, settings: SettingsDto) -> AppResult<SettingsDto> {
    state.settings.update(settings, &CallContext::GUI).await
}

#[tauri::command]
pub fn list_service_kinds(state: State<'_, AppState>) -> AppResult<Vec<ServiceKindDto>> {
    state.settings.service_kinds()
}
