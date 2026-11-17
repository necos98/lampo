//! `mysql defender-exclusion`: the MySQL-only request, local, prompts only with `--allow-elevation`,
//! one exit code per outcome, its JSON unchanged.

use std::sync::atomic::Ordering;

use lampo_application::dto::{DefenderExclusionRequestDto, ScanExclusionChangeDto, ScanExclusionOutcomeDto};
use lampo_application::{AppError, CallContext, ErrorCode};
use serde_json::{Value, json};

use crate::testing::*;

fn parse(out: &str) -> Value {
    serde_json::from_str(out.trim()).unwrap_or_else(|e| panic!("stdout is not exactly one JSON document ({e}): {out:?}"))
}

#[tokio::test]
async fn add_and_remove_run_here_with_the_elevation_flag() {
    let fake = Fake::new();
    fake.app_running.store(false, Ordering::SeqCst);
    let (code, out, err) = run_cli(&fake, &["mysql", "defender-exclusion", "add", "--allow-elevation"]).await;
    assert_eq!((code, err.as_str()), (0, ""));
    assert!(out.contains(MYSQL_DATA_FOLDER) && out.contains("defender-exclusion remove"), "{out}");
    let (code, out, _) = run_cli(&fake, &["mysql", "defender-exclusion", "remove", "--allow-elevation"]).await;
    assert_eq!(code, 0);
    assert!(out.contains("scans"), "{out}");

    let calls = fake.scan_exclusion_calls.lock().unwrap().clone();
    let mysql_only = |change| (DefenderExclusionRequestDto { change, php: false, mysql: true }, CallContext::cli(true));
    assert_eq!(calls, vec![mysql_only(ScanExclusionChangeDto::Add), mysql_only(ScanExclusionChangeDto::Remove)]);
    assert_eq!(fake.ensure_calls.load(Ordering::SeqCst), 0, "the app is never started");
    assert!(fake.sent.lock().unwrap().is_empty(), "nothing goes through the app");
}

#[tokio::test]
async fn without_the_flag_the_call_cannot_prompt() {
    let fake = Fake::new();
    *fake.scan_exclusion_result.lock().unwrap() = Err(AppError::new(ErrorCode::ElevationRequired, "re-run with --allow-elevation"));
    let (code, _, err) = run_cli(&fake, &["mysql", "defender-exclusion", "add"]).await;
    assert_eq!(code, 7);
    assert!(err.contains("--allow-elevation"), "{err}");
    assert_eq!(fake.scan_exclusion_calls.lock().unwrap()[0].1, CallContext::cli(false), "the use case is told the CLI may not prompt");
}

#[tokio::test]
async fn each_outcome_has_its_exit_code_and_json_is_the_dto() {
    for (outcome, exit, wire) in [
        (ScanExclusionOutcomeDto::Applied, 0, "applied"),
        (ScanExclusionOutcomeDto::Cancelled, 8, "cancelled"),
        (ScanExclusionOutcomeDto::DefenderUnavailable, 12, "defender-unavailable"),
        (ScanExclusionOutcomeDto::Refused, 12, "refused"),
        (ScanExclusionOutcomeDto::NotConfirmed, 12, "not-confirmed"),
    ] {
        let fake = Fake::new();
        *fake.scan_exclusion_result.lock().unwrap() = Ok(outcome);
        let (code, out, _) = run_cli(&fake, &["mysql", "defender-exclusion", "add", "--allow-elevation", "--json"]).await;
        assert_eq!(code, exit, "{outcome:?}");
        assert_eq!(parse(&out), json!({"change": "add", "outcome": wire, "folder": MYSQL_DATA_FOLDER}), "{outcome:?}");
    }
}

#[tokio::test]
async fn adapter_errors_are_reported_as_errors() {
    let fake = Fake::new();
    *fake.scan_exclusion_result.lock().unwrap() = Err(AppError::new(ErrorCode::Validation, "only the MySQL data folder of a Windows user profile can be excluded"));
    let (code, out, _) = run_cli(&fake, &["mysql", "defender-exclusion", "remove", "--allow-elevation", "--json"]).await;
    assert_eq!(code, 4);
    assert_eq!(parse(&out)["error"]["code"], "validation");
}
