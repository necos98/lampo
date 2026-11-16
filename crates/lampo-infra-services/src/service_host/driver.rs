//! What a runner tells [`super::ServiceHost`] about its service.

use std::time::Duration;

use async_trait::async_trait;
use lampo_domain::PortResult;
use lampo_domain::ports::CommandSpec;
use lampo_domain::service::ServiceLaunch;

/// Default [`ServiceDriver::is_port_conflict`] markers (lowercase): Windows' WSAEADDRINUSE text
/// and the bind errors of mysqld and redis-server.
const PORT_CONFLICT_MARKERS: [&str; 3] = ["already in use", "only one usage of each socket address", "bind on tcp/ip port"];

/// How one kind of service is run and checked. Implemented by each runner's driver.
#[async_trait]
pub trait ServiceDriver: Send + Sync {
    /// Short program name for messages (`mysqld`).
    fn program(&self) -> &'static str;
    fn command(&self, launch: &ServiceLaunch) -> PortResult<CommandSpec>;
    /// How long a start may go without becoming ready (and, see [`Self::startup_progress`],
    /// without making progress).
    fn startup_timeout(&self) -> Duration;
    /// Evidence that a slow start is still working (MySQL: the error log size, which grows
    /// during InnoDB crash recovery or a data dictionary upgrade). While the process is alive
    /// and this keeps changing, the startup deadline moves to `startup_timeout` after the last
    /// change (up to the host's one-hour limit). `None`: the deadline is fixed.
    fn startup_progress(&self, _launch: &ServiceLaunch) -> Option<u64> {
        None
    }
    /// The status message of a start running past `startup_timeout` (MySQL: `recovery`).
    fn slow_start_detail(&self) -> &'static str {
        "still starting"
    }
    /// How long a graceful shutdown may take before the process is killed.
    fn stop_grace(&self) -> Duration;
    /// The ports the spawned process itself must listen on before [`Self::probe`] is tried
    /// (Apache: every `Listen` of its configuration). Default: `launch.port`.
    fn ready_ports(&self, launch: &ServiceLaunch) -> PortResult<Vec<u16>> {
        Ok(vec![launch.port])
    }
    /// One readiness attempt: the service speaks its protocol on `launch.port`. Only called
    /// once every [`Self::ready_ports`] listener is the spawned process itself.
    async fn probe(&self, launch: &ServiceLaunch) -> bool;
    /// Asks the running process to apply a new configuration in place (Apache: graceful
    /// restart). `false`: not supported or not delivered, the host stops and starts it instead.
    async fn reload(&self, _launch: &ServiceLaunch, _pid: u32) -> bool {
        false
    }
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
    /// Whether a line explaining a failed start says the port was taken (the start then fails
    /// with `PortInUse`).
    fn is_port_conflict(&self, line: &str) -> bool {
        let line = line.to_ascii_lowercase();
        PORT_CONFLICT_MARKERS.iter().any(|marker| line.contains(marker))
    }
}
