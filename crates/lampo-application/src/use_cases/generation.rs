//! Pure builders turning the configuration (projects, worktree environments, settings,
//! installed packages) into the models the adapters render: php.ini specs, the shim index, the
//! Apache configuration and service launches. No port is called here.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use lampo_domain::logs::LogSource;
use lampo_domain::package::{EntryRole, InstalledPackage, PackageKind};
use lampo_domain::paths::LampoPaths;
use lampo_domain::php::{ExtensionName, PhpIniOverrides, PhpMinor, php_ini_overrides};
use lampo_domain::ports::ServiceRunner;
use lampo_domain::project::{Project, ProjectEnvironment, ProjectId};
use lampo_domain::service::{ServiceId, ServiceKind};
use lampo_domain::settings::Settings;
use lampo_domain::shim::{SHIM_INDEX_VERSION, ShimEntry, ShimIndex, ShimTarget};
use lampo_domain::site::SiteId;
use lampo_domain::web::{ExtensionLoad, PhpIniSpec, VirtualHost, WebServerConfig, is_zend_extension, merge_ini, vhost_addressing};
use lampo_domain::worktree::WorktreeEnvironment;

use super::service_catalog::ServiceCatalog;
use super::support::{SiteRef, all_sites, composer_phar_for, installed_php_minors, newest_of, php_dir, php_package};

/// Lampo's base php.ini values; project overrides win (`merge_ini`).
const BASE_INI: &[(&str, &str)] = &[
    ("display_errors", "On"),
    ("display_startup_errors", "On"),
    ("log_errors", "On"),
    ("error_reporting", "E_ALL"),
    ("memory_limit", "256M"),
    ("max_execution_time", "120"),
    ("upload_max_filesize", "64M"),
    ("post_max_size", "64M"),
];

/// Extensions of the php.ini used by the shims outside any project.
const DEFAULT_CLI_EXTENSIONS: &[&str] = &["curl", "fileinfo", "mbstring", "openssl", "pdo_mysql", "mysqli"];

/// Site id carried by the default CLI php.ini spec (the spec type requires one; the file goes to
/// `LampoPaths::default_php_ini_dir`).
pub(crate) fn default_ini_site() -> SiteId {
    SiteId::Project(ProjectId::new("default").expect("valid slug"))
}

/// Everything the generators read, loaded once per sync.
#[derive(Debug, Clone)]
pub(crate) struct ConfigSnapshot {
    pub projects: Vec<Project>,
    pub worktrees: Vec<WorktreeEnvironment>,
    pub settings: Settings,
    pub installed: Vec<InstalledPackage>,
}

impl ConfigSnapshot {
    pub fn sites(&self) -> Vec<SiteRef<'_>> {
        all_sites(&self.projects, &self.worktrees)
    }

    /// The PHP used outside projects: the configured default if installed, else the newest.
    pub fn default_php(&self) -> Option<PhpMinor> {
        let installed = installed_php_minors(&self.installed);
        self.settings.default_php.filter(|p| installed.contains(p)).or_else(|| installed.last().copied())
    }
}

pub(crate) fn base_ini() -> PhpIniOverrides {
    php_ini_overrides(BASE_INI.iter().copied()).expect("built-in ini values are valid")
}

pub(crate) fn bundled(name: &ExtensionName) -> ExtensionLoad {
    ExtensionLoad { name: name.clone(), zend: is_zend_extension(name), dll: None }
}

/// One php.ini per site (projects active or not, so the shims get the right extensions, and
/// every worktree environment), plus the default CLI one. Sites whose PHP is not installed are
/// skipped with a warning.
pub(crate) fn php_ini_specs(snap: &ConfigSnapshot, paths: &LampoPaths, warnings: &mut Vec<String>) -> Vec<PhpIniSpec> {
    let mut specs = Vec::new();
    for site in snap.sites() {
        let env = site.environment();
        let Some(dir) = php_dir(&snap.installed, env.php) else {
            warnings.push(format!("PHP {} is not installed: `{}` cannot run until it is", env.php, site.id));
            continue;
        };
        let mut extensions: Vec<ExtensionLoad> = env.extensions.iter().map(bundled).collect();
        for addon in &env.addons {
            let found = newest_of(&snap.installed, addon, None, Some(env.php));
            match found.and_then(|pkg| Some((pkg.php_extension.clone()?, pkg.entry(EntryRole::PhpExtension)?))) {
                Some((name, dll)) => extensions.push(ExtensionLoad { zend: is_zend_extension(&name), dll: Some(dll.to_path_buf()), name }),
                _ => warnings.push(format!("{addon} for PHP {} is not installed: `{}` runs without it", env.php, site.id)),
            }
        }
        specs.push(PhpIniSpec {
            site: site.id.clone(),
            php: env.php,
            php_dir: dir,
            output_dir: paths.php_ini_dir(&site.id),
            extensions,
            values: merge_ini(&base_ini(), &env.php_ini),
            error_log: LogSource::SitePhp(site.id.clone()).file(paths),
        });
    }
    if let Some(php) = snap.default_php() {
        let dir = php_dir(&snap.installed, php).expect("default PHP is installed");
        let extensions = DEFAULT_CLI_EXTENSIONS.iter().map(|n| bundled(&ExtensionName::new(n).expect("valid"))).collect();
        specs.push(PhpIniSpec {
            site: default_ini_site(),
            php,
            php_dir: dir,
            output_dir: paths.default_php_ini_dir(),
            extensions,
            values: base_ini(),
            error_log: paths.logs_dir().join("php-cli").join("php_errors.log"),
        });
    }
    specs
}

