//! The reconciliation loop: services plan, Apache staging/isolation, hosts file, shim index.

use std::path::PathBuf;
use std::sync::atomic::Ordering;

use lampo_domain::mysql::MysqlSeries;
use lampo_domain::package::PackageKind;
use lampo_domain::service::ServiceKind;
use lampo_domain::ports::{InstalledPackageRegistry, ProjectRepository, SettingsRepository, WorktreeRepository};
use lampo_domain::project::{ApacheDirectives, ProjectRoot, PublicDir};
use lampo_domain::service::{ServiceId, ServiceState};
use lampo_domain::site::SiteId;
use lampo_domain::worktree::{DatabaseMode, WorktreeEnvironment};

use super::{cli, gui};
use crate::error::ErrorCode;
use crate::testing::{FakeLocator, FakeRunner, World, mysql_id, php, redis_id};
use crate::use_cases::{Doctor, EnvironmentSync, ServiceQueries};
use crate::mapping::package_id_to_dto;

fn worktree(w: &World, parent: &str, root: &str, branch: &str, port: u16) -> WorktreeEnvironment {
    let p = w.projects.get(&lampo_domain::project::ProjectId::new(parent).unwrap()).unwrap().unwrap();
    let env = WorktreeEnvironment::derive(&p, &w.worktrees.list_for(&p.id).unwrap(), ProjectRoot::new(root).unwrap(), Some(branch.into()), port, &DatabaseMode::Shared, 1).unwrap();
    w.worktrees.save(&env).unwrap();
    w.fs.add(root);
    env
}

#[tokio::test]
async fn idle_machine_runs_only_the_always_on_mysql() {
    let w = World::with_basics();
    w.project("shop", "C:\\sites\\shop", false);
    w.hosts_in_sync();
    let report = w.env.sync(&cli()).await.unwrap();
    assert_eq!(report.started, ["mysql-8.4"]);
    assert_eq!(w.apache.calls(), Vec::<String>::new());
    assert!(w.web.last().is_none(), "nothing served, no Apache config applied");
    assert_eq!(w.mysql.calls(), ["prepare:mysql-8.4", "start:mysql-8.4"]);
}

#[tokio::test]
async fn running_services_are_the_union_of_active_projects_plus_default_mysql() {
    let w = World::with_basics();
    w.project("shop", "C:\\sites\\shop", true);
    w.update_project("shop", |p| p.environment.use_service(lampo_domain::service::ServiceKind::REDIS, None));
    w.project("legacy", "C:\\sites\\legacy", true);
    w.update_project("legacy", |p| p.environment.set_mysql(Some(MysqlSeries::LEGACY_5_7)));
    w.project("off", "C:\\sites\\off", false);
    w.hosts_in_sync();

    let report = w.env.sync(&gui()).await.unwrap();
    let mut started = report.started.clone();
    started.sort();
    assert_eq!(started, ["apache", "mysql-5.7", "mysql-8.4", "redis"]);
    assert_eq!(report.started.last().unwrap(), "apache", "Apache starts after its databases");
    let config = w.web.last().unwrap();
    assert_eq!(config.vhosts.iter().map(|v| v.site.to_string()).collect::<Vec<_>>(), ["legacy", "shop"]);
    assert_eq!(config.vhosts[1].php_cgi, PathBuf::from("C:\\lampo\\packages\\php-8.3.14\\php-cgi.exe"));

    // Deactivating `legacy` stops MySQL 5.7 (nobody needs it) and reloads Apache.
    w.update_project("legacy", |p| p.active = false);
    w.clear_calls();
    let report = w.env.sync(&gui()).await.unwrap();
    assert_eq!(report.stopped, ["mysql-5.7"]);
    assert_eq!(report.restarted, ["apache"]);
    assert_eq!(w.mysql.state(&MysqlSeries::DEFAULT.service_id()), ServiceState::Running);

    // Nothing changed: a second sync is a no-op.
    w.clear_calls();
    let report = w.env.sync(&gui()).await.unwrap();
    assert!(report.started.is_empty() && report.stopped.is_empty() && report.restarted.is_empty(), "{report:?}");
    assert!(w.all_calls().is_empty());
}

