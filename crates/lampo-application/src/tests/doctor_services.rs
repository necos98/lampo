//! The doctor's `services` check: the live states of the services the plan needs.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use lampo_domain::PortResult;
use lampo_domain::ports::{ControlChannelClient, ControlFrameSink};
use lampo_domain::service::{ServiceId, ServiceState};

use crate::control::{AppServiceStatus, ControlClient, ControlEnvelope, ControlFrame, ControlReply, ControlRequest, STATUS_READ_TIMEOUT};
use crate::dto::{CheckStatus, DoctorCheckDto, DoctorReportDto, StatusDto};
use crate::error::{AppError, ErrorCode};
use crate::testing::*;
use crate::use_cases::*;

fn apache_id() -> ServiceId {
    "apache".parse().unwrap()
}

/// `shop` is active: the plan needs Apache and MySQL 8.4.
fn world_with_shop() -> World {
    let w = World::with_basics();
    w.project("shop", "C:\\sites\\shop", true);
    w
}

async fn run(deps: DoctorDeps) -> (DoctorReportDto, DoctorCheckDto) {
    let report = Doctor::new(deps).run().await.unwrap();
    let line = report.checks.iter().find(|c| c.id == "services").cloned().unwrap();
    (report, line)
}

async fn doctor(w: &World) -> (DoctorReportDto, DoctorCheckDto) {
    run(w.doctor_deps(Arc::new(FakeLocator::default()))).await
}

#[tokio::test]
async fn a_failed_service_a_project_needs_is_an_error() {
    let w = world_with_shop();
    w.apache.set(&apache_id(), ServiceState::Failed);
    w.mysql.set(&mysql_id("8.4"), ServiceState::Running);

    let (report, line) = doctor(&w).await;

    assert_eq!(line.status, CheckStatus::Error);
    assert!(!report.ok, "`lampo doctor` exits 14 on any error");
    assert_eq!(report.services_down, ["apache"]);
    assert_eq!(line.message, "services Lampo should run are not running (failed: apache)");
    assert_eq!(line.fix.as_deref(), Some("lampo up apache"));
}

#[tokio::test]
async fn failed_and_stopped_services_are_listed_together() {
    let w = world_with_shop();
    w.mysql.set(&mysql_id("8.4"), ServiceState::Failed);

    let (report, line) = doctor(&w).await;

    assert_eq!(line.status, CheckStatus::Error);
    assert_eq!(report.services_down, ["mysql-8.4", "apache"]);
    assert_eq!(line.message, "services Lampo should run are not running (failed: mysql-8.4; stopped: apache)");
    assert_eq!(line.fix.as_deref(), Some("lampo up mysql-8.4 apache"));
}

#[tokio::test]
async fn a_stopped_service_a_project_needs_is_a_warning() {
    let w = world_with_shop();
    w.apache.set(&apache_id(), ServiceState::Running);

    let (report, line) = doctor(&w).await;

    assert_eq!(line.status, CheckStatus::Warning);
    assert_eq!(report.services_down, ["mysql-8.4"]);
    assert_eq!(line.fix.as_deref(), Some("lampo up mysql-8.4"));
}

#[tokio::test]
async fn running_needed_services_and_failed_unneeded_ones_are_ok() {
    let w = world_with_shop();
    w.apache.set(&apache_id(), ServiceState::Running);
    w.mysql.set(&mysql_id("8.4"), ServiceState::Starting);
    w.mysql.set(&mysql_id("5.7"), ServiceState::Failed);

    let (report, line) = doctor(&w).await;

    assert_eq!(line.status, CheckStatus::Ok, "{}", line.message);
    assert!(report.services_down.is_empty());
    assert_eq!(line.fix, None);
}

#[tokio::test]
async fn an_inactive_project_needs_nothing_but_the_always_on_mysql_is_needed() {
    let w = World::with_basics();
    w.project("shop", "C:\\sites\\shop", false);

    let (report, line) = doctor(&w).await;
    assert_eq!(line.status, CheckStatus::Warning);
    assert_eq!(report.services_down, ["mysql-8.4"], "not Apache: no active site");

    w.mysql.set(&mysql_id("8.4"), ServiceState::Running);
    assert_eq!(doctor(&w).await.1.status, CheckStatus::Ok);
}

