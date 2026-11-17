//! `logs --level` (the viewer's severity filter) and `logs --open` / `--open-folder`.

use std::sync::atomic::Ordering;

use lampo_application::use_cases::LogTarget;
use lampo_application::{AppError, ErrorCode};
use serde_json::{Value, json};

use crate::testing::*;

fn parse(out: &str) -> Value {
    serde_json::from_str(out.trim()).unwrap_or_else(|e| panic!("stdout is not exactly one JSON document ({e}): {out:?}"))
}

/// A tail with one ordinary line, one warning and one error.
fn mixed_tail(fake: &Fake) {
    *fake.tail.lock().unwrap() = Ok(dto(json!({"source": "php:shop", "file": "C:\\l\\php.log", "lines": [
        {"text": "GET / 200", "atUnixMs": 1},
        {"text": "PHP Warning: Undefined variable $user", "atUnixMs": 2},
        {"text": "PHP Fatal error: Uncaught Error", "atUnixMs": 3}
    ], "nextOffset": 90})));
}

#[tokio::test]
async fn the_level_keeps_the_lines_at_or_above_it() {
    let cases: [(&str, &[&str]); 3] = [
        ("all", &["GET / 200", "PHP Warning: Undefined variable $user", "PHP Fatal error: Uncaught Error"]),
        ("warnings", &["PHP Warning: Undefined variable $user", "PHP Fatal error: Uncaught Error"]),
        ("errors", &["PHP Fatal error: Uncaught Error"]),
    ];
    for (level, expected) in cases {
        let fake = Fake::new();
        mixed_tail(&fake);
        let (code, out, _) = run_cli(&fake, &["logs", "php", "--level", level, "--json"]).await;
        let doc = parse(&out);
        let texts: Vec<&str> = doc["lines"].as_array().unwrap().iter().map(|l| l["text"].as_str().unwrap()).collect();
        assert_eq!((code, texts.as_slice()), (0, expected), "{level}");
        assert_eq!(doc["nextOffset"], 90, "following continues after the filtered lines too");
    }
}

#[tokio::test]
async fn without_a_level_every_line_is_printed() {
    let fake = Fake::new();
    mixed_tail(&fake);
    let (code, out, _) = run_cli(&fake, &["logs", "php"]).await;
    assert_eq!((code, out.lines().count()), (0, 3));
}

#[tokio::test]
async fn follow_filters_the_new_lines_too() {
    let fake = Fake::new();
    mixed_tail(&fake);
    fake.follow_reads.lock().unwrap().push_back(dto(json!({"source": "php:shop", "file": "f", "lines": [
        {"text": "[core:notice] reload", "atUnixMs": 4},
        {"text": "[core:alert] disk full", "atUnixMs": 5}
    ], "nextOffset": 120})));
    let (code, out, _) = run_cli(&fake, &["logs", "php", "--follow", "--level", "errors"]).await;
    assert_eq!((code, out.as_str()), (0, "PHP Fatal error: Uncaught Error\n[core:alert] disk full\n"));
    assert_eq!(fake.follow_offsets.lock().unwrap().as_slice(), [90]);
}

#[tokio::test]
async fn open_and_open_folder_start_the_users_app_on_the_expanded_source() {
    for (flag, target, name) in [("--open", LogTarget::File, "file"), ("--open-folder", LogTarget::Folder, "folder")] {
        let fake = Fake::new();
        fake.app_running.store(false, Ordering::SeqCst);
        let (code, out, err) = run_cli(&fake, &["logs", "php", flag, "--json"]).await;
        assert_eq!((code, err.as_str()), (0, ""), "{flag}");
        assert_eq!(parse(&out), json!({"source": "php:shop", "target": name}), "{flag}");
        assert_eq!(fake.opened_logs.lock().unwrap().as_slice(), [("php:shop".to_string(), target)], "{flag}");
        assert!(fake.tail_requests.lock().unwrap().is_empty(), "{flag}: nothing is read");
        assert_eq!(fake.ensure_calls.load(Ordering::SeqCst), 0, "{flag}: the app is not started");
    }
    let fake = Fake::new();
    let (code, out, _) = run_cli(&fake, &["logs", "apache-error", "--open-folder"]).await;
    assert_eq!((code, out.as_str()), (0, "opened the log folder of `apache-error`\n"));
}

#[tokio::test]
async fn a_log_not_written_yet_is_reported_with_its_exit_code() {
    let fake = Fake::new();
    *fake.open_log_result.lock().unwrap() = Err(AppError::new(ErrorCode::Validation, "the log file does not exist yet: C:\\l\\php.log"));
    let (code, out, _) = run_cli(&fake, &["logs", "php", "--open", "--json"]).await;
    assert_eq!(code, 4);
    assert!(parse(&out)["error"]["message"].as_str().unwrap().contains("does not exist yet"));
}

#[test]
fn open_needs_a_source_and_refuses_the_printing_flags() {
    let parse = |args: &[&str]| crate::cli::build_command().try_get_matches_from(std::iter::once("lampo").chain(args.iter().copied())).map(|_| ());
    assert!(parse(&["logs", "php", "--open"]).is_ok());
    for bad in [
        &["logs", "--open"][..],
        &["logs", "php", "--open", "--open-folder"],
        &["logs", "php", "--open", "--follow"],
        &["logs", "php", "--open-folder", "--level", "errors"],
        &["logs", "php", "--open", "-n", "5"],
        &["logs", "php", "--open", "--since", "10m"],
        &["logs", "php", "--level", "debug"],
    ] {
        assert!(parse(bad).is_err(), "{bad:?}");
    }
}
