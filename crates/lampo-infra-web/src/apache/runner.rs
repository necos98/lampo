use std::sync::Arc;

use async_trait::async_trait;
use lampo_domain::PortResult;
use lampo_domain::ports::{EventPublisher, ProcessSupervisor, ServiceRunner};
use lampo_domain::service::{ServiceId, ServiceKind, ServiceLaunch, ServiceStatus};
use lampo_infra_fs::ensure_dir;
use lampo_infra_services::ServiceHost;

use super::driver::{ApacheDriver, check_installation};
use super::signal::{HttpdSignals, NamedEventSignals};

/// Runs `httpd.exe -f <config_dir>\httpd.conf` in the foreground under the supervisor (no
/// Windows service), on the shared [`ServiceHost`] sequence with an [`ApacheDriver`]. Stop and
/// restart are graceful through httpd's named events (see [`super::signal`]), with the
/// supervisor's kill as fallback. Ready means every port of the live configuration's `Listen`
/// directives is listened on by httpd's own process tree (not by another server that took a
/// port). Publishes status/log events.
pub struct ApacheRunner {
    host: ServiceHost,
    driver: ApacheDriver,
}

impl ApacheRunner {
    pub fn new(supervisor: Arc<dyn ProcessSupervisor>, events: Arc<dyn EventPublisher>) -> Self {
        Self::with_signals(supervisor, events, Arc::new(NamedEventSignals))
    }

    pub fn with_signals(supervisor: Arc<dyn ProcessSupervisor>, events: Arc<dyn EventPublisher>, signals: Arc<dyn HttpdSignals>) -> Self {
        Self { host: ServiceHost::new(supervisor, events), driver: ApacheDriver::new(signals) }
    }
}

#[async_trait]
impl ServiceRunner for ApacheRunner {
    fn kind(&self) -> ServiceKind {
        ServiceKind::APACHE
    }

    /// Checks the installation and the generated configuration, creates the log folder.
    async fn prepare(&self, launch: &ServiceLaunch) -> PortResult<()> {
        check_installation(launch)?;
        ensure_dir(&launch.log_dir)
    }

    async fn start(&self, launch: &ServiceLaunch) -> PortResult<()> {
        self.host.start(&self.driver, launch).await
    }

    async fn stop(&self, service: &ServiceId) -> PortResult<()> {
        if !service.is_apache() {
            return Ok(());
        }
        self.host.stop(&self.driver, service).await
    }

    /// Graceful restart (configuration re-read, in-flight requests complete, idle php-cgi
    /// processes recycled), ready once httpd listens on every port of the new configuration.
    /// Falls back to stop + start if the signal cannot be delivered or httpd dies or does not
    /// listen while re-reading its configuration.
    async fn restart(&self, launch: &ServiceLaunch) -> PortResult<()> {
        self.host.restart(&self.driver, launch).await
    }

    fn status(&self, service: &ServiceId) -> ServiceStatus {
        if !service.is_apache() {
            return ServiceStatus::stopped(service.clone());
        }
        self.host.status(service)
    }
}

#[cfg(test)]
mod tests;
