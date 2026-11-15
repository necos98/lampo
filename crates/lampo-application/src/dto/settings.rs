use serde::{Deserialize, Serialize};

/// One configured service instance (`mysql-8.4`, `redis`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceInstanceDto {
    /// Service id (`mysql-8.4`).
    pub service: String,
    /// Display label from the kind's descriptor (`MySQL 8.4`); ignored on input.
    #[serde(default)]
    pub label: String,
    pub port: u16,
    pub always_on: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortRangeDto {
    pub start: u16,
    pub end: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsDto {
    pub default_php_version: Option<String>,
    pub tld: String,
    pub apache_port: u16,
    /// Every other service instance and its port.
    pub services: Vec<ServiceInstanceDto>,
    /// Dedicated loopback listeners (worktree environments, loopback-port addresses).
    pub loopback_port_range: PortRangeDto,
    pub close_to_tray: bool,
    /// Read-only from the GUI's point of view (set by the first-run wizard).
    pub first_run_completed: bool,
}

/// A registered kind of service, so front-ends offer what the backend knows (no hard-coded
/// list): the project editor lists these, with the configured versions of versioned kinds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceKindDto {
    pub kind: String,
    pub label: String,
    /// A project picks one of `versions`.
    pub versioned: bool,
    /// Configured series of a versioned kind, settings order (`["8.4", "5.7"]`).
    pub versions: Vec<String>,
    /// The version new projects get (first always-on instance), if any.
    pub default_version: Option<String>,
}
