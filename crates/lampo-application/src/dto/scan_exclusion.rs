//! DTOs of the optional Microsoft Defender exclusions: the MySQL data folder, the PHP/Apache
//! server processes and a projects folder (`lampo defender exclusion`, Settings). The `ScanExclusion*` types are the
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

/// Which files one request changes, all through one UAC prompt. At least one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "DefenderExclusionRequest"))]
#[serde(rename_all = "camelCase")]
pub struct DefenderExclusionRequestDto {
    pub change: ScanExclusionChangeDto,
    /// The files opened by `php-cgi.exe` of every installed PHP and by Apache's `httpd.exe`
    /// (process exclusions).
    pub php: bool,
    /// The MySQL data folder (a path exclusion).
    pub mysql: bool,
    /// A folder holding the user's projects (a path exclusion): `projectsFolder`, or the
    /// suggested one ([`DefenderExclusionInfoDto::projects_folder`]) when it is absent.
    #[serde(default)]
    pub projects: bool,
    /// The projects folder to exclude (an absolute path); only with `projects`.
    #[serde(default)]
    #[cfg_attr(test, ts(optional = nullable))]
    pub projects_folder: Option<String>,
    /// The process exclusions Lampo applied to server executables no longer installed (a PHP or
    /// Apache uninstalled since): only to remove them, leaving the installed ones excluded.
    #[serde(default)]
    pub uninstalled: bool,
}

impl DefenderExclusionRequestDto {
    /// A request naming no projects folder.
    pub fn new(change: ScanExclusionChangeDto, php: bool, mysql: bool) -> Self {
        Self { change, php, mysql, projects: false, projects_folder: None, uninstalled: false }
    }

    /// The removal of the exclusions of the executables no longer installed.
    pub fn remove_uninstalled() -> Self {
        Self { uninstalled: true, ..Self::new(ScanExclusionChangeDto::Remove, false, false) }
    }
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
    /// The projects folder, when the request named it.
    pub projects: Option<String>,
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
    /// The suggested projects folder (`projects`): the deepest folder holding the folders of the
    /// registered projects that may be excluded; `null` when there is none.
    pub projects_folder: Option<String>,
    /// The executables Lampo excluded that are no longer installed (`uninstalled`, only to
    /// remove): a PHP or Apache uninstalled since. Empty when the record of the exclusions does
    /// not load.
    pub uninstalled_processes: Vec<String>,
}

/// A change of the PHP process exclusion Lampo proposes after a PHP is installed or uninstalled,
/// and never makes by itself: only when an exclusion Lampo applied is in place and Microsoft
/// Defender is the active antivirus.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "DefenderExclusionSuggestion"))]
#[serde(rename_all = "camelCase")]
pub struct DefenderExclusionSuggestionDto {
    pub change: ScanExclusionChangeDto,
    /// The executables concerned: the installed `php-cgi.exe` not excluded yet (`add`), or the
    /// excluded executables no longer installed (`remove`).
    pub processes: Vec<String>,
    /// The request making the change (one UAC prompt).
    pub request: DefenderExclusionRequestDto,
    /// The CLI command making the change.
    pub command: String,
}

/// What one Defender exclusion excludes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "DefenderExclusionKind"))]
#[serde(rename_all = "kebab-case")]
pub enum DefenderExclusionKindDto {
    /// The files a process opens (`-ExclusionProcess`).
    Process,
    /// A folder and everything in it (`-ExclusionPath`).
    Path,
}

/// One exclusion of a request as a PowerShell command, for whoever manages the PC (group policy,
/// Intune) when Defender refuses Lampo's request: the exact path a request would name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "DefenderExclusionCommand"))]
#[serde(rename_all = "camelCase")]
pub struct DefenderExclusionCommandDto {
    pub kind: DefenderExclusionKindDto,
    pub path: String,
    /// `Add-MpPreference -ExclusionProcess '<path>'` (or `Remove-MpPreference`, `-ExclusionPath`),
    /// one line, the path quoted for PowerShell.
    pub command: String,
}
