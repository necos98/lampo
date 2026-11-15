use std::future::Future;
use std::sync::Arc;

use lampo_application::dto::{RunningTaskDto, TaskOperationDto, TaskStartedDto};
use lampo_application::AppResult;
use lampo_domain::ports::ProgressSink;
use lampo_domain::progress::{CancelFlag, TaskId};
use tauri::State;

use crate::state::AppState;

/// Runs `work` as a background task doing `operation`: registers it, publishes progress and
/// `task-finished`.
pub fn spawn_task<F, Fut, T>(state: &AppState, operation: TaskOperationDto, work: F) -> TaskStartedDto
where
    F: FnOnce(Arc<dyn ProgressSink>, CancelFlag) -> Fut + Send + 'static,
    Fut: Future<Output = AppResult<T>> + Send + 'static,
    T: Send + 'static,
{
    let (id, cancel) = state.tasks.begin(operation);
    let progress = state.tasks.progress_sink(id.clone(), state.events.clone());
    let tasks = state.tasks.clone();
    let events = state.events.clone();
    let task_id = id.clone();
    tauri::async_runtime::spawn(async move {
        let result = work(progress, cancel).await;
        tasks.finish(&task_id, &result, events.as_ref());
    });
    TaskStartedDto { task_id: id.to_string() }
}

/// `true` if the task was running and is now asked to stop.
#[tauri::command]
pub fn cancel_task(state: State<'_, AppState>, task_id: String) -> bool {
    state.tasks.cancel(&TaskId::new(task_id))
}

/// Tasks still running (e.g. a download started before the window was closed and reopened).
#[tauri::command]
pub fn list_running_tasks(state: State<'_, AppState>) -> Vec<RunningTaskDto> {
    state.tasks.running_tasks()
}
