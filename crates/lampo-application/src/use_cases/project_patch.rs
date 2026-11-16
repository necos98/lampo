//! Partial edits of a project (`lampo project set`, `lampo project rename`): PHP minor,
//! extensions, add-ons, php.ini overrides, services, database name, public folder, Apache
//! directives, environment variables, display name and addresses.
//!
//! The patch is applied to the *stored* project inside [`ProjectWriter::modify`] (under the
//! allocator lock), so it never overwrites a concurrent edit with stale data the way a
//! read-modify-write of the full [`UpdateProjectRequest`] from another process would. It goes
//! through the same rules as the GUI's update (`Project::set_environment`, the
//! [`ProjectEnvironmentPolicy`](super::project_policy::ProjectEnvironmentPolicy) of every save,
//! `check_conflicts`) and the same sync afterwards.

use std::collections::BTreeSet;
use std::sync::Arc;

use lampo_domain::address::SiteAddress;
use lampo_domain::mysql::DatabaseName;
use lampo_domain::package::PackageKind;
use lampo_domain::php::{ExtensionName, IniKey, PhpMinor, extensions_for_php, php_ini_overrides};
use lampo_domain::ports::InstalledPackageRegistry;
use lampo_domain::env_var::EnvVarName;
use lampo_domain::project::{ApacheDirectives, DisplayName, Project, ProjectEnvVars, ProjectEnvironment, PublicDir};
use lampo_domain::service::ServiceKind;
use lampo_domain::settings::Settings;
use lampo_domain::tcp_port::Port;
use lampo_domain::version::VersionSeries;

use super::allocation::SiteSnapshot;
use super::project_policy::{normalize_extensions, require_mysql_for_database};
use super::project_writer::ProjectWriter;
use super::projects::{require_hosts_elevation, require_php_installed};
use super::service_catalog::ServiceCatalog;
use super::support::validation;
use crate::context::CallContext;
use crate::dto::*;
use crate::error::{AppError, AppResult};

/// Applies a [`ProjectPatchRequest`].
pub struct PatchProjectEnvironment {
    writer: ProjectWriter,
    installed: Arc<dyn InstalledPackageRegistry>,
}

impl PatchProjectEnvironment {
    pub fn new(writer: ProjectWriter, installed: Arc<dyn InstalledPackageRegistry>) -> Self {
        Self { writer, installed }
    }

    /// `NotFound` for an unknown project; `PackageMissing` for a PHP minor that is not
    /// installed; `Validation` for malformed values, an item both added
    /// and removed, a bundled extension the project's PHP does not ship, an unknown or
    /// unconfigured service (version), or a database name without MySQL; `PackageMissing` for
    /// an add-on (added, or kept across a PHP switch) not installed for the project's PHP minor
    /// (the message names the install
    /// command). Display name and addresses: `Validation` for a malformed one or removing every
    /// address, `Conflict` for a hostname or port another site uses (`check_conflicts`),
    /// `ElevationRequired` when hosts-file names change without elevation. Nothing is saved on
    /// error. Then syncs like every project update.
    pub async fn execute(&self, request: ProjectPatchRequest, ctx: &CallContext) -> AppResult<ProjectDto> {
        let patch = Patch::parse(&request)?;
        if let Some(php) = patch.php {
            require_php_installed(&self.writer, self.installed.as_ref(), &request.project_id, php)?;
        }
        let catalog = self.writer.services().clone();
        let change = |project: &mut Project, snap: &SiteSnapshot, free_port: Option<Port>| -> AppResult<bool> {
            let mut env = project.environment.clone();
            patch.apply(&mut env, catalog.as_ref(), &snap.settings)?;
            if patch.database.is_some() {
                require_mysql_for_database(&env)?;
            }
            let mut addresses = project.addresses().to_vec();
            patch.apply_addresses(&mut addresses, free_port)?;
            let hosts_before: BTreeSet<String> = project.hosts_file_names().map(ToString::to_string).collect();
            let before = (project.name.clone(), project.addresses().to_vec(), project.environment.clone(), project.database().cloned());
            project.set_addresses(addresses)?;
            let hosts_after: BTreeSet<String> = project.hosts_file_names().map(ToString::to_string).collect();
            let touched: Vec<String> = hosts_before.symmetric_difference(&hosts_after).cloned().collect();
            require_hosts_elevation(ctx, &touched, "this address change", "nothing was changed")?;
            project.set_environment(env)?;
            if let Some(db) = &patch.database {
                project.set_database_name(db.clone());
            }
            if let Some(name) = &patch.name {
                project.name = name.clone();
            }
            Ok((project.name.clone(), project.addresses().to_vec(), project.environment.clone(), project.database().cloned()) != before)
        };
        let saved = if patch.wants_free_port() {
            self.writer.modify_with_free_port(&request.project_id, |project, snap, port| change(project, snap, Some(port))).await?
        } else {
            self.writer.modify(&request.project_id, |project, snap| change(project, snap, None)).await?
        };
        self.writer.finish(saved, ctx).await
    }
}

