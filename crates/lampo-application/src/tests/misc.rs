//! Presets, doctor, agent context, toolchain, logs, services, database, settings, status and the
//! control-request dispatcher.

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use lampo_domain::events::DomainEvent;
use lampo_domain::mysql::MysqlSeries;
use lampo_domain::package::PackageKind;
use lampo_domain::ports::{ControlFrameSink, ControlRequestHandler, SettingsRepository, WorktreeRepository};
use lampo_domain::preset::builtin;
use lampo_domain::progress::CancelFlag;
use lampo_domain::project::{ProjectId, ProjectRoot};
use lampo_domain::service::builtin as service_builtin;
use lampo_domain::service::{ServiceClient, ServiceDescriptor, ServiceId, ServiceKind, ServiceState};
use lampo_domain::worktree::{DatabaseMode, Occupancy, WorktreeEnvironment};
use lampo_domain::PortError;
use lampo_domain::tcp_port::Port;

use super::{cli, gui};
use crate::control::*;
use crate::dto::*;
use crate::error::ErrorCode;
use crate::mapping::settings_to_dto;
use crate::testing::*;
use crate::use_cases::*;

fn add_worktree(w: &World, root: &str, branch: &str, port: u16, mode: DatabaseMode) -> WorktreeEnvironment {
    let parent = w.projects.0.lock().unwrap().get(&ProjectId::new("shop").unwrap()).cloned().unwrap();
    let existing = w.worktrees.list_all().unwrap();
    let env = WorktreeEnvironment::derive(&parent, &Occupancy { worktrees: &existing, ..Default::default() }, ProjectRoot::new(root).unwrap(), Some(branch.into()), Port::of(port), &mode, 5)
        .unwrap();
    w.worktrees.save(&env).unwrap();
    w.fs.add(root);
    env
}

fn resolve(cwd: &str) -> ResolveRequestDto {
    ResolveRequestDto { cwd: cwd.into(), ..Default::default() }
}

// --- presets ---------------------------------------------------------------------------------

#[test]
fn presets_list_builtins_first_and_protect_them() {
    let w = World::with_basics();
    w.project("shop", "C:\\sites\\shop", false);
    let save = SavePreset::new(w.projects.clone(), w.presets.clone(), lampo_domain::preset::builtin::all());
    save.execute(SavePresetRequest { project_id: "shop".into(), name: "Zeta".into(), description: "z".into() }).unwrap();
    save.execute(SavePresetRequest { project_id: "shop".into(), name: "Alpha".into(), description: String::new() }).unwrap();
    let list = w.presets().list().unwrap();
    let ids: Vec<&str> = list.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, ["laravel", "wordpress", "symfony", "generic-php", "legacy", "alpha", "zeta"]);
    assert_eq!(list[0].environment.php_version, "8.4", "built-ins use the newest installed PHP");
    assert_eq!(w.presets().find("nope").unwrap_err().code, ErrorCode::NotFound);

    let clash = save.execute(SavePresetRequest { project_id: "shop".into(), name: "Laravel".into(), description: String::new() }).unwrap_err();
    assert_eq!(clash.code, ErrorCode::Conflict);
    let delete = DeletePreset::new(w.presets.clone(), lampo_domain::preset::builtin::all());
    assert_eq!(delete.execute("laravel").unwrap_err().code, ErrorCode::Conflict);
    assert_eq!(delete.execute("ghost").unwrap_err().code, ErrorCode::NotFound);
    delete.execute("alpha").unwrap();
    assert_eq!(w.presets().list().unwrap().len(), builtin::all().len() + 1);
}

// --- doctor ----------------------------------------------------------------------------------

#[tokio::test]
async fn doctor_reports_every_problem_category() {
    let w = World::with_basics();
    w.project("shop", "C:\\sites\\shop", true);
    w.update_project("shop", |p| p.environment.php = php("8.2"));
    w.project("gone", "C:\\sites\\gone", false);
    w.fs.remove("C:\\sites\\gone");
    add_worktree(&w, "D:\\wt\\old", "old", 8100, DatabaseMode::Shared);
    w.fs.remove("D:\\wt\\old");
    w.installed.add(PackageKind::PHP, "5.6.40", None);
    // A VS 2026 Apache build needs a 14.50+ runtime; the fake machine has 14.40.
    let mut apache = w.installed.add(PackageKind::APACHE, "2.4.71-261112", None);
    apache.vc_runtime = Some(lampo_domain::vc_runtime::VcRuntime::Vs18);
    lampo_domain::ports::InstalledPackageRegistry::record(w.installed.as_ref(), &apache).unwrap();
    w.ports.busy(80, "skype.exe");
    w.control.0.store(false, Ordering::SeqCst);
    let locator = Arc::new(FakeLocator::default());
    locator.0.lock().unwrap().insert("php.exe".into(), vec![PathBuf::from("C:\\xampp\\php\\php.exe"), w.paths.shim_dir().join("php.exe")]);
    locator.0.lock().unwrap().insert("mysql.exe".into(), vec![w.paths.shim_dir().join("mysql.exe")]);
    locator.0.lock().unwrap().insert("composer.bat".into(), vec![PathBuf::from("C:\\ProgramData\\ComposerSetup\\bin\\composer.bat")]);
    let doctor = Doctor::new(w.doctor_deps(locator));
    let report = doctor.run().await.unwrap();
    assert!(!report.ok);
    assert_eq!(report.missing_vc_runtimes.len(), 2);
    assert_eq!(report.missing_vc_runtimes[0].required_by, ["php-5.6.40"]);
    assert_eq!(report.missing_vc_runtimes[1].required_by, ["apache-2.4.71-261112"]);
    assert_eq!(report.port_conflicts.iter().map(|c| (c.port, c.wanted_by.as_str())).collect::<Vec<_>>(), [(80, "apache")]);
    assert_eq!(report.shadowed_tools.iter().map(|s| s.tool.as_str()).collect::<Vec<_>>(), ["php", "composer"]);
    assert_eq!(report.stale_worktrees, ["shop@old"]);
    assert_eq!(report.missing_packages, ["php 8.2"]);
    assert!(!report.hosts_file_in_sync);
    let status = |id: &str| report.checks.iter().find(|c| c.id == id).unwrap().status;
    assert_eq!(status("folders"), CheckStatus::Warning);
    assert_eq!(status("shims-on-path"), CheckStatus::Warning);
    assert_eq!(status("app"), CheckStatus::Warning);
    assert_eq!(status("packages"), CheckStatus::Error);

    // With the app running, ports held by its own services are not conflicts.
    w.control.0.store(true, Ordering::SeqCst);
    assert!(doctor.run().await.unwrap().port_conflicts.is_empty());
}

