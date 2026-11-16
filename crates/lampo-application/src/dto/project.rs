use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::database::{DatabaseConnectionDto, MailConnectionDto};
use super::worktree::WorktreeEnvironmentDto;

/// How a site is reached. Wire form: `{ "kind": "hostname", "host": "shop.test" }`,
/// `{ "kind": "localhost", "host": "shop.localhost" }`, `{ "kind": "loopback-port", "port": 8101 }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "SiteAddress"))]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum SiteAddressDto {
    /// Listed in the hosts file (changing it needs a UAC prompt).
    Hostname { host: String },
    /// `*.localhost`, no hosts entry.
    Localhost { host: String },
    /// Dedicated `127.0.0.1:<port>` listener, no hosts entry.
    LoopbackPort { port: u16 },
}

/// Default addresses for a new project when none are given explicitly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "Addressing"))]
#[serde(rename_all = "kebab-case")]
pub enum AddressingDto {
    /// `<id>.<tld>` (hosts file) + `<id>.localhost`. Used when the field is omitted (protocol
    /// compatibility); the GUI offers it as an explicit option.
    #[default]
    Standard,
    /// `<id>.localhost` + a dedicated loopback port: never needs elevation. The built-in value of
    /// `SettingsDto::default_addressing`, which front-ends use when nothing is chosen.
    NoElevation,
    /// Only `<id>.<tld>` (hosts file): needs elevation, like `Standard`.
    HostsOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "TlsMode"))]
#[serde(rename_all = "kebab-case")]
pub enum TlsModeDto {
    #[default]
    Disabled,
    /// Reserved for the future local-CA feature; rejected with `unsupported` for now.
    LocalCa,
}

/// The environment a preset carries and a project copies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "ProjectEnvironment"))]
#[serde(rename_all = "camelCase")]
pub struct ProjectEnvironmentDto {
    /// PHP minor, e.g. `"8.3"`.
    pub php_version: String,
    /// Bundled extensions (`"intl"`, `"pdo_mysql"`).
    pub extensions: Vec<String>,
    /// Add-on extension package kinds (`"xdebug"`, `"php-redis"`).
    pub addons: Vec<String>,
    pub php_ini: BTreeMap<String, String>,
    /// Relative to the project folder; `""` = the folder itself.
    pub public_dir: String,
    /// Services the project needs while served (`{ "kind": "mysql", "version": "8.4" }`,
    /// `{ "kind": "redis", "version": null }`). The MySQL entry also holds its database.
    pub services: Vec<ServiceUseDto>,
    pub apache_directives: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "Project"))]
#[serde(rename_all = "camelCase")]
pub struct ProjectDto {
    pub id: String,
    pub name: String,
    pub root: String,
    /// `false` when the folder was moved/deleted (offer "relink").
    pub folder_exists: bool,
    pub addresses: Vec<SiteAddressDto>,
    /// Primary URL.
    pub url: String,
    /// One URL per address, primary first.
    pub urls: Vec<String>,
    pub document_root: String,
    pub environment: ProjectEnvironmentDto,
    pub database_name: Option<String>,
    /// Connection details to show (never written to `.env`).
    pub database: Option<DatabaseConnectionDto>,
    /// SMTP endpoint and web UI of Mailpit, when the project uses it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mail: Option<MailConnectionDto>,
    pub tls: TlsModeDto,
    pub active: bool,
    pub origin_preset: Option<String>,
    /// Worktree environments of this project.
    pub worktrees: Vec<WorktreeEnvironmentDto>,
    /// What a save has to tell about the saved project (`bcmath is built into PHP 8.3, nothing
    /// to enable`); always empty outside a save's answer.
    #[serde(default)]
    pub notices: Vec<String>,
}

/// A project file that does not load (malformed, or edited into invalid values). The project is
/// hidden until the file is repaired; its loopback ports stay reserved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "UnreadableProject"))]
#[serde(rename_all = "camelCase")]
pub struct UnreadableProjectDto {
    pub file: String,
    pub reason: String,
}

/// Every registered project: the ones that load and the files that do not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "ProjectList"))]
#[serde(rename_all = "camelCase")]
pub struct ProjectListDto {
    pub projects: Vec<ProjectDto>,
    pub unreadable: Vec<UnreadableProjectDto>,
}

/// Compact row for `lampo status` / `lampo project list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "ProjectSummary"))]
#[serde(rename_all = "camelCase")]
pub struct ProjectSummaryDto {
    pub id: String,
    pub name: String,
    pub root: String,
    pub active: bool,
    pub url: String,
    pub php_version: String,
    pub mysql_version: Option<String>,
    pub worktree_count: usize,
}

