//! The stored MySQL passwords: the root password of each local instance
//! (`secrets\<service-id>.root.dpapi`) and the password of each project's dedicated account
//! (`secrets\<project-id>.project.dpapi`), one file each, encrypted with DPAPI for the current
//! Windows user: another user, or a copy of the file on another machine, cannot decrypt it.

use std::path::{Path, PathBuf};

use lampo_domain::mysql::credentials::{MAX_PASSWORD_BYTES, MysqlPassword};
use lampo_domain::ports::{MysqlProjectPasswordReader, MysqlProjectPasswordWriter, MysqlRootPasswordReader, MysqlRootPasswordWriter};
use lampo_domain::project::ProjectId;
use lampo_domain::service::ServiceId;
use lampo_domain::{PortError, PortResult};

use crate::win::dpapi;

/// A DPAPI blob of the longest password is well under this: anything larger is not ours.
const MAX_FILE_BYTES: u64 = 16 * 1024 + MAX_PASSWORD_BYTES as u64;

/// One kind of stored password: its file suffix and the DPAPI entropy binding its files to this
/// purpose (a blob of something else, or of the other kind, does not decrypt).
struct Kind {
    suffix: &'static str,
    entropy: &'static [u8],
    /// `root password of mysql-8.4`, `password of the MySQL account of shop`.
    describe: fn(&str) -> String,
}

const ROOT: Kind = Kind { suffix: ".root.dpapi", entropy: b"lampo/mysql-root-password/v1", describe: |instance| format!("root password of {instance}") };
const PROJECT: Kind = Kind { suffix: ".project.dpapi", entropy: b"lampo/mysql-project-password/v1", describe: |project| format!("password of the MySQL account of {project}") };

/// The root and project-account password ports over DPAPI-encrypted files in one directory
/// (`LampoPaths::secrets_dir`).
pub struct DpapiMysqlPasswords {
    dir: PathBuf,
}

impl DpapiMysqlPasswords {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    fn file(&self, kind: &Kind, owner: &str) -> PathBuf {
        self.dir.join(format!("{owner}{}", kind.suffix))
    }

    fn read(&self, kind: &Kind, owner: &str) -> PortResult<Option<MysqlPassword>> {
        let file = self.file(kind, owner);
        let Some(sealed) = read_bounded(&file)? else { return Ok(None) };
        let unreadable = |reason: String| PortError::invalid_data(file.display().to_string(), format!("the stored {} cannot be read: {reason}", (kind.describe)(owner)));
        let plain = dpapi::unprotect(&sealed, kind.entropy).map_err(|e| unreadable(format!("{e} (was it saved by another Windows user?)")))?;
        let text = String::from_utf8(plain).map_err(|_| unreadable("not UTF-8".into()))?;
        MysqlPassword::new(text).map(Some).map_err(|e| unreadable(e.to_string()))
    }

    fn seal(kind: &Kind, owner: &str, password: &MysqlPassword) -> PortResult<Vec<u8>> {
        dpapi::protect(password.expose().as_bytes(), kind.entropy).map_err(|e| PortError::io(format!("encrypting the {}", (kind.describe)(owner)), e))
    }
}

