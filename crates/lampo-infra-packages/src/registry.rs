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
//!   mailpit-1.33\             mailpit.exe
//!   composer-2.11\            composer.phar   (latest stable)
//!   composer-2.2\             composer.phar   (2.2 LTS, PHP < 7.2)
//!   xdebug-3.5-php8.3\        php_xdebug.dll
//!   php-redis-6.3-php8.3\     php_redis.dll
//! ```
//!
//! A directory is a **slot**: kind + `<major>.<minor>` series (+ PHP minor for add-on
//! extensions). A patch update (8.3.14 → 8.3.15) is extracted into the *same* slot, replacing the
//! old tree atomically (see [`crate::slot`]; a single-file payload replaces its slot the same
//! way), and its record replaces the old record. Paths
//! that other components derive from a slot (`php-8.3\php-cgi.exe` in vhosts, `PHPRC` pointing at
//! the per-project `generated\php\<site>\php.ini`) therefore stay valid across patch updates;
//! nothing per-project lives inside a package directory.
//!
//! `installed.toml` is written atomically and durably. A record that does not validate is
//! skipped (logged); a file that no longer parses is kept as `installed.toml.corrupt-<unix ms>`
//! and rebuilt from the slot directories, with versions recorded as unknown (`8.3-unknown`).
//!
//! Consumers must use [`InstalledPackage::install_dir`] (or [`InstalledPackageRegistry::install_dir`])
//! rather than computing paths, as the slot name intentionally differs from `PackageId::key()`.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use async_trait::async_trait;
use lampo_domain::package::{InstalledPackage, PackageId, PackageKind};
use lampo_domain::paths::LampoPaths;
use lampo_domain::php::PhpMinor;
use lampo_domain::ports::InstalledPackageRegistry;
use lampo_domain::vc_runtime::VcRuntime;
use lampo_domain::version::PackageVersion;
use lampo_domain::{PortError, PortResult};
use lampo_infra_fs::{corrupt_backup_path, write_atomic};
use serde::{Deserialize, Serialize};

use crate::leftovers::{self, Leftover};
use crate::recovery;

/// Separator before the PHP minor in the slot name of an add-on (`xdebug-3.5-php8.3`).
pub(crate) const PHP_MINOR_SEPARATOR: &str = "-php";

/// First line of `installed.toml`.
const FILE_HEADER: &str = "# Managed by Lampo. Packages installed under this directory.\n";

