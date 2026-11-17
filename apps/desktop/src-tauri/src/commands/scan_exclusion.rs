//! The optional Microsoft Defender exclusions (Settings): the MySQL data folder, the PHP/Apache
//! server processes and a projects folder, and their PowerShell commands for whoever manages the
//! PC. Each change shows one UAC prompt owned by the app window; only an explicit click asks for
//! it. After a PHP is installed or uninstalled, the change to propose.

use lampo_application::dto::{DefenderExclusionCommandDto, DefenderExclusionInfoDto, DefenderExclusionRequestDto, DefenderExclusionResultDto, DefenderExclusionSuggestionDto};
use lampo_application::{AppResult, CallContext};
use tauri::State;

use crate::state::AppState;

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

/// The exclusion changes to propose after a PHP was installed or uninstalled: reads Defender's
/// state (a few seconds at most), so async.
#[tauri::command]
pub async fn defender_exclusion_suggestions(state: State<'_, AppState>) -> AppResult<Vec<DefenderExclusionSuggestionDto>> {
    Ok(state.defender_advice.suggestions().await)
}

/// The PowerShell commands of `request`, for whoever manages the PC when Defender refuses the
/// change: no prompt, nothing changed.
#[tauri::command]
pub fn defender_exclusion_commands(state: State<'_, AppState>, request: DefenderExclusionRequestDto) -> AppResult<Vec<DefenderExclusionCommandDto>> {
    state.scan_exclusion.commands(request)
}
