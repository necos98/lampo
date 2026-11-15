//! The single error type returned by every use case and sent to front-ends.
//!
//! `code` is stable (GUI messages and CLI exit codes are keyed by it); `message` is English,
//! developer-facing; `details` carries structured extras (e.g. `httpd -t` output).

use lampo_domain::{DomainError, PortError};
use serde::{Deserialize, Serialize};

/// Stable error categories. Adding a variant is a contract change: update the TS mirror
/// (`src/lib/api/types.ts`), the Italian messages and the CLI exit-code table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ErrorCode {
    /// Invalid input (bad version, hostname, path…).
    Validation,
    NotFound,
    /// Duplicate id/folder/hostname/port, package in use, …
    Conflict,
    /// No project/site could be resolved for the current directory.
    ProjectNotResolved,
    /// The operation needs the hosts-file helper (UAC) but the caller did not allow elevation.
    ElevationRequired,
    /// The user refused the UAC prompt.
    ElevationDenied,
    /// The desktop app is not running and could not be started / reached.
    AppUnavailable,
    /// A required package (PHP minor, MySQL series, Apache…) is not installed.
    PackageMissing,
    Network,
    ChecksumMismatch,
    /// `httpd -t` rejected the generated configuration (`details.output`).
    ConfigRejected,
    PortInUse,
    /// An external program failed.
    ProcessFailed,
    Io,
    Cancelled,
    Timeout,
    Unsupported,
    /// CLI and app speak different control-protocol versions.
    ProtocolMismatch,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "camelCase")]
#[error("{code:?}: {message}")]
pub struct AppError {
    pub code: ErrorCode,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

pub type AppResult<T> = Result<T, AppError>;

impl AppError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self { code, message: message.into(), details: None }
    }

    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
    }

    pub fn not_found(entity: &str, id: impl std::fmt::Display) -> Self {
        Self::new(ErrorCode::NotFound, format!("{entity} `{id}` not found"))
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Internal, message)
    }
}

impl From<DomainError> for AppError {
    fn from(e: DomainError) -> Self {
        let code = match &e {
            DomainError::DuplicatePort { .. }
            | DomainError::DuplicateProjectId(_)
            | DomainError::DuplicateProjectRoot { .. }
            | DomainError::DuplicateHostname { .. }
            | DomainError::DuplicateLoopbackPort { .. }
            | DomainError::LoopbackPortTaken { .. }
            | DomainError::NoFreeLoopbackPort { .. }
            | DomainError::BuiltinPresetIsReadOnly(_) => ErrorCode::Conflict,
            DomainError::ToolNotAvailable { .. } => ErrorCode::PackageMissing,
            DomainError::ChecksumConflict { .. } => ErrorCode::ChecksumMismatch,
            _ => ErrorCode::Validation,
        };
        let error = Self::new(code, e.to_string());
        match e {
            // The GUI shows the reason next to the directives editor.
            DomainError::InvalidApacheDirectives(reason) => error.with_details(serde_json::json!({ "field": APACHE_DIRECTIVES_FIELD, "reason": reason })),
            _ => error,
        }
    }
}

/// `details.field` of a validation error about the project's custom Apache directives.
pub const APACHE_DIRECTIVES_FIELD: &str = "apacheDirectives";

impl From<PortError> for AppError {
    fn from(e: PortError) -> Self {
        let message = e.to_string();
        match e {
            PortError::Domain(d) => d.into(),
            PortError::NotFound { entity: "app", .. } => Self::new(ErrorCode::AppUnavailable, message),
            PortError::NotFound { .. } => Self::new(ErrorCode::NotFound, message),
            PortError::AlreadyExists { .. } => Self::new(ErrorCode::Conflict, message),
            PortError::Io { .. } => Self::new(ErrorCode::Io, message),
            PortError::Network(_) => Self::new(ErrorCode::Network, message),
            PortError::ChecksumMismatch { .. } => Self::new(ErrorCode::ChecksumMismatch, message),
            PortError::InvalidData { .. } => Self::new(ErrorCode::Validation, message),
            PortError::ConfigRejected { output } => Self::new(ErrorCode::ConfigRejected, message).with_details(serde_json::json!({ "output": output })),
            PortError::PortInUse { port, owner } => Self::new(ErrorCode::PortInUse, message).with_details(serde_json::json!({ "port": port, "owner": owner })),
            PortError::Process { .. } => Self::new(ErrorCode::ProcessFailed, message),
            PortError::ElevationDenied => Self::new(ErrorCode::ElevationDenied, message),
            PortError::Cancelled => Self::new(ErrorCode::Cancelled, message),
            PortError::Unsupported(_) => Self::new(ErrorCode::Unsupported, message),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_for_front_ends() {
        let e: AppError = PortError::ConfigRejected { output: "Syntax error on line 3".into() }.into();
        let json = serde_json::to_value(&e).unwrap();
        assert_eq!(json["code"], "config-rejected");
        assert_eq!(json["details"]["output"], "Syntax error on line 3");
    }

    #[test]
    fn maps_domain_conflicts() {
        let e: AppError = DomainError::DuplicateProjectId("shop".into()).into();
        assert_eq!(e.code, ErrorCode::Conflict);
        let e: AppError = PortError::not_found("app", "pipe").into();
        assert_eq!(e.code, ErrorCode::AppUnavailable);
    }

    #[test]
    fn refused_apache_directives_carry_the_reason_for_the_editor() {
        let e: AppError = lampo_domain::project::ApacheDirectives::new("Include x").unwrap_err().into();
        assert_eq!(e.code, ErrorCode::Validation);
        let details = e.details.unwrap();
        assert_eq!(details["field"], APACHE_DIRECTIVES_FIELD);
        assert!(details["reason"].as_str().unwrap().starts_with("line 1: `Include` is not allowed"), "{details}");
    }
}
