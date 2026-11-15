use lampo_application::dto::{DoctorReportDto, ShimStatusDto};
use lampo_application::AppResult;
use tauri::State;

use crate::state::AppState;

#[tauri::command]
pub async fn run_doctor(state: State<'_, AppState>) -> AppResult<DoctorReportDto> {
    state.doctor.run().await
}

#[tauri::command]
pub fn shim_status(state: State<'_, AppState>) -> AppResult<ShimStatusDto> {
    state.shims.status()
}

#[tauri::command]
pub fn install_shims(state: State<'_, AppState>) -> AppResult<ShimStatusDto> {
    state.shims.install()
}
