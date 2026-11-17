//! Worktree environments: idempotent `up`, port allocation, database modes, `down`, prune.

use std::sync::atomic::Ordering;

use lampo_domain::events::DomainEvent;
use lampo_domain::mysql::MysqlSeries;
use lampo_domain::ports::{SettingsRepository, WorktreeRepository};
use lampo_domain::progress::CancelFlag;
use lampo_domain::project::ProjectRoot;
use lampo_domain::tcp_port::Port;

use super::gui;
use crate::dto::*;
use crate::error::ErrorCode;
use crate::testing::{RecordingProgress, World};
use crate::use_cases::{EnvironmentSync, ServiceEnsurer, WorktreeQueries};

fn setup() -> World {
    let w = World::with_basics();
    w.project("shop", "C:\\sites\\shop", false);
    w.hosts_in_sync();
    w.git.linked("D:\\wt\\login", "C:\\sites\\shop", Some("feature/login"));
    w.git.linked("D:\\wt\\cart", "C:\\sites\\shop", Some("feature/cart"));
    w.fs.add("D:\\wt\\login");
    w.fs.add("D:\\wt\\cart");
    w.db.dbs.lock().unwrap().insert("shop".into());
    w
}

fn up_request(path: &str, database: DatabaseModeDto) -> WorktreeUpRequest {
    WorktreeUpRequest { path: path.into(), project_id: None, database }
}

async fn up(w: &World, path: &str, database: DatabaseModeDto) -> crate::error::AppResult<WorktreeUpResultDto> {
    w.worktree_up().execute(up_request(path, database), &gui(), &RecordingProgress::default(), &CancelFlag::new()).await
}

#[tokio::test]
async fn up_is_idempotent_and_allocates_a_stable_port_from_the_range() {
    let w = setup();
    w.ports.busy(8100, "node.exe (pid 7)");
    let use_case = w.worktree_up();
    let first = use_case.execute(up_request("D:\\wt\\login\\src\\app", DatabaseModeDto::Shared), &gui(), &RecordingProgress::default(), &CancelFlag::new()).await.unwrap();
    assert!(first.created);
    let env = &first.environment;
    assert_eq!(env.site, "shop@feature-login");
    assert_eq!(env.loopback_port, 8101, "8100 is busy on the OS");
    assert_eq!(env.url, "http://127.0.0.1:8101");
    assert_eq!(env.urls, ["http://127.0.0.1:8101", "http://feature-login.shop.localhost"]);
    assert_eq!(env.root, "D:\\wt\\login");
    assert_eq!(env.database_mode, DatabaseModeDto::Shared);
    assert_eq!(env.database.as_ref().unwrap().database, "shop");
    assert!(w.events.all().contains(&DomainEvent::WorktreesChanged { project: lampo_domain::project::ProjectId::new("shop").unwrap() }));
    assert!(w.web.last().unwrap().vhosts.iter().any(|v| v.site.to_string() == "shop@feature-login"), "served although the parent is inactive");

    let again = use_case.execute(up_request("D:\\wt\\login", DatabaseModeDto::Fresh), &gui(), &RecordingProgress::default(), &CancelFlag::new()).await.unwrap();
    assert!(!again.created);
    assert_eq!(again.environment, first.environment, "same port, mode of a later call ignored");
    assert_eq!(w.worktrees.list_all().unwrap().len(), 1);

    let other = use_case.execute(up_request("D:\\wt\\cart", DatabaseModeDto::Shared), &gui(), &RecordingProgress::default(), &CancelFlag::new()).await.unwrap();
    assert_eq!(other.environment.loopback_port, 8102);
}

#[tokio::test]
async fn up_never_needs_elevation() {
    let w = setup();
    // An unrelated hosts change is pending (a project without its hosts entry yet).
    w.project("blog", "C:\\sites\\blog", true);
    up(&w, "D:\\wt\\login", DatabaseModeDto::Shared).await.unwrap();
    assert_eq!(w.hosts.writes.load(Ordering::SeqCst), 0, "no UAC prompt, even from the GUI");
}

#[tokio::test]
async fn up_requires_a_linked_worktree_or_an_explicit_project() {
    let w = setup();
    w.fs.add("E:\\copies\\shop");
    let err = up(&w, "E:\\copies\\shop", DatabaseModeDto::Shared).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Validation);

    let request = WorktreeUpRequest { path: "E:\\copies\\shop".into(), project_id: Some("shop".into()), database: DatabaseModeDto::Shared };
    let ok = w.worktree_up().execute(request, &gui(), &RecordingProgress::default(), &CancelFlag::new()).await.unwrap();
    assert_eq!(ok.environment.root, "E:\\copies\\shop");

    w.git.linked("F:\\wt\\x", "C:\\unregistered", None);
    assert_eq!(up(&w, "F:\\wt\\x", DatabaseModeDto::Shared).await.unwrap_err().code, ErrorCode::ProjectNotResolved);
}

