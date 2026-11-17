//! The only folders a projects-folder exclusion may name: a folder the user keeps projects in,
//! inside a user profile registered on this machine. Unlike the MySQL folder and the server
//! executables the name is free, so the rules say what it may never be. Checked by the adapter
//! before the UAC prompt and again by the elevated helper; the configuration of the projects
//! (a file the user can write) never decides anything here, it only suggests a folder.
//!
//! [`resolve_projects_folder`] is pure. The requested path must be an absolute local path made of
//! plain components ([`super::target`]: no UNC or `\\?\` prefix, no `.`/`..`, no empty
//! component, no trailing dot or space, no wildcard, `%` or `:` stream) and:
//!
//! * not a **system folder**: the drive root, `<drive>\Windows`, `<drive>\Program Files*`,
//!   `<drive>\ProgramData` and anything below them;
//! * strictly **inside a registered profile** (`C:\Users\<user>\...`), never the profile itself
//!   nor a folder holding a profile, and accepted by the rules of every profile it is in;
//! * never in **`AppData`** (programs' data and settings, the Startup folder) nor in a
//!   **`Start Menu`** folder (where the Startup folder of a profile lives);
//! * never the root of a **main user folder**: `Desktop`, `Documents`, `Downloads`, also under a
//!   first-level `OneDrive*` folder (their redirected copies), nor that `OneDrive*` folder
//!   itself. Their subfolders (`Desktop\Progetti`) are fine;
//! * without **`~`** below the profile: an 8.3 short name (`APPDAT~1`) would hide one of the
//!   names above.
//!
//! The folder returned keeps the profile's own spelling, then the requested names.
//! [`validated_projects_folder`] then refuses a junction, link or mount point anywhere along it
//! ([`check_no_links`]), and a missing folder when adding.

use std::fmt;
use std::path::PathBuf;

use super::target::{TargetError, check_no_links, join_components, plain_local_components, profile_components, same_components};

/// The data folder of a profile: never excluded, nor anything in it.
pub const APP_DATA_DIR: &str = "AppData";
/// The folder holding the Startup folder (`...\Start Menu\Programs\Startup`).
pub const START_MENU_DIR: &str = "Start Menu";
/// The main user folders: their subfolders may be excluded, they themselves may not.
pub const USER_FOLDERS: [&str; 3] = ["Desktop", "Documents", "Downloads"];
/// The prefix of the OneDrive folders of a profile (`OneDrive`, `OneDrive - Contoso`), where the
/// main user folders are redirected when backed up.
pub const ONE_DRIVE_PREFIX: &str = "OneDrive";
/// System folders directly under a drive root: never excluded, nor anything in them.
pub const SYSTEM_DIRS: [&str; 2] = ["Windows", "ProgramData"];
/// The prefix of `Program Files` and `Program Files (x86)`.
pub const PROGRAM_FILES_PREFIX: &str = "Program Files";
/// Marks an 8.3 short name.
const SHORT_NAME_MARK: char = '~';

/// Why a folder may not be the projects folder (beyond its form, [`TargetError::Malformed`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectsFolderRule {
    /// The drive's `Windows`, `Program Files*` or `ProgramData`, or a folder in them.
    SystemFolder,
    /// Not inside a user profile registered on this machine.
    OutsideProfiles,
    /// A user profile itself, or a folder holding one.
    ProfileFolder,
    /// `AppData` of a profile or a folder in it.
    AppData,
    /// A `Start Menu` folder or a folder in it.
    StartMenu,
    /// `Desktop`, `Documents` or `Downloads` itself, or a `OneDrive*` folder (or its copies of them).
    UserFolderRoot,
    /// A folder name holding `~` (an 8.3 short name hides the real name).
    ShortName,
}

impl fmt::Display for ProjectsFolderRule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::SystemFolder => "Windows, Program Files and ProgramData and the folders in them are never excluded",
            Self::OutsideProfiles => "the projects folder must be inside a Windows user profile (C:\\Users\\<user>\\...)",
            Self::ProfileFolder => "a user profile itself is never excluded: choose the folder holding your projects inside it",
            Self::AppData => "AppData and the folders in it are never excluded",
            Self::StartMenu => "Start Menu folders (where the Startup folder is) are never excluded",
            Self::UserFolderRoot => "Desktop, Documents, Downloads and OneDrive themselves are never excluded: choose a folder in them (e.g. Desktop\\Progetti)",
            Self::ShortName => "folder names holding `~` are refused: use the full names, not the 8.3 short ones",
        })
    }
}

