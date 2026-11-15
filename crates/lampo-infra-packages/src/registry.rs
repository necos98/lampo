//! The installed-package registry and the on-disk layout of packages.
//!
//! ```text
//! %LOCALAPPDATA%\Lampo\packages\
//!   installed.toml            one [[package]] per installed slot (id + directory name)
//!   php-8.3\                  PHP 8.3: whichever 8.3.x is installed (php.exe, php-cgi.exe, ext\)
//!   php-5.6\
//!   apache-2.4\               bin\httpd.exe, modules\ …   (Apache24\ stripped)
//!   mod-fcgid-2.3\            mod_fcgid.so
//!   mysql-8.4\                bin\mysqld.exe …            (mysql-8.4.x-winx64\ stripped)
//!   mysql-5.7\
//!   redis-8.10\               redis-server.exe, redis-cli.exe, msys-2.0.dll
//!   composer-2.11\            composer.phar   (latest stable)
//!   composer-2.2\             composer.phar   (2.2 LTS, PHP < 7.2)
//!   xdebug-3.5-php8.3\        php_xdebug.dll
//!   php-redis-6.3-php8.3\     php_redis.dll
//! ```
//!
//! A directory is a **slot**: kind + `<major>.<minor>` series (+ PHP minor for add-on
//! extensions). A patch update (8.3.14 → 8.3.15) is extracted into the *same* slot, replacing the
//! old tree atomically (see [`crate::extract`]), and its record replaces the old record. Paths
//! that other components derive from a slot (`php-8.3\php-cgi.exe` in vhosts, `PHPRC` pointing at
//! the per-project `generated\php\<site>\php.ini`) therefore stay valid across patch updates;
//! nothing per-project lives inside a package directory.
//!
//! Consumers must use [`InstalledPackage::install_dir`] (or [`InstalledPackageRegistry::install_dir`])
//! rather than computing paths, as the slot name intentionally differs from `PackageId::key()`.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use lampo_domain::package::{InstalledPackage, PackageId, PackageKind};
use lampo_domain::paths::LampoPaths;
use lampo_domain::php::PhpMinor;
use lampo_domain::ports::InstalledPackageRegistry;
use lampo_domain::vc_runtime::VcRuntime;
use lampo_domain::version::PackageVersion;
use lampo_domain::{PortError, PortResult};
use serde::{Deserialize, Serialize};

use crate::extract::{clean_leftovers, unique_sibling};

/// The directory name of the slot `id` belongs to (`php-8.3`, `xdebug-3.5-php8.3`).
pub fn slot_key(id: &PackageId) -> String {
    let series = id.series();
    match id.php {
        Some(php) => format!("{}-{series}-php{php}", id.kind),
        None => format!("{}-{series}", id.kind),
    }
}

fn same_slot(a: &PackageId, b: &PackageId) -> bool {
    a.kind == b.kind && a.series() == b.series() && a.php == b.php
}

/// `%LOCALAPPDATA%\Lampo\packages\installed.toml` + one directory per slot (see module docs).
pub struct FsInstalledPackageRegistry {
    paths: LampoPaths,
    /// Serialises read-modify-write of `installed.toml` within the process.
    lock: Mutex<()>,
}

impl FsInstalledPackageRegistry {
    pub fn new(paths: LampoPaths) -> Self {
        Self { paths, lock: Mutex::new(()) }
    }

    fn file(&self) -> PathBuf {
        self.paths.installed_packages_file()
    }

    fn load(&self) -> PortResult<Vec<Record>> {
        let file = self.file();
        let text = match std::fs::read_to_string(&file) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(PortError::io(format!("reading {}", file.display()), e)),
        };
        let parsed: RegistryFile = toml::from_str(&text).map_err(|e| PortError::invalid_data(file.display().to_string(), e))?;
        Ok(parsed.package)
    }

    fn save(&self, mut records: Vec<Record>) -> PortResult<()> {
        records.sort_by(|a, b| (&a.kind, &a.version, &a.php).cmp(&(&b.kind, &b.version, &b.php)));
        let file = self.file();
        let dir = self.paths.packages_dir();
        std::fs::create_dir_all(&dir).map_err(|e| PortError::io(format!("creating {}", dir.display()), e))?;
        let text = toml::to_string(&RegistryFile { package: records }).map_err(|e| PortError::io("serialising installed.toml", e))?;
        let tmp = file.with_extension("toml.tmp");
        std::fs::write(&tmp, format!("# Managed by Lampo. Packages installed under this directory.\n{text}")).map_err(|e| PortError::io(format!("writing {}", tmp.display()), e))?;
        std::fs::rename(&tmp, &file).map_err(|e| PortError::io(format!("replacing {}", file.display()), e))
    }

    fn to_installed(&self, record: &Record) -> PortResult<InstalledPackage> {
        let source = self.file().display().to_string();
        let kind = PackageKind::new(record.kind.clone()).map_err(|e| PortError::invalid_data(source.clone(), e))?;
        let version = PackageVersion::parse(&record.version).map_err(|e| PortError::invalid_data(source.clone(), e))?;
        let php = record.php.as_deref().map(PhpMinor::parse).transpose().map_err(|e| PortError::invalid_data(source.clone(), e))?;
        if !is_plain_dir_name(&record.dir) {
            return Err(PortError::invalid_data(source, format!("invalid package directory `{}`", record.dir)));
        }
        Ok(crate::layout::resolve(InstalledPackage::new(PackageId { kind, version, php }, self.paths.packages_dir().join(&record.dir), record.vc_runtime)))
    }
}

