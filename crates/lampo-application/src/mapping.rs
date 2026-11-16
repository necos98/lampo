//! Conversions between domain types and DTOs. DTO → domain conversions validate (they are the
//! boundary where untrusted input becomes domain values).

use std::collections::BTreeSet;

use lampo_domain::address::SiteAddress;
use lampo_domain::hostname::{Hostname, Tld};
use lampo_domain::mysql::DatabaseConnection;
use lampo_domain::package::{PackageId, PackageKind};
use lampo_domain::php::{ExtensionName, PhpMinor, php_ini_overrides};
use lampo_domain::project::{ApacheDirectives, ProjectEnvironment, PublicDir, TlsMode};
use lampo_domain::service::{ServiceId, ServiceKind};
use lampo_domain::settings::{MailConnection, ServiceInstanceSettings, Settings};
use lampo_domain::version::{PackageVersion, VersionSeries};
use lampo_domain::tcp_port::{Port, PortRange};
use lampo_domain::worktree::DatabaseMode;
use lampo_domain::DomainError;

use crate::dto::*;
use crate::use_cases::ServiceCatalog;

pub fn environment_to_dto(env: &ProjectEnvironment) -> ProjectEnvironmentDto {
    ProjectEnvironmentDto {
        php_version: env.php.to_string(),
        extensions: env.extensions.iter().map(|e| e.to_string()).collect(),
        addons: env.addons.iter().map(|a| a.to_string()).collect(),
        php_ini: env.php_ini.iter().map(|(k, v)| (k.to_string(), v.as_str().to_string())).collect(),
        public_dir: env.public_dir.as_str().to_string(),
        services: env.services.iter().map(|(kind, u)| ServiceUseDto { kind: kind.to_string(), version: u.version.map(|v| v.to_string()) }).collect(),
        apache_directives: env.apache_directives.as_str().to_string(),
    }
}

/// Parses the DTO; the service kinds and versions are checked against the registered kinds
/// by `ServiceCatalog::validate_environment` (the use cases call it).
pub fn environment_from_dto(dto: &ProjectEnvironmentDto) -> Result<ProjectEnvironment, DomainError> {
    let mut env = ProjectEnvironment {
        php: PhpMinor::parse(&dto.php_version)?,
        extensions: dto.extensions.iter().map(ExtensionName::new).collect::<Result<BTreeSet<_>, _>>()?,
        addons: dto.addons.iter().map(|a| PackageKind::new(a.clone())).collect::<Result<BTreeSet<_>, _>>()?,
        php_ini: php_ini_overrides(dto.php_ini.iter().map(|(k, v)| (k.as_str(), v.clone())))?,
        public_dir: PublicDir::new(&dto.public_dir)?,
        services: Default::default(),
        apache_directives: ApacheDirectives::new(dto.apache_directives.clone())?,
    };
    for s in &dto.services {
        env.use_service(ServiceKind::new(s.kind.trim())?, s.version.as_deref().map(VersionSeries::parse).transpose()?);
    }
    Ok(env)
}

pub fn address_to_dto(a: &SiteAddress) -> SiteAddressDto {
    match a {
        SiteAddress::Hostname(h) => SiteAddressDto::Hostname { host: h.to_string() },
        SiteAddress::Localhost(h) => SiteAddressDto::Localhost { host: h.to_string() },
        SiteAddress::LoopbackPort(p) => SiteAddressDto::LoopbackPort { port: p.get() },
    }
}

pub fn address_from_dto(dto: &SiteAddressDto) -> Result<SiteAddress, DomainError> {
    match dto {
        SiteAddressDto::Hostname { host } => SiteAddress::hostname(Hostname::new(host)?),
        SiteAddressDto::Localhost { host } => {
            let label = host.trim().trim_end_matches('.').strip_suffix(".localhost").unwrap_or(host.trim());
            SiteAddress::localhost(label)
        }
        SiteAddressDto::LoopbackPort { port } => SiteAddress::loopback_port(Port::try_from(*port)?),
    }
}

