//! The optional Microsoft Defender exclusions of Lampo's MySQL data folder (a path exclusion) and
//! of the servers Lampo runs from its packages, `php-cgi.exe` and `httpd.exe` (process
//! exclusions).
//!
//! * [`target`] — the only folder that may be excluded, and the link check (both sides);
//! * [`executable`] — the only executables that may be excluded, and their check (both sides);
//! * [`request`] — the helper's command line in this mode;
//! * [`profiles`] — the user profiles of this machine (registry);
//! * [`powershell`] — the Defender cmdlets, run by the helper;
//! * [`helper`] — the logic of the helper's Defender mode (and its security rules);
//! * [`DefenderScanExclusion`] — the `ScanExclusionEditor` adapter.

pub mod executable;
pub mod helper;
pub mod powershell;
pub mod profiles;
pub mod request;
pub mod target;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use lampo_domain::ports::ScanExclusionEditor;
use lampo_domain::scan_exclusion::{ScanExclusionChange, ScanExclusionOutcome, ScanExclusionTargets};
use lampo_domain::{PortError, PortResult};

use self::profiles::UserProfiles;
use self::request::{ExclusionArgs, MAX_PROCESSES};
use self::target::TargetError;
use crate::elevation::{HelperLauncher, LaunchOutcome};

const HELPER: &str = "lampo-hosts-helper";

/// Checks every target as the helper will (so a refused target never shows a prompt), creates
/// the folder when adding (as the user, who owns the data folder), then runs
/// `lampo-hosts-helper.exe --defender-exclusion add|remove [--path <folder>] [--process <exe>]...`
/// once through the [`HelperLauncher`] (one UAC prompt in production) and maps its exit code.
pub struct DefenderScanExclusion {
    helper_exe: PathBuf,
    launcher: Arc<dyn HelperLauncher>,
    profiles: Arc<dyn UserProfiles>,
}

impl DefenderScanExclusion {
    pub fn new(helper_exe: PathBuf, launcher: Arc<dyn HelperLauncher>, profiles: Arc<dyn UserProfiles>) -> Self {
        Self { helper_exe, launcher, profiles }
    }

    /// The targets the helper will accept for `targets`, the folder created when adding.
    fn prepare(&self, targets: &ScanExclusionTargets, change: ScanExclusionChange) -> PortResult<ScanExclusionTargets> {
        if targets.is_empty() {
            return Err(PortError::invalid_data("Defender exclusion", "nothing to exclude"));
        }
        if targets.processes.len() > MAX_PROCESSES {
            return Err(PortError::invalid_data("Defender exclusion", format!("more than {MAX_PROCESSES} executables in one request")));
        }
        let profiles = self.profiles.profile_dirs();
        let adding = change == ScanExclusionChange::Add;
        let folder = targets.folder.as_deref().map(|folder| prepare_folder(folder, &profiles, adding)).transpose()?;
        let processes = targets.processes.iter().map(|exe| executable::validated_executable(requested(exe)?, &profiles, adding).map_err(|e| refused(exe, e))).collect::<PortResult<_>>()?;
        Ok(ScanExclusionTargets { folder, processes })
    }
}

fn refused(path: &Path, error: TargetError) -> PortError {
    PortError::invalid_data(path.display().to_string(), error)
}

/// `path` as the helper's command line carries it.
fn requested(path: &Path) -> PortResult<&str> {
    path.to_str().ok_or_else(|| refused(path, TargetError::Malformed))
}

/// The folder the helper will accept for `folder`, created when adding.
fn prepare_folder(folder: &Path, profiles: &[PathBuf], adding: bool) -> PortResult<PathBuf> {
    let target = target::resolve_target(requested(folder)?, profiles).map_err(|e| refused(folder, e))?;
    if adding {
        std::fs::create_dir_all(&target).map_err(|e| PortError::io(format!("creating {}", target.display()), e))?;
    }
    target::check_no_links(&target, adding).map_err(|e| refused(folder, e))?;
    Ok(target)
}

