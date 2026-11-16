//! Background services (Apache, MySQL instances, Redis, …), their status, and the rule that
//! decides which of them must run.
//!
//! A service *kind* is data ([`ServiceKind`], an open identifier like `PackageKind`), described
//! by a [`ServiceDescriptor`] registered in the composition roots (built-ins in [`builtin`]).
//! Adding PostgreSQL, Mailpit or Memcached = one descriptor + one `ServiceRunner`; projects
//! name the services they use in `ProjectEnvironment::services`. Apache stays special as the web
//! tier: it runs whenever a site is served.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::error::DomainError;
use crate::package::{EntryRole, PackageKind};
use crate::project::Project;
use crate::settings::Settings;
use crate::site::SiteId;
use crate::tcp_port::Port;
use crate::version::VersionSeries;
use crate::worktree::WorktreeEnvironment;

/// Kind of service (`apache`, `mysql`, `redis`): lowercase letters, digits, dashes. Routes a
/// [`ServiceId`] to its descriptor and its `ServiceRunner`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ServiceKind(Cow<'static, str>);

impl ServiceKind {
    /// The web tier.
    pub const APACHE: ServiceKind = ServiceKind(Cow::Borrowed("apache"));
    pub const MYSQL: ServiceKind = ServiceKind(Cow::Borrowed("mysql"));
    pub const REDIS: ServiceKind = ServiceKind(Cow::Borrowed("redis"));
    /// Local SMTP catcher with a web UI.
    pub const MAILPIT: ServiceKind = ServiceKind(Cow::Borrowed("mailpit"));

    pub fn new(kind: impl Into<String>) -> Result<Self, DomainError> {
        let k: String = kind.into();
        let ok = !k.is_empty() && k.len() <= 32 && k.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') && k.as_bytes()[0].is_ascii_lowercase() && !k.ends_with('-');
        if ok { Ok(Self(Cow::Owned(k))) } else { Err(DomainError::InvalidServiceId(k)) }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ServiceKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for ServiceKind {
    type Error = DomainError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::new(s)
    }
}

impl From<ServiceKind> for String {
    fn from(v: ServiceKind) -> Self {
        v.0.into_owned()
    }
}

/// One concrete service instance: a kind, plus a version series for kinds whose instances run
/// side by side per series. Text form: `apache`, `redis`, `mysql-8.4`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ServiceId {
    kind: ServiceKind,
    version: Option<VersionSeries>,
}

impl ServiceId {
    pub fn new(kind: ServiceKind, version: Option<VersionSeries>) -> Self {
        Self { kind, version }
    }

    pub fn apache() -> Self {
        Self::new(ServiceKind::APACHE, None)
    }

    pub fn kind(&self) -> &ServiceKind {
        &self.kind
    }

    pub fn version(&self) -> Option<VersionSeries> {
        self.version
    }

    pub fn is_apache(&self) -> bool {
        self.kind == ServiceKind::APACHE
    }

    /// `<kind>` or `<kind>\<series>`: where this instance's generated config and data live.
    pub fn path_segments(&self) -> Vec<String> {
        let mut v = vec![self.kind.to_string()];
        v.extend(self.version.map(|s| s.to_string()));
        v
    }
}

impl fmt::Display for ServiceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.version {
            Some(v) => write!(f, "{}-{v}", self.kind),
            None => write!(f, "{}", self.kind),
        }
    }
}

impl FromStr for ServiceId {
    type Err = DomainError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bad = || DomainError::InvalidServiceId(s.to_string());
        if let Some((kind, version)) = s.rsplit_once('-') {
            if version.starts_with(|c: char| c.is_ascii_digit()) {
                return Ok(Self::new(ServiceKind::new(kind).map_err(|_| bad())?, Some(VersionSeries::parse(version).map_err(|_| bad())?)));
            }
        }
        Ok(Self::new(ServiceKind::new(s).map_err(|_| bad())?, None))
    }
}

