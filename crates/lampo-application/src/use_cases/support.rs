//! Helpers shared by several use cases (crate-private): id parsing, lookups in the installed
//! package set, the per-site view used by every generator, DTO presentation and small
//! allocation rules. No port is called here except through the arguments.

use std::path::{Path, PathBuf};

use lampo_domain::address::SiteAddress;
use lampo_domain::mysql::{DatabaseConnection, DatabaseName, MysqlSeries};
use lampo_domain::package::{EntryRole, InstalledPackage, PackageKind};
use lampo_domain::php::{ComposerChannel, PhpMinor};
use lampo_domain::ports::{FileSystemInspector, InstalledPackageRegistry, MysqlEndpoint, ProjectRepository, WorktreeRepository};
use lampo_domain::project::{Project, ProjectEnvironment, ProjectId, ProjectRoot, TlsMode};
use lampo_domain::service::ServiceKind;
use lampo_domain::settings::Settings;
use lampo_domain::site::SiteId;
use lampo_domain::version::VersionSeries;
use lampo_domain::worktree::{WorktreeDatabase, WorktreeEnvironment};
use lampo_domain::DomainError;

use super::allocation::SiteSnapshot;
use super::environment::EnvironmentSync;
use super::mysql_accounts::ConnectionPasswords;
use super::service_catalog::{ServiceCatalog, SiteMysqlClient};
use crate::context::{CallContext, ElevationPolicy};
use crate::dto::*;
use crate::error::{AppError, AppResult, ErrorCode};
use crate::mapping::{address_to_dto, connection_to_dto, credentials_to_dto, environment_to_dto, mail_to_dto, tls_to_dto};

// ---------------------------------------------------------------------------------------------
// Identifiers and lookups
// ---------------------------------------------------------------------------------------------

pub(crate) fn parse_project_id(raw: &str) -> AppResult<ProjectId> {
    Ok(ProjectId::new(raw.trim())?)
}

pub(crate) fn parse_site(raw: &str) -> AppResult<SiteId> {
    Ok(raw.parse::<SiteId>()?)
}

/// The project `raw`, or `NotFound`.
pub(crate) fn load_project(repo: &dyn ProjectRepository, raw: &str) -> AppResult<Project> {
    let id = parse_project_id(raw)?;
    repo.get(&id)?.ok_or_else(|| AppError::not_found("project", &id))
}

/// A resolved site: the project it belongs to and, for worktree environments, the environment.
pub(crate) fn load_site(projects: &dyn ProjectRepository, worktrees: &dyn WorktreeRepository, raw: &str) -> AppResult<(SiteId, Project, Option<WorktreeEnvironment>)> {
    let site = parse_site(raw)?;
    let project = projects.get(site.project())?.ok_or_else(|| AppError::not_found("project", site.project()))?;
    let worktree = match &site {
        SiteId::Project(_) => None,
        SiteId::Worktree { parent, slug } => Some(worktrees.get(parent, slug)?.ok_or_else(|| AppError::not_found("worktree environment", &site))?),
    };
    Ok((site, project, worktree))
}

/// `package` (e.g. `PHP 8.3`) is not installed; `details.package` names it for the GUI.
pub(crate) fn package_missing(package: impl std::fmt::Display) -> AppError {
    let package = package.to_string();
    AppError::new(ErrorCode::PackageMissing, format!("{package} is not installed")).with_details(package_details(&package))
}

/// `part` of an installed `package` (e.g. its command line) is missing.
pub(crate) fn package_part_missing(package: impl std::fmt::Display, part: &str) -> AppError {
    let package = package.to_string();
    AppError::new(ErrorCode::PackageMissing, format!("{package} is installed without its {part}")).with_details(package_details(&package))
}

/// `details` naming the package an error is about (`details.package`).
pub(crate) fn package_details(package: &str) -> serde_json::Value {
    serde_json::json!({ "package": package })
}

pub(crate) fn validation(message: impl Into<String>) -> AppError {
    AppError::new(ErrorCode::Validation, message)
}

/// Newest installed build of `kind` (any series when `series` is `None`) for `php`.
pub(crate) fn newest_of<'a>(installed: &'a [InstalledPackage], kind: &PackageKind, series: Option<VersionSeries>, php: Option<PhpMinor>) -> Option<&'a InstalledPackage> {
    installed.iter().filter(|p| &p.id.kind == kind && series.is_none_or(|s| p.id.series() == s) && p.id.php == php).max_by(|a, b| a.id.version.cmp(&b.id.version))
}

/// The newest installed patch of a PHP minor.
pub(crate) fn php_package(installed: &[InstalledPackage], php: PhpMinor) -> Option<&InstalledPackage> {
    newest_of(installed, &PackageKind::PHP, Some(php.as_series()), None)
}

/// Install dir of the newest installed patch of a PHP minor.
pub(crate) fn php_dir(installed: &[InstalledPackage], php: PhpMinor) -> Option<PathBuf> {
    newest_of(installed, &PackageKind::PHP, Some(php.as_series()), None).map(|p| p.install_dir.clone())
}

