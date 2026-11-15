use serde::{Deserialize, Serialize};

use super::database::DatabaseConnectionDto;

/// Database mode chosen when a worktree environment is created.
/// Wire form: `{ "mode": "shared" }`, `{ "mode": "fresh" }`, `{ "mode": "clone", "source": "shop" }`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "kebab-case")]
pub enum DatabaseModeDto {
    #[default]
    Shared,
    Fresh,
    Clone {
        source: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeEnvironmentDto {
    /// `shop@feature-login`.
    pub site: String,
    pub project_id: String,
    pub slug: String,
    /// Worktree folder.
    pub root: String,
    pub branch: Option<String>,
    /// Canonical URL (`http://127.0.0.1:8101`).
    pub url: String,
    /// Canonical URL first, then `http://<slug>.<project>.localhost[:port]`.
    pub urls: Vec<String>,
    pub loopback_port: u16,
    pub localhost_alias: String,
    pub document_root: String,
    pub database_mode: DatabaseModeDto,
    pub database: Option<DatabaseConnectionDto>,
    /// The folder no longer exists (prunable).
    pub stale: bool,
    pub created_at_unix_ms: u64,
}

/// `lampo worktree up [path]`: idempotent.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WorktreeUpRequest {
    /// Any folder inside the worktree.
    pub path: String,
    /// Parent project; default: resolved from the worktree's main repository.
    pub project_id: Option<String>,
    /// Ignored when the environment already exists.
    pub database: DatabaseModeDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeUpResultDto {
    pub environment: WorktreeEnvironmentDto,
    /// `false` when it already existed.
    pub created: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeDownRequest {
    /// Worktree folder or site id (`shop@feature-login`).
    pub target: String,
    /// Also drop the environment's own database (never the parent's).
    #[serde(default)]
    pub drop_database: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeDownResultDto {
    pub site: String,
    pub database_dropped: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PruneWorktreesRequest {
    /// Limit to one project.
    pub project_id: Option<String>,
    pub dry_run: bool,
    pub drop_databases: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PruneReportDto {
    pub dry_run: bool,
    /// Site ids removed (or that would be, with `dryRun`).
    pub removed: Vec<String>,
    pub dropped_databases: Vec<String>,
}