#[tokio::test]
async fn doctor_runs_the_standard_checks_in_order() {
    let w = World::with_basics();
    let report = Doctor::new(w.doctor_deps(Arc::new(FakeLocator::default()))).run().await.unwrap();
    let ids: Vec<&str> = report.checks.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, ["vc-runtime", "ports", "shims-on-path", "path-shadowing", "hosts-block", "packages", "folders", "stale-worktrees", "app"]);
}

/// The cheap query the GUI polls agrees with the doctor's `hosts-block` check.
#[tokio::test]
async fn hosts_sync_query_matches_the_doctor() {
    let w = World::with_basics();
    w.project("shop", "C:\\sites\\shop", true);
    let query = HostsSyncQuery::new(w.projects.clone(), w.hosts.clone());
    let doctor = Doctor::new(w.doctor_deps(Arc::new(FakeLocator::default())));

    assert!(!query.in_sync().unwrap(), "no Lampo block yet");
    assert!(!doctor.run().await.unwrap().hosts_file_in_sync);

    w.hosts_in_sync();
    assert!(query.in_sync().unwrap());
    assert!(doctor.run().await.unwrap().hosts_file_in_sync);

    w.project("blog", "C:\\sites\\blog", true);
    assert!(!query.in_sync().unwrap(), "a new project's hostname is missing");
}

#[test]
fn hosts_sync_query_fails_when_the_hosts_file_is_unreadable() {
    let w = World::with_basics();
    w.hosts_in_sync();
    w.hosts.unreadable.store(true, Ordering::SeqCst);
    let query = HostsSyncQuery::new(w.projects.clone(), w.hosts.clone());
    assert!(query.in_sync().is_err(), "unknown must not be reported as out of sync");
}

#[tokio::test]
async fn an_unreadable_hosts_file_is_a_doctor_warning() {
    let w = World::with_basics();
    w.hosts.unreadable.store(true, Ordering::SeqCst);
    let report = Doctor::new(w.doctor_deps(Arc::new(FakeLocator::default()))).run().await.unwrap();
    assert!(!report.hosts_file_in_sync);
    assert_eq!(report.checks.iter().find(|c| c.id == "hosts-block").unwrap().status, CheckStatus::Warning);
}

// --- resolution, agent context, toolchain ----------------------------------------------------

#[tokio::test]
async fn agent_context_inside_worktrees_with_and_without_environment() {
    let w = World::with_basics();
    w.project("shop", "C:\\sites\\shop", true);
    w.hosts_in_sync();
    add_worktree(&w, "D:\\wt\\fix", "fix-1", 8105, DatabaseMode::Fresh);
    w.git.linked("D:\\wt\\fix", "C:\\sites\\shop", Some("fix-1"));
    w.git.linked("D:\\wt\\other", "C:\\sites\\shop", Some("other"));
    w.env.write_generated_files().unwrap();
    let agent = AgentContextQuery::new(AgentContextDeps {
        resolver: w.resolver(),
        projects: w.projects.clone(),
        worktrees: w.worktrees.clone(),
        settings: w.settings.clone(),
        git: w.git.clone(),
        fs: w.fs.clone(),
        control: w.control.clone(),
        paths: w.paths.clone(),
        services: w.services.clone(),
    });

    let with_env = agent.execute(&resolve("D:\\wt\\fix\\src")).unwrap();
    assert_eq!(with_env.resolution.kind, ResolvedKind::WorktreeEnvironment);
    assert_eq!(with_env.url.as_deref(), Some("http://127.0.0.1:8105"));
    assert_eq!(with_env.database.as_ref().unwrap().database, "shop_wt_fix_1");
    assert!(with_env.worktree.as_ref().unwrap().environment_up);
    assert_eq!(with_env.php.binary, "C:\\lampo\\packages\\php-8.3.14\\php.exe");
    assert_eq!(with_env.php.composer_channel, "stable");
    assert_eq!(with_env.logs.len(), 4);
    assert!(with_env.hints.is_empty(), "{:?}", with_env.hints);
    assert_eq!(with_env.commands, agent_command_catalog());

    let without = agent.execute(&resolve("D:\\wt\\other\\app")).unwrap();
    assert_eq!(without.resolution.kind, ResolvedKind::WorktreeWithoutEnvironment);
    assert_eq!(without.url.as_deref(), Some("http://shop.test"), "the parent's data");
    assert_eq!(without.database.as_ref().unwrap().database, "shop");
    let wt = without.worktree.unwrap();
    assert_eq!((wt.worktree_root.as_str(), wt.environment_up, wt.branch.as_deref()), ("D:\\wt\\other", false, Some("other")));
    assert!(without.hints.iter().any(|h| h.contains("lampo worktree up")));

    let outside = agent.execute(&resolve("E:\\elsewhere")).unwrap();
    assert_eq!(outside.resolution.kind, ResolvedKind::GlobalDefault);
    assert_eq!(outside.php.version, "8.4");
    assert!(outside.project_id.is_none());
}