#[async_trait]
impl ScanExclusionEditor for DefenderScanExclusion {
    async fn change(&self, targets: &ScanExclusionTargets, change: ScanExclusionChange) -> PortResult<ScanExclusionOutcome> {
        let targets = self.prepare(targets, change)?;
        if !self.helper_exe.is_file() {
            return Err(PortError::not_found("elevated helper", self.helper_exe.display().to_string()));
        }
        let display = |p: &PathBuf| p.display().to_string();
        let args = ExclusionArgs { change, path: targets.folder.as_ref().map(display), processes: targets.processes.iter().map(display).collect() }.to_args();
        let (launcher, helper_exe) = (self.launcher.clone(), self.helper_exe.clone());
        let outcome = tokio::task::spawn_blocking(move || launcher.run(&helper_exe, &args)).await.map_err(|e| PortError::process(HELPER, e))??;
        match outcome {
            LaunchOutcome::Denied => Ok(ScanExclusionOutcome::Cancelled),
            LaunchOutcome::Exited(code) => helper_exit::to_outcome(code),
        }
    }
}

/// Exit codes of the helper's Defender mode (contract between the helper and
/// [`DefenderScanExclusion`]); distinct from the hosts mode's codes but for the shared ones.
pub mod helper_exit {
    use lampo_domain::scan_exclusion::ScanExclusionOutcome;
    use lampo_domain::{PortError, PortResult};

    pub use crate::hosts::helper_exit::{BAD_ARGUMENTS, OK};

    /// A target is not Lampo's MySQL folder or one of its server executables for a user profile,
    /// or a link is in its way.
    pub const INVALID_TARGET: i32 = 20;
    pub const DEFENDER_UNAVAILABLE: i32 = 21;
    pub const REFUSED: i32 = 22;
    pub const NOT_CONFIRMED: i32 = 23;
    /// PowerShell could not be run, timed out or failed outside the script's checks.
    pub const DEFENDER_FAILED: i32 = 24;