impl InstalledPackageRegistry for FsInstalledPackageRegistry {
    /// Recorded packages whose directory still exists.
    fn list(&self) -> PortResult<Vec<InstalledPackage>> {
        let _guard = self.lock.lock().unwrap_or_else(|p| p.into_inner());
        let mut out = Vec::new();
        for record in self.load()? {
            let package = self.to_installed(&record)?;
            if package.install_dir.is_dir() {
                out.push(package);
            } else {
                tracing::warn!(package = %package.id, dir = %package.install_dir.display(), "recorded package directory is missing; ignoring it");
            }
        }
        Ok(out)
    }

    fn install_dir(&self, id: &PackageId) -> PathBuf {
        self.paths.packages_dir().join(slot_key(id))
    }

    /// Records `package`, replacing any record of the same slot (patch update) or id.
    fn record(&self, package: &InstalledPackage) -> PortResult<()> {
        let _guard = self.lock.lock().unwrap_or_else(|p| p.into_inner());
        let packages_dir = self.paths.packages_dir();
        let dir = package
            .install_dir
            .strip_prefix(&packages_dir)
            .ok()
            .and_then(|p| p.to_str())
            .filter(|p| is_plain_dir_name(p))
            .ok_or_else(|| PortError::io("recording a package", format!("{} is not a directory directly under {}", package.install_dir.display(), packages_dir.display())))?
            .to_string();
        let mut records: Vec<Record> =
            self.load()?.into_iter().filter(|r| !self.to_installed(r).is_ok_and(|p| same_slot(&p.id, &package.id) || p.id == package.id) && r.dir != dir).collect();
        records.push(Record { kind: package.id.kind.to_string(), version: package.id.version.to_string(), php: package.id.php.map(|p| p.to_string()), dir, vc_runtime: package.vc_runtime });
        self.save(records)
    }

    /// Deletes the files and the record of exactly `id`. If the slot now holds another version,
    /// `NotFound` is returned and nothing is touched.
    fn remove(&self, id: &PackageId) -> PortResult<()> {
        let _guard = self.lock.lock().unwrap_or_else(|p| p.into_inner());
        let records = self.load()?;
        let mut target = None;
        let mut keep = Vec::with_capacity(records.len());
        for record in records {
            if target.is_none() && self.to_installed(&record).is_ok_and(|p| &p.id == id) {
                target = Some(record);
            } else {
                keep.push(record);
            }
        }
        let record = target.ok_or_else(|| PortError::not_found("package", id.to_string()))?;
        let dir = self.paths.packages_dir().join(&record.dir);
        // Move aside first: if a running service holds files, this fails and nothing changes.
        let trash = unique_sibling(&dir, "trash");
        if dir.exists() {
            std::fs::rename(&dir, &trash).map_err(|e| PortError::io(format!("removing {} (is a service using it? stop it and retry)", dir.display()), e))?;
        }
        self.save(keep)?;
        if let Err(e) = remove_tree(&trash) {
            tracing::warn!(dir = %trash.display(), error = %e, "could not delete removed package files; they will be cleaned up later");
        }
        clean_leftovers(&dir);
        Ok(())
    }
}