/// The projects folder `requested` names, with its profile's spelling.
pub fn resolve_projects_folder(requested: &str, profiles: &[PathBuf]) -> Result<PathBuf, TargetError> {
    let requested = plain_local_components(requested).ok_or(TargetError::Malformed)?;
    let refuse = |rule| Err(TargetError::NotProjectsFolder(rule));
    if is_system_folder(&requested) {
        return refuse(ProjectsFolderRule::SystemFolder);
    }
    let registered: Vec<Vec<&str>> = profile_components(profiles).collect();
    let containing: Vec<&Vec<&str>> = registered.iter().filter(|profile| starts_with(&requested, profile)).collect();
    let Some(spelling) = containing.iter().max_by_key(|profile| profile.len()) else { return refuse(ProjectsFolderRule::OutsideProfiles) };
    if registered.iter().any(|profile| starts_with(profile, &requested)) {
        return refuse(ProjectsFolderRule::ProfileFolder);
    }
    if let Some(rule) = containing.iter().find_map(|profile| broken_rule(&requested[profile.len()..])) {
        return refuse(rule);
    }
    Ok(join_components(&[spelling.as_slice(), &requested[spelling.len()..]].concat()))
}

/// [`resolve_projects_folder`] then [`check_no_links`]: the folder both sides of the prompt
/// accept for `requested`. Adding needs the folder (`must_exist`); removing does not.
pub fn validated_projects_folder(requested: &str, profiles: &[PathBuf], must_exist: bool) -> Result<PathBuf, TargetError> {
    let folder = resolve_projects_folder(requested, profiles)?;
    check_no_links(&folder, must_exist)?;
    Ok(folder)
}

/// `path` is `prefix` or below it, ignoring case.
fn starts_with(path: &[&str], prefix: &[&str]) -> bool {
    path.get(..prefix.len()).is_some_and(|head| same_components(head, prefix))
}

/// The drive's `Windows`, `ProgramData` or `Program Files*`, or below them.
fn is_system_folder(path: &[&str]) -> bool {
    path.get(1).is_some_and(|top| SYSTEM_DIRS.iter().any(|dir| top.eq_ignore_ascii_case(dir)) || starts_with_ignoring_case(top, PROGRAM_FILES_PREFIX))
}

/// The rule `relative` (the components below a profile) breaks, if any.
fn broken_rule(relative: &[&str]) -> Option<ProjectsFolderRule> {
    let is = |component: &str, name: &str| component.eq_ignore_ascii_case(name);
    let Some(first) = relative.first() else { return Some(ProjectsFolderRule::ProfileFolder) };
    if is(first, APP_DATA_DIR) {
        return Some(ProjectsFolderRule::AppData);
    }
    if relative.iter().any(|c| is(c, START_MENU_DIR)) {
        return Some(ProjectsFolderRule::StartMenu);
    }
    if relative.iter().any(|c| c.contains(SHORT_NAME_MARK)) {
        return Some(ProjectsFolderRule::ShortName);
    }
    let is_user_folder = |c: &str| USER_FOLDERS.iter().any(|name| is(c, name));
    let user_folder_root = match relative {
        [only] => is_user_folder(only) || starts_with_ignoring_case(only, ONE_DRIVE_PREFIX),
        [one_drive, folder] => starts_with_ignoring_case(one_drive, ONE_DRIVE_PREFIX) && is_user_folder(folder),
        _ => false,
    };
    user_folder_root.then_some(ProjectsFolderRule::UserFolderRoot)
}

