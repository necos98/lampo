//! The rules every project save shares (GUI update, `lampo project set`, create, import, PHP
//! switch) and the removal of a project under the allocator lock.

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::task::{Context, Poll, Waker};

use lampo_domain::events::DomainEvent;
use lampo_domain::package::PackageKind;
use lampo_domain::ports::{ProjectRepository, WorktreeRepository};
use lampo_domain::project::{DisplayName, ProjectId, ProjectRoot};
use lampo_domain::worktree::{DatabaseMode, Occupancy, WorktreeEnvironment};
use lampo_domain::tcp_port::Port;

use super::{cli, gui};
use crate::context::CallContext;
use crate::dto::*;
use crate::error::ErrorCode;
use crate::testing::*;
use crate::use_cases::*;

fn mysql(version: &str) -> ServiceUseDto {
    ServiceUseDto { kind: "mysql".into(), version: Some(version.into()) }
}

/// The GUI's full save of `id` as stored, with `edit` applied to the request.
async fn gui_update(w: &World, sync: Arc<CountingSync>, id: &str, edit: impl FnOnce(&mut UpdateProjectRequest)) -> crate::error::AppResult<ProjectDto> {
    let current = w.project_queries().get(id).unwrap();
    let mut request = UpdateProjectRequest {
        project_id: id.into(),
        name: current.name.clone(),
        addresses: current.addresses.clone(),
        environment: current.environment.clone(),
        database_name: current.database_name.clone(),
        tls: TlsModeDto::Disabled,
    };
    edit(&mut request);
    UpdateProject::new(w.project_writer_with(sync)).execute(request, &gui()).await
}

#[tokio::test]
async fn the_gui_update_refuses_what_the_cli_patch_refuses() {
    let w = World::with_basics();
    w.project("shop", r"C:\sites\shop", true);
    w.hosts_in_sync();
    let before = w.projects.list().unwrap();
    let sync = Arc::new(CountingSync::default());

    // MySQL 8.0 is not a configured instance: saved, it would fail every sync.
    let err = gui_update(&w, sync.clone(), "shop", |r| r.environment.services = vec![mysql("8.0")]).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Validation);
    assert!(err.message.contains("8.0") && err.message.contains("configured"), "{}", err.message);
    // Xdebug is not installed for the project's PHP 8.3.
    let err = gui_update(&w, sync.clone(), "shop", |r| r.environment.addons = vec!["xdebug".into()]).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::PackageMissing);
    assert!(err.message.contains("lampo package install xdebug --php 8.3"), "{}", err.message);
    assert_eq!(err.details, Some(serde_json::json!({ "package": "xdebug", "php": "8.3" })), "the GUI names the add-on and its PHP");
    // PHP 8.3 ships no `intll`.
    let err = gui_update(&w, sync.clone(), "shop", |r| r.environment.extensions = vec!["intll".into()]).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Validation);
    assert!(err.message.contains("`intll`"), "{}", err.message);
    // A database name without MySQL used to be dropped silently.
    let err = gui_update(&w, sync.clone(), "shop", |r| {
        r.environment.services.clear();
        r.database_name = Some("shop_main".into());
    })
    .await
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::Validation);

    assert_eq!(w.projects.list().unwrap(), before, "nothing saved");
    assert_eq!(sync.calls.load(Ordering::SeqCst), 0, "nothing synced");

    // What is valid still saves: the configured MySQL 5.7, an installed add-on, a bundled extension.
    w.installed.add(PackageKind::XDEBUG, "3.3.2", Some("8.3"));
    let dto = gui_update(&w, sync.clone(), "shop", |r| {
        r.environment.services = vec![mysql("5.7")];
        r.environment.addons = vec!["xdebug".into()];
        r.environment.extensions = vec!["intl".into()];
    })
    .await
    .unwrap();
    assert_eq!((dto.environment.service_version("mysql"), dto.environment.addons.as_slice()), (Some("5.7"), ["xdebug".to_string()].as_slice()));
}

#[tokio::test]
async fn a_save_checks_only_what_it_changes_so_an_outdated_project_can_still_be_edited() {
    let w = World::with_basics();
    w.project("shop", r"C:\sites\shop", true);
    // Stored before the rule: an add-on that is no longer installed.
    w.update_project("shop", |p| {
        p.environment.addons.insert(PackageKind::XDEBUG);
    });
    w.hosts_in_sync();
    let sync = Arc::new(CountingSync::default());

    let dto = gui_update(&w, sync.clone(), "shop", |r| r.name = "Shop (renamed)".into()).await.unwrap();
    assert_eq!(dto.name, "Shop (renamed)");
    let dto = SetProjectActive::new(w.project_writer_with(sync)).execute(SetProjectActiveRequest { project_id: "shop".into(), active: false }, &cli()).await.unwrap();
    assert!(!dto.active, "the broken project can be switched off");
}