fn read_bounded(file: &Path) -> PortResult<Option<Vec<u8>>> {
    let len = match std::fs::metadata(file) {
        Ok(meta) => meta.len(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(PortError::io(format!("reading {}", file.display()), e)),
    };
    if len > MAX_FILE_BYTES {
        return Err(PortError::invalid_data(file.display().to_string(), format!("{len} bytes: not a stored MySQL password")));
    }
    std::fs::read(file).map(Some).map_err(|e| PortError::io(format!("reading {}", file.display()), e))
}

/// Writes `contents` to `file` unless it exists, atomically: a complete file or none, and of two
/// writers racing exactly one wins. `false` when the file existed.
fn write_new(file: &Path, contents: &[u8]) -> PortResult<bool> {
    use std::io::Write;
    let context = || format!("writing {}", file.display());
    let dir = file.parent().unwrap_or(Path::new("."));
    lampo_infra_fs::ensure_dir(dir)?;
    let mut staged = tempfile::Builder::new().prefix(".lampo-").suffix(".tmp").tempfile_in(dir).map_err(|e| PortError::io(context(), e))?;
    staged.write_all(contents).map_err(|e| PortError::io(context(), e))?;
    staged.as_file().sync_all().map_err(|e| PortError::io(context(), e))?;
    match staged.persist_noclobber(file) {
        Ok(_) => Ok(true),
        Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(e) => Err(PortError::io(context(), e.error)),
    }
}

impl MysqlRootPasswordReader for DpapiMysqlPasswords {
    fn root_password(&self, instance: &ServiceId) -> PortResult<Option<MysqlPassword>> {
        self.read(&ROOT, &instance.to_string())
    }
}

impl MysqlRootPasswordWriter for DpapiMysqlPasswords {
    fn save_root_password(&self, instance: &ServiceId, password: &MysqlPassword) -> PortResult<()> {
        let owner = instance.to_string();
        lampo_infra_fs::write_atomic(&self.file(&ROOT, &owner), &Self::seal(&ROOT, &owner, password)?)
    }
}

impl MysqlProjectPasswordReader for DpapiMysqlPasswords {
    fn project_password(&self, project: &ProjectId) -> PortResult<Option<MysqlPassword>> {
        self.read(&PROJECT, project.as_str())
    }
}

impl MysqlProjectPasswordWriter for DpapiMysqlPasswords {
    fn save_project_password_if_absent(&self, project: &ProjectId, password: &MysqlPassword) -> PortResult<MysqlPassword> {
        let owner = project.as_str();
        if write_new(&self.file(&PROJECT, owner), &Self::seal(&PROJECT, owner, password)?)? {
            return Ok(password.clone());
        }
        self.read(&PROJECT, owner)?.ok_or_else(|| PortError::invalid_data(self.file(&PROJECT, owner).display().to_string(), "vanished while being read"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instance() -> ServiceId {
        "mysql-8.4".parse().unwrap()
    }

    fn shop() -> ProjectId {
        ProjectId::new("shop").unwrap()
    }

    #[test]
    fn nothing_stored_reads_as_no_password() {
        let dir = tempfile::tempdir().unwrap();
        let store = DpapiMysqlPasswords::new(dir.path().join("secrets"));
        assert_eq!(store.root_password(&instance()).unwrap(), None);
        assert_eq!(store.project_password(&shop()).unwrap(), None);
    }

    #[test]
    fn saves_encrypted_and_reads_back_per_instance() {
        let dir = tempfile::tempdir().unwrap();
        let store = DpapiMysqlPasswords::new(dir.path().join("secrets"));
        let password = MysqlPassword::new("s3cret àè #1").unwrap();
        store.save_root_password(&instance(), &password).unwrap();

        let file = dir.path().join("secrets").join("mysql-8.4.root.dpapi");
        let sealed = std::fs::read(&file).unwrap();
        assert!(!sealed.windows(6).any(|w| w == b"s3cret"), "never stored in clear");
        assert_eq!(store.root_password(&instance()).unwrap(), Some(password));
        assert_eq!(store.root_password(&"mysql-5.7".parse().unwrap()).unwrap(), None, "one password per instance");

        let replaced = MysqlPassword::new("other").unwrap();
        store.save_root_password(&instance(), &replaced).unwrap();
        assert_eq!(store.root_password(&instance()).unwrap(), Some(replaced));
    }

    #[test]
    fn a_project_password_is_stored_once_and_the_first_one_wins() {
        let dir = tempfile::tempdir().unwrap();
        let store = DpapiMysqlPasswords::new(dir.path().join("secrets"));
        let first = MysqlPassword::new("first-pass").unwrap();
        assert_eq!(store.save_project_password_if_absent(&shop(), &first).unwrap(), first);
        let second = MysqlPassword::new("second-pass").unwrap();
        assert_eq!(store.save_project_password_if_absent(&shop(), &second).unwrap(), first, "an existing password is kept");
        assert_eq!(store.project_password(&shop()).unwrap(), Some(first));

        let sealed = std::fs::read(dir.path().join("secrets").join("shop.project.dpapi")).unwrap();
        assert!(!sealed.windows(5).any(|w| w == b"first"), "never stored in clear");
        let leftovers: Vec<_> = std::fs::read_dir(dir.path().join("secrets")).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(leftovers, ["shop.project.dpapi"], "no staged file left behind");
    }

    #[test]
    fn racing_first_saves_agree_on_one_password() {
        let dir = tempfile::tempdir().unwrap();
        let store = std::sync::Arc::new(DpapiMysqlPasswords::new(dir.path()));
        let saved: Vec<MysqlPassword> = (0..8)
            .map(|i| {
                let store = store.clone();
                std::thread::spawn(move || store.save_project_password_if_absent(&shop(), &MysqlPassword::new(format!("candidate-{i}")).unwrap()).unwrap())
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|t| t.join().unwrap())
            .collect();
        assert!(saved.windows(2).all(|w| w[0] == w[1]), "{saved:?}");
        assert_eq!(store.project_password(&shop()).unwrap().as_ref(), saved.first());
    }

    #[test]
    fn the_two_kinds_never_read_each_others_files() {
        let dir = tempfile::tempdir().unwrap();
        let store = DpapiMysqlPasswords::new(dir.path());
        store.save_root_password(&instance(), &MysqlPassword::new("root-pass").unwrap()).unwrap();
        std::fs::copy(dir.path().join("mysql-8.4.root.dpapi"), dir.path().join("shop.project.dpapi")).unwrap();
        assert!(matches!(store.project_password(&shop()), Err(PortError::InvalidData { .. })), "bound to its purpose");
    }

    #[test]
    fn a_file_that_is_not_ours_is_invalid_data() {
        let dir = tempfile::tempdir().unwrap();
        let store = DpapiMysqlPasswords::new(dir.path());
        let file = dir.path().join("mysql-8.4.root.dpapi");
        std::fs::write(&file, b"plain text password").unwrap();
        assert!(matches!(store.root_password(&instance()), Err(PortError::InvalidData { .. })));
        std::fs::write(&file, vec![0u8; MAX_FILE_BYTES as usize + 1]).unwrap();
        assert!(matches!(store.root_password(&instance()), Err(PortError::InvalidData { .. })));
    }
}