#[tokio::test]
async fn up_maps_monorepo_subprojects_into_the_worktree() {
    let w = World::with_basics();
    w.project("api", "C:\\sites\\mono\\api", false);
    w.hosts_in_sync();
    w.git.linked("E:\\agents\\mono-feat", "C:\\sites\\mono", Some("feat"));
    let result = up(&w, "E:\\agents\\mono-feat\\api\\src", DatabaseModeDto::Shared).await.unwrap();
    assert_eq!(result.environment.project_id, "api");
    assert_eq!(result.environment.root, "E:\\agents\\mono-feat\\api");
}

#[tokio::test]
async fn fresh_mode_creates_an_owned_database() {
    let w = setup();
    let result = up(&w, "D:\\wt\\login", DatabaseModeDto::Fresh).await.unwrap();
    assert_eq!(result.environment.database_mode, DatabaseModeDto::Fresh);
    assert_eq!(result.environment.database.as_ref().unwrap().database, "shop_wt_feature_login");
    assert!(w.db.has("shop_wt_feature_login"));
    assert_eq!(w.mysql.state(&lampo_domain::mysql::MysqlSeries::DEFAULT.service_id()), lampo_domain::service::ServiceState::Running);
}

#[tokio::test]
async fn clone_mode_copies_a_parent_database_with_progress() {
    let w = setup();
    let progress = RecordingProgress::default();
    let request = up_request("D:\\wt\\login", DatabaseModeDto::Clone { source: "shop".into() });
    let result = w.worktree_up().execute(request, &gui(), &progress, &CancelFlag::new()).await.unwrap();
    assert_eq!(result.environment.database_mode, DatabaseModeDto::Clone { source: "shop".into() });
    assert!(w.db.calls().contains(&"clone:shop->shop_wt_feature_login".to_string()));
    assert!(!progress.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn clone_mode_refuses_a_system_schema_as_source() {
    let w = setup();
    let err = up(&w, "D:\\wt\\login", DatabaseModeDto::Clone { source: "MySQL".into() }).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Validation);
    assert!(w.db.calls().is_empty());
    assert!(w.worktrees.list_all().unwrap().is_empty());
}

#[tokio::test]
async fn failed_or_cancelled_clones_leave_nothing_behind() {
    let w = setup();
    let err = up(&w, "D:\\wt\\login", DatabaseModeDto::Clone { source: "missing".into() }).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);
    assert!(w.worktrees.list_all().unwrap().is_empty());

    w.db.fail_clone.store(true, Ordering::SeqCst);
    let err = up(&w, "D:\\wt\\login", DatabaseModeDto::Clone { source: "shop".into() }).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::ProcessFailed);
    assert!(w.worktrees.list_all().unwrap().is_empty());
    assert!(!w.db.has("shop_wt_feature_login"), "the partial clone is dropped");

    w.db.fail_clone.store(false, Ordering::SeqCst);
    let cancel = CancelFlag::new();
    cancel.cancel();
    let request = up_request("D:\\wt\\login", DatabaseModeDto::Clone { source: "shop".into() });
    let err = w.worktree_up().execute(request, &gui(), &RecordingProgress::default(), &cancel).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Cancelled);
    assert!(w.worktrees.list_all().unwrap().is_empty());
}

#[tokio::test]
async fn fresh_mode_is_rejected_when_the_parent_has_no_mysql() {
    let w = setup();
    w.update_project("shop", |p| p.environment.set_mysql(None));
    let err = up(&w, "D:\\wt\\login", DatabaseModeDto::Fresh).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Validation);
    assert!(w.worktrees.list_all().unwrap().is_empty());
}

#[tokio::test]
async fn down_drops_only_the_environments_own_database_and_only_when_asked() {
    let w = setup();
    up(&w, "D:\\wt\\login", DatabaseModeDto::Fresh).await.unwrap();
    up(&w, "D:\\wt\\cart", DatabaseModeDto::Shared).await.unwrap();
    let down = w.worktree_down();

    let shared = down.execute(WorktreeDownRequest { target: "shop@feature-cart".into(), drop_database: true }, &gui()).await.unwrap();
    assert_eq!(shared.database_dropped, None);
    assert!(w.db.has("shop"), "the parent's database is never dropped");

    let own = down.execute(WorktreeDownRequest { target: "D:\\wt\\login\\src".into(), drop_database: true }, &gui()).await.unwrap();
    assert_eq!(own.site, "shop@feature-login");
    assert_eq!(own.database_dropped.as_deref(), Some("shop_wt_feature_login"));
    assert!(!w.db.has("shop_wt_feature_login"));
    assert!(w.worktrees.list_all().unwrap().is_empty());

    let missing = down.execute(WorktreeDownRequest { target: "shop@nope".into(), drop_database: false }, &gui()).await.unwrap_err();
    assert_eq!(missing.code, ErrorCode::NotFound);
}

