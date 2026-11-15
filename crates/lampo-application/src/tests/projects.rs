//! Project creation from presets, detection, lifecycle, relink, export/import.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use lampo_domain::detection::DetectionHint;
use lampo_domain::events::DomainEvent;
use lampo_domain::package::PackageKind;
use lampo_domain::ports::{CustomPresetRepository, ProjectRepository, WorktreeRepository};
use lampo_domain::preset::PresetId;
use lampo_domain::project::{ProjectId, ProjectRoot};
use lampo_domain::service::ServiceKind;
use lampo_domain::worktree::{DatabaseMode, WorktreeEnvironment};

use super::{cli, gui};
use crate::dto::*;
use crate::error::{AppError, ErrorCode};
use crate::testing::*;
use crate::use_cases::*;

fn hint(preset: &str, php_version: Option<&str>) -> DetectionHint {
    DetectionHint::new(preset, PresetId::new(preset).unwrap(), php_version.map(php), 90, vec!["found artisan".into()])
}

fn detect(w: &World, hint: Option<DetectionHint>) -> Arc<DetectProject> {
    Arc::new(DetectProject::new(vec![Arc::new(FakeDetector(None, true)), Arc::new(FakeDetector(hint, false))], w.projects.clone(), w.fs.clone()))
}

fn create(w: &World, hint: Option<DetectionHint>) -> CreateProject {
    CreateProject::new(w.project_writer(), w.presets(), detect(w, hint), w.fs.clone(), w.database_service())
}

fn req(root: &str) -> CreateProjectRequest {
    CreateProjectRequest { root: root.into(), ..Default::default() }
}

#[tokio::test]
async fn detection_suggests_preset_name_and_flags_registered_folders() {
    let w = World::with_basics();
    w.fs.add("C:\\sites\\My Shop");
    let d = detect(&w, Some(hint("laravel", Some("8.2"))));
    let dto = d.execute("c:/sites/My Shop/").unwrap();
    assert_eq!(dto.folder, "C:\\sites\\My Shop");
    assert_eq!((dto.preset_id.as_deref(), dto.php_version.as_deref()), (Some("laravel"), Some("8.2")));
    assert_eq!((dto.suggested_name.as_str(), dto.suggested_id.as_str()), ("My Shop", "my-shop"));
    assert_eq!(dto.already_registered, None);
    assert_eq!(d.execute("C:\\nowhere").unwrap_err().code, ErrorCode::Validation);
    assert_eq!(d.execute("relative\\path").unwrap_err().code, ErrorCode::Validation);
    w.project("shop", "C:\\sites\\shop", false);
    assert_eq!(d.execute("C:\\sites\\shop").unwrap().already_registered.as_deref(), Some("shop"));
}

#[tokio::test]
async fn detected_services_and_warnings_are_reported_and_unversioned_ones_are_added() {
    let w = World::with_basics();
    w.fs.add("C:\\sites\\sail");
    w.hosts_in_sync();
    let mut found = hint("laravel", Some("8.3"));
    found.services = vec![ServiceKind::MYSQL, ServiceKind::REDIS, ServiceKind::new("postgres").unwrap()];
    found.warnings = vec!["PostgreSQL is not supported by Lampo".into()];
    let dto = detect(&w, Some(found.clone())).execute("C:\\sites\\sail").unwrap();
    assert_eq!(dto.services, ["mysql", "redis", "postgres"]);
    assert_eq!(dto.warnings, ["PostgreSQL is not supported by Lampo"]);

    let env = create(&w, Some(found)).execute(req("C:\\sites\\sail"), &gui()).await.unwrap().project.environment;
    assert!(env.service("redis").is_some(), "redis added from the detection");
    assert!(env.service_version("mysql").is_some(), "MySQL keeps the preset's series");
    assert!(env.service("postgres").is_none(), "unknown kinds are never added");
}

