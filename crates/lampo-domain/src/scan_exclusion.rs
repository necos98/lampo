//! The optional exclusions of Lampo's files from the antivirus scan (Microsoft Defender):
//!
//! * the **MySQL data folder**: it holds only data files (`.ibd`, redo logs) that test suites
//!   rewrite all the time, and scanning each write costs time;
//! * the **server processes** (`php-cgi.exe` of every installed PHP, `httpd.exe` of Apache): a
//!   process exclusion skips the files those processes open, so the first request to a site no
//!   longer waits for Defender to scan hundreds of PHP sources (after every definition update,
//!   reboot or change of the files), wherever the projects are, and no source folder is
//!   excluded.
//!
//! Lampo never adds an exclusion by itself: only an explicit request of the user does, through a
//! UAC prompt (one per request, whatever it names). A standard user cannot read Defender's
//! exclusions, so there is no reliable "is it active" state: only the outcome of the last request.

use std::path::PathBuf;

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
}

impl ScanExclusionTargets {
    pub fn is_empty(&self) -> bool {
        self.folder.is_none() && self.processes.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_are_empty_only_without_folder_and_processes() {
        assert!(ScanExclusionTargets::default().is_empty());
        assert!(!ScanExclusionTargets { folder: Some(PathBuf::from(r"C:\x")), processes: Vec::new() }.is_empty());
        assert!(!ScanExclusionTargets { folder: None, processes: vec![PathBuf::from(r"C:\x\php-cgi.exe")] }.is_empty());
    }
}
