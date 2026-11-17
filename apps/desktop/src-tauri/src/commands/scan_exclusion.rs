//! The optional Microsoft Defender exclusion of the MySQL data folder (Settings > Strumenti).
//! Each change shows a UAC prompt owned by the app window; only an explicit click asks for it.

use lampo_application::dto::{ScanExclusionChangeDto, ScanExclusionInfoDto, ScanExclusionResultDto};
use lampo_application::{AppResult, CallContext};
use tauri::State;

use crate::state::AppState;

#[tauri::command]
pub fn mysql_scan_exclusion_info(state: State<'_, AppState>) -> ScanExclusionInfoDto {
    state.scan_exclusion.info()
}

/// Waits for the UAC prompt and Defender: async, so the UI stays responsive.
#[tauri::command]
pub async fn change_mysql_scan_exclusion(state: State<'_, AppState>, change: ScanExclusionChangeDto) -> AppResult<ScanExclusionResultDto> {
    state.scan_exclusion.change(change, CallContext::GUI).await
}