#[tokio::test]
async fn create_copies_the_detected_preset_and_raises_php_to_the_detected_minimum() {
    let w = World::with_basics();
    w.fs.add("C:\\sites\\api");
    w.hosts_in_sync();
    let dto = create(&w, Some(hint("laravel", Some("8.2")))).execute(req("C:\\sites\\api"), &gui()).await.unwrap().project;
    assert_eq!(dto.id, "api");
    assert_eq!(dto.origin_preset.as_deref(), Some("laravel"));
    assert_eq!(dto.environment.public_dir, "public");
    assert_eq!(dto.environment.php_version, "8.4", "the preset's latest installed PHP already satisfies ^8.2");
    assert!(dto.environment.extensions.contains(&"pdo_mysql".to_string()));
    assert_eq!(dto.urls, ["http://api.test", "http://api.localhost"]);
    assert!(!dto.active);
    assert_eq!(dto.database_name.as_deref(), Some("api"));
    assert!(w.events.all().contains(&DomainEvent::ProjectsChanged { changed: vec![ProjectId::new("api").unwrap()] }));
    assert!(w.hosts.writes.load(Ordering::SeqCst) == 1, "GUI: the new hostname is written with elevation");

    w.fs.add("C:\\sites\\old");
    let dto = create(&w, Some(hint("legacy", Some("8.1")))).execute(req("C:\\sites\\old"), &gui()).await.unwrap().project;
    assert_eq!(dto.environment.php_version, "8.1", "the legacy preset's 7.4 is older than composer.json requires");
    assert_eq!(dto.environment.service_version("mysql"), Some("5.7"));
}

#[tokio::test]
async fn presets_are_copied_not_linked() {
    let w = World::with_basics();
    w.project("shop", "C:\\sites\\shop", false);
    w.update_project("shop", |p| p.environment.use_service(lampo_domain::service::ServiceKind::REDIS, None));
    let saved = SavePreset::new(w.projects.clone(), w.presets.clone(), lampo_domain::preset::builtin::all())
        .execute(SavePresetRequest { project_id: "shop".into(), name: "My Stack".into(), description: String::new() })
        .unwrap();
    assert_eq!(saved.id, "my-stack");

    w.fs.add("C:\\sites\\blog");
    w.hosts_in_sync();
    let request = CreateProjectRequest { preset_id: Some("my-stack".into()), php_version: Some("8.4".into()), addressing: AddressingDto::NoElevation, ..req("C:\\sites\\blog") };
    let blog = create(&w, None).execute(request, &cli()).await.unwrap().project;
    assert!(blog.environment.service("redis").is_some());
    assert_eq!(blog.environment.php_version, "8.4");

    // Editing the preset afterwards does not touch the project.
    let mut preset = w.presets.list().unwrap().remove(0);
    preset.environment.drop_service(&lampo_domain::service::ServiceKind::REDIS);
    w.presets.save(&preset).unwrap();
    assert!(w.projects.get(&ProjectId::new("blog").unwrap()).unwrap().unwrap().environment.uses(&lampo_domain::service::ServiceKind::REDIS));
}

#[tokio::test]
async fn no_elevation_addressing_allocates_a_free_loopback_port_and_never_prompts() {
    let w = World::with_basics();
    w.project("shop", "C:\\sites\\shop", false);
    let parent = w.projects.get(&ProjectId::new("shop").unwrap()).unwrap().unwrap();
    let wt = WorktreeEnvironment::derive(&parent, &[], ProjectRoot::new("D:\\wt\\a").unwrap(), None, 8100, &DatabaseMode::Shared, 1).unwrap();
    w.worktrees.save(&wt).unwrap();
    w.ports.busy(8101, "node.exe");
    w.hosts_in_sync();
    w.fs.add("C:\\sites\\api");
    let request = CreateProjectRequest { addressing: AddressingDto::NoElevation, activate: true, ..req("C:\\sites\\api") };
    let dto = create(&w, None).execute(request, &cli()).await.unwrap().project;
    assert_eq!(dto.addresses, [SiteAddressDto::Localhost { host: "api.localhost".into() }, SiteAddressDto::LoopbackPort { port: 8102 }]);
    assert_eq!(dto.origin_preset.as_deref(), Some("generic-php"), "fallback preset");
    assert!(dto.active);
    assert_eq!(w.hosts.writes.load(Ordering::SeqCst), 0);
    assert!(w.web.last().unwrap().vhosts.iter().any(|v| v.site.to_string() == "api" && v.loopback_port == Some(8102)));
}

