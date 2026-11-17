//! The optional Microsoft Defender exclusion of Lampo's MySQL data folder.
//!
//! * [`target`] — the only folder that may be excluded, and the link check (both sides);
//! * [`request`] — the helper's command line in this mode;
//! * [`profiles`] — the user profiles of this machine (registry);
//! * [`powershell`] — the Defender cmdlets, run by the helper;
//! * [`helper`] — the logic of the helper's Defender mode (and its security rules);
//! * [`DefenderScanExclusion`] — the `ScanExclusionEditor` adapter.

pub mod helper;
pub mod powershell;
pub mod profiles;
pub mod request;
pub mod target;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use lampo_domain::ports::ScanExclusionEditor;
use lampo_domain::scan_exclusion::{ScanExclusionChange, ScanExclusionOutcome};
use lampo_domain::{PortError, PortResult};

use self::profiles::UserProfiles;
use self::request::ExclusionArgs;
use crate::elevation::{HelperLauncher, LaunchOutcome};

const HELPER: &str = "lampo-hosts-helper";

/// Checks the folder as the helper will (so a refused folder never shows a prompt), creates it
/// when adding (as the user, who owns the data folder), then runs
/// `lampo-hosts-helper.exe --defender-exclusion add|remove --path <folder>` through the
/// [`HelperLauncher`] (UAC prompt in production) and maps its exit code.
pub struct DefenderScanExclusion {
    helper_exe: PathBuf,
    launcher: Arc<dyn HelperLauncher>,
    profiles: Arc<dyn UserProfiles>,
}

impl DefenderScanExclusion {
    pub fn new(helper_exe: PathBuf, launcher: Arc<dyn HelperLauncher>, profiles: Arc<dyn UserProfiles>) -> Self {
        Self { helper_exe, launcher, profiles }
    }

    /// The folder the helper will accept for `folder`, created when adding.
    fn prepare(&self, folder: &Path, change: ScanExclusionChange) -> PortResult<PathBuf> {
        let refused = |e: target::TargetError| PortError::invalid_data(folder.display().to_string(), e);
        let requested = folder.to_str().ok_or_else(|| refused(target::TargetError::Malformed))?;
        let target = target::resolve_target(requested, &self.profiles.profile_dirs()).map_err(refused)?;
        let adding = change == ScanExclusionChange::Add;
        if adding {
            std::fs::create_dir_all(&target).map_err(|e| PortError::io(format!("creating {}", target.display()), e))?;
        }
        target::check_no_links(&target, adding).map_err(refused)?;
        Ok(target)
    }
}

#[async_trait]
impl ScanExclusionEditor for DefenderScanExclusion {
    async fn change(&self, folder: &Path, change: ScanExclusionChange) -> PortResult<ScanExclusionOutcome> {
        let target = self.prepare(folder, change)?;
        if !self.helper_exe.is_file() {
            return Err(PortError::not_found("elevated helper", self.helper_exe.display().to_string()));
        }
        let args = ExclusionArgs { change, path: target.display().to_string() }.to_args();
        let (launcher, helper_exe) = (self.launcher.clone(), self.helper_exe.clone());
        let outcome = tokio::task::spawn_blocking(move || launcher.run(&helper_exe, &args)).await.map_err(|e| PortError::process(HELPER, e))??;
        match outcome {
            LaunchOutcome::Denied => Ok(ScanExclusionOutcome::Cancelled),
            LaunchOutcome::Exited(code) => helper_exit::to_outcome(code, &target),
        }
    }
}

/// Exit codes of the helper's Defender mode (contract between the helper and
/// [`DefenderScanExclusion`]); distinct from the hosts mode's codes but for the shared ones.
pub mod helper_exit {
    use std::path::Path;

    use lampo_domain::scan_exclusion::ScanExclusionOutcome;
    use lampo_domain::{PortError, PortResult};

    pub use crate::hosts::helper_exit::{BAD_ARGUMENTS, OK};

    /// The folder is not the Lampo MySQL folder of a user profile, or a link is in its way.
    pub const INVALID_TARGET: i32 = 20;
    pub const DEFENDER_UNAVAILABLE: i32 = 21;
    pub const REFUSED: i32 = 22;
    pub const NOT_CONFIRMED: i32 = 23;
    /// PowerShell could not be run, timed out or failed outside the script's checks.
    pub const DEFENDER_FAILED: i32 = 24;

