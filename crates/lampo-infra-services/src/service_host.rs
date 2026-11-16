//! What every supervised service runner shares: the status table and its events, the observer
//! that writes captured output to `<log_dir>\output.log` (rotated by size), and the start/stop
//! sequence (port check → spawn via [`ProcessSupervisor`] → health check → running, the deadline
//! pushed forward while a slow start keeps making progress; graceful request → `terminate`).
//! A runner only describes its service through [`ServiceDriver`]: command line,
//! readiness probe, shutdown request, failure diagnostics. Adding a service kind means adding a
//! driver and a runner type; this module does not change.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use lampo_domain::events::DomainEvent;
use lampo_domain::ports::{ChildId, CommandSpec, EventPublisher, ProcessObserver, ProcessSupervisor};
use lampo_domain::service::{LogStream, ServiceId, ServiceLaunch, ServiceState, ServiceStatus};
use lampo_domain::{PortError, PortResult};
use tokio::sync::watch;
use tokio::time::Instant;

use crate::rotating_log::RotatingLog;
use crate::startup_deadline::StartupDeadline;
use crate::win;

/// Captured stdout/stderr of a service, in its log dir (`LogSource::ServiceOutput`).
const OUTPUT_LOG: &str = "output.log";
const RECENT_LINES: usize = 20;
const READY_POLL: Duration = Duration::from_millis(250);
/// Longest start, however long [`ServiceDriver::startup_progress`] keeps moving.
pub(crate) const MAX_STARTUP: Duration = Duration::from_secs(60 * 60);

/// How one kind of service is run and checked. Implemented by each runner's driver.
#[async_trait]
pub(crate) trait ServiceDriver: Send + Sync {
    /// Short program name for messages (`mysqld`).
    fn program(&self) -> &'static str;
    fn command(&self, launch: &ServiceLaunch) -> PortResult<CommandSpec>;
    /// How long a start may go without becoming ready (and, see [`Self::startup_progress`],
    /// without making progress).
    fn startup_timeout(&self) -> Duration;
    /// Evidence that a slow start is still working (MySQL: the error log size, which grows
    /// during InnoDB crash recovery or a data dictionary upgrade). While the process is alive
    /// and this keeps changing, the startup deadline moves to `startup_timeout` after the last
    /// change (up to [`MAX_STARTUP`]). `None`: the deadline is fixed.
    fn startup_progress(&self, _launch: &ServiceLaunch) -> Option<u64> {
        None
    }
    /// The status message of a start running past `startup_timeout` (MySQL: `recovery`).
    fn slow_start_detail(&self) -> &'static str {
        "still starting"
    }
    /// How long a graceful shutdown may take before the process is killed.
    fn stop_grace(&self) -> Duration;
    /// One readiness attempt: the service speaks its protocol on `launch.port`. Only called
    /// once the port's listener is the spawned process itself.
    async fn probe(&self, launch: &ServiceLaunch) -> bool;
    /// Asks the process to shut down cleanly (best effort; `terminate` follows).
    async fn request_shutdown(&self, launch: &ServiceLaunch, pid: u32);
    /// `request_shutdown` reaches the process by pid, not through `launch.port`: it is safe
    /// before the service listens and can never hit another server on the port. Otherwise it is
    /// only sent once the port's listener is the spawned process.
    fn shutdown_addresses_the_process(&self) -> bool {
        false
    }
    /// Remembered before a start (e.g. the error log size) and passed to `failure_lines`.
    fn start_marker(&self, _launch: &ServiceLaunch) -> u64 {
        0
    }
    /// The lines explaining a failed start (e.g. `[ERROR]` lines of the error log since the
    /// marker). Empty: the last captured output lines are used.
    fn failure_lines(&self, _launch: &ServiceLaunch, _marker: u64) -> Vec<String> {
        Vec::new()
    }
}

#[derive(Debug, Clone)]
struct Instance {
    status: ServiceStatus,
    child: Option<ChildId>,
    launch: Option<ServiceLaunch>,
    stopping: bool,
    generation: u64,
}

/// Live statuses, publishing `ServiceStatusChanged` on every change.
struct StatusBoard {
    instances: Mutex<HashMap<ServiceId, Instance>>,
    events: Arc<dyn EventPublisher>,
}

impl StatusBoard {
    fn get(&self, service: &ServiceId) -> Option<Instance> {
        self.instances.lock().ok()?.get(service).cloned()
    }

