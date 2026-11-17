//! The optional exclusions of Lampo's files from the antivirus scan (Microsoft Defender):
//!
//! * the **MySQL data folder**: it holds only data files (`.ibd`, redo logs) that test suites
//!   rewrite all the time, and scanning each write costs time;
//! * the **server processes** (`php-cgi.exe` of every installed PHP, `httpd.exe` of Apache): a
//!   process exclusion skips the files those processes open, so the first request to a site no
//!   longer waits for Defender to scan hundreds of PHP sources (after every definition update,
//!   reboot or change of the files), wherever the projects are, and no source folder is
//!   excluded;
//! * a **projects folder** chosen by the user (for example `%USERPROFILE%\Desktop\Progetti`): the
//!   heaviest trade-off, since downloaded code (`vendor\`, `node_modules\`) ends up there, but
//!   the only one that also speeds up `composer install`, `git checkout`, `npm install` and the
//!   IDE. Never one of the user's main folders themselves, nor a system folder.
//!
//! Lampo never adds an exclusion by itself: only an explicit request of the user does, through a
//! UAC prompt (one per request, whatever it names). A standard user cannot read Defender's
//! exclusions, so there is no reliable "is it active" state: only the outcome of the requests,
//! which Lampo remembers for the server processes ([`ProcessExclusionRecord`]). A standard user
//! can read whether Defender scans on access at all ([`DefenderState`]).

use std::path::{Path, PathBuf};

/// What a request does to the exclusions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScanExclusionChange {
    Add,
    Remove,
}

/// How a request ended. None of these is a failure of Lampo: they are what the user is told.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScanExclusionOutcome {
    /// Defender confirmed the change.
    Applied,
    /// The user declined the UAC prompt: nothing changed.
    Cancelled,
    /// Defender is not installed or not the active antivirus (another product, or turned off).
    DefenderUnavailable,
    /// Defender refused the change (Tamper Protection, a group policy or Intune).
    Refused,
    /// Defender accepted the request but the change cannot be seen afterwards (exclusions
    /// managed or hidden by a policy).
    NotConfirmed,
}

/// Everything one request names, changed together (one UAC prompt).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanExclusionTargets {
    /// Lampo's MySQL data folder (a path exclusion).
    pub folder: Option<PathBuf>,
    /// Lampo's server executables (one process exclusion each, never a wildcard).
    pub processes: Vec<PathBuf>,
    /// A folder holding the user's projects (a path exclusion).
    pub projects: Option<PathBuf>,
}

impl ScanExclusionTargets {
    pub fn is_empty(&self) -> bool {
        self.folder.is_none() && self.processes.is_empty() && self.projects.is_none()
    }
}

/// What Microsoft Defender does, as a standard user can read it (`Get-MpComputerStatus`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DefenderState {
    /// The active antivirus with real-time protection on: it scans the files processes open.
    RealTimeProtection,
    /// The active antivirus with real-time protection off: it scans nothing on access.
    RealTimeProtectionOff,
    /// Running next to another antivirus (passive or EDR block mode): it scans nothing on access.
    Passive,
    /// Not installed, or turned off.
    Unavailable,
}

/// One request naming the server processes, and how it ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessExclusionRequest {
    pub change: ScanExclusionChange,
    pub outcome: ScanExclusionOutcome,
    /// The executables it named.
    pub processes: Vec<PathBuf>,
}

/// What Lampo remembers of its requests naming the server processes: the last one, and the
/// executables Defender confirmed excluded (added and not removed by Lampo since). Defender's own
/// list cannot be read without elevation, so this is the best a standard user can know.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProcessExclusionRecord {
    /// None: never asked.
    pub last_request: Option<ProcessExclusionRequest>,
    pub excluded: Vec<PathBuf>,
}

impl ProcessExclusionRecord {
    /// Records `request`; only an `Applied` one changes [`Self::excluded`] (an add keeps what
    /// earlier adds excluded, as Defender does).
    pub fn record(&mut self, request: ProcessExclusionRequest) {
        if request.outcome == ScanExclusionOutcome::Applied {
            match request.change {
                ScanExclusionChange::Add => {
                    for exe in &request.processes {
                        if !self.is_excluded(exe) {
                            self.excluded.push(exe.clone());
                        }
                    }
                }
                ScanExclusionChange::Remove => self.excluded.retain(|exe| !request.processes.iter().any(|removed| same_path(exe, removed))),
            }
        }
        self.last_request = Some(request);
    }

    /// Whether Defender confirmed `executable` excluded (Windows paths: ignoring case).
    pub fn is_excluded(&self, executable: &Path) -> bool {
        self.excluded.iter().any(|exe| same_path(exe, executable))
    }
}

fn same_path(a: &Path, b: &Path) -> bool {
    a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PHP_83: &str = r"C:\lampo\packages\php-8.3\php-cgi.exe";
    const PHP_74: &str = r"C:\lampo\packages\php-7.4\php-cgi.exe";

    fn request(change: ScanExclusionChange, outcome: ScanExclusionOutcome, processes: &[&str]) -> ProcessExclusionRequest {
        ProcessExclusionRequest { change, outcome, processes: processes.iter().map(PathBuf::from).collect() }
    }

    #[test]
    fn only_applied_requests_change_what_is_excluded() {
        let mut record = ProcessExclusionRecord::default();
        record.record(request(ScanExclusionChange::Add, ScanExclusionOutcome::Cancelled, &[PHP_83]));
        assert!(!record.is_excluded(Path::new(PHP_83)));
        assert_eq!(record.last_request.as_ref().map(|r| r.outcome), Some(ScanExclusionOutcome::Cancelled));

        record.record(request(ScanExclusionChange::Add, ScanExclusionOutcome::Applied, &[PHP_83]));
        record.record(request(ScanExclusionChange::Add, ScanExclusionOutcome::Refused, &[PHP_74]));
        assert!(record.is_excluded(Path::new(PHP_83)) && !record.is_excluded(Path::new(PHP_74)));
        assert_eq!(record.last_request.as_ref().map(|r| r.outcome), Some(ScanExclusionOutcome::Refused));
    }

    #[test]
    fn adds_accumulate_and_removals_take_away() {
        let mut record = ProcessExclusionRecord::default();
        record.record(request(ScanExclusionChange::Add, ScanExclusionOutcome::Applied, &[PHP_83]));
        record.record(request(ScanExclusionChange::Add, ScanExclusionOutcome::Applied, &[PHP_83, PHP_74]));
        assert_eq!(record.excluded, [PathBuf::from(PHP_83), PathBuf::from(PHP_74)], "no duplicate");
        record.record(request(ScanExclusionChange::Remove, ScanExclusionOutcome::Applied, &[&PHP_83.to_uppercase()]));
        assert_eq!(record.excluded, [PathBuf::from(PHP_74)], "ignoring case");
        record.record(request(ScanExclusionChange::Remove, ScanExclusionOutcome::NotConfirmed, &[PHP_74]));
        assert!(record.is_excluded(Path::new(PHP_74)), "not confirmed: still believed excluded");
    }

    #[test]
    fn targets_are_empty_only_without_any_target() {
        assert!(ScanExclusionTargets::default().is_empty());
        assert!(!ScanExclusionTargets { folder: Some(PathBuf::from(r"C:\x")), ..Default::default() }.is_empty());
        assert!(!ScanExclusionTargets { processes: vec![PathBuf::from(r"C:\x\php-cgi.exe")], ..Default::default() }.is_empty());
        assert!(!ScanExclusionTargets { projects: Some(PathBuf::from(r"C:\Users\me\Desktop\Progetti")), ..Default::default() }.is_empty());
    }
}
