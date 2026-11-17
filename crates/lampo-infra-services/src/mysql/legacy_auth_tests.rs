//! PHP before 7.4 on a real MySQL 8.4 (ignored by default; needs an extracted MySQL zip, as
//! `tests/mysql_smoke.rs`, and optionally a PHP 7.x build with `mysqli` to connect through it):
//!
//! ```powershell
//! $env:LAMPO_TEST_MYSQL_DIR = "C:\path\to\mysql-8.4.x-winx64"
//! $env:LAMPO_TEST_LEGACY_PHP = "C:\path\to\php-7.2\php.exe"   # optional
//! cargo test -p lampo-infra-services -- --ignored native_password
//! ```
//!
//! With Lampo's `my.ini`, the account of a PHP 7.2 project and root are moved to
//! `mysql_native_password` (keeping their passwords and the data), idempotently, and accept a
//! client that offers only that plugin.

use std::path::PathBuf;

use lampo_domain::mysql::DatabaseName;
use lampo_domain::mysql::accounts::MysqlUserName;
use lampo_domain::mysql::auth::MysqlAuthPlugin;
use lampo_domain::mysql::credentials::MysqlPassword;
use lampo_domain::ports::{CommandOutput, DatabaseAdmin, MysqlAccountAdmin, MysqlEndpoint, MysqlRootPasswordWriter, NullEventPublisher, ProjectAccountGrant};
use lampo_domain::project::ProjectId;

use super::credential_tests::{free_port, run_as};
use super::*;
use crate::{DpapiMysqlPasswords, JobObjectSupervisor, MysqlCliAdmin};

const PROJECT_PASSWORD: &str = "Leg4cyPassw0rd";

/// `mysql.exe` offering only `mysql_native_password`, as PHP's `mysqlnd` before 7.4 does.
async fn native_only_client(commands: &dyn CommandRunner, endpoint: &MysqlEndpoint, user: &str, password: &str) -> CommandOutput {
    let option_file = ClientOptionFile::for_password(&MysqlPassword::new(password).unwrap()).unwrap();
    let mut args = client_args(endpoint.port, user, Some(&option_file));
    args.extend(["--default-auth=mysql_native_password".into(), "--execute=SELECT COUNT(*) FROM legacy.items".into()]);
    commands.run(CommandSpec { program: endpoint.client_dir.join("mysql.exe"), args, ..Default::default() }, MYSQLADMIN_TIMEOUT).await.unwrap()
}

/// `user`'s plugin on every host, as root.
async fn plugins_of(commands: &dyn CommandRunner, endpoint: &MysqlEndpoint, root_password: Option<&str>, user: &str) -> Vec<String> {
    let out = run_as(commands, endpoint, ROOT_ACCOUNT, root_password, &format!("SELECT plugin FROM mysql.user WHERE User = '{user}'")).await;
    assert!(out.success(), "{}", out.stderr);
    out.stdout.lines().skip(1).map(|l| l.trim().to_string()).collect()
}

