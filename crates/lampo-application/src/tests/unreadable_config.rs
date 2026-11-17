//! Configuration files that do not load: their projects are hidden, not gone. Nothing treats
//! them as missing (prune, doctor, new ids), and the read-only views say they are there.

use std::sync::Arc;

use lampo_domain::ports::{ProjectRepository, WorktreeRepository};
use lampo_domain::progress::CancelFlag;
use lampo_domain::project::{ProjectId, ProjectRoot};
use lampo_domain::tcp_port::Port;
use lampo_domain::worktree::{DatabaseMode, Occupancy, WorktreeEnvironment};

use super::gui;
use crate::dto::*;
use crate::error::ErrorCode;
use crate::testing::{FakeLocator, FakeTransfer, RecordingProgress, World};
use crate::use_cases::{CreateProject, DetectProject, Doctor};

const SHOP_FILE: &str = r"C:\Lampo\projects\shop.toml";

fn shop_id() -> ProjectId {
    ProjectId::new("shop").unwrap()
}

/// `shop` with a worktree environment that owns a database, then its file edited into something
/// that does not load. The environment's folder still exists.
async fn shop_hidden_with_a_worktree() -> World {
    let w = World::with_basics();
    w.project("shop", r"C:\sites\shop", false);
    w.hosts_in_sync();
    w.git.linked(r"D:\wt\login", r"C:\sites\shop", Some("feature/login"));
    w.fs.add(r"D:\wt\login");
    let request = WorktreeUpRequest { path: r"D:\wt\login".into(), project_id: None, database: DatabaseModeDto::Fresh };
    w.worktree_up().execute(request, &gui(), &RecordingProgress::default(), &CancelFlag::new()).await.unwrap();
    w.projects.delete(&shop_id()).unwrap();
    w.unreadable_project(SHOP_FILE, &[]);
    w
}

/// An environment whose parent was really deleted: an orphan, which prune still removes.
fn orphan(w: &World) -> WorktreeEnvironment {
    let blog = w.project("blog", r"C:\sites\blog", false);
    let env = WorktreeEnvironment::derive(&blog, &Occupancy::default(), ProjectRoot::new(r"D:\wt\blog-fix").unwrap(), None, Port::of(8150), &DatabaseMode::Shared, 1).unwrap();
    w.worktrees.save(&env).unwrap();
    w.fs.add(r"D:\wt\blog-fix");
    w.projects.delete(&blog.id).unwrap();
    env
}

#[tokio::test]
async fn the_environments_of_an_unreadable_project_are_neither_stale_nor_pruned() {
    let w = shop_hidden_with_a_worktree().await;
    let orphan = orphan(&w);

    let report = w.prune_worktrees().execute(PruneWorktreesRequest { project_id: None, dry_run: false, drop_databases: true }, &gui()).await.unwrap();
    assert_eq!(report.removed, [orphan.site_id().to_string()], "only the orphan");
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    assert_eq!(w.worktrees.list_for(&shop_id()).unwrap().len(), 1, "port and database stay tracked");
    assert!(w.db.has("shop_wt_feature_login"));

    let doctor = Doctor::new(w.doctor_deps(Arc::new(FakeLocator::default()))).run().await.unwrap();
    assert!(doctor.stale_worktrees.is_empty(), "{:?}", doctor.stale_worktrees);
}

#[tokio::test]
async fn create_and_import_never_take_the_id_of_an_unreadable_file() {
    let w = shop_hidden_with_a_worktree().await;
    w.fs.add(r"D:\copy\shop");
    let detect = Arc::new(DetectProject::new(Vec::new(), w.projects.clone(), w.fs.clone()));
    assert_eq!(detect.execute(r"D:\copy\shop").unwrap().suggested_id, "shop-2");
    let create = CreateProject::new(w.project_writer(), w.presets(), detect, w.fs.clone(), w.database_service());

    let explicit = CreateProjectRequest { root: r"D:\copy\shop".into(), id: Some("shop".into()), addressing: AddressingDto::Standard, ..Default::default() };
    let err = create.execute(explicit, &gui()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Conflict, "{err:?}");
    assert!(err.message.contains("`shop`") && err.message.contains("does not load"), "{}", err.message);
    assert!(w.projects.list().unwrap().is_empty(), "nothing saved over the file");

    let created = create.execute(CreateProjectRequest { root: r"D:\copy\shop".into(), addressing: AddressingDto::Standard, ..Default::default() }, &gui()).await.unwrap();
    assert_eq!(created.project.id, "shop-2");
    assert!(created.project.worktrees.is_empty(), "the hidden project's environments stay its own");

    let exported = w.project("shop", r"E:\other\shop", false);
    w.projects.delete(&exported.id).unwrap();
    let transfer = Arc::new(FakeTransfer::default());
    *transfer.to_import.lock().unwrap() = Some(exported);
    let imported = w.import_project(transfer).execute(ImportProjectRequest { file: r"D:\shop.toml".into(), root: None }, &gui()).await.unwrap();
    assert_eq!(imported.project.id, "shop-3");
    assert_eq!(imported.project.urls, ["http://shop-3.test", "http://shop-3.localhost"]);
    assert!(w.projects.get(&shop_id()).unwrap().is_none(), "the unreadable file was not replaced");
}

#[tokio::test]
async fn status_and_doctor_name_every_unreadable_project_file() {
    let w = shop_hidden_with_a_worktree().await;
    let reason = format!("invalid data in {SHOP_FILE}: a project needs at least one address");
    let expected = [UnreadableProjectDto { file: SHOP_FILE.into(), reason: reason.clone() }];

    let summaries = w.project_queries().summaries().unwrap();
    assert!(summaries.projects.is_empty());
    assert_eq!(summaries.unreadable, expected);

    let report = Doctor::new(w.doctor_deps(Arc::new(FakeLocator::default()))).run().await.unwrap();
    assert_eq!(report.unreadable_projects, expected);
    let line = report.checks.iter().find(|c| c.id == "project-files").unwrap();
    assert_eq!(line.status, CheckStatus::Warning);
    assert!(line.message.contains(&reason), "{}", line.message);
}

#[tokio::test]
async fn doctor_reports_an_invalid_settings_file_instead_of_failing() {
    let w = World::with_basics();
    w.settings.fail_loads("port 3306 is assigned to both apache and mysql-8.4");

    let report = Doctor::new(w.doctor_deps(Arc::new(FakeLocator::default()))).run().await.unwrap();
    assert!(!report.ok);
    let line = report.checks.iter().find(|c| c.id == "settings").unwrap();
    assert_eq!(line.status, CheckStatus::Error);
    assert!(line.message.contains("settings.toml") && line.message.contains("port 3306"), "{}", line.message);
    assert!(report.checks.len() > 2, "the other checks still ran on the defaults");

    w.settings.3.lock().unwrap().take();
    let report = Doctor::new(w.doctor_deps(Arc::new(FakeLocator::default()))).run().await.unwrap();
    let lines: Vec<_> = report.checks.iter().filter(|c| ["settings", "project-files"].contains(&c.id.as_str())).map(|c| c.status).collect();
    assert_eq!(lines, [CheckStatus::Ok, CheckStatus::Ok]);
}