#[tokio::test]
async fn worktree_environments_are_served_while_they_exist_and_inherit_the_parent_live() {
    let w = World::with_basics();
    w.project("shop", "C:\\sites\\shop", false);
    w.update_project("shop", |p| p.environment.use_service(lampo_domain::service::ServiceKind::REDIS, None));
    w.hosts_in_sync();
    worktree(&w, "shop", "D:\\wt\\fix", "fix-1", 8100);

    let report = w.env.sync(&gui()).await.unwrap();
    assert!(report.started.contains(&"apache".to_string()));
    assert!(report.started.contains(&"redis".to_string()), "the inactive parent's needs come with its worktree");
    let vhost = w.web.last().unwrap().vhosts.into_iter().next().unwrap();
    assert_eq!(vhost.site.to_string(), "shop@fix-1");
    assert_eq!(vhost.loopback_port, Some(8100));
    assert_eq!(vhost.server_names.iter().map(|h| h.to_string()).collect::<Vec<_>>(), ["fix-1.shop.localhost"]);
    assert_eq!(vhost.document_root, PathBuf::from("D:\\wt\\fix"));
    assert_eq!(w.web.last().unwrap().listen_ports(), vec![80, 8100]);

    // The parent changes PHP and document root: the worktree follows on the next sync.
    w.update_project("shop", |p| {
        p.environment.php = php("8.4");
        p.environment.public_dir = PublicDir::new("public").unwrap();
    });
    w.clear_calls();
    let report = w.env.sync(&gui()).await.unwrap();
    assert_eq!(report.restarted, ["apache"]);
    let vhost = w.web.last().unwrap().vhosts.into_iter().next().unwrap();
    assert_eq!(vhost.php_cgi, PathBuf::from("C:\\lampo\\packages\\php-8.4.1\\php-cgi.exe"));
    assert_eq!(vhost.document_root, PathBuf::from("D:\\wt\\fix").join("public"));
    let ini = w.ini.0.lock().unwrap().iter().find(|s| s.site.to_string() == "shop@fix-1").cloned().unwrap();
    assert_eq!(ini.php, php("8.4"));

    // Removing the worktree environment stops Apache again.
    w.worktrees.delete_all_for(&lampo_domain::project::ProjectId::new("shop").unwrap()).unwrap();
    let report = w.env.sync(&gui()).await.unwrap();
    let mut stopped = report.stopped.clone();
    stopped.sort();
    assert_eq!(stopped, ["apache", "redis"]);
}

#[tokio::test]
async fn one_broken_project_never_takes_the_others_down() {
    let w = World::with_basics();
    w.project("shop", "C:\\sites\\shop", true);
    w.project("blog", "C:\\sites\\blog", true);
    w.update_project("blog", |p| p.environment.apache_directives = ApacheDirectives::new("Broken directive").unwrap());
    w.hosts_in_sync();

    let err = w.env.sync(&gui()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::ConfigRejected);
    let details = err.details.clone().unwrap();
    assert_eq!(details["excludedSites"], serde_json::json!(["blog"]));
    assert!(details["output"].as_str().unwrap().contains("Broken"));
    let applied = w.web.last().unwrap();
    assert_eq!(applied.vhosts.iter().map(|v| v.site.to_string()).collect::<Vec<_>>(), ["shop"]);
    assert_eq!(w.apache.state(&ServiceId::apache()), ServiceState::Running, "the healthy sites are served");
    assert!(details["report"]["started"].as_array().unwrap().contains(&serde_json::json!("apache")));

    // Unchanged config: not re-validated, still reported.
    let attempts = w.web.attempts.load(Ordering::SeqCst);
    assert_eq!(w.env.sync(&gui()).await.unwrap_err().code, ErrorCode::ConfigRejected);
    assert_eq!(w.web.attempts.load(Ordering::SeqCst), attempts);

    // Fixed: everything is served again.
    w.update_project("blog", |p| p.environment.apache_directives = ApacheDirectives::default());
    let report = w.env.sync(&gui()).await.unwrap();
    assert_eq!(report.restarted, ["apache"]);
    assert_eq!(w.web.last().unwrap().vhosts.len(), 2);
}

