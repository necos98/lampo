//! Error types shared by every layer.
//!
//! * [`DomainError`] — a domain invariant was violated (bad input, conflicting projects, …).
//!   Produced by constructors and domain rules; never carries IO details.
//! * [`PortError`] — what a port implementation (adapter) reports back. Adapters map their
//!   library errors (io, reqwest, zip, …) into one of these variants, so the application layer
//!   can react by *category* (retry on network, show the `httpd -t` output, ask again for UAC…)
//!   without depending on any adapter crate.
//!
//! Both are `Clone + PartialEq` (they hold strings, not sources) so they can travel inside events
//! and be asserted on in tests.

use std::fmt::Display;

/// A violated domain invariant. The message is developer-facing English; the GUI maps
/// the variant to an Italian message.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DomainError {
    #[error("invalid PHP version `{0}`: expected `<major>.<minor>`, 5.6 or later")]
    InvalidPhpVersion(String),
    #[error("invalid version `{0}`")]
    InvalidVersion(String),
    #[error("invalid MySQL version `{0}`: expected `<major>.<minor>`, 5.7 or later")]
    InvalidMysqlSeries(String),
    #[error("invalid package kind `{0}`: use lowercase letters, digits and dashes")]
    InvalidPackageKind(String),
    #[error("invalid SHA-256 digest `{0}`: expected 64 hex characters")]
    InvalidSha256(String),
    #[error("invalid download URL `{0}`: only https URLs are allowed")]
    InvalidDownloadUrl(String),
    /// A live catalog advertises another SHA-256 than the one pinned in the embedded catalog.
    #[error("integrity check failed for {id}: the pinned SHA-256 is {frozen} but the live catalog publishes {live}; it is not installed")]
    ChecksumConflict { id: String, frozen: String, live: String },
    #[error("invalid identifier `{0}`: use 1-63 lowercase letters, digits and dashes, starting with a letter or digit")]
    InvalidSlug(String),
    #[error("invalid name: {0}")]
    InvalidName(String),
    #[error("invalid hostname `{value}`: {reason}")]
    InvalidHostname { value: String, reason: String },
    #[error("invalid project folder `{0}`: expected an absolute Windows path such as `C:\\sites\\shop`")]
    InvalidProjectRoot(String),
    #[error("invalid public directory `{0}`: it must be a relative path inside the project folder")]
    InvalidPublicDir(String),
    #[error("invalid PHP extension name `{0}`")]
    InvalidExtensionName(String),
    #[error("invalid php.ini key `{0}`")]
    InvalidIniKey(String),
    #[error("invalid php.ini value for `{key}`: {reason}")]
    InvalidIniValue { key: String, reason: String },
    #[error("invalid environment variable name `{name}`: {reason}")]
    InvalidEnvVarName { name: String, reason: String },
    /// Never carries the value, which may be a secret.
    #[error("invalid value of the environment variable `{name}`: {reason}")]
    InvalidEnvVarValue { name: String, reason: String },
    #[error("invalid environment variables: {0}")]
    InvalidEnvVars(String),
    #[error("invalid Apache directives: {0}")]
    InvalidApacheDirectives(String),
    #[error("invalid database name `{0}`: use 1-64 letters, digits and underscores")]
    InvalidDatabaseName(String),
    #[error("`{0}` is a MySQL system schema shared by every project and cannot be used, created, dropped, imported into or cloned")]
    ReservedDatabaseName(String),
    #[error("invalid port {0}")]
    InvalidPort(u32),
    #[error("invalid service id `{0}`: expected `apache`, `redis` or `mysql-<major>.<minor>`")]
    InvalidServiceId(String),
    #[error("port {port} is assigned to both {first} and {second}")]
    DuplicatePort { port: u16, first: String, second: String },
    #[error("a project with id `{0}` already exists")]
    DuplicateProjectId(String),
    #[error("project `{other}` already uses the folder `{root}`")]
    DuplicateProjectRoot { root: String, other: String },
    #[error("project `{other}` already uses the hostname `{hostname}`")]
    DuplicateHostname { hostname: String, other: String },
    #[error("the database `{0}` needs MySQL, which the project does not use")]
    DatabaseWithoutMysql(String),
    #[error("MySQL {0} is not configured in the settings")]
    UnknownMysqlSeries(String),
    #[error("invalid settings: {0}")]
    InvalidSettings(String),
    #[error("invalid new project: {0}")]
    InvalidScaffold(String),
    #[error("a built-in preset cannot be modified or deleted: `{0}`")]
    BuiltinPresetIsReadOnly(String),
    #[error("invalid worktree environment: {0}")]
    InvalidWorktree(String),
    #[error("no free loopback port left in the range {start}-{end}")]
    NoFreeLoopbackPort { start: u16, end: u16 },
    #[error("invalid log source `{0}`")]
    InvalidLogSource(String),
    #[error("invalid addresses: {0}")]
    InvalidAddresses(String),
    #[error("project `{other}` already uses the loopback port {port}")]
    DuplicateLoopbackPort { port: u16, other: String },
    #[error("loopback port {port} is already used by {service}")]
    LoopbackPortTaken { port: u16, service: String },
    /// A shim or `lampo exec` needs a tool that is not available for the resolved project.
    #[error("{tool} is not available: {reason}")]
    ToolNotAvailable { tool: String, reason: String },
}

