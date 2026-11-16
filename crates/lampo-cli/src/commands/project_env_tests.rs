//! `--env` / `--no-env` of `project create` and `project set`, and how `project info` shows the
//! variables.

use lampo_application::control::ControlRequest;
use serde_json::{Value, json};

use crate::testing::*;

fn sent_patch(fake: &Fake) -> lampo_application::dto::ProjectPatchRequest {
    match fake.last_request().0 {
        ControlRequest::ProjectPatch(r) => r,
        other => panic!("wrong request: {other:?}"),
    }
}

fn with_vars(vars: Value) -> Value {
    let mut project = project_json();
    project["environment"]["envVars"] = vars;
    project
}

#[tokio::test]
async fn project_set_forwards_variables_verbatim_and_removals() {
    let fake = Fake::new();
    fake.reply(Ok(with_vars(json!({"APP_ENV": "local"}))));
    let args = ["project", "set", "--env", "APP_ENV=local", "--env", "MAIL_FROM=Shop <a@b.test>, x=y", "--env", "EMPTY=", "--no-env", "DEBUG,OLD", "--json"];
    let (code, out, _) = run_cli(&fake, &args).await;
    assert_eq!(code, 0, "{out}");
    let r = sent_patch(&fake);
    let set: Vec<(String, String)> = r.env_set.into_iter().collect();
    assert_eq!(set, [("APP_ENV".into(), "local".into()), ("EMPTY".into(), String::new()), ("MAIL_FROM".into(), "Shop <a@b.test>, x=y".into())]);
    assert_eq!(r.env_unset, ["DEBUG", "OLD"]);
    assert!(r.ini_set.is_empty() && r.apache_directives.is_none(), "only what is named");
    let doc: Value = serde_json::from_str(out.trim()).unwrap();
    assert_eq!(doc["environment"]["envVars"]["APP_ENV"], "local", "project info --json shows the values");
}

#[tokio::test]
async fn a_malformed_env_flag_is_refused_without_echoing_it() {
    let fake = Fake::new();
    for bad in ["s3cr3t", "=s3cr3t"] {
        let (code, out, _) = run_cli(&fake, &["project", "set", "--env", bad, "--json"]).await;
        assert_eq!(code, 4);
        let message = serde_json::from_str::<Value>(out.trim()).unwrap()["error"]["message"].as_str().unwrap().to_string();
        assert!(message.contains("KEY=VALUE") && !message.contains("s3cr3t"), "{message}");
    }
    let (code, _, _) = run_cli(&fake, &["project", "set", "--env", "A=1", "--env", "A=2"]).await;
    assert_eq!(code, 4, "a name given twice is refused, not silently overwritten");
    assert!(fake.sent.lock().unwrap().is_empty(), "nothing sent");
}

#[tokio::test]
async fn project_create_forwards_its_variables() {
    let fake = Fake::new();
    fake.reply(Ok(with_vars(json!({"APP_ENV": "local"}))));
    let (code, _, err) = run_cli(&fake, &["project", "create", "--env", "APP_ENV=local", "--env", "APP_URL=http://shop.localhost"]).await;
    assert_eq!(code, 0, "{err}");
    let ControlRequest::ProjectCreate(r) = fake.last_request().0 else { panic!("wrong request") };
    assert_eq!(r.env_vars.into_iter().collect::<Vec<_>>(), [("APP_ENV".to_string(), "local".to_string()), ("APP_URL".into(), "http://shop.localhost".into())]);
}

#[tokio::test]
async fn project_info_lists_the_names_but_never_the_values() {
    let fake = Fake::new();
    fake.reply(Ok(with_vars(json!({"APP_ENV": "local", "DB_PASSWORD": "s3cr3t"}))));
    let (code, out, _) = run_cli(&fake, &["project", "set", "--env", "DB_PASSWORD=s3cr3t"]).await;
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("env vars:  APP_ENV, DB_PASSWORD\n"), "{out}");
    assert!(!out.contains("s3cr3t"), "{out}");
}
