//! [`ServiceHost`] start/stop/restart sequence against a scripted supervisor and driver, on tokio's
//! paused clock: slow starts that keep making progress, starts that give up, a stop arriving
//! while a start waits, restarts in place or through stop and start.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use lampo_domain::events::DomainEvent;
use lampo_domain::mysql::MysqlSeries;
use lampo_domain::ports::{ChildId, CommandSpec, EventPublisher, ProcessObserver, ProcessSupervisor, SpawnedChild};
use lampo_domain::service::{ServiceLaunch, ServiceState, ServiceStatus};
use lampo_domain::tcp_port::Port;
use lampo_domain::{PortError, PortResult};
use tokio::time::Instant;

use crate::service_host::{ServiceDriver, ServiceHost};

const TIMEOUT: Duration = Duration::from_secs(10);
const PID: u32 = 4242;

type Calls = Arc<Mutex<Vec<&'static str>>>;

/// A child that listens (owns its port) from `listen_after` on, if ever.
struct FakeSupervisor {
    t0: Instant,
    listen_after: Option<Duration>,
    alive: AtomicBool,
    observer: Mutex<Option<Arc<dyn ProcessObserver>>>,
    calls: Calls,
}

#[async_trait]
impl ProcessSupervisor for FakeSupervisor {
    async fn spawn(&self, _spec: CommandSpec, observer: Arc<dyn ProcessObserver>) -> PortResult<SpawnedChild> {
        *self.observer.lock().unwrap() = Some(observer);
        self.alive.store(true, Ordering::SeqCst);
        Ok(SpawnedChild { id: ChildId(1), pid: PID })
    }

    async fn terminate(&self, _child: ChildId, _grace: Duration) -> PortResult<()> {
        self.calls.lock().unwrap().push("terminate");
        if self.alive.swap(false, Ordering::SeqCst) {
            let observer = self.observer.lock().unwrap().clone();
            observer.expect("spawned").on_exit(None);
        }
        Ok(())
    }

    fn is_alive(&self, _child: ChildId) -> bool {
        self.alive.load(Ordering::SeqCst)
    }

    fn owns_listener(&self, child: ChildId, _port: Port) -> bool {
        self.is_alive(child) && self.listen_after.is_some_and(|after| self.t0.elapsed() >= after)
    }
}

/// A server whose error log grows every second until `grows_for` has passed (`None`: no
/// progress marker at all).
struct FakeDriver {
    t0: Instant,
    grows_for: Option<Duration>,
    /// Whether `reload` applies a new configuration in place.
    reloads: bool,
    calls: Calls,
}

#[async_trait]
impl ServiceDriver for FakeDriver {
    fn program(&self) -> &'static str {
        "fake"
    }

    fn command(&self, _launch: &ServiceLaunch) -> PortResult<CommandSpec> {
        Ok(CommandSpec::default())
    }

    fn startup_timeout(&self) -> Duration {
        TIMEOUT
    }

    fn startup_progress(&self, _launch: &ServiceLaunch) -> Option<u64> {
        self.grows_for.map(|limit| self.t0.elapsed().min(limit).as_secs())
    }

    fn slow_start_detail(&self) -> &'static str {
        "recovery"
    }

    fn stop_grace(&self) -> Duration {
        Duration::from_secs(5)
    }

    async fn probe(&self, _launch: &ServiceLaunch) -> bool {
        true
    }

    async fn reload(&self, _launch: &ServiceLaunch, pid: u32) -> bool {
        assert_eq!(pid, PID);
        self.calls.lock().unwrap().push("reload");
        self.reloads
    }

    async fn request_shutdown(&self, _launch: &ServiceLaunch, pid: u32) {
        assert_eq!(pid, PID);
        self.calls.lock().unwrap().push("request_shutdown");
    }

    fn shutdown_addresses_the_process(&self) -> bool {
        true
    }
}

#[derive(Default)]
struct Statuses(Mutex<Vec<ServiceStatus>>);

impl EventPublisher for Statuses {
    fn publish(&self, event: DomainEvent) {
        if let DomainEvent::ServiceStatusChanged(status) = event {
            self.0.lock().unwrap().push(status);
        }
    }
}

struct Fixture {
    _dir: tempfile::TempDir,
    launch: ServiceLaunch,
    host: ServiceHost,
    supervisor: Arc<FakeSupervisor>,
    driver: FakeDriver,
    statuses: Arc<Statuses>,
    calls: Calls,
    t0: Instant,
}

fn fixture(grows_for: Option<Duration>, listen_after: Option<Duration>) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    // Nobody listens on it: the host's own "port already in use" check passes.
    let port = Port::new(std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()).unwrap();
    let launch = ServiceLaunch {
        service: MysqlSeries::DEFAULT.service_id(),
        install_dir: dir.path().join("install"),
        port,
        ui_port: None,
        config_dir: dir.path().join("config"),
        data_dir: None,
        log_dir: dir.path().join("logs"),
    };
    let (t0, calls) = (Instant::now(), Calls::default());
    let supervisor = Arc::new(FakeSupervisor { t0, listen_after, alive: AtomicBool::new(false), observer: Mutex::default(), calls: calls.clone() });
    let statuses = Arc::new(Statuses::default());
    let host = ServiceHost::new(supervisor.clone(), statuses.clone());
    Fixture { _dir: dir, launch, host, supervisor, driver: FakeDriver { t0, grows_for, reloads: false, calls: calls.clone() }, statuses, calls, t0 }
}

