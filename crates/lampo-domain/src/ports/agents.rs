//! Coding-agent integrations: the agent's own CLI and the files Lampo generates for it.

use std::path::Path;
use std::time::Duration;

use async_trait::async_trait;

use crate::error::PortResult;
use crate::ports::services::CommandOutput;

/// The `claude` CLI of Claude Code (native installer or npm shim), run hidden. Implemented by
/// `lampo-infra-services` (`ClaudeCliRunner`).
#[async_trait]
pub trait ClaudeCodeCli: Send + Sync {
    /// The first line of `claude --version`, or `None` when Claude Code is not installed or
    /// does not answer in time. Never an error for "not installed".
    async fn detect(&self) -> PortResult<Option<String>>;
    /// Runs `claude <args>` to completion; a non-zero exit is *not* an error here.
    async fn run(&self, args: &[String], timeout: Duration) -> PortResult<CommandOutput>;
}

/// Text files Lampo generates for agents (the plugin folder). Implemented by
/// `lampo-infra-store` (`LocalFileSystem`).
pub trait AgentPluginFiles: Send + Sync {
    /// `None` when the file does not exist.
    fn read_text(&self, path: &Path) -> PortResult<Option<String>>;
    fn exists(&self, path: &Path) -> bool;
    /// Entry names of `dir` (files and folders); empty when it does not exist.
    fn list_dir(&self, dir: &Path) -> PortResult<Vec<String>>;
    /// Creates the parent folders, then writes UTF-8 `contents` (no BOM), replacing the file.
    fn write_text(&self, path: &Path, contents: &str) -> PortResult<()>;
    /// Removes a file or a whole folder; Ok if absent.
    fn remove(&self, path: &Path) -> PortResult<()>;
}