/// The request, parsed and checked for contradictions before taking the lock.
struct Patch {
    php: Option<PhpMinor>,
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
    env_set: ProjectEnvVars,
    env_unset: Vec<EnvVarName>,
    name: Option<DisplayName>,
    /// In request order (appended in that order).
    addresses_add: Vec<AddressArg>,
    addresses_remove: BTreeSet<AddressArg>,
}

/// An address named in a patch: an exact one, or [`AUTO_LOOPBACK`].
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum AddressArg {
    Exact(SiteAddress),
    AnyLoopback,
}

impl AddressArg {
    fn parse(raw: &str) -> AppResult<Self> {
        if raw.trim().eq_ignore_ascii_case(AUTO_LOOPBACK) {
            return Ok(Self::AnyLoopback);
        }
        Ok(Self::Exact(SiteAddress::parse(raw)?))
    }

    fn matches(&self, address: &SiteAddress) -> bool {
        match self {
            Self::Exact(exact) => exact == address,
            Self::AnyLoopback => address.loopback().is_some(),
        }
    }
}

impl std::fmt::Display for AddressArg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Exact(a) => write!(f, "{a}"),
            Self::AnyLoopback => f.write_str(AUTO_LOOPBACK),
        }
    }
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
            php: r.php_version.as_deref().map(PhpMinor::parse).transpose()?,
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
            env_set: ProjectEnvVars::from_pairs(r.env_set.iter().map(|(name, value)| (name.as_str(), value.clone())))?,
            env_unset: r.env_unset.iter().map(EnvVarName::existing).collect::<Result<_, _>>()?,
            name: r.name.as_deref().map(DisplayName::new).transpose()?,
            addresses_add: r.addresses_add.iter().map(|a| AddressArg::parse(a)).collect::<AppResult<_>>()?,
            addresses_remove: r.addresses_remove.iter().map(|a| AddressArg::parse(a)).collect::<AppResult<_>>()?,
        };
        disjoint("address", &patch.addresses_add.iter().cloned().collect(), &patch.addresses_remove)?;
        disjoint("extension", &patch.extensions_add, &patch.extensions_remove)?;
        disjoint("add-on", &patch.addons_add, &patch.addons_remove)?;
        let ini_set: BTreeSet<IniKey> = patch.ini_set.keys().cloned().collect();
        disjoint("php.ini key", &ini_set, &patch.ini_unset)?;
        let services_add: BTreeSet<ServiceKind> = patch.services_add.iter().map(|(k, _)| k.clone()).collect();
        disjoint("service", &services_add, &patch.services_remove)?;
        if services_add.len() != patch.services_add.len() {
            return Err(validation("a service kind is added twice"));
        }
        if let Some(both) = patch.env_unset.iter().find(|name| patch.env_set.get(name.as_str()).is_some()) {
            return Err(validation(format!("environment variable `{both}` is both set and removed")));
        }
        Ok(patch)
    }

    /// The PHP minor first (the save checks what follows against it), then removals, then
    /// additions and replacements. Extensions are compared as that PHP names them, so removing
    /// `gd` also removes a stored `gd2`.
    fn apply(&self, env: &mut ProjectEnvironment, catalog: &ServiceCatalog, settings: &Settings) -> AppResult<()> {
        if let Some(php) = self.php {
            env.php = php;
        }
        normalize_extensions(env);
        let removed = extensions_for_php(&self.extensions_remove, env.php);
        env.extensions.retain(|e| !removed.contains(e));
        env.extensions.extend(extensions_for_php(&self.extensions_add, env.php));
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
        for name in &self.env_unset {
            env.env_vars.remove(name.as_str());
        }
        for (name, value) in self.env_set.iter() {
            env.env_vars.set(name.clone(), value.clone());
        }
        Ok(())
    }

    /// Whether an address must be allocated: [`AUTO_LOOPBACK`] added.
    fn wants_free_port(&self) -> bool {
        self.addresses_add.contains(&AddressArg::AnyLoopback)
    }

    /// Removals first, then additions appended in order (the first address, the primary one,
    /// stays first unless removed: then the next one becomes primary, as in the GUI).
    /// `free_port` is set when [`Self::wants_free_port`].
    fn apply_addresses(&self, addresses: &mut Vec<SiteAddress>, free_port: Option<Port>) -> AppResult<()> {
        if self.addresses_add.is_empty() && self.addresses_remove.is_empty() {
            return Ok(());
        }
        addresses.retain(|a| !self.addresses_remove.iter().any(|r| r.matches(a)));
        for wanted in &self.addresses_add {
            let address = match wanted {
                AddressArg::Exact(a) => a.clone(),
                AddressArg::AnyLoopback if addresses.iter().any(|a| a.loopback().is_some()) => continue,
                AddressArg::AnyLoopback => SiteAddress::loopback_port(free_port.ok_or_else(|| AppError::internal("no loopback port was allocated"))?)?,
            };
            if !addresses.contains(&address) {
                addresses.push(address);
            }
        }
        if addresses.is_empty() {
            return Err(validation("a project needs at least one address: add another one (`--address`) before removing the last"));
        }
        if addresses.iter().filter(|a| a.loopback().is_some()).count() > 1 {
            return Err(validation(format!("a project can have at most one dedicated loopback port: remove the current one too (`--no-address {AUTO_LOOPBACK}`)")));
        }
        Ok(())
    }
}

/// The version a project uses of `kind`: none for unversioned kinds (`redis`); for versioned
/// ones (`mysql`) the requested series (the save's policy refuses one the settings do not
/// configure), else the default instance (the always-on one).
fn service_version(catalog: &ServiceCatalog, settings: &Settings, kind: &ServiceKind, requested: Option<VersionSeries>) -> AppResult<Option<VersionSeries>> {
    let known = || catalog.descriptors().iter().map(|d| d.kind()).filter(|k| k != &ServiceKind::APACHE).map(|k| k.to_string()).collect::<Vec<_>>().join(", ");
    let descriptor = catalog.descriptor(kind).ok_or_else(|| validation(format!("unknown service kind `{kind}` (known: {})", known())))?;
    if !descriptor.versioned() || requested.is_some() {
        // Checked by the save: the catalog refuses a version of an unversioned kind, the
        // policy an instance the settings do not configure.
        return Ok(requested);
    }
    catalog
        .default_instance(kind, settings)
        .and_then(|id| id.version())
        .map(Some)
        .ok_or_else(|| validation(format!("no {} instance is configured: add one in the Lampo settings", descriptor.display_name())))
}
