//! Hands a stored MySQL password to a client without putting it on its command line (where other
//! processes and logs see it): a temporary option file ([`MysqlPassword::client_option_file`])
//! created with a descriptor that lets only the current user open it, so it is never readable
//! with inherited permissions, and deleted when the [`ClientOptionFile`] is dropped, after the
//! client has exited. Statements carrying a password (`ALTER USER … IDENTIFIED BY`) reach the
//! client the same way, as a [`SecretTempFile`] it reads on its standard input.

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::ptr::null_mut;

use lampo_domain::mysql::credentials::MysqlPassword;
use lampo_domain::ports::MysqlRootPasswordReader;
use lampo_domain::service::ServiceId;
use lampo_domain::{PortError, PortResult};
use windows_sys::Win32::Foundation::GENERIC_WRITE;
use windows_sys::Win32::Storage::FileSystem::{CREATE_NEW, CreateFileW, FILE_ATTRIBUTE_TEMPORARY};

use crate::win::{UserOnlySecurity, owned, wide};

const PREFIX: &str = "lampo-mysql-";
const OPTION_FILE_SUFFIX: &str = ".cnf";
/// Random characters of the file name (unguessable, and no collision between concurrent clients).
const RANDOM_CHARS: usize = 16;

/// A temporary file only the current user can open, holding a secret, deleted when dropped.
#[derive(Debug)]
pub(crate) struct SecretTempFile {
    path: Option<tempfile::TempPath>,
}

impl SecretTempFile {
    /// Writes `contents` to a new file named `lampo-mysql-<random><suffix>` in `dir`.
    pub(crate) fn write_in(dir: &Path, suffix: &str, contents: &[u8]) -> PortResult<Self> {
        let context = || format!("writing a temporary MySQL file in {}", dir.display());
        let security = UserOnlySecurity::new(false).map_err(|e| PortError::io(context(), e))?;
        let mut file =
            tempfile::Builder::new().prefix(PREFIX).suffix(suffix).rand_bytes(RANDOM_CHARS).make_in(dir, |path| create_new(path, &security)).map_err(|e| PortError::io(context(), e))?;
        // A failed write drops `file`, which deletes it.
        file.write_all(contents).map_err(|e| PortError::io(context(), e))?;
        file.as_file().sync_all().map_err(|e| PortError::io(context(), e))?;
        // Closed here: the client opens it by name.
        Ok(Self { path: Some(file.into_temp_path()) })
    }

    pub(crate) fn path(&self) -> &Path {
        self.path.as_deref().expect("present until dropped")
    }
}

impl Drop for SecretTempFile {
    fn drop(&mut self) {
        if let Some(path) = self.path.take() {
            let shown: PathBuf = path.to_path_buf();
            if let Err(e) = path.close() {
                tracing::warn!(file = %shown.display(), error = %e, "cannot delete a temporary file holding a MySQL password");
            }
        }
    }
}

/// An option file with a password, deleted when dropped: keep it alive until the client exits.
#[derive(Debug)]
pub struct ClientOptionFile {
    file: SecretTempFile,
}

impl ClientOptionFile {
    /// The option file for clients connecting as the root of `instance`, when a root password is
    /// stored for it (`None`: root has no password, the clients need no option file).
    pub fn for_instance(passwords: &dyn MysqlRootPasswordReader, instance: &ServiceId) -> PortResult<Option<Self>> {
        passwords.root_password(instance)?.map(|password| Self::for_password(&password)).transpose()
    }

    /// The option file of `password`, in the temporary folder.
    pub fn for_password(password: &MysqlPassword) -> PortResult<Self> {
        Self::write_in(&std::env::temp_dir(), password)
    }

    /// Writes the option file of `password` into `dir`.
    fn write_in(dir: &Path, password: &MysqlPassword) -> PortResult<Self> {
        Ok(Self { file: SecretTempFile::write_in(dir, OPTION_FILE_SUFFIX, password.client_option_file().as_bytes())? })
    }

    pub fn path(&self) -> &Path {
        self.file.path()
    }

    /// For Lampo's own clients: `--defaults-file=<file>` reads this file and no other option file
    /// (as `--no-defaults` would, so a stray `my.ini` cannot change the connection). Must be the
    /// first argument.
    pub fn defaults_file_arg(&self) -> String {
        format!("--defaults-file={}", self.path().display())
    }

    /// For the shims: `--defaults-extra-file=<file>` reads it after the usual option files, which
    /// keep applying. Must be the first argument.
    pub fn defaults_extra_file_arg(&self) -> String {
        format!("--defaults-extra-file={}", self.path().display())
    }
}

