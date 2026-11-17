//! The only folder the Defender exclusion may name: `<profile>\AppData\Local\Lampo\mysql`, for a
//! user profile registered on this machine. Checked by the adapter before the UAC prompt and
//! again by the elevated helper, which must never become a way to exclude any other path. The
//! executables a process exclusion may name follow the same rules ([`super::executable`]).
//!
//! * [`resolve_target`] is pure: the requested path must be an absolute local path made only of
//!   plain components (no UNC or `\\?\` prefix, no `.`/`..`, no empty component, no trailing dot
//!   or space, no wildcard, `%` or `:` stream) and equal, ignoring case, to the folder of one of
//!   the given profiles. The folder returned is rebuilt from that profile's own path, so nothing
//!   of the request but the choice of the profile reaches Defender.
//! * [`check_no_links`] walks the folder from the drive root: no component may be a reparse
//!   point (junction, symbolic link, mount point) or a file.

use std::fmt;
use std::path::{Path, PathBuf};

/// Lampo's data folder under `%LOCALAPPDATA%`: `lampo_infra_store::paths::APP_DIR_NAME`.
pub const LAMPO_DIR: &str = "Lampo";
/// The MySQL folder of the data root: `ServiceKind::MYSQL`, holding every instance's data.
pub const MYSQL_DIR: &str = "mysql";
/// Lampo's data folder, relative to a user profile.
pub const PROFILE_DATA_DIRS: [&str; 3] = ["AppData", "Local", LAMPO_DIR];
/// The excluded folder, relative to a user profile.
pub const PROFILE_RELATIVE_DIRS: [&str; 4] = ["AppData", "Local", LAMPO_DIR, MYSQL_DIR];
/// Longest path accepted, in characters.
pub const MAX_PATH_CHARS: usize = 1024;

const SEPARATOR: char = '\\';
/// Characters Defender reads as wildcards or variables, or Windows refuses or reads as a stream.
const FORBIDDEN_CHARS: [char; 9] = ['/', ':', '*', '?', '"', '<', '>', '|', '%'];

/// Why a folder may not be excluded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetError {
    /// Not an absolute local path made of plain components.
    Malformed,
    /// Not `<profile>\AppData\Local\Lampo\mysql` for a user profile of this machine.
    NotLampoMysqlFolder,
    /// Not `php-cgi.exe` of a PHP package or `bin\httpd.exe` of an Apache package under
    /// `<profile>\AppData\Local\Lampo\packages` ([`super::executable`]).
    NotLampoExecutable,
    /// A component is a junction, symbolic link or mount point.
    ReparsePoint(PathBuf),
    /// A component is a file.
    NotADirectory(PathBuf),
    /// The executable is a folder.
    NotAFile(PathBuf),
    /// The folder or executable does not exist (adding needs it).
    Missing(PathBuf),
    Unreadable {
        path: PathBuf,
        message: String,
    },
}

impl fmt::Display for TargetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed => write!(f, "not an absolute local path made of plain folder names"),
            Self::NotLampoMysqlFolder => write!(
                f,
                "only the MySQL data folder of a Windows user profile can be excluded (<profile>\\{}): \
                 not available with a data folder elsewhere (LAMPO_HOME, a moved %LOCALAPPDATA%)",
                PROFILE_RELATIVE_DIRS.join("\\")
            ),
            Self::NotLampoExecutable => write!(
                f,
                "only php-cgi.exe of a PHP package or bin\\httpd.exe of an Apache package installed by Lampo in a Windows user profile \
                 (<profile>\\{}\\packages) can be excluded: not available with a data folder elsewhere (LAMPO_HOME, a moved %LOCALAPPDATA%)",
                PROFILE_DATA_DIRS.join("\\")
            ),
            Self::ReparsePoint(path) => write!(f, "{} is a junction or a link", path.display()),
            Self::NotADirectory(path) => write!(f, "{} is not a folder", path.display()),
            Self::NotAFile(path) => write!(f, "{} is not a file", path.display()),
            Self::Missing(path) => write!(f, "{} does not exist", path.display()),
            Self::Unreadable { path, message } => write!(f, "cannot inspect {}: {message}", path.display()),
        }
    }
}

/// The excluded folder of the profile `requested` names, rebuilt from that profile's path.
/// Profiles whose path is not a plain local path (`%SystemRoot%\...`, UNC) never match.
pub fn resolve_target(requested: &str, profiles: &[PathBuf]) -> Result<PathBuf, TargetError> {
    let requested = plain_local_components(requested).ok_or(TargetError::Malformed)?;
    profile_components(profiles)
        .map(|profile| [profile, PROFILE_RELATIVE_DIRS.to_vec()].concat())
        .find(|expected| same_components(expected, &requested))
        .map(|expected| join_components(&expected))
        .ok_or(TargetError::NotLampoMysqlFolder)
}

