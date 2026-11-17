//! The optional Microsoft Defender exclusions (Settings): the MySQL data folder and the PHP/Apache
//! server processes. Each change shows one UAC prompt owned by the app window; only an explicit
//! click asks for it.

use lampo_application::dto::{DefenderExclusionInfoDto, DefenderExclusionRequestDto, DefenderExclusionResultDto, ScanExclusionChangeDto, ScanExclusionInfoDto, ScanExclusionResultDto};
use lampo_application::{AppResult, CallContext};
use tauri::State;

use crate::state::AppState;

#[tauri::command]
pub fn mysql_scan_exclusion_info(state: State<'_, AppState>) -> ScanExclusionInfoDto {
    state.scan_exclusion.mysql_info()
}

/// Waits for the UAC prompt and Defender: async, so the UI stays responsive.
#[tauri::command]
pub async fn change_mysql_scan_exclusion(state: State<'_, AppState>, change: ScanExclusionChangeDto) -> AppResult<ScanExclusionResultDto> {
    state.scan_exclusion.change_mysql(change, CallContext::GUI).await
}

/// What `php` and `mysql` would cover now (the installed `php-cgi.exe`/`httpd.exe`).
#[tauri::command]
pub fn defender_exclusion_info(state: State<'_, AppState>) -> AppResult<DefenderExclusionInfoDto> {
    state.scan_exclusion.info()
}

/// Waits for the UAC prompt and Defender: async, so the UI stays responsive.
#[tauri::command]
pub async fn change_defender_exclusion(state: State<'_, AppState>, request: DefenderExclusionRequestDto) -> AppResult<DefenderExclusionResultDto> {
    state.scan_exclusion.change(request, CallContext::GUI).await
}
