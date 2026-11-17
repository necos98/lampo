//! Defender mode of `lampo-hosts-helper.exe`:
//! `--defender-exclusion add|remove --path <folder>`.
//!
//! Security design — the helper runs as an administrator, so it must not become a way to exclude
//! any folder from the antivirus scan:
//! * **One folder.** The path must be `<profile>\AppData\Local\Lampo\mysql` for a user profile
//!   registered on this machine (`HKLM\...\ProfileList`, [`super::profiles`]); the folder handed
//!   to Defender is rebuilt from that profile's own path ([`super::target`]). Another
//!   administrator approving the prompt (whose own `%LOCALAPPDATA%` differs) still excludes the
//!   requesting user's folder, which the request names.
//! * **Real folders only.** No component may be a junction, link or mount point, and adding needs
//!   the folder to exist.
//! * **No interpolation.** Defender is reached through a constant PowerShell script, the folder
//!   through an environment variable ([`super::powershell`]).
//! * **Path exclusions only**: never a process, an extension or another folder.

use lampo_domain::scan_exclusion::ScanExclusionChange;

use super::helper_exit;
use super::powershell::{DefenderResult, DefenderSettings, PowerShellDefender};
use super::profiles::{RegistryProfiles, UserProfiles};
use super::request::ExclusionArgs;
use super::target::{check_no_links, resolve_target};

/// Entire Defender run; returns the process exit code.
pub fn run(args: &[String]) -> i32 {
    run_with(args, &RegistryProfiles, &PowerShellDefender::default())
}