#[tokio::test]
async fn stale_environments_are_detected_and_pruned() {
    let w = setup();
    up(&w, "D:\\wt\\login", DatabaseModeDto::Fresh).await.unwrap();
    up(&w, "D:\\wt\\cart", DatabaseModeDto::Shared).await.unwrap();
    w.fs.remove("D:\\wt\\login");

    let queries = WorktreeQueries::new(w.worktrees.clone(), w.projects.clone(), w.settings.clone(), w.fs.clone(), w.passwords.clone());
    let listed = queries.list(Some("shop")).unwrap();
    assert_eq!(listed.iter().filter(|e| e.stale).map(|e| e.site.as_str()).collect::<Vec<_>>(), ["shop@feature-login"]);
    assert_eq!(queries.list(Some("nope")).unwrap_err().code, ErrorCode::NotFound);

    let prune = w.prune_worktrees();
    let dry = prune.execute(PruneWorktreesRequest { project_id: None, dry_run: true, drop_databases: true }, &gui()).await.unwrap();
    assert_eq!(dry.removed, ["shop@feature-login"]);
    assert_eq!(dry.dropped_databases, ["shop_wt_feature_login"]);
    assert_eq!(w.worktrees.list_all().unwrap().len(), 2, "dry run changes nothing");
    assert!(w.db.has("shop_wt_feature_login"));

    let real = prune.execute(PruneWorktreesRequest { project_id: Some("shop".into()), dry_run: false, drop_databases: true }, &gui()).await.unwrap();
    assert_eq!(real.removed, ["shop@feature-login"]);
    assert_eq!(real.dropped_databases, ["shop_wt_feature_login"]);
    let left = w.worktrees.list_all().unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].root, ProjectRoot::new("D:\\wt\\cart").unwrap());
    assert!(!w.web.last().unwrap().vhosts.iter().any(|v| v.site.to_string() == "shop@feature-login"));
}

#[tokio::test]
async fn prune_reports_a_failed_environment_and_still_removes_and_syncs_the_others() {
    let w = setup();
    up(&w, r"D:\wt\login", DatabaseModeDto::Fresh).await.unwrap();
    up(&w, r"D:\wt\cart", DatabaseModeDto::Fresh).await.unwrap();
    w.fs.remove(r"D:\wt\login");
    w.fs.remove(r"D:\wt\cart");
    w.db.fail_drop_of.lock().unwrap().insert("shop_wt_feature_login".into());
    let events = w.events.all().len();

    let report = w.prune_worktrees().execute(PruneWorktreesRequest { project_id: None, dry_run: false, drop_databases: true }, &gui()).await.unwrap();
    assert_eq!(report.removed, ["shop@feature-cart"]);
    assert_eq!(report.dropped_databases, ["shop_wt_feature_cart"]);
    assert_eq!(report.failed.len(), 1);
    assert_eq!((report.failed[0].site.as_str(), report.failed[0].code), ("shop@feature-login", ErrorCode::ProcessFailed));
    assert!(report.failed[0].message.contains("ERROR 1010"), "{}", report.failed[0].message);
    // The failed one stays registered with its database, for a retry.
    let left = w.worktrees.list_all().unwrap();
    assert_eq!(left.iter().map(|e| e.root.as_str()).collect::<Vec<_>>(), [r"D:\wt\login"]);
    assert!(w.db.has("shop_wt_feature_login"));
    assert!(w.events.all()[events..].contains(&DomainEvent::WorktreesChanged { project: lampo_domain::project::ProjectId::new("shop").unwrap() }));
    assert!(!w.web.last().unwrap().vhosts.iter().any(|v| v.site.to_string() == "shop@feature-cart"), "synced");
}

