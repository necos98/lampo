//! Global application settings (stored in `%APPDATA%\Lampo\settings.toml`).
//!
//! Service instances are data (`services`, one entry per instance: `mysql-8.4`, `redis`,
//! `mailpit`, …). Files written before that (with `mysql_instances` and `redis_port`) are
//! migrated on read, and a file that lacks a built-in unversioned instance (Mailpit, added
//! later) gets it with its default ports (moved up past any port already taken).
//!
//! An instance has one port, plus an optional `ui_port` for kinds with a second listener (the
//! Mailpit web UI): absent in older files, it falls back to the kind's default.

use serde::{Deserialize, Serialize};

use crate::error::DomainError;
use crate::hostname::Tld;
use crate::mysql::MysqlSeries;
use crate::php::PhpMinor;
use crate::service::{ServiceDescriptor, ServiceId, ServiceKind};
use crate::tcp_port::PortRange;

/// One configured service instance (shared by all projects using it).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceInstanceSettings {
    pub service: ServiceId,
    /// The main port (Mailpit: SMTP).
    pub port: u16,
    /// `true`: runs whenever the app runs (the default MySQL). `false`: started only while a
    /// served site needs it.
    #[serde(default)]
    pub always_on: bool,
    /// Second listener of kinds that have one (Mailpit's web UI). `None`: the kind's default
    /// (`ServiceDescriptor::default_ui_port`), or no second listener.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui_port: Option<u16>,
}

/// SMTP endpoint and web UI of the shared Mailpit, for sites that use it. Plain SMTP: no
/// authentication (any credentials are accepted) and no encryption.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MailConnection {
    pub host: String,
    pub smtp_port: u16,
    pub web_port: u16,
}

impl MailConnection {
    /// `http://127.0.0.1:8025`.
    pub fn web_url(&self) -> String {
        format!("http://{}:{}", self.host, self.web_port)
    }
}

impl ServiceInstanceSettings {
    pub fn new(service: ServiceId, port: u16, always_on: bool) -> Self {
        Self { service, port, always_on, ui_port: None }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "SettingsFile")]
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
                ServiceInstanceSettings::new(mysql_id(MysqlSeries::DEFAULT), 3306, true),
                ServiceInstanceSettings::new(mysql_id(MysqlSeries::LEGACY_5_7), 3307, false),
                ServiceInstanceSettings::new(ServiceId::new(ServiceKind::REDIS, None), 6379, false),
                ServiceInstanceSettings { ui_port: Some(8025), ..ServiceInstanceSettings::new(ServiceId::new(ServiceKind::MAILPIT, None), 1025, false) },
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

/// Migrates the legacy fields, then validates: a file edited by hand into an invalid state
/// (two services on one port, no always-on MySQL, …) does not load.
impl TryFrom<SettingsFile> for Settings {
    type Error = DomainError;

    fn try_from(f: SettingsFile) -> Result<Self, Self::Error> {
        let mut services = f.services.unwrap_or_else(|| Settings::default().services);
        if let Some(legacy) = f.mysql_instances {
            services.retain(|s| s.service.kind() != &ServiceKind::MYSQL);
            let mysql = legacy.into_iter().map(|m| ServiceInstanceSettings::new(mysql_id(m.series), m.port, m.always_on));
            services.splice(0..0, mysql);
        }
        if let Some(port) = f.redis_port {
            let redis = ServiceId::new(ServiceKind::REDIS, None);
            match services.iter_mut().find(|s| s.service == redis) {
                Some(s) => s.port = port,
                None => services.push(ServiceInstanceSettings::new(redis, port, false)),
            }
        }
        add_missing_builtins(&mut services, f.apache_port);
        let settings = Self {
            default_php: f.default_php,
            tld: f.tld,
            apache_port: f.apache_port,
            services,
            loopback_ports: f.loopback_ports,
            close_to_tray: f.close_to_tray,
            first_run_completed: f.first_run_completed,
        };
        settings.validate()?;
        Ok(settings)
    }
}

/// Appends the default unversioned instances (Redis, Mailpit) a file does not list, so a kind
/// added after the file was written is configurable like the others. A default port another
/// instance (or Apache) already has moves up to the next free one.
fn add_missing_builtins(services: &mut Vec<ServiceInstanceSettings>, apache_port: u16) {
    for default in Settings::default().services.into_iter().filter(|d| d.service.version().is_none()) {
        if services.iter().any(|s| s.service == default.service) {
            continue;
        }
        let mut taken: Vec<u16> = services.iter().flat_map(|s| std::iter::once(s.port).chain(s.ui_port)).collect();
        taken.push(apache_port);
        let mut free = |from: u16| {
            let port = (from..=u16::MAX).find(|p| !taken.contains(p)).unwrap_or(from);
            taken.push(port);
            port
        };
        let port = free(default.port);
        let ui_port = default.ui_port.map(&mut free);
        services.push(ServiceInstanceSettings { port, ui_port, ..default });
    }
}