    fn update(&self, service: &ServiceId, f: impl FnOnce(&mut Instance)) {
        let changed = {
            let Ok(mut map) = self.instances.lock() else { return };
            let inst = map.entry(service.clone()).or_insert_with(|| Instance { status: ServiceStatus::stopped(service.clone()), child: None, launch: None, stopping: false, generation: 0 });
            let before = inst.status.clone();
            f(inst);
            (inst.status != before).then(|| inst.status.clone())
        };
        if let Some(status) = changed {
            self.events.publish(DomainEvent::ServiceStatusChanged(status));
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// `2026-11-13T10:15:30.123Z` (parsed back by the log reader for `--since`).
pub(crate) fn iso_utc(ms: u64) -> String {
    let secs = ms / 1000;
    let (days, rem) = ((secs / 86_400) as i64, secs % 86_400);
    // Civil-from-days (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z", rem / 3600, rem % 3600 / 60, rem % 60, ms % 1000)
}

/// Writes `content` atomically (a crash or a concurrent reader never sees half a file) unless
/// the file already holds exactly that.
pub(crate) fn write_if_changed(path: &Path, content: &str) -> PortResult<()> {
    if std::fs::read_to_string(path).is_ok_and(|c| c == content) {
        return Ok(());
    }
    lampo_infra_fs::write_atomic(path, content.as_bytes())
}

pub(crate) fn create_dir(dir: &Path) -> PortResult<()> {
    std::fs::create_dir_all(dir).map_err(|e| PortError::io(format!("creating {}", dir.display()), e))
}

/// Finds `exe` in a package: `<dir>\bin`, `<dir>`, or the same under a single top-level folder
/// (zips that wrap everything in `mysql-8.4.3-winx64\`).
pub(crate) fn find_executable(install_dir: &Path, exe: &str) -> PortResult<PathBuf> {
    let mut roots = vec![install_dir.to_path_buf()];
    if let Ok(entries) = std::fs::read_dir(install_dir) {
        roots.extend(entries.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_dir())).map(|e| e.path()));
    }
    roots
        .iter()
        .flat_map(|r| [r.join("bin").join(exe), r.join(exe)])
        .find(|p| p.is_file())
        .ok_or_else(|| PortError::not_found("executable", install_dir.join("bin").join(exe).display().to_string()))
}

/// Writes captured output to `output.log`, publishes `ServiceLog`, remembers the last lines,
/// and turns an exit into a status change.
struct ServiceObserver {
    service: ServiceId,
    generation: u64,
    board: Arc<StatusBoard>,
    log: Mutex<RotatingLog>,
    recent: Mutex<VecDeque<String>>,
    exited: watch::Sender<bool>,
}

impl ServiceObserver {
    fn recent_lines(&self) -> Vec<String> {
        self.recent.lock().map(|r| r.iter().cloned().collect()).unwrap_or_default()
    }
}

impl ProcessObserver for ServiceObserver {
    fn on_output(&self, stream: LogStream, line: &str) {
        let at = now_ms();
        if let Ok(mut log) = self.log.lock() {
            if let Err(e) = log.write(format!("{} {line}\n", iso_utc(at)).as_bytes()) {
                tracing::warn!("{}: output line not written to {OUTPUT_LOG}: {e}", self.service);
            }
        }
        if let Ok(mut recent) = self.recent.lock() {
            if recent.len() == RECENT_LINES {
                recent.pop_front();
            }
            recent.push_back(line.to_string());
        }
        self.board.events.publish(DomainEvent::ServiceLog { service: self.service.clone(), stream, line: line.to_string(), at_unix_ms: at });
    }

    fn on_exit(&self, code: Option<i32>) {
        let last = self.recent.lock().ok().and_then(|r| r.back().cloned());
        self.board.update(&self.service, |inst| {
            if inst.generation != self.generation {
                return;
            }
            inst.child = None;
            inst.status.pid = None;
            if inst.stopping {
                inst.status.state = ServiceState::Stopped;
                inst.status.message = None;
            } else {
                inst.status.state = ServiceState::Failed;
                let code = code.map_or("killed".to_string(), |c| format!("exit code {c}"));
                inst.status.message = Some(match last {
                    Some(l) => format!("exited unexpectedly ({code}): {l}"),
                    None => format!("exited unexpectedly ({code})"),
                });
            }
        });
        let _ = self.exited.send(true);
    }
}

/// Start/stop/status for every instance of one runner.
pub(crate) struct ServiceHost {
    supervisor: Arc<dyn ProcessSupervisor>,
    board: Arc<StatusBoard>,
    /// Serialises start/stop per instance.
    locks: Mutex<HashMap<ServiceId, Arc<tokio::sync::Mutex<()>>>>,
}

impl ServiceHost {
    pub(crate) fn new(supervisor: Arc<dyn ProcessSupervisor>, events: Arc<dyn EventPublisher>) -> Self {
        Self { supervisor, board: Arc::new(StatusBoard { instances: Mutex::default(), events }), locks: Mutex::default() }
    }