    pub fn to_outcome(code: i32) -> PortResult<ScanExclusionOutcome> {
        match code {
            OK => Ok(ScanExclusionOutcome::Applied),
            DEFENDER_UNAVAILABLE => Ok(ScanExclusionOutcome::DefenderUnavailable),
            REFUSED => Ok(ScanExclusionOutcome::Refused),
            NOT_CONFIRMED => Ok(ScanExclusionOutcome::NotConfirmed),
            INVALID_TARGET => Err(PortError::invalid_data("Defender exclusion", "the elevated helper refused a target (changed while the prompt was shown?)")),
            DEFENDER_FAILED => Err(PortError::process("powershell", "the Defender cmdlets could not be run or did not answer in time")),
            BAD_ARGUMENTS => Err(PortError::process(super::HELPER, "the helper rejected its arguments (version mismatch?)")),
            other => Err(PortError::process(super::HELPER, format!("exited with code {other}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use lampo_domain::package::{EntryRole, InstalledPackage, PackageId, PackageKind};
    use lampo_domain::version::PackageVersion;

    use super::*;

    struct FixedProfiles(Vec<PathBuf>);

    impl UserProfiles for FixedProfiles {
        fn profile_dirs(&self) -> Vec<PathBuf> {
            self.0.clone()
        }
    }

    /// Records the command lines and answers with a fixed outcome (never elevates).
    struct FakeLauncher {
        outcome: LaunchOutcome,
        calls: Mutex<Vec<Vec<String>>>,
    }

    impl HelperLauncher for FakeLauncher {
        fn run(&self, _helper: &Path, args: &[String]) -> PortResult<LaunchOutcome> {
            self.calls.lock().unwrap().push(args.to_vec());
            Ok(self.outcome)
        }
    }

    struct Setup {
        _dir: tempfile::TempDir,
        profile: PathBuf,
        folder: PathBuf,
        php: PathBuf,
        httpd: PathBuf,
        launcher: Arc<FakeLauncher>,
        adapter: DefenderScanExclusion,
    }

    impl Setup {
        fn calls(&self) -> Vec<ExclusionArgs> {
            self.launcher.calls.lock().unwrap().iter().map(|args| ExclusionArgs::parse(args).expect("a valid command line")).collect()
        }
    }

    /// A profile with PHP 8.3 and Apache 2.4 installed (no MySQL folder yet).
    fn setup(outcome: LaunchOutcome) -> Setup {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("me");
        let lampo = profile.join(r"AppData\Local\Lampo");
        let (folder, php, httpd) = (lampo.join("mysql"), lampo.join(r"packages\php-8.3\php-cgi.exe"), lampo.join(r"packages\apache-2.4\bin\httpd.exe"));
        for exe in [&php, &httpd] {
            std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
            std::fs::write(exe, b"").unwrap();
        }
        let helper = dir.path().join("lampo-hosts-helper.exe");
        std::fs::write(&helper, b"").unwrap();
        let launcher = Arc::new(FakeLauncher { outcome, calls: Mutex::new(Vec::new()) });
        let adapter = DefenderScanExclusion::new(helper, launcher.clone(), Arc::new(FixedProfiles(vec![profile.clone()])));
        Setup { _dir: dir, profile, folder, php, httpd, launcher, adapter }
    }

    fn folder_only(folder: &Path) -> ScanExclusionTargets {
        ScanExclusionTargets { folder: Some(folder.to_path_buf()), processes: Vec::new() }
    }

    #[tokio::test]
    async fn adding_creates_the_folder_and_asks_the_helper() {
        let s = setup(LaunchOutcome::Exited(helper_exit::OK));
        assert_eq!(s.adapter.change(&folder_only(&s.folder), ScanExclusionChange::Add).await, Ok(ScanExclusionOutcome::Applied));
        assert!(s.folder.is_dir(), "created as the user before the prompt");
        assert_eq!(s.calls(), vec![ExclusionArgs { change: ScanExclusionChange::Add, path: Some(s.folder.display().to_string()), processes: Vec::new() }]);
    }

    #[tokio::test]
    async fn removing_neither_needs_nor_creates_the_folder() {
        let s = setup(LaunchOutcome::Exited(helper_exit::OK));
        assert_eq!(s.adapter.change(&folder_only(&s.folder), ScanExclusionChange::Remove).await, Ok(ScanExclusionOutcome::Applied));
        assert!(!s.folder.exists());
        assert_eq!(s.calls()[0].change, ScanExclusionChange::Remove);
    }

    #[tokio::test]
    async fn every_target_goes_in_one_prompt() {
        let s = setup(LaunchOutcome::Exited(helper_exit::OK));
        let targets = ScanExclusionTargets { folder: Some(s.folder.clone()), processes: vec![s.php.clone(), s.httpd.clone()] };
        assert_eq!(s.adapter.change(&targets, ScanExclusionChange::Add).await, Ok(ScanExclusionOutcome::Applied));
        let processes = vec![s.php.display().to_string(), s.httpd.display().to_string()];
        assert_eq!(s.calls(), vec![ExclusionArgs { change: ScanExclusionChange::Add, path: Some(s.folder.display().to_string()), processes }]);
    }

    #[tokio::test]
    async fn removing_works_for_an_uninstalled_package() {
        let s = setup(LaunchOutcome::Exited(helper_exit::OK));
        std::fs::remove_dir_all(s.php.parent().unwrap()).unwrap();
        let targets = ScanExclusionTargets { folder: None, processes: vec![s.php.clone()] };
        assert!(matches!(s.adapter.change(&targets, ScanExclusionChange::Add).await, Err(PortError::InvalidData { .. })), "adding needs the file");
        assert_eq!(s.adapter.change(&targets, ScanExclusionChange::Remove).await, Ok(ScanExclusionOutcome::Applied));
        assert_eq!(s.calls().len(), 1, "only the removal prompted");
    }

    #[tokio::test]
    async fn every_outcome_the_user_can_act_on_is_not_an_error() {
        for (launch, outcome) in [
            (LaunchOutcome::Denied, ScanExclusionOutcome::Cancelled),
            (LaunchOutcome::Exited(helper_exit::DEFENDER_UNAVAILABLE), ScanExclusionOutcome::DefenderUnavailable),
            (LaunchOutcome::Exited(helper_exit::REFUSED), ScanExclusionOutcome::Refused),
            (LaunchOutcome::Exited(helper_exit::NOT_CONFIRMED), ScanExclusionOutcome::NotConfirmed),
        ] {
            let s = setup(launch);
            assert_eq!(s.adapter.change(&folder_only(&s.folder), ScanExclusionChange::Add).await, Ok(outcome), "{launch:?}");
        }
    }

    #[tokio::test]
    async fn helper_failures_are_errors() {
        for (code, expected) in [(helper_exit::INVALID_TARGET, "InvalidData"), (helper_exit::DEFENDER_FAILED, "Process"), (helper_exit::BAD_ARGUMENTS, "Process"), (99, "Process")] {
            let s = setup(LaunchOutcome::Exited(code));
            let err = s.adapter.change(&folder_only(&s.folder), ScanExclusionChange::Add).await.unwrap_err();
            assert!(format!("{err:?}").starts_with(expected), "{code}: {err:?}");
        }
    }

    #[tokio::test]
    async fn refused_folders_never_show_a_prompt() {
        let s = setup(LaunchOutcome::Exited(helper_exit::OK));
        let packages = s.folder.with_file_name("packages");
        let elsewhere = s.profile.join(r"AppData\Local\Other\mysql");
        for folder in [packages, elsewhere, s.folder.with_file_name("mysql2"), PathBuf::from(r"Lampo\mysql")] {
            let err = s.adapter.change(&folder_only(&folder), ScanExclusionChange::Add).await.unwrap_err();
            assert!(matches!(err, PortError::InvalidData { .. }), "{folder:?}: {err:?}");
        }
        assert!(!s.folder.with_file_name("mysql2").exists(), "nothing created");
        assert!(s.launcher.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn refused_executables_never_show_a_prompt() {
        let s = setup(LaunchOutcome::Exited(helper_exit::OK));
        let php_cli = s.php.with_file_name("php.exe");
        std::fs::write(&php_cli, b"").unwrap();
        let empty = ScanExclusionTargets::default();
        let mixed = ScanExclusionTargets { folder: Some(s.folder.clone()), processes: vec![s.php.clone(), php_cli.clone()] };
        let too_many = ScanExclusionTargets { folder: None, processes: vec![s.php.clone(); MAX_PROCESSES + 1] };
        for targets in [empty, mixed, too_many, ScanExclusionTargets { folder: None, processes: vec![PathBuf::from(r"C:\Windows\System32\cmd.exe")] }] {
            let err = s.adapter.change(&targets, ScanExclusionChange::Add).await.unwrap_err();
            assert!(matches!(err, PortError::InvalidData { .. }), "{targets:?}: {err:?}");
        }
        assert!(s.launcher.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_missing_helper_is_not_found() {
        let s = setup(LaunchOutcome::Exited(helper_exit::OK));
        let adapter = DefenderScanExclusion::new(s.profile.join("missing.exe"), s.launcher.clone(), Arc::new(FixedProfiles(vec![s.profile.clone()])));
        assert!(matches!(adapter.change(&folder_only(&s.folder), ScanExclusionChange::Remove).await, Err(PortError::NotFound { .. })));
    }

    #[test]
    fn the_fixed_folder_names_match_lampos_layout() {
        assert_eq!(target::LAMPO_DIR, lampo_infra_store::paths::APP_DIR_NAME);
        assert_eq!(target::MYSQL_DIR, lampo_domain::service::ServiceKind::MYSQL.as_str());
        assert!(target::PROFILE_RELATIVE_DIRS.starts_with(&target::PROFILE_DATA_DIRS), "the MySQL folder is in the data folder");
        assert_eq!(executable::PHP_KIND, PackageKind::PHP.as_str());
        assert_eq!(executable::APACHE_KIND, PackageKind::APACHE.as_str());
    }

    /// The paths the package registry gives the application are the ones the helper accepts.
    #[test]
    fn the_registrys_executables_are_accepted() {
        let profile = PathBuf::from(r"C:\Users\me");
        let paths = lampo_domain::paths::LampoPaths::new(profile.join(r"AppData\Roaming\Lampo"), profile.join(r"AppData\Local\Lampo"));
        for (kind, version, role) in
            [(PackageKind::PHP, "8.3.14", EntryRole::FastCgi), (PackageKind::PHP, "5.6.40", EntryRole::FastCgi), (PackageKind::APACHE, "2.4.62-240904", EntryRole::Server)]
        {
            let id = PackageId { kind, version: PackageVersion::parse(version).unwrap(), php: None };
            let dir = paths.packages_dir().join(lampo_infra_packages::registry::slot_key(&id));
            let package = lampo_infra_packages::layout::resolve(InstalledPackage::new(id, dir, None));
            let exe = package.entry(role).unwrap().display().to_string();
            assert_eq!(executable::resolve_executable(&exe, std::slice::from_ref(&profile)), Ok(PathBuf::from(&exe)), "{exe}");
        }
    }
}