/// Create a project. Only `root` is required: anything omitted is filled from the preset
/// (`preset_id`, else the detected one, else `generic-php`), detection and settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", default)]
pub struct CreateProjectRequest {
    pub root: String,
    /// Default: folder name.
    #[cfg_attr(test, ts(optional = nullable))]
    pub name: Option<String>,
    /// Default: derived from the name.
    #[cfg_attr(test, ts(optional = nullable))]
    pub id: Option<String>,
    #[cfg_attr(test, ts(optional = nullable))]
    pub preset_id: Option<String>,
    /// Overrides the preset's PHP minor.
    #[cfg_attr(test, ts(optional = nullable))]
    pub php_version: Option<String>,
    /// Full environment; when present the preset is only recorded as origin.
    #[cfg_attr(test, ts(optional = nullable))]
    pub environment: Option<ProjectEnvironmentDto>,
    /// Explicit addresses; when absent `addressing` decides.
    #[cfg_attr(test, ts(optional = nullable))]
    pub addresses: Option<Vec<SiteAddressDto>>,
    #[cfg_attr(test, ts(optional = nullable))]
    pub addressing: AddressingDto,
    #[cfg_attr(test, ts(optional = nullable))]
    pub activate: bool,
    /// Create the project's MySQL database (if it uses MySQL), starting the instance on demand.
    /// Default `true` (also when the field is omitted). A failure never fails the creation: it
    /// is reported in [`CreateProjectResultDto::database_setup`].
    #[cfg_attr(test, ts(optional = nullable))]
    pub create_database: bool,
}

impl Default for CreateProjectRequest {
    fn default() -> Self {
        Self {
            root: String::new(),
            name: None,
            id: None,
            preset_id: None,
            php_version: None,
            environment: None,
            addresses: None,
            addressing: AddressingDto::default(),
            activate: false,
            create_database: true,
        }
    }
}

/// Result of creating a project: the project itself (flattened, so on the wire this *is* a
/// [`ProjectDto`] plus one optional field) and what happened to its database.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "CreateProjectResult"))]
#[serde(rename_all = "camelCase")]
pub struct CreateProjectResultDto {
    #[serde(flatten)]
    pub project: ProjectDto,
    /// `null`/absent when nothing was attempted: the project uses no MySQL, or
    /// `createDatabase` was `false`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database_setup: Option<super::database::DatabaseSetupDto>,
}

/// Result of removing a project: what was kept on purpose and is no longer reachable from Lampo.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "DeleteProjectResult"))]
#[serde(rename_all = "camelCase")]
pub struct DeleteProjectResultDto {
    /// The own databases of its worktree environments, left in MySQL (a removal never drops a
    /// database): drop them by hand if they are not needed.
    #[serde(default)]
    pub orphaned_databases: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct UpdateProjectRequest {
    pub project_id: String,
    pub name: String,
    pub addresses: Vec<SiteAddressDto>,
    pub environment: ProjectEnvironmentDto,
    pub database_name: Option<String>,
    pub tls: TlsModeDto,
}

/// Partial change of a project's environment (`lampo project set`), applied atomically to the
/// stored project: unlike [`UpdateProjectRequest`] nothing omitted is overwritten, so a CLI call
/// never loses a concurrent GUI edit. Removals apply before additions; naming the same item in
/// both is a `validation` error. An empty patch returns the project unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProjectPatchRequest {
    pub project_id: String,
    /// PHP minor to switch to (`8.4`): it must be installed (`package-missing` otherwise), like
    /// every add-on the project keeps. Applied before the other changes, so the extensions and
    /// add-ons named in the same patch are checked against the new PHP.
    pub php_version: Option<String>,
    /// Bundled extensions to enable (`intl`, `php_intl.dll`).
    pub extensions_add: Vec<String>,
    pub extensions_remove: Vec<String>,
    /// Add-on package kinds to enable (`xdebug`, `php-redis`): each must be installed for the
    /// project's PHP minor (`package-missing` otherwise).
    pub addons_add: Vec<String>,
    pub addons_remove: Vec<String>,
    /// php.ini overrides to set (key → value).
    pub ini_set: BTreeMap<String, String>,
    /// php.ini overrides to remove (back to Lampo's base value).
    pub ini_unset: Vec<String>,
    /// Services to use. A versioned kind without `version` gets the default configured instance
    /// (`mysql` → the always-on series); a versioned kind already used switches series.
    pub services_add: Vec<ServiceUseDto>,
    /// Service kinds to stop using (`mysql`, `redis`). Removing MySQL never drops a database.
    pub services_remove: Vec<String>,
    /// Database shown in the connection details; requires the project to use MySQL.
    pub database_name: Option<String>,
    /// Document root relative to the project folder (`""` = the folder itself).
    pub public_dir: Option<String>,
    /// Extra directives inside the project's `<VirtualHost>` (replaces them; `""` clears).
    pub apache_directives: Option<String>,
    /// New display name (`lampo project rename`); the id, URLs and database stay.
    pub name: Option<String>,
    /// Addresses to add, in their displayed form: `shop.test` (hosts file: needs elevation),
    /// `shop.localhost`, `127.0.0.1:8101`, or [`AUTO_LOOPBACK`] for a free dedicated port (a
    /// no-op when the project already has one). Appended, so the primary address (the first)
    /// stays unless it is removed.
    pub addresses_add: Vec<String>,
    /// Addresses to remove (same forms; [`AUTO_LOOPBACK`] = the dedicated port, whichever it
    /// is). At least one address must remain.
    pub addresses_remove: Vec<String>,
}