    fn lock_for(&self, service: &ServiceId) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = self.locks.lock().unwrap_or_else(|p| p.into_inner());
        locks.entry(service.clone()).or_default().clone()
    }

    pub(crate) fn status(&self, service: &ServiceId) -> ServiceStatus {
        self.board.get(service).map(|i| i.status).unwrap_or_else(|| ServiceStatus::stopped(service.clone()))
    }

    fn fail(&self, service: &ServiceId, message: String) {
        self.board.update(service, |inst| {
            inst.child = None;
            inst.stopping = false;
            inst.status.state = ServiceState::Failed;
            inst.status.pid = None;
            inst.status.message = Some(message);
        });
    }

    pub(crate) async fn start(&self, driver: &dyn ServiceDriver, launch: &ServiceLaunch) -> PortResult<()> {
        let service = &launch.service;
        let lock = self.lock_for(service);
        let _guard = lock.lock().await;
        if let Some(inst) = self.board.get(service) {
            if inst.status.state == ServiceState::Running && inst.child.is_some_and(|c| self.supervisor.is_alive(c)) {
                return Ok(());
            }
        }
        for port in launch.ports() {
            if let Ok(Some(pid)) = win::tcp_listener_pid(port) {
                let owner = win::describe_process(pid);
                self.fail(service, format!("port {port} is already in use by {owner}"));
                return Err(PortError::PortInUse { port, owner: Some(owner) });
            }
        }
        let spec = driver.command(launch)?;
        let marker = driver.start_marker(launch);
        create_dir(&launch.log_dir)?;
        let log = RotatingLog::open(&launch.log_dir.join(OUTPUT_LOG))?;

        let mut generation = 0;
        self.board.update(service, |inst| {
            inst.generation += 1;
            generation = inst.generation;
            inst.stopping = false;
            inst.launch = Some(launch.clone());
            inst.status = ServiceStatus { service: service.clone(), state: ServiceState::Starting, pid: None, port: Some(launch.port), message: None };
        });
        let (exited_tx, mut exited) = watch::channel(false);
        let observer = Arc::new(ServiceObserver { service: service.clone(), generation, board: self.board.clone(), log: Mutex::new(log), recent: Mutex::default(), exited: exited_tx });
        let spawned = match self.supervisor.spawn(spec, observer.clone()).await {
            Ok(s) => s,
            Err(e) => {
                self.fail(service, e.to_string());
                return Err(e);
            }
        };
        self.board.update(service, |inst| {
            inst.child = Some(spawned.id);
            inst.status.pid = Some(spawned.pid);
        });

        let mut deadline = StartupDeadline::new(Instant::now(), driver.startup_timeout(), MAX_STARTUP, driver.startup_progress(launch));
        let mut slow = false;
        let reason = loop {
            if self.stop_requested(service, generation) {
                let result = self.shut_down(driver, launch, spawned.id, Some(spawned.pid)).await;
                self.board.update(service, |inst| {
                    if inst.generation == generation {
                        inst.child = None;
                        inst.stopping = false;
                        inst.status = ServiceStatus::stopped(service.clone());
                    }
                });
                return result.and(Err(PortError::Cancelled));
            }
            if *exited.borrow() {
                break "exited during startup".to_string();
            }
            // The listener may be a child of the spawned process (mysqld 8 runs a monitor
            // process on Windows); anything else on the port is somebody else's server.
            if self.supervisor.owns_listener(spawned.id, launch.port) && driver.probe(launch).await {
                self.board.update(service, |inst| {
                    if inst.generation == generation && inst.child.is_some() && !inst.stopping {
                        inst.status.state = ServiceState::Running;
                        inst.status.message = None;
                    }
                });
                return Ok(());
            }
            let now = Instant::now();
            deadline.observe(now, driver.startup_progress(launch));
            if deadline.expired(now) {
                let _ = self.shut_down(driver, launch, spawned.id, Some(spawned.pid)).await;
                break format!("did not accept connections on port {} within {}s", launch.port, deadline.elapsed(now).as_secs());
            }
            if !slow && deadline.is_extended(now) {
                slow = true;
                self.board.update(service, |inst| {
                    if inst.generation == generation && inst.status.state == ServiceState::Starting {
                        inst.status.message = Some(driver.slow_start_detail().to_string());
                    }
                });
            }
            tokio::select! {
                _ = tokio::time::sleep(READY_POLL) => {}
                _ = exited.changed() => {}
            }
        };

        let mut lines = driver.failure_lines(launch, marker);
        if lines.is_empty() {
            lines = observer.recent_lines();
            lines.drain(..lines.len().saturating_sub(10));
        }
        let headline = format!("{} {reason}", driver.program());
        let detail = lines.first().map_or(headline.clone(), |l| format!("{headline}: {l}"));
        self.fail(service, detail);
        let port_conflict = lines.iter().any(|l| {
            let l = l.to_ascii_lowercase();
            l.contains("already in use") || l.contains("only one usage of each socket address") || l.contains("bind on tcp/ip port")
        });
        if port_conflict {
            return Err(PortError::PortInUse { port: launch.port, owner: win::tcp_listener_pid(launch.port).ok().flatten().map(win::describe_process) });
        }
        let message = if lines.is_empty() { headline } else { format!("{headline}\n{}", lines.join("\n")) };
        Err(PortError::process(driver.program(), message))
    }

