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
    assert!(w.projects.list().unwrap().is_empty(), "refused before saving: a retry with --allow-elevation creates it");
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
    w.delete_project_with(sync.clone()).execute("shop", &cli()).await.unwrap();
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
    assert_eq!(dto.project.id, "shop-2");
    assert!(!dto.project.active);
    assert_eq!(dto.project.urls, ["http://shop-2.test", "http://shop-2.localhost"]);
    assert_eq!(dto.project.database_name.as_deref(), Some("shop_2"));
    assert_eq!(dto.project.environment.service_version("mysql"), Some("8.4"));
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
    w.projects.save(&exported).unwrap();
    w.fs.add("D:\\moved\\api");
    let err = w.relink_project().execute(RelinkProjectRequest { project_id: "api".into(), root: "D:\\moved\\api".into() }, &gui()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Conflict, "{err:?}");
}

/// A project file that does not load is reported with the listing, not silently dropped: the
/// GUI tells the user how many projects it could not read, and why.
#[test]
fn the_project_list_reports_unreadable_files() {
    let w = World::with_basics();
    w.project("shop", "C:\\sites\\shop", false);
    w.unreadable_project("C:\\Lampo\\projects\\blog.toml", &[8101]);
    let list = w.project_queries().list().unwrap();
    assert_eq!(list.projects.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["shop"]);
    assert_eq!(
        list.unreadable,
        [UnreadableProjectDto { file: "C:\\Lampo\\projects\\blog.toml".into(), reason: "invalid data in C:\\Lampo\\projects\\blog.toml: a project needs at least one address".into() }]
    );
}

