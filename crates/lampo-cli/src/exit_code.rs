//! **Stable, documented exit codes** of `lampo` (agents branch on them). Never renumber;
//! only append. The README table is generated from [`ExitCode::TABLE`].

use lampo_application::ErrorCode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ExitCode {
    Success = 0,
    /// Unclassified/internal failure.
    Failure = 1,
    /// Bad command line (clap's own code).
    Usage = 2,
    NotFound = 3,
    Validation = 4,
    /// Duplicate, already exists, port in use, package in use.
    Conflict = 5,
    /// No project/site for the current directory (and none given).
    ProjectNotResolved = 6,
    /// Needs a UAC prompt (hosts file, Defender exclusion); re-run with `--allow-elevation`.
    ElevationRequired = 7,
    /// The UAC prompt was refused.
    ElevationDenied = 8,
    /// The app is not running and could not be started.
    AppUnavailable = 9,
    /// A required package (PHP minor, MySQL series…) is not installed.
    PackageMissing = 10,
    /// Network or checksum failure while downloading.
    Download = 11,
    /// An external program failed (`httpd -t` rejected config, mysqld failed…).
    External = 12,
    /// Cancelled or timed out.
    Interrupted = 13,
    /// `lampo doctor` found at least one error.
    DoctorProblems = 14,
    /// CLI and app versions speak different control protocols.
    ProtocolMismatch = 15,
}

/// Exit code of a shim when it cannot run the tool itself (no project and no default PHP,
/// tool not installed). Otherwise shims return the tool's own exit code.
pub const SHIM_UNAVAILABLE: i32 = 127;

impl ExitCode {
    pub const TABLE: &'static [(ExitCode, &'static str)] = &[
        (ExitCode::Success, "success"),
        (ExitCode::Failure, "unclassified failure"),
        (ExitCode::Usage, "invalid command line"),
        (ExitCode::NotFound, "project, site, package, service or file not found"),
        (ExitCode::Validation, "invalid input"),
        (ExitCode::Conflict, "conflict (duplicate, port in use, in use)"),
        (ExitCode::ProjectNotResolved, "no project for the current directory"),
        (ExitCode::ElevationRequired, "needs a UAC prompt (hosts file, Defender exclusion): re-run with --allow-elevation"),
        (ExitCode::ElevationDenied, "UAC prompt refused"),
        (ExitCode::AppUnavailable, "Lampo app not running and could not be started"),
        (ExitCode::PackageMissing, "required package not installed"),
        (ExitCode::Download, "network or checksum failure"),
        (ExitCode::External, "external program failed"),
        (ExitCode::Interrupted, "cancelled or timed out"),
        (ExitCode::DoctorProblems, "doctor found problems"),
        (ExitCode::ProtocolMismatch, "CLI/app version mismatch"),
    ];

    pub fn code(self) -> i32 {
        self as u8 as i32
    }
}

impl From<ErrorCode> for ExitCode {
    fn from(code: ErrorCode) -> Self {
        match code {
            ErrorCode::Validation => Self::Validation,
            ErrorCode::NotFound => Self::NotFound,
            ErrorCode::Conflict | ErrorCode::PortInUse => Self::Conflict,
            ErrorCode::ProjectNotResolved => Self::ProjectNotResolved,
            ErrorCode::ElevationRequired => Self::ElevationRequired,
            ErrorCode::ElevationDenied => Self::ElevationDenied,
            ErrorCode::AppUnavailable => Self::AppUnavailable,
            ErrorCode::PackageMissing => Self::PackageMissing,
            ErrorCode::Network | ErrorCode::ChecksumMismatch => Self::Download,
            ErrorCode::ConfigRejected | ErrorCode::ProcessFailed => Self::External,
            ErrorCode::Cancelled | ErrorCode::Timeout => Self::Interrupted,
            ErrorCode::ProtocolMismatch => Self::ProtocolMismatch,
            ErrorCode::Io | ErrorCode::Unsupported | ErrorCode::Internal => Self::Failure,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_is_complete_and_ordered() {
        for (i, (code, _)) in ExitCode::TABLE.iter().enumerate() {
            assert_eq!(code.code(), i as i32);
        }
    }
}
