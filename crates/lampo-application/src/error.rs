//! The single error type returned by every use case and sent to front-ends.
//!
//! `code` is stable (GUI messages and CLI exit codes are keyed by it); `message` is English,
//! developer-facing; `details` carries structured extras (e.g. `httpd -t` output).

use lampo_domain::{DomainError, PortError, env_var};
use serde::{Deserialize, Serialize};

/// Stable error categories. Adding a variant is a contract change: regenerate the TS bindings
/// (`ts_bindings`), update the GUI's code list (`src/lib/api/errors.ts`), the Italian messages
/// and the CLI exit-code table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
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
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
#[error("{code:?}: {message}")]
pub struct AppError {
    pub code: ErrorCode,
    /// English, developer-facing: front-ends show their own text for `code`.
    pub message: String,
    /// Structured extras the GUI puts into its own text: `{ output }` (a string, the program's
    /// output) for `config-rejected` and a failed `process-failed` step, plus `excludedSites` when
    /// some sites were left out; `{ port, owner }` for `port-in-use`; `{ entity, id }` for
    /// `not-found`; `{ package, php? }`, `{ tool }` or `{ redistributable }` for
    /// `package-missing`; `{ package, usedBy }` for a package still in use (`conflict`);
    /// `{ field: "envVars", reason, name?, part?, reserved? }` for a refused environment variable
    /// (`validation`; never its value).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(type = "unknown"))]
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

    /// `details` carry `entity` (English, e.g. `project`) and `id` for the GUI's text.
    pub fn not_found(entity: &str, id: impl std::fmt::Display) -> Self {
        let id = id.to_string();
        Self::new(ErrorCode::NotFound, format!("{entity} `{id}` not found")).with_details(serde_json::json!({ "entity": entity, "id": id }))
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
            // The GUI shows it on the variable's row: the name and which part, never the value.
            DomainError::InvalidEnvVarName { name, reason } => {
                let reserved = env_var::is_reserved(&name);
                error.with_details(serde_json::json!({ "field": ENV_VARS_FIELD, "name": name, "part": "name", "reserved": reserved, "reason": reason }))
            }
            DomainError::InvalidEnvVarValue { name, reason } => error.with_details(serde_json::json!({ "field": ENV_VARS_FIELD, "name": name, "part": "value", "reason": reason })),
            DomainError::InvalidEnvVars(reason) => error.with_details(serde_json::json!({ "field": ENV_VARS_FIELD, "reason": reason })),
            _ => error,
        }
    }
}

/// `details.field` of a validation error about the project's environment variables.
pub const ENV_VARS_FIELD: &str = "envVars";

/// `details.field` of a validation error about the project's custom Apache directives.
pub const APACHE_DIRECTIVES_FIELD: &str = "apacheDirectives";

impl From<PortError> for AppError {
    fn from(e: PortError) -> Self {
        let message = e.to_string();
        match e {
            PortError::Domain(d) => d.into(),
            PortError::NotFound { entity: "app", .. } => Self::new(ErrorCode::AppUnavailable, message),
            PortError::NotFound { entity, id } => Self::not_found(entity, id),
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
            // Same category as the cause (a cancellation stays `cancelled`), told it was partial.
            PortError::Interrupted(cause) => Self { message, ..Self::from(*cause) },
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
    fn not_found_names_the_entity_and_the_id_for_front_ends() {
        let e: AppError = PortError::not_found("site", "shop.feature").into();
        assert_eq!(e.code, ErrorCode::NotFound);
        assert_eq!(e.message, "site `shop.feature` not found");
        assert_eq!(e.details, Some(serde_json::json!({ "entity": "site", "id": "shop.feature" })));
    }

    #[test]
    fn an_interrupted_error_keeps_the_category_of_its_cause() {
        let e: AppError = PortError::Cancelled.interrupted().into();
        assert_eq!(e.code, ErrorCode::Cancelled);
        assert!(e.message.contains("stopped part-way"), "{}", e.message);
        let e: AppError = PortError::process("mysql", "ERROR 1064").interrupted().into();
        assert_eq!(e.code, ErrorCode::ProcessFailed);
    }

    #[test]
    fn refused_apache_directives_carry_the_reason_for_the_editor() {
        let e: AppError = lampo_domain::project::ApacheDirectives::new("Include x").unwrap_err().into();
        assert_eq!(e.code, ErrorCode::Validation);
        let details = e.details.unwrap();
        assert_eq!(details["field"], APACHE_DIRECTIVES_FIELD);
        assert!(details["reason"].as_str().unwrap().starts_with("line 1: `Include` is not allowed"), "{details}");
    }

    fn env_error(pairs: impl IntoIterator<Item = (String, String)>) -> AppError {
        env_var::ProjectEnvVars::from_pairs(pairs).unwrap_err().into()
    }

    fn pair(name: &str, value: &str) -> [(String, String); 1] {
        [(name.to_string(), value.to_string())]
    }

    #[test]
    fn refused_environment_variable_names_point_at_their_row() {
        let e = env_error(pair("lampo_project", "x"));
        assert_eq!(e.code, ErrorCode::Validation);
        let details = e.details.unwrap();
        assert_eq!(details["field"], ENV_VARS_FIELD);
        assert_eq!(details["name"], "lampo_project");
        assert_eq!(details["part"], "name");
        assert_eq!(details["reserved"], true);

        let details = env_error(pair("1BAD", "x")).details.unwrap();
        assert_eq!(details["part"], "name");
        assert_eq!(details["reserved"], false);
    }

    #[test]
    fn refused_environment_variable_values_are_never_repeated() {
        let e = env_error(pair("API_KEY", "s3cret${HOME}"));
        let details = e.details.unwrap();
        assert_eq!(details["name"], "API_KEY");
        assert_eq!(details["part"], "value");
        assert!(!details.to_string().contains("s3cret") && !e.message.contains("s3cret"), "{details}");
    }

    #[test]
    fn too_many_environment_variables_name_no_row() {
        let many = (0..=env_var::ProjectEnvVars::MAX_COUNT).map(|i| (format!("V{i}"), String::new()));
        let details = env_error(many).details.unwrap();
        assert_eq!(details["field"], ENV_VARS_FIELD);
        assert!(details.get("name").is_none(), "{details}");
    }
}
