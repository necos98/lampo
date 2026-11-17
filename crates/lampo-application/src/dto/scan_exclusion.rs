//! DTOs of the optional Microsoft Defender exclusion of the MySQL data folder (Settings >
//! Strumenti, `lampo mysql defender-exclusion`).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "ScanExclusionChange"))]
#[serde(rename_all = "kebab-case")]
pub enum ScanExclusionChangeDto {
    Add,
    Remove,
}

/// How a request ended: none of these is an error, each is told to the user as it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "ScanExclusionOutcome"))]
#[serde(rename_all = "kebab-case")]
pub enum ScanExclusionOutcomeDto {
    /// Defender confirmed the change.
    Applied,
    /// The UAC prompt was declined: nothing changed.
    Cancelled,
    /// Defender is not installed or not the active antivirus.
    DefenderUnavailable,
    /// Defender refused the change (Tamper Protection, a group policy or Intune).
    Refused,
    /// Accepted, but the change cannot be seen afterwards (exclusions managed by a policy).
    NotConfirmed,
}

/// The result of one request. Defender's exclusions cannot be read without administrator
/// rights, so this is all Lampo knows: there is no "active" state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "ScanExclusionResult"))]
#[serde(rename_all = "camelCase")]
pub struct ScanExclusionResultDto {
    pub change: ScanExclusionChangeDto,
    pub outcome: ScanExclusionOutcomeDto,
    /// The excluded folder (`%LOCALAPPDATA%\Lampo\mysql`).
    pub folder: String,
}

/// What the exclusion would cover, shown before the user asks for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "ScanExclusionInfo"))]
#[serde(rename_all = "camelCase")]
pub struct ScanExclusionInfoDto {
    pub folder: String,
}
