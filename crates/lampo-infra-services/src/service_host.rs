//! What every supervised service runner shares: the status table and its events, the observer
//! that writes captured output to `<log_dir>\output.log` (rotated by size), and the
//! start/restart/stop sequence (port check → spawn via [`ProcessSupervisor`] → health check →
//! running, the deadline pushed forward while a slow start keeps making progress; a restart
//! reloads in place when the driver can, else stops and starts; graceful request →
//! `terminate`). A runner only describes its service through [`ServiceDriver`]: command line,
//! readiness ports and probe, reload and shutdown requests, failure diagnostics. Adding a
//! service kind means adding a driver and a runner type; this module does not change.

mod driver;
mod observer;
mod readiness;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use lampo_domain::ports::{ChildId, EventPublisher, ProcessSupervisor};
use lampo_domain::service::{ServiceId, ServiceLaunch, ServiceState, ServiceStatus};
use lampo_domain::tcp_port::Port;
use lampo_domain::{PortError, PortResult};
use lampo_infra_fs::ensure_dir;
use tokio::sync::watch;

pub use driver::ServiceDriver;
use observer::{Instance, OUTPUT_LOG, ServiceObserver, StatusBoard};
use readiness::{Awaited, Readiness};

use crate::rotating_log::RotatingLog;
use crate::win;

const READY_POLL: Duration = Duration::from_millis(250);
/// Longest start, however long [`ServiceDriver::startup_progress`] keeps moving.
pub(crate) const MAX_STARTUP: Duration = Duration::from_secs(60 * 60);

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

/// Start/stop/restart/status for every instance of one runner.
pub struct ServiceHost {
    supervisor: Arc<dyn ProcessSupervisor>,
    board: Arc<StatusBoard>,
    /// Serialises start/stop/restart per instance.
    locks: Mutex<HashMap<ServiceId, Arc<tokio::sync::Mutex<()>>>>,
}

impl ServiceHost {
    pub fn new(supervisor: Arc<dyn ProcessSupervisor>, events: Arc<dyn EventPublisher>) -> Self {
        Self { supervisor, board: Arc::new(StatusBoard { instances: Mutex::default(), events }), locks: Mutex::default() }
    }