/// Error reported by a port implementation.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PortError {
    #[error("{entity} `{id}` not found")]
    NotFound { entity: &'static str, id: String },
    #[error("{entity} `{id}` already exists")]
    AlreadyExists { entity: &'static str, id: String },
    #[error("I/O error while {context}: {message}")]
    Io { context: String, message: String },
    #[error("network error: {0}")]
    Network(String),
    #[error("checksum mismatch for `{file}`: expected {expected}, got {actual}")]
    ChecksumMismatch { file: String, expected: String, actual: String },
    #[error("invalid data in {source_name}: {message}")]
    InvalidData { source_name: String, message: String },
    /// `httpd -t` (or an equivalent validator) rejected the generated configuration.
    /// `output` is the validator's full output, shown verbatim to the user.
    #[error("generated configuration was rejected: {output}")]
    ConfigRejected { output: String },
    #[error("port {port} is already in use{}", owner.as_ref().map(|o| format!(" by {o}")).unwrap_or_default())]
    PortInUse { port: u16, owner: Option<String> },
    #[error("process `{program}` failed: {message}")]
    Process { program: String, message: String },
    /// The user refused the UAC prompt (or elevation is otherwise impossible).
    #[error("administrator permission was denied")]
    ElevationDenied,
    /// The operation observed a [`crate::progress::CancelFlag`] and stopped.
    #[error("operation cancelled")]
    Cancelled,
    #[error("unsupported: {0}")]
    Unsupported(String),
    /// The operation failed (or was cancelled) after it had started changing state: what it
    /// already wrote was kept. The inner error says why it stopped.
    #[error("{0} (stopped part-way: the changes already made were kept)")]
    Interrupted(Box<PortError>),
    #[error(transparent)]
    Domain(#[from] DomainError),
}

impl PortError {
    /// Convenience constructor for adapters: `PortError::io("writing httpd.conf", err)`.
    pub fn io(context: impl Into<String>, err: impl Display) -> Self {
        Self::Io { context: context.into(), message: err.to_string() }
    }

    pub fn not_found(entity: &'static str, id: impl Into<String>) -> Self {
        Self::NotFound { entity, id: id.into() }
    }

    pub fn invalid_data(source_name: impl Into<String>, message: impl Display) -> Self {
        Self::InvalidData { source_name: source_name.into(), message: message.to_string() }
    }

    pub fn process(program: impl Into<String>, message: impl Display) -> Self {
        Self::Process { program: program.into(), message: message.to_string() }
    }

    /// Marks `self` as having happened part-way through a state change ([`Self::Interrupted`]).
    pub fn interrupted(self) -> Self {
        match self {
            already @ Self::Interrupted(_) => already,
            cause => Self::Interrupted(Box::new(cause)),
        }
    }
}

/// Result type returned by every port method.
pub type PortResult<T> = Result<T, PortError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_in_use_message_mentions_owner_when_known() {
        let e = PortError::PortInUse { port: 80, owner: Some("skype.exe (pid 42)".into()) };
        assert_eq!(e.to_string(), "port 80 is already in use by skype.exe (pid 42)");
        let e = PortError::PortInUse { port: 80, owner: None };
        assert_eq!(e.to_string(), "port 80 is already in use");
    }

    #[test]
    fn interrupted_wraps_its_cause_once() {
        let e = PortError::Cancelled.interrupted().interrupted();
        assert_eq!(e, PortError::Interrupted(Box::new(PortError::Cancelled)));
        assert_eq!(e.to_string(), "operation cancelled (stopped part-way: the changes already made were kept)");
    }

    #[test]
    fn domain_errors_convert_into_port_errors() {
        let e: PortError = DomainError::InvalidPort(0).into();
        assert!(matches!(e, PortError::Domain(DomainError::InvalidPort(0))));
    }
}
