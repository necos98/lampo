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
//! presets and exports are written for other builds: refusing those would block a legitimate
//! create, import or PHP switch for nothing. Names the builds changed across minors (GD is `gd2`
//! before PHP 8.0) are not a problem at all: [`normalize_extensions`] stores them as the
//! project's PHP names them, and a module compiled into the build (`bcmath`) is accepted with a
//! notice ([`ProjectEnvironmentPolicy::notices`]) and left out of php.ini.
//!
//! [`ProjectWriter`]: super::project_writer::ProjectWriter

use std::collections::BTreeSet;
use std::sync::Arc;

use lampo_domain::package::PackageKind;
use lampo_domain::php::{ExtensionName, PhpMinor, extensions_for_php};
use lampo_domain::ports::InstalledPackageRegistry;
use lampo_domain::project::ProjectEnvironment;
use lampo_domain::service::ServiceId;
use lampo_domain::settings::Settings;

use super::php_extensions::PhpExtensionsQuery;
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
    /// Always: its custom Apache directives and environment variables (a stored project may
    /// predate a rule: it loads, but is never saved again unfixed) and its service kinds and
    /// versions. For what it introduces:
    /// `Validation` for an instance the settings do not configure, `PackageMissing` for an add-on
    /// not installed for its PHP minor (a PHP switch re-checks every add-on). `Validation` for a
    /// bundled extension added to an existing project that its PHP build neither ships nor has
    /// compiled in (a stored name renamed for a new PHP minor is not an addition).
    pub(crate) fn check(&self, stored: Option<&ProjectEnvironment>, candidate: &ProjectEnvironment, settings: &Settings) -> AppResult<()> {
        candidate.apache_directives.validate()?;
        candidate.env_vars.validate()?;
        self.services.validate_environment(candidate)?;
        let stored_services: Vec<ServiceId> = stored.map(|s| s.service_ids().collect()).unwrap_or_default();
        self.check_instances(candidate.service_ids().filter(|id| !stored_services.contains(id)), settings)?;
        let same_php = stored.filter(|s| s.php == candidate.php);
        self.check_addons(&introduced(&candidate.addons, same_php.map(|s| &s.addons)), candidate.php)?;
        match stored {
            Some(stored) => self.check_bundled(&introduced(&candidate.extensions, Some(&extensions_for_php(&stored.extensions, candidate.php))), candidate.php),
            None => Ok(()),
        }
    }

    /// What a saved `env` should be told: each extension it names that is compiled into its PHP
    /// build, so nothing is enabled for it. Empty when the build cannot tell (PHP not installed).
    pub(crate) fn notices(&self, env: &ProjectEnvironment) -> Vec<String> {
        let names = extensions_for_php(&env.extensions, env.php);
        match self.extensions.installed_build_extensions(env.php, &names) {
            Ok(Some(build)) => build.compiled_in(&names).iter().map(|name| format!("{name} is built into PHP {}, nothing to enable", env.php)).collect(),
            Ok(None) => Vec::new(),
            Err(e) => {
                tracing::warn!("cannot read the extensions of PHP {}: {}", env.php, e.message);
                Vec::new()
            }
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

    /// Bundled extensions must exist in the project's PHP build, as a DLL (`ext\php_<name>.dll`)
    /// or compiled in. Skipped when the build cannot be scanned (PHP not installed: the sync
    /// reports that).
    fn check_bundled(&self, names: &BTreeSet<&ExtensionName>, php: PhpMinor) -> AppResult<()> {
        if names.is_empty() {
            return Ok(());
        }
        let wanted: BTreeSet<ExtensionName> = names.iter().map(|n| (*n).clone()).collect();
        let Ok(Some(build)) = self.extensions.installed_build_extensions(php, &wanted) else { return Ok(()) };
        let unknown: Vec<String> = wanted.iter().filter(|n| !build.loadable.contains(n) && !build.builtin.contains(n)).map(|n| format!("`{n}`")).collect();
        if unknown.is_empty() {
            return Ok(());
        }
        let list: Vec<&str> = build.loadable.iter().map(ExtensionName::as_str).collect();
        Err(validation(format!("PHP {php} has no bundled extension {} (available: {}); add-on packages such as xdebug go in `--addon`", unknown.join(", "), list.join(", "))))
    }
}

/// A database name needs MySQL: a project without it has no database (`Validation`).
pub(crate) fn require_mysql_for_database(env: &ProjectEnvironment) -> AppResult<()> {
    match env.mysql() {
        Some(_) => Ok(()),
        None => Err(validation("the project uses no MySQL: add the MySQL service to set a database name")),
    }
}

/// Stores `env`'s extensions as its PHP builds name them (`gd` ↔ `gd2`), so a project switched to
/// another minor keeps loading them. Every save runs it before the checks.
pub(crate) fn normalize_extensions(env: &mut ProjectEnvironment) {
    env.extensions = extensions_for_php(&env.extensions, env.php);
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
    .with_details(serde_json::json!({ "package": kind.to_string(), "php": php.to_string() }))
}
