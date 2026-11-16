//! How [`lampo_infra_services::ServiceHost`] runs httpd: `httpd.exe -f <config_dir>\httpd.conf`
//! in the foreground (no Windows service), ready once every port of the live configuration's
//! `Listen` directives is httpd's own, reloaded and shut down through its named events (see
//! [`super::signal`]).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use lampo_domain::ports::CommandSpec;
use lampo_domain::service::ServiceLaunch;
use lampo_domain::{PortError, PortResult};
use lampo_infra_services::ServiceDriver;

use super::config::main_conf_path;
use super::listen::ports_to_own;
use super::signal::{HttpdSignal, HttpdSignals};
use super::validation::{httpd_exe, rotatelogs_exe};

/// How long to wait for a started (or restarted) httpd to listen on every configured port.
pub(crate) const START_TIMEOUT: Duration = Duration::from_secs(20);
/// After a graceful shutdown request, how long before the supervisor kills the process tree.
/// Measured with Apache Lounge 2.4 + mod_fcgid 2.3.10: with php-cgi processes alive, mod_fcgid
/// waits ~8.5 s for them before terminating them itself, ~11 s in total.
const STOP_GRACE: Duration = Duration::from_secs(20);
/// What httpd prints when a `Listen` address is taken (`AH00072: make_sock: could not bind`,
/// WSAEADDRINUSE).
const BIND_FAILURE_MARKERS: [&str; 2] = ["could not bind", "10048"];

pub(crate) struct ApacheDriver {
    signals: Arc<dyn HttpdSignals>,
}

impl ApacheDriver {
    pub(crate) fn new(signals: Arc<dyn HttpdSignals>) -> Self {
        Self { signals }
    }

    /// Sets httpd's named event; `false` when it cannot be delivered.
    fn signal(&self, pid: u32, signal: HttpdSignal) -> bool {
        match self.signals.send(pid, signal) {
            Ok(()) => true,
            Err(e) => {
                tracing::debug!("httpd {signal:?} signal not delivered to pid {pid}: {e}");
                false
            }
        }
    }
}

#[async_trait]
impl ServiceDriver for ApacheDriver {
    fn program(&self) -> &'static str {
        "httpd"
    }

    fn command(&self, launch: &ServiceLaunch) -> PortResult<CommandSpec> {
        let (program, conf) = check_installation(launch)?;
        Ok(CommandSpec { program, args: vec!["-f".into(), conf.display().to_string()], working_dir: Some(launch.install_dir.clone()), env: Vec::new(), path_prepend: Vec::new() })
    }

    fn startup_timeout(&self) -> Duration {
        START_TIMEOUT
    }

    fn stop_grace(&self) -> Duration {
        STOP_GRACE
    }

    /// Every port of the live configuration, a vhost's dedicated loopback port included.
    fn ready_ports(&self, launch: &ServiceLaunch) -> PortResult<Vec<u16>> {
        ports_to_own(&main_conf_path(&launch.config_dir), launch.port)
    }

    /// Owning every listener is the readiness check: no request is made.
    async fn probe(&self, _launch: &ServiceLaunch) -> bool {
        true
    }

    /// Graceful restart: the configuration is re-read, in-flight requests complete, idle
    /// php-cgi processes are recycled.
    async fn reload(&self, _launch: &ServiceLaunch, pid: u32) -> bool {
        self.signal(pid, HttpdSignal::Restart)
    }

    async fn request_shutdown(&self, _launch: &ServiceLaunch, pid: u32) {
        self.signal(pid, HttpdSignal::Shutdown);
    }

    fn shutdown_addresses_the_process(&self) -> bool {
        true
    }

    fn is_port_conflict(&self, line: &str) -> bool {
        BIND_FAILURE_MARKERS.iter().any(|marker| line.contains(marker))
    }
}

/// `(httpd.exe, httpd.conf)`, once they and `rotatelogs.exe` exist, or a `NotFound` naming what is
/// missing.
pub(crate) fn check_installation(launch: &ServiceLaunch) -> PortResult<(PathBuf, PathBuf)> {
    let program = httpd_exe(&launch.install_dir);
    if !program.is_file() {
        return Err(PortError::not_found("Apache executable", program.display().to_string()));
    }
    // The generated logs are piped through it: without it httpd would exit at start-up.
    let rotatelogs = rotatelogs_exe(&launch.install_dir);
    if !rotatelogs.is_file() {
        return Err(PortError::not_found("Apache rotatelogs", rotatelogs.display().to_string()));
    }
    let conf = main_conf_path(&launch.config_dir);
    if !conf.is_file() {
        return Err(PortError::not_found("Apache configuration", conf.display().to_string()));
    }
    Ok((program, conf))
}
