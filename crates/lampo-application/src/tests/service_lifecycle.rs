//! One owner of the service lifecycle: manual starts/restarts/stops and database operations go
//! through the lock of the environment sync, and a running database operation keeps its MySQL
//! instance from being restarted or stopped by a sync.

use std::sync::atomic::Ordering;
use std::future::Future;
use std::time::Duration;

use lampo_domain::mysql::MysqlSeries;
use lampo_domain::ports::SettingsRepository;
use lampo_domain::progress::CancelFlag;
use lampo_domain::service::{ServiceId, ServiceState};
use lampo_domain::tcp_port::Port;

use super::{cli, gui};
use crate::dto::{DatabaseImportRequest, DatabaseRequest, ServicesRequest};
use crate::error::ErrorCode;
use crate::testing::{RecordingProgress, World};
use crate::use_cases::{EnvironmentSync, ServiceEnsurer};

const POSTPONED_CHANGE_TIMEOUT: Duration = Duration::from_secs(5);
/// A held start never completes by itself: a stop that waited for it would never return.
const STOP_TIMEOUT: Duration = Duration::from_secs(5);

fn default_mysql() -> ServiceId {
    MysqlSeries::DEFAULT.service_id()
}

fn services(ids: &[&ServiceId]) -> ServicesRequest {
    ServicesRequest { services: ids.iter().map(|id| id.to_string()).collect() }
}

fn move_default_mysql_to(w: &World, port: u16) {
    let mut s = w.settings.load().unwrap();
    s.services[0].port = Port::of(port);
    w.settings.save(&s).unwrap();
}

fn restarts_of(w: &World, id: &ServiceId) -> usize {
    let call = format!("restart:{id}");
    w.mysql.calls().iter().filter(|c| **c == call).count()
}

/// Runs the loop applying postponed changes until `done` holds; fails after a timeout.
async fn apply_postponed_until(w: &World, done: impl Fn() -> bool) {
    let applied = async {
        while !done() {
            tokio::task::yield_now().await;
        }
    };
    tokio::select! {
        () = w.env.apply_deferred_changes() => unreachable!("the loop never ends"),
        result = tokio::time::timeout(POSTPONED_CHANGE_TIMEOUT, applied) => result.expect("the postponed change was never applied"),
    }
}

/// Runs `first` and `second` concurrently; fails with `stuck` unless both complete in time.
async fn together<A: Future, B: Future>(first: A, second: B, stuck: &str) -> (A::Output, B::Output) {
    tokio::time::timeout(STOP_TIMEOUT, async { tokio::join!(first, second) }).await.expect(stuck)
}

#[tokio::test]
async fn a_service_started_outside_a_sync_is_restarted_when_its_port_changes() {
    let w = World::with_basics();
    w.hosts_in_sync();
    let report = w.service_control.up(services(&[&default_mysql()]), &cli()).await.unwrap();
    assert_eq!(report.started, ["mysql-8.4"]);

    move_default_mysql_to(&w, 3316);
    w.clear_calls();
    let report = w.env.sync(&gui()).await.unwrap();
    assert_eq!(report.restarted, ["mysql-8.4"], "the launch of the manual start was recorded");
    assert_eq!(w.mysql.launches.lock().unwrap().last().unwrap().port, 3316);
}

#[tokio::test]
async fn a_manual_restart_records_the_launch_the_sync_compares_with() {
    let w = World::with_basics();
    w.hosts_in_sync();
    w.env.sync(&gui()).await.unwrap();
    move_default_mysql_to(&w, 3316);

    let report = w.service_control.restart(services(&[&default_mysql()])).await.unwrap();
    assert_eq!(report.restarted, ["mysql-8.4"]);
    w.clear_calls();
    let report = w.env.sync(&gui()).await.unwrap();
    assert!(report.restarted.is_empty(), "already running with the new port: {report:?}");
    assert!(w.mysql.calls().is_empty());
}

