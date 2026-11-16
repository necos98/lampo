use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;

use futures_util::FutureExt;
use lampo_application::dto::{RunningTaskDto, TaskOperationDto, TaskStartedDto};
use lampo_application::tasks::TaskRegistry;
use lampo_application::AppResult;
use lampo_domain::ports::{EventPublisher, ProgressSink};
use lampo_domain::progress::{CancelFlag, TaskId};
use tauri::State;

use super::blocking::panic_error;
use crate::state::AppState;

/// Runs `work` as a background task doing `operation`: registers it, publishes progress and
/// `task-finished`. A panic of `work` ends the task as failed (`AppError::internal`) instead of
/// leaving it "running" forever.
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
    // `work` is called inside the future so that a panic while building it is caught as well.
    tauri::async_runtime::spawn(run_to_finish(tasks, events, task_id, async move { work(progress, cancel).await }));
    TaskStartedDto { task_id: id.to_string() }
}

/// Awaits `work` and always closes the task `id`, also when `work` panics.
async fn run_to_finish<Fut, T>(tasks: Arc<TaskRegistry>, events: Arc<dyn EventPublisher>, id: TaskId, work: Fut)
where
    Fut: Future<Output = AppResult<T>>,
{
    let result = AssertUnwindSafe(work).catch_unwind().await.unwrap_or_else(|payload| Err(panic_error(payload.as_ref())));
    tasks.finish(&id, &result, events.as_ref());
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

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use lampo_domain::events::{DomainEvent, TaskOutcome};

    use super::*;

    #[derive(Default)]
    struct RecordingEvents(Mutex<Vec<DomainEvent>>);

    impl EventPublisher for RecordingEvents {
        fn publish(&self, event: DomainEvent) {
            self.0.lock().unwrap().push(event);
        }
    }

    fn finished_outcome(events: &RecordingEvents, id: &TaskId) -> Option<TaskOutcome> {
        events.0.lock().unwrap().iter().find_map(|event| match event {
            DomainEvent::TaskFinished { task, outcome } if task == id => Some(outcome.clone()),
            _ => None,
        })
    }

    fn run(work: impl Future<Output = AppResult<()>>) -> (Arc<TaskRegistry>, Arc<RecordingEvents>, TaskId) {
        let tasks = Arc::new(TaskRegistry::new());
        let events = Arc::new(RecordingEvents::default());
        let (id, _cancel) = tasks.begin(TaskOperationDto::FirstRun);
        tauri::async_runtime::block_on(run_to_finish(tasks.clone(), events.clone(), id.clone(), work));
        (tasks, events, id)
    }

    #[test]
    fn a_completed_task_is_closed_as_succeeded() {
        let (tasks, events, id) = run(async { Ok(()) });
        assert!(tasks.running().is_empty());
        assert_eq!(finished_outcome(&events, &id), Some(TaskOutcome::Succeeded));
    }

    #[test]
    fn a_panicking_task_is_closed_as_failed_with_an_internal_error() {
        let (tasks, events, id) = run(async { panic!("download corrotto") });
        assert!(tasks.running().is_empty(), "the task must not stay running");
        match finished_outcome(&events, &id) {
            Some(TaskOutcome::Failed { code, message }) => {
                assert_eq!(code, "internal");
                assert!(message.contains("download corrotto"), "{message}");
            }
            other => panic!("unexpected outcome {other:?}"),
        }
    }
}