#[tokio::test]
async fn create_rejects_bad_folders_and_conflicts() {
    let w = World::with_basics();
    w.project("shop", "C:\\sites\\shop", false);
    w.hosts_in_sync();
    assert_eq!(create(&w, None).execute(req("C:\\missing"), &gui()).await.unwrap_err().code, ErrorCode::Validation);
    assert_eq!(create(&w, None).execute(req("c:\\SITES\\shop"), &gui()).await.unwrap_err().code, ErrorCode::Conflict);
    w.fs.add("C:\\sites\\shop2");
    let dup_id = CreateProjectRequest { id: Some("shop".into()), ..req("C:\\sites\\shop2") };
    assert_eq!(create(&w, None).execute(dup_id, &gui()).await.unwrap_err().code, ErrorCode::Conflict);
    let on_worktree_port = CreateProjectRequest { addresses: Some(vec![SiteAddressDto::LoopbackPort { port: 3306 }]), ..req("C:\\sites\\shop2") };
    assert_eq!(create(&w, None).execute(on_worktree_port, &gui()).await.unwrap_err().code, ErrorCode::Conflict);
    assert_eq!(w.projects.list().unwrap().len(), 1);
}

#[tokio::test]
async fn the_cli_gets_elevation_required_when_a_new_hostname_needs_the_hosts_file() {
    let w = World::with_basics();
    w.fs.add("C:\\sites\\shop");
    let err = create(&w, None).execute(req("C:\\sites\\shop"), &cli()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::ElevationRequired);
    assert_eq!(w.projects.list().unwrap().len(), 1, "the project is saved; only the hosts file waits");
}

#[tokio::test]
async fn activate_set_php_relink_update_and_delete() {
    let w = World::with_basics();
    w.project("shop", "C:\\sites\\shop", false);
    w.hosts_in_sync();
    let sync = Arc::new(CountingSync::default());
    let queries = w.project_queries();

    let writer = w.project_writer_with(sync.clone());
    let dto = SetProjectActive::new(writer.clone()).execute(SetProjectActiveRequest { project_id: "shop".into(), active: true }, &cli()).await.unwrap();
    assert!(dto.active && dto.folder_exists);
    assert_eq!(sync.calls.load(Ordering::SeqCst), 1);

    let set_php = SetProjectPhp::new(writer.clone(), w.installed.clone());
    let err = set_php.execute(SetProjectPhpRequest { project_id: "shop".into(), php_version: "7.4".into() }, &cli()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::PackageMissing);
    let dto = set_php.execute(SetProjectPhpRequest { project_id: "shop".into(), php_version: "8.4".into() }, &cli()).await.unwrap();
    assert_eq!(dto.environment.php_version, "8.4");

    let relink = RelinkProject::new(writer.clone(), w.fs.clone());
    assert_eq!(relink.execute(RelinkProjectRequest { project_id: "shop".into(), root: "D:\\gone".into() }, &cli()).await.unwrap_err().code, ErrorCode::Validation);
    w.fs.add("D:\\moved\\shop");
    let dto = relink.execute(RelinkProjectRequest { project_id: "shop".into(), root: "D:\\moved\\shop".into() }, &cli()).await.unwrap();
    assert_eq!((dto.id.as_str(), dto.root.as_str(), dto.environment.php_version.as_str()), ("shop", "D:\\moved\\shop", "8.4"));

    let update = UpdateProject::new(writer.clone());
    let mut request = UpdateProjectRequest {
        project_id: "shop".into(),
        name: "Shop 2".into(),
        addresses: vec![SiteAddressDto::Localhost { host: "shop.localhost".into() }],
        environment: dto.environment.clone(),
        database_name: Some("shop_main".into()),
        tls: TlsModeDto::LocalCa,
    };
    assert_eq!(update.execute(request.clone(), &cli()).await.unwrap_err().code, ErrorCode::Unsupported);
    request.tls = TlsModeDto::Disabled;
    let dto = update.execute(request, &cli()).await.unwrap();
    assert_eq!((dto.name.as_str(), dto.database_name.as_deref(), dto.url.as_str()), ("Shop 2", Some("shop_main"), "http://shop.localhost"));

    let parent = w.projects.get(&ProjectId::new("shop").unwrap()).unwrap().unwrap();
    let wt = WorktreeEnvironment::derive(&parent, &[], ProjectRoot::new("D:\\wt\\a").unwrap(), None, 8100, &DatabaseMode::Shared, 1).unwrap();
    w.worktrees.save(&wt).unwrap();
    assert_eq!(queries.get("shop").unwrap().worktrees.len(), 1);
    DeleteProject::new(w.projects.clone(), w.worktrees.clone(), sync.clone(), w.events.clone()).execute("shop", &cli()).await.unwrap();
    assert!(w.projects.list().unwrap().is_empty());
    assert!(w.worktrees.list_all().unwrap().is_empty());
    assert!(w.events.all().contains(&DomainEvent::WorktreesChanged { project: ProjectId::new("shop").unwrap() }));
    assert_eq!(queries.get("shop").unwrap_err().code, ErrorCode::NotFound);
}

