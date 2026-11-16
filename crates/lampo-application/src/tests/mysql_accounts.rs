//! MySQL accounts: root's password and the projects' dedicated accounts on the running
//! instances, the transition of projects still on root, and the credentials shown and handed to
//! the shims.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use lampo_domain::mysql::accounts::MysqlCredentials;
use lampo_domain::ports::{ProjectRepository, WorktreeRepository};
use lampo_domain::project::{ProjectId, ProjectRoot};
use lampo_domain::service::ServiceKind;
use lampo_domain::tcp_port::Port;
use lampo_domain::worktree::{DatabaseMode, Occupancy, WorktreeEnvironment};

use super::cli;
use crate::dto::*;
use crate::testing::*;
use crate::use_cases::*;

fn dedicated(w: &World, id: &str) {
    w.project(id, &format!(r"C:\sites\{id}"), false);
    w.update_project(id, |p| p.mysql_credentials = MysqlCredentials::Project);
}

fn worktree_of(w: &World, parent: &str, root: &str, port: u16) {
    let p = w.projects.get(&ProjectId::new(parent).unwrap()).unwrap().unwrap();
    let existing = w.worktrees.list_all().unwrap();
    let env =
        WorktreeEnvironment::derive(&p, &Occupancy { worktrees: &existing, ..Default::default() }, ProjectRoot::new(root).unwrap(), None, Port::of(port), &DatabaseMode::Fresh, 1).unwrap();
    w.worktrees.save(&env).unwrap();
    w.fs.add(root);
}

fn root_password(w: &World) -> Option<String> {
    w.secrets.root.lock().unwrap().get(&mysql_id("8.4")).map(|p| p.expose().to_string())
}

#[tokio::test]
async fn a_start_secures_root_and_gives_each_dedicated_project_its_databases() {
    let w = World::with_basics();
    dedicated(&w, "shop");
    dedicated(&w, "blog");
    worktree_of(&w, "shop", r"D:\wt\fix", 8105);
    w.hosts_in_sync();

    let report = w.env.sync(&cli()).await.unwrap();
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    assert_eq!(root_password(&w).as_deref(), Some("generated-1"), "generated and stored before it is applied");
    assert_eq!(
        w.mysql_accounts.calls(),
        ["secure-root:3306:generated-1", "accounts:3306:lampo_blog=blog;lampo_shop=shop+shop_wt_fix"],
        "root first, then the accounts as root with its password"
    );
    let stored: Vec<String> = w.secrets.projects.lock().unwrap().keys().map(ToString::to_string).collect();
    assert_eq!(stored, ["blog", "shop"]);

    w.env.sync(&cli()).await.unwrap();
    assert_eq!(w.mysql_accounts.calls().len(), 2, "an unchanged plan is not applied again");
}

#[tokio::test]
async fn a_project_still_on_root_keeps_root_without_a_password() {
    let w = World::with_basics();
    w.project("legacy", r"C:\sites\legacy", false);
    dedicated(&w, "shop");
    w.hosts_in_sync();
    w.env.sync(&cli()).await.unwrap();
    assert_eq!(w.mysql_accounts.calls(), ["accounts:3306:lampo_shop=shop"], "the legacy .env keeps working");
    assert_eq!(root_password(&w), None);
}

/// An existing installation: root without a password until its last project adopts a dedicated
/// account; then the next sync migrates the running instance.
#[tokio::test]
async fn switching_the_last_root_project_migrates_the_running_instance() {
    let w = World::with_basics();
    w.project("legacy", r"C:\sites\legacy", false);
    w.hosts_in_sync();
    w.env.sync(&cli()).await.unwrap();
    assert_eq!(w.mysql_accounts.calls(), ["accounts:3306:"], "stale lampo_ accounts are still reconciled");

    let request = ProjectPatchRequest { project_id: "legacy".into(), mysql_credentials: Some(MysqlCredentialsDto::Project), ..Default::default() };
    let dto = w.patch_project().execute(request, &cli()).await.unwrap();
    assert_eq!(dto.mysql_credentials, MysqlCredentialsDto::Project);
    let db = dto.database.unwrap();
    assert_eq!((db.user.as_str(), db.password.as_str()), ("lampo_legacy", "generated-1"), "the new credentials for its .env");
    assert_eq!(w.mysql_accounts.calls()[1..], ["secure-root:3306:generated-2", "accounts:3306:lampo_legacy=legacy"]);
    assert!(w.mysql.calls().iter().all(|c| !c.starts_with("restart")), "no restart: the accounts change on the running server");
}

#[tokio::test]
async fn a_failure_is_a_warning_and_the_next_sync_tries_again() {
    let w = World::with_basics();
    dedicated(&w, "shop");
    w.hosts_in_sync();
    w.mysql_accounts.fail_next.store(true, Ordering::SeqCst);
    let report = w.env.sync(&cli()).await.unwrap();
    assert!(report.warnings.iter().any(|m| m.contains("the MySQL accounts of mysql-8.4 were not updated") && m.contains("Lost connection")), "{:?}", report.warnings);
    assert!(w.mysql_accounts.calls().is_empty());

    w.env.sync(&cli()).await.unwrap();
    assert_eq!(w.mysql_accounts.calls(), ["secure-root:3306:generated-1", "accounts:3306:lampo_shop=shop"], "the stored root password is the one applied");
}