#[tokio::test]
async fn a_php_switch_rechecks_the_add_ons_but_not_the_bundled_extensions() {
    let w = World::with_basics();
    w.project("shop", r"C:\sites\shop", false);
    w.installed.add(PackageKind::XDEBUG, "3.3.2", Some("8.3"));
    w.update_project("shop", |p| {
        p.environment.addons.insert(PackageKind::XDEBUG);
        // Named as PHP 7.x builds ship GD: a build without it comments the line out, so it never
        // blocks a switch.
        p.environment.extensions.insert(lampo_domain::php::ExtensionName::new("gd2").unwrap());
    });
    w.hosts_in_sync();
    let before = w.projects.list().unwrap();
    let set_php = SetProjectPhp::new(w.project_writer(), w.installed.clone());

    let err = set_php.execute(SetProjectPhpRequest { project_id: "shop".into(), php_version: "8.4".into() }, &cli()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::PackageMissing);
    assert!(err.message.contains("lampo package install xdebug --php 8.4"), "{}", err.message);
    assert_eq!(w.projects.list().unwrap(), before, "nothing saved");

    w.installed.add(PackageKind::XDEBUG, "3.3.2", Some("8.4"));
    let dto = set_php.execute(SetProjectPhpRequest { project_id: "shop".into(), php_version: "8.4".into() }, &cli()).await.unwrap();
    assert_eq!((dto.environment.php_version.as_str(), dto.environment.extensions.as_slice()), ("8.4", ["gd2".to_string()].as_slice()));
}

/// `lampo project set --php` switches the minor in the same atomic patch as the other flags,
/// with the checks of `set-php`: the PHP and the add-ons kept must be installed for it.
#[tokio::test]
async fn a_cli_patch_switches_php_with_the_checks_of_set_php() {
    let w = World::with_basics();
    w.project("shop", r"C:\sites\shop", false);
    w.installed.add(PackageKind::XDEBUG, "3.3.2", Some("8.3"));
    w.update_project("shop", |p| {
        p.environment.addons.insert(PackageKind::XDEBUG);
    });
    w.hosts_in_sync();
    let before = w.projects.list().unwrap();
    let sync = Arc::new(CountingSync::default());
    let patcher = w.patch_project_with(sync.clone());
    let switch = |php: &str| ProjectPatchRequest { project_id: "shop".into(), php_version: Some(php.into()), ..Default::default() };

    let err = patcher.execute(switch("7.4"), &cli()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::PackageMissing);
    assert!(err.message.contains("PHP 7.4"), "{}", err.message);
    let err = patcher.execute(ProjectPatchRequest { project_id: "nope".into(), ..switch("7.4") }, &cli()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound, "an unknown project is reported before the missing PHP");
    assert_eq!(patcher.execute(switch("8"), &cli()).await.unwrap_err().code, ErrorCode::Validation);
    let err = patcher.execute(switch("8.4"), &cli()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::PackageMissing);
    assert!(err.message.contains("lampo package install xdebug --php 8.4"), "{}", err.message);
    assert_eq!(w.projects.list().unwrap(), before, "nothing saved");
    assert_eq!(sync.calls.load(Ordering::SeqCst), 0);

    // Dropping the add-on in the same patch makes the switch valid.
    let request = ProjectPatchRequest { addons_remove: vec!["xdebug".into()], extensions_add: vec!["intl".into()], ..switch("8.4") };
    let dto = patcher.execute(request, &cli()).await.unwrap();
    assert_eq!(dto.environment.php_version, "8.4");
    assert!(dto.environment.addons.is_empty());
    assert_eq!(dto.environment.extensions, ["intl"]);
    assert_eq!(sync.calls.load(Ordering::SeqCst), 1, "one save, one sync");
}

