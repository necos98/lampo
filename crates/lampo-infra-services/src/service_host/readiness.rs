//! Waiting for a spawned (or reloaded) process to become ready: it listens itself on every
//! port of [`ServiceDriver::ready_ports`] and passes the driver's probe, within the startup
//! deadline (pushed forward while a slow start keeps making progress).

use std::time::Duration;

use lampo_domain::ports::ChildId;
use lampo_domain::service::{ServiceId, ServiceLaunch, ServiceState};
use lampo_domain::tcp_port::Port;
use tokio::sync::watch;
use tokio::time::Instant;

use super::{MAX_STARTUP, READY_POLL, ServiceDriver, ServiceHost};
use crate::startup_deadline::StartupDeadline;

/// How a wait for readiness ended.
pub(super) enum Readiness {
    Ready,
    /// A stop arrived while waiting (only during a start).
    StopRequested,
    Exited,
    /// The deadline passed; the reason for the failure message.
    TimedOut(String),
}

/// The process being waited for.
pub(super) struct Awaited {
    pub(super) child: ChildId,
    pub(super) generation: u64,
    pub(super) ports: Vec<Port>,
    /// Becomes `true` when the process exits.
    pub(super) exited: watch::Receiver<bool>,
}

impl ServiceHost {
    pub(super) async fn await_ready(&self, driver: &dyn ServiceDriver, launch: &ServiceLaunch, awaited: &mut Awaited) -> Readiness {
        let service = &launch.service;
        let mut deadline = StartupDeadline::new(Instant::now(), driver.startup_timeout(), MAX_STARTUP, driver.startup_progress(launch));
        let mut slow = false;
        loop {
            if self.stop_requested(service, awaited.generation) {
                return Readiness::StopRequested;
            }
            if *awaited.exited.borrow() {
                return Readiness::Exited;
            }
            // The listener may be a child of the spawned process (mysqld 8 runs a monitor
            // process on Windows, httpd a worker child); anything else on the port is somebody
            // else's server.
            let missing: Vec<Port> = awaited.ports.iter().copied().filter(|&port| !self.supervisor.owns_listener(awaited.child, port)).collect();
            if missing.is_empty() && driver.probe(launch).await {
                return Readiness::Ready;
            }
            let now = Instant::now();
            deadline.observe(now, driver.startup_progress(launch));
            if deadline.expired(now) {
                return Readiness::TimedOut(timeout_reason(launch.port, &missing, deadline.elapsed(now)));
            }
            if !slow && deadline.is_extended(now) {
                slow = true;
                self.board.update(service, |inst| {
                    if inst.generation == awaited.generation && inst.status.state == ServiceState::Starting {
                        inst.status.message = Some(driver.slow_start_detail().to_string());
                    }
                });
            }
            tokio::select! {
                _ = tokio::time::sleep(READY_POLL) => {}
                _ = awaited.exited.changed() => {}
            }
        }
    }

    /// Whether a stop arrived while the start of `generation` was waiting for readiness.
    fn stop_requested(&self, service: &ServiceId, generation: u64) -> bool {
        self.board.get(service).is_some_and(|inst| inst.generation == generation && inst.stopping)
    }
}

/// Names the ports other than `port` the process never listened on, if any; otherwise the
/// service did not answer on `port`.
fn timeout_reason(port: Port, missing: &[Port], waited: Duration) -> String {
    let secs = waited.as_secs();
    if missing.iter().all(|&p| p == port) {
        return format!("did not accept connections on port {port} within {secs}s");
    }
    let noun = if missing.len() == 1 { "port" } else { "ports" };
    let list = missing.iter().map(Port::to_string).collect::<Vec<_>>().join(", ");
    format!("did not listen on {noun} {list} within {secs}s")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeouts_name_the_ports_never_listened_on() {
        let waited = Duration::from_secs(20);
        assert_eq!(timeout_reason(Port::of(80), &[], waited), "did not accept connections on port 80 within 20s");
        assert_eq!(timeout_reason(Port::of(80), &[Port::of(80)], waited), "did not accept connections on port 80 within 20s");
        assert_eq!(timeout_reason(Port::of(80), &[Port::of(8101)], waited), "did not listen on port 8101 within 20s");
        assert_eq!(timeout_reason(Port::of(80), &[Port::of(80), Port::of(8101)], waited), "did not listen on ports 80, 8101 within 20s");
    }
}
