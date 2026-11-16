//! The registered service kinds ([`ServiceDescriptor`]s, given by the composition roots) and
//! every rule that depends on them: which services are configured, their ports, how to launch
//! an instance, which log sources and client defaults they offer. The rest of the application
//! asks this catalog instead of matching on kinds.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use lampo_domain::package::InstalledPackage;
use lampo_domain::paths::LampoPaths;
use lampo_domain::project::{Project, ProjectEnvironment};
use lampo_domain::service::{ServiceDescriptor, ServiceId, ServiceKind, ServiceLaunch, plan_services};
use lampo_domain::settings::Settings;
use lampo_domain::shim::ShimService;
use lampo_domain::worktree::WorktreeEnvironment;
use lampo_domain::DomainError;

use super::support::{newest_of, package_missing};
use crate::error::{AppError, AppResult, ErrorCode};

pub struct ServiceCatalog {
    descriptors: Vec<Arc<dyn ServiceDescriptor>>,
}

impl ServiceCatalog {
    /// `descriptors` is `lampo_domain::service::builtin::all()` (plus any extra kind) in
    /// production; one per kind.
    pub fn new(descriptors: Vec<Arc<dyn ServiceDescriptor>>) -> Self {
        Self { descriptors }
    }

    pub fn descriptor(&self, kind: &ServiceKind) -> Option<&dyn ServiceDescriptor> {
        self.descriptors.iter().find(|d| &d.kind() == kind).map(|d| d.as_ref())
    }

    fn require(&self, kind: &ServiceKind) -> AppResult<&dyn ServiceDescriptor> {
        self.descriptor(kind).ok_or_else(|| AppError::new(ErrorCode::Validation, format!("unknown service kind `{kind}`")))
    }

    /// Registered kinds, in registration order.
    pub fn descriptors(&self) -> &[Arc<dyn ServiceDescriptor>] {
        &self.descriptors
    }