#[tokio::test]
async fn an_import_follows_the_same_rules() {
    let w = World::with_basics();
    let mut exported = w.project("shop", r"C:\sites\shop", false);
    w.projects.delete(&exported.id).unwrap();
    exported.environment.set_mysql(Some(lampo_domain::mysql::MysqlSeries::parse("8.0").unwrap()));
    let transfer = Arc::new(FakeTransfer::default());
    *transfer.to_import.lock().unwrap() = Some(exported);

    let err = w.import_project(transfer).execute(ImportProjectRequest { file: r"D:\shop.toml".into(), root: None }, &gui()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Validation);
    assert!(err.message.contains("8.0"), "{}", err.message);
    assert!(w.projects.list().unwrap().is_empty(), "nothing imported");
}

#[tokio::test]
async fn a_cli_create_with_a_hosts_name_and_no_elevation_saves_nothing() {
    let w = World::with_basics();
    w.fs.add(r"C:\sites\shop");
    let detect = Arc::new(DetectProject::new(Vec::new(), w.projects.clone(), w.fs.clone()));
    let create = CreateProject::new(w.project_writer(), w.presets(), detect, w.fs.clone(), w.database_service());
    let request = CreateProjectRequest { root: r"C:\sites\shop".into(), addressing: AddressingDto::Standard, ..Default::default() };

    let err = create.execute(request.clone(), &cli()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::ElevationRequired);
    assert!(err.message.contains("`shop.test`") && err.message.contains("nothing was created"), "{}", err.message);
    assert!(w.projects.list().unwrap().is_empty(), "nothing saved");
    assert!(w.db.calls().is_empty(), "no database created");
    assert!(!w.events.all().iter().any(|e| matches!(e, DomainEvent::ProjectsChanged { .. })));

    // The retry with `--allow-elevation` creates it (no `Conflict` with a half-done first attempt).
    let created = create.execute(request, &CallContext::cli(true)).await.unwrap();
    assert_eq!(created.project.id, "shop");
    assert_eq!(created.database_setup.map(|s| s.name).as_deref(), Some("shop"));
}

#[tokio::test]
async fn a_delete_waits_for_a_concurrent_edit_instead_of_being_undone_by_it() {
    let w = World::with_basics();
    w.project("shop", r"C:\sites\shop", false);
    w.hosts_in_sync();
    let sync = Arc::new(CountingSync::default());
    let writer = w.project_writer_with(sync.clone());
    let delete = w.delete_project_with(sync);
    let ctx = gui();
    let mut deletion = Box::pin(delete.execute("shop", &ctx));
    let mut finished_during_the_edit = None;

    writer
        .modify("shop", |project, _| {
            // The delete starts while this edit holds the project: it must wait for it.
            if let Poll::Ready(result) = deletion.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
                finished_during_the_edit = Some(result);
            }
            project.name = DisplayName::new("Shop (renamed)").unwrap();
            Ok(true)
        })
        .await
        .unwrap();
    assert!(finished_during_the_edit.is_none(), "the delete did not wait for the edit");
    deletion.await.unwrap();

    assert_eq!(w.projects.get(&ProjectId::new("shop").unwrap()).unwrap(), None, "the edit did not bring the deleted project back");
}

#[tokio::test]
async fn a_delete_reports_the_worktree_databases_it_leaves_in_mysql() {
    let w = World::with_basics();
    let shop = w.project("shop", r"C:\sites\shop", false);
    w.hosts_in_sync();
    let own =
        WorktreeEnvironment::derive(&shop, &Occupancy::default(), ProjectRoot::new(r"D:\wt\login").unwrap(), Some("feature/login".into()), Port::of(8100), &DatabaseMode::Fresh, 1).unwrap();
    let taken = Occupancy { worktrees: std::slice::from_ref(&own), ..Default::default() };
    let shared = WorktreeEnvironment::derive(&shop, &taken, ProjectRoot::new(r"D:\wt\cart").unwrap(), Some("feature/cart".into()), Port::of(8101), &DatabaseMode::Shared, 1).unwrap();
    w.worktrees.save(&own).unwrap();
    w.worktrees.save(&shared).unwrap();

    let result = w.delete_project_with(Arc::new(CountingSync::default())).execute("shop", &gui()).await.unwrap();
    assert_eq!(result.orphaned_databases, [own.owned_database().unwrap().to_string()], "only the environments' own databases, never dropped");
    assert!(w.worktrees.list_all().unwrap().is_empty());
    assert!(w.events.all().contains(&DomainEvent::WorktreesChanged { project: shop.id.clone() }));

    let result = w.delete_project_with(Arc::new(CountingSync::default())).execute("shop", &gui()).await.unwrap_err();
    assert_eq!(result.code, ErrorCode::NotFound);
}