#[tokio::test]
async fn a_sync_during_an_import_never_restarts_its_mysql_and_restarts_it_afterwards() {
    let w = World::with_basics();
    w.project("shop", "C:\\sites\\shop", false);
    w.hosts_in_sync();
    w.env.sync(&gui()).await.unwrap();
    w.db.hold_import.store(true, Ordering::SeqCst);
    let db = w.database_service();
    let (progress, cancel) = (RecordingProgress::default(), CancelFlag::new());
    let request = DatabaseImportRequest { site: "shop".into(), name: None, file: "D:\\dump.sql".into(), allow_database_switch: false, compat: None };

    let during_import = async {
        w.db.import_started.notified().await;
        move_default_mysql_to(&w, 3316);
        let synced = w.env.sync(&gui()).await;
        let restarted = w.service_control.restart(services(&[&default_mysql()])).await;
        let stopped = w.service_control.down(services(&[&default_mysql()])).await;
        w.db.import_resumed.notify_one();
        (synced, restarted, stopped)
    };
    let (imported, (synced, restarted, stopped)) = tokio::join!(db.import(&request, &progress, &cancel), during_import);

    imported.unwrap();
    let report = synced.unwrap();
    assert!(report.restarted.is_empty(), "{report:?}");
    assert!(report.warnings.iter().any(|m| m.contains("mysql-8.4") && m.contains("database operation")), "{:?}", report.warnings);
    assert_eq!(restarted.unwrap_err().code, ErrorCode::Conflict, "a manual restart is refused too");
    assert_eq!(stopped.unwrap_err().code, ErrorCode::Conflict, "and so is a manual stop");
    assert_eq!(restarts_of(&w, &default_mysql()), 0);
    assert_eq!(w.mysql.state(&default_mysql()), ServiceState::Running);

    // The import is over: the postponed restart is applied.
    apply_postponed_until(&w, || restarts_of(&w, &default_mysql()) == 1).await;
    assert_eq!(w.mysql.launches.lock().unwrap().last().unwrap().port, 3316);
}

#[tokio::test]
async fn a_leased_service_no_longer_needed_stops_once_released() {
    let w = World::with_basics();
    w.project("legacy", "C:\\sites\\legacy", true);
    w.update_project("legacy", |p| p.environment.set_mysql(Some(MysqlSeries::LEGACY_5_7)));
    w.hosts_in_sync();
    w.env.sync(&gui()).await.unwrap();
    let legacy = MysqlSeries::LEGACY_5_7.service_id();

    let lease = w.service_control.ensure_running(legacy.clone()).await.unwrap();
    w.update_project("legacy", |p| p.active = false);
    let report = w.env.sync(&gui()).await.unwrap();
    assert!(!report.stopped.contains(&legacy.to_string()), "{report:?}");
    assert!(report.warnings.iter().any(|m| m.contains("mysql-5.7") && m.contains("stops when the operation finishes")), "{:?}", report.warnings);
    assert_eq!(w.mysql.state(&legacy), ServiceState::Running);

    drop(lease);
    apply_postponed_until(&w, || w.mysql.state(&legacy) == ServiceState::Stopped).await;
}

#[tokio::test]
async fn a_lease_starts_a_stopped_service_and_ends_with_its_holder() {
    let w = World::with_basics();
    w.hosts_in_sync();
    let lease = w.service_control.ensure_running(default_mysql()).await.unwrap();
    assert_eq!(w.mysql.calls(), ["prepare:mysql-8.4", "start:mysql-8.4"]);
    assert_eq!(w.service_control.restart(services(&[&default_mysql()])).await.unwrap_err().code, ErrorCode::Conflict);

    drop(lease);
    let report = w.service_control.restart(services(&[&default_mysql()])).await.unwrap();
    assert_eq!(report.restarted, ["mysql-8.4"]);
    let report = w.service_control.down(services(&[&default_mysql()])).await.unwrap();
    assert_eq!(report.stopped, ["mysql-8.4"]);
}

#[tokio::test]
async fn a_database_operation_reaches_mysql_on_the_port_it_runs_on_while_its_restart_waits() {
    let w = World::with_basics();
    w.project("shop", "C:\\sites\\shop", false);
    w.hosts_in_sync();
    w.env.sync(&gui()).await.unwrap();
    w.db.hold_import.store(true, Ordering::SeqCst);
    let db = w.database_service();
    let (progress, cancel) = (RecordingProgress::default(), CancelFlag::new());
    let request = DatabaseImportRequest { site: "shop".into(), name: None, file: "D:\\dump.sql".into(), allow_database_switch: false, compat: None };

    let during_import = async {
        w.db.import_started.notified().await;
        move_default_mysql_to(&w, 3316);
        w.env.sync(&gui()).await.unwrap();
        let created = db.create(&DatabaseRequest { site: "shop".into(), name: Some("extra".into()) }).await;
        w.db.import_resumed.notify_one();
        created
    };
    let (imported, created) = tokio::join!(db.import(&request, &progress, &cancel), during_import);

    imported.unwrap();
    created.unwrap();
    assert_eq!(w.db.calls().iter().filter(|c| c.starts_with("create:")).collect::<Vec<_>>(), ["create:3306:extra"], "the restart onto 3316 is still postponed");
}