fn agent_command_catalog() -> Vec<CliCommandDto> {
    crate::use_cases::agent::cli_command_catalog()
}

#[tokio::test]
async fn toolchain_env_and_exec_plan() {
    let w = World::with_basics();
    w.project("shop", "C:\\sites\\shop", false);
    w.env.write_generated_files().unwrap();
    let tc = ToolchainQuery::new(w.resolver(), w.paths.clone());
    let env = tc.env(&resolve("C:\\sites\\shop\\public")).unwrap();
    assert_eq!(env.site.as_deref(), Some("shop"));
    assert_eq!(env.path_prepend[0], w.paths.shim_dir().display().to_string());
    assert_eq!(env.vars["LAMPO_PROJECT"], "shop");
    assert_eq!(env.vars["LAMPO_DB_DATABASE"], "shop");
    assert!(env.vars.keys().all(|k| k == "PHPRC" || k.starts_with("LAMPO_")));
    let plan = tc.exec_plan(&resolve("C:\\sites\\shop"), &["php".into(), "artisan".into(), "migrate".into()]).unwrap();
    assert_eq!((plan.program.as_str(), plan.args.clone()), ("php", vec!["artisan".to_string(), "migrate".to_string()]));
    assert_eq!(tc.exec_plan(&resolve("C:\\sites\\shop"), &[]).unwrap_err().code, ErrorCode::Validation);

    let explicit = ResolveRequestDto { cwd: "E:\\x".into(), explicit_site: Some("nope".into()), ..Default::default() };
    assert_eq!(tc.env(&explicit).unwrap_err().code, ErrorCode::NotFound);
    assert_eq!(env.vars["LAMPO_PROJECT_ROOT"], "C:\\sites\\shop");
    assert_eq!(env.notice, None);

    // Nothing installed: no default PHP, nothing resolves.
    let empty = World::new();
    empty.env.write_generated_files().unwrap();
    assert_eq!(empty.resolver().resolve_dto(&resolve("E:\\x")).unwrap_err().code, ErrorCode::ProjectNotResolved);
}

/// In the terminal of `shop`, `cd ..\blog`: blog's toolchain, with a note about the variable.
#[tokio::test]
async fn inherited_project_gives_way_to_the_folder_of_another_project() {
    let w = World::with_basics();
    w.project("shop", "C:\\sites\\shop", false);
    w.project("blog", "C:\\sites\\blog", false);
    w.env.write_generated_files().unwrap();
    let tc = ToolchainQuery::new(w.resolver(), w.paths.clone());
    let inherited = |cwd: &str| ResolveRequestDto { cwd: cwd.into(), inherited_site: Some("shop".into()), inherited_root: Some("C:\\sites\\shop".into()), ..Default::default() };

    let env = tc.env(&inherited("C:\\sites\\blog")).unwrap();
    assert_eq!(env.site.as_deref(), Some("blog"));
    assert_eq!(env.vars["LAMPO_PROJECT"], "blog");
    assert_eq!(env.vars["LAMPO_PROJECT_ROOT"], "C:\\sites\\blog");
    assert_eq!(env.notice.as_deref(), Some("LAMPO_PROJECT=shop ignored: this folder belongs to blog (set LAMPO_QUIET=1 to hide this note)"));

    let inside = tc.env(&inherited("C:\\sites\\shop\\app")).unwrap();
    assert_eq!((inside.site.as_deref(), inside.notice), (Some("shop"), None));
    assert_eq!(tc.env(&inherited("E:\\scratch")).unwrap().site.as_deref(), Some("shop"));
}

// --- logs ------------------------------------------------------------------------------------

