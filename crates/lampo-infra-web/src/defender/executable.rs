//! The only executables a Defender process exclusion may name: the servers Lampo runs from its
//! own packages, for a user profile registered on this machine:
//!
//! * `<profile>\AppData\Local\Lampo\packages\php-<major>.<minor>\php-cgi.exe`;
//! * `<profile>\AppData\Local\Lampo\packages\apache-<major>.<minor>\bin\httpd.exe`.
//!
//! Checked by the adapter before the UAC prompt and again by the elevated helper, with the rules
//! of [`super::target`]: [`resolve_executable`] is pure (a plain absolute local path, equal
//! ignoring case to one of those forms) and rebuilds the path from the profile's own path, the
//! fixed names and the series digits, so nothing else of the request reaches Defender;
//! [`check_executable`] refuses a junction or link anywhere along it, and a folder in place of
//! the file. Never a wildcard: one exclusion per installed executable.

use std::path::{Path, PathBuf};

use super::target::{PROFILE_DATA_DIRS, TargetError, check_no_links, is_reparse_point, join_components, plain_local_components, profile_components, same_components};

/// The packages folder of the data root: `LampoPaths::packages_dir`.
pub const PACKAGES_DIR: &str = "packages";
/// The PHP slot prefix (`PackageKind::PHP`) and its FastCGI executable, relative to the slot.
pub const PHP_KIND: &str = "php";
pub const PHP_CGI: [&str; 1] = ["php-cgi.exe"];
/// The Apache slot prefix (`PackageKind::APACHE`) and its server, relative to the slot.
pub const APACHE_KIND: &str = "apache";
pub const HTTPD: [&str; 2] = ["bin", "httpd.exe"];
/// Most digits of the major or minor number of a slot's series.
const MAX_SERIES_DIGITS: usize = 4;

/// A server executable of a package kind.
struct ServerExecutable {
    kind: &'static str,
    relative: &'static [&'static str],
}

const EXECUTABLES: [ServerExecutable; 2] = [ServerExecutable { kind: PHP_KIND, relative: &PHP_CGI }, ServerExecutable { kind: APACHE_KIND, relative: &HTTPD }];

/// The executable `requested` names, rebuilt from its profile's path.
pub fn resolve_executable(requested: &str, profiles: &[PathBuf]) -> Result<PathBuf, TargetError> {
    let requested = plain_local_components(requested).ok_or(TargetError::Malformed)?;
    profile_components(profiles).find_map(|profile| match_executable(&profile, &requested)).ok_or(TargetError::NotLampoExecutable)
}

/// `requested` as a server executable of `profile`'s packages folder, rebuilt.
fn match_executable(profile: &[&str], requested: &[&str]) -> Option<PathBuf> {
    let packages = [profile, &PROFILE_DATA_DIRS, &[PACKAGES_DIR]].concat();
    let (prefix, rest) = requested.split_at_checked(packages.len())?;
    let (slot, tail) = rest.split_first()?;
    if !same_components(&packages, prefix) {
        return None;
    }
    EXECUTABLES.iter().find_map(|exe| {
        let series = slot_series(slot, exe.kind)?;
        same_components(exe.relative, tail).then(|| {
            let slot = format!("{}-{series}", exe.kind);
            join_components(&[packages.as_slice(), &[slot.as_str()], exe.relative].concat())
        })
    })
}

/// `8.3` for the slot `php-8.3` of kind `php` (the kind ignoring case): `<major>.<minor>`, digits
/// only. Add-on slots (`php-redis-6.3-php8.3`) and leftovers (`php-8.3.old-1-2`) are not series.
fn slot_series<'a>(slot: &'a str, kind: &str) -> Option<&'a str> {
    let (prefix, rest) = (slot.get(..kind.len())?, slot.get(kind.len()..)?);
    if !prefix.eq_ignore_ascii_case(kind) {
        return None;
    }
    let series = rest.strip_prefix('-')?;
    let (major, minor) = series.split_once('.')?;
    [major, minor].iter().all(|n| (1..=MAX_SERIES_DIGITS).contains(&n.len()) && n.bytes().all(|b| b.is_ascii_digit())).then_some(series)
}

/// [`resolve_executable`] then [`check_executable`]: the executable both sides of the prompt
/// accept for `requested`.
pub fn validated_executable(requested: &str, profiles: &[PathBuf], must_exist: bool) -> Result<PathBuf, TargetError> {
    let exe = resolve_executable(requested, profiles)?;
    check_executable(&exe, must_exist)?;
    Ok(exe)
}

