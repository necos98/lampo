//! The antivirus scan exclusions of Lampo's files. Implemented by `lampo-infra-web`
//! (`DefenderScanExclusion`, through the elevated helper).

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
}