#[test]
fn logs_sources_tail_and_follow() {
    let w = World::with_basics();
    w.project("shop", "C:\\sites\\shop", false);
    let reader = Arc::new(FakeLogs::default());
    let php_log = w.paths.site_log_dir(&"shop".parse().unwrap()).join("php_errors.log");
    reader.0.lock().unwrap().insert(php_log.display().to_string().to_lowercase(), vec!["a".into(), "b".into(), "c".into()]);
    let logs = LogQueries::new(reader, w.projects.clone(), w.settings.clone(), w.services.clone(), w.paths.clone());

    let site = logs.sources(Some("shop")).unwrap();
    assert_eq!(site.iter().map(|s| (s.id.as_str(), s.exists)).collect::<Vec<_>>(), [("access:shop", false), ("error:shop", false), ("php:shop", true), ("mysql-8.4-error", false)]);
    let all = logs.sources(None).unwrap();
    assert_eq!(all.first().unwrap().id, "apache");
    assert!(all.iter().any(|s| s.id == "mysql-5.7-error"));
    assert_eq!(logs.sources(Some("ghost")).unwrap_err().code, ErrorCode::NotFound);

    let tail = logs.tail(&LogTailRequest { source: "php:shop".into(), lines: 2, since_unix_ms: None }).unwrap();
    assert_eq!(tail.lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(), ["b", "c"]);
    let follow = logs.read_from("php:shop", 2).unwrap();
    assert_eq!(follow.lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(), ["b", "c"]);
    assert_eq!(logs.tail(&LogTailRequest { source: "nginx".into(), lines: 1, since_unix_ms: None }).unwrap_err().code, ErrorCode::Validation);
}

// --- services, status, settings --------------------------------------------------------------

#[tokio::test]
async fn service_queries_and_manual_control() {
    let w = World::with_basics();
    w.project("shop", "C:\\sites\\shop", true);
    w.hosts_in_sync();
    let queries = Arc::new(ServiceQueries::new(w.runners(), w.services.clone(), w.projects.clone(), w.worktrees.clone(), w.settings.clone()));
    let list = queries.list().unwrap();
    assert_eq!(list.iter().map(|s| s.service.as_str()).collect::<Vec<_>>(), ["apache", "mysql-8.4", "mysql-5.7", "redis", "mailpit"]);
    assert_eq!((list[4].port, list[4].ui_port, list[4].ui_url.as_deref()), (Some(1025), Some(8025), Some("http://127.0.0.1:8025")));
    assert_eq!((list[3].ui_port, list[3].ui_url.as_deref()), (None, None));
    let mysql = &list[1];
    assert_eq!((mysql.required, mysql.required_by.clone(), mysql.state), (true, vec!["shop".to_string()], ServiceStateDto::Stopped));
    assert!(!list[2].required);
    assert_eq!(list[3].port, Some(6379));

    let ctl = &w.service_control;
    let report = ctl.up(ServicesRequest { services: vec!["mysql-5.7".into()] }, &cli()).await.unwrap();
    assert_eq!(report.started, ["mysql-5.7"]);
    assert_eq!(ctl.up(ServicesRequest { services: vec!["nginx".into()] }, &cli()).await.unwrap_err().code, ErrorCode::Validation);
    w.ports.busy(6379, "redis-server.exe");
    assert_eq!(ctl.up(ServicesRequest { services: vec!["redis".into()] }, &cli()).await.unwrap_err().code, ErrorCode::PortInUse);

    // A full `up` reconciles with the plan: the manually started, unneeded 5.7 is stopped.
    let report = ctl.up(ServicesRequest::default(), &gui()).await.unwrap();
    assert!(report.started.contains(&"apache".to_string()));
    assert_eq!(report.stopped, ["mysql-5.7"]);
    let report = ctl.restart(ServicesRequest::default()).await.unwrap();
    assert_eq!(report.restarted, ["apache", "mysql-8.4"]);
    let report = ctl.down(ServicesRequest::default()).await.unwrap();
    assert_eq!(report.stopped, ["apache", "mysql-8.4"]);
    assert_eq!(w.mysql.state(&MysqlSeries::DEFAULT.service_id()), ServiceState::Stopped);

    let status = StatusQuery::new(w.project_queries(), queries, Arc::new(|| true)).execute().await.unwrap();
    assert!(status.app_running);
    assert_eq!(status.projects.len(), 1);
    assert_eq!(status.services.len(), 5);
}

#[tokio::test]
async fn settings_update_validates_saves_and_syncs() {
    let w = World::with_basics();
    w.hosts_in_sync();
    let service = w.settings_service();
    let mut dto = service.get().unwrap();
    dto.services[2].port = 3306;
    assert_eq!(service.update(dto.clone(), &gui()).await.unwrap_err().code, ErrorCode::Conflict);
    dto.services[2].port = 6380;
    dto.first_run_completed = false;
    let saved = service.update(dto, &gui()).await.unwrap();
    assert_eq!((saved.services[2].service.as_str(), saved.services[2].port), ("redis", 6380));
    assert!(saved.first_run_completed, "not user-editable");
    assert!(w.events.all().contains(&DomainEvent::SettingsChanged));
    assert_eq!(w.mysql.state(&MysqlSeries::DEFAULT.service_id()), ServiceState::Running, "synced");
    assert_eq!(settings_to_dto(&w.settings.load().unwrap(), &w.services).services[2].port, 6380);
}

// --- database --------------------------------------------------------------------------------

