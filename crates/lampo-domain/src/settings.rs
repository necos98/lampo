//! Global application settings (stored in `%APPDATA%\Lampo\settings.toml`).
//!
//! Service instances are data (`services`, one entry per instance: `mysql-8.4`, `redis`, …).
//! Files written before that (with `mysql_instances` and `redis_port`) are migrated on read.

use serde::{Deserialize, Serialize};

use crate::error::DomainError;
use crate::hostname::Tld;
use crate::mysql::MysqlSeries;
use crate::php::PhpMinor;
use crate::service::{ServiceId, ServiceKind};
use crate::worktree::PortRange;

/// One configured service instance (shared by all projects using it).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceInstanceSettings {
    pub service: ServiceId,
    pub port: u16,
    /// `true`: runs whenever the app runs (the default MySQL). `false`: started only while a
    /// served site needs it.
    #[serde(default)]
    pub always_on: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "SettingsFile")]
pub struct Settings {
    /// PHP used by the CLI shims outside any project folder. `None` until a PHP is installed.
    pub default_php: Option<PhpMinor>,
    pub tld: Tld,
    /// The web tier's shared port.
    pub apache_port: u16,
    /// Every other service instance with its port.
    pub services: Vec<ServiceInstanceSettings>,
    /// Range for dedicated loopback listeners (worktree environments, `LoopbackPort` addresses).
    pub loopback_ports: PortRange,
    /// Closing the window hides to tray (and destroys the webview) instead of quitting.
    pub close_to_tray: bool,
    pub first_run_completed: bool,
}

fn mysql_id(series: MysqlSeries) -> ServiceId {
    ServiceId::new(ServiceKind::MYSQL, Some(series.as_series()))
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            default_php: None,
            tld: Tld::default(),
            apache_port: 80,
            services: vec![
                ServiceInstanceSettings { service: mysql_id(MysqlSeries::DEFAULT), port: 3306, always_on: true },
                ServiceInstanceSettings { service: mysql_id(MysqlSeries::LEGACY_5_7), port: 3307, always_on: false },
                ServiceInstanceSettings { service: ServiceId::new(ServiceKind::REDIS, None), port: 6379, always_on: false },
            ],
            loopback_ports: PortRange::default(),
            close_to_tray: true,
            first_run_completed: false,
        }
    }
}

/// The file shape: today's fields plus the legacy ones (`mysql_instances`, `redis_port`), which
/// are folded into `services` when present.
#[derive(Deserialize)]
#[serde(default)]
struct SettingsFile {
    default_php: Option<PhpMinor>,
    tld: Tld,
    apache_port: u16,
    services: Option<Vec<ServiceInstanceSettings>>,
    loopback_ports: PortRange,
    close_to_tray: bool,
    first_run_completed: bool,
    mysql_instances: Option<Vec<LegacyMysqlInstance>>,
    redis_port: Option<u16>,
}

#[derive(Deserialize)]
struct LegacyMysqlInstance {
    series: MysqlSeries,
    port: u16,
    always_on: bool,
}

impl Default for SettingsFile {
    fn default() -> Self {
        let d = Settings::default();
        Self {
            default_php: d.default_php,
            tld: d.tld,
            apache_port: d.apache_port,
            services: None,
            loopback_ports: d.loopback_ports,
            close_to_tray: d.close_to_tray,
            first_run_completed: d.first_run_completed,
            mysql_instances: None,
            redis_port: None,
        }
    }
}

impl From<SettingsFile> for Settings {
    fn from(f: SettingsFile) -> Self {
        let mut services = f.services.unwrap_or_else(|| Settings::default().services);
        if let Some(legacy) = f.mysql_instances {
            services.retain(|s| s.service.kind() != &ServiceKind::MYSQL);
            let mysql = legacy.into_iter().map(|m| ServiceInstanceSettings { service: mysql_id(m.series), port: m.port, always_on: m.always_on });
            services.splice(0..0, mysql);
        }
        if let Some(port) = f.redis_port {
            let redis = ServiceId::new(ServiceKind::REDIS, None);
            match services.iter_mut().find(|s| s.service == redis) {
                Some(s) => s.port = port,
                None => services.push(ServiceInstanceSettings { service: redis, port, always_on: false }),
            }
        }
        Self {
            default_php: f.default_php,
            tld: f.tld,
            apache_port: f.apache_port,
            services,
            loopback_ports: f.loopback_ports,
            close_to_tray: f.close_to_tray,
            first_run_completed: f.first_run_completed,
        }
    }
}

