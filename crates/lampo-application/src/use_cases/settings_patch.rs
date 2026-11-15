//! Partial edits of the global settings (`lampo settings set`): Apache's port, service instance
//! ports and web UI ports (Mailpit), the default PHP and the loopback port range.
//!
//! The patch is applied to the *stored* settings under the allocator lock
//! ([`LoopbackPortAllocator::reserve`]), so it never reverts a concurrent edit the way a
//! read-modify-write of the whole [`SettingsDto`] from another process would, and its check
//! against the sites' dedicated loopback ports cannot interleave with a project save or a port
//! allocation (which check the other direction under the same lock). Then it syncs like a GUI
//! save: running services whose port changed are restarted.

use std::collections::{BTreeMap, BTreeSet};
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
        let patch = Patch::parse(&request, &self.deps.services)?;
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
                let changed = changed_ports(&current, &next, &self.deps.services);
                check_loopback_ports(&changed, &snap.projects, &snap.worktrees)?;
                self.check_free(&changed, &current)?;
                if next != current {
                    self.deps.settings.save(&next)?;
                    self.deps.events.publish(DomainEvent::SettingsChanged);
                }
                Ok((current, next, snap.projects.clone(), snap.worktrees.clone()))
            })
            .await?;
        let changes = describe_changes(&before, &after, &self.deps.services);
        let sync = match changes.is_empty() {
            true => SyncReportDto::default(),
            false => self.deps.env.sync(ctx).await.map_err(|e| AppError { message: format!("the settings were saved, but applying them failed: {}", e.message), ..e })?,
        };
        let notes = notes(&before, &after, &projects, &worktrees, &sync, &self.deps.services);
        Ok(SettingsPatchResultDto { settings: settings_to_dto(&after, &self.deps.services), changes, sync, notes })
    }

    /// New ports must be free on the OS, unless a Lampo service holds them now (a swap between
    /// instances: the sync moves them).
    fn check_free(&self, changed: &[PortChange], current: &Settings) -> AppResult<()> {
        for c in changed {
            if current.service_on_port(c.port).is_some() {
                continue;
            }
            if let PortAvailability::InUse { owner } = self.deps.port_probe.check(c.port)? {
                return Err(PortError::PortInUse { port: c.port, owner }.into());
            }
        }
        Ok(())
    }
}

/// The request, parsed before taking the lock.
struct Patch {
    apache_port: Option<u16>,
    service_ports: Vec<(ServiceId, u16)>,
    /// Web UI ports, only for kinds that have one.
    ui_ports: Vec<(ServiceId, u16)>,
    default_php: Option<PhpMinor>,
    loopback_ports: Option<PortRange>,
}

/// `(id, port)` pairs of a request map, validated: well-formed ids, no Apache, no duplicates.
fn parse_ports(ports: &BTreeMap<String, u16>, what: &str) -> AppResult<Vec<(ServiceId, u16)>> {
    let mut out: Vec<(ServiceId, u16)> = Vec::new();
    for (id, port) in ports {
        let id: ServiceId = id.trim().to_ascii_lowercase().parse()?;
        if id.is_apache() {
            return Err(validation("Apache's port is set with `apachePort` (`--apache-port`)"));
        }
        if out.iter().any(|(other, _)| other == &id) {
            return Err(validation(format!("the {what} of {id} is given twice")));
        }
        out.push((id, *port));
    }
    Ok(out)
}

impl Patch {
    fn parse(r: &SettingsPatchRequest, catalog: &ServiceCatalog) -> AppResult<Self> {
        let ui_ports = parse_ports(&r.service_ui_ports, "web UI port")?;
        for (id, _) in &ui_ports {
            if catalog.descriptor(id.kind()).and_then(|d| d.default_ui_port()).is_none() {
                return Err(validation(format!("{id} has no web UI port (only Mailpit has one)")));
            }
        }
        Ok(Self {
            apache_port: r.apache_port,
            service_ports: parse_ports(&r.service_ports, "port")?,
            ui_ports,
            default_php: r.default_php_version.as_deref().map(|v| PhpMinor::parse(v.trim())).transpose()?,
            loopback_ports: r.loopback_port_range.map(|range| PortRange::new(range.start, range.end)).transpose()?,
        })
    }