/// Every PHP minor with at least one installed build, ascending.
pub(crate) fn installed_php_minors(installed: &[InstalledPackage]) -> Vec<PhpMinor> {
    let mut minors: Vec<PhpMinor> = installed.iter().filter(|p| p.id.kind == PackageKind::PHP).filter_map(|p| PhpMinor::try_from(p.id.series()).ok()).collect();
    minors.sort();
    minors.dedup();
    minors
}

/// The Composer 2.2 LTS series.
pub(crate) const COMPOSER_LTS_SERIES: VersionSeries = VersionSeries { major: 2, minor: 2 };

/// The series a Composer channel requires (`None` = any build).
pub(crate) fn composer_series(channel: ComposerChannel) -> Option<VersionSeries> {
    match channel {
        ComposerChannel::Lts22 => Some(COMPOSER_LTS_SERIES),
        ComposerChannel::Stable => None,
    }
}

/// `composer.phar` usable with `php` (2.2 LTS below PHP 7.2, newest otherwise).
pub(crate) fn composer_phar_for(installed: &[InstalledPackage], php: PhpMinor) -> Option<PathBuf> {
    newest_of(installed, &PackageKind::COMPOSER, composer_series(php.composer_channel()), None).and_then(|p| p.entry(EntryRole::Phar)).map(Path::to_path_buf)
}

/// Wire name of a Composer channel (`lts22`, `stable`).
pub(crate) fn composer_channel_name(channel: ComposerChannel) -> String {
    serde_json::to_value(channel).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default()
}

// ---------------------------------------------------------------------------------------------
// Sites
// ---------------------------------------------------------------------------------------------

/// A project or a worktree environment, seen uniformly. Worktree environments inherit the
/// parent's environment *live* (no snapshot): every accessor reads the parent.
#[derive(Clone)]
pub(crate) struct SiteRef<'a> {
    pub id: SiteId,
    pub project: &'a Project,
    pub worktree: Option<&'a WorktreeEnvironment>,
}

impl SiteRef<'_> {
    pub fn environment(&self) -> &ProjectEnvironment {
        &self.project.environment
    }

    pub fn root(&self) -> &ProjectRoot {
        self.worktree.map(|w| &w.root).unwrap_or(&self.project.root)
    }

    pub fn document_root(&self) -> PathBuf {
        match self.worktree {
            Some(w) => w.document_root(self.project),
            None => self.project.document_root(),
        }
    }

    pub fn addresses(&self) -> Vec<SiteAddress> {
        match self.worktree {
            Some(w) => w.addresses(),
            None => self.project.addresses().to_vec(),
        }
    }

    /// Worktree environments are plain HTTP on loopback.
    pub fn tls(&self) -> TlsMode {
        if self.worktree.is_some() { TlsMode::Disabled } else { self.project.tls }
    }

    pub fn database(&self) -> Option<DatabaseName> {
        match self.worktree {
            Some(w) => w.database_name(self.project).cloned(),
            None => self.project.database().cloned(),
        }
    }

    /// What its MySQL clients default to: its database and, with a dedicated account, whose (a
    /// worktree environment uses its parent's).
    pub fn mysql_client(&self) -> SiteMysqlClient {
        SiteMysqlClient { database: self.database().map(|d| d.to_string()), project_account: self.project.mysql_project_account().cloned() }
    }

    /// Served by Apache: active projects and every worktree environment.
    pub fn served(&self) -> bool {
        self.worktree.is_some() || self.project.active
    }
}

/// Every project plus every worktree environment whose parent exists, sorted by site id.
pub(crate) fn all_sites<'a>(projects: &'a [Project], worktrees: &'a [WorktreeEnvironment]) -> Vec<SiteRef<'a>> {
    let mut sites: Vec<SiteRef<'a>> = projects.iter().map(|p| SiteRef { id: SiteId::Project(p.id.clone()), project: p, worktree: None }).collect();
    for w in worktrees {
        if let Some(parent) = projects.iter().find(|p| p.id == w.parent) {
            sites.push(SiteRef { id: w.site_id(), project: parent, worktree: Some(w) });
        }
    }
    sites.sort_by(|a, b| a.id.cmp(&b.id));
    sites
}

// ---------------------------------------------------------------------------------------------
// Presentation
// ---------------------------------------------------------------------------------------------

/// The connection details to show, with the password of the account they name.
pub(crate) fn shown_connection(connection: Option<DatabaseConnection>, passwords: &dyn ConnectionPasswords) -> Option<DatabaseConnectionDto> {
    connection.map(|mut c| {
        passwords.reveal(&mut c);
        connection_to_dto(&c)
    })
}

