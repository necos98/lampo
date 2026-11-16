//! Events pushed to front-ends: the **single source of truth for event names** and payloads.
//! Mirrored in `apps/desktop/src/lib/api/events.ts`.

use lampo_domain::auto_restart::AutoRestartStep;
use lampo_domain::events::{DomainEvent, TaskOutcome};
use lampo_domain::progress::{Progress, ProgressStage};
use lampo_domain::service::{LogStream, ServiceState, ServiceStatus};
use lampo_domain::tcp_port::Port;
use serde::{Deserialize, Serialize};

use crate::dto::ServiceStateDto;
use crate::error::{AppError, ErrorCode};

/// Event names (Tauri event channel names; also used as `event` in control-pipe frames).
pub mod names {
    pub const TASK_PROGRESS: &str = "lampo://task-progress";
    pub const TASK_FINISHED: &str = "lampo://task-finished";
    pub const SERVICE_STATUS: &str = "lampo://service-status";
    pub const SERVICE_LOG: &str = "lampo://service-log";
    pub const PROJECTS_CHANGED: &str = "lampo://projects-changed";
    pub const WORKTREES_CHANGED: &str = "lampo://worktrees-changed";
    pub const SETTINGS_CHANGED: &str = "lampo://settings-changed";
    pub const PACKAGES_CHANGED: &str = "lampo://packages-changed";
    pub const SERVICE_AUTO_RESTART: &str = "lampo://service-auto-restart";

    pub const ALL: &[&str] = &[TASK_PROGRESS, TASK_FINISHED, SERVICE_STATUS, SERVICE_LOG, PROJECTS_CHANGED, WORKTREES_CHANGED, SETTINGS_CHANGED, PACKAGES_CHANGED, SERVICE_AUTO_RESTART];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "ProgressStage"))]
#[serde(rename_all = "kebab-case")]
pub enum ProgressStageDto {
    Resolving,
    Downloading,
    Verifying,
    Extracting,
    Configuring,
    Initializing,
    Running,
}

impl From<ProgressStage> for ProgressStageDto {
    fn from(s: ProgressStage) -> Self {
        match s {
            ProgressStage::Resolving => Self::Resolving,
            ProgressStage::Downloading => Self::Downloading,
            ProgressStage::Verifying => Self::Verifying,
            ProgressStage::Extracting => Self::Extracting,
            ProgressStage::Configuring => Self::Configuring,
            ProgressStage::Initializing => Self::Initializing,
            ProgressStage::Running => Self::Running,
        }
    }
}

/// Progress of a long operation (bytes while downloading/extracting, steps otherwise).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "ProgressReport"))]
#[serde(rename_all = "camelCase")]
pub struct ProgressDto {
    pub stage: ProgressStageDto,
    pub done: u64,
    pub total: Option<u64>,
    pub subject: Option<String>,
    /// One output line of a running program (a log entry), when the report carries one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl From<Progress> for ProgressDto {
    fn from(p: Progress) -> Self {
        Self { stage: p.stage.into(), done: p.done, total: p.total, subject: p.subject, message: p.message }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct TaskProgressPayload {
    pub task_id: String,
    #[serde(flatten)]
    pub progress: ProgressDto,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "TaskOutcome"))]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum TaskOutcomeDto {
    Succeeded,
    Cancelled,
    Failed { error: AppError },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct TaskFinishedPayload {
    pub task_id: String,
    pub outcome: TaskOutcomeDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ServiceStatusPayload {
    pub service: String,
    pub kind: String,
    pub state: ServiceStateDto,
    pub pid: Option<u32>,
    pub port: Option<u16>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "LogStream"))]
