use lampo_application::dto::*;
use lampo_application::AppResult;
use tauri::State;

use super::blocking::run_blocking;
use crate::state::AppState;

#[tauri::command]
pub async fn list_presets(state: State<'_, AppState>) -> AppResult<Vec<PresetDto>> {
    let presets = state.preset_queries.clone();
    run_blocking(move || presets.list()).await
}

#[tauri::command]
pub async fn save_preset(state: State<'_, AppState>, request: SavePresetRequest) -> AppResult<PresetDto> {
    let save = state.save_preset.clone();
    run_blocking(move || save.execute(request)).await
}

#[tauri::command]
pub async fn delete_preset(state: State<'_, AppState>, preset_id: String) -> AppResult<()> {
    let delete = state.delete_preset.clone();
    run_blocking(move || delete.execute(&preset_id)).await
}