/// The directory name of the slot `id` belongs to (`php-8.3`, `xdebug-3.5-php8.3`).
pub fn slot_key(id: &PackageId) -> String {
    let series = id.series();
    match id.php {
        Some(php) => format!("{}-{series}{PHP_MINOR_SEPARATOR}{php}", id.kind),
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

    /// The records of `installed.toml` (none if it does not exist). A file that does not parse
    /// is replaced by one rebuilt from the slot directories (see [`Self::recover`]).
    fn load(&self) -> PortResult<Vec<Record>> {
        let file = self.file();
        let bytes = match std::fs::read(&file) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(PortError::io(format!("reading {}", file.display()), e)),
        };
        match parse(&bytes) {
            Ok(records) => Ok(records),
            Err(reason) => self.recover(&file, &reason),
        }
    }

    /// Keeps a copy of the unparsable `file` as `installed.toml.corrupt-<unix ms>` and rebuilds
    /// the records from the slot directories (versions unknown, see [`crate::recovery`]). The
    /// rebuilt file is saved only once the copy exists; until then the records are only returned.
    fn recover(&self, file: &Path, reason: &str) -> PortResult<Vec<Record>> {
        let packages_dir = self.paths.packages_dir();
        let records = recovery::records_from_slots(&packages_dir).map_err(|e| PortError::io(format!("listing {}", packages_dir.display()), e))?;
        let backup = corrupt_backup_path(file);
        match std::fs::copy(file, &backup) {
            Ok(_) => {
                tracing::warn!(file = %file.display(), backup = %backup.display(), reason, packages = records.len(), "installed.toml did not load: kept a copy and rebuilt it from the package folders");
                if let Err(e) = self.save(records.clone()) {
                    tracing::warn!(error = %e, "cannot save the rebuilt installed.toml; it is rebuilt again next time");
                }
            }
            Err(e) => tracing::warn!(file = %file.display(), reason, error = %e, "installed.toml did not load and cannot be backed up: left untouched, packages listed from their folders"),
        }
        Ok(records)
    }

    fn save(&self, mut records: Vec<Record>) -> PortResult<()> {
        records.sort_by(|a, b| (&a.kind, &a.version, &a.php).cmp(&(&b.kind, &b.version, &b.php)));
        let text = toml::to_string(&RegistryFile { package: records }).map_err(|e| PortError::io("serialising installed.toml", e))?;
        write_atomic(&self.file(), format!("{FILE_HEADER}{text}").as_bytes())
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

#[async_trait]
impl InstalledPackageRegistry for FsInstalledPackageRegistry {
    /// Recorded packages whose directory still exists. Invalid records are skipped (logged).
    fn list(&self) -> PortResult<Vec<InstalledPackage>> {
        let _guard = self.lock.lock().unwrap_or_else(|p| p.into_inner());
        let mut out = Vec::new();
        for record in self.load()? {
            let package = match self.to_installed(&record) {
                Ok(package) => package,
                Err(e) => {
                    tracing::warn!(dir = %record.dir, error = %e, "skipping an invalid record of installed.toml");
                    continue;
                }
            };
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
    /// `NotFound` is returned and nothing is touched. Once the record is gone the files are
    /// deleted on a blocking thread.
    async fn remove(&self, id: &PackageId) -> PortResult<()> {
        let (dir, trash) = self.unrecord(id)?;
        let deleted = tokio::task::spawn_blocking(move || {
            if let Err(e) = remove_tree(&trash) {
                tracing::warn!(dir = %trash.display(), error = %e, "could not delete removed package files; they will be cleaned up later");
            }
            leftovers::discard(&dir);
        })
        .await;
        if let Err(e) = deleted {
            tracing::warn!(error = %e, "deleting removed package files did not finish; they will be cleaned up later");
        }
        Ok(())
    }
}

impl FsInstalledPackageRegistry {
    /// Moves the slot of exactly `id` aside and drops its record. Returns the slot and where its
    /// files now are (deleted by the caller). If the record cannot be dropped, the files are put
    /// back: the package stays installed (a trash copy would be deleted at the next start).
    fn unrecord(&self, id: &PackageId) -> PortResult<(PathBuf, PathBuf)> {
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
        let trash = leftovers::sibling(&dir, Leftover::Removed);
        let moved = dir.exists();
        if moved {
            std::fs::rename(&dir, &trash).map_err(|e| PortError::io(format!("removing {} (is a service using it? stop it and retry)", dir.display()), e))?;
        }
        if let Err(e) = self.save(keep) {
            if moved {
                put_back(&trash, &dir);
            }
            return Err(e);
        }
        Ok((dir, trash))
    }
}

/// Renames the files of a removal that did not happen from `trash` back into the slot `dir`.
fn put_back(trash: &Path, dir: &Path) {
    if let Err(e) = std::fs::rename(trash, dir) {
        tracing::error!(dir = %dir.display(), from = %trash.display(), error = %e, "cannot put the files of a package that stays installed back; move the folder back by hand before restarting Lampo");
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

/// The records of the bytes of `installed.toml`, or why they do not parse.
fn parse(bytes: &[u8]) -> Result<Vec<Record>, String> {
    let text = std::str::from_utf8(bytes).map_err(|e| format!("not valid UTF-8: {e}"))?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    toml::from_str::<RegistryFile>(text).map(|file| file.package).map_err(|e| e.to_string())
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct RegistryFile {
    #[serde(default)]
    package: Vec<Record>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Record {
    pub(crate) kind: String,
    pub(crate) version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) php: Option<String>,
    /// Directory name under `packages\` (relative, so `LAMPO_HOME` moves keep working).
    pub(crate) dir: String,
    /// Visual C++ toolset of the build (`vs17`), when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) vc_runtime: Option<VcRuntime>,
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

    #[tokio::test]
    async fn patch_update_replaces_the_record_of_the_same_slot() {
        let (_d, reg) = registry();
        let old = install(&reg, &id(PackageKind::PHP, "8.3.14", None));
        install(&reg, &id(PackageKind::PHP, "5.6.40", None));
        let new = install(&reg, &id(PackageKind::PHP, "8.3.15", None));
        assert_eq!(old.install_dir, new.install_dir, "same directory across patch versions");
        let list = reg.list().unwrap();
        let versions: Vec<_> = list.iter().map(|p| p.id.version.to_string()).collect();
        assert_eq!(versions, ["5.6.40", "8.3.15"]);
        // Removing the superseded id is refused and leaves the new files alone.
        assert!(matches!(reg.remove(&old.id).await, Err(PortError::NotFound { .. })));
        assert!(new.install_dir.join("version.txt").exists());
    }

    #[test]
    fn extensions_are_per_php_minor() {
        let (_d, reg) = registry();
        install(&reg, &id(PackageKind::XDEBUG, "3.5.3", Some("8.3")));
        install(&reg, &id(PackageKind::XDEBUG, "3.5.3", Some("8.4")));
        assert_eq!(reg.list().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn remove_deletes_files_and_record() {
        let (_d, reg) = registry();
        let p = install(&reg, &id(PackageKind::MYSQL, "8.4.13", None));
        reg.remove(&p.id).await.unwrap();
        assert!(!p.install_dir.exists());
        assert!(reg.list().unwrap().is_empty());
        assert_eq!(std::fs::read_dir(p.install_dir.parent().unwrap()).unwrap().count(), 1, "only installed.toml remains");
    }

    /// An uninstall empties the slot on purpose: a previous copy a crashed update left must not
    /// come back in its place.
    #[cfg(windows)]
    #[tokio::test]
    async fn remove_never_resurrects_a_previous_copy() {
        let (_d, reg) = registry();
        let p = install(&reg, &id(PackageKind::MYSQL, "8.4.13", None));
        let stale = p.install_dir.with_file_name(format!("mysql-8.4.old-{}-1", crate::pid::exited_process_id()));
        std::fs::create_dir_all(&stale).unwrap();
        reg.remove(&p.id).await.unwrap();
        assert!(!p.install_dir.exists() && !stale.exists());
    }

    /// `installed.toml` cannot be replaced (another program holds it): the record stays, so the
    /// files must stay too, or the next start would delete them as an uninstall's trash.
    #[cfg(windows)]
    #[tokio::test]
    async fn a_record_that_cannot_be_dropped_keeps_the_files() {
        use std::os::windows::fs::OpenOptionsExt;
        /// Others may read the file, never replace or delete it.
        const FILE_SHARE_READ: u32 = 0x1;

        let (_d, reg) = registry();
        let p = install(&reg, &id(PackageKind::MYSQL, "8.4.13", None));
        let held = std::fs::OpenOptions::new().read(true).share_mode(FILE_SHARE_READ).open(reg.file()).unwrap();

        assert!(matches!(reg.remove(&p.id).await, Err(PortError::Io { .. })));

        drop(held);
        assert_eq!(std::fs::read_to_string(p.install_dir.join("version.txt")).unwrap(), "8.4.13", "the files are back in the slot");
        assert_eq!(reg.list().unwrap().iter().map(|p| p.id.clone()).collect::<Vec<_>>(), std::slice::from_ref(&p.id));
        assert!(crate::leftovers::slots_with_leftovers(&reg.paths.packages_dir()).unwrap().is_empty(), "no trash copy for the next start to delete");
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

    fn corrupt_backups(reg: &FsInstalledPackageRegistry) -> Vec<String> {
        std::fs::read_dir(reg.paths.packages_dir())
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.file_name().unwrap().to_string_lossy().starts_with("installed.toml.corrupt-"))
            .map(|p| std::fs::read_to_string(p).unwrap())
            .collect()
    }

    #[test]
    fn an_invalid_record_is_skipped_and_the_others_still_listed() {
        let (_d, reg) = registry();
        install(&reg, &id(PackageKind::PHP, "8.3.15", None));
        let text = std::fs::read_to_string(reg.file()).unwrap();
        std::fs::write(reg.file(), format!("{text}\n[[package]]\nkind = \"php\"\nversion = \"not a version\"\ndir = \"php-9.9\"\n")).unwrap();
        let versions: Vec<String> = reg.list().unwrap().iter().map(|p| p.id.version.to_string()).collect();
        assert_eq!(versions, ["8.3.15"]);
        assert!(corrupt_backups(&reg).is_empty(), "a readable file is not rebuilt");
    }

    #[test]
    fn an_unreadable_registry_is_backed_up_and_rebuilt_from_the_slots() {
        let (_d, reg) = registry();
        install(&reg, &id(PackageKind::PHP, "8.3.15", None));
        install(&reg, &id(PackageKind::XDEBUG, "3.5.3", Some("8.3")));
        std::fs::create_dir_all(reg.paths.packages_dir().join("php-8.3.old-12-34")).unwrap();
        std::fs::write(reg.file(), "garbage [[[").unwrap();

        let mut list: Vec<(String, String, Option<String>)> =
            reg.list().unwrap().into_iter().map(|p| (p.id.kind.to_string(), p.id.version.to_string(), p.id.php.map(|m| m.to_string()))).collect();
        list.sort();
        assert_eq!(list, [("php".into(), "8.3-unknown".into(), None), ("xdebug".into(), "3.5-unknown".into(), Some("8.3".into()))]);
        assert_eq!(corrupt_backups(&reg), ["garbage [[["], "the unreadable file is kept");
        assert!(std::fs::read_to_string(reg.file()).unwrap().contains("dir = \"php-8.3\""), "the rebuilt registry is saved");

        // A later install of the series records the real version over the unknown one.
        install(&reg, &id(PackageKind::PHP, "8.3.16", None));
        let php: Vec<String> = reg.list().unwrap().iter().filter(|p| p.id.kind == PackageKind::PHP).map(|p| p.id.version.to_string()).collect();
        assert_eq!(php, ["8.3.16"]);
        assert_eq!(corrupt_backups(&reg).len(), 1);
    }

    #[test]
    fn a_registry_that_is_not_utf8_is_rebuilt_too() {
        let (_d, reg) = registry();
        install(&reg, &id(PackageKind::REDIS, "8.10.3", None));
        std::fs::write(reg.file(), [0xff, 0xfe, 0x00]).unwrap();
        assert_eq!(reg.list().unwrap().len(), 1);
    }

    #[test]
    fn saving_leaves_no_temp_file() {
        let (_d, reg) = registry();
        install(&reg, &id(PackageKind::MAILPIT, "1.33.0", None));
        let names: Vec<String> = std::fs::read_dir(reg.paths.packages_dir()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(names.len(), 2, "installed.toml and the slot: {names:?}");
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