impl Settings {
    /// Validates cross-field invariants: non-zero, distinct ports (web UI ports included);
    /// unique instances; at least one always-on MySQL instance (the default one).
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
            if let Some(ui) = s.ui_port {
                ports.push((ui, format!("{} (web UI)", s.service)));
            }
        }
        for (i, (port, owner)) in ports.iter().enumerate() {
            if *port == 0 {
                return Err(DomainError::InvalidPort(0));
            }
            if let Some((_, first)) = ports[..i].iter().find(|(p, _)| p == port) {
                return Err(DomainError::DuplicatePort { port: *port, first: first.clone(), second: owner.clone() });
            }
        }
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
        self.services.iter().find(|s| s.port == port || s.ui_port == Some(port)).map(|s| s.service.to_string())
    }

    /// The configured web UI port of `id`, if it has one set.
    pub fn service_ui_port(&self, id: &ServiceId) -> Option<u16> {
        self.service(id).and_then(|s| s.ui_port)
    }

    /// Where the shared Mailpit catches mail (configured ports, else the kind's defaults).
    pub fn mail_connection(&self) -> MailConnection {
        let id = ServiceId::new(ServiceKind::MAILPIT, None);
        let d = crate::service::builtin::Mailpit;
        MailConnection {
            host: "127.0.0.1".into(),
            smtp_port: self.service_port(&id).unwrap_or(d.default_port()),
            web_port: self.service_ui_port(&id).or(d.default_ui_port()).unwrap_or(8025),
        }
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

    /// A file edited by hand into an invalid state does not load: `validate` runs on read.
    #[test]
    fn invalid_files_are_rejected_on_read() {
        let clash = r#"{"apache_port": 3306}"#;
        let err = serde_json::from_str::<Settings>(clash).unwrap_err();
        assert!(err.to_string().contains("port 3306"), "{err}");
        let no_always_on = r#"{"services": [{"service": "mysql-8.4", "port": 3306, "always_on": false}]}"#;
        assert!(serde_json::from_str::<Settings>(no_always_on).unwrap_err().to_string().contains("always on"));
        let zero = r#"{"apache_port": 0}"#;
        assert!(serde_json::from_str::<Settings>(zero).is_err());
    }

    #[test]
    fn legacy_files_are_migrated_on_read() {
        let legacy = r#"{"apache_port": 80, "redis_port": 6380,
            "mysql_instances": [{"series": "8.4", "port": 3310, "always_on": true}, {"series": "8.0", "port": 3311, "always_on": false}]}"#;
        let s: Settings = serde_json::from_str(legacy).unwrap();
        assert_eq!(
            s.services.iter().map(|i| (i.service.to_string(), i.port, i.always_on)).collect::<Vec<_>>(),
            [("mysql-8.4".to_string(), 3310, true), ("mysql-8.0".to_string(), 3311, false), ("redis".to_string(), 6380, false), ("mailpit".to_string(), 1025, false)]
        );
        s.validate().unwrap();
        // Saved in the new shape, read back unchanged.
        let again: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(again, s);
    }

    #[test]
    fn mailpit_has_an_smtp_and_a_web_ui_port() {
        let s = Settings::default();
        let mailpit: ServiceId = "mailpit".parse().unwrap();
        assert_eq!((s.service_port(&mailpit), s.service_ui_port(&mailpit)), (Some(1025), Some(8025)));
        assert_eq!(s.service_on_port(8025).as_deref(), Some("mailpit"));
        assert_eq!(s.service_on_port(1025).as_deref(), Some("mailpit"));
        let mail = s.mail_connection();
        assert_eq!((mail.host.as_str(), mail.smtp_port, mail.web_url()), ("127.0.0.1", 1025, "http://127.0.0.1:8025".to_string()));
        let mut clash = s.clone();
        clash.services[3].ui_port = Some(3306);
        assert!(matches!(clash.validate(), Err(DomainError::DuplicatePort { port: 3306, .. })), "{:?}", clash.validate());
        let mut own = s.clone();
        own.services[3].ui_port = Some(1025);
        assert!(own.validate().is_err(), "SMTP and UI need distinct ports");
        // Instances without a second listener do not write the field.
        let json = serde_json::to_value(&s).unwrap();
        assert!(json["services"][2].get("ui_port").is_none(), "{json}");
        assert_eq!(json["services"][3]["ui_port"], 8025);
    }

    #[test]
    fn files_written_before_mailpit_get_it_on_free_ports() {
        let before = r#"{"apache_port": 8025, "services": [
            {"service": "mysql-8.4", "port": 3306, "always_on": true},
            {"service": "redis", "port": 1025}]}"#;
        let s: Settings = serde_json::from_str(before).unwrap();
        assert_eq!(s.services.len(), 3);
        let mailpit = &s.services[2];
        assert_eq!((mailpit.service.to_string(), mailpit.port, mailpit.ui_port, mailpit.always_on), ("mailpit".to_string(), 1026, Some(8026), false));
        s.validate().unwrap();
        // A file that already lists Mailpit (without a UI port) keeps it as it is.
        let listed: Settings = serde_json::from_str(r#"{"services": [{"service": "mysql-8.4", "port": 3306, "always_on": true}, {"service": "mailpit", "port": 2025}]}"#).unwrap();
        let m = listed.service(&"mailpit".parse().unwrap()).unwrap();
        assert_eq!((m.port, m.ui_port), (2025, None));
        assert_eq!(listed.services.iter().map(|s| s.service.to_string()).collect::<Vec<_>>(), ["mysql-8.4", "mailpit", "redis"]);
    }
}