    /// `current` with the patch applied and validated (`Settings::validate`: distinct, non-zero
    /// ports, web UI ports included).
    fn apply(&self, current: &Settings) -> AppResult<Settings> {
        let mut next = current.clone();
        if let Some(port) = self.apache_port {
            next.apache_port = port;
        }
        let ports = self.service_ports.iter().map(|(id, port)| (id, port, false)).chain(self.ui_ports.iter().map(|(id, port)| (id, port, true)));
        for (id, port, ui) in ports {
            let Some(instance) = next.services.iter_mut().find(|s| &s.service == id) else {
                let configured: Vec<String> = current.services.iter().map(|s| s.service.to_string()).collect();
                return Err(validation(format!("{id} is not a configured service instance (configured: {})", configured.join(", "))));
            };
            match ui {
                false => instance.port = *port,
                true => instance.ui_port = Some(*port),
            }
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

/// One port that differs between two settings: a service's main port, or its web UI port.
struct PortChange {
    id: ServiceId,
    ui: bool,
    port: u16,
}

impl PortChange {
    /// `apache.port`, `mysql-8.4.port`, `mailpit.ui-port`.
    fn setting(&self) -> String {
        format!("{}.{}", self.id, if self.ui { "ui-port" } else { "port" })
    }

    fn previous(&self, before: &Settings, catalog: &ServiceCatalog) -> Option<u16> {
        match self.ui {
            false => before.service_port(&self.id),
            true => catalog.ui_port(&self.id, before),
        }
    }
}

/// Every port (Apache's, services', web UIs') that differs between `before` and `after`, with
/// its new value. A web UI port is compared by its effective value (configured, else default).
fn changed_ports(before: &Settings, after: &Settings, catalog: &ServiceCatalog) -> Vec<PortChange> {
    let mut out = Vec::new();
    if before.apache_port != after.apache_port {
        out.push(PortChange { id: ServiceId::apache(), ui: false, port: after.apache_port });
    }
    for s in &after.services {
        if before.service_port(&s.service) != Some(s.port) {
            out.push(PortChange { id: s.service.clone(), ui: false, port: s.port });
        }
        if let Some(ui) = catalog.ui_port(&s.service, after).filter(|ui| catalog.ui_port(&s.service, before) != Some(*ui)) {
            out.push(PortChange { id: s.service.clone(), ui: true, port: ui });
        }
    }
    out
}

/// A service port may not be a site's dedicated loopback port (Apache listens there for it).
fn check_loopback_ports(changed: &[PortChange], projects: &[Project], worktrees: &[WorktreeEnvironment]) -> AppResult<()> {
    for port in changed.iter().map(|c| c.port) {
        let project = projects.iter().find(|p| p.addresses.iter().any(|a| a.loopback() == Some(port))).map(|p| p.id.to_string());
        let site = project.or_else(|| worktrees.iter().find(|w| w.loopback_port == port).map(|w| w.site_id().to_string()));
        if let Some(site) = site {
            return Err(DomainError::LoopbackPortTaken { port, service: format!("site `{site}`") }.into());
        }
    }
    Ok(())
}

fn describe_changes(before: &Settings, after: &Settings, catalog: &ServiceCatalog) -> Vec<SettingChangeDto> {
    let change = |setting: String, from: String, to: String| SettingChangeDto { setting, from, to };
    let mut out: Vec<SettingChangeDto> =
        changed_ports(before, after, catalog).into_iter().map(|c| change(c.setting(), c.previous(before, catalog).map(|p| p.to_string()).unwrap_or_default(), c.port.to_string())).collect();
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
    for c in changed_ports(before, after, catalog) {
        let (id, port) = (&c.id, c.port);
        let label = catalog.label(id);
        let name = id.to_string();
        if c.ui {
            out.push(match applied.contains(name.as_str()) {
                true => format!("The {label} web UI is now at http://127.0.0.1:{port}."),
                false => format!("{label} is not running: its web UI listens on port {port} when it next starts."),
            });
            continue;
        }
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
        let mut users: Vec<String> = projects.iter().filter(|p| p.environment.service_ids().any(|s| &s == id)).map(|p| p.id.to_string()).collect();
        users.extend(worktrees.iter().filter(|w| projects.iter().any(|p| p.id == w.parent && p.environment.service_ids().any(|s| &s == id))).map(|w| w.site_id().to_string()));
        if !users.is_empty() {
            let variable = match id.kind() {
                k if k == &ServiceKind::MYSQL => "DB_PORT",
                k if k == &ServiceKind::MAILPIT => "MAIL_PORT (php.ini's smtp_port follows by itself)",
                _ => "the port setting (REDIS_PORT)",
            };
            out.push(format!("Sites using {label}: {}. Update {variable} to {port} in their .env files; `lampo agent context` shows the new connection.", users.join(", ")));
        }
    }
    out
}
