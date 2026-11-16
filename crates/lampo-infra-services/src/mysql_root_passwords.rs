//! The root passwords of the local MySQL instances, one file per instance
//! (`secrets\<service-id>.root.dpapi`) encrypted with DPAPI for the current Windows user: another
//! user, or a copy of the file on another machine, cannot decrypt it.

use std::path::{Path, PathBuf};

use lampo_domain::mysql::credentials::{MAX_PASSWORD_BYTES, MysqlPassword};
use lampo_domain::ports::{MysqlRootPasswordReader, MysqlRootPasswordWriter};
use lampo_domain::service::ServiceId;
use lampo_domain::{PortError, PortResult};

use crate::win::dpapi;

/// Binds the encrypted files to this purpose: a DPAPI blob of something else does not decrypt.
const ENTROPY: &[u8] = b"lampo/mysql-root-password/v1";
const FILE_SUFFIX: &str = ".root.dpapi";
/// A DPAPI blob of the longest password is well under this: anything larger is not ours.
const MAX_FILE_BYTES: u64 = 16 * 1024 + MAX_PASSWORD_BYTES as u64;

/// [`MysqlRootPasswordReader`] and [`MysqlRootPasswordWriter`] over DPAPI-encrypted files in one
/// directory (`LampoPaths::secrets_dir`).
pub struct DpapiMysqlRootPasswords {
    dir: PathBuf,
}

impl DpapiMysqlRootPasswords {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    fn file(&self, instance: &ServiceId) -> PathBuf {
        self.dir.join(format!("{instance}{FILE_SUFFIX}"))
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

impl MysqlRootPasswordReader for DpapiMysqlRootPasswords {
    fn root_password(&self, instance: &ServiceId) -> PortResult<Option<MysqlPassword>> {
        let file = self.file(instance);
        let Some(sealed) = read_bounded(&file)? else { return Ok(None) };
        let unreadable = |reason: String| PortError::invalid_data(file.display().to_string(), format!("the stored root password of {instance} cannot be read: {reason}"));
        let plain = dpapi::unprotect(&sealed, ENTROPY).map_err(|e| unreadable(format!("{e} (was it saved by another Windows user?)")))?;
        let text = String::from_utf8(plain).map_err(|_| unreadable("not UTF-8".into()))?;
        MysqlPassword::new(text).map(Some).map_err(|e| unreadable(e.to_string()))
    }
}

impl MysqlRootPasswordWriter for DpapiMysqlRootPasswords {
    fn save_root_password(&self, instance: &ServiceId, password: &MysqlPassword) -> PortResult<()> {
        let sealed = dpapi::protect(password.expose().as_bytes(), ENTROPY).map_err(|e| PortError::io(format!("encrypting the root password of {instance}"), e))?;
        lampo_infra_fs::write_atomic(&self.file(instance), &sealed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instance() -> ServiceId {
        "mysql-8.4".parse().unwrap()
    }

    #[test]
    fn nothing_stored_reads_as_no_password() {
        let dir = tempfile::tempdir().unwrap();
        let store = DpapiMysqlRootPasswords::new(dir.path().join("secrets"));
        assert_eq!(store.root_password(&instance()).unwrap(), None);
    }

    #[test]
    fn saves_encrypted_and_reads_back_per_instance() {
        let dir = tempfile::tempdir().unwrap();
        let store = DpapiMysqlRootPasswords::new(dir.path().join("secrets"));
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
    fn a_file_that_is_not_ours_is_invalid_data() {
        let dir = tempfile::tempdir().unwrap();
        let store = DpapiMysqlRootPasswords::new(dir.path());
        let file = dir.path().join("mysql-8.4.root.dpapi");
        std::fs::write(&file, b"plain text password").unwrap();
        assert!(matches!(store.root_password(&instance()), Err(PortError::InvalidData { .. })));
        std::fs::write(&file, vec![0u8; MAX_FILE_BYTES as usize + 1]).unwrap();
        assert!(matches!(store.root_password(&instance()), Err(PortError::InvalidData { .. })));
    }
}
