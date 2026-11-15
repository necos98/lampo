//! Partial edits of a project's environment (`lampo project set`): extensions, add-ons, php.ini
//! overrides, services, database name, public folder, Apache directives.
//!
//! The patch is applied to the *stored* project inside [`ProjectWriter::modify`] (under the
//! allocator lock), so it never overwrites a concurrent edit with stale data the way a
//! read-modify-write of the full [`UpdateProjectRequest`] from another process would. It goes
//! through the same domain rules as the GUI's update (`Project::set_environment`, the service
//! catalog's validation, `check_conflicts`) and the same sync afterwards.

use std::collections::BTreeSet;
use std::sync::Arc;

use lampo_domain::mysql::DatabaseName;
use lampo_domain::package::PackageKind;
use lampo_domain::php::{ExtensionName, IniKey, PhpMinor, php_ini_overrides};
use lampo_domain::ports::InstalledPackageRegistry;
use lampo_domain::project::{ApacheDirectives, ProjectEnvironment, PublicDir};
use lampo_domain::service::ServiceKind;
use lampo_domain::settings::Settings;
use lampo_domain::version::VersionSeries;

use super::packages::PhpExtensionsQuery;
use super::projects::ProjectWriter;
use super::service_catalog::ServiceCatalog;
use super::support::{newest_of, validation};
use crate::context::CallContext;
use crate::dto::*;
use crate::error::{AppError, AppResult, ErrorCode};

/// Applies a [`ProjectPatchRequest`].
pub struct PatchProjectEnvironment {
    writer: ProjectWriter,
    installed: Arc<dyn InstalledPackageRegistry>,
    extensions: Arc<PhpExtensionsQuery>,
}

impl PatchProjectEnvironment {
    pub fn new(writer: ProjectWriter, installed: Arc<dyn InstalledPackageRegistry>, extensions: Arc<PhpExtensionsQuery>) -> Self {
        Self { writer, installed, extensions }
    }

    /// `NotFound` for an unknown project; `Validation` for malformed values, an item both added
    /// and removed, a bundled extension the project's PHP does not ship, an unknown or
    /// unconfigured service (version), or a database name without MySQL; `PackageMissing` for
    /// an add-on not installed for the project's PHP minor (the message names the install
    /// command). Nothing is saved on error. Then syncs like every project update.
    pub async fn execute(&self, request: ProjectPatchRequest, ctx: &CallContext) -> AppResult<ProjectDto> {
        let patch = Patch::parse(&request)?;
        let installed = if patch.addons_add.is_empty() { Vec::new() } else { self.installed.list()? };
        let catalog = self.writer.services.clone();
        let project = self
            .writer
            .modify(&request.project_id, |project, snap| {
                let mut env = project.environment.clone();
                patch.apply(&mut env, catalog.as_ref(), &snap.settings)?;
                for kind in &patch.addons_add {
                    if newest_of(&installed, kind, None, Some(env.php)).is_none() {
                        return Err(addon_missing(kind, env.php));
                    }
                }
                self.check_bundled(&patch.extensions_add, env.php)?;
                if patch.database.is_some() && env.mysql().is_none() {
                    return Err(validation("the project uses no MySQL: add it with `--service mysql` to set a database name"));
                }
                let before = (project.environment.clone(), project.database.clone());
                project.set_environment(env)?;
                if let Some(db) = &patch.database {
                    project.database = Some(db.clone());
                }
                Ok((project.environment.clone(), project.database.clone()) != before)
            })
            .await?;
        self.writer.finish(&project, ctx).await
    }