fn starts_with_ignoring_case(text: &str, prefix: &str) -> bool {
    text.get(..prefix.len()).is_some_and(|head| head.eq_ignore_ascii_case(prefix))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    const PROJECTS: &str = r"C:\Users\me\Desktop\Progetti";

    fn profiles() -> Vec<PathBuf> {
        [r"C:\Users\me", r"D:\Profiles\Other Admin", r"%SystemRoot%\ServiceProfiles\LocalService", r"\\server\profiles\roaming"].map(PathBuf::from).to_vec()
    }

    fn refused(path: &str) -> Result<PathBuf, TargetError> {
        resolve_projects_folder(path, &profiles())
    }

    #[test]
    fn folders_inside_a_profile_are_accepted_with_its_spelling() {
        for accepted in [
            PROJECTS,
            r"C:\Users\me\Desktop\Progetti\cliente\sito",
            r"C:\Users\me\Documents\Code",
            r"C:\Users\me\Downloads\progetti",
            r"C:\Users\me\Progetti",
            r"C:\Users\me\source\repos",
            r"C:\Users\me\OneDrive\Desktop\Progetti",
            r"C:\Users\me\OneDrive - Contoso\Progetti",
            r"C:\Users\me\Projects\startup",
            r"C:\Users\me\Projects\Windows",
            r"D:\Profiles\Other Admin\Progetti",
        ] {
            assert_eq!(refused(accepted), Ok(PathBuf::from(accepted)), "{accepted}");
        }
        assert_eq!(refused(r"c:\USERS\ME\Desktop\Progetti"), Ok(PathBuf::from(r"C:\Users\me\Desktop\Progetti")), "the profile's spelling, the requested names");
        assert_eq!(resolve_projects_folder(PROJECTS, &[PathBuf::from(r"C:\Users\me\")]), Ok(PathBuf::from(PROJECTS)), "a trailing separator in the registry");
    }

    #[test]
    fn system_folders_and_drive_roots_are_refused() {
        for system in [r"C:\Windows", r"C:\Windows\System32", r"c:\WINDOWS\Temp\x", r"C:\Program Files", r"C:\Program Files (x86)\x", r"C:\ProgramData", r"D:\ProgramData\x"] {
            assert_eq!(refused(system), Err(TargetError::NotProjectsFolder(ProjectsFolderRule::SystemFolder)), "{system}");
        }
        // Even when a profile is registered there.
        let odd = [PathBuf::from(r"C:\Windows\Profiles\me")];
        assert_eq!(resolve_projects_folder(r"C:\Windows\Profiles\me\Progetti", &odd), Err(TargetError::NotProjectsFolder(ProjectsFolderRule::SystemFolder)));
        for root in [r"C:\", "C:", r"D:\"] {
            assert_eq!(refused(root), Err(TargetError::Malformed), "{root}");
        }
    }

    #[test]
    fn folders_outside_every_profile_are_refused() {
        for outside in [r"C:\Progetti", r"D:\www\sites", r"C:\Users\someone\Progetti", r"C:\Users\Public\Progetti", r"C:\Users\mex\Progetti", r"E:\Profiles\Other Admin\x"] {
            assert_eq!(refused(outside), Err(TargetError::NotProjectsFolder(ProjectsFolderRule::OutsideProfiles)), "{outside}");
        }
        assert_eq!(resolve_projects_folder(PROJECTS, &[]), Err(TargetError::NotProjectsFolder(ProjectsFolderRule::OutsideProfiles)), "no registered profile");
        assert_eq!(resolve_projects_folder(r"\\server\profiles\roaming\Progetti", &profiles()), Err(TargetError::Malformed), "UNC profiles never match");
    }

    #[test]
    fn a_profile_itself_or_a_folder_holding_one_is_refused() {
        for profile in [r"C:\Users\me", r"c:\users\ME", r"D:\Profiles\Other Admin"] {
            assert_eq!(refused(profile), Err(TargetError::NotProjectsFolder(ProjectsFolderRule::ProfileFolder)), "{profile}");
        }
        let nested = [PathBuf::from(r"D:\Profiles"), PathBuf::from(r"D:\Profiles\Other")];
        assert_eq!(resolve_projects_folder(r"D:\Profiles\Other", &nested), Err(TargetError::NotProjectsFolder(ProjectsFolderRule::ProfileFolder)));
        assert_eq!(resolve_projects_folder(r"D:\Profiles\Other\AppData\x", &nested), Err(TargetError::NotProjectsFolder(ProjectsFolderRule::AppData)), "every profile's rules");
        assert_eq!(resolve_projects_folder(r"D:\Profiles\Other\Progetti", &nested), Ok(PathBuf::from(r"D:\Profiles\Other\Progetti")));
        let inner = [PathBuf::from(r"D:\Profiles"), PathBuf::from(r"D:\Profiles\Team\Other")];
        assert_eq!(resolve_projects_folder(r"D:\Profiles\Team", &inner), Err(TargetError::NotProjectsFolder(ProjectsFolderRule::ProfileFolder)), "holds a profile");
    }

    #[test]
    fn app_data_and_start_menu_folders_are_refused() {
        for app_data in
            [r"C:\Users\me\AppData", r"C:\Users\me\appdata\Local", r"C:\Users\me\AppData\Local\Lampo", r"C:\Users\me\AppData\Roaming\Microsoft\Windows\Start Menu\Programs\Startup"]
        {
            assert_eq!(refused(app_data), Err(TargetError::NotProjectsFolder(ProjectsFolderRule::AppData)), "{app_data}");
        }
        for start_menu in [r"C:\Users\me\Start Menu", r"C:\Users\me\Start Menu\Programs\Startup", r"C:\Users\me\Desktop\start menu\x"] {
            assert_eq!(refused(start_menu), Err(TargetError::NotProjectsFolder(ProjectsFolderRule::StartMenu)), "{start_menu}");
        }
    }

    #[test]
    fn main_user_folders_themselves_are_refused() {
        for root in [
            r"C:\Users\me\Desktop",
            r"C:\Users\me\documents",
            r"C:\Users\me\DOWNLOADS",
            r"C:\Users\me\OneDrive",
            r"C:\Users\me\OneDrive - Contoso",
            r"C:\Users\me\OneDrive\Desktop",
            r"C:\Users\me\OneDrive - Contoso\Documents",
        ] {
            assert_eq!(refused(root), Err(TargetError::NotProjectsFolder(ProjectsFolderRule::UserFolderRoot)), "{root}");
        }
    }

    #[test]
    fn short_names_are_refused() {
        for short in [r"C:\Users\me\APPDAT~1\Local", r"C:\Users\me\DESKTO~1", r"C:\Users\me\Desktop\PROGET~1"] {
            assert_eq!(refused(short), Err(TargetError::NotProjectsFolder(ProjectsFolderRule::ShortName)), "{short}");
        }
    }

    #[test]
    fn malformed_paths_are_refused() {
        for bad in [
            "",
            r"Desktop\Progetti",
            r"\Users\me\Desktop\Progetti",
            r"\\?\C:\Users\me\Desktop\Progetti",
            r"\\server\share\Progetti",
            r"C:\Users\me\Desktop\..\AppData",
            r"C:\Users\me\.\AppData",
            r"C:\Users\me\Desktop\Progetti\",
            r"C:\Users\me\Desktop\Progetti.",
            r"C:\Users\me\Desktop\Progetti ",
            r"C:\Users\me\Desktop\Progetti::$INDEX_ALLOCATION",
            r"C:\Users\me\Desktop\*",
            r"%USERPROFILE%\Desktop\Progetti",
            "C:/Users/me/Desktop/Progetti",
            "C:\\Users\\me\\Desktop\\Progetti\n",
        ] {
            assert_eq!(refused(bad), Err(TargetError::Malformed), "{bad:?}");
        }
    }

    #[test]
    fn adding_needs_an_existing_folder_without_links() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("me");
        let projects = profile.join(r"Desktop\Progetti");
        let requested = projects.display().to_string();
        let profiles = [profile.clone()];
        assert_eq!(validated_projects_folder(&requested, &profiles, true), Err(TargetError::Missing(projects.clone())));
        assert_eq!(validated_projects_folder(&requested, &profiles, false), Ok(projects.clone()), "removing a folder deleted since");
        std::fs::create_dir_all(&projects).unwrap();
        assert_eq!(validated_projects_folder(&requested, &profiles, true), Ok(projects.clone()));
        let file = profile.join("notes.txt");
        std::fs::write(&file, b"").unwrap();
        assert_eq!(validated_projects_folder(&file.display().to_string(), &profiles, true), Err(TargetError::NotADirectory(file)));
    }

    /// `mklink /J` needs no privilege, unlike a symbolic link.
    #[cfg(windows)]
    fn junction(link: &Path, target: &Path) {
        let status = std::process::Command::new("cmd").args(["/d", "/c", "mklink", "/J"]).arg(link).arg(target).stdout(std::process::Stdio::null()).status().unwrap();
        assert!(status.success(), "mklink /J failed");
    }

    #[cfg(windows)]
    #[test]
    fn a_junction_along_the_folder_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("me");
        let app_data = profile.join(r"AppData\Local");
        std::fs::create_dir_all(&app_data).unwrap();
        std::fs::create_dir_all(profile.join("Desktop")).unwrap();
        // `Desktop\Progetti` -> `AppData\Local`: the names pass, the link does not.
        let linked = profile.join(r"Desktop\Progetti");
        junction(&linked, &app_data);
        for must_exist in [true, false] {
            let result = validated_projects_folder(&linked.display().to_string(), std::slice::from_ref(&profile), must_exist);
            assert_eq!(result, Err(TargetError::ReparsePoint(linked.clone())), "must_exist: {must_exist}");
        }
        let below = linked.join("x");
        assert_eq!(validated_projects_folder(&below.display().to_string(), &[profile], false), Err(TargetError::ReparsePoint(linked)));
    }

    #[test]
    fn errors_say_the_rule() {
        let text = TargetError::NotProjectsFolder(ProjectsFolderRule::UserFolderRoot).to_string();
        assert!(text.contains("Desktop") && text.contains("Desktop\\Progetti"), "{text}");
        assert!(TargetError::NotProjectsFolder(ProjectsFolderRule::OutsideProfiles).to_string().contains(r"C:\Users"));
    }
}