/// The components of every profile that is a plain local path (a trailing separator in the
/// registry is ignored); `%SystemRoot%\...` and UNC profiles are skipped.
pub(super) fn profile_components(profiles: &[PathBuf]) -> impl Iterator<Item = Vec<&str>> {
    profiles.iter().filter_map(|profile| profile.to_str()).filter_map(|profile| plain_local_components(profile.strip_suffix(SEPARATOR).unwrap_or(profile)))
}

pub(super) fn join_components(components: &[&str]) -> PathBuf {
    PathBuf::from(components.join("\\"))
}

/// `["C:", "Users", "me"]` for `C:\Users\me`; `None` for anything but a drive-rooted path of
/// plain components.
pub(super) fn plain_local_components(path: &str) -> Option<Vec<&str>> {
    if path.chars().count() > MAX_PATH_CHARS {
        return None;
    }
    let mut parts = path.split(SEPARATOR);
    let drive = parts.next()?;
    let is_drive = matches!(drive.as_bytes(), [letter, b':'] if letter.is_ascii_alphabetic());
    let rest: Vec<&str> = parts.collect();
    (is_drive && !rest.is_empty() && rest.iter().all(|c| is_plain_component(c))).then(|| [vec![drive], rest].concat())
}

fn is_plain_component(component: &str) -> bool {
    !component.is_empty() && component != "." && component != ".." && !component.ends_with(['.', ' ']) && !component.chars().any(|c| c.is_control() || FORBIDDEN_CHARS.contains(&c))
}

pub(super) fn same_components(a: &[&str], b: &[&str]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.to_lowercase() == y.to_lowercase())
}

/// Every component of `folder`, from the drive root down, is a real folder. A missing component
/// ends the walk: an error when `must_exist`, fine otherwise (nothing below it can be a link).
pub fn check_no_links(folder: &Path, must_exist: bool) -> Result<(), TargetError> {
    let mut chain: Vec<&Path> = folder.ancestors().filter(|p| !p.as_os_str().is_empty()).collect();
    chain.reverse();
    for path in chain {
        match std::fs::symlink_metadata(path) {
            Ok(meta) if is_reparse_point(&meta) => return Err(TargetError::ReparsePoint(path.to_path_buf())),
            Ok(meta) if !meta.is_dir() => return Err(TargetError::NotADirectory(path.to_path_buf())),
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return if must_exist { Err(TargetError::Missing(folder.to_path_buf())) } else { Ok(()) };
            }
            Err(e) => return Err(TargetError::Unreadable { path: path.to_path_buf(), message: e.to_string() }),
        }
    }
    Ok(())
}

