//! `project activate --all` / `project deactivate --all`: one project at a time, missing folders
//! skipped, refusals listed and reflected in the exit code.

use std::sync::atomic::Ordering;

use lampo_application::control::ControlRequest;
use lampo_application::dto::UnreadableProjectDto;
use lampo_application::{AppError, ErrorCode};
use serde_json::{Value, json};

use crate::testing::*;

fn parse(out: &str) -> Value {
    serde_json::from_str(out.trim()).unwrap_or_else(|e| panic!("stdout is not exactly one JSON document ({e}): {out:?}"))
}

fn project(id: &str, active: bool, folder_exists: bool) -> Value {
    let mut p = project_json();
    p["id"] = json!(id);
    p["active"] = json!(active);
    p["folderExists"] = json!(folder_exists);
    p
}

/// `shop` active; `blog`, `api` inactive; `gone` inactive with its folder missing.
fn fake_with_projects() -> std::sync::Arc<Fake> {
    let fake = Fake::new();
    *fake.projects.lock().unwrap() = vec![project("api", false, true), project("blog", false, true), project("gone", false, false), project("shop", true, true)];
    fake
}

/// The `(project, active)` of every request sent, in order.
fn switches(fake: &Fake) -> Vec<(String, bool)> {
    fake.sent
        .lock()
        .unwrap()
        .iter()
        .map(|(request, _)| match request {
            ControlRequest::ProjectSetActive(r) => (r.project_id.clone(), r.active),
            other => panic!("unexpected request {other:?}"),
        })
        .collect()
}

#[tokio::test]
async fn activate_all_switches_the_inactive_projects_and_skips_missing_folders() {
    let fake = fake_with_projects();
    fake.reply(Ok(project_json()));
    fake.reply(Ok(project_json()));
    let (code, out, err) = run_cli(&fake, &["project", "activate", "--all", "--json"]).await;
    assert_eq!((code, err.as_str()), (0, ""));
    assert_eq!(parse(&out), json!({"active": true, "changed": ["api", "blog"], "missingFolder": ["gone"], "failed": [], "elevationPending": false}));
    assert_eq!(switches(&fake), [("api".to_string(), true), ("blog".to_string(), true)]);
}

#[tokio::test]
async fn a_refusal_does_not_stop_the_others_and_sets_the_exit_code() {
    let fake = fake_with_projects();
    fake.reply(Err(AppError::new(ErrorCode::PortInUse, "port 8101 is in use")));
    fake.reply(Ok(project_json()));
    let (code, out, _) = run_cli(&fake, &["project", "activate", "--all", "--json"]).await;
    assert_eq!(code, 5);
    let doc = parse(&out);
    assert_eq!(doc["changed"], json!(["blog"]));
    assert_eq!(doc["failed"], json!([{"projectId": "api", "code": "port-in-use", "message": "port 8101 is in use"}]));
    assert_eq!(switches(&fake).len(), 2);

    let fake = fake_with_projects();
    fake.reply(Err(AppError::new(ErrorCode::PortInUse, "port 8101 is in use")));
    fake.reply(Ok(project_json()));
    let (code, out, _) = run_cli(&fake, &["project", "activate", "--all"]).await;
    assert_eq!(code, 5);
    assert_eq!(out, "activated 1 project(s): blog\nskipped, folder missing: gone\nnot activated: api: port 8101 is in use\n");
}

#[tokio::test]
async fn deactivate_all_counts_a_pending_hosts_file_as_changed_with_exit_7() {
    let fake = fake_with_projects();
    fake.reply(Err(AppError::new(ErrorCode::ElevationRequired, "the hosts file needs administrator permission")));
    let (code, out, err) = run_cli(&fake, &["project", "deactivate", "--all", "--json"]).await;
    assert_eq!(code, 7);
    assert_eq!(parse(&out), json!({"active": false, "changed": ["shop"], "missingFolder": [], "failed": [], "elevationPending": true}));
    assert!(err.contains("--allow-elevation"), "{err}");
    assert_eq!(switches(&fake), [("shop".to_string(), false)]);
}

#[tokio::test]
async fn nothing_to_switch_sends_nothing_and_never_starts_the_app() {
    let fake = Fake::new();
    fake.app_running.store(false, Ordering::SeqCst);
    let (code, out, _) = run_cli(&fake, &["project", "activate", "--all"]).await;
    assert_eq!((code, out.as_str()), (0, "every project is already active\n"));
    assert!(fake.sent.lock().unwrap().is_empty());
    assert_eq!(fake.ensure_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn an_unreachable_app_stops_the_command_with_its_error() {
    let fake = fake_with_projects();
    fake.app_running.store(false, Ordering::SeqCst);
    let (code, out, _) = run_cli(&fake, &["project", "activate", "--all", "--no-start", "--json"]).await;
    assert_eq!(code, 9);
    assert_eq!(parse(&out)["error"]["code"], "app-unavailable", "one JSON document: the error");
}

#[tokio::test]
async fn project_files_that_do_not_load_are_reported_on_stderr() {
    let fake = Fake::new();
    *fake.unreadable_projects.lock().unwrap() = vec![UnreadableProjectDto { file: "C:\\p\\bad.toml".into(), reason: "invalid data in C:\\p\\bad.toml".into() }];
    let (code, out, err) = run_cli(&fake, &["project", "activate", "--all", "--json"]).await;
    assert_eq!(code, 0);
    assert_eq!(parse(&out)["changed"], json!([]));
    assert!(err.contains("1 project file(s) do not load and were left as they are: invalid data in C:\\p\\bad.toml"), "{err}");
}

#[test]
fn all_and_an_id_are_exclusive() {
    let parse = |args: &[&str]| crate::cli::build_command().try_get_matches_from(std::iter::once("lampo").chain(args.iter().copied())).map(|_| ());
    assert!(parse(&["project", "deactivate", "--all"]).is_ok());
    assert!(parse(&["project", "activate", "shop", "--all"]).is_err());
    assert!(parse(&["project", "deactivate", "shop", "--all"]).is_err());
}
