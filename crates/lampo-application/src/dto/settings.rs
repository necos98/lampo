use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{AddressingDto, SyncReportDto};

/// One configured service instance (`mysql-8.4`, `redis`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "ServiceInstance"))]
#[serde(rename_all = "camelCase")]
pub struct ServiceInstanceDto {
    /// Service id (`mysql-8.4`).
    pub service: String,
    /// Display label from the kind's descriptor (`MySQL 8.4`); ignored on input.
    #[serde(default)]
    pub label: String,
    /// The main port (Mailpit: SMTP).
    pub port: u16,
    pub always_on: bool,
    /// Web UI port of kinds that have one (Mailpit), else `null`. On output the effective value
    /// (configured, else the default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui_port: Option<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "PortRange"))]
#[serde(rename_all = "camelCase")]
pub struct PortRangeDto {
    pub start: u16,
    pub end: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "Settings"))]
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
    /// Addresses new projects get when the creator chooses none.
    pub default_addressing: AddressingDto,
}

/// A registered kind of service, so front-ends offer what the backend knows (no hard-coded
/// list): the project editor lists these, with the configured versions of versioned kinds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "ServiceKindInfo"))]
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

/// `lampo settings set` (control request `settings-patch`): only the fields given change,
/// applied to the stored settings inside the app (never a whole [`SettingsDto`] from another
/// process, so a concurrent GUI save is not reverted). Omitted fields default: a client sends
/// only what it changes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SettingsPatchRequest {
    /// Apache's shared port.
    pub apache_port: Option<u16>,
    /// Ports of configured service instances, by service id (`mysql-8.4`, `redis`).
    pub service_ports: BTreeMap<String, u16>,
    /// Web UI ports of configured instances whose kind has one (`mailpit`).
    pub service_ui_ports: BTreeMap<String, u16>,
    /// PHP used by the shims outside any project (`8.4`); must be installed.
    pub default_php_version: Option<String>,
    /// Range new dedicated loopback ports (worktree environments) are taken from.
    pub loopback_port_range: Option<PortRangeDto>,
    /// Addresses new projects get when the creator chooses none.
    pub default_addressing: Option<AddressingDto>,
}

impl SettingsPatchRequest {
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

/// One changed setting: `setting` is `apache.port`, `<service>.port` (`mysql-8.4.port`), `<service>.ui-port` (`mailpit.ui-port`),
/// `default-php`, `loopback-port-range` or `default-addressing`; `from`/`to` are display values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingChangeDto {
    pub setting: String,
    pub from: String,
    pub to: String,
}

/// Reply of a settings patch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsPatchResultDto {
    /// The settings after the patch.
    pub settings: SettingsDto,
    /// What actually changed (empty: everything already had these values; nothing was synced).
    pub changes: Vec<SettingChangeDto>,
    /// The environment sync that applied the change (running services restarted on their new
    /// port; empty when nothing changed).
    pub sync: SyncReportDto,
    /// Follow-ups for the caller: services not running (the port applies when they start),
    /// projects whose `.env` / URLs mention the old port.
    pub notes: Vec<String>,
}