/// `CreateFileW(CREATE_NEW)` with `security`: the descriptor is in place from the first moment.
fn create_new(path: &Path, security: &UserOnlySecurity) -> std::io::Result<File> {
    let name = wide(path);
    // SAFETY: valid NUL-terminated path and security attributes alive for the call.
    let handle = unsafe { CreateFileW(name.as_ptr(), GENERIC_WRITE, 0, security.as_ptr(), CREATE_NEW, FILE_ATTRIBUTE_TEMPORARY, null_mut()) };
    Ok(File::from(owned(handle)?))
}

#[cfg(test)]
pub(crate) mod tests {
    use std::os::windows::io::AsRawHandle;

    use lampo_domain::ports::NoMysqlRootPasswords;
    use windows_sys::Win32::Security::Authorization::{ConvertSecurityDescriptorToStringSecurityDescriptorW, GetSecurityInfo, SDDL_REVISION_1, SE_FILE_OBJECT};
    use windows_sys::Win32::Security::{DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR};

    use super::*;
    use crate::win::current_user_sid;

    /// The DACL of `path` in SDDL (`D:P(A;;FA;;;S-1-5-…)`).
    pub(crate) fn dacl_of(path: &Path) -> String {
        let file = File::open(path).unwrap();
        unsafe {
            let mut sd: PSECURITY_DESCRIPTOR = null_mut();
            let rc = GetSecurityInfo(file.as_raw_handle() as _, SE_FILE_OBJECT, DACL_SECURITY_INFORMATION, null_mut(), null_mut(), null_mut(), null_mut(), &mut sd);
            assert_eq!(rc, 0);
            let mut text: *mut u16 = null_mut();
            assert_ne!(ConvertSecurityDescriptorToStringSecurityDescriptorW(sd, SDDL_REVISION_1, DACL_SECURITY_INFORMATION, &mut text, null_mut()), 0);
            let len = (0..).take_while(|&i| *text.add(i) != 0).count();
            let sddl = String::from_utf16_lossy(std::slice::from_raw_parts(text, len));
            windows_sys::Win32::Foundation::LocalFree(text.cast());
            windows_sys::Win32::Foundation::LocalFree(sd);
            sddl
        }
    }

    /// The only access granted is the current user's full access, nothing inherited.
    pub(crate) fn assert_user_only(path: &Path) {
        assert_eq!(dacl_of(path), format!("D:P(A;;FA;;;{})", current_user_sid().unwrap()));
    }

    struct Stored(MysqlPassword);
    impl MysqlRootPasswordReader for Stored {
        fn root_password(&self, _instance: &ServiceId) -> PortResult<Option<MysqlPassword>> {
            Ok(Some(self.0.clone()))
        }
    }

    #[test]
    fn holds_the_password_for_the_current_user_only_until_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let file = ClientOptionFile::write_in(dir.path(), &MysqlPassword::new(r"p\w #1").unwrap()).unwrap();
        let path = file.path().to_path_buf();
        assert_eq!(path.parent(), Some(dir.path()));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[client]\npassword=\"p\\\\w #1\"\n");
        assert_user_only(&path);
        assert_eq!(file.defaults_file_arg(), format!("--defaults-file={}", path.display()));
        assert_eq!(file.defaults_extra_file_arg(), format!("--defaults-extra-file={}", path.display()));
        drop(file);
        assert!(!path.exists(), "deleted after use");
    }

    #[test]
    fn concurrent_files_never_collide() {
        let dir = tempfile::tempdir().unwrap();
        let password = MysqlPassword::new("x").unwrap();
        let files: Vec<ClientOptionFile> = (0..8).map(|_| ClientOptionFile::write_in(dir.path(), &password).unwrap()).collect();
        let paths: std::collections::HashSet<&Path> = files.iter().map(ClientOptionFile::path).collect();
        assert_eq!(paths.len(), 8);
    }

    #[test]
    fn only_a_stored_password_makes_a_file() {
        let instance: ServiceId = "mysql-8.4".parse().unwrap();
        assert!(ClientOptionFile::for_instance(&NoMysqlRootPasswords, &instance).unwrap().is_none());
        let file = ClientOptionFile::for_instance(&Stored(MysqlPassword::new("s3cret").unwrap()), &instance).unwrap().unwrap();
        assert!(std::fs::read_to_string(file.path()).unwrap().contains("password=\"s3cret\""));
    }
}
