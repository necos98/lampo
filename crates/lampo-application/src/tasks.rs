//! Bookkeeping for background tasks started by front-ends (package installs, first-run setup,
//! worktree creation with a cloned database). Front-ends spawn the work on their runtime; this
//! registry hands out ids and cancel flags, turns progress into events and remembers what is
//! still running (so a window created later can follow it, see [`TaskRegistry::running_tasks`]).

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use lampo_domain::events::{DomainEvent, TaskOutcome};
use lampo_domain::ports::{EventPublisher, ProgressSink};
use lampo_domain::progress::{CancelFlag, Progress, TaskId};

use crate::dto::{RunningTaskDto, TaskOperationDto};
use crate::error::{AppError, AppResult};

struct Running {
    operation: TaskOperationDto,
    cancel: CancelFlag,
    /// Order of `begin`, so listings are stable.
    sequence: u64,
    progress: Option<Progress>,
    subjects: BTreeMap<String, Progress>,
}

#[derive(Default)]
struct Inner {
    running: HashMap<TaskId, Running>,
    next: u64,
}

#[derive(Default)]
pub struct TaskRegistry {
    inner: Mutex<Inner>,
}

impl TaskRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Registers a new task doing `operation` and returns its id and cancel flag.
    pub fn begin(&self, operation: TaskOperationDto) -> (TaskId, CancelFlag) {
        let id = TaskId::new(uuid::Uuid::new_v4().to_string());
        let flag = CancelFlag::new();
        let mut inner = self.lock();
        inner.next += 1;
        let sequence = inner.next;
        inner.running.insert(id.clone(), Running { operation, cancel: flag.clone(), sequence, progress: None, subjects: BTreeMap::new() });
        (id, flag)
    }

    /// A sink that remembers the task's latest progress and publishes `TaskProgress` events.
    pub fn progress_sink(self: &Arc<Self>, task: TaskId, events: Arc<dyn EventPublisher>) -> Arc<dyn ProgressSink> {
        Arc::new(TaskProgressPublisher { task, events, registry: Some(self.clone()) })
    }

    fn record(&self, id: &TaskId, progress: &Progress) {
        if let Some(task) = self.lock().running.get_mut(id) {
            if let Some(subject) = &progress.subject {
                task.subjects.insert(subject.clone(), progress.clone());
            }
            task.progress = Some(progress.clone());
        }
    }

    /// Requests cancellation. `false` if the task is unknown or already finished.
    pub fn cancel(&self, id: &TaskId) -> bool {
        match self.lock().running.get(id) {
            Some(task) => {
                task.cancel.cancel();
                true
            }
            None => false,
        }
    }

    /// Forgets the task and publishes `TaskFinished` for `result`.
    pub fn finish<T>(&self, id: &TaskId, result: &AppResult<T>, events: &dyn EventPublisher) {
        self.lock().running.remove(id);
        let outcome = match result {
            Ok(_) => TaskOutcome::Succeeded,
            Err(AppError { code: crate::error::ErrorCode::Cancelled, .. }) => TaskOutcome::Cancelled,
            Err(e) => {
                TaskOutcome::Failed { code: serde_json::to_value(e.code).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_else(|| "internal".into()), message: e.message.clone() }
            }
        };
        events.publish(DomainEvent::TaskFinished { task: id.clone(), outcome });
    }

    pub fn running(&self) -> Vec<TaskId> {
        self.lock().running.keys().cloned().collect()
    }

    /// Every unfinished task with what it does and its latest progress, oldest first.
    pub fn running_tasks(&self) -> Vec<RunningTaskDto> {
        let inner = self.lock();
        let mut tasks: Vec<(&TaskId, &Running)> = inner.running.iter().collect();
        tasks.sort_by_key(|(_, t)| t.sequence);
        tasks
            .into_iter()
            .map(|(id, t)| RunningTaskDto {
                task_id: id.to_string(),
                operation: t.operation.clone(),
                progress: t.progress.clone().map(Into::into),
                subjects: t.subjects.values().cloned().map(Into::into).collect(),
            })
            .collect()
    }
}

/// A [`ProgressSink`] that publishes `DomainEvent::TaskProgress` for one task (and, when built by
/// [`TaskRegistry::progress_sink`], remembers it in the registry).
pub struct TaskProgressPublisher {
    task: TaskId,
    events: Arc<dyn EventPublisher>,
    registry: Option<Arc<TaskRegistry>>,
}

impl TaskProgressPublisher {
    pub fn new(task: TaskId, events: Arc<dyn EventPublisher>) -> Self {
        Self { task, events, registry: None }
    }
}

impl ProgressSink for TaskProgressPublisher {
    fn report(&self, progress: Progress) {
        if let Some(registry) = &self.registry {
            registry.record(&self.task, &progress);
        }
        self.events.publish(DomainEvent::TaskProgress { task: self.task.clone(), progress });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lampo_domain::ports::NullEventPublisher;
    use lampo_domain::progress::ProgressStage;

    use crate::dto::{InstallPackageRequest, PackageIdDto};

    #[test]
    fn begin_cancel_finish() {
        let reg = TaskRegistry::new();
        let (id, flag) = reg.begin(TaskOperationDto::FirstRun);
        assert!(reg.cancel(&id));
        assert!(flag.is_cancelled());
        reg.finish(&id, &AppResult::<()>::Ok(()), &NullEventPublisher);
        assert!(!reg.cancel(&id));
    }

    #[test]
    fn running_tasks_carry_their_operation_and_latest_progress() {
        let reg = Arc::new(TaskRegistry::new());
        let install = TaskOperationDto::InstallPackage { request: InstallPackageRequest { kind: "php".into(), series: Some("8.4".into()), ..Default::default() } };
        let (first, _) = reg.begin(install.clone());
        let (second, _) = reg.begin(TaskOperationDto::FirstRun);
        let sink = reg.progress_sink(second.clone(), Arc::new(NullEventPublisher));
        sink.report(Progress::new(ProgressStage::Downloading, 10, Some(100)).with_subject("apache"));
        sink.report(Progress::new(ProgressStage::Downloading, 5, Some(50)).with_subject("php"));

        let tasks = reg.running_tasks();
        assert_eq!(tasks.iter().map(|t| t.task_id.clone()).collect::<Vec<_>>(), [first.to_string(), second.to_string()]);
        assert_eq!(tasks[0].operation, install);
        assert!(tasks[0].progress.is_none());
        assert_eq!(tasks[1].progress.as_ref().unwrap().subject.as_deref(), Some("php"));
        assert_eq!(tasks[1].subjects.len(), 2);
        let json = serde_json::to_value(&tasks[0]).unwrap();
        assert_eq!(json["operation"]["kind"], "install-package");
        assert_eq!(json["operation"]["request"]["series"], "8.4");
        let uninstall = TaskOperationDto::UninstallPackage { id: PackageIdDto { kind: "php".into(), version: "8.3.15".into(), php_version: None } };
        let json = serde_json::to_value(&uninstall).unwrap();
        assert_eq!((&json["kind"], &json["id"]["version"], &json["id"]["phpVersion"]), (&"uninstall-package".into(), &"8.3.15".into(), &serde_json::Value::Null));
        assert_eq!(serde_json::from_value::<TaskOperationDto>(json).unwrap(), uninstall);

        reg.finish(&second, &AppResult::<()>::Ok(()), &NullEventPublisher);
        assert_eq!(reg.running_tasks().len(), 1);
    }
}