    fn lock_for(&self, service: &ServiceId) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = self.locks.lock().unwrap_or_else(|p| p.into_inner());
        locks.entry(service.clone()).or_default().clone()
    }

    pub fn status(&self, service: &ServiceId) -> ServiceStatus {
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

    /// The process of `generation` is ready with `launch`, unless a stop or an exit overtook it.
    fn mark_running(&self, launch: &ServiceLaunch, generation: u64) {
        self.board.update(&launch.service, |inst| {
            if inst.generation == generation && inst.child.is_some() && !inst.stopping {
                inst.launch = Some(launch.clone());
                inst.status.state = ServiceState::Running;
                inst.status.message = None;
            }
        });
    }

    /// Starts the instance unless it is already running; returns once it is ready (or failed).
    pub async fn start(&self, driver: &dyn ServiceDriver, launch: &ServiceLaunch) -> PortResult<()> {
        let lock = self.lock_for(&launch.service);
        let _guard = lock.lock().await;
        self.start_locked(driver, launch).await
    }

    async fn start_locked(&self, driver: &dyn ServiceDriver, launch: &ServiceLaunch) -> PortResult<()> {
        let service = &launch.service;
        if let Some(inst) = self.board.get(service) {
            if inst.status.state == ServiceState::Running && inst.child.is_some_and(|c| self.supervisor.is_alive(c)) {
                return Ok(());
            }
        }
        for port in launch.ports() {
            if let Ok(Some(pid)) = win::tcp_listener_pid(port.get()) {
                let owner = win::describe_process(pid);
                self.fail(service, format!("port {port} is already in use by {owner}"));
                return Err(PortError::PortInUse { port: port.get(), owner: Some(owner) });
            }
        }
        let spec = driver.command(launch)?;
        let ports = driver.ready_ports(launch)?;
        let marker = driver.start_marker(launch);
        ensure_dir(&launch.log_dir)?;
        let log = RotatingLog::open(&launch.log_dir.join(OUTPUT_LOG))?;

        let mut generation = 0;
        self.board.update(service, |inst| {
            inst.generation += 1;
            generation = inst.generation;
            inst.stopping = false;
            inst.launch = Some(launch.clone());
            inst.status = ServiceStatus { service: service.clone(), state: ServiceState::Starting, pid: None, port: Some(launch.port), message: None };
        });
        let (exited_tx, exited) = watch::channel(false);
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
            inst.exited = Some(exited.clone());
        });

        let mut awaited = Awaited { child: spawned.id, generation, ports, exited };
        let reason = match self.await_ready(driver, launch, &mut awaited).await {
            Readiness::Ready => {
                self.mark_running(launch, generation);
                return Ok(());
            }
            Readiness::StopRequested => {
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
            Readiness::Exited => "exited during startup".to_string(),
            Readiness::TimedOut(reason) => {
                let _ = self.shut_down(driver, launch, spawned.id, Some(spawned.pid)).await;
                reason
            }
        };
        Err(self.start_failed(driver, launch, marker, &observer, &reason))
    }

    /// Records a failed start and builds its error from the driver's diagnostics (or the last
    /// captured output lines).
    fn start_failed(&self, driver: &dyn ServiceDriver, launch: &ServiceLaunch, marker: u64, observer: &ServiceObserver, reason: &str) -> PortError {
        let mut lines = driver.failure_lines(launch, marker);
        if lines.is_empty() {
            lines = observer.recent_lines();
            lines.drain(..lines.len().saturating_sub(10));
        }
        let headline = format!("{} {reason}", driver.program());
        let detail = lines.first().map_or(headline.clone(), |l| format!("{headline}: {l}"));
        self.fail(&launch.service, detail);
        if lines.iter().any(|l| driver.is_port_conflict(l)) {
            return PortError::PortInUse { port: launch.port.get(), owner: win::tcp_listener_pid(launch.port.get()).ok().flatten().map(win::describe_process) };
        }
        let message = if lines.is_empty() { headline } else { format!("{headline}\n{}", lines.join("\n")) };
        PortError::process(driver.program(), message)
    }

    /// Applies a new configuration. In place through [`ServiceDriver::reload`] when the instance
    /// runs on the same port, ready once it listens on [`ServiceDriver::ready_ports`] again;
    /// otherwise, or when the reloaded process exits or does not get ready, stop then start. Not
    /// running: start.
    pub async fn restart(&self, driver: &dyn ServiceDriver, launch: &ServiceLaunch) -> PortResult<()> {
        let service = &launch.service;
        let lock = self.lock_for(service);
        let _guard = lock.lock().await;
        let Some(inst) = self.board.get(service).filter(|i| i.child.is_some_and(|c| self.supervisor.is_alive(c))) else {
            return self.start_locked(driver, launch).await;
        };
        // The new configuration must be one a fresh start would accept.
        driver.command(launch)?;
        let ports = driver.ready_ports(launch)?;
        if self.reloaded(driver, launch, &inst, ports).await {
            return Ok(());
        }
        self.stop_locked(driver, service).await?;
        self.start_locked(driver, launch).await
    }

    /// Whether the running `inst` applied the new configuration in place and is ready again.
    async fn reloaded(&self, driver: &dyn ServiceDriver, launch: &ServiceLaunch, inst: &Instance, ports: Vec<Port>) -> bool {
        let (Some(child), Some(pid), Some(exited)) = (inst.child, inst.status.pid, inst.exited.clone()) else { return false };
        if inst.status.port != Some(launch.port) || !driver.reload(launch, pid).await {
            return false;
        }
        let mut awaited = Awaited { child, generation: inst.generation, ports, exited };
        if matches!(self.await_ready(driver, launch, &mut awaited).await, Readiness::Ready) {
            self.mark_running(launch, inst.generation);
            return true;
        }
        tracing::warn!("{} did not survive a reload: starting it again", driver.program());
        false
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

    pub async fn stop(&self, driver: &dyn ServiceDriver, service: &ServiceId) -> PortResult<()> {
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
        self.stop_locked(driver, service).await
    }

    async fn stop_locked(&self, driver: &dyn ServiceDriver, service: &ServiceId) -> PortResult<()> {
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
