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
use super::signal::{HttpdSignal, HttpdSignals, NamedEventSignals};
use super::validation::httpd_exe;
use crate::fs_util::ensure_dir;

/// How long to wait for a started httpd to accept connections.
const START_TIMEOUT: Duration = Duration::from_secs(20);
/// After a graceful shutdown request, how long before the supervisor kills the process tree.
/// Measured with Apache Lounge 2.4 + mod_fcgid 2.3.10: with php-cgi processes alive, mod_fcgid
/// waits ~8.5 s for them before terminating them itself, ~11 s in total.
const STOP_GRACE: Duration = Duration::from_secs(20);
/// A graceful restart re-reads the configuration; a fatal error makes the parent exit soon.
const RESTART_SETTLE: Duration = Duration::from_millis(1500);
/// Output lines kept to explain a failed start.
const RECENT_LINES: usize = 30;

/// Runs `httpd.exe -f <config_dir>\httpd.conf` in the foreground under the supervisor (no
/// Windows service). Stop and restart are graceful through httpd's named events (see
/// [`super::signal`]), with the supervisor's kill as fallback. Publishes status/log events.
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
        match self.wait_until_ready(child.id, launch.port, START_TIMEOUT).await {
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

    /// Polls the shared port until httpd accepts connections, failing early if it exits.
    async fn wait_until_ready(&self, child: ChildId, port: u16, timeout: Duration) -> PortResult<()> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if !self.supervisor.is_alive(child) {
                return Err(self.start_failure(port));
            }
            let connect = tokio::net::TcpStream::connect(("127.0.0.1", port));
            if let Ok(Ok(_)) = tokio::time::timeout(Duration::from_millis(500), connect).await {
                // Someone else may own the port while httpd is about to fail to bind it.
                tokio::time::sleep(Duration::from_millis(200)).await;
                if self.supervisor.is_alive(child) {
                    return Ok(());
                }
                return Err(self.start_failure(port));
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(PortError::process("httpd", format!("Apache did not accept connections on port {port} within {}s", timeout.as_secs())));
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
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
    /// processes recycled). Falls back to stop + start if the signal cannot be delivered or
    /// httpd dies while re-reading its configuration.
    async fn restart(&self, launch: &ServiceLaunch) -> PortResult<()> {
        let _serial = self.control.lock().await;
        let Some(child) = self.live_child() else {
            return self.start_inner(launch).await;
        };
        let same_port = self.lock().status.port == Some(launch.port);
        if same_port && self.signals.send(child.pid, HttpdSignal::Restart).is_ok() {
            tokio::time::sleep(RESTART_SETTLE).await;
            if self.wait_until_ready(child.id, launch.port, START_TIMEOUT).await.is_ok() {
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

/// `(httpd.exe, httpd.conf)` or a `NotFound` naming what is missing.
fn check_installation(launch: &ServiceLaunch) -> PortResult<(std::path::PathBuf, std::path::PathBuf)> {
    let program = httpd_exe(&launch.install_dir);
    if !program.is_file() {
        return Err(PortError::not_found("Apache executable", program.display().to_string()));
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
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

    use super::*;

    #[derive(Default)]
    struct FakeSupervisor {
        alive: AtomicBool,
        next: AtomicU64,
        observers: Mutex<Vec<Arc<dyn ProcessObserver>>>,
        specs: Mutex<Vec<CommandSpec>>,
    }

    #[async_trait]
    impl ProcessSupervisor for FakeSupervisor {
        async fn spawn(&self, spec: CommandSpec, observer: Arc<dyn ProcessObserver>) -> PortResult<SpawnedChild> {
            self.specs.lock().unwrap().push(spec);
            self.observers.lock().unwrap().push(observer);
            self.alive.store(true, Ordering::SeqCst);
            let n = self.next.fetch_add(1, Ordering::SeqCst);
            Ok(SpawnedChild { id: ChildId(n), pid: 1000 + n as u32 })
        }
        async fn terminate(&self, _child: ChildId, _grace: Duration) -> PortResult<()> {
            self.alive.store(false, Ordering::SeqCst);
            Ok(())
        }
        fn is_alive(&self, _child: ChildId) -> bool {
            self.alive.load(Ordering::SeqCst)
        }
    }

    #[derive(Default)]
    struct Events(Mutex<Vec<DomainEvent>>);
    impl EventPublisher for Events {
        fn publish(&self, event: DomainEvent) {
            self.0.lock().unwrap().push(event);
        }
    }

    #[derive(Default)]
    struct Signals(Mutex<Vec<(u32, HttpdSignal)>>);
    impl HttpdSignals for Signals {
        fn send(&self, pid: u32, signal: HttpdSignal) -> std::io::Result<()> {
            self.0.lock().unwrap().push((pid, signal));
            Ok(())
        }
    }

    struct Fixture {
        _dir: tempfile::TempDir,
        launch: ServiceLaunch,
        _listener: std::net::TcpListener,
        supervisor: Arc<FakeSupervisor>,
        events: Arc<Events>,
        signals: Arc<Signals>,
        runner: ApacheRunner,
    }

    /// A fake Apache install and a real listener standing in for httpd's socket.
    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let install = dir.path().join("apache 2.4");
        std::fs::create_dir_all(install.join("bin")).unwrap();
        std::fs::write(install.join("bin").join("httpd.exe"), b"").unwrap();
        let config_dir = dir.path().join("generated").join("apache");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::write(config_dir.join("httpd.conf"), b"").unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let launch = ServiceLaunch { service: ServiceId::apache(), install_dir: install, port, config_dir, data_dir: None, log_dir: dir.path().join("logs") };
        let supervisor = Arc::new(FakeSupervisor::default());
        let events = Arc::new(Events::default());
        let signals = Arc::new(Signals::default());
        let runner = ApacheRunner::with_signals(supervisor.clone(), events.clone(), signals.clone());
        Fixture { _dir: dir, launch, _listener: listener, supervisor, events, signals, runner }
    }

    #[tokio::test]
    async fn start_runs_httpd_with_generated_config_and_reports_running() {
        let f = fixture();
        f.runner.prepare(&f.launch).await.unwrap();
        f.runner.start(&f.launch).await.unwrap();
        let spec = f.supervisor.specs.lock().unwrap()[0].clone();
        assert!(spec.program.ends_with("bin/httpd.exe") || spec.program.ends_with(r"bin\httpd.exe"));
        assert_eq!(spec.args[0], "-f");
        assert!(spec.args[1].ends_with("httpd.conf"));
        let status = f.runner.status(&ServiceId::apache());
        assert_eq!(status.state, ServiceState::Running);
        assert_eq!(status.pid, Some(1000));
        assert!(f.launch.log_dir.is_dir());
        // Starting twice is a no-op.
        f.runner.start(&f.launch).await.unwrap();
        assert_eq!(f.supervisor.specs.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn unexpected_exit_is_published_as_failed() {
        let f = fixture();
        f.runner.start(&f.launch).await.unwrap();
        let observer = f.supervisor.observers.lock().unwrap()[0].clone();
        observer.on_output(LogStream::Stderr, "AH00000: boom");
        f.supervisor.alive.store(false, Ordering::SeqCst);
        observer.on_exit(Some(1));
        let status = f.runner.status(&ServiceId::apache());
        assert_eq!(status.state, ServiceState::Failed);
        assert!(status.message.unwrap().contains("boom"));
        let events = f.events.0.lock().unwrap();
        assert!(events.iter().any(|e| matches!(e, DomainEvent::ServiceLog { line, .. } if line == "AH00000: boom")));
        assert!(matches!(events.last(), Some(DomainEvent::ServiceStatusChanged(s)) if s.state == ServiceState::Failed));
    }

    #[tokio::test]
    async fn stop_is_graceful_then_supervised() {
        let f = fixture();
        f.runner.start(&f.launch).await.unwrap();
        f.runner.stop(&ServiceId::apache()).await.unwrap();
        assert_eq!(f.signals.0.lock().unwrap().as_slice(), &[(1000, HttpdSignal::Shutdown)]);
        assert_eq!(f.runner.status(&ServiceId::apache()).state, ServiceState::Stopped);
        // The exit callback of a requested stop does not turn into a failure.
        f.supervisor.observers.lock().unwrap()[0].on_exit(Some(0));
        assert_eq!(f.runner.status(&ServiceId::apache()).state, ServiceState::Stopped);
    }

    #[tokio::test]
    async fn restart_signals_the_running_parent() {
        let f = fixture();
        f.runner.start(&f.launch).await.unwrap();
        f.runner.restart(&f.launch).await.unwrap();
        assert_eq!(f.signals.0.lock().unwrap().as_slice(), &[(1000, HttpdSignal::Restart)]);
        assert_eq!(f.supervisor.specs.lock().unwrap().len(), 1, "no new process");
        assert_eq!(f.runner.status(&ServiceId::apache()).state, ServiceState::Running);
    }

    #[tokio::test]
    async fn missing_config_is_reported_before_spawning() {
        let f = fixture();
        std::fs::remove_file(f.launch.config_dir.join("httpd.conf")).unwrap();
        let err = f.runner.start(&f.launch).await.unwrap_err();
        assert!(matches!(err, PortError::NotFound { entity: "Apache configuration", .. }));
        assert!(f.supervisor.specs.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn bind_failure_maps_to_port_in_use() {
        let f = fixture();
        // A port nobody listens on, and an httpd that dies complaining about the bind.
        let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let launch = ServiceLaunch { port: free, ..f.launch.clone() };
        let sup = f.supervisor.clone();
        let killer = tokio::spawn(async move {
            loop {
                let obs = sup.observers.lock().unwrap().first().cloned();
                if let Some(o) = obs {
                    o.on_output(LogStream::Stderr, "(OS 10048)Only one usage of each socket address is normally permitted.  : AH00072: make_sock: could not bind to address 127.0.0.1:80");
                    sup.alive.store(false, Ordering::SeqCst);
                    o.on_exit(Some(1));
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        });
        let err = f.runner.start(&launch).await.unwrap_err();
        killer.await.unwrap();
        assert_eq!(err, PortError::PortInUse { port: free, owner: None });
        assert_eq!(f.runner.status(&ServiceId::apache()).state, ServiceState::Failed);
    }
}
