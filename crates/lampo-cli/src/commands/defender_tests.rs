//! `defender exclusion add|remove [--php] [--mysql] [--projects [<folder>]]`: local, one request
//! per call, prompts only with `--allow-elevation`, one exit code per outcome.

use std::sync::atomic::Ordering;

use lampo_application::dto::{DefenderExclusionRequestDto, ScanExclusionChangeDto, ScanExclusionOutcomeDto};
use lampo_application::{AppError, CallContext, ErrorCode};
use serde_json::{Value, json};

use crate::testing::*;

fn parse(out: &str) -> Value {
    serde_json::from_str(out.trim()).unwrap_or_else(|e| panic!("stdout is not exactly one JSON document ({e}): {out:?}"))
}

fn request(change: ScanExclusionChangeDto, php: bool, mysql: bool) -> DefenderExclusionRequestDto {
    DefenderExclusionRequestDto::new(change, php, mysql)
}

fn projects(change: ScanExclusionChangeDto, folder: Option<&str>) -> DefenderExclusionRequestDto {
    DefenderExclusionRequestDto { projects: true, projects_folder: folder.map(String::from), ..request(change, false, false) }
}

#[tokio::test]
async fn projects_without_a_folder_asks_for_the_suggested_one() {
    let fake = Fake::new();
    let (code, out, err) = run_cli(&fake, &["defender", "exclusion", "add", "--projects", "--allow-elevation"]).await;
    assert_eq!((code, err.as_str()), (0, ""));
    assert!(out.contains(SUGGESTED_PROJECTS_FOLDER) && out.contains("remove --projects"), "{out}");
    let (code, out, _) = run_cli(&fake, &["defender", "exclusion", "remove", "--php", "--projects", "--allow-elevation", "--json"]).await;
    assert_eq!(code, 0);
    assert_eq!(parse(&out)["projects"], SUGGESTED_PROJECTS_FOLDER);
    let calls = fake.scan_exclusion_calls.lock().unwrap().clone();
    let both = DefenderExclusionRequestDto { php: true, ..projects(ScanExclusionChangeDto::Remove, None) };
    assert_eq!(calls, vec![(projects(ScanExclusionChangeDto::Add, None), CallContext::cli(true)), (both, CallContext::cli(true))]);
}

/// The fake terminal is in `C:\work\shop`.
#[tokio::test]
async fn a_given_projects_folder_is_made_absolute_against_the_terminal() {
    let fake = Fake::new();
    run_cli(&fake, &["defender", "exclusion", "add", "--projects", r"C:\Users\me\Code", "--allow-elevation"]).await;
    run_cli(&fake, &["defender", "exclusion", "remove", "--projects", r"..\Progetti", "--allow-elevation"]).await;
    let calls: Vec<DefenderExclusionRequestDto> = fake.scan_exclusion_calls.lock().unwrap().iter().map(|(request, _)| request.clone()).collect();
    assert_eq!(calls, vec![projects(ScanExclusionChangeDto::Add, Some(r"C:\Users\me\Code")), projects(ScanExclusionChangeDto::Remove, Some(r"C:\work\Progetti"))]);
}

#[tokio::test]
async fn a_refused_projects_folder_is_invalid_input() {
    let fake = Fake::new();
    *fake.scan_exclusion_result.lock().unwrap() = Err(AppError::new(ErrorCode::Validation, "AppData and the folders in it are never excluded"));
    let (code, _, err) = run_cli(&fake, &["defender", "exclusion", "add", "--projects", r"C:\Users\me\AppData", "--allow-elevation"]).await;
    assert_eq!(code, 4);
    assert!(err.contains("AppData"), "{err}");
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
        assert_eq!(parse(&out), json!({"change": "add", "outcome": wire, "folder": null, "processes": SERVER_EXECUTABLES, "projects": null}), "{outcome:?}");
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

fn command_requests(fake: &Fake) -> Vec<DefenderExclusionRequestDto> {
    fake.scan_exclusion_command_calls.lock().unwrap().clone()
}

#[tokio::test]
async fn commands_print_one_powershell_line_per_exclusion_without_prompting() {
    let fake = Fake::new();
    let (code, out, err) = run_cli(&fake, &["defender", "exclusion", "commands"]).await;
    assert_eq!((code, err.as_str()), (0, ""));
    let expected: Vec<String> = SERVER_EXECUTABLES.iter().map(|exe| format!("Add-MpPreference -ExclusionProcess '{exe}'")).collect();
    assert_eq!(out.lines().collect::<Vec<_>>(), expected, "only the lines, ready to be sent");
    assert_eq!(command_requests(&fake), vec![request(ScanExclusionChangeDto::Add, true, false)], "--php without a selector");
    assert!(fake.scan_exclusion_calls.lock().unwrap().is_empty(), "nothing is changed");
    assert_eq!(fake.ensure_calls.load(Ordering::SeqCst), 0, "the app is never started");
}

#[tokio::test]
async fn commands_take_the_selectors_of_add() {
    let fake = Fake::new();
    let (code, out, _) = run_cli(&fake, &["defender", "exclusion", "commands", "--mysql", "--projects", r"..\Progetti"]).await;
    assert_eq!(code, 0);
    assert_eq!(out.lines().collect::<Vec<_>>(), [format!("Add-MpPreference -ExclusionPath '{MYSQL_DATA_FOLDER}'"), r"Add-MpPreference -ExclusionPath 'C:\work\Progetti'".to_string()]);
    run_cli(&fake, &["defender", "exclusion", "commands", "--php", "--projects"]).await;
    let given = DefenderExclusionRequestDto { mysql: true, ..projects(ScanExclusionChangeDto::Add, Some(r"C:\work\Progetti")) };
    let suggested = DefenderExclusionRequestDto { php: true, ..projects(ScanExclusionChangeDto::Add, None) };
    assert_eq!(command_requests(&fake), vec![given, suggested], "the requests `add` would make");
}

#[tokio::test]
async fn commands_json_is_the_structured_list() {
    let fake = Fake::new();
    let (code, out, _) = run_cli(&fake, &["defender", "exclusion", "commands", "--mysql", "--json"]).await;
    assert_eq!(code, 0);
    let command = format!("Add-MpPreference -ExclusionPath '{MYSQL_DATA_FOLDER}'");
    assert_eq!(parse(&out), json!([{ "kind": "path", "path": MYSQL_DATA_FOLDER, "command": command }]));
}

#[tokio::test]
async fn commands_fail_as_add_would() {
    let fake = Fake::new();
    *fake.scan_exclusion_result.lock().unwrap() = Err(AppError::new(ErrorCode::PackageMissing, "no PHP or Apache package is installed"));
    let (code, out, _) = run_cli(&fake, &["defender", "exclusion", "commands", "--json"]).await;
    assert_eq!(code, 10);
    assert_eq!(parse(&out)["error"]["code"], "package-missing");
    *fake.scan_exclusion_result.lock().unwrap() = Err(AppError::new(ErrorCode::Validation, "AppData and the folders in it are never excluded"));
    let (code, _, err) = run_cli(&fake, &["defender", "exclusion", "commands", "--projects", r"C:\Users\me\AppData"]).await;
    assert_eq!(code, 4);
    assert!(err.contains("AppData"), "{err}");
}

#[test]
fn commands_accept_no_other_flag() {
    for args in [&["lampo", "defender", "exclusion", "commands", "--path", r"C:\x"][..], &["lampo", "defender", "exclusion", "commands", "--remove"][..]] {
        assert!(crate::cli::build_command().try_get_matches_from(args).is_err(), "{args:?}");
    }
}