#[tokio::test]
async fn unreadable_states_are_a_warning_not_an_error() {
    let w = world_with_shop();
    let deps = DoctorDeps { service_states: Arc::new(UnreadableServiceStates), ..w.doctor_deps(Arc::new(FakeLocator::default())) };

    let (report, line) = run(deps).await;

    assert_eq!(line.status, CheckStatus::Warning);
    assert_eq!(line.message, "the service states cannot be read: the Lampo app is not running");
    assert_eq!(line.fix.as_deref(), Some("lampo up"));
    assert!(report.services_down.is_empty());
}

/// An app that answers every request with `reply`, recording the request and its timeout.
struct AnsweringApp {
    reply: ControlReply,
    seen: Mutex<Vec<(ControlRequest, Duration)>>,
}

#[async_trait]
impl ControlChannelClient for AnsweringApp {
    fn is_server_running(&self) -> bool {
        true
    }
    async fn exchange(&self, request: String, _events: &dyn ControlFrameSink, timeout: Duration) -> PortResult<String> {
        let envelope: ControlEnvelope = serde_json::from_str(&request).unwrap();
        self.seen.lock().unwrap().push((envelope.request, timeout));
        Ok(serde_json::to_string(&ControlFrame::Response { reply: self.reply.clone() }).unwrap())
    }
}

fn app_service_status(channel: Arc<dyn ControlChannelClient>) -> (AppServiceStatus, Arc<FakeLauncher>) {
    let launcher = Arc::new(FakeLauncher { launches: Default::default() });
    let clock = Arc::new(ManualClock::default());
    (AppServiceStatus::new(Arc::new(ControlClient::new(channel, launcher.clone(), clock.clone(), clock))), launcher)
}

#[tokio::test]
async fn the_cli_reads_the_states_from_the_running_app() {
    let w = world_with_shop();
    w.apache.set(&apache_id(), ServiceState::Failed);
    let services = ServiceQueries::new(w.runners(), w.services.clone(), w.projects.clone(), w.worktrees.clone(), w.settings.clone()).list().unwrap();
    let status = StatusDto { app_running: true, services: services.clone(), projects: Vec::new(), worktree_count: 0, unreadable_projects: Vec::new() };
    let app = Arc::new(AnsweringApp { reply: ControlReply::Ok { payload: serde_json::to_value(status).unwrap() }, seen: Mutex::default() });
    let (source, _) = app_service_status(app.clone());

    assert_eq!(source.statuses().await.unwrap(), services);
    assert_eq!(*app.seen.lock().unwrap(), [(ControlRequest::Status, STATUS_READ_TIMEOUT)], "a bounded read");

    let deps = DoctorDeps { service_states: Arc::new(source), ..w.doctor_deps(Arc::new(FakeLocator::default())) };
    let (report, line) = run(deps).await;
    assert_eq!((line.status, report.ok), (CheckStatus::Error, false));
}

#[tokio::test]
async fn the_cli_does_not_start_the_app_to_read_the_states() {
    let channel = Arc::new(FakeControl(AtomicBool::new(false)));
    let (source, launcher) = app_service_status(channel.clone());

    assert_eq!(source.statuses().await.unwrap_err().code, ErrorCode::AppUnavailable);
    assert_eq!(launcher.launches.load(Ordering::SeqCst), 0);

    // Running, but gone before it answers.
    channel.0.store(true, Ordering::SeqCst);
    assert_eq!(source.statuses().await.unwrap_err().code, ErrorCode::AppUnavailable);
}

#[tokio::test]
async fn an_error_reply_of_the_app_is_propagated() {
    let error = AppError::internal("boom");
    let app = Arc::new(AnsweringApp { reply: ControlReply::Error { error: error.clone() }, seen: Mutex::default() });
    let (source, _) = app_service_status(app);

    assert_eq!(source.statuses().await.unwrap_err(), error);
}
