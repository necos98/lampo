use lampo_application::dto::{DoctorReportDto, ShimStatusDto};
use lampo_application::AppResult;
use tauri::State;

use super::blocking::run_blocking;
use crate::state::AppState;

#[tauri::command]
pub async fn run_doctor(state: State<'_, AppState>) -> AppResult<DoctorReportDto> {
    state.doctor.run().await
}

#[tauri::command]
pub async fn shim_status(state: State<'_, AppState>) -> AppResult<ShimStatusDto> {
    let shims = state.shims.clone();
    run_blocking(move || shims.status()).await
}

/// Copies the shim executables and broadcasts the PATH change (`WM_SETTINGCHANGE`, up to a few
/// seconds while other windows answer).
#[tauri::command]
pub async fn install_shims(state: State<'_, AppState>) -> AppResult<ShimStatusDto> {
    let shims = state.shims.clone();
    run_blocking(move || shims.install()).await
}