#[tokio::test]
async fn mutations_surface_sync_failures() {
    let w = World::with_basics();
    w.project("shop", "C:\\sites\\shop", false);
    let sync = Arc::new(CountingSync::default());
    *sync.fail_with.lock().unwrap() = Some(AppError::new(ErrorCode::PortInUse, "port 80 is already in use"));
    let err = SetProjectActive::new(w.project_writer_with(sync.clone())).execute(SetProjectActiveRequest { project_id: "shop".into(), active: true }, &cli()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::PortInUse);
    assert!(w.projects.get(&ProjectId::new("shop").unwrap()).unwrap().unwrap().active);
}

#[tokio::test]
async fn export_and_import_dedupe_ids_and_regenerate_addresses() {
    let w = World::with_basics();
    let shop = w.project("shop", "C:\\sites\\shop", true);
    w.hosts_in_sync();
    let transfer = Arc::new(FakeTransfer::default());
    ExportProject::new(w.projects.clone(), transfer.clone()).execute(ExportProjectRequest { project_id: "shop".into(), file: "D:\\shop.toml".into() }).unwrap();
    assert_eq!(transfer.exported.lock().unwrap()[0].0.as_str(), "shop");

    *transfer.to_import.lock().unwrap() = Some(shop);
    w.fs.add("D:\\copy\\shop");
    let import = w.import_project(transfer.clone());
    let same_root = import.execute(ImportProjectRequest { file: "D:\\shop.toml".into(), root: None }, &gui()).await.unwrap_err();
    assert_eq!(same_root.code, ErrorCode::Conflict);
    let dto = import.execute(ImportProjectRequest { file: "D:\\shop.toml".into(), root: Some("D:\\copy\\shop".into()) }, &gui()).await.unwrap();
    assert_eq!(dto.id, "shop-2");
    assert!(!dto.active);
    assert_eq!(dto.urls, ["http://shop-2.test", "http://shop-2.localhost"]);
    assert_eq!(dto.database_name.as_deref(), Some("shop_2"));
    assert_eq!(dto.environment.service_version("mysql"), Some("8.4"));
}

/// H1 regression: an imported project keeps its exported addresses, including a loopback port.
/// When that port belongs to a worktree environment the import must be refused, otherwise Apache
/// gets two vhosts on the same `127.0.0.1:<port>` and serves one site's code for the other.
#[tokio::test]
async fn import_and_relink_refuse_a_loopback_port_owned_by_a_worktree_environment() {
    let w = World::with_basics();
    w.project("shop", "C:\\sites\\shop", false);
    let parent = w.projects.get(&ProjectId::new("shop").unwrap()).unwrap().unwrap();
    let wt = WorktreeEnvironment::derive(&parent, &[], ProjectRoot::new("D:\\wt\\a").unwrap(), None, 8100, &DatabaseMode::Shared, 1).unwrap();
    w.worktrees.save(&wt).unwrap();
    w.hosts_in_sync();

    // An export from another machine: `api` on 127.0.0.1:8100.
    let mut exported = w.project("api", "C:\\sites\\api", false);
    w.projects.delete(&exported.id).unwrap();
    exported.set_addresses(vec![lampo_domain::address::SiteAddress::localhost("api").unwrap(), lampo_domain::address::SiteAddress::LoopbackPort(8100)]).unwrap();
    let transfer = Arc::new(FakeTransfer::default());
    *transfer.to_import.lock().unwrap() = Some(exported.clone());
    let import = w.import_project(transfer);
    let err = import.execute(ImportProjectRequest { file: "D:\\api.toml".into(), root: None }, &gui()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Conflict, "{err:?}");
    assert!(w.projects.get(&exported.id).unwrap().is_none(), "nothing is saved");

    // The same port reached through a relink of a project that already holds it (pre-fix data).
    exported.addresses.push(lampo_domain::address::SiteAddress::LoopbackPort(8100));
    exported.addresses.dedup();
    w.projects.save(&exported).unwrap();
    w.fs.add("D:\\moved\\api");
    let err = w.relink_project().execute(RelinkProjectRequest { project_id: "api".into(), root: "D:\\moved\\api".into() }, &gui()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Conflict, "{err:?}");
}

