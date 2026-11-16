//! `open`: the resolved site in the user's desktop apps, without the app and without prompts.

use std::sync::atomic::Ordering;

use clap::Parser;
use lampo_application::dto::ExternalApp;
use lampo_application::{AppError, ErrorCode};
use serde_json::{Value, json};

use crate::cli::Cli;
use crate::testing::*;

fn parse(out: &str) -> Value {
    serde_json::from_str(out.trim()).unwrap_or_else(|e| panic!("stdout is not exactly one JSON document ({e}): {out:?}"))
}

#[tokio::test]
async fn each_flag_opens_its_app_and_explorer_is_the_default() {
    let cases: [(&[&str], ExternalApp); 5] = [
        (&[], ExternalApp::Folder),
        (&["--explorer"], ExternalApp::Folder),
        (&["--code"], ExternalApp::VsCode),
        (&["--terminal"], ExternalApp::Terminal),
        (&["--browser"], ExternalApp::Browser),
    ];
    for (flags, app) in cases {
        let fake = Fake::new();
        let mut args = vec!["open"];
        args.extend_from_slice(flags);
        let (code, _, err) = run_cli(&fake, &args).await;
        assert_eq!((code, err.as_str()), (0, ""), "{flags:?}");
        assert_eq!(fake.opened.lock().unwrap().as_slice(), [("shop".to_string(), app)], "{flags:?}");
    }
}

#[tokio::test]
async fn open_json_is_the_dto_and_never_starts_the_app() {
    let fake = Fake::new();
    fake.app_running.store(false, Ordering::SeqCst);
    let (code, out, err) = run_cli(&fake, &["open", "--browser", "--json"]).await;
    assert_eq!((code, err.as_str()), (0, ""));
    assert_eq!(parse(&out), json!({"site": "shop", "app": "browser", "target": "http://127.0.0.1:8101", "served": true}));
    assert_eq!(fake.ensure_calls.load(Ordering::SeqCst), 0);
    assert!(fake.sent.lock().unwrap().is_empty(), "nothing goes through the app");
}

#[tokio::test]
async fn open_prints_what_was_opened_on_the_resolved_worktree_site() {
    let fake = Fake::new();
    fake.resolves_to(Some("shop@fix"));
    let (code, out, _) = run_cli(&fake, &["open", "--code"]).await;
    assert_eq!(code, 0, "{out}");
    assert_eq!(out.trim_end(), r"shop@fix: opened C:\work\shop in Visual Studio Code");
    assert_eq!(fake.opened.lock().unwrap()[0].0, "shop@fix", "a worktree environment opens as itself");
}

#[tokio::test]
async fn outside_every_project_is_exit_6_and_opens_nothing() {
    let fake = Fake::new();
    fake.resolves_to(None);
    let (code, out, _) = run_cli(&fake, &["open", "--json"]).await;
    assert_eq!(code, 6);
    let error = &parse(&out)["error"];
    assert_eq!((error["code"].as_str(), error["exitCode"].as_i64()), (Some("project-not-resolved"), Some(6)));
    assert!(error["message"].as_str().unwrap().contains("not inside a Lampo project"), "{error}");
    assert!(fake.opened.lock().unwrap().is_empty());
}

#[tokio::test]
async fn launch_failures_keep_their_exit_codes() {
    let cases = [(ErrorCode::PackageMissing, 10), (ErrorCode::Validation, 4), (ErrorCode::ProcessFailed, 12), (ErrorCode::NotFound, 3)];
    for (error, exit) in cases {
        let fake = Fake::new();
        *fake.open_result.lock().unwrap() = Some(Err(AppError::new(error, "cannot open")));
        let (code, out, _) = run_cli(&fake, &["open", "--code", "--json"]).await;
        assert_eq!(code, exit, "{error:?}");
        assert_eq!(parse(&out)["error"]["message"], "cannot open");
    }
}

#[tokio::test]
async fn the_browser_on_an_inactive_project_warns_on_stderr() {
    let fake = Fake::new();
    let mut unserved = opened_json("shop", ExternalApp::Browser);
    unserved["served"] = json!(false);
    *fake.open_result.lock().unwrap() = Some(Ok(unserved.clone()));
    let (code, out, err) = run_cli(&fake, &["open", "--browser", "--json"]).await;
    assert_eq!(code, 0);
    assert_eq!(parse(&out), unserved, "stdout stays one JSON document");
    assert!(err.contains("`shop` is not active") && err.contains("lampo project activate shop"), "{err}");

    let mut unserved_code = opened_json("shop", ExternalApp::VsCode);
    unserved_code["served"] = json!(false);
    *fake.open_result.lock().unwrap() = Some(Ok(unserved_code));
    let (code, _, err) = run_cli(&fake, &["open", "--code"]).await;
    assert_eq!((code, err.as_str()), (0, ""), "only the browser needs the site served");
}

#[test]
fn open_takes_at_most_one_app_and_no_positional() {
    for ok in [&["lampo", "open"][..], &["lampo", "open", "--terminal"], &["lampo", "open", "--browser", "-p", "shop", "--json"]] {
        assert!(Cli::try_parse_from(ok).is_ok(), "{ok:?}");
    }
    for bad in [&["lampo", "open", "--code", "--browser"][..], &["lampo", "open", "shop"], &["lampo", "open", "--vim"]] {
        assert!(Cli::try_parse_from(bad).is_err(), "{bad:?}");
    }
}