/// No component of `exe`'s folder is a link or a file, and `exe` itself is a real file. A missing
/// executable is an error only when `must_exist` (adding): removing works for a package
/// uninstalled since.
pub fn check_executable(exe: &Path, must_exist: bool) -> Result<(), TargetError> {
    let folder = exe.parent().ok_or(TargetError::Malformed)?;
    check_no_links(folder, must_exist).map_err(|e| match e {
        TargetError::Missing(_) => TargetError::Missing(exe.to_path_buf()),
        other => other,
    })?;
    match std::fs::symlink_metadata(exe) {
        Ok(meta) if is_reparse_point(&meta) => Err(TargetError::ReparsePoint(exe.to_path_buf())),
        Ok(meta) if !meta.is_file() => Err(TargetError::NotAFile(exe.to_path_buf())),
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if must_exist {
                Err(TargetError::Missing(exe.to_path_buf()))
            } else {
                Ok(())
            }
        }
        Err(e) => Err(TargetError::Unreadable { path: exe.to_path_buf(), message: e.to_string() }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PHP: &str = r"C:\Users\me\AppData\Local\Lampo\packages\php-8.3\php-cgi.exe";
    const HTTPD_EXE: &str = r"C:\Users\me\AppData\Local\Lampo\packages\apache-2.4\bin\httpd.exe";

    fn profiles() -> Vec<PathBuf> {
        [r"C:\Users\me", r"D:\Profiles\Other Admin", r"%SystemRoot%\ServiceProfiles\LocalService", r"\\server\profiles\roaming"].map(PathBuf::from).to_vec()
    }

    #[test]
    fn php_cgi_and_httpd_of_a_profile_are_accepted_and_rebuilt() {
        assert_eq!(resolve_executable(PHP, &profiles()), Ok(PathBuf::from(PHP)));
        assert_eq!(resolve_executable(HTTPD_EXE, &profiles()), Ok(PathBuf::from(HTTPD_EXE)));
        let legacy = r"C:\Users\me\AppData\Local\Lampo\packages\php-5.6\php-cgi.exe";
        assert_eq!(resolve_executable(legacy, &profiles()), Ok(PathBuf::from(legacy)));
        let other = r"D:\Profiles\Other Admin\AppData\Local\Lampo\packages\php-8.4\php-cgi.exe";
        assert_eq!(resolve_executable(other, &profiles()), Ok(PathBuf::from(other)), "another administrator approving the prompt");
        // The result keeps the profile's spelling and Lampo's own names.
        assert_eq!(resolve_executable(r"c:\users\ME\appdata\local\LAMPO\Packages\PHP-8.3\PHP-CGI.EXE", &profiles()), Ok(PathBuf::from(PHP)));
        assert_eq!(resolve_executable(r"C:\USERS\me\AppData\Local\Lampo\packages\Apache-2.4\BIN\Httpd.exe", &profiles()), Ok(PathBuf::from(HTTPD_EXE)));
    }

    #[test]
    fn other_executables_are_refused() {
        for other in [
            r"C:\Users\me\AppData\Local\Lampo\packages\php-8.3\php.exe",
            r"C:\Users\me\AppData\Local\Lampo\packages\php-8.3\php-cgi.exe.exe",
            r"C:\Users\me\AppData\Local\Lampo\packages\php-8.3\bin\php-cgi.exe",
            r"C:\Users\me\AppData\Local\Lampo\packages\php-8.3\ext\php-cgi.exe",
            r"C:\Users\me\AppData\Local\Lampo\packages\php-8.3\bin\httpd.exe",
            r"C:\Users\me\AppData\Local\Lampo\packages\apache-2.4\httpd.exe",
            r"C:\Users\me\AppData\Local\Lampo\packages\apache-2.4\php-cgi.exe",
            r"C:\Users\me\AppData\Local\Lampo\packages\apache-2.4\bin\ab.exe",
            r"C:\Users\me\AppData\Local\Lampo\packages\mysql-8.4\bin\mysqld.exe",
            r"C:\Users\me\AppData\Local\Lampo\packages\php-redis-6.3-php8.3\php-cgi.exe",
            r"C:\Users\me\AppData\Local\Lampo\packages\xdebug-3.5-php8.3\php-cgi.exe",
            r"C:\Users\me\AppData\Local\Lampo\packages\php-8.3.old-12-34\php-cgi.exe",
            r"C:\Users\me\AppData\Local\Lampo\packages\php-8.3.14\php-cgi.exe",
            r"C:\Users\me\AppData\Local\Lampo\packages\php-8\php-cgi.exe",
            r"C:\Users\me\AppData\Local\Lampo\packages\php-x.y\php-cgi.exe",
            r"C:\Users\me\AppData\Local\Lampo\packages\php-+8.3\php-cgi.exe",
            r"C:\Users\me\AppData\Local\Lampo\packages\php-12345.1\php-cgi.exe",
            r"C:\Users\me\AppData\Local\Lampo\packages\php8.3\php-cgi.exe",
            r"C:\Users\me\AppData\Local\Lampo\packages\php-cgi.exe",
            r"C:\Users\me\AppData\Local\Lampo\packages\other\php-8.3\php-cgi.exe",
            r"C:\Users\me\AppData\Local\Lampo\mysql\php-8.3\php-cgi.exe",
            r"C:\Users\me\AppData\Roaming\Lampo\packages\php-8.3\php-cgi.exe",
            r"C:\Users\me\AppData\Local\Lampo\php-8.3\php-cgi.exe",
            r"C:\Users\someone\AppData\Local\Lampo\packages\php-8.3\php-cgi.exe",
            r"C:\php\php-cgi.exe",
            r"C:\Windows\System32\cmd.exe",
            r"C:\Windows\ServiceProfiles\LocalService\AppData\Local\Lampo\packages\php-8.3\php-cgi.exe",
        ] {
            assert_eq!(resolve_executable(other, &profiles()), Err(TargetError::NotLampoExecutable), "{other}");
        }
        assert_eq!(resolve_executable(PHP, &[]), Err(TargetError::NotLampoExecutable), "no registered profile");
    }

    #[test]
    fn malformed_paths_are_refused() {
        for bad in [
            "",
            r"php-cgi.exe",
            r"AppData\Local\Lampo\packages\php-8.3\php-cgi.exe",
            r"\\server\profiles\roaming\AppData\Local\Lampo\packages\php-8.3\php-cgi.exe",
            r"\\?\C:\Users\me\AppData\Local\Lampo\packages\php-8.3\php-cgi.exe",
            r"C:\Users\me\AppData\Local\Lampo\packages\..\packages\php-8.3\php-cgi.exe",
            r"C:\Users\me\AppData\Local\Lampo\packages\php-8.3\.\php-cgi.exe",
            r"C:\Users\me\AppData\Local\Lampo\packages\php-8.3\php-cgi.exe\",
            r"C:\Users\me\AppData\Local\Lampo\packages\php-8.3\php-cgi.exe.",
            r"C:\Users\me\AppData\Local\Lampo\packages\php-8.3\php-cgi.exe:stream",
            r"C:\Users\me\AppData\Local\Lampo\packages\php-*\php-cgi.exe",
            r"C:\Users\me\AppData\Local\Lampo\packages\php-8.3\php-cgi.ex?",
            r"%LOCALAPPDATA%\Lampo\packages\php-8.3\php-cgi.exe",
            r"C:/Users/me/AppData/Local/Lampo/packages/php-8.3/php-cgi.exe",
            "C:\\Users\\me\\AppData\\Local\\Lampo\\packages\\php-8.3\\php-cgi.exe\n",
        ] {
            assert_eq!(resolve_executable(bad, &profiles()), Err(TargetError::Malformed), "{bad:?}");
        }
    }

    #[test]
    fn adding_needs_a_real_file_removing_does_not() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("php-8.3").join("php-cgi.exe");
        assert_eq!(check_executable(&exe, true), Err(TargetError::Missing(exe.clone())), "no folder");
        assert_eq!(check_executable(&exe, false), Ok(()), "an uninstalled package");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        assert_eq!(check_executable(&exe, true), Err(TargetError::Missing(exe.clone())), "no file");
        assert_eq!(check_executable(&exe, false), Ok(()));
        std::fs::write(&exe, b"").unwrap();
        assert_eq!(check_executable(&exe, true), Ok(()));
    }

    #[test]
    fn a_folder_in_place_of_the_executable_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("php-cgi.exe");
        std::fs::create_dir(&exe).unwrap();
        assert_eq!(check_executable(&exe, false), Err(TargetError::NotAFile(exe)));
    }

    #[cfg(windows)]
    #[test]
    fn a_junction_along_the_executable_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let elsewhere = dir.path().join("elsewhere");
        std::fs::create_dir(&elsewhere).unwrap();
        std::fs::write(elsewhere.join("php-cgi.exe"), b"").unwrap();
        let slot = dir.path().join("php-8.3");
        let status = std::process::Command::new("cmd").args(["/d", "/c", "mklink", "/J"]).arg(&slot).arg(&elsewhere).stdout(std::process::Stdio::null()).status().unwrap();
        assert!(status.success(), "mklink /J failed");
        for must_exist in [true, false] {
            assert_eq!(check_executable(&slot.join("php-cgi.exe"), must_exist), Err(TargetError::ReparsePoint(slot.clone())), "must_exist: {must_exist}");
        }
    }

    #[test]
    fn the_error_names_the_rule() {
        let text = TargetError::NotLampoExecutable.to_string();
        assert!(text.contains("php-cgi.exe") && text.contains(r"bin\httpd.exe") && text.contains(r"AppData\Local\Lampo\packages"), "{text}");
    }
}