fn shim_target(snap: &ConfigSnapshot, catalog: &ServiceCatalog, php: PhpMinor, phprc_dir: PathBuf, env: Option<&ProjectEnvironment>, database: Option<String>) -> Option<ShimTarget> {
    let package = php_package(&snap.installed, php)?;
    Some(ShimTarget {
        php,
        php_dir: package.install_dir.clone(),
        php_exe: package.entry(EntryRole::Cli)?.to_path_buf(),
        phprc_dir: Some(phprc_dir),
        composer_phar: composer_phar_for(&snap.installed, php),
        composer_channel: php.composer_channel(),
        services: catalog.shim_services(env, database, &snap.settings, &snap.installed),
    })
}

/// The shim index: every project and worktree environment whose PHP is installed, plus the
/// default target. A site gets its own instance of each service it uses (none if that instance
/// is not installed) and the default instance of the others (`ServiceCatalog::shim_services`).
pub(crate) fn shim_index(snap: &ConfigSnapshot, catalog: &ServiceCatalog, paths: &LampoPaths) -> ShimIndex {
    let projects = snap
        .sites()
        .into_iter()
        .filter_map(|site| {
            let env = site.environment();
            let target = shim_target(snap, catalog, env.php, paths.php_ini_dir(&site.id), Some(env), site.database().map(|d| d.to_string()))?;
            Some(ShimEntry { site: site.id.clone(), root: site.root().clone(), target })
        })
        .collect();
    let default = snap.default_php().and_then(|php| shim_target(snap, catalog, php, paths.default_php_ini_dir(), None, None));
    ShimIndex { version: SHIM_INDEX_VERSION, default, projects }
}

/// The Apache configuration for every served site whose PHP is installed (`None` when nothing is
/// served or Apache / mod_fcgid are missing, with a warning).
pub(crate) fn web_config(snap: &ConfigSnapshot, paths: &LampoPaths, warnings: &mut Vec<String>) -> Option<WebServerConfig> {
    let served: Vec<SiteRef<'_>> = snap.sites().into_iter().filter(SiteRef::served).collect();
    if served.is_empty() {
        return None;
    }
    let Some(apache) = newest_of(&snap.installed, &PackageKind::APACHE, None, None) else {
        warnings.push("Apache is not installed: no site can be served".into());
        return None;
    };
    let Some(mod_fcgid) = newest_of(&snap.installed, &PackageKind::MOD_FCGID, None, None).and_then(|p| p.entry(EntryRole::ApacheModule)).map(Path::to_path_buf) else {
        warnings.push("mod_fcgid is not installed: no site can be served".into());
        return None;
    };
    let vhosts = served
        .iter()
        .filter_map(|site| {
            let php_cgi = php_package(&snap.installed, site.environment().php)?.entry(EntryRole::FastCgi)?.to_path_buf();
            let (server_names, loopback_port) = vhost_addressing(&site.addresses());
            Some(VirtualHost {
                site: site.id.clone(),
                server_names,
                loopback_port,
                project_root: site.root().as_path().to_path_buf(),
                document_root: site.document_root(),
                php_cgi,
                phprc_dir: paths.php_ini_dir(&site.id),
                custom_directives: site.environment().apache_directives.clone(),
                tls: site.tls(),
                error_log: LogSource::SiteError(site.id.clone()).file(paths),
                access_log: LogSource::SiteAccess(site.id.clone()).file(paths),
            })
        })
        .collect();
    Some(WebServerConfig {
        server_root: apache.install_dir.clone(),
        mod_fcgid,
        listen_port: snap.settings.apache_port,
        config_dir: paths.apache_config_dir(),
        log_dir: paths.service_log_dir(&ServiceId::apache()),
        vhosts,
    })
}

/// The runner registered for `kind` (routing by `ServiceRunner::kind`, open-closed).
pub(crate) fn runner_for<'a>(runners: &'a [Arc<dyn ServiceRunner>], kind: &ServiceKind) -> Option<&'a Arc<dyn ServiceRunner>> {
    runners.iter().find(|r| &r.kind() == kind)
}