#[tokio::test]
async fn a_restarted_instance_is_provisioned_again_and_one_started_on_demand_too() {
    let w = World::with_basics();
    dedicated(&w, "shop");
    w.hosts_in_sync();
    w.env.sync(&cli()).await.unwrap();
    let mysql = mysql_id("8.4");
    ServiceLifecycle::stop(w.env.as_ref(), &mysql).await.unwrap();
    assert!(ServiceLifecycle::ensure(w.env.as_ref(), &mysql).await.unwrap());
    assert_eq!(w.mysql_accounts.calls()[2..], ["secure-root:3306:generated-1", "accounts:3306:lampo_shop=shop"], "the password already stored is reused");
}

#[tokio::test]
async fn new_projects_follow_the_setting_and_get_their_credentials_shown() {
    let w = World::with_basics();
    w.hosts_in_sync();
    let create = CreateProject::new(w.project_writer(), w.presets(), Arc::new(DetectProject::new(Vec::new(), w.projects.clone(), w.fs.clone())), w.fs.clone(), w.database_service());
    for (folder, on, expected) in [(r"C:\sites\fresh", true, MysqlCredentialsDto::Project), (r"C:\sites\old-style", false, MysqlCredentialsDto::Root)] {
        w.settings.0.lock().unwrap().mysql_project_credentials = on;
        w.fs.add(folder);
        let created = create
            .execute(CreateProjectRequest { root: folder.into(), preset_id: Some("laravel".into()), addressing: AddressingDto::NoElevation, ..Default::default() }, &cli())
            .await
            .unwrap();
        assert_eq!(created.project.mysql_credentials, expected, "{folder}");
    }
    let fresh = w.project_queries().get("fresh").unwrap().database.unwrap();
    assert_eq!(fresh.user, "lampo_fresh");
    assert!(fresh.password.starts_with("generated-"), "{}", fresh.password);
    // Creating `fresh` started MySQL with no project on root: root got its password then, and a
    // project put on root afterwards is shown that password (it is never removed).
    let old = w.project_queries().get("old-style").unwrap().database.unwrap();
    assert_eq!(Some(old.password.as_str()), root_password(&w).as_deref());
    assert_eq!(old.user, "root");
}

#[tokio::test]
async fn root_without_a_stored_password_is_shown_with_an_empty_one() {
    let w = World::with_basics();
    w.project("legacy", r"C:\sites\legacy", false);
    let db = w.project_queries().get("legacy").unwrap().database.unwrap();
    assert_eq!((db.user.as_str(), db.password.as_str()), ("root", ""), "as before dedicated accounts");
}

#[tokio::test]
async fn the_shim_index_names_the_dedicated_account_of_projects_and_their_worktrees() {
    let w = World::with_basics();
    dedicated(&w, "shop");
    w.project("legacy", r"C:\sites\legacy", false);
    worktree_of(&w, "shop", r"D:\wt\fix", 8105);
    w.env.write_generated_files().unwrap();
    let index = w.shim_index.current();
    let mysql_of = |site: &str| index.by_site(&site.parse().unwrap()).unwrap().target.services[&ServiceKind::MYSQL].clone();
    for site in ["shop", "shop@fix"] {
        let m = mysql_of(site);
        assert_eq!((m.user.as_deref(), m.project_account.as_ref().map(ProjectId::as_str)), (Some("lampo_shop"), Some("shop")), "{site}");
    }
    let legacy = mysql_of("legacy");
    assert_eq!((legacy.user.as_deref(), legacy.project_account), (Some("root"), None));
    let default = index.default.unwrap().services[&ServiceKind::MYSQL].clone();
    assert_eq!((default.user.as_deref(), default.project_account), (Some("root"), None), "outside projects: root");
}

#[tokio::test]
async fn the_gui_switches_a_projects_credentials_and_older_clients_keep_them() {
    let w = World::with_basics();
    w.project("shop", r"C:\sites\shop", false);
    w.hosts_in_sync();
    let update = UpdateProject::new(w.project_writer());
    let current = w.project_queries().get("shop").unwrap();
    let request = |credentials| UpdateProjectRequest {
        project_id: "shop".into(),
        name: current.name.clone(),
        addresses: current.addresses.clone(),
        environment: current.environment.clone(),
        database_name: current.database_name.clone(),
        tls: TlsModeDto::Disabled,
        mysql_credentials: credentials,
    };
    assert_eq!(update.execute(request(Some(MysqlCredentialsDto::Project)), &cli()).await.unwrap().mysql_credentials, MysqlCredentialsDto::Project);
    assert_eq!(update.execute(request(None), &cli()).await.unwrap().mysql_credentials, MysqlCredentialsDto::Project, "absent: unchanged");
}

#[tokio::test]
async fn the_setting_for_new_projects_needs_no_sync() {
    let w = World::with_basics();
    w.hosts_in_sync();
    let result = w.patch_settings().execute(SettingsPatchRequest { mysql_project_credentials: Some(false), ..Default::default() }, &cli()).await.unwrap();
    assert!(!result.settings.mysql_project_credentials);
    assert_eq!(result.changes.iter().map(|c| (c.setting.as_str(), c.from.as_str(), c.to.as_str())).collect::<Vec<_>>(), [("project-credentials", "on", "off")]);
    assert!(result.sync.started.is_empty(), "only projects created later are affected");
}
