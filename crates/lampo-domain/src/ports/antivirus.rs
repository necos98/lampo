//! The antivirus scan exclusions of Lampo's files and Defender's state. Implemented by `lampo-infra-web`
//! (`DefenderScanExclusion` through the elevated helper, `PowerShellDefenderState`) and
//! `lampo-infra-store` (`TomlProcessExclusionRecord`).

use std::path::{Path, PathBuf};

use async_trait::async_trait;

use crate::error::PortResult;
use crate::scan_exclusion::{DefenderState, ProcessExclusionRecord, ScanExclusionChange, ScanExclusionOutcome, ScanExclusionTargets};

/// Adds or removes the Microsoft Defender exclusions of `targets`, all through one UAC prompt.
///
/// The outcomes the user can act on (prompt declined, Defender absent or not active, a policy
/// refusing or hiding the change) are [`ScanExclusionOutcome`]s, not errors. Errors:
/// `InvalidData` for a target the elevated side would refuse, or no target at all (checked
/// before any prompt), `NotFound` when the helper is missing, `Process` when it cannot be run or
/// fails.
#[async_trait]
pub trait ScanExclusionEditor: Send + Sync {
    async fn change(&self, targets: &ScanExclusionTargets, change: ScanExclusionChange) -> PortResult<ScanExclusionOutcome>;

    /// `folder` as adding it as the projects folder would name it (the spelling the elevated side
    /// rebuilds), or `InvalidData` when that would be refused. Never shows a prompt: it tells
    /// which folder may be suggested.
    fn accepted_projects_folder(&self, folder: &Path) -> PortResult<PathBuf>;

    /// `targets` as [`Self::change`] would hand them to the elevated side for `change` (the
    /// spelling it rebuilds), after the same checks, or the same error. Never shows a prompt and
    /// changes nothing (a missing MySQL folder, which adding creates, is not an error): it tells
    /// what someone else may exclude for the user.
    fn accepted_targets(&self, targets: &ScanExclusionTargets, change: ScanExclusionChange) -> PortResult<ScanExclusionTargets>;
}

/// Reads Microsoft Defender's state without elevation. Implemented by `lampo-infra-web`
/// (`PowerShellDefenderState`). Errors: `Process` when the state cannot be read (PowerShell
/// missing, failing or not answering within a few seconds): the caller treats it as unknown.
#[async_trait]
pub trait DefenderStateReader: Send + Sync {
    async fn state(&self) -> PortResult<DefenderState>;
}

/// Reads what Lampo remembers of its requests naming the server processes. Implemented by
/// `lampo-infra-store` (`TomlProcessExclusionRecord`). A missing record is the default (never
/// asked); `InvalidData` when it does not load.
pub trait ProcessExclusionRecordReader: Send + Sync {
    fn read(&self) -> PortResult<ProcessExclusionRecord>;
}

/// Replaces the record of the requests naming the server processes.
pub trait ProcessExclusionRecordWriter: Send + Sync {
    fn write(&self, record: &ProcessExclusionRecord) -> PortResult<()>;
}