#[tokio::test]
async fn fresh_mode_never_adopts_a_database_left_by_an_earlier_down() {
    let w = setup();
    // Kept by a `down` without `--drop-database`: nobody owns it any more, but it holds data.
    w.db.dbs.lock().unwrap().insert("shop_wt_feature_login".into());
    let result = up(&w, r"D:\wt\login", DatabaseModeDto::Fresh).await.unwrap();
    assert_eq!(result.environment.database.as_ref().unwrap().database, "shop_wt_feature_login_2");
    assert!(w.db.has("shop_wt_feature_login_2"));

    let down = w.worktree_down().execute(WorktreeDownRequest { target: "shop@feature-login".into(), drop_database: true }, &gui()).await.unwrap();
    assert_eq!(down.database_dropped.as_deref(), Some("shop_wt_feature_login_2"));
    assert!(w.db.has("shop_wt_feature_login"), "the leftover database is never dropped by an environment that did not create it");
}

#[tokio::test]
async fn own_databases_never_reuse_the_database_of_another_site() {
    let w = setup();
    w.project("blog", r"C:\sites\blog", false);
    w.update_project("blog", |p| p.set_database_name(lampo_domain::mysql::DatabaseName::new("shop_wt_feature_login").unwrap()));
    let result = up(&w, r"D:\wt\login", DatabaseModeDto::Fresh).await.unwrap();
    assert_eq!(result.environment.database.as_ref().unwrap().database, "shop_wt_feature_login_2");
}

#[tokio::test]
async fn a_database_created_concurrently_by_another_client_is_a_conflict_and_is_kept() {
    for mode in [DatabaseModeDto::Fresh, DatabaseModeDto::Clone { source: "shop".into() }] {
        let w = setup();
        // Created between the listing and the `CREATE DATABASE` (another MySQL client).
        w.db.dbs.lock().unwrap().insert("shop_wt_feature_login".into());
        w.db.unlisted.lock().unwrap().insert("shop_wt_feature_login".into());
        let err = up(&w, r"D:\wt\login", mode.clone()).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::Conflict, "{mode:?}: {err}");
        assert!(w.worktrees.list_all().unwrap().is_empty(), "{mode:?}");
        assert!(w.db.has("shop_wt_feature_login"), "{mode:?}: another client's database is never dropped");
    }
}

#[tokio::test]
async fn up_refuses_a_folder_registered_as_a_project() {
    let w = setup();
    w.project("login-app", r"D:\wt\login", false);
    let err = up(&w, r"D:\wt\login", DatabaseModeDto::Shared).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Conflict, "{err}");
    assert!(err.message.contains("login-app"), "{}", err.message);
    assert!(w.worktrees.list_all().unwrap().is_empty());
}

#[tokio::test]
async fn up_picks_a_slug_whose_alias_no_project_uses() {
    let w = setup();
    w.project("blog", r"C:\sites\blog", false);
    let taken = lampo_domain::hostname::Hostname::new("feature-login.shop.localhost").unwrap();
    w.update_project("blog", |p| p.set_addresses(vec![lampo_domain::address::SiteAddress::Localhost(taken)]).unwrap());
    let result = up(&w, r"D:\wt\login", DatabaseModeDto::Shared).await.unwrap();
    assert_eq!(result.environment.site, "shop@feature-login-2");
    assert_eq!(result.environment.urls[1], "http://feature-login-2.shop.localhost");
}

#[tokio::test]
async fn up_reports_a_failure_that_comes_after_an_unrelated_pending_hosts_change() {
    let w = setup();
    // An unrelated hosts change is pending (a project without its hosts entry yet) and the
    // hosts step runs before the services.
    w.project("blog", r"C:\sites\blog", true);
    w.ports.busy(80, "nginx.exe (pid 9)");
    let err = up(&w, r"D:\wt\login", DatabaseModeDto::Shared).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::PortInUse, "{err}");
    assert_eq!(err.details.unwrap()["port"], 80, "with its own details");
    assert_eq!(w.hosts.writes.load(Ordering::SeqCst), 0, "still no UAC prompt");
}

#[tokio::test]
async fn up_creates_its_database_on_the_port_mysql_runs_on_while_its_restart_waits() {
    let w = setup();
    w.env.sync(&gui()).await.unwrap();
    let lease = w.service_control.ensure_running(MysqlSeries::DEFAULT.service_id()).await.unwrap();
    let mut settings = w.settings.load().unwrap();
    settings.services[0].port = Port::of(3316);
    w.settings.save(&settings).unwrap();
    w.env.sync(&gui()).await.unwrap();

    up(&w, r"D:\wt\login", DatabaseModeDto::Fresh).await.unwrap();
    let creates: Vec<String> = w.db.calls().into_iter().filter(|c| c.starts_with("create:")).collect();
    assert!(creates.len() == 1 && creates[0].starts_with("create:3306:"), "the restart onto 3316 is still postponed: {creates:?}");
    drop(lease);
}