pub fn tls_to_dto(t: TlsMode) -> TlsModeDto {
    match t {
        TlsMode::Disabled => TlsModeDto::Disabled,
        TlsMode::LocalCa => TlsModeDto::LocalCa,
    }
}

pub fn connection_to_dto(c: &DatabaseConnection) -> DatabaseConnectionDto {
    DatabaseConnectionDto {
        mysql_version: c.series.to_string(),
        host: c.host.clone(),
        port: c.port.get(),
        user: c.user.clone(),
        password: c.password.clone(),
        database: c.database.to_string(),
    }
}

pub fn mail_to_dto(m: &MailConnection) -> MailConnectionDto {
    MailConnectionDto { host: m.host.clone(), smtp_port: m.smtp_port.get(), web_url: m.web_url() }
}

pub fn package_id_to_dto(id: &PackageId) -> PackageIdDto {
    PackageIdDto { kind: id.kind.to_string(), version: id.version.to_string(), php_version: id.php.map(|p| p.to_string()) }
}

pub fn package_id_from_dto(dto: &PackageIdDto) -> Result<PackageId, DomainError> {
    Ok(PackageId { kind: PackageKind::new(dto.kind.clone())?, version: PackageVersion::parse(&dto.version)?, php: dto.php_version.as_deref().map(PhpMinor::parse).transpose()? })
}

pub fn database_mode_from_dto(dto: &DatabaseModeDto) -> Result<DatabaseMode, DomainError> {
    Ok(match dto {
        DatabaseModeDto::Shared => DatabaseMode::Shared,
        DatabaseModeDto::Fresh => DatabaseMode::Fresh,
        DatabaseModeDto::Clone { source } => DatabaseMode::Clone { source: lampo_domain::mysql::DatabaseName::new(source)? },
    })
}

pub fn settings_to_dto(s: &Settings, catalog: &ServiceCatalog) -> SettingsDto {
    SettingsDto {
        default_php_version: s.default_php.map(|p| p.to_string()),
        tld: s.tld.to_string(),
        apache_port: s.apache_port.get(),
        services: s
            .services
            .iter()
            .map(|i| ServiceInstanceDto {
                service: i.service.to_string(),
                label: catalog.label(&i.service),
                port: i.port.get(),
                always_on: i.always_on,
                ui_port: catalog.ui_port(&i.service, s).map(Port::get),
            })
            .collect(),
        loopback_port_range: PortRangeDto { start: s.loopback_ports.start().get(), end: s.loopback_ports.end().get() },
        close_to_tray: s.close_to_tray,
        first_run_completed: s.first_run_completed,
    }
}

/// Validates and converts. Not user-editable, so taken from `current`: `first_run_completed` and
/// which service instances exist (see [`service_instances_from_dto`]).
pub fn settings_from_dto(dto: &SettingsDto, current: &Settings) -> Result<Settings, DomainError> {
    let s = Settings {
        default_php: dto.default_php_version.as_deref().map(PhpMinor::parse).transpose()?,
        tld: Tld::new(&dto.tld)?,
        apache_port: Port::try_from(dto.apache_port)?,
        services: service_instances_from_dto(&dto.services, &current.services)?,
        loopback_ports: PortRange::new(dto.loopback_port_range.start, dto.loopback_port_range.end)?,
        close_to_tray: dto.close_to_tray,
        first_run_completed: current.first_run_completed,
    };
    s.validate()?;
    Ok(s)
}

