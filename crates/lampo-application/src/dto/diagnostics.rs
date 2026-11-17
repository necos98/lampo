use serde::{Deserialize, Serialize};

use super::project::UnreadableProjectDto;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum CheckStatus {
    Ok,
    Warning,
    Error,
}

/// One `doctor` check. `id` is stable (`settings`, `project-files`, `vc-runtime`, `ports`,
/// `path-shadowing`, `shims-on-path`, `hosts-block`, `packages`, `services`, `mysql-auth`, `defender`,
/// `stale-worktrees`, `folders`, `app`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "DoctorCheck"))]
#[serde(rename_all = "camelCase")]
pub struct DoctorCheckDto {
    pub id: String,
    pub status: CheckStatus,
    pub message: String,
    /// Suggested fix (command or action), English.
    pub fix: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "VcRuntimeIssue"))]
#[serde(rename_all = "camelCase")]
pub struct VcRuntimeIssueDto {
    pub redistributable: String,
    pub download_url: String,
    /// Package keys needing it (`php-8.4.1`).
    pub required_by: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "PortConflict"))]
#[serde(rename_all = "camelCase")]
pub struct PortConflictDto {
    pub port: u16,
    /// Service or site wanting the port.
    pub wanted_by: String,
    pub owner: Option<String>,
}

/// A tool whose Lampo shim is shadowed by another executable earlier on PATH.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "ShadowedTool"))]
#[serde(rename_all = "camelCase")]
pub struct ShadowedToolDto {
    /// `php`, `composer`, `mysql`, `mysqldump`, `redis-cli`.
    pub tool: String,
    pub shim: String,
    pub shadowed_by: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "DoctorReport"))]
#[serde(rename_all = "camelCase")]
pub struct DoctorReportDto {
    /// No check has status `error`.
    pub ok: bool,
    pub checks: Vec<DoctorCheckDto>,
    pub missing_vc_runtimes: Vec<VcRuntimeIssueDto>,
    pub port_conflicts: Vec<PortConflictDto>,
    pub shadowed_tools: Vec<ShadowedToolDto>,
    /// Site ids of worktree environments whose folder is gone.
    pub stale_worktrees: Vec<String>,
    /// Package requirements not satisfied (`php 8.2`, `mysql 5.7`).
    pub missing_packages: Vec<String>,
    /// Services the plan needs that are `failed` or `stopped` (`apache`, `mysql-8.4`).
    pub services_down: Vec<String>,
    pub hosts_file_in_sync: bool,
    /// Project files that do not load, with the reason (`project-files`).
    #[serde(default)]
    pub unreadable_projects: Vec<UnreadableProjectDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "ShimStatus"))]
#[serde(rename_all = "camelCase")]
pub struct ShimStatusDto {
    pub shim_dir: String,
    pub installed: bool,
    pub on_path: bool,
    pub shadowed: Vec<ShadowedToolDto>,
}