#[tokio::test]
async fn project_views_report_missing_folders_and_summaries() {
    let w = World::with_basics();
    w.project("shop", "C:\\sites\\shop", true);
    w.project("gone", "C:\\sites\\gone", false);
    w.fs.remove("C:\\sites\\gone");
    let q = w.project_queries();
    let list = q.list().unwrap();
    assert_eq!(list.iter().map(|p| (p.id.as_str(), p.folder_exists)).collect::<Vec<_>>(), [("gone", false), ("shop", true)]);
    let shop = q.get("shop").unwrap();
    assert_eq!(shop.database.as_ref().unwrap().port, 3306);
    let summaries = q.summaries().unwrap();
    assert_eq!(summaries[1].php_version, "8.3");
    assert_eq!(q.get("Bad Id").unwrap_err().code, ErrorCode::Validation);
    let _ = PackageKind::PHP;
}

/// M7: the GUI's address suggestions come from the domain rules and the shared allocator.
#[tokio::test]
async fn address_suggestions_use_the_domain_defaults_and_the_allocator() {
    let w = World::with_basics();
    w.project("shop", r"C:\sites\shop", false);
    let parent = w.projects.get(&ProjectId::new("shop").unwrap()).unwrap().unwrap();
    let wt = WorktreeEnvironment::derive(&parent, &[], ProjectRoot::new(r"D:\wt\a").unwrap(), None, 8100, &DatabaseMode::Shared, 1).unwrap();
    w.worktrees.save(&wt).unwrap();
    w.ports.busy(8101, "node.exe");
    let s = SuggestAddresses::new(w.allocator.clone(), w.settings.clone()).execute("blog").await.unwrap();
    assert_eq!(s.hostname, SiteAddressDto::Hostname { host: "blog.test".into() });
    assert_eq!(s.localhost, SiteAddressDto::Localhost { host: "blog.localhost".into() });
    assert_eq!(s.loopback_port, SiteAddressDto::LoopbackPort { port: 8102 }, "8100 is a worktree's, 8101 is busy");
    assert_eq!(s.no_elevation, [s.localhost.clone(), s.loopback_port.clone()]);
    assert_eq!(s.standard, [s.hostname.clone(), s.localhost.clone()]);
    assert_eq!(SuggestAddresses::new(w.allocator.clone(), w.settings.clone()).execute("Bad Id").await.unwrap_err().code, ErrorCode::Validation);
}

// --- the new project's database ----------------------------------------------------------------

fn no_mysql_environment() -> ProjectEnvironmentDto {
    ProjectEnvironmentDto {
        php_version: "8.4".into(),
        extensions: vec![],
        addons: vec![],
        php_ini: Default::default(),
        public_dir: String::new(),
        services: vec![],
        apache_directives: String::new(),
    }
}

#[tokio::test]
async fn create_creates_the_projects_mysql_database_on_its_instance() {
    let w = World::with_basics();
    w.fs.add(r"C:\sites\api");
    w.hosts_in_sync();
    let result = create(&w, Some(hint("laravel", None))).execute(req(r"C:\sites\api"), &gui()).await.unwrap();
    assert_eq!(result.project.database_name.as_deref(), Some("api"));
    assert_eq!(result.database_setup, Some(DatabaseSetupDto { name: "api".into(), status: DatabaseSetupStatusDto::Created, message: None }));
    assert_eq!(w.db.calls(), ["create:3306:api"]);
    assert!(w.db.has("api"));

    // A folder reusing a database that already exists: reported, not an error.
    w.db.dbs.lock().unwrap().insert("blog".into());
    w.fs.add(r"C:\sites\blog");
    w.hosts_in_sync();
    let result = create(&w, None).execute(req(r"C:\sites\blog"), &gui()).await.unwrap();
    assert_eq!(result.database_setup.map(|d| d.status), Some(DatabaseSetupStatusDto::AlreadyExists));
}