impl TryFrom<String> for ServiceId {
    type Error = DomainError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

impl From<ServiceId> for String {
    fn from(v: ServiceId) -> Self {
        v.to_string()
    }
}

/// How a kind of client connects, for the shims and `lampo exec`/`env`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceClient {
    /// Default user (`root` for MySQL), if the protocol has one.
    pub user: Option<&'static str>,
    /// Variables exported as `LAMPO_<prefix>_HOST`, `_PORT`, `_USER`, `_DATABASE`.
    pub env_prefix: &'static str,
    /// The entry point of the service's package holding its clients.
    pub client_dir: EntryRole,
}

/// Everything Lampo needs to know about one kind of service. Implemented per kind (pure data,
/// built-ins in [`builtin`]) and registered in the composition roots.
pub trait ServiceDescriptor: Send + Sync {
    fn kind(&self) -> ServiceKind;
    /// For messages and the GUI (`MySQL`).
    fn display_name(&self) -> &'static str;
    /// Package providing the binaries.
    fn package_kind(&self) -> PackageKind;
    /// Instances run side by side per version series (MySQL 8.4 and 5.7); a project using the
    /// service names the series it wants.
    fn versioned(&self) -> bool;
    /// Port of an instance not listed in the settings.
    fn default_port(&self) -> Port;
    /// Keeps persistent data (`<data>\<kind>[\<series>]\data`).
    fn has_data_dir(&self) -> bool;
    /// Writes its own error log next to the captured output (`<service>-error` log source).
    fn has_error_log(&self) -> bool;
    /// Connection defaults for clients; `None` when there is no client to shim.
    fn client(&self) -> Option<ServiceClient>;
    /// Default port of the instance's second listener, its web UI (Mailpit's 8025); `None`
    /// for kinds with a single port. Configurable per instance
    /// (`ServiceInstanceSettings::ui_port`).
    fn default_ui_port(&self) -> Option<Port> {
        None
    }
    /// php.ini values every site using an instance of this kind gets, given the instance's
    /// port (Mailpit: `SMTP`/`smtp_port` for `mail()`). The project's php.ini values win.
    fn php_ini(&self, _port: Port) -> Vec<(&'static str, String)> {
        Vec::new()
    }
    /// Validates the version series a project asks for.
    fn validate_version(&self, version: Option<VersionSeries>) -> Result<(), DomainError> {
        match (self.versioned(), version) {
            (true, None) => Err(DomainError::InvalidServiceId(format!("{} needs a version (e.g. `{}-8.4`)", self.display_name(), self.kind()))),
            (false, Some(v)) => Err(DomainError::InvalidServiceId(format!("{} has no versions (got {v})", self.display_name()))),
            _ => Ok(()),
        }
    }
    /// Label of an instance (`MySQL 8.4`).
    fn label(&self, id: &ServiceId) -> String {
        match id.version() {
            Some(v) => format!("{} {v}", self.display_name()),
            None => self.display_name().to_string(),
        }
    }
}

/// The built-in service kinds.
pub mod builtin {
    use super::*;