#[tokio::test]
async fn database_operations_target_the_sites_instance() {
    let w = World::with_basics();
    w.project("shop", "C:\\sites\\shop", false);
    w.project("plain", "C:\\sites\\plain", false);
    w.update_project("plain", |p| p.environment.set_mysql(None));
    w.update_project("shop", |p| p.environment.set_mysql(Some(MysqlSeries::LEGACY_5_7)));
    add_worktree(&w, "D:\\wt\\fix", "fix", 8100, DatabaseMode::Fresh);
    let db = DatabaseService::new(w.db.clone(), w.projects.clone(), w.worktrees.clone(), w.settings.clone(), w.installed.clone(), w.service_control.clone(), w.services.clone());

    assert_eq!(db.create(&DatabaseRequest { site: "shop".into(), name: None }).await.unwrap(), "shop");
    assert_eq!(w.db.calls(), ["create:3307:shop"]);
    assert_eq!(w.mysql.state(&MysqlSeries::LEGACY_5_7.service_id()), ServiceState::Running, "started on demand");
    assert_eq!(db.create(&DatabaseRequest { site: "shop@fix".into(), name: None }).await.unwrap(), "shop_wt_fix");
    let list = db.list(&DatabaseListRequest { site: "shop@fix".into() }).await.unwrap();
    assert_eq!((list.port, list.user.as_str(), list.site_database.as_deref()), (3307, "root", Some("shop_wt_fix")));
    assert_eq!(list.databases, ["shop", "shop_wt_fix"]);

    assert_eq!(db.drop_database(&DatabaseRequest { site: "shop".into(), name: None }).await.unwrap_err().code, ErrorCode::Validation);
    assert_eq!(db.drop_database(&DatabaseRequest { site: "shop".into(), name: Some("shop_wt_fix".into()) }).await.unwrap(), "shop_wt_fix");
    assert_eq!(db.list(&DatabaseListRequest { site: "plain".into() }).await.unwrap_err().code, ErrorCode::Validation);
    assert_eq!(db.list(&DatabaseListRequest { site: "ghost".into() }).await.unwrap_err().code, ErrorCode::NotFound);

    let progress = RecordingProgress::default();
    db.import(&import_request(None, false), &progress, &CancelFlag::new()).await.unwrap();
    assert!(w.db.calls().contains(&"import:shop:D:\\dump.sql".to_string()), "a database switch is refused by default");
    db.import(&import_request(Some("shop"), true), &progress, &CancelFlag::new()).await.unwrap();
    assert!(w.db.calls().contains(&"import:shop:D:\\dump.sql:allow-use".to_string()), "--allow-use reaches the adapter");
    for (compat, expected) in [(DumpCompatDto::Mariadb, "mariadb"), (DumpCompatDto::Mysql80, "mysql80")] {
        let request = DatabaseImportRequest { compat: Some(compat), ..import_request(None, false) };
        db.import(&request, &progress, &CancelFlag::new()).await.unwrap();
        assert!(w.db.calls().contains(&format!("import:shop:D:\\dump.sql:compat={expected}")), "--compat reaches the adapter");
    }
    assert!(!progress.0.lock().unwrap().is_empty());
    let cancel = CancelFlag::new();
    cancel.cancel();
    let err = db.export(&DatabaseFileRequest { site: "shop".into(), name: None, file: "D:\\out.sql".into() }, &progress, &cancel).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Cancelled);
}

/// MySQL as a registered kind whose clients connect as `user`.
struct MysqlConnectingAs(&'static str);

impl ServiceDescriptor for MysqlConnectingAs {
    fn kind(&self) -> ServiceKind {
        service_builtin::Mysql.kind()
    }
    fn display_name(&self) -> &'static str {
        service_builtin::Mysql.display_name()
    }
    fn package_kind(&self) -> PackageKind {
        service_builtin::Mysql.package_kind()
    }
    fn versioned(&self) -> bool {
        service_builtin::Mysql.versioned()
    }
    fn default_port(&self) -> Port {
        service_builtin::Mysql.default_port()
    }
    fn has_data_dir(&self) -> bool {
        service_builtin::Mysql.has_data_dir()
    }
    fn has_error_log(&self) -> bool {
        service_builtin::Mysql.has_error_log()
    }
    fn client(&self) -> Option<ServiceClient> {
        service_builtin::Mysql.client().map(|c| ServiceClient { user: Some(self.0), ..c })
    }
}

#[test]
fn the_catalog_names_each_kinds_client_user() {
    let w = World::with_basics();
    assert_eq!(w.services.client_user(&ServiceKind::MYSQL), Some("root"));
    assert_eq!(w.services.client_user(&ServiceKind::REDIS), None, "Redis has no users");
    assert_eq!(w.services.client_user(&ServiceKind::new("memcached").unwrap()), None, "unknown kind");
}

/// The user of a database endpoint is the one the MySQL descriptor names: one source of truth.
#[tokio::test]
async fn database_endpoints_connect_as_the_user_of_the_mysql_descriptor() {
    let w = World::with_basics();
    w.project("shop", "C:\\sites\\shop", false);
    let catalog = Arc::new(ServiceCatalog::new(vec![Arc::new(MysqlConnectingAs("lampo"))]));
    let db = DatabaseService::new(w.db.clone(), w.projects.clone(), w.worktrees.clone(), w.settings.clone(), w.installed.clone(), w.service_control.clone(), catalog);

    assert_eq!(db.list(&DatabaseListRequest { site: "shop".into() }).await.unwrap().user, "lampo");
}

