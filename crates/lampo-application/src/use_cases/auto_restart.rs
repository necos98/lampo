//! Restarting the services that exit unexpectedly (setting `auto_restart_services`, on by
//! default). The runners publish `ServiceCrashed`; [`CrashRelay`], wrapped around the app's
//! event publisher, hands each crash to [`ServiceAutoRestart`], which starts the service again
//! through [`CrashRecovery`] (the environment: a stop by the user or a sync since the crash
//! cancels the restart) after the backoff of [`AutoRestartPolicy`], and publishes every step
//! (`ServiceAutoRestart`) for the GUI's notifications. A stop the user asked for never crashes:
//! the runners report it as a stop.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use async_trait::async_trait;
use lampo_domain::auto_restart::{AutoRestartPolicy, AutoRestartStep, RestartBudget};
use lampo_domain::events::DomainEvent;
use lampo_domain::ports::{Clock, EventPublisher, SettingsRepository, Timer};
use lampo_domain::service::ServiceId;
use tokio::sync::mpsc;

use super::lifecycle::ShutdownGate;
use crate::blocking::in_place;
use crate::error::AppResult;

/// A running service that exited without being asked to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Crash {
    pub service: ServiceId,
    /// Why it ended (`exited unexpectedly (exit code 1): …`).
    pub message: String,
}

/// The crashes [`CrashRelay`] saw, for [`ServiceAutoRestart::run`].
pub type CrashReceiver = mpsc::UnboundedReceiver<Crash>;

/// Publishes every event to the wrapped publisher and also hands the crashes to the receiver
/// returned by [`CrashRelay::new`]. Unbounded: one message per crash of a running process.
pub struct CrashRelay {
    inner: Arc<dyn EventPublisher>,
    crashes: mpsc::UnboundedSender<Crash>,
}

impl CrashRelay {
    pub fn new(inner: Arc<dyn EventPublisher>) -> (Self, CrashReceiver) {
        let (crashes, receiver) = mpsc::unbounded_channel();
        (Self { inner, crashes }, receiver)
    }
}

impl EventPublisher for CrashRelay {
    fn publish(&self, event: DomainEvent) {
        if let DomainEvent::ServiceCrashed { service, message } = &event {
            // A closed receiver (the watchdog is gone): nobody restarts services any more.
            let _ = self.crashes.send(Crash { service: service.clone(), message: message.clone() });
        }
        self.inner.publish(event);
    }
}

/// What starting a crashed service again came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Revival {
    /// Started now.
    Restarted,
    /// Up again already (a sync or the user started it meanwhile).
    AlreadyUp,
    /// Stopped since the crash, by the user, a sync or the shutdown: it stays down.
    NoLongerWanted,
}

/// Starts again a service that crashed, serialised with the rest of its lifecycle.
#[async_trait]
pub trait CrashRecovery: Send + Sync {
    /// Starts `service` with its current configuration unless it was stopped since it last
    /// started or the app is going away (`NoLongerWanted`), or it is up (`AlreadyUp`). A start
    /// that fails is an error.
    async fn revive(&self, service: &ServiceId) -> AppResult<Revival>;
}

pub struct ServiceAutoRestartDeps {
    pub recovery: Arc<dyn CrashRecovery>,
    /// `auto_restart_services`, read before every attempt.
    pub settings: Arc<dyn SettingsRepository>,
    pub events: Arc<dyn EventPublisher>,
    pub timer: Arc<dyn Timer>,
    pub clock: Arc<dyn Clock>,
    /// Closed while the app goes away: nothing is restarted then.
    pub gate: Arc<ShutdownGate>,
}

/// Where one service is in its automatic restarts.
#[derive(Default)]
struct Tracked {
    budget: RestartBudget,
    /// A restart sequence is running: a crash reported meanwhile adds nothing.
    recovering: bool,
}

/// Restarts crashed services: up to [`AutoRestartPolicy::max_attempts`] attempts in a row with
/// a doubling wait, the count starting over once a service ran stably or the sequence ended
/// without restarting it (someone else started it, it was stopped, …). Each step is published as
/// `ServiceAutoRestart`; when every attempt failed the service stays `Failed` (`GaveUp`).
pub struct ServiceAutoRestart {
    deps: ServiceAutoRestartDeps,
    policy: AutoRestartPolicy,
    services: Mutex<HashMap<ServiceId, Tracked>>,
}

