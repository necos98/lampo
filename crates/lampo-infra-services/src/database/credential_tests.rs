//! The stored root password in every `MysqlCliAdmin` command: through a user-only option file,
//! never the command line, and only for the root account.

use std::path::Path;
use std::sync::Arc;

use lampo_domain::mysql::DatabaseName;
use lampo_domain::mysql::credentials::MysqlPassword;
use lampo_domain::ports::{CommandSpec, DatabaseAdmin, DumpImportOptions, MysqlEndpoint, MysqlRootPasswordReader, NoMysqlRootPasswords, NullProgressSink};
use lampo_domain::progress::CancelFlag;
use lampo_domain::service::ServiceId;
use lampo_domain::PortResult;

use super::MysqlCliAdmin;
use super::tests::{FakeRunner, endpoint};

struct StoredRootPassword;
impl MysqlRootPasswordReader for StoredRootPassword {
    fn root_password(&self, instance: &ServiceId) -> PortResult<Option<MysqlPassword>> {
        assert_eq!(instance.to_string(), "mysql-8.4", "the endpoint's instance");
        Ok(Some(MysqlPassword::new("s3cret").unwrap()))
    }
}

/// Lists, creates, imports, exports and clones: every client command Lampo runs.
async fn run_every_operation(admin: &MysqlCliAdmin, endpoint: &MysqlEndpoint, dir: &Path) {
    let (shop, copy) = (DatabaseName::new("shop").unwrap(), DatabaseName::new("shop_copy").unwrap());
    let dump = dir.join("dump.sql");
    std::fs::write(&dump, b"SELECT 1;").unwrap();
    admin.list(endpoint).await.unwrap();
    admin.create(endpoint, &shop).await.unwrap();
    admin.drop_database(endpoint, &copy).await.unwrap();
    admin.import(endpoint, &shop, &dump, DumpImportOptions::default(), &NullProgressSink, &CancelFlag::new()).await.unwrap();
    admin.export(endpoint, &shop, &dir.join("out.sql"), &NullProgressSink, &CancelFlag::new()).await.unwrap();
    admin.clone_database(endpoint, &shop, &copy, &NullProgressSink, &CancelFlag::new()).await.unwrap();
}

fn every_stage(fake: &FakeRunner) -> Vec<CommandSpec> {
    fake.calls.lock().unwrap().iter().flatten().cloned().collect()
}

#[tokio::test]
async fn a_stored_root_password_reaches_every_client_through_an_option_file_only() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Arc::new(FakeRunner::default());
    let admin = MysqlCliAdmin::new(fake.clone(), fake.clone(), Arc::new(StoredRootPassword));
    run_every_operation(&admin, &endpoint(dir.path()), dir.path()).await;

    let stages = every_stage(&fake);
    assert_eq!(stages.len(), 9, "list, create, drop, create + restore, dump, create + dump | restore");
    for spec in &stages {
        assert!(spec.args[0].starts_with("--defaults-file="), "the option file comes first: {:?}", spec.args);
        assert!(!spec.args.iter().any(|a| a == "--no-defaults" || a.starts_with("-p") || a.starts_with("--password") || a.contains("s3cret")), "{:?}", spec.args);
    }
    let option_files = fake.option_files.lock().unwrap();
    assert_eq!(option_files.len(), stages.len());
    for (path, contents) in option_files.iter() {
        assert_eq!(contents, "[client]\npassword=\"s3cret\"\n");
        assert!(!path.exists(), "deleted once the command is done: {}", path.display());
    }
    let clone = fake.calls.lock().unwrap().last().unwrap().clone();
    assert_eq!(clone[0].args[0], clone[1].args[0], "both ends of a clone share one option file");
}

#[tokio::test]
async fn without_a_stored_password_the_clients_read_no_option_file() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Arc::new(FakeRunner::default());
    let admin = MysqlCliAdmin::new(fake.clone(), fake.clone(), Arc::new(NoMysqlRootPasswords));
    run_every_operation(&admin, &endpoint(dir.path()), dir.path()).await;
    for spec in every_stage(&fake) {
        assert_eq!(spec.args[..5], ["--no-defaults", "--protocol=TCP", "--host=127.0.0.1", "--port=3306", "--user=root"], "as before stored passwords existed");
    }
    assert!(fake.option_files.lock().unwrap().is_empty());
}

#[tokio::test]
async fn the_root_password_is_never_given_to_another_account() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Arc::new(FakeRunner::default());
    let admin = MysqlCliAdmin::new(fake.clone(), fake.clone(), Arc::new(StoredRootPassword));
    let app = MysqlEndpoint { user: "app".into(), ..endpoint(dir.path()) };
    admin.list(&app).await.unwrap();
    assert_eq!(every_stage(&fake)[0].args[0], "--no-defaults");
}