impl Settings {
    /// Validates cross-field invariants: non-zero, distinct ports; unique instances; at least
    /// one always-on MySQL instance (the default one).
    pub fn validate(&self) -> Result<(), DomainError> {
        let mut ports: Vec<(u16, String)> = vec![(self.apache_port, ServiceId::apache().to_string())];
        for s in &self.services {
            if s.service.is_apache() {
                return Err(DomainError::InvalidSettings("Apache's port is `apache_port`".into()));
            }
            if self.services.iter().filter(|o| o.service == s.service).count() > 1 {
                return Err(DomainError::InvalidSettings(format!("{} is configured twice", s.service)));
            }
            ports.push((s.port, s.service.to_string()));
        }
        for (i, (port, owner)) in ports.iter().enumerate() {
            if *port == 0 {
                return Err(DomainError::InvalidPort(0));
            }
            if let Some((_, first)) = ports[..i].iter().find(|(p, _)| p == port) {
                return Err(DomainError::DuplicatePort { port: *port, first: first.clone(), second: owner.clone() });
            }
        }
        PortRange::new(self.loopback_ports.start, self.loopback_ports.end)?;
        if !self.services.iter().any(|s| s.always_on && s.service.kind() == &ServiceKind::MYSQL) {
            return Err(DomainError::InvalidSettings("one MySQL instance must be always on".into()));
        }
        Ok(())
    }

    /// The configured instance `id`, if any.
    pub fn service(&self, id: &ServiceId) -> Option<&ServiceInstanceSettings> {
        self.services.iter().find(|s| &s.service == id)
    }

    /// The configured port of `id` (Apache: `apache_port`); `None` when it is not configured.
    pub fn service_port(&self, id: &ServiceId) -> Option<u16> {
        match id.is_apache() {
            true => Some(self.apache_port),
            false => self.service(id).map(|s| s.port),
        }
    }

    /// The service configured on `port` (`apache`, `mysql-8.4`, …), if any. Loopback site
    /// addresses must avoid these ports.
    pub fn service_on_port(&self, port: u16) -> Option<String> {
        if port == self.apache_port {
            return Some(ServiceId::apache().to_string());
        }
        self.services.iter().find(|s| s.port == port).map(|s| s.service.to_string())
    }

    /// Configured MySQL series, in settings order.
    pub fn mysql_series(&self) -> impl Iterator<Item = (MysqlSeries, &ServiceInstanceSettings)> {
        self.services.iter().filter(|s| s.service.kind() == &ServiceKind::MYSQL).filter_map(|s| s.service.version().and_then(|v| MysqlSeries::try_from(v).ok()).map(|series| (series, s)))
    }

    pub fn mysql_port(&self, series: MysqlSeries) -> Option<u16> {
        self.service_port(&mysql_id(series))
    }

    /// The series new projects get by default (the first always-on instance).
    pub fn default_mysql(&self) -> MysqlSeries {
        self.mysql_series().find(|(_, s)| s.always_on).map(|(series, _)| series).unwrap_or(MysqlSeries::DEFAULT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid() {
        let s = Settings::default();
        s.validate().unwrap();
        assert_eq!(s.default_mysql(), MysqlSeries::DEFAULT);
        assert_eq!(s.mysql_port(MysqlSeries::LEGACY_5_7), Some(3307));
        assert_eq!(s.service_port(&"redis".parse().unwrap()), Some(6379));
        assert_eq!(s.service_on_port(3307).as_deref(), Some("mysql-5.7"));
    }

    #[test]
    fn rejects_port_collisions() {
        let mut s = Settings::default();
        s.services[2].port = 3306;
        assert!(matches!(s.validate(), Err(DomainError::DuplicatePort { port: 3306, .. })));
        let s = Settings { apache_port: 0, ..Default::default() };
        assert!(s.validate().is_err());
    }

    #[test]
    fn requires_an_always_on_mysql() {
        let mut s = Settings::default();
        for m in &mut s.services {
            m.always_on = false;
        }
        assert!(s.validate().is_err());
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let s: Settings = serde_json::from_str(r#"{"apache_port": 8080}"#).unwrap();
        assert_eq!(s.apache_port, 8080);
        assert_eq!(s.services, Settings::default().services);
    }

    #[test]
    fn legacy_files_are_migrated_on_read() {
        let legacy = r#"{"apache_port": 80, "redis_port": 6380,
            "mysql_instances": [{"series": "8.4", "port": 3310, "always_on": true}, {"series": "8.0", "port": 3311, "always_on": false}]}"#;
        let s: Settings = serde_json::from_str(legacy).unwrap();
        assert_eq!(
            s.services.iter().map(|i| (i.service.to_string(), i.port, i.always_on)).collect::<Vec<_>>(),
            [("mysql-8.4".to_string(), 3310, true), ("mysql-8.0".to_string(), 3311, false), ("redis".to_string(), 6380, false),]
        );
        s.validate().unwrap();
        // Saved in the new shape, read back unchanged.
        let again: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(again, s);
    }
}