impl ServiceAutoRestart {
    pub fn new(deps: ServiceAutoRestartDeps) -> Self {
        Self { deps, policy: AutoRestartPolicy::DEFAULT, services: Mutex::default() }
    }

    /// Handles every crash `crashes` delivers, each in a task of its own (a slow restart of one
    /// service never delays another's). Runs for the app's lifetime: spawned by the composition
    /// root.
    pub async fn run(self: Arc<Self>, mut crashes: CrashReceiver) {
        while let Some(crash) = crashes.recv().await {
            let this = self.clone();
            tokio::spawn(async move { this.recover(crash).await });
        }
    }

    /// The restart sequence of one crash: wait, start, and again after a failed start, until the
    /// service runs, is no longer wanted, the setting is turned off, the app goes away or the
    /// attempts are used up.
    pub async fn recover(&self, crash: Crash) {
        let Crash { service, mut message } = crash;
        if !self.enabled() || !self.claim(&service) {
            return;
        }
        let mut recovering = Recovering { owner: self, service: &service, restarted: false };
        loop {
            let Some(attempt) = self.next_attempt(&service) else {
                tracing::warn!("{service} exited unexpectedly {} times in a row: not restarted again", self.policy.max_attempts);
                self.publish(&service, AutoRestartStep::GaveUp { attempts: self.policy.max_attempts, message });
                return;
            };
            let delay = self.policy.delay(attempt);
            self.publish(&service, AutoRestartStep::Scheduled { attempt, max_attempts: self.policy.max_attempts, delay });
            self.deps.timer.sleep(delay).await;
            if !self.enabled() {
                return;
            }
            match self.deps.recovery.revive(&service).await {
                Ok(Revival::Restarted) => {
                    let now = self.deps.clock.now_unix_ms();
                    self.tracked(|services| services.entry(service.clone()).or_default().budget.restarted(now));
                    recovering.restarted = true;
                    tracing::info!("{service} restarted after exiting unexpectedly (attempt {attempt})");
                    self.publish(&service, AutoRestartStep::Restarted { attempt });
                    return;
                }
                Ok(Revival::AlreadyUp | Revival::NoLongerWanted) => return,
                Err(e) => {
                    tracing::warn!("restarting {service} (attempt {attempt}) failed: {}", e.message);
                    message = e.message;
                }
            }
        }
    }

    /// The setting is on and the app is not going away. Unreadable settings count as off.
    fn enabled(&self) -> bool {
        if self.deps.gate.is_closed() {
            return false;
        }
        match in_place(|| self.deps.settings.load()) {
            Ok(settings) => settings.auto_restart_services,
            Err(e) => {
                tracing::warn!("crashed services are not restarted: the settings cannot be read: {e}");
                false
            }
        }
    }

    /// Marks `service` as recovering; `false` when a sequence already runs for it.
    fn claim(&self, service: &ServiceId) -> bool {
        self.tracked(|services| !std::mem::replace(&mut services.entry(service.clone()).or_default().recovering, true))
    }

    fn next_attempt(&self, service: &ServiceId) -> Option<u32> {
        let now = self.deps.clock.now_unix_ms();
        self.tracked(|services| services.entry(service.clone()).or_default().budget.next_attempt(&self.policy, now))
    }

    fn tracked<T>(&self, f: impl FnOnce(&mut HashMap<ServiceId, Tracked>) -> T) -> T {
        f(&mut lock(&self.services))
    }

    fn publish(&self, service: &ServiceId, step: AutoRestartStep) {
        self.deps.events.publish(DomainEvent::ServiceAutoRestart { service: service.clone(), step });
    }
}

/// Ends the recovery of `service` however [`ServiceAutoRestart::recover`] returns. Only a
/// restart of its own keeps the attempts (until the service runs stably); any other end (given
/// up, started by someone else, stopped, the setting turned off, the app going away) forgets
/// them, so whatever starts the service next starts afresh. Released and forgotten under one
/// lock: a crash reported meanwhile finds either this sequence or none.
struct Recovering<'a> {
    owner: &'a ServiceAutoRestart,
    service: &'a ServiceId,
    restarted: bool,
}

impl Drop for Recovering<'_> {
    fn drop(&mut self) {
        self.owner.tracked(|services| {
            if !self.restarted {
                services.remove(self.service);
            } else if let Some(tracked) = services.get_mut(self.service) {
                tracked.recovering = false;
            }
        });
    }
}

/// The table stays usable after a panic elsewhere: it only holds counters.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}
