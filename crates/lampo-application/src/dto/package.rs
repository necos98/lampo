use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "PackageId"))]
#[serde(rename_all = "camelCase")]
pub struct PackageIdDto {
    /// `php`, `apache`, `mod-fcgid`, `mysql`, `redis`, `composer`, `xdebug`, `php-redis`.
    pub kind: String,
    pub version: String,
    /// For add-on extensions: the PHP minor they are built for.
    pub php_version: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "CatalogChannel"))]
#[serde(rename_all = "lowercase")]
pub enum CatalogChannelDto {
    Frozen,
    Live,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "CatalogEntry"))]
#[serde(rename_all = "camelCase")]
pub struct CatalogEntryDto {
    pub id: PackageIdDto,
    pub channel: CatalogChannelDto,
    pub end_of_life: bool,
    pub size_bytes: Option<u64>,
    /// `vc11`, `vc14`, `vc15`, `vs16`, `vs17`.
    pub vc_runtime: Option<String>,
    /// Whether the required VC++ runtime is installed (`null` if none required).
    pub vc_runtime_installed: Option<bool>,
    pub installed: bool,
    /// `<major>.<minor>` of the version (`8.3`), as the domain computes it.
    #[serde(default)]
    pub series: String,
    /// Tab the GUI lists it under: `php`, a service kind (`mysql`, `redis`, `apache`),
    /// `addons` (PHP-pinned extensions) or `tools` (anything else).
    #[serde(default)]
    pub group: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "InstalledPackage"))]
#[serde(rename_all = "camelCase")]
pub struct InstalledPackageDto {
    pub id: PackageIdDto,
    pub path: String,
    /// Site ids (`shop`, `shop@fix-1`) or `system` using this package.
    pub used_by: Vec<String>,
    /// `<major>.<minor>` of the version.
    #[serde(default)]
    pub series: String,
}

/// Install a package. Without `version`: newest in `series` (if given) else newest overall.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", default)]
pub struct InstallPackageRequest {
    pub kind: String,
    #[cfg_attr(test, ts(optional = nullable))]
    pub version: Option<String>,
    /// `"8.3"` → latest 8.3.x.
    #[cfg_attr(test, ts(optional = nullable))]
    pub series: Option<String>,
    #[cfg_attr(test, ts(optional = nullable))]
    pub php_version: Option<String>,
}

/// Uninstall the one installed package matching these (`lampo package uninstall`). Same fields
/// as [`InstallPackageRequest`]: an exact `version`, else a `series` (`"8.3"`), else any build
/// of `kind`; `phpVersion` narrows add-ons to the builds for that PHP minor.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UninstallPackageRequest {
    pub kind: String,
    pub version: Option<String>,
    pub series: Option<String>,
    pub php_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "UpdateAvailable"))]
#[serde(rename_all = "camelCase")]
pub struct UpdateAvailableDto {
    pub kind: String,
    pub series: String,
    pub php_version: Option<String>,
    pub installed: String,
    pub latest: String,
}

/// An extension an installed PHP minor can load (from its `ext` folder).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "PhpExtension"))]
#[serde(rename_all = "camelCase")]
pub struct PhpExtensionDto {
    /// As written in a project's extension list (`intl`, `pdo_mysql`).
    pub name: String,
    /// Loaded with `zend_extension` (`opcache`).
    pub zend: bool,
}

/// `lampo package extensions`: the bundled extensions of one installed PHP minor, i.e. what
/// `lampo project set --ext` accepts for a project on that minor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhpExtensionListDto {
    /// PHP minor listed (`"8.3"`).
    pub php_version: String,
    /// The project whose `enabled` flags are shown: the resolved site's project when it uses
    /// this PHP minor, else `null` (and every `enabled` is `null`).
    pub project_id: Option<String>,
    pub extensions: Vec<PhpExtensionStateDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhpExtensionStateDto {
    /// As written in a project's extension list and accepted by `project set --ext`.
    pub name: String,
    /// Loaded with `zend_extension` (`opcache`).
    pub zend: bool,
    /// Enabled in the environment of [`PhpExtensionListDto::project_id`]; `null` without one.
    pub enabled: Option<bool>,
}

/// Returned by commands that start a background task; progress arrives as events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "TaskStarted"))]
#[serde(rename_all = "camelCase")]
pub struct TaskStartedDto {
    pub task_id: String,
}

/// What a background task does, as requested by the front-end that started it. Lets a window
/// created later (the GUI destroys its window on close) find and follow tasks still running.
/// Wire form: `{ "kind": "install-package", "request": {...} }`, `{ "kind": "first-run" }`,
/// `{ "kind": "worktree-up", "request": {...} }`, `{ "kind": "scaffold-project", "request": {...} }`,
/// `{ "kind": "uninstall-package", "id": {...} }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "TaskOperation"))]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum TaskOperationDto {
    InstallPackage { request: InstallPackageRequest },
    FirstRun,
    WorktreeUp { request: super::worktree::WorktreeUpRequest },
    ScaffoldProject { request: super::scaffold::ScaffoldProjectRequest },
    UninstallPackage { id: PackageIdDto },
}

/// A background task that has not finished yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "RunningTask"))]
#[serde(rename_all = "camelCase")]
pub struct RunningTaskDto {
    pub task_id: String,
    pub operation: TaskOperationDto,
    /// Latest progress report, if any arrived yet.
    pub progress: Option<crate::events::ProgressDto>,
    /// Latest progress report per subject (a first run installs several packages).
    pub subjects: Vec<crate::events::ProgressDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "FirstRunItem"))]
#[serde(rename_all = "camelCase")]
pub struct FirstRunItemDto {
    pub kind: String,
    /// Series to install (`"8.4"`), or `null` for latest.
    pub series: Option<String>,
    pub installed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "FirstRunStatus"))]
#[serde(rename_all = "camelCase")]
pub struct FirstRunStatusDto {
    pub completed: bool,
    /// Apache, mod_fcgid, latest PHP, MySQL default series, Composer.
    pub items: Vec<FirstRunItemDto>,
}
