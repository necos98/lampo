//! Automatic restart of crashed services: the backoff and the attempt limit of the watchdog
//! (against a scripted recovery), and the recovery of the environment, which never restarts a
//! service stopped since its crash.

use std::collections::VecDeque;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use lampo_domain::auto_restart::AutoRestartStep;
use lampo_domain::events::DomainEvent;
use lampo_domain::mysql::MysqlSeries;
use lampo_domain::ports::{EventPublisher, SettingsRepository};
use lampo_domain::service::{ServiceId, ServiceState};
use tokio::sync::Notify;

use super::{cli, gui};
use crate::dto::ServicesRequest;
use crate::error::{AppError, AppResult};
use crate::testing::{ManualClock, RecordingEvents, World};
use crate::use_cases::{Crash, CrashRecovery, CrashRelay, EnvironmentSync, Revival, ServiceAutoRestart, ServiceAutoRestartDeps, ShutdownGate};

const STABLE_MS: u64 = 60_000;

fn mysql() -> ServiceId {
    MysqlSeries::DEFAULT.service_id()
}

fn crash_of(service: &ServiceId) -> Crash {
    Crash { service: service.clone(), message: "exited unexpectedly (exit code 1)".into() }
}

/// Answers `revive` from a script (then `Restarted`); `hold` makes a call wait for a release.
#[derive(Default)]
struct ScriptedRecovery {
    answers: Mutex<VecDeque<AppResult<Revival>>>,
    calls: Mutex<usize>,
    hold: Option<Arc<Notify>>,
}

impl ScriptedRecovery {
    fn answering(answers: impl IntoIterator<Item = AppResult<Revival>>) -> Arc<Self> {
        Arc::new(Self { answers: Mutex::new(answers.into_iter().collect()), ..Self::default() })
    }

    fn calls(&self) -> usize {
        *self.calls.lock().unwrap()
    }
}

#[async_trait]
impl CrashRecovery for ScriptedRecovery {
    async fn revive(&self, _service: &ServiceId) -> AppResult<Revival> {
        *self.calls.lock().unwrap() += 1;
        if let Some(hold) = &self.hold {
            hold.notified().await;
        }
        self.answers.lock().unwrap().pop_front().unwrap_or(Ok(Revival::Restarted))
    }
}

struct Watchdog {
    restart: Arc<ServiceAutoRestart>,
    clock: Arc<ManualClock>,
    events: Arc<RecordingEvents>,
}

impl Watchdog {
    /// The automatic restart steps published, in order.
    fn steps(&self) -> Vec<AutoRestartStep> {
        self.events
            .all()
            .into_iter()
            .filter_map(|e| match e {
                DomainEvent::ServiceAutoRestart { step, .. } => Some(step),
                _ => None,
            })
            .collect()
    }

    fn delays(&self) -> Vec<u64> {
        self.steps()
            .into_iter()
            .filter_map(|s| match s {
                AutoRestartStep::Scheduled { delay, .. } => Some(delay.as_secs()),
                _ => None,
            })
            .collect()
    }
}

fn watchdog(w: &World, recovery: Arc<dyn CrashRecovery>, gate: Arc<ShutdownGate>) -> Watchdog {
    let clock = Arc::new(ManualClock::default());
    let events = Arc::new(RecordingEvents::default());
    let restart =
        Arc::new(ServiceAutoRestart::new(ServiceAutoRestartDeps { recovery, settings: w.settings.clone(), events: events.clone(), timer: clock.clone(), clock: clock.clone(), gate }));
    Watchdog { restart, clock, events }
}

fn scheduled(attempt: u32, secs: u64) -> AutoRestartStep {
    AutoRestartStep::Scheduled { attempt, max_attempts: 3, delay: Duration::from_secs(secs) }
}

