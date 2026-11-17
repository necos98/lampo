//! The antivirus scan exclusions of Lampo's files. Implemented by `lampo-infra-web`
//! (`DefenderScanExclusion`, through the elevated helper).

use std::path::{Path, PathBuf};

use async_trait::async_trait;

use crate::error::PortResult;
use crate::scan_exclusion::{ScanExclusionChange, ScanExclusionOutcome, ScanExclusionTargets};

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
}
