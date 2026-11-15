//! Untrusted configuration: custom Apache directives and imported exports must not run code or
//! expose the disk. The import leaves out what is untrusted and reports it; a stored project
//! whose directives break the policy (written before a rule existed) is refused on save and
//! left out of Apache on sync.

use std::sync::Arc;

use lampo_domain::ports::ProjectRepository;
use lampo_domain::project::{ApacheDirectives, ProjectId, TlsMode};
use lampo_domain::project_import::DroppedOnImport;

use super::{cli, gui};
use crate::context::CallContext;
use crate::dto::*;
use crate::error::ErrorCode;
use crate::testing::*;
use crate::use_cases::*;

/// Directives as they come back from a project file written before the policy existed.
fn stored_directives(text: &str) -> ApacheDirectives {
    serde_json::from_value(serde_json::Value::String(text.into())).expect("structurally valid")
}

const PIPED_LOG: &str = "CustomLog \"|C:/Windows/System32/cmd.exe /c calc\" common";

#[tokio::test]
async fn an_import_reports_what_was_left_out_and_never_brings_https() {
    let w = World::with_basics();
    w.hosts_in_sync();
    let mut exported = w.project("shop", "C:\\sites\\shop", true);
    w.projects.delete(&exported.id).unwrap();
    exported.tls = TlsMode::LocalCa;
    let transfer = Arc::new(FakeTransfer::default());
    *transfer.to_import.lock().unwrap() = Some(exported);
    *transfer.dropped.lock().unwrap() = DroppedOnImport { apache_directives: Some(PIPED_LOG.into()), php_ini_keys: vec!["auto_prepend_file".into()] };

    let result = w.import_project(transfer).execute(ImportProjectRequest { file: "D:\\shop.lampo.toml".into(), root: None }, &gui()).await.unwrap();
    assert_eq!(result.dropped_apache_directives.as_deref(), Some(PIPED_LOG));
    assert_eq!(result.dropped_php_ini_keys, ["auto_prepend_file"]);
    assert!(!result.project.active);
    let saved = w.projects.get(&ProjectId::new("shop").unwrap()).unwrap().unwrap();
    assert_eq!(saved.tls, TlsMode::Disabled);

    let json = serde_json::to_value(&result).unwrap();
    assert_eq!(json["id"], "shop", "the project fields stay at the top level");
    assert_eq!(json["droppedApacheDirectives"], PIPED_LOG);
    assert_eq!(json["droppedPhpIniKeys"], serde_json::json!(["auto_prepend_file"]));
}

/// From the CLI without `--allow-elevation`, an export with a hosts-file name is refused before
/// anything is saved (otherwise the project would be imported but the result, with what was
/// left out, lost in the `ElevationRequired` error of the sync).
#[tokio::test]
async fn a_cli_import_needing_the_hosts_file_saves_nothing_without_elevation() {
    let w = World::with_basics();
    w.hosts_in_sync();
    let exported = w.project("shop", "C:\\sites\\shop", false);
    w.projects.delete(&exported.id).unwrap();
    let transfer = Arc::new(FakeTransfer::default());
    *transfer.to_import.lock().unwrap() = Some(exported);
    let import = w.import_project(transfer);
    let request = || ImportProjectRequest { file: "D:\\shop.lampo.toml".into(), root: None };

    let err = import.execute(request(), &cli()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::ElevationRequired);
    assert!(err.message.contains("`shop.test`") && err.message.contains("nothing was imported"), "{}", err.message);
    assert!(w.projects.list().unwrap().is_empty());
    assert!(import.execute(request(), &CallContext::cli(true)).await.is_ok());
}

#[tokio::test]
async fn an_import_without_a_file_is_a_validation_error() {
    let w = World::with_basics();
    let err = w.import_project(Arc::new(FakeTransfer::default())).execute(ImportProjectRequest { file: "  ".into(), root: None }, &gui()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Validation);
}

#[tokio::test]
async fn a_stored_project_with_refused_directives_cannot_be_saved_until_fixed() {
    let w = World::with_basics();
    w.project("blog", "C:\\sites\\blog", false);
    w.update_project("blog", |p| p.environment.apache_directives = stored_directives(&format!("Header set X 1\n{PIPED_LOG}")));
    w.hosts_in_sync();
    assert_eq!(w.project_queries().list().unwrap().len(), 1, "it still loads");

    let activate = SetProjectActive::new(w.project_writer());
    let err = activate.execute(SetProjectActiveRequest { project_id: "blog".into(), active: true }, &gui()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Validation);
    assert!(err.message.contains("line 2: `CustomLog` is not allowed: a piped log runs a program"), "{}", err.message);
    assert!(!w.projects.get(&ProjectId::new("blog").unwrap()).unwrap().unwrap().active, "nothing was saved");

    // Removing the directive fixes it.
    let fixed = w.patch_project().execute(ProjectPatchRequest { project_id: "blog".into(), apache_directives: Some("Header set X 1".into()), ..Default::default() }, &gui()).await;
    assert!(fixed.is_ok(), "{fixed:?}");
    assert!(activate.execute(SetProjectActiveRequest { project_id: "blog".into(), active: true }, &gui()).await.is_ok());
}

#[tokio::test]
async fn a_stored_project_with_refused_directives_is_left_out_of_apache() {
    let w = World::with_basics();
    w.project("shop", "C:\\sites\\shop", true);
    w.project("blog", "C:\\sites\\blog", true);
    w.update_project("blog", |p| p.environment.apache_directives = stored_directives("Include C:/evil.conf"));
    w.hosts_in_sync();

    let err = w.env.sync(&gui()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::ConfigRejected);
    let details = err.details.clone().unwrap();
    assert_eq!(details["excludedSites"], serde_json::json!(["blog"]));
    assert!(details["output"].as_str().unwrap().contains("`Include` is not allowed"), "{details}");
    let applied = w.web.last().unwrap();
    assert_eq!(applied.vhosts.iter().map(|v| v.site.to_string()).collect::<Vec<_>>(), ["shop"]);
    assert!(details["report"]["started"].as_array().unwrap().contains(&serde_json::json!("apache")), "the other sites are served");

    // Still reported on the next sync, nothing else to apply.
    assert_eq!(w.env.sync(&gui()).await.unwrap_err().code, ErrorCode::ConfigRejected);
}