/// A PHP 7.x `mysqli` connection with the project's account, when `LAMPO_TEST_LEGACY_PHP` names
/// a `php.exe`.
async fn legacy_php_connects(commands: &dyn CommandRunner, endpoint: &MysqlEndpoint, user: &str) {
    let Some(php) = std::env::var_os("LAMPO_TEST_LEGACY_PHP").map(PathBuf::from) else { return };
    let script = format!("$db = new mysqli('127.0.0.1', '{user}', '{PROJECT_PASSWORD}', 'legacy', {}); echo $db->query('SELECT COUNT(*) FROM items')->fetch_row()[0];", endpoint.port);
    let args = vec!["-d".into(), "extension=mysqli".into(), "-r".into(), script];
    let out = commands.run(CommandSpec { program: php, args, ..Default::default() }, MYSQLADMIN_TIMEOUT).await.unwrap();
    assert!(out.success() && out.stdout.trim() == "1", "PHP did not connect: {} {}", out.stdout, out.stderr);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a MySQL zip: set LAMPO_TEST_MYSQL_DIR"]
async fn old_php_accounts_and_root_move_to_native_password_keeping_their_passwords() {
    let Some(install) = std::env::var_os("LAMPO_TEST_MYSQL_DIR").map(PathBuf::from) else { return };
    let root = tempfile::tempdir().unwrap();
    let launch = ServiceLaunch {
        service: MysqlSeries::DEFAULT.service_id(),
        install_dir: install.clone(),
        port: free_port(),
        ui_port: None,
        config_dir: root.path().join("generated"),
        data_dir: Some(root.path().join("data")),
        log_dir: root.path().join("logs"),
    };
    let store = Arc::new(DpapiMysqlPasswords::new(root.path().join("secrets")));
    let sup = Arc::new(JobObjectSupervisor::new());
    let runner = MysqlRunner::new(sup.clone(), sup.clone(), store.clone(), Arc::new(NullEventPublisher));
    runner.prepare(&launch).await.unwrap();
    runner.start(&launch).await.unwrap();
    let endpoint = MysqlEndpoint { series: MysqlSeries::DEFAULT, client_dir: install.join("bin"), port: launch.port, user: ROOT_ACCOUNT.into() };
    let admin = MysqlCliAdmin::new(sup.clone(), sup.clone(), store.clone());

    let legacy = DatabaseName::new("legacy").unwrap();
    admin.create(&endpoint, &legacy).await.unwrap();
    let seed = root.path().join("seed.sql");
    std::fs::write(&seed, "CREATE TABLE items (id INT PRIMARY KEY);\nINSERT INTO items VALUES (1);\n").unwrap();
    admin.import(&endpoint, &legacy, &seed, Default::default(), &lampo_domain::ports::NullProgressSink, &lampo_domain::progress::CancelFlag::new()).await.unwrap();

    // Created on MySQL 8's default (a project still on PHP 8), then switched to PHP 7.2.
    let user = MysqlUserName::for_project(&ProjectId::new("legacy").unwrap());
    let grant = |auth| ProjectAccountGrant { user: user.clone(), password: MysqlPassword::new(PROJECT_PASSWORD).unwrap(), databases: [legacy.clone()].into(), auth: Some(auth) };
    admin.sync_project_accounts(&endpoint, &[grant(MysqlAuthPlugin::CachingSha2)]).await.unwrap();
    assert_eq!(plugins_of(sup.as_ref(), &endpoint, None, user.as_str()).await, ["caching_sha2_password", "caching_sha2_password"]);
    for _ in 0..2 {
        admin.sync_project_accounts(&endpoint, &[grant(MysqlAuthPlugin::Native)]).await.unwrap();
    }
    assert_eq!(plugins_of(sup.as_ref(), &endpoint, None, user.as_str()).await, ["mysql_native_password", "mysql_native_password"]);
    let out = native_only_client(sup.as_ref(), &endpoint, user.as_str(), PROJECT_PASSWORD).await;
    assert!(out.success() && out.stdout.contains('1'), "the project account: {} {}", out.stdout, out.stderr);
    legacy_php_connects(sup.as_ref(), &endpoint, user.as_str()).await;

    // Root of a project on root credentials, with the password it has.
    let root_password = "R00tPassw0rd";
    assert!(run_as(sup.as_ref(), &endpoint, ROOT_ACCOUNT, None, &format!("ALTER USER 'root'@'localhost' IDENTIFIED BY '{root_password}'")).await.success());
    store.save_root_password(&launch.service, &MysqlPassword::new(root_password).unwrap()).unwrap();
    assert!(admin.use_native_password_for_root(&endpoint).await.unwrap());
    assert!(!admin.use_native_password_for_root(&endpoint).await.unwrap(), "idempotent");
    assert!(plugins_of(sup.as_ref(), &endpoint, Some(root_password), ROOT_ACCOUNT).await.iter().all(|p| p == "mysql_native_password"));
    let out = native_only_client(sup.as_ref(), &endpoint, ROOT_ACCOUNT, root_password).await;
    assert!(out.success() && out.stdout.contains('1'), "root keeps its password and the data: {} {}", out.stdout, out.stderr);
    runner.stop(&launch.service).await.unwrap();
}