impl Fixture {
    fn calls(&self) -> Vec<&'static str> {
        self.calls.lock().unwrap().clone()
    }

    fn state(&self) -> ServiceState {
        self.host.status(&self.launch.service).state
    }

    /// The observer of the last spawned process.
    fn supervisor_observer(&self) -> Arc<dyn ProcessObserver> {
        self.supervisor.observer.lock().unwrap().clone().expect("spawned")
    }
}

#[tokio::test(start_paused = true)]
async fn a_start_that_keeps_logging_outlives_the_timeout_and_shows_recovery() {
    let f = fixture(Some(Duration::from_secs(60)), Some(Duration::from_secs(25)));
    f.host.start(&f.driver, &f.launch).await.unwrap();
    assert_eq!(f.state(), ServiceState::Running);
    assert!(f.t0.elapsed() >= Duration::from_secs(25));
    let statuses = f.statuses.0.lock().unwrap();
    assert!(statuses.iter().any(|s| s.state == ServiceState::Starting && s.message.as_deref() == Some("recovery")), "{statuses:?}");
    assert_eq!(statuses.last().unwrap().message, None, "the detail is cleared once running");
    assert!(f.calls().is_empty(), "nothing was stopped");
}

#[tokio::test(start_paused = true)]
async fn a_start_without_progress_gives_up_at_the_timeout_asking_for_a_clean_shutdown_first() {
    let f = fixture(None, None);
    let err = f.host.start(&f.driver, &f.launch).await.unwrap_err();
    assert!(err.to_string().contains("within 10s"), "{err}");
    assert!(f.t0.elapsed() < TIMEOUT + Duration::from_secs(1));
    assert_eq!(f.calls(), ["request_shutdown", "terminate"]);
    assert_eq!(f.state(), ServiceState::Failed);
}

#[tokio::test(start_paused = true)]
async fn progress_that_stops_ends_the_start_one_timeout_later() {
    let f = fixture(Some(Duration::from_secs(30)), None);
    let err = f.host.start(&f.driver, &f.launch).await.unwrap_err();
    let elapsed = f.t0.elapsed();
    assert!(elapsed >= Duration::from_secs(40) && elapsed < Duration::from_secs(41), "{elapsed:?}");
    assert!(err.to_string().contains("within 40s"), "{err}");
    assert_eq!(f.calls(), ["request_shutdown", "terminate"]);
}

#[tokio::test(start_paused = true)]
async fn a_stop_during_a_slow_start_shuts_the_server_down_cleanly() {
    let f = fixture(Some(Duration::from_secs(3600)), None);
    let stop = async {
        tokio::time::sleep(Duration::from_secs(30)).await;
        f.host.stop(&f.driver, &f.launch.service).await
    };
    let (started, stopped) = tokio::join!(f.host.start(&f.driver, &f.launch), stop);
    assert_eq!(started, Err(PortError::Cancelled));
    stopped.unwrap();
    assert!(f.t0.elapsed() < Duration::from_secs(31), "the stop did not wait for the start to time out");
    assert_eq!(f.calls(), ["request_shutdown", "terminate"]);
    assert_eq!(f.state(), ServiceState::Stopped);
}

#[tokio::test(start_paused = true)]
async fn a_restart_reloads_a_running_server_in_place_when_the_driver_can() {
    let mut f = fixture(None, Some(Duration::ZERO));
    f.driver.reloads = true;
    f.host.start(&f.driver, &f.launch).await.unwrap();
    f.host.restart(&f.driver, &f.launch).await.unwrap();
    assert_eq!(f.calls(), ["reload"], "no stop, no new process");
    assert_eq!(f.state(), ServiceState::Running);
    assert_eq!(f.host.status(&f.launch.service).pid, Some(PID));
}

#[tokio::test(start_paused = true)]
async fn a_restart_without_reload_stops_and_starts() {
    let f = fixture(None, Some(Duration::ZERO));
    f.host.start(&f.driver, &f.launch).await.unwrap();
    f.host.restart(&f.driver, &f.launch).await.unwrap();
    assert_eq!(f.calls(), ["reload", "request_shutdown", "terminate"]);
    assert_eq!(f.state(), ServiceState::Running);
}

#[tokio::test(start_paused = true)]
async fn a_restart_of_a_stopped_server_starts_it() {
    let f = fixture(None, Some(Duration::ZERO));
    f.host.restart(&f.driver, &f.launch).await.unwrap();
    assert!(f.calls().is_empty(), "nothing to reload or stop");
    assert_eq!(f.state(), ServiceState::Running);
}

#[tokio::test(start_paused = true)]
async fn an_exit_reported_after_a_stop_is_not_a_failure() {
    let f = fixture(None, Some(Duration::ZERO));
    f.host.start(&f.driver, &f.launch).await.unwrap();
    let observer = f.supervisor_observer();
    f.host.stop(&f.driver, &f.launch.service).await.unwrap();
    observer.on_exit(Some(0));
    assert_eq!(f.state(), ServiceState::Stopped);
}