#[tokio::test]
async fn project_views_report_missing_folders_and_summaries() {
    let w = World::with_basics();
    w.project("shop", "C:\\sites\\shop", true);
    w.project("gone", "C:\\sites\\gone", false);
    w.fs.remove("C:\\sites\\gone");
    let q = w.project_queries();
    let list = q.list().unwrap();
    assert_eq!(list.projects.iter().map(|p| (p.id.as_str(), p.folder_exists)).collect::<Vec<_>>(), [("gone", false), ("shop", true)]);
    assert!(list.unreadable.is_empty());
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

// --- partial environment edits (`lampo project set`) -------------------------------------------

fn patch(id: &str) -> ProjectPatchRequest {
    ProjectPatchRequest { project_id: id.into(), ..Default::default() }
}

fn service(kind: &str, version: Option<&str>) -> ServiceUseDto {
    ServiceUseDto { kind: kind.into(), version: version.map(Into::into) }
}

#[tokio::test]
async fn patch_changes_only_what_it_names_and_regenerates_apache_php_and_services() {
    let w = World::with_basics();
    w.project("shop", r"C:\sites\shop", true);
    w.update_project("shop", |p| {
        p.environment.extensions.insert(lampo_domain::php::ExtensionName::new("zip").unwrap());
        p.environment.php_ini = lampo_domain::php::php_ini_overrides([("display_errors", "On")]).unwrap();
    });
    w.hosts_in_sync();
    w.installed.add(PackageKind::XDEBUG, "3.3.2", Some("8.3"));
    // A GUI edit saved meanwhile: the patch must not revert it.
    w.update_project("shop", |p| p.name = lampo_domain::project::DisplayName::new("Shop (GUI)").unwrap());

    let request = ProjectPatchRequest {
        extensions_add: vec!["intl".into(), "php_gd.dll".into()],
        extensions_remove: vec!["zip".into()],
        addons_add: vec!["xdebug".into()],
        ini_set: [("memory_limit".to_string(), "512M".to_string())].into(),
        services_add: vec![service("redis", None)],
        public_dir: Some("public".into()),
        apache_directives: Some("Header set X-Lampo 1".into()),
        ..patch("shop")
    };
    let dto = w.patch_project().execute(request, &cli()).await.unwrap();
    assert_eq!(dto.name, "Shop (GUI)");
    assert_eq!(dto.environment.extensions, ["gd", "intl"]);
    assert_eq!(dto.environment.addons, ["xdebug"]);
    assert_eq!(dto.environment.php_ini.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>(), ["display_errors=On", "memory_limit=512M"]);
    assert_eq!(dto.environment.services, [service("mysql", Some("8.4")), service("redis", None)]);
    assert_eq!(dto.database_name.as_deref(), Some("shop"));
    assert_eq!(dto.environment.public_dir, "public");

    // Regenerated like after a GUI save.
    let ini = w.ini.0.lock().unwrap().iter().find(|s| s.site.to_string() == "shop").cloned().unwrap();
    assert_eq!(ini.values.iter().find(|(k, _)| k.as_str() == "memory_limit").map(|(_, v)| v.as_str()), Some("512M"));
    let loaded: Vec<String> = ini.extensions.iter().map(|e| e.name.to_string()).collect();
    assert!(loaded.contains(&"xdebug".to_string()) && loaded.contains(&"intl".to_string()) && !loaded.contains(&"zip".to_string()), "{loaded:?}");
    let vhost = w.web.last().unwrap().vhosts.into_iter().next().unwrap();
    assert_eq!(vhost.document_root, std::path::PathBuf::from(r"C:\sites\shop").join("public"));
    assert!(vhost.custom_directives.as_str().contains("X-Lampo"));
    assert_eq!(w.redis.state(&redis_id()), lampo_domain::service::ServiceState::Running);
    assert!(w.events.all().contains(&DomainEvent::ProjectsChanged { changed: vec![ProjectId::new("shop").unwrap()] }));

    // Removing, switching MySQL series, naming the database.
    let request = ProjectPatchRequest {
        addons_remove: vec!["xdebug".into()],
        ini_unset: vec!["display_errors".into()],
        services_add: vec![service("mysql", Some("5.7"))],
        services_remove: vec!["redis".into()],
        database_name: Some("shop_legacy".into()),
        ..patch("shop")
    };
    let dto = w.patch_project().execute(request, &cli()).await.unwrap();
    assert!(dto.environment.addons.is_empty());
    assert_eq!(dto.environment.php_ini.keys().collect::<Vec<_>>(), ["memory_limit"]);
    assert_eq!(dto.environment.services, [service("mysql", Some("5.7"))]);
    assert_eq!(dto.database.as_ref().map(|d| (d.database.as_str(), d.mysql_version.as_str())), Some(("shop_legacy", "5.7")));
    assert_eq!(dto.environment.extensions, ["gd", "intl"], "untouched");

    // Dropping MySQL clears the database name; adding it back without a series uses the default
    // instance and the default database name.
    let dto = w.patch_project().execute(ProjectPatchRequest { services_remove: vec!["mysql".into()], ..patch("shop") }, &cli()).await.unwrap();
    assert_eq!((dto.database_name.as_deref(), dto.environment.services.len()), (None, 0));
    let dto = w.patch_project().execute(ProjectPatchRequest { services_add: vec![service("mysql", None)], ..patch("shop") }, &cli()).await.unwrap();
    assert_eq!((dto.database_name.as_deref(), dto.environment.service_version("mysql")), (Some("shop"), Some("8.4")));
}

#[tokio::test]
async fn patch_rejects_bad_requests_without_saving_anything() {
    let w = World::with_basics();
    w.project("shop", r"C:\sites\shop", false);
    w.project("blog", r"C:\sites\blog", false);
    w.update_project("blog", |p| p.set_environment(lampo_domain::project::ProjectEnvironment::minimal(php("8.4"))).unwrap());
    w.installed.add(PackageKind::XDEBUG, "3.3.2", Some("8.4"));
    let before = w.projects.list().unwrap();
    let sync = Arc::new(CountingSync::default());
    let patcher = w.patch_project_with(sync.clone());

    // Xdebug is installed for PHP 8.4 only: the message names the install command.
    let request = ProjectPatchRequest { addons_add: vec!["xdebug".into()], extensions_add: vec!["intl".into()], ..patch("shop") };
    let err = patcher.execute(request, &cli()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::PackageMissing);
    assert!(err.message.contains("lampo package install xdebug --php 8.3"), "{}", err.message);
    let err = patcher.execute(ProjectPatchRequest { extensions_add: vec!["intll".into()], ..patch("shop") }, &cli()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Validation);
    assert!(err.message.contains("`intll`") && err.message.contains("available: gd, intl"), "{}", err.message);
    for request in [
        ProjectPatchRequest { extensions_add: vec!["intl".into()], extensions_remove: vec!["php_intl.dll".into()], ..patch("shop") },
        ProjectPatchRequest { ini_set: [("memory_limit".to_string(), "1G".to_string())].into(), ini_unset: vec!["memory_limit".into()], ..patch("shop") },
        ProjectPatchRequest { ini_set: [("extension".to_string(), "php_x.dll".to_string())].into(), ..patch("shop") },
        ProjectPatchRequest { extensions_add: vec!["bad name".into()], ..patch("shop") },
        ProjectPatchRequest { services_add: vec![service("mysql", Some("8.0"))], ..patch("shop") },
        ProjectPatchRequest { services_add: vec![service("postgres", None)], ..patch("shop") },
        ProjectPatchRequest { services_add: vec![service("redis", Some("7.2"))], ..patch("shop") },
        ProjectPatchRequest { services_add: vec![service("apache", None)], ..patch("shop") },
        ProjectPatchRequest { services_add: vec![service("redis", None)], services_remove: vec!["redis".into()], ..patch("shop") },
        ProjectPatchRequest { public_dir: Some(r"..\outside".into()), ..patch("shop") },
        ProjectPatchRequest { apache_directives: Some("<VirtualHost *:80>".into()), ..patch("shop") },
        ProjectPatchRequest { database_name: Some("bad name!".into()), ..patch("shop") },
        ProjectPatchRequest { database_name: Some("blog_db".into()), ..patch("blog") },
    ] {
        let shown = format!("{request:?}");
        assert_eq!(patcher.execute(request, &cli()).await.unwrap_err().code, ErrorCode::Validation, "{shown}");
    }
    let err = patcher.execute(ProjectPatchRequest { extensions_add: vec!["intl".into()], ..patch("ghost") }, &cli()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);
    assert_eq!(w.projects.list().unwrap(), before, "nothing saved");
    assert_eq!(sync.calls.load(Ordering::SeqCst), 0, "nothing synced");

    // The same add-on for the project on PHP 8.4 is fine; an empty patch saves nothing.
    let dto = patcher.execute(ProjectPatchRequest { addons_add: vec!["xdebug".into()], ..patch("blog") }, &cli()).await.unwrap();
    assert_eq!(dto.environment.addons, ["xdebug"]);
    let events = w.events.all().len();
    let dto = patcher.execute(patch("blog"), &cli()).await.unwrap();
    assert_eq!(dto.environment.addons, ["xdebug"]);
    assert_eq!(w.events.all().len(), events, "no save, no ProjectsChanged");
}

// --- display name, addresses and removal from the CLI ----------------------------------------

fn addresses_of(dto: &ProjectDto) -> Vec<String> {
    dto.addresses.iter().map(|a| crate::mapping::address_from_dto(a).unwrap().to_string()).collect()
}

fn addresses(id: &str, add: &[&str], remove: &[&str]) -> ProjectPatchRequest {
    ProjectPatchRequest { addresses_add: add.iter().map(|a| a.to_string()).collect(), addresses_remove: remove.iter().map(|a| a.to_string()).collect(), ..patch(id) }
}

fn one_address(w: &World, id: &str, address: &str) {
    w.update_project(id, |p| p.set_addresses(vec![lampo_domain::address::SiteAddress::parse(address).unwrap()]).unwrap());
}

#[tokio::test]
async fn patch_renames_and_appends_addresses_keeping_the_primary_one() {
    let w = World::with_basics();
    w.project("shop", r"C:\sites\shop", true);
    w.hosts_in_sync();
    // A GUI edit saved meanwhile: the rename must not revert it.
    w.update_project("shop", |p| p.environment.public_dir = lampo_domain::project::PublicDir::new("public").unwrap());

    let dto = w.patch_project().execute(ProjectPatchRequest { name: Some("  Shop Legacy ".into()), ..patch("shop") }, &cli()).await.unwrap();
    assert_eq!((dto.id.as_str(), dto.name.as_str(), dto.environment.public_dir.as_str()), ("shop", "Shop Legacy", "public"));
    assert_eq!(dto.database_name.as_deref(), Some("shop"), "the database name follows the id, not the display name");

    // `.localhost` names and loopback ports need no elevation.
    let dto = w.patch_project().execute(addresses("shop", &["api.shop.localhost", "loopback"], &[]), &cli()).await.unwrap();
    assert_eq!(addresses_of(&dto), ["shop.test", "shop.localhost", "api.shop.localhost", "127.0.0.1:8100"]);
    assert_eq!(dto.url, "http://shop.test", "the first address stays primary");
    let vhost = w.web.last().unwrap().vhosts.into_iter().find(|v| v.site.to_string() == "shop").unwrap();
    assert_eq!(vhost.loopback_port, Some(8100), "Apache regenerated");
    assert!(vhost.server_names.iter().any(|h| h.as_str() == "api.shop.localhost"));
    assert_eq!(w.hosts.writes.load(Ordering::SeqCst), 0);

    // Adding what is there (`loopback` with a port already assigned) is a no-op: no save.
    let events = w.events.all().len();
    let dto = w.patch_project().execute(addresses("shop", &["loopback", "shop.localhost"], &[]), &cli()).await.unwrap();
    assert_eq!(addresses_of(&dto).len(), 4);
    assert_eq!(w.events.all().len(), events);

    // A hosts-file name needs elevation: refused before saving, then applied with it.
    let before = w.projects.list().unwrap();
    let err = w.patch_project().execute(addresses("shop", &[], &["shop.test"]), &cli()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::ElevationRequired);
    assert!(err.message.contains("`shop.test`") && err.message.contains("--allow-elevation") && err.message.contains("nothing was changed"), "{}", err.message);
    assert_eq!(w.projects.list().unwrap(), before, "nothing saved");
    let dto = w.patch_project().execute(addresses("shop", &["shop.dev"], &["shop.test"]), &crate::context::CallContext::cli(true)).await.unwrap();
    assert_eq!(addresses_of(&dto), ["shop.localhost", "api.shop.localhost", "127.0.0.1:8100", "shop.dev"]);
    assert_eq!(dto.url, "http://shop.localhost", "removing the primary address promotes the next one");
    let names: Vec<String> = w.hosts.block.lock().unwrap().clone().unwrap().entries().map(|e| e.hostname.to_string()).collect();
    assert!(names.contains(&"shop.dev".to_string()) && !names.contains(&"shop.test".to_string()), "{names:?}");

    // Removing the dedicated port by keyword, and a name.
    let dto = w.patch_project().execute(addresses("shop", &[], &["loopback", "api.shop.localhost"]), &cli()).await.unwrap();
    assert_eq!(addresses_of(&dto), ["shop.localhost", "shop.dev"]);
}

#[tokio::test]
async fn address_patches_are_validated_against_the_project_and_other_sites() {
    let w = World::with_basics();
    w.project("shop", r"C:\sites\shop", false);
    w.project("blog", r"C:\sites\blog", false);
    one_address(&w, "shop", "shop.localhost");
    w.update_project("blog", |p| {
        let mut list = p.addresses().to_vec();
        list.push(lampo_domain::address::SiteAddress::parse("127.0.0.1:8200").unwrap());
        p.set_addresses(list).unwrap();
    });
    w.hosts_in_sync();
    let before = w.projects.list().unwrap();
    let sync = Arc::new(CountingSync::default());
    let patcher = w.patch_project_with(sync.clone());

    for (request, code) in [
        (addresses("shop", &[], &["shop.localhost"]), ErrorCode::Validation),
        (addresses("shop", &["x.localhost"], &["shop.localhost", "x.localhost"]), ErrorCode::Validation),
        (addresses("shop", &["loopback"], &["loopback"]), ErrorCode::Validation),
        (addresses("shop", &["127.0.0.1:9000", "127.0.0.1:9001"], &[]), ErrorCode::Validation),
        (addresses("shop", &["127.0.0.1:80"], &[]), ErrorCode::Validation),
        (addresses("shop", &["shop"], &[]), ErrorCode::Validation),
        (addresses("shop", &["http://shop.test"], &[]), ErrorCode::Validation),
        (addresses("shop", &["127.0.0.1:8200"], &[]), ErrorCode::Conflict),
        (addresses("shop", &["blog.test"], &[]), ErrorCode::Conflict),
        (addresses("shop", &["blog.localhost"], &[]), ErrorCode::Conflict),
        (ProjectPatchRequest { name: Some("  ".into()), ..patch("shop") }, ErrorCode::Validation),
    ] {
        let shown = format!("{request:?}");
        // With elevation granted, so the rule itself is what fails.
        assert_eq!(patcher.execute(request, &gui()).await.unwrap_err().code, code, "{shown}");
    }
    let err = patcher.execute(addresses("ghost", &["x.localhost"], &[]), &cli()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);
    assert_eq!(w.projects.list().unwrap(), before, "nothing saved");
    assert_eq!(sync.calls.load(Ordering::SeqCst), 0, "nothing synced");
}

#[tokio::test]
async fn delete_from_the_cli_needs_elevation_only_for_hosts_file_names() {
    let w = World::with_basics();
    w.project("shop", r"C:\sites\shop", true);
    w.project("api", r"C:\sites\api", true);
    one_address(&w, "api", "api.localhost");
    w.hosts_in_sync();
    let delete = w.delete_project_with(w.env.clone());

    let err = delete.execute("shop", &cli()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::ElevationRequired);
    assert!(err.message.contains("`shop.test`") && err.message.contains("nothing was removed"), "{}", err.message);
    assert_eq!(w.projects.list().unwrap().len(), 2, "still registered: a retry with --allow-elevation finds it");

    // An active project without hosts-file names goes without elevation; Apache stops serving it.
    delete.execute("api", &cli()).await.unwrap();
    assert!(w.web.last().unwrap().vhosts.iter().all(|v| v.site.to_string() != "api"));
    delete.execute("shop", &crate::context::CallContext::cli(true)).await.unwrap();
    assert!(w.projects.list().unwrap().is_empty());
    assert!(w.hosts.block.lock().unwrap().as_ref().unwrap().is_empty(), "hosts entries removed");
}