#[tokio::test]
async fn database_operations_refuse_system_schemas_before_reaching_the_server() {
    let w = World::with_basics();
    w.project("shop", "C:\\sites\\shop", false);
    let db = DatabaseService::new(w.db.clone(), w.projects.clone(), w.worktrees.clone(), w.settings.clone(), w.installed.clone(), w.service_control.clone(), w.services.clone());
    let progress = RecordingProgress::default();

    for name in ["mysql", "SYS", "Information_Schema", "performance_schema"] {
        let request = DatabaseRequest { site: "shop".into(), name: Some(name.into()) };
        let err = db.drop_database(&request).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::Validation, "drop {name}");
        assert!(err.message.contains("system schema"), "{}", err.message);
        assert_eq!(db.create(&request).await.unwrap_err().code, ErrorCode::Validation, "create {name}");
        assert_eq!(db.import(&import_request(Some(name), false), &progress, &CancelFlag::new()).await.unwrap_err().code, ErrorCode::Validation, "import {name}");
    }
    assert!(w.db.calls().is_empty(), "nothing reached the server: {:?}", w.db.calls());
}

/// Imports `D:\dump.sql` into `name` (default: the site's database) of the site `shop`.
fn import_request(name: Option<&str>, allow_database_switch: bool) -> DatabaseImportRequest {
    DatabaseImportRequest { site: "shop".into(), name: name.map(String::from), file: "D:\\dump.sql".into(), allow_database_switch, compat: None }
}

#[tokio::test]
async fn an_interrupted_import_says_the_database_may_be_incomplete_and_how_to_restore_it() {
    let w = World::with_basics();
    w.project("shop", "C:\\sites\\shop", false);
    let db = w.database_service();
    let progress = RecordingProgress::default();

    *w.db.import_error.lock().unwrap() = Some(PortError::Cancelled.interrupted());
    let err = db.import(&import_request(None, false), &progress, &CancelFlag::new()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Cancelled, "still a cancellation");
    assert!(err.message.starts_with("operation cancelled. The import stopped part-way: the database `shop` may now be incomplete"), "{}", err.message);
    assert!(err.message.contains("`lampo db drop shop --yes`") && err.message.contains("import the dump again"), "{}", err.message);

    *w.db.import_error.lock().unwrap() = Some(PortError::process("mysql", "ERROR 1064 at line 7").interrupted());
    let err = db.import(&import_request(Some("shop_copy"), false), &progress, &CancelFlag::new()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::ProcessFailed);
    assert!(err.message.contains("ERROR 1064 at line 7") && err.message.contains("`shop_copy` may now be incomplete"), "{}", err.message);

    // Refused before reaching the server: nothing was written, no restore advice.
    *w.db.import_error.lock().unwrap() = Some(PortError::invalid_data("all.sql", "line 2 switches or creates a database"));
    let err = db.import(&import_request(None, false), &progress, &CancelFlag::new()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Validation);
    assert!(!err.message.contains("incomplete"), "{}", err.message);
}

// --- shims -----------------------------------------------------------------------------------

#[test]
fn shim_setup_installs_and_puts_the_dir_on_path_once() {
    let w = World::new();
    let user_path = Arc::new(FakeUserPath::default());
    let setup = ShimSetup::new(Arc::new(FakeShimInstaller::default()), user_path.clone(), Arc::new(FakeLocator::default()), w.paths.clone(), lampo_domain::shim::tools::all());
    let before = setup.status().unwrap();
    assert!(!before.installed && !before.on_path);
    setup.install().unwrap();
    let after = setup.install().unwrap();
    assert!(after.installed && after.on_path && after.shadowed.is_empty());
    assert_eq!(user_path.0.lock().unwrap().len(), 1);
}

#[test]
fn shim_setup_uninstall_takes_the_dir_off_path_and_deletes_it_idempotently() {
    let w = World::new();
    let other = PathBuf::from("C:\\Tools");
    let user_path = Arc::new(FakeUserPath(Mutex::new(vec![other.clone()])));
    let setup = ShimSetup::new(Arc::new(FakeShimInstaller::default()), user_path.clone(), Arc::new(FakeLocator::default()), w.paths.clone(), lampo_domain::shim::tools::all());
    setup.install().unwrap();
    let after = setup.uninstall().unwrap();
    assert!(!after.installed && !after.on_path);
    assert_eq!(*user_path.0.lock().unwrap(), vec![other], "the other entries stay");
    let again = setup.uninstall().unwrap();
    assert!(!again.installed && !again.on_path);
}

#[test]
fn shim_setup_uninstall_cleans_the_path_even_when_a_running_copy_blocks_the_delete() {
    let w = World::new();
    let installer = Arc::new(FakeShimInstaller::default());
    let user_path = Arc::new(FakeUserPath::default());
    let setup = ShimSetup::new(installer.clone(), user_path.clone(), Arc::new(FakeLocator::default()), w.paths.clone(), lampo_domain::shim::tools::all());
    setup.install().unwrap();
    installer.2.store(true, Ordering::SeqCst);
    assert!(setup.uninstall().is_err());
    assert!(user_path.0.lock().unwrap().is_empty());
}

// --- control dispatcher ----------------------------------------------------------------------

#[derive(Default)]
struct Frames(Mutex<Vec<String>>);
impl ControlFrameSink for Frames {
    fn frame(&self, json: String) {
        self.0.lock().unwrap().push(json);
    }
}

