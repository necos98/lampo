//! Events published through [`crate::ports::EventPublisher`]. The desktop app forwards them to
//! the GUI (mapped to DTOs by `lampo-application::events`); other hosts may log or ignore them.

use serde::{Deserialize, Serialize};

use crate::progress::{Progress, TaskId};
use crate::project::ProjectId;
use crate::service::{LogStream, ServiceId, ServiceStatus};

/// How a background task ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum TaskOutcome {
    Succeeded,
    Cancelled,
    /// `code` is the application error code, `message` its English description.
    Failed {
        code: String,
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DomainEvent {
    /// A service changed state (published by runners, including on unexpected exit).
    ServiceStatusChanged(ServiceStatus),
    /// One line of a service's stdout/stderr. `at_unix_ms` is set by the publishing adapter.
    ServiceLog {
        service: ServiceId,
        stream: LogStream,
        line: String,
        at_unix_ms: u64,
    },
    TaskProgress {
        task: TaskId,
        progress: Progress,
    },
    TaskFinished {
        task: TaskId,
        outcome: TaskOutcome,
    },
    /// The project list or a project changed (created, updated, toggled, deleted, imported).
    ProjectsChanged {
        changed: Vec<ProjectId>,
    },
    /// Worktree environments of `project` were created or removed.
    WorktreesChanged {
        project: ProjectId,
    },
    /// Settings were saved.
    SettingsChanged,
    /// The installed package set changed.
    PackagesChanged,
}