#[tokio::test]
async fn a_crashed_service_restarts_after_two_seconds() {
    let w = World::with_basics();
    let recovery = ScriptedRecovery::answering([]);
    let dog = watchdog(&w, recovery.clone(), Arc::default());
    dog.restart.recover(crash_of(&mysql())).await;
    assert_eq!(dog.steps(), [scheduled(1, 2), AutoRestartStep::Restarted { attempt: 1 }]);
    assert_eq!(dog.clock.now_ms.load(Ordering::SeqCst), 2_000, "waited before starting");
    assert_eq!(recovery.calls(), 1);
}

#[tokio::test]
async fn three_failed_starts_two_four_and_eight_seconds_apart_then_it_gives_up() {
    let w = World::with_basics();
    let failure = |n: u32| Err(AppError::internal(format!("start {n} failed")));
    let recovery = ScriptedRecovery::answering([failure(1), failure(2), failure(3)]);
    let dog = watchdog(&w, recovery.clone(), Arc::default());
    dog.restart.recover(crash_of(&mysql())).await;
    assert_eq!(dog.delays(), [2, 4, 8]);
    assert_eq!(dog.steps().last(), Some(&AutoRestartStep::GaveUp { attempts: 3, message: "start 3 failed".into() }));
    assert_eq!(recovery.calls(), 3, "no fourth attempt");
}

#[tokio::test]
async fn crashes_in_a_row_use_up_the_attempts_and_a_stable_run_starts_over() {
    let w = World::with_basics();
    let dog = watchdog(&w, ScriptedRecovery::answering([]), Arc::default());
    let crash = || dog.restart.recover(crash_of(&mysql()));
    crash().await;
    crash().await;
    assert_eq!(dog.delays(), [2, 4], "the second crash came before the service ran stably");

    dog.clock.now_ms.fetch_add(STABLE_MS, Ordering::SeqCst);
    crash().await;
    assert_eq!(dog.delays(), [2, 4, 2], "a minute of running starts over");
    crash().await;
    crash().await;
    crash().await;
    assert_eq!(dog.steps().last(), Some(&AutoRestartStep::GaveUp { attempts: 3, message: "exited unexpectedly (exit code 1)".into() }));

    // Giving up forgets the service: started again by hand, its next crash starts over.
    crash().await;
    assert_eq!(dog.delays().last(), Some(&2));
}

#[tokio::test]
async fn nothing_restarts_when_the_setting_is_off() {
    let w = World::with_basics();
    let mut settings = w.settings.load().unwrap();
    settings.auto_restart_services = false;
    w.settings.save(&settings).unwrap();
    let recovery = ScriptedRecovery::answering([]);
    let dog = watchdog(&w, recovery.clone(), Arc::default());
    dog.restart.recover(crash_of(&mysql())).await;
    assert!(dog.steps().is_empty());
    assert_eq!(recovery.calls(), 0);
}

#[tokio::test]
async fn nothing_restarts_while_the_app_goes_away() {
    let w = World::with_basics();
    let shutdown = w.shutdown();
    shutdown.run().await.unwrap();
    let recovery = ScriptedRecovery::answering([]);
    let dog = watchdog(&w, recovery.clone(), shutdown.gate());
    dog.restart.recover(crash_of(&mysql())).await;
    assert!(dog.steps().is_empty());
    assert_eq!(recovery.calls(), 0);
}

#[tokio::test]
async fn a_service_stopped_since_its_crash_is_left_alone() {
    let w = World::with_basics();
    let recovery = ScriptedRecovery::answering([Ok(Revival::NoLongerWanted)]);
    let dog = watchdog(&w, recovery.clone(), Arc::default());
    dog.restart.recover(crash_of(&mysql())).await;
    assert_eq!(dog.steps(), [scheduled(1, 2)], "no restart, no failure");
    assert_eq!(recovery.calls(), 1);
}

#[tokio::test]
async fn a_crash_reported_while_a_restart_runs_adds_nothing() {
    let w = World::with_basics();
    let hold = Arc::new(Notify::new());
    let recovery = Arc::new(ScriptedRecovery { hold: Some(hold.clone()), ..ScriptedRecovery::default() });
    let dog = watchdog(&w, recovery.clone(), Arc::default());
    let second = async {
        while recovery.calls() == 0 {
            tokio::task::yield_now().await;
        }
        dog.restart.recover(crash_of(&mysql())).await;
        hold.notify_one();
    };
    tokio::join!(dog.restart.recover(crash_of(&mysql())), second);
    assert_eq!(recovery.calls(), 1);
    assert_eq!(dog.steps(), [scheduled(1, 2), AutoRestartStep::Restarted { attempt: 1 }]);
}

