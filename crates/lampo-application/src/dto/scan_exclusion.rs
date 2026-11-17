//! DTOs of the optional Microsoft Defender exclusions: the MySQL data folder and the PHP/Apache
//! server processes (`lampo defender exclusion`, Settings). The `ScanExclusion*` types are the
//! MySQL-only form (`lampo mysql defender-exclusion`, the settings panel), kept as they are.

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

/// Which of Lampo's files one request changes, all through one UAC prompt. At least one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "DefenderExclusionRequest"))]
#[serde(rename_all = "camelCase")]
pub struct DefenderExclusionRequestDto {
    pub change: ScanExclusionChangeDto,
    /// The files opened by `php-cgi.exe` of every installed PHP and by Apache's `httpd.exe`
    /// (process exclusions).
    pub php: bool,
    /// The MySQL data folder (a path exclusion).
    pub mysql: bool,
}

/// The result of one request (see [`ScanExclusionResultDto`]: there is no "active" state).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "DefenderExclusionResult"))]
#[serde(rename_all = "camelCase")]
pub struct DefenderExclusionResultDto {
    pub change: ScanExclusionChangeDto,
    pub outcome: ScanExclusionOutcomeDto,
    /// The MySQL data folder, when the request named it.
    pub folder: Option<String>,
    /// The executables the request named (empty without `php`).
    pub processes: Vec<String>,
}

/// What each part of a request would cover now, shown before the user asks for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "DefenderExclusionInfo"))]
#[serde(rename_all = "camelCase")]
pub struct DefenderExclusionInfoDto {
    /// The MySQL data folder (`mysql`).
    pub folder: String,
    /// `php-cgi.exe` of every installed PHP and `httpd.exe` of every installed Apache (`php`).
    pub processes: Vec<String>,
}