    /// Whether a stop arrived while the start of `generation` was waiting for readiness.
    fn stop_requested(&self, service: &ServiceId, generation: u64) -> bool {
        self.board.get(service).is_some_and(|inst| inst.generation == generation && inst.stopping)
    }

    /// The driver's shutdown request (only when it cannot reach another server), then the
    /// supervisor's termination of the process tree after `stop_grace`.
    async fn shut_down(&self, driver: &dyn ServiceDriver, launch: &ServiceLaunch, child: ChildId, pid: Option<u32>) -> PortResult<()> {
        if let Some(pid) = pid.filter(|_| self.supervisor.is_alive(child)) {
            if driver.shutdown_addresses_the_process() || self.supervisor.owns_listener(child, launch.port) {
                driver.request_shutdown(launch, pid).await;
            }
        }
        self.supervisor.terminate(child, driver.stop_grace()).await
    }

    pub(crate) async fn stop(&self, driver: &dyn ServiceDriver, service: &ServiceId) -> PortResult<()> {
        // A start waiting for readiness (possibly a long MySQL recovery, when the app shuts down)
        // holds the lock: ask it to shut its process down cleanly and give up.
        self.board.update(service, |inst| {
            if inst.status.state == ServiceState::Starting {
                inst.stopping = true;
                inst.status.state = ServiceState::Stopping;
            }
        });
        let lock = self.lock_for(service);
        let _guard = lock.lock().await;
        let Some(inst) = self.board.get(service) else { return Ok(()) };
        let (Some(child), Some(launch)) = (inst.child, inst.launch.clone()) else {
            self.board.update(service, |i| {
                if i.status.state != ServiceState::Failed {
                    i.status = ServiceStatus::stopped(service.clone());
                }
            });
            return Ok(());
        };
        self.board.update(service, |i| {
            i.stopping = true;
            i.status.state = ServiceState::Stopping;
        });
        let result = self.shut_down(driver, &launch, child, inst.status.pid).await;
        self.board.update(service, |i| {
            i.child = None;
            i.stopping = false;
            i.status = ServiceStatus::stopped(service.clone());
        });
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_timestamps_roundtrip_through_the_log_parser() {
        assert_eq!(iso_utc(0), "1970-01-01T00:00:00.000Z");
        let ms = crate::logs::civil_to_unix_ms(2026, 11, 13, 10, 15, 30, 123).unwrap();
        assert_eq!(iso_utc(ms), "2026-11-13T10:15:30.123Z");
        assert_eq!(crate::logs::parse_timestamp(&format!("{} hello", iso_utc(ms))), Some(ms));
    }

    #[test]
    fn finds_executables_in_wrapped_packages() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("mysql-8.4.3-winx64").join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("mysqld.exe"), b"").unwrap();
        assert_eq!(find_executable(dir.path(), "mysqld.exe").unwrap(), bin.join("mysqld.exe"));
        assert!(matches!(find_executable(dir.path(), "redis-server.exe"), Err(PortError::NotFound { .. })));
    }
}