#[tokio::test]
async fn a_stop_gives_up_a_slow_manual_start_without_waiting_for_it() {
    let w = World::with_basics();
    w.hosts_in_sync();
    w.mysql.hold_starts.store(true, Ordering::SeqCst);
    let stop = async {
        w.mysql.until_starting(&default_mysql()).await;
        w.service_control.down(services(&[&default_mysql()])).await
    };
    let (started, stopped) = together(w.service_control.up(services(&[&default_mysql()]), &cli()), stop, "the stop waited for the start").await;

    assert_eq!(started.unwrap_err().code, ErrorCode::Cancelled);
    assert_eq!(stopped.unwrap().stopped, ["mysql-8.4"]);
    assert_eq!(w.mysql.state(&default_mysql()), ServiceState::Stopped);
}

#[tokio::test]
async fn a_stop_gives_up_the_slow_start_of_a_sync() {
    let w = World::with_basics();
    w.hosts_in_sync();
    w.mysql.hold_starts.store(true, Ordering::SeqCst);
    let stop = async {
        w.mysql.until_starting(&default_mysql()).await;
        w.service_control.down(services(&[&default_mysql()])).await
    };
    let (synced, stopped) = together(w.env.sync(&gui()), stop, "the stop waited for the sync").await;

    let report = synced.unwrap();
    assert!(report.started.is_empty(), "{report:?}");
    assert!(report.warnings.iter().any(|m| m.contains("mysql-8.4") && m.contains("stopped while it was starting")), "{:?}", report.warnings);
    assert_eq!(stopped.unwrap().stopped, ["mysql-8.4"]);
    assert_eq!(w.mysql.state(&default_mysql()), ServiceState::Stopped);
}

#[tokio::test]
async fn a_sync_running_when_the_app_goes_away_starts_nothing_more() {
    let w = World::with_basics();
    w.project("shop", "C:\\sites\\shop", true);
    w.hosts_in_sync();
    w.mysql.hold_starts.store(true, Ordering::SeqCst);
    let shutdown = w.shutdown();
    let quit = async {
        w.mysql.until_starting(&default_mysql()).await;
        shutdown.run().await
    };
    let (synced, quitted) = together(w.env.sync(&gui()), quit, "the shutdown waited for the sync").await;

    quitted.unwrap();
    assert_eq!(synced.unwrap_err().code, ErrorCode::AppUnavailable);
    assert!(!w.apache.calls().contains(&"start:apache".to_string()), "{:?}", w.apache.calls());
    assert_eq!(w.mysql.state(&default_mysql()), ServiceState::Stopped);
    assert_eq!(w.apache.state(&ServiceId::apache()), ServiceState::Stopped);
}

#[tokio::test]
async fn once_the_app_goes_away_no_sync_start_or_lease_brings_a_service_up() {
    let w = World::with_basics();
    w.project("shop", "C:\\sites\\shop", true);
    w.hosts_in_sync();
    w.shutdown().run().await.unwrap();

    assert_eq!(w.env.sync(&gui()).await.unwrap_err().code, ErrorCode::AppUnavailable);
    assert_eq!(w.service_control.up(services(&[&default_mysql()]), &cli()).await.unwrap_err().code, ErrorCode::AppUnavailable);
    assert_eq!(w.service_control.ensure_running(default_mysql()).await.unwrap_err().code, ErrorCode::AppUnavailable);
    assert!(w.all_calls().iter().all(|c| !c.starts_with("start:") && !c.starts_with("restart:")), "{:?}", w.all_calls());
}

#[tokio::test]
async fn a_lease_refuses_a_busy_port_like_a_start() {
    let w = World::with_basics();
    w.ports.busy(3306, "mysqld.exe (pid 7)");
    let err = w.service_control.ensure_running(default_mysql()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::PortInUse);
    assert_eq!(w.mysql.state(&default_mysql()), ServiceState::Stopped);
}
