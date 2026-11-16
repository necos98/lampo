//! Per-project environment variables: set by the patch and at creation, refused when Lampo
//! manages the name, carried into the vhost (`SetEnv`), the shim index and `lampo env`, and
//! followed by worktree environments.

use std::collections::BTreeMap;
use std::sync::Arc;

use lampo_domain::ports::ProjectRepository;
use lampo_domain::progress::CancelFlag;
use lampo_domain::project::ProjectEnvVars;

use super::{cli, gui};
use crate::dto::*;
use crate::error::ErrorCode;
use crate::testing::{RecordingProgress, World};
use crate::use_cases::*;

fn patch(id: &str) -> ProjectPatchRequest {
    ProjectPatchRequest { project_id: id.into(), ..Default::default() }
}

fn vars(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

/// The project's variables as the shim index holds them for `site`.
fn shim_vars(w: &World, site: &str) -> Vec<(String, String)> {
    let index = w.shim_index.current();
    let entry = index.projects.iter().find(|e| e.site.to_string() == site).unwrap_or_else(|| panic!("{site} is in the shim index"));
    entry.target.env_vars.effective().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

#[tokio::test]
async fn patch_sets_replaces_and_removes_variables_and_regenerates_the_site() {
    let w = World::with_basics();
    w.project("shop", r"C:\sites\shop", true);
    w.hosts_in_sync();

    let request = ProjectPatchRequest { env_set: vars(&[("APP_ENV", "local"), ("MAIL_FROM", "Shop <shop@example.test>, sales")]), ..patch("shop") };
    let dto = w.patch_project().execute(request, &cli()).await.unwrap();
    assert_eq!(dto.environment.env_vars, vars(&[("APP_ENV", "local"), ("MAIL_FROM", "Shop <shop@example.test>, sales")]));

    let vhost = w.web.last().unwrap().vhosts.into_iter().next().unwrap();
    assert_eq!(vhost.env_vars.get("APP_ENV").map(|v| v.as_str()), Some("local"), "the vhost gets them (SetEnv)");
    assert_eq!(shim_vars(&w, "shop"), [("APP_ENV".to_string(), "local".to_string()), ("MAIL_FROM".into(), "Shop <shop@example.test>, sales".into())]);

    // Removals first, then additions; a name in another case replaces the stored one; removing
    // an absent variable is a no-op.
    let request = ProjectPatchRequest { env_unset: vec!["mail_from".into(), "NOT_THERE".into()], env_set: vars(&[("app_env", "testing")]), ..patch("shop") };
    let dto = w.patch_project().execute(request, &cli()).await.unwrap();
    assert_eq!(dto.environment.env_vars, vars(&[("app_env", "testing")]));
    assert_eq!(shim_vars(&w, "shop"), [("app_env".to_string(), "testing".to_string())]);
}

#[tokio::test]
async fn patch_refuses_reserved_names_bad_values_and_contradictions_without_saving() {
    let w = World::with_basics();
    w.project("shop", r"C:\sites\shop", false);
    let before = w.projects.list().unwrap();
    for request in [
        ProjectPatchRequest { env_set: vars(&[("PATH", "C:/evil")]), ..patch("shop") },
        ProjectPatchRequest { env_set: vars(&[("phprc", "C:/evil")]), ..patch("shop") },
        ProjectPatchRequest { env_set: vars(&[("LAMPO_PROJECT", "other")]), ..patch("shop") },
        ProjectPatchRequest { env_set: vars(&[("PHP_INI_SCAN_DIR", "C:/evil")]), ..patch("shop") },
        ProjectPatchRequest { env_set: vars(&[("APP-ENV", "x")]), ..patch("shop") },
        ProjectPatchRequest { env_set: vars(&[("APP_ENV", "a\nb")]), ..patch("shop") },
        ProjectPatchRequest { env_set: vars(&[("APP_ENV", "${SRVROOT}")]), ..patch("shop") },
        ProjectPatchRequest { env_set: vars(&[("APP_ENV", "a"), ("app_env", "b")]), ..patch("shop") },
        ProjectPatchRequest { env_set: vars(&[("APP_ENV", "a")]), env_unset: vec!["App_Env".into()], ..patch("shop") },
        ProjectPatchRequest { env_unset: vec!["bad name".into()], ..patch("shop") },
    ] {
        let names: Vec<String> = request.env_set.keys().cloned().collect();
        let err = w.patch_project().execute(request, &cli()).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::Validation, "{names:?}: {}", err.message);
    }
    assert_eq!(w.projects.list().unwrap(), before, "nothing saved");

    let err = w.patch_project().execute(ProjectPatchRequest { env_set: vars(&[("DB_PASSWORD", "s3cr3t\n")]), ..patch("shop") }, &cli()).await.unwrap_err();
    assert!(err.message.contains("DB_PASSWORD") && !err.message.contains("s3cr3t"), "the value is never echoed: {}", err.message);
}

#[tokio::test]
async fn a_stored_reserved_name_is_never_exported_and_can_be_removed() {
    let w = World::with_basics();
    w.project("shop", r"C:\sites\shop", true);
    w.hosts_in_sync();
    // A file edited by hand: it loads (structural checks only) but the name is Lampo's.
    let stored: ProjectEnvVars = serde_json::from_str(r#"{"PHPRC": "C:/evil", "APP_ENV": "local"}"#).unwrap();
    w.update_project("shop", |p| p.environment.env_vars = stored);

    let err = w.patch_project().execute(ProjectPatchRequest { env_set: vars(&[("DEBUG", "1")]), ..patch("shop") }, &cli()).await.unwrap_err();
    assert!(err.message.contains("PHPRC") && err.message.contains("managed by Lampo"), "never saved again unfixed: {}", err.message);
    let dto = w.patch_project().execute(ProjectPatchRequest { env_unset: vec!["PHPRC".into()], ..patch("shop") }, &cli()).await.unwrap();
    assert_eq!(dto.environment.env_vars, vars(&[("APP_ENV", "local")]));
}

#[tokio::test]
async fn create_adds_the_requested_variables_to_the_preset_environment() {
    let w = World::with_basics();
    w.fs.add(r"C:\sites\api");
    w.hosts_in_sync();
    let detect = Arc::new(DetectProject::new(vec![], w.projects.clone(), w.fs.clone()));
    let create = CreateProject::new(w.project_writer(), w.presets(), detect, w.fs.clone(), w.database_service());
    let request = CreateProjectRequest { root: r"C:\sites\api".into(), env_vars: vars(&[("APP_ENV", "local")]), create_database: false, ..Default::default() };
    let created = create.execute(request, &gui()).await.unwrap();
    assert_eq!(created.project.environment.env_vars, vars(&[("APP_ENV", "local")]));

    w.fs.add(r"C:\sites\bad");
    let request = CreateProjectRequest { root: r"C:\sites\bad".into(), env_vars: vars(&[("LAMPO_DB_HOST", "x")]), ..Default::default() };
    assert_eq!(create.execute(request, &gui()).await.unwrap_err().code, ErrorCode::Validation);
    assert_eq!(w.projects.list().unwrap().len(), 1, "nothing created");
}

#[tokio::test]
async fn worktree_environments_follow_the_parent_variables() {
    let w = World::with_basics();
    w.project("shop", r"C:\sites\shop", false);
    w.hosts_in_sync();
    w.git.linked(r"D:\wt\login", r"C:\sites\shop", Some("feature/login"));
    w.fs.add(r"D:\wt\login");
    w.patch_project().execute(ProjectPatchRequest { env_set: vars(&[("APP_ENV", "local")]), ..patch("shop") }, &cli()).await.unwrap();
    let request = WorktreeUpRequest { path: r"D:\wt\login".into(), project_id: None, database: DatabaseModeDto::Shared };
    let up = w.worktree_up().execute(request, &gui(), &RecordingProgress::default(), &CancelFlag::new()).await.unwrap();
    let site = up.environment.site.clone();

    assert_eq!(shim_vars(&w, &site), [("APP_ENV".to_string(), "local".to_string())]);
    let vhost = w.web.last().unwrap().vhosts.into_iter().find(|v| v.site.to_string() == site).expect("the worktree is served");
    assert_eq!(vhost.env_vars.get("APP_ENV").map(|v| v.as_str()), Some("local"));
}

#[tokio::test]
async fn lampo_env_exports_the_variables_after_lampo_ones() {
    let w = World::with_basics();
    w.project("shop", r"C:\sites\shop", false);
    w.update_project("shop", |p| p.environment.env_vars = ProjectEnvVars::from_pairs([("APP_ENV", "local")]).unwrap());
    w.env.write_generated_files().unwrap();
    let tc = ToolchainQuery::new(w.resolver(), w.paths.clone());
    let env = tc.env(&ResolveRequestDto { cwd: r"C:\sites\shop".into(), ..Default::default() }).unwrap();
    assert_eq!(env.vars["APP_ENV"], "local");
    assert_eq!(env.vars["LAMPO_PROJECT"], "shop", "Lampo's own variables are still there");
}
