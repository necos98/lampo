//! DTOs of the coding-agent integrations (Settings > Integrazioni).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "ClaudeCodeState"))]
#[serde(rename_all = "kebab-case")]
pub enum ClaudeCodeStateDto {
    /// Isolated data root (`LAMPO_HOME`): the real Claude Code is never touched.
    Isolated,
    /// `claude` not found (or not answering).
    ClaudeMissing,
    NotInstalled,
    Installed,
    /// Registered, but not with the plugin this app version generates.
    Outdated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "ClaudeCodeStatus"))]
#[serde(rename_all = "camelCase")]
pub struct ClaudeCodeStatusDto {
    pub state: ClaudeCodeStateDto,
    /// First line of `claude --version`.
    pub claude_version: Option<String>,
    /// Version recorded by the last successful install (`.installed-version`).
    pub installed_version: Option<String>,
    /// Version of the plugin this app generates now (`<app>-h<hash>`).
    pub expected_version: String,
    /// Folder of the generated plugin (a local marketplace).
    pub plugin_dir: String,
    /// The `lampo.exe` the hook and the skills call (the copy in the shim dir).
    pub cli_path: String,
    /// Whether that copy exists (installed with the command-line tools).
    pub cli_installed: bool,
    /// Last automatic update at startup that failed, cleared by a successful install/update.
    pub update_error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "StepOutcome"))]
#[serde(rename_all = "kebab-case")]
pub enum StepOutcomeDto {
    Ok,
    Failed,
    /// Failed, but the step is best-effort (cleanup of something that may not exist).
    Ignored,
}

/// One line of the step log shown in the GUI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "ClaudeCodeStep"))]
#[serde(rename_all = "camelCase")]
pub struct ClaudeCodeStepDto {
    /// Stable kebab-case id (`detect-claude`, `check-cli`, `write-files`, `validate-plugin`,
    /// `validate-marketplace`, `remove-previous-plugin`, `remove-previous-marketplace`,
    /// `add-marketplace`, `install-plugin`, `verify`, `write-marker`, `remove-plugin`,
    /// `remove-marketplace`, `remove-files`).
    pub step: String,
    pub outcome: StepOutcomeDto,
    /// Version, path or the first lines of the program's output.
    pub detail: Option<String>,
}

/// Result of an install/update/removal: never an error for a failing step (the log says which).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "ClaudeCodeAction"))]
#[serde(rename_all = "camelCase")]
pub struct ClaudeCodeActionDto {
    pub ok: bool,
    pub steps: Vec<ClaudeCodeStepDto>,
    pub status: ClaudeCodeStatusDto,
}
