//! The on-disk layout of Lampo. Pure path arithmetic: the two roots are resolved from the
//! environment by `lampo-infra-store` (`%APPDATA%\Lampo`, `%LOCALAPPDATA%\Lampo`) in the
//! composition root and passed around as a value.
//!
//! ```text
//! %APPDATA%\Lampo\                 (config_root: small, user-owned, roams)
//!   settings.toml
//!   projects\<project-id>.toml
//!   projects\<project-id>.worktrees\<slug>.toml   worktree environments of that project
//!   presets\<preset-id>.toml
//! %LOCALAPPDATA%\Lampo\            (data_root: heavy, machine-local)
//!   packages\<slot>\               installed packages; the folder of each one is decided by the
//!                                  `InstalledPackageRegistry` (one per kind + series, e.g.
//!                                  `php-8.3`) and read from `InstalledPackage::install_dir`
//!   packages\installed.toml        registry of installed packages
//!   downloads\                     partial/complete downloads (cleaned after install)
//!   <kind>\[<series>\]data\        one data dir per service instance (mysql\8.4\data\)
//!   redis\data\
//!   generated\apache\httpd.conf
//!   generated\apache\vhosts\<site-id>.conf        site-id = `shop` or `shop@feature-x`
//!   generated\mysql\<series>\my.ini
//!   generated\redis\redis.conf
//!   generated\php\<site-id>\php.ini
//!   logs\<service-id>\{output,error}.log          (see logs::LogSource)
//!   logs\projects\<site-id>\{access,error,php_errors}.log
//!   logs\php-cli\php_errors.log                 PHP outside sites (shims, Lampo's own tools)
//!   bin\                           shims: php.exe composer.exe mysql.exe mysqldump.exe redis-cli.exe
//!                                  (copies of lampo-shim.exe) and lampo.exe (copy of the CLI) —
//!                                  added to the user PATH
//!   bin\shim-index.json            read by the shims on every invocation
//!   secrets\<service-id>.root.dpapi  password of a MySQL instance's root, when it has one,
//!                                  encrypted for the Windows user (DPAPI)
//!   app-location.txt               folder of the last started lampo-desktop.exe, read by copies
//!                                  of the CLI that do not sit next to the app (bin\lampo.exe)
//!   tools\laravel-installer\<php>\ Lampo's own `laravel/installer` (Composer project)
//!   tools\composer\<php>\composer.bat   `composer` for scaffolding commands
//!   generated\php\_tools-<php>\php.ini  php.ini of Lampo's own tools
//!   agent-plugins\claude-code\     generated Claude Code plugin (a local marketplace, see
//!                                  `claude_code`), registered in user scope only on request
//! ```

use std::path::{Path, PathBuf};

use crate::php::PhpMinor;
use crate::preset::PresetId;
use crate::project::ProjectId;
use crate::service::{ServiceId, ServiceKind};
use crate::site::{SiteId, WorktreeSlug};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LampoPaths {
    pub config_root: PathBuf,
    pub data_root: PathBuf,
}

impl LampoPaths {
    pub fn new(config_root: impl Into<PathBuf>, data_root: impl Into<PathBuf>) -> Self {
        Self { config_root: config_root.into(), data_root: data_root.into() }
    }

    // --- config (roaming) ---

    pub fn settings_file(&self) -> PathBuf {
        self.config_root.join("settings.toml")
    }

    pub fn projects_dir(&self) -> PathBuf {
        self.config_root.join("projects")
    }

    pub fn project_file(&self, id: &ProjectId) -> PathBuf {
        self.projects_dir().join(format!("{id}.toml"))
    }

    /// Worktree environments of a project: `projects\<project-id>.worktrees\<slug>.toml`.
    pub fn worktrees_dir(&self, parent: &ProjectId) -> PathBuf {
        self.projects_dir().join(format!("{parent}.worktrees"))
    }

    pub fn worktree_file(&self, parent: &ProjectId, slug: &WorktreeSlug) -> PathBuf {
        self.worktrees_dir(parent).join(format!("{slug}.toml"))
    }

    pub fn presets_dir(&self) -> PathBuf {
        self.config_root.join("presets")
    }

    pub fn preset_file(&self, id: &PresetId) -> PathBuf {
        self.presets_dir().join(format!("{id}.toml"))
    }

    // --- data (local) ---

    pub fn packages_dir(&self) -> PathBuf {
        self.data_root.join("packages")
    }

    pub fn installed_packages_file(&self) -> PathBuf {
        self.packages_dir().join("installed.toml")
    }

    pub fn downloads_dir(&self) -> PathBuf {
        self.data_root.join("downloads")
    }

    /// Persistent data of a service instance: `<data>\<kind>[\<series>]\data`
    /// (`mysql\8.4\data`, `redis\data`). Only kinds whose descriptor `has_data_dir`.
    pub fn service_data_dir(&self, service: &ServiceId) -> PathBuf {
        let mut p = self.data_root.clone();
        p.extend(service.path_segments());
        p.join("data")
    }

    /// Every instance of a service kind: `<data>\<kind>` (`mysql`, holding `8.4\data`, ...).
    pub fn service_kind_dir(&self, kind: &ServiceKind) -> PathBuf {
        self.data_root.join(kind.as_str())
    }

    pub fn generated_dir(&self) -> PathBuf {
        self.data_root.join("generated")
    }

    pub fn apache_config_dir(&self) -> PathBuf {
        self.service_config_dir(&ServiceId::apache())
    }

    /// Generated configuration of a service instance: `generated\<kind>[\<series>]`.
    pub fn service_config_dir(&self, service: &ServiceId) -> PathBuf {
        let mut p = self.generated_dir();
        p.extend(service.path_segments());
        p
    }