#[tokio::test]
async fn the_relay_hands_crashes_to_the_watchdog_and_forwards_every_event() {
    let w = World::with_basics();
    let gui_events = Arc::new(RecordingEvents::default());
    let (relay, crashes) = CrashRelay::new(gui_events.clone());
    let recovery = ScriptedRecovery::answering([]);
    let dog = watchdog(&w, recovery.clone(), Arc::default());
    let running = tokio::spawn(dog.restart.clone().run(crashes));

    let crashed = DomainEvent::ServiceCrashed { service: mysql(), message: "exited unexpectedly (killed)".into() };
    relay.publish(crashed.clone());
    relay.publish(DomainEvent::SettingsChanged);
    assert_eq!(gui_events.all(), [crashed, DomainEvent::SettingsChanged]);

    drop(relay);
    running.await.unwrap();
    // The spawned recovery finishes on its own task.
    let finished = async {
        while dog.steps().len() < 2 {
            tokio::task::yield_now().await;
        }
    };
    tokio::time::timeout(Duration::from_secs(5), finished).await.expect("the crash was never handled");
    assert_eq!(recovery.calls(), 1);
    assert_eq!(dog.steps(), [scheduled(1, 2), AutoRestartStep::Restarted { attempt: 1 }]);
}

// --- the environment's recovery -------------------------------------------------------------

fn services(id: &ServiceId) -> ServicesRequest {
    ServicesRequest { services: vec![id.to_string()] }
}

/// MySQL 8.4 started by the sync at app start, then crashed.
async fn crashed_mysql() -> World {
    let w = World::with_basics();
    w.hosts_in_sync();
    w.env.sync(&gui()).await.unwrap();
    assert_eq!(w.mysql.state(&mysql()), ServiceState::Running);
    w.mysql.set(&mysql(), ServiceState::Failed);
    w.clear_calls();
    w
}

#[tokio::test]
async fn the_environment_starts_a_crashed_service_again() {
    let w = crashed_mysql().await;
    let dog = watchdog(&w, w.env.clone(), Arc::default());
    dog.restart.recover(crash_of(&mysql())).await;
    assert_eq!(w.mysql.calls(), ["prepare:mysql-8.4", "start:mysql-8.4"]);
    assert_eq!(w.mysql.state(&mysql()), ServiceState::Running);
    assert_eq!(dog.steps().last(), Some(&AutoRestartStep::Restarted { attempt: 1 }));
}

#[tokio::test]
async fn a_service_the_user_stopped_after_its_crash_stays_stopped() {
    let w = crashed_mysql().await;
    w.service_control.down(services(&mysql())).await.unwrap();
    w.clear_calls();
    assert_eq!(w.env.revive(&mysql()).await.unwrap(), Revival::NoLongerWanted);
    assert!(w.mysql.calls().is_empty(), "{:?}", w.mysql.calls());
    assert_eq!(w.mysql.state(&mysql()), ServiceState::Stopped);
}

#[tokio::test]
async fn a_service_up_again_is_not_started_twice() {
    let w = crashed_mysql().await;
    w.service_control.up(services(&mysql()), &cli()).await.unwrap();
    w.clear_calls();
    assert_eq!(w.env.revive(&mysql()).await.unwrap(), Revival::AlreadyUp);
    assert!(w.mysql.calls().is_empty());
}

#[tokio::test]
async fn a_service_never_started_by_the_app_is_not_wanted() {
    let w = World::with_basics();
    let redis: ServiceId = "redis".parse().unwrap();
    assert_eq!(w.env.revive(&redis).await.unwrap(), Revival::NoLongerWanted);
    assert!(w.redis.calls().is_empty());
}
