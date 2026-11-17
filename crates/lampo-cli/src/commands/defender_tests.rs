//! `defender exclusion add|remove [--php] [--mysql]`: local, one request per call, prompts only
//! with `--allow-elevation`, one exit code per outcome.

use std::sync::atomic::Ordering;

use lampo_application::dto::{DefenderExclusionRequestDto, ScanExclusionChangeDto, ScanExclusionOutcomeDto};
use lampo_application::{AppError, CallContext, ErrorCode};
use serde_json::{Value, json};

use crate::testing::*;

fn parse(out: &str) -> Value {
    serde_json::from_str(out.trim()).unwrap_or_else(|e| panic!("stdout is not exactly one JSON document ({e}): {out:?}"))
}

fn request(change: ScanExclusionChangeDto, php: bool, mysql: bool) -> DefenderExclusionRequestDto {
    DefenderExclusionRequestDto { change, php, mysql }
}

#[tokio::test]
async fn php_add_and_remove_run_here_with_the_elevation_flag() {
    let fake = Fake::new();
    fake.app_running.store(false, Ordering::SeqCst);
    let (code, out, err) = run_cli(&fake, &["defender", "exclusion", "add", "--php", "--allow-elevation"]).await;
    assert_eq!((code, err.as_str()), (0, ""));
    assert!(SERVER_EXECUTABLES.iter().all(|exe| out.contains(exe)) && out.contains("defender exclusion remove --php"), "{out}");
    let (code, out, _) = run_cli(&fake, &["defender", "exclusion", "remove", "--php", "--allow-elevation"]).await;
    assert_eq!(code, 0);
    assert!(out.contains("scans again"), "{out}");

    let calls = fake.scan_exclusion_calls.lock().unwrap().clone();
    let elevated = CallContext::cli(true);
    assert_eq!(calls, vec![(request(ScanExclusionChangeDto::Add, true, false), elevated), (request(ScanExclusionChangeDto::Remove, true, false), elevated)]);
    assert_eq!(fake.ensure_calls.load(Ordering::SeqCst), 0, "the app is never started");
    assert!(fake.sent.lock().unwrap().is_empty(), "nothing goes through the app");
}

#[tokio::test]
async fn the_selectors_become_one_request() {
    for (flags, php, mysql) in [(&["--mysql"][..], false, true), (&["--php", "--mysql"][..], true, true), (&["--mysql", "--php"][..], true, true)] {
        let fake = Fake::new();
        let mut args = vec!["defender", "exclusion", "add", "--allow-elevation"];
        args.extend_from_slice(flags);
        let (code, _, _) = run_cli(&fake, &args).await;
        assert_eq!(code, 0, "{flags:?}");
        assert_eq!(*fake.scan_exclusion_calls.lock().unwrap(), vec![(request(ScanExclusionChangeDto::Add, php, mysql), CallContext::cli(true))], "{flags:?}");
    }
}

#[tokio::test]
async fn mysql_is_the_same_request_as_the_old_command() {
    let fake = Fake::new();
    run_cli(&fake, &["defender", "exclusion", "remove", "--mysql", "--allow-elevation"]).await;
    run_cli(&fake, &["mysql", "defender-exclusion", "remove", "--allow-elevation"]).await;
    let calls = fake.scan_exclusion_calls.lock().unwrap().clone();
    assert_eq!(calls[0], calls[1]);
}

#[test]
fn a_selector_is_required() {
    for args in [&["lampo", "defender", "exclusion", "add"][..], &["lampo", "defender", "exclusion", "remove", "--allow-elevation"][..]] {
        let err = crate::cli::build_command().try_get_matches_from(args).unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::MissingRequiredArgument, "{args:?}");
    }
    for args in [&["lampo", "defender", "exclusion", "add", "--extension", ".php"][..], &["lampo", "defender", "exclusion", "add", "--php", "--path", r"C:\x"][..]] {
        assert!(crate::cli::build_command().try_get_matches_from(args).is_err(), "{args:?}");
    }
}

#[tokio::test]
async fn without_the_flag_the_call_cannot_prompt() {
    let fake = Fake::new();
    *fake.scan_exclusion_result.lock().unwrap() = Err(AppError::new(ErrorCode::ElevationRequired, "re-run with --allow-elevation"));
    let (code, _, err) = run_cli(&fake, &["defender", "exclusion", "add", "--php"]).await;
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
        let (code, out, _) = run_cli(&fake, &["defender", "exclusion", "add", "--php", "--allow-elevation", "--json"]).await;
        assert_eq!(code, exit, "{outcome:?}");
        assert_eq!(parse(&out), json!({"change": "add", "outcome": wire, "folder": null, "processes": SERVER_EXECUTABLES}), "{outcome:?}");
    }
}

#[tokio::test]
async fn no_php_installed_is_exit_10() {
    let fake = Fake::new();
    *fake.scan_exclusion_result.lock().unwrap() = Err(AppError::new(ErrorCode::PackageMissing, "no PHP or Apache package is installed"));
    let (code, out, _) = run_cli(&fake, &["defender", "exclusion", "add", "--php", "--allow-elevation", "--json"]).await;
    assert_eq!(code, 10);
    assert_eq!(parse(&out)["error"]["code"], "package-missing");
}

#[test]
fn both_forms_are_advertised_as_mutating_and_local() {
    let catalog = lampo_application::use_cases::agent::cli_command_catalog();
    for name in ["lampo defender exclusion add", "lampo defender exclusion remove", "lampo mysql defender-exclusion add"] {
        let entry = catalog.iter().find(|c| c.name == name).unwrap_or_else(|| panic!("{name} advertised"));
        assert!(entry.mutating && !entry.requires_app && entry.usage.contains("--allow-elevation"), "{entry:?}");
    }
}