    pub fn to_outcome(code: i32, folder: &Path) -> PortResult<ScanExclusionOutcome> {
        match code {
            OK => Ok(ScanExclusionOutcome::Applied),
            DEFENDER_UNAVAILABLE => Ok(ScanExclusionOutcome::DefenderUnavailable),
            REFUSED => Ok(ScanExclusionOutcome::Refused),
            NOT_CONFIRMED => Ok(ScanExclusionOutcome::NotConfirmed),
            INVALID_TARGET => Err(PortError::invalid_data(folder.display().to_string(), "the elevated helper refused the folder (changed while the prompt was shown?)")),
            DEFENDER_FAILED => Err(PortError::process("powershell", "the Defender cmdlets could not be run or did not answer in time")),
            BAD_ARGUMENTS => Err(PortError::process(super::HELPER, "the helper rejected its arguments (version mismatch?)")),
            other => Err(PortError::process(super::HELPER, format!("exited with code {other}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

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
        launcher: Arc<FakeLauncher>,
        adapter: DefenderScanExclusion,
    }

    fn setup(outcome: LaunchOutcome) -> Setup {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("me");
        let folder = profile.join(r"AppData\Local\Lampo\mysql");
        let helper = dir.path().join("lampo-hosts-helper.exe");
        std::fs::write(&helper, b"").unwrap();
        let launcher = Arc::new(FakeLauncher { outcome, calls: Mutex::new(Vec::new()) });
        let adapter = DefenderScanExclusion::new(helper, launcher.clone(), Arc::new(FixedProfiles(vec![profile.clone()])));
        Setup { _dir: dir, profile, folder, launcher, adapter }
    }

    #[tokio::test]
    async fn adding_creates_the_folder_and_asks_the_helper() {
        let s = setup(LaunchOutcome::Exited(helper_exit::OK));
        assert_eq!(s.adapter.change(&s.folder, ScanExclusionChange::Add).await, Ok(ScanExclusionOutcome::Applied));
        assert!(s.folder.is_dir(), "created as the user before the prompt");
        let calls = s.launcher.calls.lock().unwrap();
        assert_eq!(ExclusionArgs::parse(&calls[0]), Some(ExclusionArgs { change: ScanExclusionChange::Add, path: s.folder.display().to_string() }));
    }

    #[tokio::test]
    async fn removing_neither_needs_nor_creates_the_folder() {
        let s = setup(LaunchOutcome::Exited(helper_exit::OK));
        assert_eq!(s.adapter.change(&s.folder, ScanExclusionChange::Remove).await, Ok(ScanExclusionOutcome::Applied));
        assert!(!s.folder.exists());
        assert_eq!(ExclusionArgs::parse(&s.launcher.calls.lock().unwrap()[0]).unwrap().change, ScanExclusionChange::Remove);
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
            assert_eq!(s.adapter.change(&s.folder, ScanExclusionChange::Add).await, Ok(outcome), "{launch:?}");
        }
    }

    #[tokio::test]
    async fn helper_failures_are_errors() {
        for (code, expected) in [(helper_exit::INVALID_TARGET, "InvalidData"), (helper_exit::DEFENDER_FAILED, "Process"), (helper_exit::BAD_ARGUMENTS, "Process"), (99, "Process")] {
            let s = setup(LaunchOutcome::Exited(code));
            let err = s.adapter.change(&s.folder, ScanExclusionChange::Add).await.unwrap_err();
            assert!(format!("{err:?}").starts_with(expected), "{code}: {err:?}");
        }
    }

    #[tokio::test]
    async fn refused_folders_never_show_a_prompt() {
        let s = setup(LaunchOutcome::Exited(helper_exit::OK));
        let packages = s.folder.with_file_name("packages");
        let elsewhere = s.profile.join(r"AppData\Local\Other\mysql");
        for folder in [packages, elsewhere, s.folder.with_file_name("mysql2"), PathBuf::from(r"Lampo\mysql")] {
            let err = s.adapter.change(&folder, ScanExclusionChange::Add).await.unwrap_err();
            assert!(matches!(err, PortError::InvalidData { .. }), "{folder:?}: {err:?}");
            assert!(!folder.exists() || folder.is_relative(), "nothing created for {folder:?}");
        }
        assert!(s.launcher.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_missing_helper_is_not_found() {
        let s = setup(LaunchOutcome::Exited(helper_exit::OK));
        let adapter = DefenderScanExclusion::new(s.profile.join("missing.exe"), s.launcher.clone(), Arc::new(FixedProfiles(vec![s.profile.clone()])));
        assert!(matches!(adapter.change(&s.folder, ScanExclusionChange::Remove).await, Err(PortError::NotFound { .. })));
    }

    #[test]
    fn the_fixed_folder_names_match_lampos_layout() {
        assert_eq!(target::LAMPO_DIR, lampo_infra_store::paths::APP_DIR_NAME);
        assert_eq!(target::MYSQL_DIR, lampo_domain::service::ServiceKind::MYSQL.as_str());
    }
}
