use lampo_application::dto::{DoctorReportDto, ShimStatusDto};
use lampo_application::AppResult;
use tauri::State;

use super::blocking::run_blocking;
use crate::state::AppState;

#[tauri::command]
pub async fn run_doctor(state: State<'_, AppState>) -> AppResult<DoctorReportDto> {
    state.doctor.run().await
}

/// Only the hosts-file comparison of the doctor: what the GUI re-checks after project changes.
#[tauri::command]
pub async fn hosts_in_sync(state: State<'_, AppState>) -> AppResult<bool> {
    let hosts_sync = state.hosts_sync.clone();
    run_blocking(move || hosts_sync.in_sync()).await
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
