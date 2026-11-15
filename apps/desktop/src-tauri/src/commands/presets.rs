use lampo_application::dto::*;
use lampo_application::AppResult;
use tauri::State;

use crate::state::AppState;

#[tauri::command]
pub fn list_presets(state: State<'_, AppState>) -> AppResult<Vec<PresetDto>> {
    state.preset_queries.list()
}

#[tauri::command]
pub fn save_preset(state: State<'_, AppState>, request: SavePresetRequest) -> AppResult<PresetDto> {
    state.save_preset.execute(request)
}

#[tauri::command]
pub fn delete_preset(state: State<'_, AppState>, preset_id: String) -> AppResult<()> {
    state.delete_preset.execute(&preset_id)
}