pub(crate) fn worktree_dto(
    w: &WorktreeEnvironment,
    parent: &Project,
    settings: &Settings,
    fs: Option<&dyn FileSystemInspector>,
    passwords: &dyn ConnectionPasswords,
) -> WorktreeEnvironmentDto {
    let alias = w.localhost_alias();
    let mode = match &w.database {
        WorktreeDatabase::Shared => DatabaseModeDto::Shared,
        WorktreeDatabase::Own { cloned_from: None, .. } => DatabaseModeDto::Fresh,
        WorktreeDatabase::Own { cloned_from: Some(source), .. } => DatabaseModeDto::Clone { source: source.to_string() },
    };
    WorktreeEnvironmentDto {
        site: w.site_id().to_string(),
        project_id: w.parent.to_string(),
        slug: w.slug.to_string(),
        root: w.root.to_string(),
        branch: w.branch.clone(),
        url: w.url(),
        urls: vec![w.url(), SiteAddress::Localhost(alias.clone()).url(settings.apache_port, TlsMode::Disabled)],
        loopback_port: w.loopback_port.get(),
        localhost_alias: alias.to_string(),
        document_root: w.document_root(parent).display().to_string(),
        database_mode: mode,
        database: shown_connection(w.database_connection(parent, settings).ok().flatten(), passwords),
        stale: fs.is_some_and(|fs| w.is_stale(std::slice::from_ref(parent), |root| fs.is_dir(root.as_path()))),
        created_at_unix_ms: w.created_at_unix_ms,
    }
}

/// `worktrees` may contain any environment; only the project's own are listed.
pub(crate) fn project_dto(
    project: &Project,
    worktrees: &[WorktreeEnvironment],
    settings: &Settings,
    fs: Option<&dyn FileSystemInspector>,
    passwords: &dyn ConnectionPasswords,
) -> ProjectDto {
    ProjectDto {
        id: project.id.to_string(),
        name: project.name.to_string(),
        root: project.root.to_string(),
        folder_exists: fs.is_none_or(|fs| fs.is_dir(project.root.as_path())),
        addresses: project.addresses().iter().map(address_to_dto).collect(),
        url: project.url(settings),
        urls: project.urls(settings),
        document_root: project.document_root().display().to_string(),
        environment: environment_to_dto(&project.environment),
        database_name: project.database().map(|d| d.to_string()),
        database: shown_connection(project.database_connection(settings).ok().flatten(), passwords),
        mysql_credentials: credentials_to_dto(project.mysql_credentials),
        mail: project.environment.uses(&ServiceKind::MAILPIT).then(|| mail_to_dto(&settings.mail_connection())),
        tls: tls_to_dto(project.tls),
        active: project.active,
        origin_preset: project.origin_preset.as_ref().map(|p| p.to_string()),
        worktrees: worktrees.iter().filter(|w| w.parent == project.id).map(|w| worktree_dto(w, project, settings, fs, passwords)).collect(),
        notices: Vec::new(),
    }
}

// ---------------------------------------------------------------------------------------------
// MySQL
// ---------------------------------------------------------------------------------------------

/// Where to reach the MySQL `series` instance with its clients, as the user its descriptor names.
pub(crate) fn mysql_endpoint(installed: &dyn InstalledPackageRegistry, settings: &Settings, catalog: &ServiceCatalog, series: MysqlSeries) -> AppResult<MysqlEndpoint> {
    mysql_endpoint_in(&installed.list()?, settings, catalog, series)
}

/// [`mysql_endpoint`] among `packages`, the installed packages already read.
pub(crate) fn mysql_endpoint_in(packages: &[InstalledPackage], settings: &Settings, catalog: &ServiceCatalog, series: MysqlSeries) -> AppResult<MysqlEndpoint> {
    let port = settings.mysql_port(series).ok_or_else(|| DomainError::UnknownMysqlSeries(series.to_string()))?;
    let package = newest_of(packages, &PackageKind::MYSQL, Some(series.as_series()), None).ok_or_else(|| package_missing(format!("MySQL {series}")))?;
    let client_dir = package.entry(EntryRole::ClientDir).ok_or_else(|| package_part_missing(format!("MySQL {series}"), "clients"))?.to_path_buf();
    let user = catalog.client_user(&ServiceKind::MYSQL).ok_or_else(|| AppError::internal("the MySQL service kind names no client user"))?;
    Ok(MysqlEndpoint { series, client_dir, port, user: user.to_string() })
}

// ---------------------------------------------------------------------------------------------
// Environment sync
// ---------------------------------------------------------------------------------------------

/// Syncs without ever prompting for elevation, and does not fail because an *unrelated* hosts
/// change is pending (worktree environments never touch the hosts file). `sites`: the registry
/// a commit handed on ([`EnvironmentSync::sync_sites`]), if any.
pub(crate) async fn sync_without_elevation(env: &dyn EnvironmentSync, sites: Option<SiteSnapshot>, ctx: &CallContext) -> AppResult<()> {
    let ctx = CallContext { origin: ctx.origin, elevation: ElevationPolicy::Deny };
    let synced = match sites {
        Some(sites) => env.sync_sites(sites, &ctx).await,
        None => env.sync(&ctx).await,
    };
    match synced {
        Ok(_) => Ok(()),
        Err(e) if e.code == ErrorCode::ElevationRequired => {
            tracing::info!("hosts file left unchanged: {}", e.message);
            Ok(())
        }
        Err(e) => Err(e),
    }
}