fn dispatcher(w: &World, quit: Arc<std::sync::atomic::AtomicBool>, gate: Arc<ShutdownGate>) -> ControlDispatcher {
    let presets = w.presets();
    let detect = Arc::new(DetectProject::new(vec![], w.projects.clone(), w.fs.clone()));
    let queries = w.project_queries();
    let service_queries = Arc::new(ServiceQueries::new(w.runners(), w.services.clone(), w.projects.clone(), w.worktrees.clone(), w.settings.clone()));
    let install = Arc::new(InstallPackage::new(InstallPackageDeps {
        sources: vec![FakeSource::new("embedded", vec![catalog_entry(PackageKind::REDIS, "5.0.14.1", None)])],
        downloader: Arc::new(FakeDownloader::default()),
        extractor: Arc::new(FakeExtractor::default()),
        installed: w.installed.clone(),
        slots: Arc::new(MemSlotLock::default()),
        hooks: vec![],
        vc: Arc::new(FakeVc::modern()),
        settings: w.settings_writer(),
        paths: w.paths.clone(),
        env: w.env.clone(),
        events: w.events.clone(),
    }));
    let writer = w.project_writer();
    ControlDispatcher::new(ControlServices {
        app_info: Arc::new(AppInfoQuery::new("0.1.0", w.paths.clone())),
        status: Arc::new(StatusQuery::new(queries.clone(), service_queries, Arc::new(|| true))),
        create_project: Arc::new(CreateProject::new(writer.clone(), presets, detect, w.fs.clone(), w.database_service())),
        scaffold_project: Arc::new(ScaffoldProject::new(ScaffoldProjectDeps {
            files: Arc::new(crate::testing::FakeScaffoldFiles::default()),
            runner: crate::testing::FakeStreamingRunner::new(Arc::default()),
            locator: Arc::new(crate::testing::FakeLocator::default()),
            installed: w.installed.clone(),
            php_ini: w.ini.clone(),
            paths: w.paths.clone(),
        })),
        update_project: Arc::new(UpdateProject::new(writer.clone())),
        set_project_active: Arc::new(SetProjectActive::new(writer.clone())),
        set_project_php: Arc::new(SetProjectPhp::new(writer, w.installed.clone())),
        patch_project: Arc::new(w.patch_project()),
        delete_project: Arc::new(w.delete_project_with(w.env.clone())),
        import_project: Arc::new(w.import_project(Arc::new(crate::testing::FakeTransfer::default()))),
        service_control: w.service_control.clone(),
        environment: w.env.clone(),
        install_package: install,
        uninstall_package: Arc::new(w.uninstall_package()),
        database: Arc::new(DatabaseService::new(
            w.db.clone(),
            w.projects.clone(),
            w.worktrees.clone(),
            w.settings.clone(),
            w.installed.clone(),
            w.service_control.clone(),
            w.services.clone(),
        )),
        worktree_up: Arc::new(w.worktree_up()),
        worktree_down: Arc::new(w.worktree_down()),
        prune_worktrees: Arc::new(w.prune_worktrees()),
        patch_settings: Arc::new(w.patch_settings()),
        quit: Arc::new(move || quit.store(true, Ordering::SeqCst)),
        gate,
    })
}

async fn call(d: &ControlDispatcher, request: ControlRequest, ctx: crate::context::CallContext) -> (ControlFrame, Vec<String>) {
    let frames = Frames::default();
    let json = serde_json::to_string(&ControlEnvelope { protocol: PROTOCOL_VERSION, context: ctx, request }).unwrap();
    let reply = d.handle(json, &frames, &CancelFlag::new()).await;
    (serde_json::from_str(&reply).unwrap(), frames.0.into_inner().unwrap())
}

fn ok_payload(frame: ControlFrame) -> serde_json::Value {
    match frame {
        ControlFrame::Response { reply: ControlReply::Ok { payload } } => payload,
        other => panic!("expected ok, got {other:?}"),
    }
}

fn error_code(frame: ControlFrame) -> ErrorCode {
    match frame {
        ControlFrame::Response { reply: ControlReply::Error { error } } => error.code,
        other => panic!("expected error, got {other:?}"),
    }
}

