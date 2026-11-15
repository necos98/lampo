//! The environment rules every project save enforces, whichever front-end made the change: the
//! GUI's full update, `lampo project set`, create, import, the PHP switch. [`ProjectWriter`]
//! runs them in its single save path, so no use case can skip one.
//!
//! A rule that depends on what is installed or configured right now (a MySQL instance, an add-on
//! build, a bundled extension) is checked for what the save *introduces*: a project stored before
//! a package went away still loads and can still be renamed or switched off, but a save never
//! adds something that would make every sync fail.
//!
//! Bundled extensions are the exception: a name is checked when a save adds it to an existing
//! project (a mistyped `intll`), not for a new project nor on a PHP switch. A DLL the build lacks
//! never breaks the site (its php.ini line is commented out and the sync reports it), while
//! presets and exports are written for other builds and builds name some extensions differently
//! across minors (GD is `gd2` before PHP 8.0): refusing those would block a legitimate create,
//! import or PHP switch for nothing.
//!
//! [`ProjectWriter`]: super::project_writer::ProjectWriter

use std::collections::BTreeSet;
use std::sync::Arc;

use lampo_domain::package::PackageKind;
use lampo_domain::php::{ExtensionName, PhpMinor};
use lampo_domain::ports::InstalledPackageRegistry;
use lampo_domain::project::ProjectEnvironment;
use lampo_domain::service::ServiceId;
use lampo_domain::settings::Settings;

use super::packages::PhpExtensionsQuery;
use super::service_catalog::ServiceCatalog;
use super::support::{newest_of, validation};
use crate::error::{AppError, AppResult, ErrorCode};

pub struct ProjectEnvironmentPolicy {
    services: Arc<ServiceCatalog>,
    installed: Arc<dyn InstalledPackageRegistry>,
    extensions: Arc<PhpExtensionsQuery>,
}

impl ProjectEnvironmentPolicy {
    pub fn new(services: Arc<ServiceCatalog>, installed: Arc<dyn InstalledPackageRegistry>, extensions: Arc<PhpExtensionsQuery>) -> Self {
        Self { services, installed, extensions }
    }

    /// The registered service kinds: a project may only use these.
    pub(crate) fn services(&self) -> &Arc<ServiceCatalog> {
        &self.services
    }

    /// Checks `candidate` against `stored`, the saved environment (`None` for a new project).
    /// Always: its custom Apache directives (a stored project may predate a rule: it loads, but
    /// is never saved again unfixed) and its service kinds and versions. For what it introduces:
    /// `Validation` for an instance the settings do not configure, `PackageMissing` for an add-on
    /// not installed for its PHP minor (a PHP switch re-checks every add-on). `Validation` for a
    /// bundled extension added to an existing project that its PHP build does not ship.
    pub(crate) fn check(&self, stored: Option<&ProjectEnvironment>, candidate: &ProjectEnvironment, settings: &Settings) -> AppResult<()> {
        candidate.apache_directives.validate()?;
        self.services.validate_environment(candidate)?;
        let stored_services: Vec<ServiceId> = stored.map(|s| s.service_ids().collect()).unwrap_or_default();
        self.check_instances(candidate.service_ids().filter(|id| !stored_services.contains(id)), settings)?;
        let same_php = stored.filter(|s| s.php == candidate.php);
        self.check_addons(&introduced(&candidate.addons, same_php.map(|s| &s.addons)), candidate.php)?;
        match stored {
            Some(stored) => self.check_bundled(&introduced(&candidate.extensions, Some(&stored.extensions)), candidate.php),
            None => Ok(()),
        }
    }

    /// Every versioned service must be an instance of the settings (unversioned kinds always are).
    fn check_instances(&self, mut ids: impl Iterator<Item = ServiceId>, settings: &Settings) -> AppResult<()> {
        let configured = self.services.configured(settings);
        let Some(id) = ids.find(|id| !configured.contains(id)) else { return Ok(()) };
        let of_kind: Vec<String> = configured.iter().filter(|c| c.kind() == id.kind()).filter_map(ServiceId::version).map(|v| v.to_string()).collect();
        let list = if of_kind.is_empty() { "none".to_string() } else { of_kind.join(", ") };
        Err(validation(format!("{} is not a configured instance (configured: {list}); add it in the Lampo settings", self.services.label(&id))))
    }

    fn check_addons(&self, kinds: &BTreeSet<&PackageKind>, php: PhpMinor) -> AppResult<()> {
        if kinds.is_empty() {
            return Ok(());
        }
        let installed = self.installed.list()?;
        match kinds.iter().find(|kind| newest_of(&installed, kind, None, Some(php)).is_none()) {
            Some(kind) => Err(addon_missing(kind, php)),
            None => Ok(()),
        }
    }

    /// Bundled extensions must exist in the project's PHP build (`ext\php_<name>.dll`). Skipped
    /// when the build cannot be scanned (PHP not installed: the sync reports that).
    fn check_bundled(&self, names: &BTreeSet<&ExtensionName>, php: PhpMinor) -> AppResult<()> {
        if names.is_empty() {
            return Ok(());
        }
        let Ok(available) = self.extensions.list(&php.to_string()) else { return Ok(()) };
        let unknown: Vec<&str> = names.iter().map(|n| n.as_str()).filter(|n| !available.iter().any(|a| a.name == *n)).collect();
        if unknown.is_empty() {
            return Ok(());
        }
        let list: Vec<&str> = available.iter().map(|a| a.name.as_str()).collect();
        Err(validation(format!(
            "PHP {php} has no bundled extension {} (available: {}); add-on packages such as xdebug go in `--addon`",
            unknown.iter().map(|n| format!("`{n}`")).collect::<Vec<_>>().join(", "),
            list.join(", ")
        )))
    }
}

/// A database name needs MySQL: a project without it has no database (`Validation`).
pub(crate) fn require_mysql_for_database(env: &ProjectEnvironment) -> AppResult<()> {
    match env.mysql() {
        Some(_) => Ok(()),
        None => Err(validation("the project uses no MySQL: add the MySQL service to set a database name")),
    }
}

/// The items of `now` that `before` lacks (all of them without `before`).
fn introduced<'a, T: Ord>(now: &'a BTreeSet<T>, before: Option<&BTreeSet<T>>) -> BTreeSet<&'a T> {
    now.iter().filter(|item| before.is_none_or(|b| !b.contains(*item))).collect()
}

fn addon_missing(kind: &PackageKind, php: PhpMinor) -> AppError {
    AppError::new(
        ErrorCode::PackageMissing,
        format!("{kind} for PHP {php} is not installed: run `lampo package install {kind} --php {php}` first (add-ons are PHP-pinned packages, see `lampo package list --kind {kind}`)"),
    )
}
