//! The live status table of a host's instances and the observer of each spawned process:
//! captured output goes to `output.log`, to `ServiceLog` events and to the last lines kept for
//! failure messages; an exit becomes a status change.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use lampo_domain::events::DomainEvent;
use lampo_domain::ports::{ChildId, EventPublisher, ProcessObserver};
use lampo_domain::service::{LogStream, ServiceId, ServiceLaunch, ServiceState, ServiceStatus};
use tokio::sync::watch;

use super::{iso_utc, now_ms};
use crate::rotating_log::RotatingLog;

/// Captured stdout/stderr of a service, in its log dir (`LogSource::ServiceOutput`).
pub(super) const OUTPUT_LOG: &str = "output.log";
const RECENT_LINES: usize = 20;

#[derive(Debug, Clone)]
pub(super) struct Instance {
    pub(super) status: ServiceStatus,
    pub(super) child: Option<ChildId>,
    pub(super) launch: Option<ServiceLaunch>,
    pub(super) stopping: bool,
    pub(super) generation: u64,
    /// Becomes `true` when the process of `generation` exits.
    pub(super) exited: Option<watch::Receiver<bool>>,
}

/// Live statuses, publishing `ServiceStatusChanged` on every change.
pub(super) struct StatusBoard {
    pub(super) instances: Mutex<HashMap<ServiceId, Instance>>,
    pub(super) events: Arc<dyn EventPublisher>,
}

impl StatusBoard {
    pub(super) fn get(&self, service: &ServiceId) -> Option<Instance> {
        self.instances.lock().ok()?.get(service).cloned()
    }

    pub(super) fn update(&self, service: &ServiceId, f: impl FnOnce(&mut Instance)) {
        let changed = {
            let Ok(mut map) = self.instances.lock() else { return };
            let inst = map.entry(service.clone()).or_insert_with(|| Instance {
                status: ServiceStatus::stopped(service.clone()),
                child: None,
                launch: None,
                stopping: false,
                generation: 0,
                exited: None,
            });
            let before = inst.status.clone();
            f(inst);
            (inst.status != before).then(|| inst.status.clone())
        };
        if let Some(status) = changed {
            self.events.publish(DomainEvent::ServiceStatusChanged(status));
        }
    }
}

/// Writes captured output to `output.log`, publishes `ServiceLog`, remembers the last lines,
/// and turns an exit into a status change.
pub(super) struct ServiceObserver {
    pub(super) service: ServiceId,
    pub(super) generation: u64,
    pub(super) board: Arc<StatusBoard>,
    pub(super) log: Mutex<RotatingLog>,
    pub(super) recent: Mutex<VecDeque<String>>,
    pub(super) exited: watch::Sender<bool>,
}

impl ServiceObserver {
    pub(super) fn recent_lines(&self) -> Vec<String> {
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
            // An old process, or one a stop or a failed start already accounted for.
            if inst.generation != self.generation || inst.child.is_none() {
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
