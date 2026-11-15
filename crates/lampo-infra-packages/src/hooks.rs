//! Post-install hooks, one per kind that needs one (open-closed: add a value/type + list it in [`all`]).
//!
//! Hooks run after the files are in place and before the package is recorded. They check that the
//! package has the layout the rest of Lampo relies on (see [`crate::registry`]), so a broken or
//! unexpected archive fails the install instead of failing later at service start.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use lampo_domain::package::{InstalledPackage, PackageKind};
use lampo_domain::ports::PackageInstallHook;
use lampo_domain::{PortError, PortResult};

fn missing(package: &InstalledPackage, what: &str) -> PortError {
    PortError::invalid_data(package.id.to_string(), format!("{what} not found in {}", package.install_dir.display()))
}

/// PHP: verifies `php-cgi.exe`/`php.exe` exist and the `ext` dir is present. Nothing else is
/// written into the PHP directory: every php.ini is generated per site outside it, so replacing
/// the directory on a patch update cannot lose configuration.
#[derive(Debug, Default, Clone, Copy)]
pub struct PhpInstallHook;

#[async_trait]
impl PackageInstallHook for PhpInstallHook {
    fn kind(&self) -> PackageKind {
        PackageKind::PHP
    }

    async fn after_install(&self, package: &InstalledPackage) -> PortResult<()> {
        for exe in ["php.exe", "php-cgi.exe"] {
            if !package.install_dir.join(exe).is_file() {
                return Err(missing(package, exe));
            }
        }
        if !package.install_dir.join("ext").is_dir() {
            return Err(missing(package, "ext directory"));
        }
        Ok(())
    }
}

/// mod_fcgid: makes sure `mod_fcgid.so` is at the root of the package directory, which is where
/// the Apache configuration loads it from (`<mod-fcgid dir>\mod_fcgid.so`). Apache Lounge zips
/// have it at the root today; older ones had it in a sub-folder.
#[derive(Debug, Default, Clone, Copy)]
pub struct ModFcgidInstallHook;

pub const MOD_FCGID_FILE: &str = "mod_fcgid.so";

#[async_trait]
impl PackageInstallHook for ModFcgidInstallHook {
    fn kind(&self) -> PackageKind {
        PackageKind::MOD_FCGID
    }

    async fn after_install(&self, package: &InstalledPackage) -> PortResult<()> {
        let target = package.install_dir.join(MOD_FCGID_FILE);
        if target.is_file() {
            return Ok(());
        }
        let root = package.install_dir.clone();
        let found = tokio::task::spawn_blocking(move || find_file(&root, MOD_FCGID_FILE, 4)).await.map_err(|e| PortError::io("searching mod_fcgid.so", e))?;
        let source = found.ok_or_else(|| missing(package, MOD_FCGID_FILE))?;
        tokio::fs::copy(&source, &target).await.map_err(|e| PortError::io(format!("copying {} to {}", source.display(), target.display()), e))?;
        Ok(())
    }
}

/// Generic check for kinds whose only post-install need is "these files exist" (paths relative to
/// the install dir). Adding such a kind is one more value in [`all`].
#[derive(Debug, Clone)]
pub struct RequiredFilesHook {
    pub kind: PackageKind,
    pub files: &'static [&'static str],
}

#[async_trait]
impl PackageInstallHook for RequiredFilesHook {
    fn kind(&self) -> PackageKind {
        self.kind.clone()
    }

    async fn after_install(&self, package: &InstalledPackage) -> PortResult<()> {
        for file in self.files {
            if !package.install_dir.join(file).is_file() {
                return Err(missing(package, file));
            }
        }
        Ok(())
    }
}

fn find_file(dir: &Path, name: &str, depth: u32) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut subdirs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            subdirs.push(path);
        } else if entry.file_name().to_string_lossy().eq_ignore_ascii_case(name) {
            return Some(path);
        }
    }
    if depth == 0 {
        return None;
    }
    subdirs.sort();
    subdirs.into_iter().find_map(|d| find_file(&d, name, depth - 1))
}

pub fn all() -> Vec<Arc<dyn PackageInstallHook>> {
    vec![
        Arc::new(PhpInstallHook),
        Arc::new(ModFcgidInstallHook),
        Arc::new(RequiredFilesHook { kind: PackageKind::APACHE, files: &["bin/httpd.exe"] }),
        Arc::new(RequiredFilesHook { kind: PackageKind::MYSQL, files: &["bin/mysqld.exe", "bin/mysql.exe", "bin/mysqldump.exe"] }),
        Arc::new(RequiredFilesHook { kind: PackageKind::REDIS, files: &["redis-server.exe", "redis-cli.exe"] }),
        Arc::new(RequiredFilesHook { kind: PackageKind::COMPOSER, files: &["composer.phar"] }),
        Arc::new(RequiredFilesHook { kind: PackageKind::XDEBUG, files: &["php_xdebug.dll"] }),
        Arc::new(RequiredFilesHook { kind: PackageKind::PHP_REDIS, files: &["php_redis.dll"] }),
    ]
}

#[cfg(test)]
mod tests {
    use lampo_domain::package::PackageId;
    use lampo_domain::version::PackageVersion;

    use super::*;

    fn package(kind: PackageKind, dir: &Path) -> InstalledPackage {
        InstalledPackage::new(PackageId::new(kind, PackageVersion::parse("1.0.0").unwrap()), dir.to_path_buf(), None)
    }

    #[test]
    fn one_hook_per_kind() {
        let hooks = all();
        let mut kinds: Vec<_> = hooks.iter().map(|h| h.kind()).collect();
        kinds.sort();
        let before = kinds.len();
        kinds.dedup();
        assert_eq!(before, kinds.len());
    }

    #[tokio::test]
    async fn php_hook_checks_the_layout() {
        let dir = tempfile::tempdir().unwrap();
        let p = package(PackageKind::PHP, dir.path());
        assert!(PhpInstallHook.after_install(&p).await.is_err());
        std::fs::write(dir.path().join("php.exe"), b"").unwrap();
        std::fs::write(dir.path().join("php-cgi.exe"), b"").unwrap();
        assert!(PhpInstallHook.after_install(&p).await.is_err(), "ext missing");
        std::fs::create_dir(dir.path().join("ext")).unwrap();
        PhpInstallHook.after_install(&p).await.unwrap();
    }

    #[tokio::test]
    async fn mod_fcgid_hook_moves_the_module_to_the_root() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("mod_fcgid-2.3.10");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("mod_fcgid.so"), b"so").unwrap();
        let p = package(PackageKind::MOD_FCGID, dir.path());
        ModFcgidInstallHook.after_install(&p).await.unwrap();
        assert_eq!(std::fs::read(dir.path().join("mod_fcgid.so")).unwrap(), b"so");
        let empty = tempfile::tempdir().unwrap();
        assert!(ModFcgidInstallHook.after_install(&package(PackageKind::MOD_FCGID, empty.path())).await.is_err());
    }

    #[tokio::test]
    async fn required_files_hook() {
        let dir = tempfile::tempdir().unwrap();
        let hook = RequiredFilesHook { kind: PackageKind::REDIS, files: &["redis-server.exe", "redis-cli.exe"] };
        let p = package(PackageKind::REDIS, dir.path());
        std::fs::write(dir.path().join("redis-server.exe"), b"").unwrap();
        assert!(hook.after_install(&p).await.is_err());
        std::fs::write(dir.path().join("redis-cli.exe"), b"").unwrap();
        hook.after_install(&p).await.unwrap();
    }
}
