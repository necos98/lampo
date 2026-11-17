//! `package install|uninstall` of a PHP: the Defender exclusion changes Lampo proposes, printed
//! with their command and in `--json` as `defenderExclusion`; never made from here.

use std::sync::atomic::Ordering;

use lampo_application::dto::{DefenderExclusionRequestDto, DefenderExclusionSuggestionDto, ScanExclusionChangeDto};
use serde_json::{Value, json};

use crate::testing::*;

const NEW_PHP: &str = r"C:\lampo\packages\php-8.5\php-cgi.exe";
const OLD_PHP: &str = r"C:\lampo\packages\php-8.2\php-cgi.exe";
const ADD_COMMAND: &str = "lampo defender exclusion add --php --allow-elevation";
const REMOVE_COMMAND: &str = "lampo defender exclusion remove --uninstalled --allow-elevation";

fn parse(out: &str) -> Value {
    serde_json::from_str(out.trim()).unwrap_or_else(|e| panic!("stdout is not exactly one JSON document ({e}): {out:?}"))
}

fn add_suggestion() -> DefenderExclusionSuggestionDto {
    DefenderExclusionSuggestionDto {
        change: ScanExclusionChangeDto::Add,
        processes: vec![NEW_PHP.into()],
        request: DefenderExclusionRequestDto::new(ScanExclusionChangeDto::Add, true, false),
        command: ADD_COMMAND.into(),
    }
}

fn remove_suggestion() -> DefenderExclusionSuggestionDto {
    DefenderExclusionSuggestionDto {
        change: ScanExclusionChangeDto::Remove,
        processes: vec![OLD_PHP.into()],
        request: DefenderExclusionRequestDto::remove_uninstalled(),
        command: REMOVE_COMMAND.into(),
    }
}

fn installed_php(fake: &Fake) {
    fake.reply(Ok(json!({"id": {"kind": "php", "version": "8.5.0", "phpVersion": null}, "path": r"C:\lampo\packages\php-8.5", "usedBy": []})));
}

fn uninstalled_php(fake: &Fake) {
    fake.reply(Ok(json!({"kind": "php", "version": "8.2.27", "phpVersion": null})));
}

#[tokio::test]
async fn installing_a_php_prints_the_proposed_update_with_its_command() {
    let fake = Fake::new();
    *fake.defender_suggestions.lock().unwrap() = vec![add_suggestion()];
    installed_php(&fake);
    let (code, out, _) = run_cli(&fake, &["package", "install", "php", "8.5"]).await;
    assert_eq!(code, 0);
    let lines: Vec<&str> = out.trim().lines().collect();
    assert!(lines[0].starts_with("Installed php 8.5.0"), "{out}");
    assert!(lines[1].contains(NEW_PHP) && lines[1].contains(ADD_COMMAND), "{out}");
    assert!(fake.scan_exclusion_calls.lock().unwrap().is_empty(), "only proposed, never requested");
}

#[tokio::test]
async fn the_json_output_has_a_structured_field() {
    let fake = Fake::new();
    *fake.defender_suggestions.lock().unwrap() = vec![add_suggestion()];
    installed_php(&fake);
    let (code, out, _) = run_cli(&fake, &["package", "install", "php", "8.5", "--json"]).await;
    assert_eq!(code, 0);
    let doc = parse(&out);
    assert_eq!(doc["id"]["version"], "8.5.0", "the package fields stay where they were");
    assert_eq!(
        doc["defenderExclusion"],
        json!([{"change": "add", "processes": [NEW_PHP], "request": {"change": "add", "php": true, "mysql": false, "projects": false, "projectsFolder": null, "uninstalled": false}, "command": ADD_COMMAND}])
    );
}

#[tokio::test]
async fn uninstalling_a_php_prints_the_proposed_removal() {
    let fake = Fake::new();
    *fake.defender_suggestions.lock().unwrap() = vec![remove_suggestion()];
    uninstalled_php(&fake);
    let (code, out, _) = run_cli(&fake, &["package", "uninstall", "php", "8.2", "--yes"]).await;
    assert_eq!(code, 0);
    let lines: Vec<&str> = out.trim().lines().collect();
    assert_eq!(lines[0], "Uninstalled php 8.2.27");
    assert!(lines[1].contains(OLD_PHP) && lines[1].contains(REMOVE_COMMAND), "{out}");

    uninstalled_php(&fake);
    let (_, out, _) = run_cli(&fake, &["package", "uninstall", "php", "8.2", "--yes", "--json"]).await;
    assert_eq!(parse(&out)["defenderExclusion"][0]["request"]["uninstalled"], true);
}

#[tokio::test]
async fn without_a_proposal_the_output_is_unchanged() {
    let fake = Fake::new();
    uninstalled_php(&fake);
    let (_, out, _) = run_cli(&fake, &["package", "uninstall", "php", "8.2", "--yes", "--json"]).await;
    assert_eq!(parse(&out), json!({"kind": "php", "version": "8.2.27", "phpVersion": null}));
    assert_eq!(fake.defender_advice_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn other_packages_never_read_defender() {
    let fake = Fake::new();
    *fake.defender_suggestions.lock().unwrap() = vec![add_suggestion()];
    fake.reply(Ok(json!({"id": {"kind": "mysql", "version": "8.4.3", "phpVersion": null}, "path": r"C:\p", "usedBy": []})));
    let (_, out, _) = run_cli(&fake, &["package", "install", "mysql", "8.4", "--json"]).await;
    assert!(parse(&out).get("defenderExclusion").is_none(), "{out}");
    fake.reply(Ok(json!({"kind": "xdebug", "version": "3.3.2", "phpVersion": "8.3"})));
    run_cli(&fake, &["package", "uninstall", "xdebug", "--php", "8.3", "--yes"]).await;
    assert_eq!(fake.defender_advice_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_failed_install_proposes_nothing() {
    let fake = Fake::new();
    fake.reply(Err(lampo_application::AppError::new(lampo_application::ErrorCode::Network, "download failed")));
    let (code, _, _) = run_cli(&fake, &["package", "install", "php", "8.5"]).await;
    assert_ne!(code, 0);
    assert_eq!(fake.defender_advice_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn remove_uninstalled_sends_the_uninstalled_request() {
    let fake = Fake::new();
    let (code, _, _) = run_cli(&fake, &["defender", "exclusion", "remove", "--uninstalled", "--allow-elevation"]).await;
    assert_eq!(code, 0);
    let calls = fake.scan_exclusion_calls.lock().unwrap().clone();
    assert_eq!(calls, vec![(DefenderExclusionRequestDto::remove_uninstalled(), lampo_application::CallContext::cli(true))]);
}
