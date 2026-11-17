use lampo_application::dto::{AppInfoDto, StatusDto};
use lampo_application::AppResult;
use tauri::{AppHandle, State};

use crate::state::AppState;

#[tauri::command]
pub fn app_info(state: State<'_, AppState>) -> AppInfoDto {
    state.app_info.execute()
}

#[tauri::command]
pub async fn get_status(state: State<'_, AppState>) -> AppResult<StatusDto> {
    state.status.execute().await
}

/// Before an app update installs (`@tauri-apps/plugin-updater` `install()` starts the NSIS
/// installer and exits the process at once, skipping the tray's graceful quit): stops every
/// service cleanly and makes the control pipe refuse requests that could start them again. The
/// app keeps running; the installer closes it.
#[tauri::command]
pub async fn prepare_update(state: State<'_, AppState>) -> AppResult<()> {
    state.shutdown.run().await
}

/// The update could not start after [`prepare_update`]: accept requests again, show in the tray
/// the stops it made while the glyph could not change, and bring the services back to the
/// configured state.
#[tauri::command]
pub async fn cancel_update(app: AppHandle, state: State<'_, AppState>) -> AppResult<()> {
    state.shutdown.cancel();
    crate::tray::redraw(&app);
    state.startup.run().await
}

/// Stops services and exits (same as the tray's "Esci").
#[tauri::command]
pub fn quit_app(app: AppHandle) {
    crate::tray::quit(&app);
}
