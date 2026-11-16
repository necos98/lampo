use lampo_application::dto::*;
use lampo_application::{AppResult, CallContext};
use tauri::State;

use super::blocking::run_blocking;
use super::tasks::spawn_task;
use crate::state::AppState;

#[tauri::command]
pub async fn list_worktrees(state: State<'_, AppState>, project_id: Option<String>) -> AppResult<Vec<WorktreeEnvironmentDto>> {
    let worktrees = state.worktree_queries.clone();
    run_blocking(move || worktrees.list(project_id.as_deref())).await
}

/// Background task (cloning a database can take a while); on success
/// `lampo://worktrees-changed` fires and `list_worktrees` returns the new environment.
#[tauri::command]
pub fn create_worktree(state: State<'_, AppState>, request: WorktreeUpRequest) -> TaskStartedDto {
    let up = state.worktree_up.clone();
    let operation = TaskOperationDto::WorktreeUp { request: request.clone() };
    spawn_task(&state, operation, move |progress, cancel| async move { up.execute(request, &CallContext::GUI, progress.as_ref(), &cancel).await })
}

#[tauri::command]
pub async fn remove_worktree(state: State<'_, AppState>, request: WorktreeDownRequest) -> AppResult<WorktreeDownResultDto> {
    state.worktree_down.execute(request, &CallContext::GUI).await
}

#[tauri::command]
pub async fn prune_worktrees(state: State<'_, AppState>, request: PruneWorktreesRequest) -> AppResult<PruneReportDto> {
    state.prune_worktrees.execute(request, &CallContext::GUI).await
}
