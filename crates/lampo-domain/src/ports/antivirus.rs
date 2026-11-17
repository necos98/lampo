//! The antivirus scan exclusion of a Lampo folder. Implemented by `lampo-infra-web`
//! (`DefenderScanExclusion`, through the elevated helper).

use std::path::Path;

use async_trait::async_trait;

use crate::error::PortResult;
use crate::scan_exclusion::{ScanExclusionChange, ScanExclusionOutcome};

/// Adds or removes the Microsoft Defender exclusion of `folder` (a UAC prompt each time).
///
/// The outcomes the user can act on (prompt declined, Defender absent or not active, a policy
/// refusing or hiding the change) are [`ScanExclusionOutcome`]s, not errors. Errors:
/// `InvalidData` for a folder the elevated side would refuse (checked before any prompt),
/// `NotFound` when the helper is missing, `Process` when it cannot be run or fails.
#[async_trait]
pub trait ScanExclusionEditor: Send + Sync {
    async fn change(&self, folder: &Path, change: ScanExclusionChange) -> PortResult<ScanExclusionOutcome>;
}