    /// Bundled extensions must exist in the project's PHP build (`ext\php_<name>.dll`). Skipped
    /// when the build cannot be scanned (PHP not installed: the sync reports that).
    fn check_bundled(&self, names: &BTreeSet<ExtensionName>, php: PhpMinor) -> AppResult<()> {
        if names.is_empty() {
            return Ok(());
        }
        let Ok(available) = self.extensions.list(&php.to_string()) else { return Ok(()) };
        let unknown: Vec<&str> = names.iter().map(ExtensionName::as_str).filter(|n| !available.iter().any(|a| a.name == *n)).collect();
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

fn addon_missing(kind: &PackageKind, php: PhpMinor) -> AppError {
    AppError::new(
        ErrorCode::PackageMissing,
        format!("{kind} for PHP {php} is not installed: run `lampo package install {kind} --php {php}` first (add-ons are PHP-pinned packages, see `lampo package list --kind {kind}`)"),
    )
}

/// The request, parsed and checked for contradictions before taking the lock.
struct Patch {
    extensions_add: BTreeSet<ExtensionName>,
    extensions_remove: BTreeSet<ExtensionName>,
    addons_add: BTreeSet<PackageKind>,
    addons_remove: BTreeSet<PackageKind>,
    ini_set: lampo_domain::php::PhpIniOverrides,
    ini_unset: BTreeSet<IniKey>,
    services_add: Vec<(ServiceKind, Option<VersionSeries>)>,
    services_remove: BTreeSet<ServiceKind>,
    database: Option<DatabaseName>,
    public_dir: Option<PublicDir>,
    apache_directives: Option<ApacheDirectives>,
}

/// Rejects an item named on both sides of a patch.
fn disjoint<T: Ord + std::fmt::Display>(what: &str, add: &BTreeSet<T>, remove: &BTreeSet<T>) -> AppResult<()> {
    match add.intersection(remove).next() {
        Some(both) => Err(validation(format!("{what} `{both}` is both added and removed"))),
        None => Ok(()),
    }
}

impl Patch {
    fn parse(r: &ProjectPatchRequest) -> AppResult<Self> {
        let extensions = |list: &[String]| list.iter().map(ExtensionName::new).collect::<Result<BTreeSet<_>, _>>();
        let kinds = |list: &[String]| list.iter().map(|k| PackageKind::new(k.trim().to_ascii_lowercase())).collect::<Result<BTreeSet<_>, _>>();
        let services = |list: &[String]| list.iter().map(|k| ServiceKind::new(k.trim().to_ascii_lowercase())).collect::<Result<BTreeSet<_>, _>>();
        let patch = Self {
            extensions_add: extensions(&r.extensions_add)?,
            extensions_remove: extensions(&r.extensions_remove)?,
            addons_add: kinds(&r.addons_add)?,
            addons_remove: kinds(&r.addons_remove)?,
            ini_set: php_ini_overrides(r.ini_set.iter().map(|(k, v)| (k.as_str(), v.clone())))?,
            ini_unset: r.ini_unset.iter().map(IniKey::new).collect::<Result<_, _>>()?,
            services_add: r
                .services_add
                .iter()
                .map(|s| Ok((ServiceKind::new(s.kind.trim().to_ascii_lowercase())?, s.version.as_deref().map(str::trim).filter(|v| !v.is_empty()).map(VersionSeries::parse).transpose()?)))
                .collect::<AppResult<_>>()?,
            services_remove: services(&r.services_remove)?,
            database: r.database_name.as_deref().map(|d| DatabaseName::new(d.trim())).transpose()?,
            public_dir: r.public_dir.as_deref().map(PublicDir::new).transpose()?,
            apache_directives: r.apache_directives.clone().map(ApacheDirectives::new).transpose()?,
        };
        disjoint("extension", &patch.extensions_add, &patch.extensions_remove)?;
        disjoint("add-on", &patch.addons_add, &patch.addons_remove)?;
        let ini_set: BTreeSet<IniKey> = patch.ini_set.keys().cloned().collect();
        disjoint("php.ini key", &ini_set, &patch.ini_unset)?;
        let services_add: BTreeSet<ServiceKind> = patch.services_add.iter().map(|(k, _)| k.clone()).collect();
        disjoint("service", &services_add, &patch.services_remove)?;
        if services_add.len() != patch.services_add.len() {
            return Err(validation("a service kind is added twice"));
        }
        Ok(patch)
    }

    /// Removals first, then additions and replacements.
    fn apply(&self, env: &mut ProjectEnvironment, catalog: &ServiceCatalog, settings: &Settings) -> AppResult<()> {
        env.extensions.retain(|e| !self.extensions_remove.contains(e));
        env.extensions.extend(self.extensions_add.iter().cloned());
        env.addons.retain(|a| !self.addons_remove.contains(a));
        env.addons.extend(self.addons_add.iter().cloned());
        env.php_ini.retain(|k, _| !self.ini_unset.contains(k));
        env.php_ini.extend(self.ini_set.iter().map(|(k, v)| (k.clone(), v.clone())));
        for kind in &self.services_remove {
            env.drop_service(kind);
        }
        for (kind, version) in &self.services_add {
            let version = service_version(catalog, settings, kind, *version)?;
            env.use_service(kind.clone(), version);
        }
        if let Some(dir) = &self.public_dir {
            env.public_dir = dir.clone();
        }
        if let Some(directives) = &self.apache_directives {
            env.apache_directives = directives.clone();
        }
        Ok(())
    }
}

/// The version a project uses of `kind`: none for unversioned kinds (`redis`); for versioned
/// ones (`mysql`) the requested series, which must be a configured instance, else the default
/// instance (the always-on one).
fn service_version(catalog: &ServiceCatalog, settings: &Settings, kind: &ServiceKind, requested: Option<VersionSeries>) -> AppResult<Option<VersionSeries>> {
    let known = || catalog.descriptors().iter().map(|d| d.kind()).filter(|k| k != &ServiceKind::APACHE).map(|k| k.to_string()).collect::<Vec<_>>().join(", ");
    let descriptor = catalog.descriptor(kind).ok_or_else(|| validation(format!("unknown service kind `{kind}` (known: {})", known())))?;
    if !descriptor.versioned() {
        // A version given for an unversioned kind is rejected by the catalog's validation.
        return Ok(requested);
    }
    let configured: Vec<VersionSeries> = settings.services.iter().filter(|s| s.service.kind() == kind).filter_map(|s| s.service.version()).collect();
    let list = || configured.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ");
    match requested {
        Some(v) if configured.contains(&v) => Ok(Some(v)),
        Some(v) => Err(validation(format!("{} {v} is not a configured instance (configured: {}); add it in the Lampo settings", descriptor.display_name(), list()))),
        None => catalog
            .default_instance(kind, settings)
            .and_then(|id| id.version())
            .map(Some)
            .ok_or_else(|| validation(format!("no {} instance is configured: add one in the Lampo settings", descriptor.display_name()))),
    }
}