    pub fn php_ini_dir(&self, site: &SiteId) -> PathBuf {
        self.generated_dir().join("php").join(site.file_key())
    }

    pub fn vhost_file(&self, site: &SiteId) -> PathBuf {
        self.apache_config_dir().join("vhosts").join(format!("{}.conf", site.file_key()))
    }

    /// php.ini used by the CLI shims outside projects.
    pub fn default_php_ini_dir(&self) -> PathBuf {
        self.generated_dir().join("php").join("_default")
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.data_root.join("logs")
    }

    pub fn service_log_dir(&self, service: &ServiceId) -> PathBuf {
        self.logs_dir().join(service.to_string())
    }

    pub fn site_log_dir(&self, site: &SiteId) -> PathBuf {
        self.logs_dir().join("projects").join(site.file_key())
    }

    /// PHP `error_log` of the php.ini files used outside sites (shims, Lampo's own tools).
    pub fn cli_php_error_log(&self) -> PathBuf {
        self.logs_dir().join("php-cli").join("php_errors.log")
    }

    pub fn shim_dir(&self) -> PathBuf {
        Self::shim_dir_in(&self.data_root)
    }

    pub fn shim_index_file(&self) -> PathBuf {
        Self::shim_index_file_in(&self.data_root)
    }

    /// [`Self::shim_dir`] from the data root alone.
    pub fn shim_dir_in(data_root: &Path) -> PathBuf {
        data_root.join("bin")
    }

    /// [`Self::shim_index_file`] from the data root alone: the shims never compute the config root.
    pub fn shim_index_file_in(data_root: &Path) -> PathBuf {
        Self::shim_dir_in(data_root).join("shim-index.json")
    }

    /// Secrets Lampo keeps for its own clients, encrypted for the current Windows user (DPAPI):
    /// `secrets\` (the MySQL root passwords, `secrets\mysql-8.4.root.dpapi`).
    pub fn secrets_dir(&self) -> PathBuf {
        Self::secrets_dir_in(&self.data_root)
    }

    /// [`Self::secrets_dir`] from the data root alone (the shims).
    pub fn secrets_dir_in(data_root: &Path) -> PathBuf {
        data_root.join("secrets")
    }

    /// Folder of the running app (`lampo-desktop.exe` and its helper executables), recorded
    /// at every app start for the CLI copies that live elsewhere: `app-location.txt`.
    pub fn app_location_file(&self) -> PathBuf {
        self.data_root.join("app-location.txt")
    }

    /// Tools Lampo manages for itself (never the user's global Composer): `tools\`.
    pub fn tools_dir(&self) -> PathBuf {
        self.data_root.join("tools")
    }

    /// Lampo's own Composer project holding `laravel/installer`, one per PHP minor (its
    /// dependencies are resolved for the PHP that runs it): `tools\laravel-installer\<php>`.
    pub fn laravel_installer_dir(&self, php: PhpMinor) -> PathBuf {
        self.tools_dir().join("laravel-installer").join(php.to_string())
    }

    /// `composer.bat` for a PHP minor, put on PATH for scaffolding commands:
    /// `tools\composer\<php>`.
    pub fn tool_composer_dir(&self, php: PhpMinor) -> PathBuf {
        self.tools_dir().join("composer").join(php.to_string())
    }

    /// The generated Claude Code plugin (a local marketplace): `agent-plugins\claude-code`.
    pub fn claude_plugin_dir(&self) -> PathBuf {
        self.data_root.join("agent-plugins").join("claude-code")
    }

    /// The CLI copy in the shim dir (`bin\lampo.exe`, refreshed after app updates): the stable
    /// path agent integrations call.
    pub fn shim_cli_exe(&self) -> PathBuf {
        self.shim_dir().join("lampo.exe")
    }

    /// php.ini used by Lampo's own tools (scaffolding) for a PHP minor:
    /// `generated\php\_tools-<php>`.
    pub fn tool_php_ini_dir(&self, php: PhpMinor) -> PathBuf {
        self.generated_dir().join("php").join(format!("_tools-{php}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_is_rooted_correctly() {
        let p = LampoPaths::new("C:\\cfg", "C:\\data");
        let id = ProjectId::new("shop").unwrap();
        assert_eq!(p.project_file(&id), PathBuf::from("C:\\cfg").join("projects").join("shop.toml"));
        let mysql: ServiceId = "mysql-8.4".parse().unwrap();
        assert_eq!(p.service_data_dir(&mysql), PathBuf::from(r"C:\data").join("mysql").join("8.4").join("data"));
        assert!(p.service_data_dir(&mysql).starts_with(p.service_kind_dir(&ServiceKind::MYSQL)), "every instance lives under its kind's folder");
        assert_eq!(p.service_kind_dir(&ServiceKind::MYSQL), PathBuf::from(r"C:\data").join("mysql"));
        assert_eq!(p.service_config_dir(&mysql), PathBuf::from(r"C:\data").join("generated").join("mysql").join("8.4"));
        assert_eq!(p.service_data_dir(&"redis".parse().unwrap()), PathBuf::from(r"C:\data").join("redis").join("data"));
        assert_eq!(p.service_log_dir(&"mysql-5.7".parse().unwrap()), PathBuf::from(r"C:\data").join("logs").join("mysql-5.7"));
        assert_eq!(p.apache_config_dir(), PathBuf::from(r"C:\data").join("generated").join("apache"));
        assert_eq!(p.app_location_file(), PathBuf::from(r"C:\data").join("app-location.txt"));
        assert_eq!(p.claude_plugin_dir(), PathBuf::from(r"C:\data").join("agent-plugins").join("claude-code"));
        assert_eq!(p.shim_cli_exe(), PathBuf::from(r"C:\data").join("bin").join("lampo.exe"));
    }
}