    pub struct Apache;
    impl ServiceDescriptor for Apache {
        fn kind(&self) -> ServiceKind {
            ServiceKind::APACHE
        }
        fn display_name(&self) -> &'static str {
            "Apache"
        }
        fn package_kind(&self) -> PackageKind {
            PackageKind::APACHE
        }
        fn versioned(&self) -> bool {
            false
        }
        fn default_port(&self) -> Port {
            Port::of(80)
        }
        fn has_data_dir(&self) -> bool {
            false
        }
        fn has_error_log(&self) -> bool {
            true
        }
        fn client(&self) -> Option<ServiceClient> {
            None
        }
    }

    pub struct Mysql;
    impl ServiceDescriptor for Mysql {
        fn kind(&self) -> ServiceKind {
            ServiceKind::MYSQL
        }
        fn display_name(&self) -> &'static str {
            "MySQL"
        }
        fn package_kind(&self) -> PackageKind {
            PackageKind::MYSQL
        }
        fn versioned(&self) -> bool {
            true
        }
        fn default_port(&self) -> Port {
            Port::of(3306)
        }
        fn has_data_dir(&self) -> bool {
            true
        }
        fn has_error_log(&self) -> bool {
            true
        }
        fn client(&self) -> Option<ServiceClient> {
            Some(ServiceClient { user: Some("root"), env_prefix: "DB", client_dir: EntryRole::ClientDir })
        }
        fn validate_version(&self, version: Option<VersionSeries>) -> Result<(), DomainError> {
            let v = version.ok_or_else(|| DomainError::InvalidServiceId("MySQL needs a version (e.g. `mysql-8.4`)".into()))?;
            crate::mysql::MysqlSeries::try_from(v).map(|_| ())
        }
    }

    pub struct Redis;
    impl ServiceDescriptor for Redis {
        fn kind(&self) -> ServiceKind {
            ServiceKind::REDIS
        }
        fn display_name(&self) -> &'static str {
            "Redis"
        }
        fn package_kind(&self) -> PackageKind {
            PackageKind::REDIS
        }
        fn versioned(&self) -> bool {
            false
        }
        fn default_port(&self) -> Port {
            Port::of(6379)
        }
        fn has_data_dir(&self) -> bool {
            true
        }
        fn has_error_log(&self) -> bool {
            false
        }
        fn client(&self) -> Option<ServiceClient> {
            Some(ServiceClient { user: None, env_prefix: "REDIS", client_dir: EntryRole::ClientDir })
        }
    }

    /// Mailpit: catches every mail the sites send over SMTP and shows it in a web UI.
    pub struct Mailpit;
    impl ServiceDescriptor for Mailpit {
        fn kind(&self) -> ServiceKind {
            ServiceKind::MAILPIT
        }
        fn display_name(&self) -> &'static str {
            "Mailpit"
        }
        fn package_kind(&self) -> PackageKind {
            PackageKind::MAILPIT
        }
        fn versioned(&self) -> bool {
            false
        }
        /// SMTP.
        fn default_port(&self) -> Port {
            Port::of(1025)
        }
        fn has_data_dir(&self) -> bool {
            true
        }
        fn has_error_log(&self) -> bool {
            false
        }
        /// No client to shim: `LAMPO_MAIL_HOST`/`_PORT` for `lampo env`/`exec`, with the package
        /// folder on their PATH (`mailpit sendmail`).
        fn client(&self) -> Option<ServiceClient> {
            Some(ServiceClient { user: None, env_prefix: "MAIL", client_dir: EntryRole::ClientDir })
        }
        fn default_ui_port(&self) -> Option<Port> {
            Some(Port::of(8025))
        }
        /// Windows PHP's `mail()` speaks SMTP to `SMTP`:`smtp_port` (no auth, no TLS).
        fn php_ini(&self, port: Port) -> Vec<(&'static str, String)> {
            vec![("SMTP", "127.0.0.1".into()), ("smtp_port", port.to_string()), ("sendmail_from", "lampo@localhost".into())]
        }
    }

    /// Apache, MySQL, Redis, Mailpit.
    pub fn all() -> Vec<Arc<dyn ServiceDescriptor>> {
        vec![Arc::new(Apache), Arc::new(Mysql), Arc::new(Redis), Arc::new(Mailpit)]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ServiceState {
    Stopped,
    Starting,
    Running,
    Stopping,
    /// Crashed or failed to start; see [`ServiceStatus::message`].
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceStatus {
    pub service: ServiceId,
    pub state: ServiceState,
    pub pid: Option<u32>,
    pub port: Option<Port>,
    /// Human-readable detail (last error line, exit code…), English.
    pub message: Option<String>,
}

impl ServiceStatus {
    pub fn stopped(service: ServiceId) -> Self {
        Self { service, state: ServiceState::Stopped, pid: None, port: None, message: None }
    }
}

/// Everything a runner needs to (re)start one service instance. Resolved by the application
/// layer from settings, installed packages, the kind's descriptor and
/// [`crate::paths::LampoPaths`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceLaunch {
    pub service: ServiceId,
    /// Root of the installed package (`...\packages\mysql-8.4`).
    pub install_dir: PathBuf,
    pub port: Port,
    /// Second listener (the web UI) of kinds that have one
    /// ([`ServiceDescriptor::default_ui_port`]).
    pub ui_port: Option<Port>,
    /// Generated configuration for this instance (`httpd.conf`, `my.ini`, `redis.conf` live here).
    pub config_dir: PathBuf,
    /// Persistent data (MySQL data dir, Redis dump dir). `None` for Apache.
    pub data_dir: Option<PathBuf>,
    pub log_dir: PathBuf,
}

impl ServiceLaunch {
    /// Every port the instance listens on (all must be free before a start).
    pub fn ports(&self) -> Vec<Port> {
        std::iter::once(self.port).chain(self.ui_port).collect()
    }
}

/// Which output stream a log line came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogStream {
    Stdout,
    Stderr,
}

