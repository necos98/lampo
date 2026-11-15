//! [`ApacheRunner`] against a scripted supervisor: readiness is "httpd's tree owns every
//! listened port", checked on tokio's paused clock where timing matters.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use super::*;

const SHARED_PORT: u16 = 8080;
const LOOPBACK_PORT: u16 = 8101;

/// Owns every port while alive, except those in `unowned` (another server holds them, or
/// httpd has not bound them yet).
#[derive(Default)]
struct FakeSupervisor {
    alive: AtomicBool,
    next: AtomicU64,
    observers: Mutex<Vec<Arc<dyn ProcessObserver>>>,
    specs: Mutex<Vec<CommandSpec>>,
    unowned: Mutex<HashSet<u16>>,
    terminated: AtomicU64,
}

impl FakeSupervisor {
    fn disown(&self, port: u16) {
        self.unowned.lock().unwrap().insert(port);
    }

    /// The port becomes httpd's after `delay`.
    fn own_after(self: &Arc<Self>, port: u16, delay: Duration) {
        self.disown(port);
        let sup = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            sup.unowned.lock().unwrap().remove(&port);
        });
    }
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
        self.terminated.fetch_add(1, Ordering::SeqCst);
        self.alive.store(false, Ordering::SeqCst);
        Ok(())
    }
    fn is_alive(&self, _child: ChildId) -> bool {
        self.alive.load(Ordering::SeqCst)
    }
    fn owns_listener(&self, child: ChildId, port: u16) -> bool {
        self.is_alive(child) && !self.unowned.lock().unwrap().contains(&port)
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
    supervisor: Arc<FakeSupervisor>,
    events: Arc<Events>,
    signals: Arc<Signals>,
    runner: ApacheRunner,
}

impl Fixture {
    /// Replaces the live `httpd.conf` (what the configurator does before a restart).
    fn write_conf(&self, text: &str) {
        std::fs::write(self.launch.config_dir.join("httpd.conf"), text).unwrap();
    }
}

/// A fake Apache install whose configuration listens on the shared port only.
fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let install = dir.path().join("apache 2.4");
    std::fs::create_dir_all(install.join("bin")).unwrap();
    std::fs::write(install.join("bin").join("httpd.exe"), b"").unwrap();
    let config_dir = dir.path().join("generated").join("apache");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::write(config_dir.join("httpd.conf"), format!("Listen 127.0.0.1:{SHARED_PORT}\nListen [::1]:{SHARED_PORT}\n")).unwrap();
    let launch = ServiceLaunch { service: ServiceId::apache(), install_dir: install, port: SHARED_PORT, ui_port: None, config_dir, data_dir: None, log_dir: dir.path().join("logs") };
    let supervisor = Arc::new(FakeSupervisor::default());
    let events = Arc::new(Events::default());
    let signals = Arc::new(Signals::default());
    let runner = ApacheRunner::with_signals(supervisor.clone(), events.clone(), signals.clone());
    Fixture { _dir: dir, launch, supervisor, events, signals, runner }
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
    // httpd never gets the port, and dies complaining about the bind.
    f.supervisor.disown(SHARED_PORT);
    let launch = f.launch.clone();
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
    assert_eq!(err, PortError::PortInUse { port: SHARED_PORT, owner: None });
    assert_eq!(f.runner.status(&ServiceId::apache()).state, ServiceState::Failed);
}

#[tokio::test(start_paused = true)]
async fn start_waits_until_httpd_listens_on_every_configured_port() {
    let f = fixture();
    f.write_conf(&format!("Listen 127.0.0.1:{SHARED_PORT}\nListen 127.0.0.1:{LOOPBACK_PORT}\n"));
    f.supervisor.own_after(LOOPBACK_PORT, Duration::from_secs(3));
    let started = tokio::time::Instant::now();
    f.runner.start(&f.launch).await.unwrap();
    assert!(started.elapsed() >= Duration::from_secs(3), "ready before the dedicated port was bound");
    assert_eq!(f.runner.status(&ServiceId::apache()).state, ServiceState::Running);
}

#[tokio::test(start_paused = true)]
async fn a_port_held_by_another_server_fails_the_start() {
    let f = fixture();
    f.write_conf(&format!("Listen 127.0.0.1:{SHARED_PORT}\nListen 127.0.0.1:{LOOPBACK_PORT}\n"));
    // Something answers on the dedicated port, but it is not httpd.
    f.supervisor.disown(LOOPBACK_PORT);
    let err = f.runner.start(&f.launch).await.unwrap_err();
    assert!(err.to_string().contains(&format!("did not listen on port {LOOPBACK_PORT} within 20s")), "{err}");
    assert_eq!(f.supervisor.terminated.load(Ordering::SeqCst), 1);
    assert_eq!(f.runner.status(&ServiceId::apache()).state, ServiceState::Failed);
}

#[tokio::test(start_paused = true)]
async fn graceful_restart_is_ready_as_soon_as_every_port_is_httpds() {
    let f = fixture();
    f.runner.start(&f.launch).await.unwrap();
    let restarted = tokio::time::Instant::now();
    f.runner.restart(&f.launch).await.unwrap();
    assert!(restarted.elapsed() < Duration::from_millis(500), "no fixed settle time: {:?}", restarted.elapsed());

    // The new configuration adds a dedicated port: the restart waits for httpd to bind it.
    f.write_conf(&format!("Listen 127.0.0.1:{SHARED_PORT}\nListen 127.0.0.1:{LOOPBACK_PORT}\n"));
    f.supervisor.own_after(LOOPBACK_PORT, Duration::from_secs(2));
    let restarted = tokio::time::Instant::now();
    f.runner.restart(&f.launch).await.unwrap();
    assert!(restarted.elapsed() >= Duration::from_secs(2));
    assert_eq!(f.supervisor.specs.lock().unwrap().len(), 1, "restarted in place");
    assert_eq!(f.runner.status(&ServiceId::apache()).state, ServiceState::Running);
}

#[tokio::test(start_paused = true)]
async fn a_restart_that_never_binds_a_new_port_falls_back_to_stop_and_start() {
    let f = fixture();
    f.runner.start(&f.launch).await.unwrap();
    f.write_conf(&format!("Listen 127.0.0.1:{SHARED_PORT}\nListen 127.0.0.1:{LOOPBACK_PORT}\n"));
    f.supervisor.own_after(LOOPBACK_PORT, START_TIMEOUT + Duration::from_secs(5));
    f.runner.restart(&f.launch).await.unwrap();
    assert_eq!(f.supervisor.specs.lock().unwrap().len(), 2, "a fresh httpd after the failed graceful restart");
    assert_eq!(f.signals.0.lock().unwrap().as_slice(), &[(1000, HttpdSignal::Restart), (1000, HttpdSignal::Shutdown)]);
    assert_eq!(f.runner.status(&ServiceId::apache()).state, ServiceState::Running);
}