#[tokio::test]
async fn a_globally_rejected_config_keeps_apache_untouched_but_applies_the_rest() {
    let w = World::with_basics();
    w.project("shop", "C:\\sites\\shop", true);
    w.hosts_in_sync();
    w.web.reject_all.store(true, Ordering::SeqCst);
    let err = w.env.sync(&gui()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::ConfigRejected);
    assert!(w.web.last().is_none());
    assert_eq!(w.apache.calls(), Vec::<String>::new(), "Apache is neither started nor reloaded");
    assert_eq!(w.mysql.state(&MysqlSeries::DEFAULT.service_id()), ServiceState::Running);
    assert!(w.shim_index.writes.load(Ordering::SeqCst) > 0);
}

#[tokio::test]
async fn hosts_changes_need_elevation_and_are_never_triggered_implicitly() {
    let w = World::with_basics();
    w.project("shop", "C:\\sites\\shop", true);

    // CLI default: everything else applied, then `ElevationRequired`.
    let err = w.env.sync(&cli()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::ElevationRequired);
    assert_eq!(w.hosts.writes.load(Ordering::SeqCst), 0);
    assert_eq!(w.apache.state(&ServiceId::apache()), ServiceState::Running);
    assert!(err.details.unwrap()["report"]["started"].as_array().unwrap().contains(&serde_json::json!("mysql-8.4")));

    // UAC refused.
    w.hosts.deny.store(true, Ordering::SeqCst);
    assert_eq!(w.env.sync(&gui()).await.unwrap_err().code, ErrorCode::ElevationDenied);

    // Allowed: written once, then in sync.
    w.hosts.deny.store(false, Ordering::SeqCst);
    let report = w.env.sync(&gui()).await.unwrap();
    assert!(report.hosts_file_updated);
    assert_eq!(w.hosts.writes.load(Ordering::SeqCst), 1);
    let report = w.env.sync(&gui()).await.unwrap();
    assert!(!report.hosts_file_updated);

    // Toggling a project never touches the hosts file (the block lists inactive projects too).
    w.update_project("shop", |p| p.active = false);
    w.env.sync(&cli()).await.unwrap();
    assert_eq!(w.hosts.writes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn busy_ports_are_reported_without_blocking_other_services() {
    let w = World::with_basics();
    w.project("shop", "C:\\sites\\shop", true);
    w.hosts_in_sync();
    w.ports.busy(80, "skype.exe (pid 42)");
    let err = w.env.sync(&gui()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::PortInUse);
    assert_eq!(err.details.as_ref().unwrap()["owner"], "skype.exe (pid 42)");
    assert_eq!(w.apache.state(&ServiceId::apache()), ServiceState::Stopped);
    assert_eq!(w.mysql.state(&MysqlSeries::DEFAULT.service_id()), ServiceState::Running);
}

#[tokio::test]
async fn services_are_restarted_when_their_configuration_changes() {
    let w = World::with_basics();
    w.hosts_in_sync();
    w.env.sync(&gui()).await.unwrap();
    let mut s = w.settings.load().unwrap();
    s.services[0].port = 3316;
    w.settings.save(&s).unwrap();
    w.clear_calls();
    let report = w.env.sync(&gui()).await.unwrap();
    assert_eq!(report.restarted, ["mysql-8.4"]);
    assert_eq!(w.mysql.calls(), ["prepare:mysql-8.4", "restart:mysql-8.4"]);
    assert_eq!(w.mysql.launches.lock().unwrap().last().unwrap().port, 3316);
}

#[tokio::test]
async fn missing_packages_are_warnings_not_failures() {
    let w = World::new();
    w.settings.0.lock().unwrap().first_run_completed = true;
    w.project("shop", "C:\\sites\\shop", true);
    w.hosts_in_sync();
    let report = w.env.sync(&gui()).await.unwrap();
    assert!(report.started.is_empty());
    assert!(report.warnings.iter().any(|m| m.contains("PHP 8.3 is not installed")), "{:?}", report.warnings);
    assert!(report.warnings.iter().any(|m| m.contains("Apache is not installed")));
}

#[tokio::test]
async fn php_ini_covers_every_site_with_addons_and_the_default_cli_ini() {
    let w = World::with_basics();
    w.installed.add(PackageKind::XDEBUG, "3.3.2", Some("8.3"));
    w.project("shop", "C:\\sites\\shop", false);
    w.update_project("shop", |p| {
        p.environment.addons.insert(PackageKind::XDEBUG);
        p.environment.addons.insert(PackageKind::PHP_REDIS);
        p.environment.extensions.insert(lampo_domain::php::ExtensionName::new("intl").unwrap());
    });
    w.hosts_in_sync();
    let report = w.env.sync(&gui()).await.unwrap();
    assert!(report.warnings.iter().any(|m| m.contains("php-redis for PHP 8.3")), "{:?}", report.warnings);
    let specs = w.ini.0.lock().unwrap().clone();
    let shop = specs.iter().find(|s| s.site.to_string() == "shop").unwrap();
    assert_eq!(shop.output_dir, w.paths.php_ini_dir(&"shop".parse().unwrap()));
    let xdebug = shop.extensions.iter().find(|e| e.name.as_str() == "xdebug").unwrap();
    assert!(xdebug.zend);
    assert_eq!(xdebug.dll.as_ref().unwrap(), &PathBuf::from("C:\\lampo\\packages\\xdebug-3.3.2-php8.3\\php_xdebug.dll"));
    assert!(shop.extensions.iter().any(|e| e.name.as_str() == "intl" && e.dll.is_none()));
    assert_eq!(shop.values.iter().find(|(k, _)| k.as_str() == "display_errors").unwrap().1.as_str(), "On");
    let default = specs.iter().find(|s| s.output_dir == w.paths.default_php_ini_dir()).unwrap();
    assert_eq!(default.php, php("8.4"));
}

#[tokio::test]
async fn shim_index_lists_projects_worktrees_and_the_default_with_the_right_composer() {
    let w = World::with_basics();
    w.installed.add(PackageKind::PHP, "5.6.40", None);
    w.installed.add(PackageKind::COMPOSER, "2.2.25", None);
    w.project("shop", "C:\\sites\\shop", false);
    w.project("old", "C:\\sites\\old", false);
    w.update_project("old", |p| {
        p.environment.php = php("5.6");
        p.environment.set_mysql(None);
    });
    worktree(&w, "shop", "D:\\wt\\fix", "fix-1", 8100);
    w.hosts_in_sync();
    w.env.write_generated_files().unwrap();

    let index = w.shim_index.current();
    let sites: Vec<String> = index.projects.iter().map(|e| e.site.to_string()).collect();
    assert_eq!(sites, ["old", "shop", "shop@fix-1"]);
    let old = index.by_site(&"old".parse().unwrap()).unwrap();
    assert_eq!(old.target.composer_phar.as_ref().unwrap(), &PathBuf::from("C:\\lampo\\packages\\composer-2.2.25\\composer.phar"));
    let old_mysql = &old.target.services[&ServiceKind::MYSQL];
    assert_eq!((old_mysql.port, old_mysql.database.is_none()), (3306, true), "no MySQL → default instance, no database");
    let shop = index.by_site(&"shop".parse().unwrap()).unwrap();
    assert_eq!(shop.target.composer_phar.as_ref().unwrap(), &PathBuf::from("C:\\lampo\\packages\\composer-2.8.4\\composer.phar"));
    assert_eq!(shop.target.phprc_dir.as_ref().unwrap(), &w.paths.php_ini_dir(&"shop".parse().unwrap()));
    let wt = index.by_site(&"shop@fix-1".parse::<SiteId>().unwrap()).unwrap();
    assert_eq!(wt.root.as_str(), "D:\\wt\\fix");
    assert_eq!(wt.target.services[&ServiceKind::MYSQL].database.as_deref(), Some("shop"));
    assert_eq!(index.default.as_ref().unwrap().php, php("8.4"));
    assert!(index.default.as_ref().unwrap().services.contains_key(&ServiceKind::REDIS));
}

#[tokio::test]
async fn uninstalling_a_php_regenerates_the_shim_index() {
    let w = World::with_basics();
    w.installed.add(PackageKind::PHP, "7.4.33", None);
    w.env.write_generated_files().unwrap();
    let writes = w.shim_index.writes.load(Ordering::SeqCst);
    let uninstall = w.uninstall_package();
    let id = w.installed.list().unwrap().into_iter().find(|p| p.id.version.as_str() == "7.4.33").unwrap().id;
    uninstall.execute(package_id_to_dto(&id)).unwrap();
    assert_eq!(w.shim_index.writes.load(Ordering::SeqCst), writes + 1);
}

/// H2: a service kind the code has never heard of (Mailpit) works end to end once a descriptor
/// and a runner are registered: projects name it, the plan starts it on its default port with a
/// generated config dir, listings and package requirements include it.
#[tokio::test]
async fn a_new_service_kind_is_added_by_registering_a_descriptor_and_a_runner() {
    use lampo_domain::package::PackageKind;
    use lampo_domain::service::{ServiceClient, ServiceDescriptor, ServiceKind};
    use std::sync::Arc;

    struct Mailpit;
    impl ServiceDescriptor for Mailpit {
        fn kind(&self) -> ServiceKind {
            ServiceKind::new("mailpit").unwrap()
        }
        fn display_name(&self) -> &'static str {
            "Mailpit"
        }
        fn package_kind(&self) -> PackageKind {
            PackageKind::new("mailpit").unwrap()
        }
        fn versioned(&self) -> bool {
            false
        }
        fn default_port(&self) -> u16 {
            8025
        }
        fn has_data_dir(&self) -> bool {
            false
        }
        fn has_error_log(&self) -> bool {
            false
        }
        fn client(&self) -> Option<ServiceClient> {
            None
        }
    }

    let runner = FakeRunner::new(ServiceKind::new("mailpit").unwrap());
    let w = World::with_kinds(vec![Arc::new(Mailpit)], vec![runner.clone()]);
    for (kind, v) in
        [(PackageKind::APACHE, "2.4.62"), (PackageKind::MOD_FCGID, "2.3.10"), (PackageKind::PHP, "8.3.14"), (PackageKind::MYSQL, "8.4.3"), (PackageKind::new("mailpit").unwrap(), "1.21.0")]
    {
        w.installed.add(kind, v, None);
    }
    w.settings.0.lock().unwrap().first_run_completed = true;
    w.project("shop", r"C:\sites\shop", true);
    w.update_project("shop", |p| p.environment.use_service(ServiceKind::new("mailpit").unwrap(), None));
    w.hosts_in_sync();

    let report = w.env.sync(&gui()).await.unwrap();
    assert!(report.started.contains(&"mailpit".to_string()), "{report:?}");
    let launch = runner.launches.lock().unwrap().last().cloned().unwrap();
    assert_eq!((launch.port, launch.data_dir.is_none()), (8025, true));
    assert_eq!(launch.config_dir, w.paths.generated_dir().join("mailpit"));

    let listed = ServiceQueries::new(w.runners(), w.services.clone(), w.projects.clone(), w.worktrees.clone(), w.settings.clone()).list().unwrap();
    let mailpit = listed.iter().find(|s| s.service == "mailpit").unwrap();
    assert_eq!((mailpit.label.as_str(), mailpit.required_by.clone()), ("Mailpit", vec!["shop".to_string()]));
    assert!(listed.iter().any(|s| s.service == redis_id().to_string()));
    assert!(listed.iter().any(|s| s.service == mysql_id("8.4").to_string()));

    // Without the package the doctor reports it like any other.
    w.installed.remove(&w.installed.list().unwrap().into_iter().find(|p| p.id.kind.as_str() == "mailpit").unwrap().id).unwrap();
    let report = Doctor::new(w.doctor_deps(Arc::new(FakeLocator::default()))).run().await.unwrap();
    assert!(report.missing_packages.contains(&"mailpit".to_string()), "{:?}", report.missing_packages);
}