#[tokio::test]
async fn control_dispatcher_routes_requests_to_use_cases() {
    let w = World::with_basics();
    w.project("shop", "C:\\sites\\shop", false);
    w.hosts_in_sync();
    w.git.linked("D:\\wt\\x", "C:\\sites\\shop", Some("x"));
    w.fs.add("D:\\wt\\x");
    let quit = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let d = dispatcher(&w, quit.clone(), Arc::default());

    let ping = ok_payload(call(&d, ControlRequest::Ping, cli()).await.0);
    assert_eq!(ping["controlProtocolVersion"], PROTOCOL_VERSION);

    let active = ok_payload(call(&d, ControlRequest::ProjectSetActive(SetProjectActiveRequest { project_id: "shop".into(), active: true }), cli()).await.0);
    assert_eq!(active["active"], true);
    assert_eq!(w.apache.state(&ServiceId::apache()), ServiceState::Running);

    let up = ok_payload(call(&d, ControlRequest::WorktreeUp(WorktreeUpRequest { path: "D:\\wt\\x".into(), ..Default::default() }), cli()).await.0);
    assert_eq!(up["environment"]["site"], "shop@x");
    assert_eq!(up["created"], true);

    let (frame, progress) = call(&d, ControlRequest::PackageInstall(InstallPackageRequest { kind: "redis".into(), ..Default::default() }), cli()).await;
    assert_eq!(ok_payload(frame)["id"]["kind"], "redis");
    assert!(!progress.is_empty());
    assert!(progress.iter().all(|f| f.contains(r#""frame":"progress""#)));
    let removed = ok_payload(call(&d, ControlRequest::PackageUninstall(UninstallPackageRequest { kind: "redis".into(), ..Default::default() }), cli()).await.0);
    assert_eq!((removed["kind"].as_str(), removed["version"].as_str()), (Some("redis"), Some("5.0.14.1")));
    let in_use = call(&d, ControlRequest::PackageUninstall(UninstallPackageRequest { kind: "php".into(), version: Some("8.3.14".into()), ..Default::default() }), cli()).await.0;
    assert_eq!(error_code(in_use), ErrorCode::Conflict);

    let status = ok_payload(call(&d, ControlRequest::Status, cli()).await.0);
    assert_eq!(status["worktreeCount"], 1);

    let missing = call(&d, ControlRequest::ProjectDelete { project_id: "ghost".into() }, cli()).await.0;
    assert_eq!(error_code(missing), ErrorCode::NotFound);

    let patched = call(&d, ControlRequest::ProjectPatch(ProjectPatchRequest { project_id: "shop".into(), extensions_add: vec!["intl".into()], ..Default::default() }), cli()).await.0;
    assert_eq!(ok_payload(patched)["environment"]["extensions"], serde_json::json!(["intl"]));
    let no_addon = call(&d, ControlRequest::ProjectPatch(ProjectPatchRequest { project_id: "shop".into(), addons_add: vec!["xdebug".into()], ..Default::default() }), cli()).await.0;
    assert_eq!(error_code(no_addon), ErrorCode::PackageMissing);

    let ports = SettingsPatchRequest { service_ports: [("redis".to_string(), 6380)].into(), ..Default::default() };
    let patched = ok_payload(call(&d, ControlRequest::SettingsPatch(ports), cli()).await.0);
    assert_eq!(patched["changes"][0]["setting"], "redis.port");
    assert_eq!(w.settings.load().unwrap().service_port(&redis_id()), Some(Port::of(6380)));

    ok_payload(call(&d, ControlRequest::Quit, cli()).await.0);
    assert!(quit.load(Ordering::SeqCst));
}

/// While the app shuts down (quit or update) the CLI cannot start services again.
#[tokio::test]
async fn control_dispatcher_refuses_mutations_while_shutting_down() {
    let w = World::with_basics();
    w.project("shop", "C:\\sites\\shop", false);
    w.hosts_in_sync();
    let shutdown = Shutdown::new(w.runners(), w.services.clone(), w.settings.clone());
    let quit = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let d = dispatcher(&w, quit.clone(), shutdown.gate());
    shutdown.run().await.unwrap();

    let up = call(&d, ControlRequest::ProjectSetActive(SetProjectActiveRequest { project_id: "shop".into(), active: true }), cli()).await.0;
    assert_eq!(error_code(up), ErrorCode::AppUnavailable);
    assert_eq!(w.apache.state(&ServiceId::apache()), ServiceState::Stopped);
    ok_payload(call(&d, ControlRequest::Status, cli()).await.0);
    ok_payload(call(&d, ControlRequest::Quit, cli()).await.0);
    assert!(quit.load(Ordering::SeqCst));

    shutdown.cancel();
    let active = ok_payload(call(&d, ControlRequest::ProjectSetActive(SetProjectActiveRequest { project_id: "shop".into(), active: true }), cli()).await.0);
    assert_eq!(active["active"], true);
}

#[tokio::test]
async fn control_dispatcher_rejects_other_protocols_and_garbage() {
    let w = World::with_basics();
    let d = dispatcher(&w, Arc::new(std::sync::atomic::AtomicBool::new(false)), Arc::default());
    let frames = Frames::default();
    let json = serde_json::to_string(&ControlEnvelope { protocol: PROTOCOL_VERSION + 1, context: cli(), request: ControlRequest::Ping }).unwrap();
    let reply: ControlFrame = serde_json::from_str(&d.handle(json, &frames, &CancelFlag::new()).await).unwrap();
    assert_eq!(error_code(reply), ErrorCode::ProtocolMismatch);
    let reply: ControlFrame = serde_json::from_str(&d.handle("{not json".into(), &frames, &CancelFlag::new()).await).unwrap();
    assert_eq!(error_code(reply), ErrorCode::ProtocolMismatch);
}

/// L5: `ensure_app` waits on the `Clock`/`Timer` ports, not on real time.
#[tokio::test]
async fn ensure_app_times_out_on_the_clock_port() {
    let channel = Arc::new(FakeControl(std::sync::atomic::AtomicBool::new(false)));
    let launcher = Arc::new(FakeLauncher { launches: Default::default() });
    let clock = Arc::new(ManualClock::default());
    let client = ControlClient::new(channel.clone(), launcher.clone(), clock.clone(), clock.clone());
    let err = client.ensure_app(std::time::Duration::from_secs(2)).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::AppUnavailable);
    assert_eq!(launcher.launches.load(Ordering::SeqCst), 1);
    assert_eq!(clock.sleeps.load(Ordering::SeqCst), 10, "2 s in 200 ms polls, without waiting for real");

    channel.0.store(true, Ordering::SeqCst);
    client.ensure_app(std::time::Duration::from_secs(2)).await.unwrap();
    assert_eq!(launcher.launches.load(Ordering::SeqCst), 1, "already running: not launched again");
}
