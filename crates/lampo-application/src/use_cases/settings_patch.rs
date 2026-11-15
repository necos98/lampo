//! Partial edits of the global settings (`lampo settings set`): Apache's port, service instance
//! ports, the default PHP and the loopback port range.
//!
//! The patch is applied to the *stored* settings under the allocator lock
//! ([`LoopbackPortAllocator::reserve`]), so it never reverts a concurrent edit the way a
//! read-modify-write of the whole [`SettingsDto`] from another process would, and its check
//! against the sites' dedicated loopback ports cannot interleave with a project save or a port
//! allocation (which check the other direction under the same lock). Then it syncs like a GUI
//! save: running services whose port changed are restarted.

use std::collections::BTreeSet;
use std::sync::Arc;

use lampo_domain::DomainError;
use lampo_domain::events::DomainEvent;
use lampo_domain::php::PhpMinor;
use lampo_domain::PortError;
use lampo_domain::ports::{EventPublisher, InstalledPackageRegistry, PortAvailability, PortProbe, SettingsRepository};
use lampo_domain::project::Project;
use lampo_domain::service::{ServiceId, ServiceKind};
use lampo_domain::settings::Settings;
use lampo_domain::worktree::{PortRange, WorktreeEnvironment};

use super::allocation::LoopbackPortAllocator;
use super::environment::EnvironmentSync;
use super::service_catalog::ServiceCatalog;
use super::support::{php_package, validation};
use crate::context::CallContext;
use crate::dto::*;
use crate::error::{AppError, AppResult, ErrorCode};
use crate::mapping::settings_to_dto;

pub struct PatchSettingsDeps {
    pub settings: Arc<dyn SettingsRepository>,
    pub allocator: Arc<LoopbackPortAllocator>,
    pub port_probe: Arc<dyn PortProbe>,
    pub installed: Arc<dyn InstalledPackageRegistry>,
    pub env: Arc<dyn EnvironmentSync>,
    pub events: Arc<dyn EventPublisher>,
    pub services: Arc<ServiceCatalog>,
}

/// Applies a [`SettingsPatchRequest`].
pub struct PatchSettings {
    deps: PatchSettingsDeps,
}

impl PatchSettings {
    pub fn new(deps: PatchSettingsDeps) -> Self {
        Self { deps }
    }

    /// `Validation` for an empty patch, a malformed or unconfigured service id, Apache given as
    /// a service, a bad port or range; `Conflict` for a port another Lampo service or a site's
    /// dedicated loopback port already has; `PortInUse` for a new port another program listens
    /// on; `PackageMissing` for a default PHP that is not installed. Nothing is saved on error.
    /// When something changed: saves, publishes `SettingsChanged`, syncs (a sync failure is
    /// returned with a message saying the settings were saved).
    pub async fn execute(&self, request: SettingsPatchRequest, ctx: &CallContext) -> AppResult<SettingsPatchResultDto> {
        if request.is_empty() {
            return Err(validation("nothing to change: give at least one setting"));
        }
        let patch = Patch::parse(&request)?;
        let installed = match patch.default_php {
            Some(_) => self.deps.installed.list()?,
            None => Vec::new(),
        };
        let (before, after, projects, worktrees) = self
            .deps
            .allocator
            .reserve(|snap| {
                let current = snap.settings.clone();
                let next = patch.apply(&current)?;
                if let Some(php) = patch.default_php.filter(|p| Some(*p) != current.default_php) {
                    if php_package(&installed, php).is_none() {
                        return Err(AppError::new(ErrorCode::PackageMissing, format!("PHP {php} is not installed: run `lampo package install php {php}` first")));
                    }
                }
                let changed = changed_ports(&current, &next);
                check_loopback_ports(&changed, &snap.projects, &snap.worktrees)?;
                self.check_free(&changed, &current)?;
                if next != current {
                    self.deps.settings.save(&next)?;
                    self.deps.events.publish(DomainEvent::SettingsChanged);
                }
                Ok((current, next, snap.projects.clone(), snap.worktrees.clone()))
            })
            .await?;
        let changes = describe_changes(&before, &after);
        let sync = match changes.is_empty() {
            true => SyncReportDto::default(),
            false => self.deps.env.sync(ctx).await.map_err(|e| AppError { message: format!("the settings were saved, but applying them failed: {}", e.message), ..e })?,
        };
        let notes = notes(&before, &after, &projects, &worktrees, &sync, &self.deps.services);
        Ok(SettingsPatchResultDto { settings: settings_to_dto(&after, &self.deps.services), changes, sync, notes })
    }

    /// New ports must be free on the OS, unless a Lampo service holds them now (a swap between
    /// instances: the sync moves them).
    fn check_free(&self, changed: &[(ServiceId, u16)], current: &Settings) -> AppResult<()> {
        for (_, port) in changed {
            if current.service_on_port(*port).is_some() {
                continue;
            }
            if let PortAvailability::InUse { owner } = self.deps.port_probe.check(*port)? {
                return Err(PortError::PortInUse { port: *port, owner }.into());
            }
        }
        Ok(())
    }
}

/// The request, parsed before taking the lock.
struct Patch {
    apache_port: Option<u16>,
    service_ports: Vec<(ServiceId, u16)>,
    default_php: Option<PhpMinor>,
    loopback_ports: Option<PortRange>,
}

