//! Defender mode of `lampo-hosts-helper.exe`:
//! `--defender-exclusion add|remove [--path <folder>] [--process <exe>]...` ([`super::request`]).
//!
//! Security design — the helper runs as an administrator, so it must not become a way to exclude
//! any file or program from the antivirus scan:
//! * **One folder.** A path must be `<profile>\AppData\Local\Lampo\mysql` for a user profile
//!   registered on this machine (`HKLM\...\ProfileList`, [`super::profiles`]); the folder handed
//!   to Defender is rebuilt from that profile's own path ([`super::target`]). Another
//!   administrator approving the prompt (whose own `%LOCALAPPDATA%` differs) still excludes the
//!   requesting user's folder, which the request names.
//! * **Lampo's servers only.** A process must be `php-cgi.exe` of a PHP package or
//!   `bin\httpd.exe` of an Apache package under that profile's `AppData\Local\Lampo\packages`,
//!   rebuilt the same way ([`super::executable`]); one exclusion per executable, never a
//!   wildcard or a bare name.
//! * **Real files only.** No component may be a junction, link or mount point, and adding needs
//!   the folder and the executables to exist (removing does not: a package may be gone).
//! * **All or nothing.** One refused target refuses the whole request before Defender is asked.
//! * **No interpolation.** Defender is reached through a constant PowerShell script, the targets
//!   through environment variables ([`super::powershell`]).
//! * **Path and process exclusions only**: never an extension.

use std::path::PathBuf;

use lampo_domain::scan_exclusion::{ScanExclusionChange, ScanExclusionTargets};

use super::executable::validated_executable;
use super::helper_exit;
use super::powershell::{DefenderResult, DefenderSettings, PowerShellDefender};
use super::profiles::{RegistryProfiles, UserProfiles};
use super::request::ExclusionArgs;
use super::target::{TargetError, check_no_links, resolve_target};

/// Entire Defender run; returns the process exit code.
pub fn run(args: &[String]) -> i32 {
    run_with(args, &RegistryProfiles, &PowerShellDefender::default())
}

/// [`run`] with the profile list and Defender passed in.
pub fn run_with(args: &[String], profiles: &dyn UserProfiles, defender: &dyn DefenderSettings) -> i32 {
    let Some(args) = ExclusionArgs::parse(args) else { return helper_exit::BAD_ARGUMENTS };
    let Ok(targets) = validated_targets(&args, &profiles.profile_dirs()) else { return helper_exit::INVALID_TARGET };
    match defender.apply(args.change, &targets) {
        DefenderResult::Applied => helper_exit::OK,
        DefenderResult::Unavailable => helper_exit::DEFENDER_UNAVAILABLE,
        DefenderResult::Refused => helper_exit::REFUSED,
        DefenderResult::NotConfirmed => helper_exit::NOT_CONFIRMED,
        DefenderResult::Failed => helper_exit::DEFENDER_FAILED,
    }
}

/// Every target of the request, rebuilt and checked; the first refusal refuses them all.
fn validated_targets(args: &ExclusionArgs, profiles: &[PathBuf]) -> Result<ScanExclusionTargets, TargetError> {
    let adding = args.change == ScanExclusionChange::Add;
    let folder = args
        .path
        .as_deref()
        .map(|path| {
            let folder = resolve_target(path, profiles)?;
            check_no_links(&folder, adding)?;
            Ok(folder)
        })
        .transpose()?;
    let processes = args.processes.iter().map(|exe| validated_executable(exe, profiles, adding)).collect::<Result<_, _>>()?;
    Ok(ScanExclusionTargets { folder, processes })
}

