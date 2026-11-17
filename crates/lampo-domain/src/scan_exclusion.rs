//! The optional exclusion of Lampo's MySQL data folder from the antivirus scan (Microsoft
//! Defender).
//!
//! The folder holds only data files (`.ibd`, redo logs) that test suites rewrite all the time;
//! scanning each write costs time and excluding them is a low-risk trade-off. Lampo never adds
//! the exclusion by itself: only an explicit request of the user does, through a UAC prompt.
//! A standard user cannot read Defender's exclusions, so there is no reliable "is it active"
//! state: only the outcome of the last request.

/// What a request does to the exclusion.
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
