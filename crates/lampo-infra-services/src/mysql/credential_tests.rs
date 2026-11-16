//! The root password in `MysqlDriver`'s `mysqladmin shutdown` fallback, with a fake runner, and
//! on a real MySQL whose root has a password (ignored by default; needs an extracted MySQL zip,
//! as `tests/mysql_smoke.rs`):
//!
//! ```powershell
//! $env:LAMPO_TEST_MYSQL_DIR = "C:\path\to\mysql-8.4.x-winx64"
//! cargo test -p lampo-infra-services -- --ignored root_password
//! ```
//!
//! There, after `ALTER USER … IDENTIFIED BY` and saving the password, `MysqlCliAdmin`, the shims'
//! option file and the `mysqladmin shutdown` fallback authenticate; without it they are refused.

use std::path::PathBuf;
use std::time::Instant;

use lampo_domain::mysql::DatabaseName;
use lampo_domain::mysql::credentials::MysqlPassword;
use lampo_domain::ports::{CommandOutput, DatabaseAdmin, DumpImportOptions, MysqlEndpoint, MysqlRootPasswordWriter, NoMysqlRootPasswords, NullEventPublisher, NullProgressSink};
use lampo_domain::progress::CancelFlag;

use super::tests::launch;
use super::*;
use crate::{DpapiMysqlRootPasswords, JobObjectSupervisor, MysqlCliAdmin};

/// Spaces, `#`, `;`, quotes and a backslash: everything the option file must carry verbatim.
const PASSWORD: &str = r"Lampo #1; it's \ fine";
/// No process has this pid: no shutdown event, so the `mysqladmin` fallback runs.
const NO_SUCH_PID: u32 = u32::MAX - 3;
const SHUTDOWN_WAIT: Duration = Duration::from_secs(60);

/// Records each command and the option file it was given, read while it "runs".
#[derive(Default)]
struct RecordingRunner {
    calls: std::sync::Mutex<Vec<(CommandSpec, Option<String>)>>,
}

#[async_trait]
impl CommandRunner for RecordingRunner {
    async fn run(&self, spec: CommandSpec, _timeout: Duration) -> PortResult<CommandOutput> {
        let option_file = spec.args[0].strip_prefix("--defaults-file=").map(|path| {
            crate::client_option_file::tests::assert_user_only(Path::new(path));
            std::fs::read_to_string(path).unwrap()
        });
        self.calls.lock().unwrap().push((spec, option_file));
        Ok(CommandOutput { exit_code: Some(0), stdout: String::new(), stderr: String::new() })
    }
}

async fn mysqladmin_shutdown(passwords: Arc<dyn MysqlRootPasswordReader>) -> (CommandSpec, Option<String>) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("bin")).unwrap();
    std::fs::write(dir.path().join("bin").join("mysqladmin.exe"), b"").unwrap();
    let commands = Arc::new(RecordingRunner::default());
    let driver = MysqlDriver { commands: commands.clone(), passwords };
    driver.request_shutdown(&ServiceLaunch { install_dir: dir.path().to_path_buf(), ..launch(MysqlSeries::DEFAULT) }, NO_SUCH_PID).await;
    let mut calls = commands.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    calls.pop().unwrap()
}

#[tokio::test]
async fn mysqladmin_shutdown_without_a_stored_password_reads_no_option_file() {
    let (spec, option_file) = mysqladmin_shutdown(Arc::new(NoMysqlRootPasswords)).await;
    assert_eq!(spec.args, ["--no-defaults", "--protocol=TCP", "--host=127.0.0.1", "--port=3307", "--user=root", "shutdown"]);
    assert_eq!(option_file, None);
}

#[tokio::test]
async fn mysqladmin_shutdown_reads_the_stored_password_from_a_user_only_option_file() {
    let secrets = tempfile::tempdir().unwrap();
    let store = crate::DpapiMysqlRootPasswords::new(secrets.path());
    store.save_root_password(&MysqlSeries::DEFAULT.service_id(), &MysqlPassword::new("s3cret").unwrap()).unwrap();
    let (spec, option_file) = mysqladmin_shutdown(Arc::new(store)).await;
    let path = spec.args[0].strip_prefix("--defaults-file=").expect("the option file comes first").to_string();
    assert_eq!(spec.args[1..], ["--protocol=TCP", "--host=127.0.0.1", "--port=3307", "--user=root", "shutdown"]);
    assert!(!spec.args.iter().any(|a| a.contains("s3cret") || a.starts_with("-p") || a.starts_with("--password")), "{:?}", spec.args);
    assert_eq!(option_file.as_deref(), Some("[client]\npassword=\"s3cret\"\n"));
    assert!(!Path::new(&path).exists(), "deleted once mysqladmin is done");
}

fn free_port() -> Port {
    Port::new(std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap().local_addr().unwrap().port()).unwrap()
}

/// `password` as an SQL string literal.
fn sql_string(password: &str) -> String {
    format!("'{}'", password.replace('\\', "\\\\").replace('\'', "''"))
}

