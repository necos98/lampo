//! DTOs for agents: site resolution, `lampo agent context`, `lampo env`, `lampo exec`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::database::{DatabaseConnectionDto, MailConnectionDto};
use super::logs::LogSourceDto;

/// Bumped on incompatible changes to [`AgentContextDto`].
pub const AGENT_CONTEXT_SCHEMA_VERSION: u32 = 2;

/// Inputs of site resolution, captured by the front-end from its process.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ResolveRequestDto {
    pub cwd: String,
    /// Value of `LAMPO_PROJECT` (`shop` or `shop@fix-1`).
    pub explicit_site: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ResolvedKind {
    Project,
    /// A worktree environment exists for this folder.
    WorktreeEnvironment,
    /// Inside a git worktree of a project, but no environment is up (parent's data is used).
    WorktreeWithoutEnvironment,
    GlobalDefault,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedSiteDto {
    pub kind: ResolvedKind,
    /// `explicit`, `directory`, `git-worktree`, `global-default`.
    pub via: String,
    pub site: Option<String>,
    pub project_id: Option<String>,
    /// Folder of the matched site.
    pub root: Option<String>,
    /// For git worktrees: the worktree folder and the main repository folder.
    pub worktree_root: Option<String>,
    pub main_root: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPhpDto {
    pub version: String,
    /// Absolute path to `php.exe`.
    pub binary: String,
    pub ini_dir: Option<String>,
    pub composer_phar: Option<String>,
    /// `lts22` or `stable`.
    pub composer_channel: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceEndpointDto {
    /// `mysql-8.4`, `redis`.
    pub service: String,
    pub kind: String,
    /// `MySQL 8.4`.
    pub label: String,
    pub host: String,
    pub port: u16,
    /// Web UI of kinds that have one (Mailpit: `http://127.0.0.1:8025`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentWorktreeDto {
    pub worktree_root: String,
    pub branch: Option<String>,
    /// Whether a worktree environment is up for this folder.
    pub environment_up: bool,
    pub site: Option<String>,
}

/// One CLI command as advertised to agents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CliCommandDto {
    /// `lampo worktree up`.
    pub name: String,
    pub usage: String,
    pub description: String,
    /// Changes state (goes through the app, which is started headless if needed).
    pub mutating: bool,
    pub requires_app: bool,
}

/// `lampo agent context --json`: everything an agent needs in one call. Works without the app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentContextDto {
    pub schema_version: u32,
    pub app_running: bool,
    pub resolution: ResolvedSiteDto,
    pub project_id: Option<String>,
    pub project_name: Option<String>,
    /// Folder the site is served from (worktree folder for worktree environments).
    pub root: Option<String>,
    pub worktree: Option<AgentWorktreeDto>,
    /// Canonical URL of the resolved site (`null` for the global default).
    pub url: Option<String>,
    pub urls: Vec<String>,
    pub document_root: Option<String>,
    pub php: AgentPhpDto,
    pub database: Option<DatabaseConnectionDto>,
    /// SMTP endpoint and web UI of Mailpit, when the site uses it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mail: Option<MailConnectionDto>,
    /// Every service the site uses (version 2; version 1 had only `redis`).
    pub services: Vec<ServiceEndpointDto>,
    pub logs: Vec<LogSourceDto>,
    pub commands: Vec<CliCommandDto>,
    /// Actionable English hints (`no worktree environment is up: run lampo worktree up`).
    pub hints: Vec<String>,
}

/// `lampo env`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolchainEnvDto {
    pub site: Option<String>,
    /// Directories to prepend to PATH, in order.
    pub path_prepend: Vec<String>,
    /// `PHPRC` and `LAMPO_*` only.
    pub vars: BTreeMap<String, String>,
}

/// `lampo exec -- <cmd> [args…]`: what the CLI must spawn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecPlanDto {
    pub program: String,
    pub args: Vec<String>,
    pub env: ToolchainEnvDto,
}