/// [`ProjectPatchRequest`] address meaning "a dedicated loopback port", whichever it is.
pub const AUTO_LOOPBACK: &str = "loopback";

impl ProjectPatchRequest {
    /// Whether the patch changes nothing.
    pub fn is_empty(&self) -> bool {
        let Self {
            project_id: _,
            php_version,
            extensions_add,
            extensions_remove,
            addons_add,
            addons_remove,
            ini_set,
            ini_unset,
            services_add,
            services_remove,
            database_name,
            public_dir,
            apache_directives,
            name,
            addresses_add,
            addresses_remove,
        } = self;
        php_version.is_none()
            && extensions_add.is_empty()
            && extensions_remove.is_empty()
            && addons_add.is_empty()
            && addons_remove.is_empty()
            && ini_set.is_empty()
            && ini_unset.is_empty()
            && services_add.is_empty()
            && services_remove.is_empty()
            && database_name.is_none()
            && public_dir.is_none()
            && apache_directives.is_none()
            && name.is_none()
            && addresses_add.is_empty()
            && addresses_remove.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SetProjectActiveRequest {
    pub project_id: String,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SetProjectPhpRequest {
    pub project_id: String,
    pub php_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct RelinkProjectRequest {
    pub project_id: String,
    pub root: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ExportProjectRequest {
    pub project_id: String,
    /// Destination file (`.toml`).
    pub file: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ImportProjectRequest {
    pub file: String,
    /// Folder on this machine, if different from the exported one.
    pub root: Option<String>,
}

/// The imported project (the fields of [`ProjectDto`]) and the settings of the file that were
/// left out because they could run code here. Front-ends show them so the user can add them
/// back knowingly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "ImportProjectResult"))]
#[serde(rename_all = "camelCase")]
pub struct ImportProjectResultDto {
    #[serde(flatten)]
    pub project: ProjectDto,
    /// The export's custom Apache directives, verbatim (never imported); `null` when it had none.
    #[serde(default)]
    pub dropped_apache_directives: Option<String>,
    /// php.ini keys left out (`auto_prepend_file`, `extension`, …), as written in the file.
    #[serde(default)]
    pub dropped_php_ini_keys: Vec<String>,
}

/// Result of analysing a folder before creating a project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "Detection"))]
#[serde(rename_all = "camelCase")]
pub struct DetectionDto {
    pub folder: String,
    /// `laravel`, `wordpress`, `symfony`, `composer` or `null`.
    pub project_type: Option<String>,
    pub preset_id: Option<String>,
    pub php_version: Option<String>,
    pub suggested_name: String,
    pub suggested_id: String,
    pub evidence: Vec<String>,
    /// Service kinds the folder's own files show it uses (`mysql`, `redis`; from a Docker
    /// Compose file). Creation adds the unversioned ones the preset lacks.
    pub services: Vec<String>,
    /// What Lampo cannot provide for this folder (PostgreSQL, search engines…), developer-facing.
    pub warnings: Vec<String>,
    /// Id of the project already registered for this folder, if any.
    pub already_registered: Option<String>,
}

/// One service a project uses: its kind and, for versioned kinds, the series.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "ServiceUse"))]
#[serde(rename_all = "camelCase")]
pub struct ServiceUseDto {
    pub kind: String,
    pub version: Option<String>,
}

impl ProjectEnvironmentDto {
    /// The entry of `kind`, if the project uses it.
    pub fn service(&self, kind: &str) -> Option<&ServiceUseDto> {
        self.services.iter().find(|s| s.kind == kind)
    }

    /// The version used of a versioned `kind` (`mysql` → `8.4`).
    pub fn service_version(&self, kind: &str) -> Option<&str> {
        self.service(kind).and_then(|s| s.version.as_deref())
    }
}

/// Addresses to offer for a project id, built by the domain rules and the port allocator
/// (`project suggest-addresses`): one per kind, plus the two default sets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "AddressSuggestions"))]
#[serde(rename_all = "camelCase")]
pub struct AddressSuggestionsDto {
    /// `<id>.<tld>` (hosts file, needs the UAC prompt once).
    pub hostname: SiteAddressDto,
    /// `<id>.localhost`.
    pub localhost: SiteAddressDto,
    /// The loopback port the allocator would assign now (not reserved).
    pub loopback_port: SiteAddressDto,
    /// `AddressingDto::NoElevation` defaults: localhost + loopback port.
    pub no_elevation: Vec<SiteAddressDto>,
    /// `AddressingDto::Standard` defaults: hostname + localhost.
    pub standard: Vec<SiteAddressDto>,
    /// `AddressingDto::HostsOnly` defaults: the hostname alone.
    pub hosts_only: Vec<SiteAddressDto>,
    /// The global default (`SettingsDto::default_addressing`), to preselect.
    pub default_addressing: AddressingDto,
}