#[tokio::test]
async fn create_does_not_touch_mysql_for_a_project_without_it() {
    let w = World::with_basics();
    w.fs.add(r"C:\sites\static");
    w.hosts_in_sync();
    let request = CreateProjectRequest { environment: Some(no_mysql_environment()), ..req(r"C:\sites\static") };
    let result = create(&w, None).execute(request, &gui()).await.unwrap();
    assert_eq!(result.project.database_name, None);
    assert_eq!(result.database_setup, None);
    assert!(w.db.calls().is_empty());
}

#[tokio::test]
async fn create_succeeds_and_reports_when_the_database_cannot_be_created() {
    let w = World::with_basics();
    w.fs.add(r"C:\sites\api");
    w.hosts_in_sync();
    w.db.fail_create.store(true, Ordering::SeqCst);
    let result = create(&w, None).execute(req(r"C:\sites\api"), &gui()).await.unwrap();
    assert_eq!(result.project.id, "api");
    assert!(w.projects.get(&ProjectId::new("api").unwrap()).unwrap().is_some(), "the project is saved");
    let setup = result.database_setup.unwrap();
    assert_eq!((setup.name.as_str(), setup.status), ("api", DatabaseSetupStatusDto::Failed));
    assert!(setup.message.unwrap().contains("access denied"));
}

#[tokio::test]
async fn create_reports_a_missing_mysql_package_without_installing_it() {
    let w = World::with_basics();
    w.installed.packages.lock().unwrap().retain(|p| p.id.kind != PackageKind::MYSQL);
    w.fs.add(r"C:\sites\api");
    w.hosts_in_sync();
    let result = create(&w, None).execute(req(r"C:\sites\api"), &gui()).await.unwrap();
    let setup = result.database_setup.unwrap();
    assert_eq!(setup.status, DatabaseSetupStatusDto::PackageMissing);
    assert!(setup.message.unwrap().contains("MySQL 8.4"));
    assert!(w.db.calls().is_empty());
}

#[tokio::test]
async fn create_database_false_skips_the_database() {
    let w = World::with_basics();
    w.fs.add(r"C:\sites\api");
    w.hosts_in_sync();
    let request = CreateProjectRequest { create_database: false, ..req(r"C:\sites\api") };
    let result = create(&w, None).execute(request, &gui()).await.unwrap();
    assert_eq!(result.project.database_name.as_deref(), Some("api"), "the name is still reserved for the project");
    assert_eq!(result.database_setup, None);
    assert!(w.db.calls().is_empty());
}

#[test]
fn create_database_defaults_to_true_on_the_wire_and_in_rust() {
    assert!(CreateProjectRequest::default().create_database);
    let parsed: CreateProjectRequest = serde_json::from_str(r#"{ "root": "C:/sites/api" }"#).unwrap();
    assert!(parsed.create_database);
    let parsed: CreateProjectRequest = serde_json::from_str(r#"{ "root": "C:/sites/api", "createDatabase": false }"#).unwrap();
    assert!(!parsed.create_database);
}

#[test]
fn the_create_result_is_a_project_with_an_optional_database_setup() {
    let w = World::with_basics();
    w.project("shop", r"C:\sites\shop", false);
    let project = w.project_queries().get("shop").unwrap();
    let plain = serde_json::to_value(CreateProjectResultDto { project: project.clone(), database_setup: None }).unwrap();
    assert_eq!(plain, serde_json::to_value(&project).unwrap(), "without a setup the wire form is exactly a ProjectDto");
    let with = CreateProjectResultDto { project, database_setup: Some(DatabaseSetupDto { name: "shop".into(), status: DatabaseSetupStatusDto::PackageMissing, message: Some("x".into()) }) };
    let json = serde_json::to_value(&with).unwrap();
    assert_eq!(json["databaseSetup"]["status"], "package-missing");
    assert_eq!(json["id"], "shop");
    let back: ProjectDto = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(back.id, "shop", "older readers expecting a ProjectDto still parse it");
    assert_eq!(serde_json::from_value::<CreateProjectResultDto>(json).unwrap(), with);
}