fn remove_tree(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_dir_all(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

fn is_plain_dir_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\', ':'])
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct RegistryFile {
    #[serde(default)]
    package: Vec<Record>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Record {
    kind: String,
    version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    php: Option<String>,
    /// Directory name under `packages\` (relative, so `LAMPO_HOME` moves keep working).
    dir: String,
    /// Visual C++ toolset of the build (`vs17`), when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    vc_runtime: Option<VcRuntime>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(kind: PackageKind, version: &str, php: Option<&str>) -> PackageId {
        PackageId { kind, version: PackageVersion::parse(version).unwrap(), php: php.map(|p| PhpMinor::parse(p).unwrap()) }
    }

    fn registry() -> (tempfile::TempDir, FsInstalledPackageRegistry) {
        let dir = tempfile::tempdir().unwrap();
        let reg = FsInstalledPackageRegistry::new(LampoPaths::new(dir.path().join("cfg"), dir.path().join("data")));
        (dir, reg)
    }

    fn install(reg: &FsInstalledPackageRegistry, id: &PackageId) -> InstalledPackage {
        let dir = reg.install_dir(id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("version.txt"), id.version.as_str()).unwrap();
        let p = InstalledPackage::new(id.clone(), dir, None);
        reg.record(&p).unwrap();
        p
    }

    #[test]
    fn the_vc_runtime_is_persisted() {
        let (_d, reg) = registry();
        let id = id(PackageKind::APACHE, "2.4.71-261112", None);
        let dir = reg.install_dir(&id);
        std::fs::create_dir_all(&dir).unwrap();
        reg.record(&InstalledPackage::new(id, dir, Some(VcRuntime::Vs18))).unwrap();
        assert_eq!(reg.list().unwrap()[0].vc_runtime, Some(VcRuntime::Vs18));
        assert!(std::fs::read_to_string(reg.file()).unwrap().contains("vc_runtime = \"vs18\""));
    }

    #[test]
    fn slot_keys() {
        assert_eq!(slot_key(&id(PackageKind::PHP, "8.3.36-1", None)), "php-8.3");
        assert_eq!(slot_key(&id(PackageKind::APACHE, "2.4.71-261112", None)), "apache-2.4");
        assert_eq!(slot_key(&id(PackageKind::XDEBUG, "3.5.3", Some("8.3"))), "xdebug-3.5-php8.3");
        assert_eq!(slot_key(&id(PackageKind::REDIS, "8.10.3", None)), "redis-8.10");
    }

    #[test]
    fn empty_registry_lists_nothing() {
        let (_d, reg) = registry();
        assert!(reg.list().unwrap().is_empty());
    }

    #[test]
    fn patch_update_replaces_the_record_of_the_same_slot() {
        let (_d, reg) = registry();
        let old = install(&reg, &id(PackageKind::PHP, "8.3.14", None));
        install(&reg, &id(PackageKind::PHP, "5.6.40", None));
        let new = install(&reg, &id(PackageKind::PHP, "8.3.15", None));
        assert_eq!(old.install_dir, new.install_dir, "same directory across patch versions");
        let list = reg.list().unwrap();
        let versions: Vec<_> = list.iter().map(|p| p.id.version.to_string()).collect();
        assert_eq!(versions, ["5.6.40", "8.3.15"]);
        // Removing the superseded id is refused and leaves the new files alone.
        assert!(matches!(reg.remove(&old.id), Err(PortError::NotFound { .. })));
        assert!(new.install_dir.join("version.txt").exists());
    }

    #[test]
    fn extensions_are_per_php_minor() {
        let (_d, reg) = registry();
        install(&reg, &id(PackageKind::XDEBUG, "3.5.3", Some("8.3")));
        install(&reg, &id(PackageKind::XDEBUG, "3.5.3", Some("8.4")));
        assert_eq!(reg.list().unwrap().len(), 2);
    }

    #[test]
    fn remove_deletes_files_and_record() {
        let (_d, reg) = registry();
        let p = install(&reg, &id(PackageKind::MYSQL, "8.4.13", None));
        reg.remove(&p.id).unwrap();
        assert!(!p.install_dir.exists());
        assert!(reg.list().unwrap().is_empty());
        assert_eq!(std::fs::read_dir(p.install_dir.parent().unwrap()).unwrap().count(), 1, "only installed.toml remains");
    }

    #[test]
    fn missing_directories_are_ignored() {
        let (_d, reg) = registry();
        let p = install(&reg, &id(PackageKind::REDIS, "8.10.3", None));
        std::fs::remove_dir_all(&p.install_dir).unwrap();
        assert!(reg.list().unwrap().is_empty());
    }

    #[test]
    fn refuses_directories_outside_the_packages_dir() {
        let (d, reg) = registry();
        let p = InstalledPackage::new(id(PackageKind::PHP, "8.3.1", None), d.path().join("elsewhere"), None);
        assert!(reg.record(&p).is_err());
    }

    #[test]
    fn file_format_is_stable() {
        let (_d, reg) = registry();
        install(&reg, &id(PackageKind::PHP_REDIS, "6.3.0", Some("8.3")));
        let text = std::fs::read_to_string(reg.file()).unwrap();
        assert!(text.contains("kind = \"php-redis\""), "{text}");
        assert!(text.contains("php = \"8.3\""), "{text}");
        assert!(text.contains("dir = \"php-redis-6.3-php8.3\""), "{text}");
    }
}
