use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use lampo_domain::events::DomainEvent;
use lampo_domain::ports::{ChildId, CommandSpec, EventPublisher, ProcessObserver, ProcessSupervisor, ServiceRunner, SpawnedChild};
use lampo_domain::service::{LogStream, ServiceId, ServiceKind, ServiceLaunch, ServiceState, ServiceStatus};
use lampo_domain::{PortError, PortResult};

use super::config::main_conf_path;
use super::listen::ports_to_own;
use super::signal::{HttpdSignal, HttpdSignals, NamedEventSignals};
use super::validation::{httpd_exe, rotatelogs_exe};
use crate::fs_util::ensure_dir;

/// How long to wait for a started (or restarted) httpd to listen on every configured port.
const START_TIMEOUT: Duration = Duration::from_secs(20);
/// Interval between two readiness checks.
const READY_POLL: Duration = Duration::from_millis(150);
/// After a graceful shutdown request, how long before the supervisor kills the process tree.
/// Measured with Apache Lounge 2.4 + mod_fcgid 2.3.10: with php-cgi processes alive, mod_fcgid
/// waits ~8.5 s for them before terminating them itself, ~11 s in total.
const STOP_GRACE: Duration = Duration::from_secs(20);
/// Output lines kept to explain a failed start.
const RECENT_LINES: usize = 30;

/// Runs `httpd.exe -f <config_dir>\httpd.conf` in the foreground under the supervisor (no
/// Windows service). Stop and restart are graceful through httpd's named events (see
/// [`super::signal`]), with the supervisor's kill as fallback. Ready means every port of the
/// live configuration's `Listen` directives is listened on by httpd's own process tree (not
/// by another server that took a port). Publishes status/log events.
pub struct ApacheRunner {
    supervisor: Arc<dyn ProcessSupervisor>,
    events: Arc<dyn EventPublisher>,
    signals: Arc<dyn HttpdSignals>,
    state: Arc<Mutex<RunState>>,
    /// Serialises start/stop/restart.
    control: tokio::sync::Mutex<()>,
}

struct RunState {
    /// Incremented per spawn so exit callbacks of an old process are ignored.
    generation: u64,
    child: Option<SpawnedChild>,
    stopping: bool,
    status: ServiceStatus,
    recent: VecDeque<String>,
}

impl ApacheRunner {
    pub fn new(supervisor: Arc<dyn ProcessSupervisor>, events: Arc<dyn EventPublisher>) -> Self {
        Self::with_signals(supervisor, events, Arc::new(NamedEventSignals))
    }

    pub fn with_signals(supervisor: Arc<dyn ProcessSupervisor>, events: Arc<dyn EventPublisher>, signals: Arc<dyn HttpdSignals>) -> Self {
        let state = RunState { generation: 0, child: None, stopping: false, status: ServiceStatus::stopped(ServiceId::apache()), recent: VecDeque::new() };
        Self { supervisor, events, signals, state: Arc::new(Mutex::new(state)), control: tokio::sync::Mutex::new(()) }
    }