impl Patch {
    fn parse(r: &SettingsPatchRequest) -> AppResult<Self> {
        let mut service_ports = Vec::new();
        for (id, port) in &r.service_ports {
            let id: ServiceId = id.trim().to_ascii_lowercase().parse()?;
            if id.is_apache() {
                return Err(validation("Apache's port is set with `apachePort` (`--apache-port`)"));
            }
            if service_ports.iter().any(|(other, _)| other == &id) {
                return Err(validation(format!("the port of {id} is given twice")));
            }
            service_ports.push((id, *port));
        }
        Ok(Self {
            apache_port: r.apache_port,
            service_ports,
            default_php: r.default_php_version.as_deref().map(|v| PhpMinor::parse(v.trim())).transpose()?,
            loopback_ports: r.loopback_port_range.map(|range| PortRange::new(range.start, range.end)).transpose()?,
        })
    }

    /// `current` with the patch applied and validated (`Settings::validate`: distinct, non-zero
    /// ports).
    fn apply(&self, current: &Settings) -> AppResult<Settings> {
        let mut next = current.clone();
        if let Some(port) = self.apache_port {
            next.apache_port = port;
        }
        for (id, port) in &self.service_ports {
            let Some(instance) = next.services.iter_mut().find(|s| &s.service == id) else {
                let configured: Vec<String> = current.services.iter().map(|s| s.service.to_string()).collect();
                return Err(validation(format!("{id} is not a configured service instance (configured: {})", configured.join(", "))));
            };
            instance.port = *port;
        }
        if let Some(php) = self.default_php {
            next.default_php = Some(php);
        }
        if let Some(range) = self.loopback_ports {
            next.loopback_ports = range;
        }
        next.validate()?;
        Ok(next)
    }
}

/// Every service (Apache included) whose port differs between `before` and `after`, with its
/// new port.
fn changed_ports(before: &Settings, after: &Settings) -> Vec<(ServiceId, u16)> {
    let mut out = Vec::new();
    if before.apache_port != after.apache_port {
        out.push((ServiceId::apache(), after.apache_port));
    }
    for s in &after.services {
        if before.service_port(&s.service) != Some(s.port) {
            out.push((s.service.clone(), s.port));
        }
    }
    out
}

/// A service port may not be a site's dedicated loopback port (Apache listens there for it).
fn check_loopback_ports(changed: &[(ServiceId, u16)], projects: &[Project], worktrees: &[WorktreeEnvironment]) -> AppResult<()> {
    for (_, port) in changed {
        let project = projects.iter().find(|p| p.addresses.iter().any(|a| a.loopback() == Some(*port))).map(|p| p.id.to_string());
        let site = project.or_else(|| worktrees.iter().find(|w| w.loopback_port == *port).map(|w| w.site_id().to_string()));
        if let Some(site) = site {
            return Err(DomainError::LoopbackPortTaken { port: *port, service: format!("site `{site}`") }.into());
        }
    }
    Ok(())
}

fn describe_changes(before: &Settings, after: &Settings) -> Vec<SettingChangeDto> {
    let change = |setting: String, from: String, to: String| SettingChangeDto { setting, from, to };
    let mut out: Vec<SettingChangeDto> = changed_ports(before, after)
        .into_iter()
        .map(|(id, port)| change(format!("{id}.port"), before.service_port(&id).map(|p| p.to_string()).unwrap_or_default(), port.to_string()))
        .collect();
    if before.default_php != after.default_php {
        let show = |p: Option<PhpMinor>| p.map(|p| p.to_string()).unwrap_or_else(|| "none".into());
        out.push(change("default-php".into(), show(before.default_php), show(after.default_php)));
    }
    if before.loopback_ports != after.loopback_ports {
        let show = |r: PortRange| format!("{}-{}", r.start, r.end);
        out.push(change("loopback-port-range".into(), show(before.loopback_ports), show(after.loopback_ports)));
    }
    out
}

/// What the caller still has to know or do after a port change.
fn notes(before: &Settings, after: &Settings, projects: &[Project], worktrees: &[WorktreeEnvironment], sync: &SyncReportDto, catalog: &ServiceCatalog) -> Vec<String> {
    let mut out = Vec::new();
    let applied: BTreeSet<&str> = sync.restarted.iter().chain(&sync.started).map(String::as_str).collect();
    for (id, port) in changed_ports(before, after) {
        let label = catalog.label(&id);
        let name = id.to_string();
        if !applied.contains(name.as_str()) {
            out.push(format!("{label} is not running: it listens on port {port} when it next starts."));
        }
        if id.is_apache() {
            let hosted: Vec<String> = projects.iter().filter(|p| p.addresses.iter().any(|a| a.loopback().is_none())).map(|p| p.id.to_string()).collect();
            if !hosted.is_empty() {
                out.push(format!(
                    "Host-name URLs now use port {port} (projects {}); dedicated loopback-port URLs are unchanged. Update APP_URL-style values that include the old port.",
                    hosted.join(", ")
                ));
            }
            continue;
        }
        let mut users: Vec<String> = projects.iter().filter(|p| p.environment.service_ids().any(|s| s == id)).map(|p| p.id.to_string()).collect();
        users.extend(worktrees.iter().filter(|w| projects.iter().any(|p| p.id == w.parent && p.environment.service_ids().any(|s| s == id))).map(|w| w.site_id().to_string()));
        if !users.is_empty() {
            let variable = if id.kind() == &ServiceKind::MYSQL { "DB_PORT" } else { "the port setting (REDIS_PORT)" };
            out.push(format!("Sites using {label}: {}. Update {variable} to {port} in their .env files; `lampo agent context` shows the new connection.", users.join(", ")));
        }
    }
    out
}