    /// Every configured service, the one definition used by sync, shutdown, `lampo down`,
    /// status listings and logs: Apache first, then each instance of the settings, then every
    /// registered unversioned kind the settings do not list (it runs on its default port).
    pub fn configured(&self, settings: &Settings) -> Vec<ServiceId> {
        let mut ids = vec![ServiceId::apache()];
        ids.extend(settings.services.iter().map(|s| s.service.clone()));
        for d in self.descriptors.iter().filter(|d| !d.versioned()) {
            let id = ServiceId::new(d.kind(), None);
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        ids
    }

    /// Port of `id`: the configured one, else the default of an unversioned kind.
    pub fn port(&self, id: &ServiceId, settings: &Settings) -> Option<u16> {
        settings.service_port(id).or_else(|| self.descriptor(id.kind()).filter(|d| !d.versioned()).map(|d| d.default_port()))
    }

    /// Web UI port of `id` for kinds with a second listener (Mailpit): the configured one,
    /// else the kind's default. `None` for single-port kinds.
    pub fn ui_port(&self, id: &ServiceId, settings: &Settings) -> Option<u16> {
        let default = self.descriptor(id.kind())?.default_ui_port()?;
        Some(settings.service_ui_port(id).unwrap_or(default))
    }

    /// `http://127.0.0.1:<ui port>` for kinds with a web UI.
    pub fn ui_url(&self, id: &ServiceId, settings: &Settings) -> Option<String> {
        self.ui_port(id, settings).map(|p| format!("http://127.0.0.1:{p}"))
    }

    /// The php.ini values the services `env` uses contribute (Mailpit's `SMTP`/`smtp_port`),
    /// in service order; the project's own php.ini values are merged on top by the caller.
    pub fn php_ini(&self, env: &ProjectEnvironment, settings: &Settings) -> Vec<(&'static str, String)> {
        env.service_ids().filter_map(|id| Some((self.descriptor(id.kind())?, self.port(&id, settings)?))).flat_map(|(d, port)| d.php_ini(port)).collect()
    }

    /// `MySQL 8.4`; the id itself for unknown kinds.
    pub fn label(&self, id: &ServiceId) -> String {
        self.descriptor(id.kind()).map(|d| d.label(id)).unwrap_or_else(|| id.to_string())
    }

    /// Ports of the services the configuration plans to run, with the service id text.
    pub fn planned_ports(&self, projects: &[Project], worktrees: &[WorktreeEnvironment], settings: &Settings) -> Vec<(String, u16)> {
        let mut out = Vec::new();
        for id in plan_services(projects, worktrees, settings).keys() {
            out.extend(self.port(id, settings).map(|p| (id.to_string(), p)));
            out.extend(self.ui_port(id, settings).map(|p| (format!("{id} (web UI)"), p)));
        }
        out
    }

    /// Rejects services of unknown kinds and versions the kind does not accept.
    pub fn validate_environment(&self, env: &ProjectEnvironment) -> AppResult<()> {
        for id in env.service_ids() {
            if id.is_apache() {
                return Err(AppError::new(ErrorCode::Validation, "Apache serves every site: it is not a per-project service"));
            }
            self.require(id.kind())?.validate_version(id.version())?;
        }
        Ok(())
    }

    /// Everything a runner needs for `service` (`PackageMissing` if not installed).
    pub fn launch(&self, service: &ServiceId, settings: &Settings, installed: &[InstalledPackage], paths: &LampoPaths) -> AppResult<ServiceLaunch> {
        let d = self.require(service.kind())?;
        let kind = d.package_kind();
        let package =
            newest_of(installed, &kind, service.version(), None).ok_or_else(|| package_missing(service.version().map(|s| format!("{kind} {s}")).unwrap_or_else(|| kind.to_string())))?;
        let port = self.port(service, settings).ok_or_else(|| DomainError::InvalidSettings(format!("{service} has no configured port")))?;
        Ok(ServiceLaunch {
            service: service.clone(),
            install_dir: package.install_dir.clone(),
            port,
            ui_port: self.ui_port(service, settings),
            config_dir: paths.service_config_dir(service),
            data_dir: d.has_data_dir().then(|| paths.service_data_dir(service)),
            log_dir: paths.service_log_dir(service),
        })
    }

    /// The services among `ids` that keep their own error log.
    pub fn with_error_log(&self, ids: impl IntoIterator<Item = ServiceId>) -> Vec<ServiceId> {
        ids.into_iter().filter(|id| self.descriptor(id.kind()).is_some_and(|d| d.has_error_log())).collect()
    }

    /// The default instance of `kind` for sites that do not use it: the first always-on
    /// configured instance, else the first configured one.
    pub(crate) fn default_instance(&self, kind: &ServiceKind, settings: &Settings) -> Option<ServiceId> {
        let of_kind = || settings.services.iter().filter(|s| s.service.kind() == kind);
        of_kind()
            .find(|s| s.always_on)
            .or_else(|| of_kind().next())
            .map(|s| s.service.clone())
            .or_else(|| self.descriptor(kind).filter(|d| !d.versioned()).map(|_| ServiceId::new(kind.clone(), None)))
    }

    /// Client defaults per kind for a target: the site's instance of each kind it uses (with
    /// `database` for MySQL), the default instance of the other kinds, each with its data dir
    /// when it keeps one. Only installed instances with a client are listed.
    pub fn shim_services(
        &self,
        env: Option<&ProjectEnvironment>,
        database: Option<String>,
        settings: &Settings,
        installed: &[InstalledPackage],
        paths: &LampoPaths,
    ) -> BTreeMap<ServiceKind, ShimService> {
        let mut out = BTreeMap::new();
        for d in &self.descriptors {
            let Some(client) = d.client() else { continue };
            let kind = d.kind();
            let site_instance = env.and_then(|e| e.services.get(&kind).map(|u| ServiceId::new(kind.clone(), u.version)));
            let uses = site_instance.is_some();
            let Some(id) = site_instance.or_else(|| self.default_instance(&kind, settings)) else { continue };
            let Some(port) = self.port(&id, settings) else { continue };
            let Some(client_dir) = newest_of(installed, &d.package_kind(), id.version(), None).and_then(|p| p.entry(client.client_dir)).map(Path::to_path_buf) else { continue };
            let database = if uses && kind == ServiceKind::MYSQL { database.clone() } else { None };
            let data_dir = d.has_data_dir().then(|| paths.service_data_dir(&id));
            let user = client.user.map(str::to_string);
            out.insert(kind, ShimService { service: id, client_dir, host: "127.0.0.1".into(), port, user, database, data_dir, env_prefix: client.env_prefix.to_string() });
        }
        out
    }
}