async fn set_root_password(commands: &dyn CommandRunner, client_dir: &Path, port: Port, password: &str) {
    let mut args = client_args(port, ROOT_ACCOUNT, None);
    args.push(format!("--execute=ALTER USER CURRENT_USER() IDENTIFIED BY {}", sql_string(password)));
    let out = commands.run(CommandSpec { program: client_dir.join("mysql.exe"), args, ..Default::default() }, MYSQLADMIN_TIMEOUT).await.unwrap();
    assert!(out.success(), "ALTER USER failed: {}", out.stderr);
}

async fn wait_until_closed(port: Port) {
    let deadline = Instant::now() + SHUTDOWN_WAIT;
    while mysql_greets(port).await {
        assert!(Instant::now() < deadline, "mysqld still answers {SHUTDOWN_WAIT:?} after mysqladmin shutdown");
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a MySQL zip: set LAMPO_TEST_MYSQL_DIR"]
async fn root_password_admin_and_shutdown_authenticate_with_the_stored_password() {
    let Some(install) = std::env::var_os("LAMPO_TEST_MYSQL_DIR").map(PathBuf::from) else { return };
    let series = std::env::var("LAMPO_TEST_MYSQL_SERIES").ok().map(|s| MysqlSeries::parse(&s).unwrap()).unwrap_or(MysqlSeries::DEFAULT);
    let root = tempfile::tempdir().unwrap();
    let launch = ServiceLaunch {
        service: series.service_id(),
        install_dir: install.clone(),
        port: free_port(),
        ui_port: None,
        config_dir: root.path().join("generated"),
        data_dir: Some(root.path().join("data")),
        log_dir: root.path().join("logs"),
    };
    let store = Arc::new(DpapiMysqlRootPasswords::new(root.path().join("secrets")));
    let sup = Arc::new(JobObjectSupervisor::new());
    let runner = MysqlRunner::new(sup.clone(), sup.clone(), store.clone(), Arc::new(NullEventPublisher));
    runner.prepare(&launch).await.unwrap();
    runner.start(&launch).await.unwrap();

    let client_dir = install.join("bin");
    set_root_password(sup.as_ref(), &client_dir, launch.port, PASSWORD).await;
    store.save_root_password(&launch.service, &MysqlPassword::new(PASSWORD).unwrap()).unwrap();

    let endpoint = MysqlEndpoint { series, client_dir, port: launch.port, user: ROOT_ACCOUNT.into() };
    let without_password = MysqlCliAdmin::new(sup.clone(), sup.clone(), Arc::new(NoMysqlRootPasswords));
    let refused = without_password.list(&endpoint).await.unwrap_err();
    assert!(refused.to_string().contains("Access denied"), "{refused}");

    let admin = MysqlCliAdmin::new(sup.clone(), sup.clone(), store.clone());
    let shop = DatabaseName::new("shop").unwrap();
    admin.create(&endpoint, &shop).await.unwrap();
    let seed = root.path().join("seed.sql");
    std::fs::write(&seed, "CREATE TABLE items (id INT PRIMARY KEY, name VARCHAR(20));\nINSERT INTO items VALUES (1, 'one');\n").unwrap();
    admin.import(&endpoint, &shop, &seed, DumpImportOptions::default(), &NullProgressSink, &CancelFlag::new()).await.unwrap();
    let dump = root.path().join("dump.sql.gz");
    admin.export(&endpoint, &shop, &dump, &NullProgressSink, &CancelFlag::new()).await.unwrap();
    let copy = DatabaseName::new("shop_copy").unwrap();
    admin.clone_database(&endpoint, &shop, &copy, &NullProgressSink, &CancelFlag::new()).await.unwrap();
    let mut names = admin.list(&endpoint).await.unwrap();
    names.sort();
    assert_eq!(names, ["shop", "shop_copy"]);

    // The shims' form: the usual option files, then this one.
    let option_file = ClientOptionFile::for_instance(store.as_ref(), &launch.service).unwrap().unwrap();
    let args = vec![option_file.defaults_extra_file_arg(), "--host=127.0.0.1".into(), format!("--port={}", launch.port), "--user=root".into(), "--execute=SELECT 1".into()];
    let out = sup.run(CommandSpec { program: endpoint.client_dir.join("mysql.exe"), args, ..Default::default() }, MYSQLADMIN_TIMEOUT).await.unwrap();
    assert!(out.success(), "the shims' option file was refused: {}", out.stderr);
    drop(option_file);

    // The fallback the shutdown event normally makes unnecessary.
    runner.driver.request_shutdown(&launch, NO_SUCH_PID).await;
    wait_until_closed(launch.port).await;
    let log = std::fs::read_to_string(error_log(&launch)).unwrap();
    assert!(log.contains("Received SHUTDOWN from user root"), "not stopped by mysqladmin:\n{log}");
    runner.stop(&launch.service).await.unwrap();
}