/// The edited port, always-on flag and web UI port of each instance of `current`, in its order.
/// The DTO must list exactly the configured instances: one added, missing (an outdated form
/// would otherwise drop a MySQL instance projects use) or given twice is refused.
fn service_instances_from_dto(dtos: &[ServiceInstanceDto], current: &[ServiceInstanceSettings]) -> Result<Vec<ServiceInstanceSettings>, DomainError> {
    let mut edited: Vec<(ServiceId, &ServiceInstanceDto)> = Vec::with_capacity(dtos.len());
    for dto in dtos {
        let id: ServiceId = dto.service.trim().parse()?;
        if edited.iter().any(|(other, _)| other == &id) {
            return Err(DomainError::InvalidSettings(format!("the service instance {id} is given twice")));
        }
        if !current.iter().any(|c| c.service == id) {
            return Err(DomainError::InvalidSettings(format!("{id} is not a configured service instance: instances cannot be added here")));
        }
        edited.push((id, dto));
    }
    current
        .iter()
        .map(|c| match edited.iter().find(|(id, _)| id == &c.service) {
            Some((_, dto)) => {
                Ok(ServiceInstanceSettings { service: c.service.clone(), port: Port::try_from(dto.port)?, always_on: dto.always_on, ui_port: dto.ui_port.map(Port::try_from).transpose()? })
            }
            None => Err(DomainError::InvalidSettings(format!("the service instance {} is missing: configured instances cannot be removed here", c.service))),
        })
        .collect()
}

/// The registered kinds a project may use (Apache, the web tier, excluded), with the configured
/// versions of versioned kinds.
pub fn service_kinds_to_dto(catalog: &ServiceCatalog, settings: &Settings) -> Vec<ServiceKindDto> {
    catalog
        .descriptors()
        .iter()
        .filter(|d| d.kind() != ServiceKind::APACHE)
        .map(|d| {
            let kind = d.kind();
            let instances: Vec<_> = settings.services.iter().filter(|s| s.service.kind() == &kind).collect();
            ServiceKindDto {
                kind: kind.to_string(),
                label: d.display_name().to_string(),
                versioned: d.versioned(),
                versions: instances.iter().filter_map(|s| s.service.version()).map(|v| v.to_string()).collect(),
                default_version: instances.iter().find(|s| s.always_on).or(instances.first()).and_then(|s| s.service.version()).map(|v| v.to_string()),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use lampo_domain::mysql::MysqlSeries;
    use lampo_domain::service::builtin;
    use std::sync::Arc;

    fn catalog() -> ServiceCatalog {
        ServiceCatalog::new(builtin::all())
    }

    #[test]
    fn environment_roundtrip() {
        let mut env = ProjectEnvironment::minimal(PhpMinor::parse("8.3").unwrap());
        env.extensions.insert(ExtensionName::new("intl").unwrap());
        env.set_mysql(Some(MysqlSeries::DEFAULT));
        env.use_service(ServiceKind::REDIS, None);
        let dto = environment_to_dto(&env);
        assert_eq!(dto.php_version, "8.3");
        assert_eq!(dto.services, [ServiceUseDto { kind: "mysql".into(), version: Some("8.4".into()) }, ServiceUseDto { kind: "redis".into(), version: None }]);
        assert_eq!(environment_from_dto(&dto).unwrap(), env);
    }

    #[test]
    fn settings_roundtrip_and_validation() {
        let s = Settings::default();
        let mut dto = settings_to_dto(&s, &catalog());
        assert_eq!(dto.services[0].label, "MySQL 8.4");
        assert_eq!(settings_from_dto(&dto, &s).unwrap(), s);
        dto.services[2].port = 3306;
        assert!(settings_from_dto(&dto, &s).is_err());
    }

    #[test]
    fn service_kinds_come_from_the_registered_descriptors() {
        let kinds = service_kinds_to_dto(&catalog(), &Settings::default());
        assert_eq!(kinds.iter().map(|k| (k.kind.as_str(), k.versioned)).collect::<Vec<_>>(), [("mysql", true), ("redis", false), ("mailpit", false)]);
        assert_eq!(kinds[0].versions, ["8.4", "5.7"]);
        assert_eq!(kinds[0].default_version.as_deref(), Some("8.4"));
        let _ = Arc::new(0);
    }

    #[test]
    fn localhost_dto_accepts_full_host() {
        let a = address_from_dto(&SiteAddressDto::Localhost { host: "shop.localhost".into() }).unwrap();
        assert_eq!(a.to_string(), "shop.localhost");
    }
}
