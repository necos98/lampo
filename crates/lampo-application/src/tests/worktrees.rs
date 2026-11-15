//! Worktree environments: idempotent `up`, port allocation, database modes, `down`, prune.

use std::sync::atomic::Ordering;

use lampo_domain::events::DomainEvent;
use lampo_domain::ports::WorktreeRepository;
use lampo_domain::progress::CancelFlag;
use lampo_domain::project::ProjectRoot;

use super::gui;
use crate::dto::*;
use crate::error::ErrorCode;
use crate::testing::{RecordingProgress, World};
use crate::use_cases::WorktreeQueries;

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

    let queries = WorktreeQueries::new(w.worktrees.clone(), w.projects.clone(), w.settings.clone(), w.fs.clone());
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