/// The set of services that must run, each with the sites requiring it
/// (an empty list means "required by the system", e.g. the always-on MySQL).
pub type ServicePlan = BTreeMap<ServiceId, Vec<SiteId>>;

/// Sites that are served: active projects plus every worktree environment whose parent exists
/// (worktree environments are served while they exist, regardless of the parent's flag).
pub fn served_sites<'a>(projects: &'a [Project], worktrees: &'a [WorktreeEnvironment]) -> Vec<(SiteId, &'a Project)> {
    let mut out: Vec<(SiteId, &Project)> = projects.iter().filter(|p| p.active).map(|p| (SiteId::Project(p.id.clone()), p)).collect();
    for w in worktrees {
        if let Some(parent) = projects.iter().find(|p| p.id == w.parent) {
            out.push((w.site_id(), parent));
        }
    }
    out
}

/// Computes the services that must run: the union of what served sites use
/// (`ProjectEnvironment::services`), plus every always-on instance of the settings. Apache (the
/// web tier) runs iff at least one site is served.
pub fn plan_services(projects: &[Project], worktrees: &[WorktreeEnvironment], settings: &Settings) -> ServicePlan {
    let mut plan = ServicePlan::new();
    for s in settings.services.iter().filter(|s| s.always_on) {
        plan.entry(s.service.clone()).or_default();
    }
    for (site, p) in served_sites(projects, worktrees) {
        plan.entry(ServiceId::apache()).or_default().push(site.clone());
        for id in p.environment.service_ids() {
            plan.entry(id).or_default().push(site.clone());
        }
    }
    plan
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mysql::MysqlSeries;
    use crate::project::tests::project;

    fn mysql(s: MysqlSeries) -> ServiceId {
        ServiceId::new(ServiceKind::MYSQL, Some(s.as_series()))
    }

    fn redis() -> ServiceId {
        ServiceId::new(ServiceKind::REDIS, None)
    }

    #[test]
    fn service_id_text_roundtrip() {
        for s in ["apache", "redis", "mysql-8.4", "mysql-5.7", "postgresql-17.2", "php-fpm"] {
            assert_eq!(s.parse::<ServiceId>().unwrap().to_string(), s);
        }
        assert_eq!("mysql-8.4".parse::<ServiceId>().unwrap(), mysql(MysqlSeries::DEFAULT));
        assert_eq!("php-fpm".parse::<ServiceId>().unwrap().kind().as_str(), "php-fpm");
        for bad in ["", "Nginx", "mysql-8", "-x", "mysql-8.x"] {
            assert!(bad.parse::<ServiceId>().is_err(), "{bad}");
        }
    }

    #[test]
    fn descriptors_validate_versions() {
        assert!(builtin::Mysql.validate_version(Some(MysqlSeries::DEFAULT.as_series())).is_ok());
        assert!(builtin::Mysql.validate_version(Some(VersionSeries { major: 5, minor: 6 })).is_err());
        assert!(builtin::Mysql.validate_version(None).is_err());
        assert!(builtin::Redis.validate_version(None).is_ok());
        assert!(builtin::Redis.validate_version(Some(VersionSeries { major: 7, minor: 0 })).is_err());
        assert_eq!(builtin::Mysql.label(&mysql(MysqlSeries::LEGACY_5_7)), "MySQL 5.7");
        assert!(builtin::Mailpit.validate_version(None).is_ok());
        assert!(builtin::Mailpit.validate_version(Some(VersionSeries { major: 1, minor: 33 })).is_err());
    }

    #[test]
    fn mailpit_has_smtp_and_ui_ports_and_php_mail_settings() {
        let d = builtin::Mailpit;
        assert_eq!((d.kind(), d.default_port(), d.default_ui_port()), (ServiceKind::MAILPIT, Port::of(1025), Some(Port::of(8025))));
        assert!(d.has_data_dir() && !d.versioned());
        assert_eq!(d.client().unwrap().env_prefix, "MAIL");
        assert_eq!(d.php_ini(Port::of(2025)), [("SMTP", "127.0.0.1".to_string()), ("smtp_port", "2025".to_string()), ("sendmail_from", "lampo@localhost".to_string())]);
        assert_eq!(builtin::Redis.default_ui_port(), None);
        assert!(builtin::Redis.php_ini(Port::of(6379)).is_empty());
        assert!(builtin::all().iter().any(|d| d.kind() == ServiceKind::MAILPIT));
        let launch = ServiceLaunch {
            service: ServiceId::new(ServiceKind::MAILPIT, None),
            install_dir: "C:\\m".into(),
            port: Port::of(1025),
            ui_port: Some(Port::of(8025)),
            config_dir: "C:\\c".into(),
            data_dir: None,
            log_dir: "C:\\l".into(),
        };
        assert_eq!(launch.ports(), [Port::of(1025), Port::of(8025)]);
        assert_eq!(ServiceLaunch { ui_port: None, ..launch }.ports(), [Port::of(1025)]);
    }

    #[test]
    fn idle_plan_only_runs_always_on_mysql() {
        let plan = plan_services(&[project("shop", "C:\\a")], &[], &Settings::default());
        assert_eq!(plan.keys().cloned().collect::<Vec<_>>(), vec![mysql(MysqlSeries::DEFAULT)]);
    }

    #[test]
    fn worktree_environments_are_served_even_if_parent_is_inactive() {
        use crate::worktree::{DatabaseMode, Occupancy, WorktreeEnvironment};
        let mut parent = project("shop", "C:\\a");
        parent.environment.use_service(ServiceKind::REDIS, None);
        let wt = WorktreeEnvironment::derive(
            &parent,
            &Occupancy::default(),
            crate::project::ProjectRoot::new("D:\\wt\\x").unwrap(),
            Some("x".into()),
            crate::tcp_port::Port::of(8100),
            &DatabaseMode::Shared,
            0,
        )
        .unwrap();
        let orphan = WorktreeEnvironment { parent: crate::project::ProjectId::new("gone").unwrap(), ..wt.clone() };
        let plan = plan_services(&[parent], &[wt, orphan], &Settings::default());
        assert_eq!(plan[&ServiceId::apache()].iter().map(|s| s.to_string()).collect::<Vec<_>>(), ["shop@x"]);
        assert!(plan.contains_key(&redis()));
    }

    #[test]
    fn plan_is_union_of_active_projects_including_unknown_kinds() {
        let mut shop = project("shop", "C:\\a");
        shop.active = true;
        shop.environment.use_service(ServiceKind::REDIS, None);
        shop.environment.use_service(ServiceKind::new("mailpit").unwrap(), None);
        let mut legacy = project("legacy", "C:\\b");
        legacy.active = true;
        legacy.environment.set_mysql(Some(MysqlSeries::LEGACY_5_7));
        let mut inactive = project("off", "C:\\c");
        inactive.environment.set_mysql(Some(MysqlSeries::LEGACY_5_7));

        let plan = plan_services(&[shop, legacy, inactive], &[], &Settings::default());
        let names = |id: ServiceId| plan[&id].iter().map(|p| p.to_string()).collect::<Vec<_>>();
        assert_eq!(plan.len(), 5);
        assert_eq!(plan[&ServiceId::apache()].len(), 2);
        assert_eq!(names(mysql(MysqlSeries::LEGACY_5_7)), ["legacy"]);
        assert_eq!(names(redis()), ["shop"]);
        assert_eq!(names("mailpit".parse().unwrap()), ["shop"]);
        assert_eq!(names(mysql(MysqlSeries::DEFAULT)), ["shop"]);
    }
}