#[cfg(test)]
mod tests {
    use std::path::Path;
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
        calls: Mutex<Vec<(ScanExclusionChange, ScanExclusionTargets)>>,
    }

    impl FakeDefender {
        fn new(result: DefenderResult) -> Self {
            Self { result, calls: Mutex::new(Vec::new()) }
        }

        fn calls(&self) -> Vec<(ScanExclusionChange, ScanExclusionTargets)> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl DefenderSettings for FakeDefender {
        fn apply(&self, change: ScanExclusionChange, targets: &ScanExclusionTargets) -> DefenderResult {
            self.calls.lock().unwrap().push((change, targets.clone()));
            self.result
        }
    }

    /// A profile in a temp folder with its Lampo MySQL folder, PHP 8.3 and Apache 2.4.
    struct Profile {
        dir: tempfile::TempDir,
        path: PathBuf,
        folder: PathBuf,
        php: PathBuf,
        httpd: PathBuf,
    }

    impl Profile {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("me");
            let lampo = path.join(r"AppData\Local\Lampo");
            let (folder, php, httpd) = (lampo.join("mysql"), lampo.join(r"packages\php-8.3\php-cgi.exe"), lampo.join(r"packages\apache-2.4\bin\httpd.exe"));
            std::fs::create_dir_all(&folder).unwrap();
            for exe in [&php, &httpd] {
                std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
                std::fs::write(exe, b"").unwrap();
            }
            Self { dir, path, folder, php, httpd }
        }

        fn profiles(&self) -> FixedProfiles {
            FixedProfiles(vec![self.path.clone()])
        }
    }

    fn args(change: &str, path: Option<&Path>, processes: &[&Path]) -> Vec<String> {
        let mut args = vec!["--defender-exclusion".to_string(), change.to_string()];
        if let Some(path) = path {
            args.extend(["--path".to_string(), path.display().to_string()]);
        }
        for process in processes {
            args.extend(["--process".to_string(), process.display().to_string()]);
        }
        args
    }

    fn targets(folder: Option<&Path>, processes: &[&Path]) -> ScanExclusionTargets {
        ScanExclusionTargets { folder: folder.map(Path::to_path_buf), processes: processes.iter().map(|p| p.to_path_buf()).collect() }
    }

    #[test]
    fn the_profile_folder_reaches_defender() {
        let p = Profile::new();
        let defender = FakeDefender::new(DefenderResult::Applied);
        assert_eq!(run_with(&args("add", Some(&p.folder), &[]), &p.profiles(), &defender), helper_exit::OK);
        assert_eq!(run_with(&args("remove", Some(&p.folder), &[]), &p.profiles(), &defender), helper_exit::OK);
        let expected = targets(Some(&p.folder), &[]);
        assert_eq!(defender.calls(), vec![(ScanExclusionChange::Add, expected.clone()), (ScanExclusionChange::Remove, expected)]);
    }

    #[test]
    fn every_executable_reaches_defender_in_one_request() {
        let p = Profile::new();
        let defender = FakeDefender::new(DefenderResult::Applied);
        assert_eq!(run_with(&args("add", None, &[&p.php, &p.httpd]), &p.profiles(), &defender), helper_exit::OK);
        assert_eq!(run_with(&args("add", Some(&p.folder), &[&p.php]), &p.profiles(), &defender), helper_exit::OK);
        assert_eq!(defender.calls(), vec![(ScanExclusionChange::Add, targets(None, &[&p.php, &p.httpd])), (ScanExclusionChange::Add, targets(Some(&p.folder), &[&p.php]))]);
    }

    #[test]
    fn defender_results_become_exit_codes() {
        let p = Profile::new();
        for (result, code) in [
            (DefenderResult::Unavailable, helper_exit::DEFENDER_UNAVAILABLE),
            (DefenderResult::Refused, helper_exit::REFUSED),
            (DefenderResult::NotConfirmed, helper_exit::NOT_CONFIRMED),
            (DefenderResult::Failed, helper_exit::DEFENDER_FAILED),
        ] {
            assert_eq!(run_with(&args("add", None, &[&p.php]), &p.profiles(), &FakeDefender::new(result)), code, "{result:?}");
        }
    }

    #[test]
    fn other_folders_never_reach_defender() {
        let p = Profile::new();
        let defender = FakeDefender::new(DefenderResult::Applied);
        let sibling = p.folder.with_file_name("mysql2");
        std::fs::create_dir_all(&sibling).unwrap();
        let relative = PathBuf::from(r"AppData\Local\Lampo\mysql");
        let dotted = p.path.join(r"AppData\Local\Lampo\..\Lampo\mysql");
        for path in [sibling, p.folder.parent().unwrap().to_path_buf(), p.dir.path().join("other").join(r"AppData\Local\Lampo\mysql"), relative, dotted] {
            assert_eq!(run_with(&args("add", Some(&path), &[]), &p.profiles(), &defender), helper_exit::INVALID_TARGET, "{path:?}");
        }
        let unc = PathBuf::from(r"\\server\share\me\AppData\Local\Lampo\mysql");
        assert_eq!(run_with(&args("add", Some(&unc), &[]), &FixedProfiles(vec![PathBuf::from(r"\\server\share\me")]), &defender), helper_exit::INVALID_TARGET);
        assert_eq!(run_with(&args("add", Some(&p.folder), &[]), &FixedProfiles(Vec::new()), &defender), helper_exit::INVALID_TARGET, "no registered profile");
        assert!(defender.calls().is_empty());
    }

    #[test]
    fn other_executables_never_reach_defender() {
        let p = Profile::new();
        let defender = FakeDefender::new(DefenderResult::Applied);
        let packages = p.php.parent().unwrap().parent().unwrap().to_path_buf();
        let mysqld = packages.join(r"mysql-8.4\bin\mysqld.exe");
        let php_cli = packages.join(r"php-8.3\php.exe");
        let outside = p.dir.path().join(r"elsewhere\php-cgi.exe");
        for exe in [&mysqld, &php_cli, &outside] {
            std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
            std::fs::write(exe, b"").unwrap();
        }
        for exe in [
            mysqld,
            php_cli,
            outside,
            PathBuf::from("php-cgi.exe"),
            packages.join(r"php-*\php-cgi.exe"),
            packages.join(r"..\packages\php-8.3\php-cgi.exe"),
            PathBuf::from(format!(r"\\?\{}", p.php.display())),
            PathBuf::from(r"\\server\share\me\AppData\Local\Lampo\packages\php-8.3\php-cgi.exe"),
        ] {
            assert_eq!(run_with(&args("add", None, &[&exe]), &p.profiles(), &defender), helper_exit::INVALID_TARGET, "{exe:?}");
            assert_eq!(run_with(&args("remove", None, &[&exe]), &p.profiles(), &defender), helper_exit::INVALID_TARGET, "{exe:?}");
        }
        assert_eq!(run_with(&args("add", None, &[&p.php]), &FixedProfiles(Vec::new()), &defender), helper_exit::INVALID_TARGET, "no registered profile");
        assert!(defender.calls().is_empty());
    }

    #[test]
    fn one_refused_target_refuses_the_whole_request() {
        let p = Profile::new();
        let defender = FakeDefender::new(DefenderResult::Applied);
        let windows = PathBuf::from(r"C:\Windows\System32\cmd.exe");
        assert_eq!(run_with(&args("add", Some(&p.folder), &[&p.php, &windows]), &p.profiles(), &defender), helper_exit::INVALID_TARGET);
        assert!(defender.calls().is_empty());
    }

    #[test]
    fn adding_needs_the_files_removing_does_not() {
        let p = Profile::new();
        std::fs::remove_dir(&p.folder).unwrap();
        std::fs::remove_dir_all(p.php.parent().unwrap()).unwrap();
        let defender = FakeDefender::new(DefenderResult::Applied);
        assert_eq!(run_with(&args("add", Some(&p.folder), &[]), &p.profiles(), &defender), helper_exit::INVALID_TARGET);
        assert_eq!(run_with(&args("add", None, &[&p.php]), &p.profiles(), &defender), helper_exit::INVALID_TARGET, "an uninstalled PHP");
        assert_eq!(run_with(&args("remove", Some(&p.folder), &[&p.php]), &p.profiles(), &defender), helper_exit::OK);
    }

    /// `mklink /J` needs no privilege, unlike a symbolic link.
    #[cfg(windows)]
    fn junction(link: &Path, target: &Path) {
        let status = std::process::Command::new("cmd").args(["/d", "/c", "mklink", "/J"]).arg(link).arg(target).stdout(std::process::Stdio::null()).status().unwrap();
        assert!(status.success(), "mklink /J failed");
    }

    #[cfg(windows)]
    #[test]
    fn a_junction_in_place_of_a_folder_is_refused() {
        let p = Profile::new();
        let elsewhere = p.dir.path().join("elsewhere");
        std::fs::create_dir(&elsewhere).unwrap();
        std::fs::write(elsewhere.join("php-cgi.exe"), b"").unwrap();
        std::fs::remove_dir(&p.folder).unwrap();
        junction(&p.folder, &elsewhere);
        std::fs::remove_dir_all(p.php.parent().unwrap()).unwrap();
        junction(p.php.parent().unwrap(), &elsewhere);
        let defender = FakeDefender::new(DefenderResult::Applied);
        assert_eq!(run_with(&args("add", Some(&p.folder), &[]), &p.profiles(), &defender), helper_exit::INVALID_TARGET);
        for change in ["add", "remove"] {
            assert_eq!(run_with(&args(change, None, &[&p.php]), &p.profiles(), &defender), helper_exit::INVALID_TARGET, "{change}");
        }
        assert!(defender.calls().is_empty());
    }

    #[test]
    fn malformed_command_lines_are_bad_arguments() {
        let defender = FakeDefender::new(DefenderResult::Applied);
        let profiles = FixedProfiles(Vec::new());
        assert_eq!(run_with(&["--defender-exclusion".to_string(), "add".to_string()], &profiles, &defender), helper_exit::BAD_ARGUMENTS);
        assert_eq!(run_with(&[], &profiles, &defender), helper_exit::BAD_ARGUMENTS);
    }
}