#[cfg(windows)]
pub(super) fn is_reparse_point(meta: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
    meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
pub(super) fn is_reparse_point(meta: &std::fs::Metadata) -> bool {
    meta.file_type().is_symlink()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ME: &str = r"C:\Users\me";
    const MINE: &str = r"C:\Users\me\AppData\Local\Lampo\mysql";

    fn profiles() -> Vec<PathBuf> {
        [ME, r"D:\Profiles\Other Admin", r"%SystemRoot%\ServiceProfiles\LocalService", r"\\server\profiles\roaming"].map(PathBuf::from).to_vec()
    }

    #[test]
    fn the_folder_of_a_profile_is_accepted_and_rebuilt_from_it() {
        assert_eq!(resolve_target(MINE, &profiles()), Ok(PathBuf::from(MINE)));
        // Another administrator approving the prompt: the folder is still the requesting user's.
        let other = r"D:\Profiles\Other Admin\AppData\Local\Lampo\mysql";
        assert_eq!(resolve_target(other, &profiles()), Ok(PathBuf::from(other)));
        // Case differences are Windows' own: the result keeps the profile's spelling.
        assert_eq!(resolve_target(r"c:\users\ME\appdata\local\LAMPO\MySQL", &profiles()), Ok(PathBuf::from(MINE)));
        assert_eq!(resolve_target(MINE, &[PathBuf::from(r"C:\Users\me\")]), Ok(PathBuf::from(MINE)), "a trailing separator in the registry");
    }

    #[test]
    fn any_other_folder_is_refused() {
        for other in [
            r"C:\Users\me\AppData\Local\Lampo\mysql2",
            r"C:\Users\me\AppData\Local\Lampo\mysql\8.4",
            r"C:\Users\me\AppData\Local\Lampo",
            r"C:\Users\me\AppData\Local\Lampo\packages",
            r"C:\Users\me\AppData\Roaming\Lampo\mysql",
            r"C:\Users\me\Lampo\mysql",
            r"C:\Lampo\mysql",
            r"C:\Users\someone\AppData\Local\Lampo\mysql",
            r"C:\Windows\System32",
            r"C:\Windows\ServiceProfiles\LocalService\AppData\Local\Lampo\mysql",
        ] {
            assert_eq!(resolve_target(other, &profiles()), Err(TargetError::NotLampoMysqlFolder), "{other}");
        }
    }

    #[test]
    fn malformed_paths_are_refused() {
        for bad in [
            "",
            r"AppData\Local\Lampo\mysql",
            r"C:AppData\Local\Lampo\mysql",
            r"\Users\me\AppData\Local\Lampo\mysql",
            r"C:\",
            r"\\server\profiles\roaming\AppData\Local\Lampo\mysql",
            r"\\?\C:\Users\me\AppData\Local\Lampo\mysql",
            r"\\.\C:\Users\me\AppData\Local\Lampo\mysql",
            r"C:\Users\me\AppData\Local\Lampo\..\Lampo\mysql",
            r"C:\Users\me\AppData\Local\Lampo\.\mysql",
            r"C:\Users\me\AppData\Local\Lampo\\mysql",
            r"C:\Users\me\AppData\Local\Lampo\mysql\",
            r"C:\Users\me\AppData\Local\Lampo\mysql.",
            r"C:\Users\me\AppData\Local\Lampo\mysql ",
            r"C:\Users\me\AppData\Local\Lampo\mysql::$DATA",
            r"C:\Users\me\AppData\Local\Lampo\mys*",
            r"C:\Users\me\AppData\Local\Lampo\mysq?",
            r"%LOCALAPPDATA%\Lampo\mysql",
            r"C:/Users/me/AppData/Local/Lampo/mysql",
            "C:\\Users\\me\\AppData\\Local\\Lampo\\mysql\n",
            "C:\\Users\\me\\AppData\\Local\\Lampo\\my\u{0}sql",
            "1:\\Users\\me\\AppData\\Local\\Lampo\\mysql",
        ] {
            assert_eq!(resolve_target(bad, &profiles()), Err(TargetError::Malformed), "{bad:?}");
        }
        let long = format!(r"C:\{}\AppData\Local\Lampo\mysql", "a".repeat(MAX_PATH_CHARS));
        assert_eq!(resolve_target(&long, &[PathBuf::from(format!(r"C:\{}", "a".repeat(MAX_PATH_CHARS)))]), Err(TargetError::Malformed));
    }

    #[test]
    fn profiles_that_are_not_plain_local_paths_never_match() {
        assert_eq!(resolve_target(MINE, &[PathBuf::from(r"C:\Users\..\Users\me")]), Err(TargetError::NotLampoMysqlFolder));
        assert_eq!(resolve_target(MINE, &[]), Err(TargetError::NotLampoMysqlFolder));
    }

    #[test]
    fn real_folders_pass_the_link_check() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("Lampo").join("mysql");
        assert_eq!(check_no_links(&folder, true), Err(TargetError::Missing(folder.clone())), "adding needs the folder");
        assert_eq!(check_no_links(&folder, false), Ok(()), "removing does not");
        std::fs::create_dir_all(&folder).unwrap();
        assert_eq!(check_no_links(&folder, true), Ok(()));
    }

    #[test]
    fn a_file_in_the_chain_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let lampo = dir.path().join("Lampo");
        std::fs::write(&lampo, b"").unwrap();
        assert_eq!(check_no_links(&lampo.join("mysql"), false), Err(TargetError::NotADirectory(lampo)));
    }

    /// `mklink /J` needs no privilege, unlike a symbolic link.
    #[cfg(windows)]
    fn junction(link: &Path, target: &Path) {
        let status = std::process::Command::new("cmd").args(["/d", "/c", "mklink", "/J"]).arg(link).arg(target).stdout(std::process::Stdio::null()).status().unwrap();
        assert!(status.success(), "mklink /J failed");
    }

    #[cfg(windows)]
    #[test]
    fn junctions_anywhere_along_the_folder_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let elsewhere = dir.path().join("elsewhere");
        std::fs::create_dir_all(elsewhere.join("mysql")).unwrap();

        // The folder itself is a junction.
        let lampo = dir.path().join("Lampo");
        std::fs::create_dir(&lampo).unwrap();
        junction(&lampo.join("mysql"), &elsewhere);
        assert_eq!(check_no_links(&lampo.join("mysql"), true), Err(TargetError::ReparsePoint(lampo.join("mysql"))));

        // A parent is a junction (`Lampo` -> elsewhere, which has a real `mysql`).
        let linked = dir.path().join("Linked");
        junction(&linked, &elsewhere);
        assert_eq!(check_no_links(&linked.join("mysql"), true), Err(TargetError::ReparsePoint(linked.clone())));
        assert_eq!(check_no_links(&linked.join("mysql"), false), Err(TargetError::ReparsePoint(linked)), "removing checks links too");
    }

    #[test]
    fn errors_name_the_folder_and_the_rule() {
        assert!(TargetError::NotLampoMysqlFolder.to_string().contains(r"AppData\Local\Lampo\mysql"));
        assert!(TargetError::ReparsePoint(PathBuf::from(MINE)).to_string().contains(MINE));
    }
}
