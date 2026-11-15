use serde::{Deserialize, Serialize};

use super::project::ProjectSummaryDto;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ServiceStateDto {
    Stopped,
    Starting,
    Running,
    Stopping,
    Failed,
    /// The app is not running, so the state is not known (CLI offline status).
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceStatusDto {
    /// `apache`, `redis`, `mysql-8.4`.
    pub service: String,
    /// `apache`, `mysql`, `redis`.
    pub kind: String,
    /// Display label from the kind's descriptor (`MySQL 8.4`).
    pub label: String,
    pub state: ServiceStateDto,
    pub pid: Option<u32>,
    pub port: Option<u16>,
    pub message: Option<String>,
    /// Whether the current plan needs it running.
    pub required: bool,
    /// Site ids needing it; empty + `required` = needed by the system (always-on MySQL).
    pub required_by: Vec<String>,
}

/// Selects services; empty = every service in the current plan.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ServicesRequest {
    pub services: Vec<String>,
}

/// What an environment sync / up / down did.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncReportDto {
    pub started: Vec<String>,
    pub stopped: Vec<String>,
    pub restarted: Vec<String>,
    pub hosts_file_updated: bool,
    pub warnings: Vec<String>,
}

/// `lampo status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusDto {
    pub app_running: bool,
    pub services: Vec<ServiceStatusDto>,
    pub projects: Vec<ProjectSummaryDto>,
    pub worktree_count: usize,
}
