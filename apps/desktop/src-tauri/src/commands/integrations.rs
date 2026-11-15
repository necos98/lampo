//! Coding-agent integrations (Settings > Integrazioni). Each runs `claude` processes (seconds),
//! so they are async commands: the UI stays responsive and receives the step log as the result.

use lampo_application::dto::{ClaudeCodeActionDto, ClaudeCodeStatusDto};
use lampo_application::AppResult;
use tauri::State;

use crate::state::AppState;

#[tauri::command]
pub async fn claude_code_status(state: State<'_, AppState>) -> AppResult<ClaudeCodeStatusDto> {
    state.claude_code.status().await
}

/// Writes the plugin and registers it in Claude Code (user scope). Explicit user request only.
#[tauri::command]
pub async fn install_claude_code(state: State<'_, AppState>) -> AppResult<ClaudeCodeActionDto> {
    state.claude_code.install().await
}

#[tauri::command]
pub async fn uninstall_claude_code(state: State<'_, AppState>) -> AppResult<ClaudeCodeActionDto> {
    state.claude_code.uninstall().await
}
