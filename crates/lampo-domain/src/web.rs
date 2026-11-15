//! Web-server and PHP runtime configuration *models*. Rendering them to text (Apache templates,
//! php.ini) is an adapter concern (`lampo-infra-web`); building them from projects is done by the
//! application layer with the helpers below.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::address::SiteAddress;
use crate::hostname::Hostname;
use crate::php::{ExtensionName, PhpIniOverrides, PhpMinor};
use crate::project::{ApacheDirectives, TlsMode};
use crate::site::SiteId;

/// One generated `<VirtualHost>` (one file per project).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VirtualHost {
    pub site: SiteId,
    /// Name-based addresses on the shared listener (hosts-file names and `*.localhost`).
    /// First = `ServerName`, rest = `ServerAlias`. May be empty if only a loopback port is used.
    pub server_names: Vec<Hostname>,
    /// Dedicated `127.0.0.1:<port>` listener (`SiteAddress::LoopbackPort`), if any.
    pub loopback_port: Option<u16>,
    pub project_root: PathBuf,
    pub document_root: PathBuf,
    /// `php-cgi.exe` of the project's PHP minor (newest installed patch).
    pub php_cgi: PathBuf,
    /// Directory holding the project's generated `php.ini`; becomes `FcgidInitialEnv PHPRC`.
    pub phprc_dir: PathBuf,
    pub custom_directives: ApacheDirectives,
    pub tls: TlsMode,
    pub error_log: PathBuf,
    pub access_log: PathBuf,
}

/// The complete Apache configuration to apply atomically (validated with `httpd -t` first).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebServerConfig {
    /// Apache install dir (`ServerRoot`).
    pub server_root: PathBuf,
    /// `mod_fcgid.so` to load.
    pub mod_fcgid: PathBuf,
    pub listen_port: u16,
    /// Directory that receives `httpd.conf` and `vhosts\<project>.conf`.
    pub config_dir: PathBuf,
    pub log_dir: PathBuf,
    /// Active projects and every worktree environment, sorted by site id.
    pub vhosts: Vec<VirtualHost>,
}

impl WebServerConfig {
    /// Every port Apache must `Listen` on: the shared port plus dedicated loopback ports.
    pub fn listen_ports(&self) -> Vec<u16> {
        let mut ports = vec![self.listen_port];
        ports.extend(self.vhosts.iter().filter_map(|v| v.loopback_port));
        ports.sort_unstable();
        ports.dedup();
        ports
    }
}

/// How one extension is loaded in php.ini.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtensionLoad {
    pub name: ExtensionName,
    /// `zend_extension=` instead of `extension=` (Xdebug, OPcache).
    pub zend: bool,
    /// Absolute DLL path for add-ons living outside PHP's `ext` dir; `None` for bundled ones.
    pub dll: Option<PathBuf>,
}

/// Splits site addresses into vhost addressing: name-based names and an optional dedicated
/// loopback port. Works for `Project::addresses` and `WorktreeEnvironment::addresses()`.
pub fn vhost_addressing(addresses: &[SiteAddress]) -> (Vec<Hostname>, Option<u16>) {
    let names = addresses.iter().filter_map(|a| a.host().cloned()).collect();
    let port = addresses.iter().find_map(|a| a.loopback());
    (names, port)
}

/// Everything needed to render one project's php.ini.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhpIniSpec {
    pub site: SiteId,
    pub php: PhpMinor,
    /// PHP install dir (`extension_dir` = `<php_dir>\ext`).
    pub php_dir: PathBuf,
    /// Where `php.ini` is written (the PHPRC dir).
    pub output_dir: PathBuf,
    pub extensions: Vec<ExtensionLoad>,
    /// Lampo defaults merged with the project overrides (project wins).
    pub values: PhpIniOverrides,
    /// PHP `error_log` file (`LogSource::SitePhp`).
    pub error_log: PathBuf,
}

/// Extensions that must be loaded with `zend_extension`.
pub fn is_zend_extension(name: &ExtensionName) -> bool {
    matches!(name.as_str(), "xdebug" | "opcache")
}

/// Merges `overrides` on top of `defaults` (overrides win).
pub fn merge_ini(defaults: &PhpIniOverrides, overrides: &PhpIniOverrides) -> PhpIniOverrides {
    let mut out = defaults.clone();
    out.extend(overrides.iter().map(|(k, v)| (k.clone(), v.clone())));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::php::php_ini_overrides;

    #[test]
    fn project_ini_overrides_win() {
        let defaults = php_ini_overrides([("memory_limit", "128M"), ("display_errors", "On")]).unwrap();
        let project = php_ini_overrides([("memory_limit", "1G")]).unwrap();
        let merged = merge_ini(&defaults, &project);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged.values().map(|v| v.as_str()).collect::<Vec<_>>(), ["On", "1G"]);
    }

    #[test]
    fn addressing_splits_names_and_port() {
        let addrs = vec![SiteAddress::LoopbackPort(8101), SiteAddress::localhost("fix.shop").unwrap()];
        let (names, port) = vhost_addressing(&addrs);
        assert_eq!(names.iter().map(|h| h.as_str()).collect::<Vec<_>>(), ["fix.shop.localhost"]);
        assert_eq!(port, Some(8101));
    }

    #[test]
    fn zend_extensions() {
        assert!(is_zend_extension(&ExtensionName::new("xdebug").unwrap()));
        assert!(!is_zend_extension(&ExtensionName::new("intl").unwrap()));
    }
}