    fn lock(&self) -> MutexGuard<'_, RunState> {
        lock(&self.state)
    }

    fn set_status(&self, state: ServiceState, pid: Option<u32>, port: Option<u16>, message: Option<String>) {
        let status = ServiceStatus { service: ServiceId::apache(), state, pid, port, message };
        self.lock().status = status.clone();
        self.events.publish(DomainEvent::ServiceStatusChanged(status));
    }

    /// The running child, if the supervisor still reports it alive.
    fn live_child(&self) -> Option<SpawnedChild> {
        let child = self.lock().child?;
        self.supervisor.is_alive(child.id).then_some(child)
    }

    async fn start_inner(&self, launch: &ServiceLaunch) -> PortResult<()> {
        if self.live_child().is_some() {
            return Ok(());
        }
        let (program, conf) = check_installation(launch)?;
        let ports = ports_to_own(&conf, launch.port)?;
        let generation = {
            let mut s = self.lock();
            s.generation += 1;
            s.stopping = false;
            s.recent.clear();
            s.generation
        };
        self.set_status(ServiceState::Starting, None, Some(launch.port), None);
        let observer = Arc::new(HttpdObserver { state: self.state.clone(), events: self.events.clone(), generation });
        let spec = CommandSpec { program, args: vec!["-f".into(), conf.display().to_string()], working_dir: Some(launch.install_dir.clone()), env: Vec::new(), path_prepend: Vec::new() };
        let child = match self.supervisor.spawn(spec, observer).await {
            Ok(c) => c,
            Err(e) => {
                self.set_status(ServiceState::Failed, None, Some(launch.port), Some(e.to_string()));
                return Err(e);
            }
        };
        self.lock().child = Some(child);
        match self.wait_until_ready(child.id, launch.port, &ports).await {
            Ok(()) => {
                self.set_status(ServiceState::Running, Some(child.pid), Some(launch.port), None);
                Ok(())
            }
            Err(e) => {
                let _ = self.supervisor.terminate(child.id, Duration::from_secs(2)).await;
                self.lock().child = None;
                self.set_status(ServiceState::Failed, None, Some(launch.port), Some(e.to_string()));
                Err(e)
            }
        }
    }

    /// Polls until httpd's process tree listens on every port of `ports` (someone else's
    /// server on one of them does not count), failing early if it exits.
    async fn wait_until_ready(&self, child: ChildId, shared_port: u16, ports: &[u16]) -> PortResult<()> {
        let deadline = tokio::time::Instant::now() + START_TIMEOUT;
        loop {
            if !self.supervisor.is_alive(child) {
                return Err(self.start_failure(shared_port));
            }
            let missing: Vec<u16> = ports.iter().copied().filter(|&port| !self.supervisor.owns_listener(child, port)).collect();
            if missing.is_empty() {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                let noun = if missing.len() == 1 { "port" } else { "ports" };
                let missing = missing.iter().map(u16::to_string).collect::<Vec<_>>().join(", ");
                return Err(PortError::process("httpd", format!("Apache did not listen on {noun} {missing} within {}s", START_TIMEOUT.as_secs())));
            }
            tokio::time::sleep(READY_POLL).await;
        }
    }

    /// Error for an httpd that exited during start-up, from its last output lines.
    fn start_failure(&self, port: u16) -> PortError {
        let recent: Vec<String> = self.lock().recent.iter().cloned().collect();
        let text = recent.join("\n");
        if text.contains("could not bind") || text.contains("10048") {
            return PortError::PortInUse { port, owner: None };
        }
        let detail = if text.is_empty() { "Apache exited during start-up (see the Apache error log)".to_string() } else { text };
        PortError::process("httpd", detail)
    }

    async fn stop_inner(&self) -> PortResult<()> {
        let Some(child) = self.lock().child else {
            if self.lock().status.state != ServiceState::Stopped {
                self.set_status(ServiceState::Stopped, None, None, None);
            }
            return Ok(());
        };
        self.lock().stopping = true;
        let port = self.lock().status.port;
        self.set_status(ServiceState::Stopping, Some(child.pid), port, None);
        if let Err(e) = self.signals.send(child.pid, HttpdSignal::Shutdown) {
            tracing::debug!("graceful httpd shutdown not possible ({e}): terminating");
        }
        let result = self.supervisor.terminate(child.id, STOP_GRACE).await;
        self.lock().child = None;
        match result {
            Ok(()) => {
                self.set_status(ServiceState::Stopped, None, None, None);
                Ok(())
            }
            Err(e) => {
                self.set_status(ServiceState::Failed, None, port, Some(e.to_string()));
                Err(e)
            }
        }
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
        let _serial = self.control.lock().await;
        self.start_inner(launch).await
    }

    async fn stop(&self, service: &ServiceId) -> PortResult<()> {
        if !service.is_apache() {
            return Ok(());
        }
        let _serial = self.control.lock().await;
        self.stop_inner().await
    }

    /// Graceful restart (configuration re-read, in-flight requests complete, idle php-cgi
    /// processes recycled), ready once httpd listens on every port of the new configuration.
    /// Falls back to stop + start if the signal cannot be delivered or httpd dies or does not
    /// listen while re-reading its configuration.
    async fn restart(&self, launch: &ServiceLaunch) -> PortResult<()> {
        let _serial = self.control.lock().await;
        let Some(child) = self.live_child() else {
            return self.start_inner(launch).await;
        };
        let (_, conf) = check_installation(launch)?;
        let ports = ports_to_own(&conf, launch.port)?;
        let same_port = self.lock().status.port == Some(launch.port);
        if same_port && self.signals.send(child.pid, HttpdSignal::Restart).is_ok() {
            if self.wait_until_ready(child.id, launch.port, &ports).await.is_ok() {
                self.set_status(ServiceState::Running, Some(child.pid), Some(launch.port), None);
                return Ok(());
            }
            tracing::warn!("httpd did not survive a graceful restart: starting it again");
        }
        self.stop_inner().await?;
        self.start_inner(launch).await
    }

    fn status(&self, service: &ServiceId) -> ServiceStatus {
        if !service.is_apache() {
            return ServiceStatus::stopped(service.clone());
        }
        self.lock().status.clone()
    }
}

/// `(httpd.exe, httpd.conf)`, once they and `rotatelogs.exe` exist, or a `NotFound` naming what is
/// missing.
fn check_installation(launch: &ServiceLaunch) -> PortResult<(std::path::PathBuf, std::path::PathBuf)> {
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
    if !Path::new(&conf).is_file() {
        return Err(PortError::not_found("Apache configuration", conf.display().to_string()));
    }
    Ok((program, conf))
}

fn lock(state: &Mutex<RunState>) -> MutexGuard<'_, RunState> {
    state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

struct HttpdObserver {
    state: Arc<Mutex<RunState>>,
    events: Arc<dyn EventPublisher>,
    generation: u64,
}

impl ProcessObserver for HttpdObserver {
    fn on_output(&self, stream: LogStream, line: &str) {
        {
            let mut s = lock(&self.state);
            if s.generation == self.generation {
                if s.recent.len() == RECENT_LINES {
                    s.recent.pop_front();
                }
                s.recent.push_back(line.to_string());
            }
        }
        let at_unix_ms = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or_default();
        self.events.publish(DomainEvent::ServiceLog { service: ServiceId::apache(), stream, line: line.to_string(), at_unix_ms });
    }

    fn on_exit(&self, code: Option<i32>) {
        let status = {
            let mut s = lock(&self.state);
            if s.generation != self.generation || s.stopping || s.status.state == ServiceState::Starting {
                // Old process, requested stop, or start-up failure (reported by `start`).
                return;
            }
            s.child = None;
            let last = s.recent.back().cloned();
            let message = match (code, last) {
                (Some(c), Some(l)) => format!("Apache exited unexpectedly (code {c}): {l}"),
                (Some(c), None) => format!("Apache exited unexpectedly (code {c})"),
                (None, _) => "Apache was killed".to_string(),
            };
            s.status = ServiceStatus { service: ServiceId::apache(), state: ServiceState::Failed, pid: None, port: s.status.port, message: Some(message) };
            s.status.clone()
        };
        self.events.publish(DomainEvent::ServiceStatusChanged(status));
    }
}

#[cfg(test)]
mod tests;