#[serde(rename_all = "lowercase")]
pub enum LogStreamDto {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ServiceLogPayload {
    pub service: String,
    pub stream: LogStreamDto,
    pub line: String,
    pub at_unix_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ProjectsChangedPayload {
    pub project_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct WorktreesChangedPayload {
    pub project_id: String,
}

/// One step of the automatic restart of a crashed service:
/// `{ "type": "scheduled", "attempt": 1, "maxAttempts": 3, "delayMs": 2000 }`,
/// `{ "type": "restarted", "attempt": 1 }`, `{ "type": "gave-up", "attempts": 3, "message": "..." }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "AutoRestartStep"))]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum AutoRestartStepDto {
    /// The service exited unexpectedly: attempt `attempt` of `max_attempts` in `delay_ms`.
    #[serde(rename_all = "camelCase")]
    Scheduled { attempt: u32, max_attempts: u32, delay_ms: u64 },
    /// Attempt `attempt` got the service running again.
    Restarted { attempt: u32 },
    /// Every attempt failed: the service stays failed; `message` is the last failure.
    GaveUp { attempts: u32, message: String },
}

impl From<AutoRestartStep> for AutoRestartStepDto {
    fn from(step: AutoRestartStep) -> Self {
        match step {
            AutoRestartStep::Scheduled { attempt, max_attempts, delay } => Self::Scheduled { attempt, max_attempts, delay_ms: u64::try_from(delay.as_millis()).unwrap_or(u64::MAX) },
            AutoRestartStep::Restarted { attempt } => Self::Restarted { attempt },
            AutoRestartStep::GaveUp { attempts, message } => Self::GaveUp { attempts, message },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ServiceAutoRestartPayload {
    pub service: String,
    pub step: AutoRestartStepDto,
}

/// A front-end event: name + JSON payload.
#[derive(Debug, Clone, PartialEq)]
pub enum AppEvent {
    TaskProgress(TaskProgressPayload),
    TaskFinished(TaskFinishedPayload),
    ServiceStatus(ServiceStatusPayload),
    ServiceLog(ServiceLogPayload),
    ProjectsChanged(ProjectsChangedPayload),
    WorktreesChanged(WorktreesChangedPayload),
    SettingsChanged,
    PackagesChanged,
    ServiceAutoRestart(ServiceAutoRestartPayload),
}

impl AppEvent {
    pub fn name(&self) -> &'static str {
        match self {
            Self::TaskProgress(_) => names::TASK_PROGRESS,
            Self::TaskFinished(_) => names::TASK_FINISHED,
            Self::ServiceStatus(_) => names::SERVICE_STATUS,
            Self::ServiceLog(_) => names::SERVICE_LOG,
            Self::ProjectsChanged(_) => names::PROJECTS_CHANGED,
            Self::WorktreesChanged(_) => names::WORKTREES_CHANGED,
            Self::SettingsChanged => names::SETTINGS_CHANGED,
            Self::PackagesChanged => names::PACKAGES_CHANGED,
            Self::ServiceAutoRestart(_) => names::SERVICE_AUTO_RESTART,
        }
    }

    /// JSON payload (`null` for payload-less events).
    pub fn payload(&self) -> serde_json::Value {
        let v = match self {
            Self::TaskProgress(p) => serde_json::to_value(p),
            Self::TaskFinished(p) => serde_json::to_value(p),
            Self::ServiceStatus(p) => serde_json::to_value(p),
            Self::ServiceLog(p) => serde_json::to_value(p),
            Self::ProjectsChanged(p) => serde_json::to_value(p),
            Self::WorktreesChanged(p) => serde_json::to_value(p),
            Self::ServiceAutoRestart(p) => serde_json::to_value(p),
            Self::SettingsChanged | Self::PackagesChanged => Ok(serde_json::Value::Null),
        };
        v.unwrap_or(serde_json::Value::Null)
    }
}

pub(crate) fn service_status_payload(s: &ServiceStatus) -> ServiceStatusPayload {
    ServiceStatusPayload { service: s.service.to_string(), kind: s.service.kind().to_string(), state: s.state.into(), pid: s.pid, port: s.port.map(Port::get), message: s.message.clone() }
}

impl From<ServiceState> for ServiceStateDto {
    fn from(s: ServiceState) -> Self {
        match s {
            ServiceState::Stopped => Self::Stopped,
            ServiceState::Starting => Self::Starting,
            ServiceState::Running => Self::Running,
            ServiceState::Stopping => Self::Stopping,
            ServiceState::Failed => Self::Failed,
        }
    }
}

impl AppEvent {
    /// The front-end event of a domain event; `None` for the ones only the backend reacts to (a
    /// crash: the status change and the automatic restart steps are what the GUI shows).
    pub fn from_domain(e: DomainEvent) -> Option<Self> {
        Some(match e {
            DomainEvent::ServiceStatusChanged(s) => Self::ServiceStatus(service_status_payload(&s)),
            DomainEvent::ServiceCrashed { .. } => return None,
            DomainEvent::ServiceAutoRestart { service, step } => Self::ServiceAutoRestart(ServiceAutoRestartPayload { service: service.to_string(), step: step.into() }),
            DomainEvent::ServiceLog { service, stream, line, at_unix_ms } => Self::ServiceLog(ServiceLogPayload {
                service: service.to_string(),
                stream: match stream {
                    LogStream::Stdout => LogStreamDto::Stdout,
                    LogStream::Stderr => LogStreamDto::Stderr,
                },
                line,
                at_unix_ms,
            }),
            DomainEvent::TaskProgress { task, progress } => Self::TaskProgress(TaskProgressPayload { task_id: task.to_string(), progress: progress.into() }),
            DomainEvent::TaskFinished { task, outcome } => Self::TaskFinished(TaskFinishedPayload {
                task_id: task.to_string(),
                outcome: match outcome {
                    TaskOutcome::Succeeded => TaskOutcomeDto::Succeeded,
                    TaskOutcome::Cancelled => TaskOutcomeDto::Cancelled,
                    TaskOutcome::Failed { code, message } => {
                        TaskOutcomeDto::Failed { error: AppError::new(serde_json::from_value(serde_json::Value::String(code)).unwrap_or(ErrorCode::Internal), message) }
                    }
                },
            }),
            DomainEvent::ProjectsChanged { changed } => Self::ProjectsChanged(ProjectsChangedPayload { project_ids: changed.into_iter().map(|p| p.to_string()).collect() }),
            DomainEvent::WorktreesChanged { project } => Self::WorktreesChanged(WorktreesChangedPayload { project_id: project.to_string() }),
            DomainEvent::SettingsChanged => Self::SettingsChanged,
            DomainEvent::PackagesChanged => Self::PackagesChanged,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lampo_domain::progress::TaskId;

    #[test]
    fn task_progress_is_flat_camel_case() {
        let ev = AppEvent::from_domain(DomainEvent::TaskProgress { task: TaskId::new("t1"), progress: Progress::new(ProgressStage::Downloading, 5, Some(10)) }).unwrap();
        assert_eq!(ev.name(), names::TASK_PROGRESS);
        let json = ev.payload();
        assert_eq!(json["taskId"], "t1");
        assert_eq!(json["stage"], "downloading");
        assert_eq!(json["total"], 10);
    }

    #[test]
    fn failed_task_keeps_error_code() {
        let ev = AppEvent::from_domain(DomainEvent::TaskFinished { task: TaskId::new("t"), outcome: TaskOutcome::Failed { code: "network".into(), message: "x".into() } }).unwrap();
        assert_eq!(ev.payload()["outcome"]["error"]["code"], "network");
    }

    #[test]
    fn auto_restart_steps_are_tagged_camel_case() {
        let service: lampo_domain::service::ServiceId = "redis".parse().unwrap();
        let step = AutoRestartStep::Scheduled { attempt: 2, max_attempts: 3, delay: std::time::Duration::from_secs(4) };
        let ev = AppEvent::from_domain(DomainEvent::ServiceAutoRestart { service: service.clone(), step }).unwrap();
        assert_eq!(ev.name(), names::SERVICE_AUTO_RESTART);
        assert_eq!(ev.payload(), serde_json::json!({"service": "redis", "step": {"type": "scheduled", "attempt": 2, "maxAttempts": 3, "delayMs": 4000}}));
        let gave_up = AppEvent::from_domain(DomainEvent::ServiceAutoRestart { service, step: AutoRestartStep::GaveUp { attempts: 3, message: "boom".into() } }).unwrap();
        assert_eq!(gave_up.payload()["step"], serde_json::json!({"type": "gave-up", "attempts": 3, "message": "boom"}));
    }

    #[test]
    fn crashes_stay_in_the_backend() {
        assert_eq!(AppEvent::from_domain(DomainEvent::ServiceCrashed { service: "redis".parse().unwrap(), message: "exited".into() }), None);
    }
}