/// [`run`] with the profile list and Defender passed in.
pub fn run_with(args: &[String], profiles: &dyn UserProfiles, defender: &dyn DefenderSettings) -> i32 {
    let Some(args) = ExclusionArgs::parse(args) else { return helper_exit::BAD_ARGUMENTS };
    let Ok(folder) = resolve_target(&args.path, &profiles.profile_dirs()) else { return helper_exit::INVALID_TARGET };
    if check_no_links(&folder, args.change == ScanExclusionChange::Add).is_err() {
        return helper_exit::INVALID_TARGET;
    }
    match defender.apply(args.change, &folder) {
        DefenderResult::Applied => helper_exit::OK,
        DefenderResult::Unavailable => helper_exit::DEFENDER_UNAVAILABLE,
        DefenderResult::Refused => helper_exit::REFUSED,
        DefenderResult::NotConfirmed => helper_exit::NOT_CONFIRMED,
        DefenderResult::Failed => helper_exit::DEFENDER_FAILED,
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    use super::*;

    struct FixedProfiles(Vec<PathBuf>);

    impl UserProfiles for FixedProfiles {
        fn profile_dirs(&self) -> Vec<PathBuf> {
            self.0.clone()
        }
    }

    /// Records what reaches Defender.
    struct FakeDefender {
        result: DefenderResult,
        calls: Mutex<Vec<(ScanExclusionChange, PathBuf)>>,
    }

    impl FakeDefender {
        fn new(result: DefenderResult) -> Self {
            Self { result, calls: Mutex::new(Vec::new()) }
        }
    }

    impl DefenderSettings for FakeDefender {
        fn apply(&self, change: ScanExclusionChange, folder: &Path) -> DefenderResult {
            self.calls.lock().unwrap().push((change, folder.to_path_buf()));
            self.result
        }
    }

    /// A profile in a temp folder with its Lampo MySQL folder.
    fn profile() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("me");
        let folder = profile.join(r"AppData\Local\Lampo\mysql");
        std::fs::create_dir_all(&folder).unwrap();
        (dir, profile, folder)
    }

    fn args(change: &str, path: &Path) -> Vec<String> {
        vec!["--defender-exclusion".into(), change.into(), "--path".into(), path.display().to_string()]
    }

    #[test]
    fn the_profile_folder_reaches_defender() {
        let (_dir, profile, folder) = profile();
        let defender = FakeDefender::new(DefenderResult::Applied);
        assert_eq!(run_with(&args("add", &folder), &FixedProfiles(vec![profile.clone()]), &defender), helper_exit::OK);
        assert_eq!(run_with(&args("remove", &folder), &FixedProfiles(vec![profile]), &defender), helper_exit::OK);
        assert_eq!(*defender.calls.lock().unwrap(), vec![(ScanExclusionChange::Add, folder.clone()), (ScanExclusionChange::Remove, folder)]);
    }

    #[test]
    fn defender_results_become_exit_codes() {
        let (_dir, profile, folder) = profile();
        for (result, code) in [
            (DefenderResult::Unavailable, helper_exit::DEFENDER_UNAVAILABLE),
            (DefenderResult::Refused, helper_exit::REFUSED),
            (DefenderResult::NotConfirmed, helper_exit::NOT_CONFIRMED),
            (DefenderResult::Failed, helper_exit::DEFENDER_FAILED),
        ] {
            assert_eq!(run_with(&args("add", &folder), &FixedProfiles(vec![profile.clone()]), &FakeDefender::new(result)), code, "{result:?}");
        }
    }

    #[test]
    fn other_folders_never_reach_defender() {
        let (dir, profile, folder) = profile();
        let defender = FakeDefender::new(DefenderResult::Applied);
        let profiles = FixedProfiles(vec![profile.clone()]);
        let sibling = folder.with_file_name("mysql2");
        std::fs::create_dir_all(&sibling).unwrap();
        let relative = PathBuf::from(r"AppData\Local\Lampo\mysql");
        let dotted = profile.join(r"AppData\Local\Lampo\..\Lampo\mysql");
        for path in [sibling, folder.parent().unwrap().to_path_buf(), dir.path().join("other").join(r"AppData\Local\Lampo\mysql"), relative, dotted] {
            assert_eq!(run_with(&args("add", &path), &profiles, &defender), helper_exit::INVALID_TARGET, "{path:?}");
        }
        let unc = PathBuf::from(r"\\server\share\me\AppData\Local\Lampo\mysql");
        assert_eq!(run_with(&args("add", &unc), &FixedProfiles(vec![PathBuf::from(r"\\server\share\me")]), &defender), helper_exit::INVALID_TARGET);
        assert_eq!(run_with(&args("add", &folder), &FixedProfiles(Vec::new()), &defender), helper_exit::INVALID_TARGET, "no registered profile");
        assert!(defender.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn adding_needs_the_folder_removing_does_not() {
        let (_dir, profile, folder) = profile();
        std::fs::remove_dir(&folder).unwrap();
        let defender = FakeDefender::new(DefenderResult::Applied);
        let profiles = FixedProfiles(vec![profile]);
        assert_eq!(run_with(&args("add", &folder), &profiles, &defender), helper_exit::INVALID_TARGET);
        assert_eq!(run_with(&args("remove", &folder), &profiles, &defender), helper_exit::OK);
    }

    #[cfg(windows)]
    #[test]
    fn a_junction_in_place_of_the_folder_is_refused() {
        let (dir, profile, folder) = profile();
        std::fs::remove_dir(&folder).unwrap();
        let elsewhere = dir.path().join("elsewhere");
        std::fs::create_dir(&elsewhere).unwrap();
        let status = std::process::Command::new("cmd").args(["/d", "/c", "mklink", "/J"]).arg(&folder).arg(&elsewhere).stdout(std::process::Stdio::null()).status().unwrap();
        assert!(status.success());
        let defender = FakeDefender::new(DefenderResult::Applied);
        assert_eq!(run_with(&args("add", &folder), &FixedProfiles(vec![profile]), &defender), helper_exit::INVALID_TARGET);
        assert!(defender.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn malformed_command_lines_are_bad_arguments() {
        let defender = FakeDefender::new(DefenderResult::Applied);
        let profiles = FixedProfiles(Vec::new());
        assert_eq!(run_with(&["--defender-exclusion".to_string(), "add".to_string()], &profiles, &defender), helper_exit::BAD_ARGUMENTS);
        assert_eq!(run_with(&[], &profiles, &defender), helper_exit::BAD_ARGUMENTS);
    }
}
