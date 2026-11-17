//! [`MysqlAccountAdmin`] for [`MysqlCliAdmin`]: root's password and plugin, and the projects'
//! dedicated accounts. Every statement carrying a password is written to a [`SecretTempFile`]
//! (only the current user can open it, deleted after the run) that `mysql` reads on its standard
//! input: never `--execute=` (a command line other processes can read).
//!
//! SQL strings are quoted for the instance's default `sql_mode` (Lampo's `my.ini` sets none, so
//! backslash is an escape character); the values quoted are Lampo's own (generated passwords,
//! `lampo_` user names) and the host names the server lists.

use std::collections::BTreeSet;

use async_trait::async_trait;
use lampo_domain::mysql::DatabaseName;
use lampo_domain::mysql::accounts::{MysqlUserName, PROJECT_ACCOUNT_PREFIX};
use lampo_domain::mysql::auth::MysqlAuthPlugin;
use lampo_domain::mysql::credentials::{MysqlPassword, ROOT_ACCOUNT};
use lampo_domain::ports::{CommandOutput, MysqlAccountAdmin, MysqlEndpoint, PipedCommand, ProjectAccountGrant, RootPasswordChange};
use lampo_domain::progress::CancelFlag;
use lampo_domain::{PortError, PortResult};

use super::connection::Clients;
use super::{MysqlCliAdmin, STATEMENT_TIMEOUT, ensure_ok, failure};
use crate::client_option_file::SecretTempFile;

/// The client's "Access denied for user" error.
const ACCESS_DENIED: &str = "ERROR 1045";
/// Clients reach the instance over TCP on 127.0.0.1, which the server may name `localhost` (as
/// for the root `--initialize-insecure` creates) or `127.0.0.1`: a project account exists for
/// both, with the same password and grants, so either match authenticates it.
const ACCOUNT_HOSTS: [&str; 2] = ["localhost", "127.0.0.1"];
const SCRIPT_SUFFIX: &str = ".sql";

/// `'text'` with `\` and `'` escaped.
fn sql_string(text: &str) -> String {
    format!("'{}'", text.replace('\\', "\\\\").replace('\'', "\\'"))
}

/// `'user'@'host'`.
fn account(user: &str, host: &str) -> String {
    format!("{}@{}", sql_string(user), sql_string(host))
}

/// The database of a `GRANT … ON <db>.*`, where `_` and `%` are wildcards: escaped, so the grant
/// names exactly this database (`shop_wt` must not also grant `shopXwt`).
fn grant_database(name: &DatabaseName) -> String {
    format!("`{}`", name.as_str().replace('`', "``").replace('_', "\\_").replace('%', "\\%"))
}

/// The `LIKE` pattern of the user names starting with `prefix`.
fn like_prefix(prefix: &str) -> String {
    format!("{}%", prefix.replace('\\', "\\\\").replace('_', "\\_").replace('%', "\\%"))
}

fn denied(out: &CommandOutput) -> bool {
    !out.success() && out.stderr.contains(ACCESS_DENIED)
}

/// `IDENTIFIED [WITH <plugin>] BY '<password>'`: without a plugin, `CREATE USER` takes the
/// server default and `ALTER USER` keeps the account's.
fn identified(auth: Option<MysqlAuthPlugin>, password: &str) -> String {
    let password = sql_string(password);
    match auth {
        Some(plugin) => format!("IDENTIFIED WITH {} BY {password}", plugin.sql_name()),
        None => format!("IDENTIFIED BY {password}"),
    }
}

/// `CREATE USER IF NOT EXISTS` + `ALTER USER` (an existing account gets the password and the
/// plugin too), then its privileges replaced by every privilege on `databases`.
fn project_account_statements(grant: &ProjectAccountGrant) -> Vec<String> {
    let identified = identified(grant.auth, grant.password.expose());
    ACCOUNT_HOSTS
        .iter()
        .flat_map(|host| {
            let who = account(grant.user.as_str(), host);
            let setup = [format!("CREATE USER IF NOT EXISTS {who} {identified}"), format!("ALTER USER {who} {identified}"), format!("REVOKE ALL PRIVILEGES, GRANT OPTION FROM {who}")];
            let grants = grant.databases.iter().map(move |db| format!("GRANT ALL PRIVILEGES ON {}.* TO {who}", grant_database(db)));
            setup.into_iter().chain(grants).collect::<Vec<_>>()
        })
        .collect()
}

/// The hosts of the `host<TAB>plugin` lines (`--batch --skip-column-names`) whose plugin is not
/// `plugin`.
fn hosts_not_on(listed: &str, plugin: MysqlAuthPlugin) -> Vec<&str> {
    listed.lines().filter_map(|line| line.split_once('\t')).filter(|(_, current)| current.trim() != plugin.sql_name()).map(|(host, _)| host).collect()
}

/// `user<TAB>host` lines (`--batch --skip-column-names`) of the project accounts not in `wanted`.
fn stale_accounts<'o>(listed: &'o str, wanted: &BTreeSet<&str>) -> Vec<(&'o str, &'o str)> {
    listed.lines().filter_map(|line| line.split_once('\t')).filter(|(user, _)| MysqlUserName::is_project_account(user) && !wanted.contains(user)).collect()
}

fn script(statements: &[String]) -> String {
    statements.iter().map(|s| format!("{s};\n")).collect()
}

impl MysqlCliAdmin {
    /// Runs `statements` through `mysql`'s standard input, from a user-only temporary file.
    async fn run_secret_statements(&self, clients: &Clients<'_>, statements: &[String]) -> PortResult<()> {
        let file = SecretTempFile::write_in(&std::env::temp_dir(), SCRIPT_SUFFIX, script(statements).as_bytes())?;
        let command = PipedCommand { stages: vec![clients.mysql_spec(&["--batch".into()])?], stdin_file: Some(file.path().to_path_buf()), ..Default::default() };
        let outputs = self.pipes.run_piped(command, &|_| {}, &CancelFlag::new(), STATEMENT_TIMEOUT).await?;
        ensure_ok("mysql", &outputs)
    }
}

#[async_trait]
impl MysqlAccountAdmin for MysqlCliAdmin {
    async fn secure_root(&self, endpoint: &MysqlEndpoint, password: &MysqlPassword) -> PortResult<RootPasswordChange> {
        let root = MysqlEndpoint { user: ROOT_ACCOUNT.into(), ..endpoint.clone() };
        let probe = self.execute(&Clients::with_password(&root, Some(password))?, "SELECT 1").await?;
        if probe.success() {
            return Ok(RootPasswordChange::AlreadySet);
        }
        if !denied(&probe) {
            return Err(failure("mysql", &probe));
        }
        let open = Clients::with_password(&root, None)?;
        let hosts = self.execute(&open, &format!("SELECT Host FROM mysql.user WHERE User = {}", sql_string(ROOT_ACCOUNT))).await?;
        if denied(&hosts) {
            return Err(PortError::process(
                "mysql",
                format!("root of MySQL {} accepts neither the stored password nor an empty one: its password was changed outside Lampo", endpoint.series),
            ));
        }
        if !hosts.success() {
            return Err(failure("mysql", &hosts));
        }
        let statements: Vec<String> = hosts
            .stdout
            .lines()
            .map(str::trim)
            .filter(|h| !h.is_empty())
            .map(|host| format!("ALTER USER {} IDENTIFIED BY {}", account(ROOT_ACCOUNT, host), sql_string(password.expose())))
            .collect();
        if statements.is_empty() {
            return Err(PortError::invalid_data(format!("MySQL {}", endpoint.series), "it has no root account"));
        }
        self.run_secret_statements(&open, &statements).await?;
        Ok(RootPasswordChange::Applied)
    }

    async fn sync_project_accounts(&self, endpoint: &MysqlEndpoint, accounts: &[ProjectAccountGrant]) -> PortResult<()> {
        let clients = self.clients(endpoint)?;
        let query = format!("SELECT User, Host FROM mysql.user WHERE User LIKE {}", sql_string(&like_prefix(PROJECT_ACCOUNT_PREFIX)));
        let listed = self.execute_ok(&clients, &query).await?;
        let wanted: BTreeSet<&str> = accounts.iter().map(|a| a.user.as_str()).collect();
        let drops = stale_accounts(&listed, &wanted).into_iter().map(|(user, host)| format!("DROP USER IF EXISTS {}", account(user, host)));
        let statements: Vec<String> = drops.chain(accounts.iter().flat_map(project_account_statements)).collect();
        if statements.is_empty() {
            return Ok(());
        }
        self.run_secret_statements(&clients, &statements).await
    }

    async fn use_native_password_for_root(&self, endpoint: &MysqlEndpoint) -> PortResult<bool> {
        let root = MysqlEndpoint { user: ROOT_ACCOUNT.into(), ..endpoint.clone() };
        let clients = self.clients(&root)?;
        let listed = self.execute_ok(&clients, &format!("SELECT Host, plugin FROM mysql.user WHERE User = {}", sql_string(ROOT_ACCOUNT))).await?;
        let hosts = hosts_not_on(&listed, MysqlAuthPlugin::Native);
        if hosts.is_empty() {
            return Ok(false);
        }
        // `IDENTIFIED WITH` without `BY` would clear the password: root keeps the one it has.
        let password = self.passwords.root_password(&endpoint.series.service_id())?;
        let identified = identified(Some(MysqlAuthPlugin::Native), password.as_ref().map_or("", MysqlPassword::expose));
        let statements: Vec<String> = hosts.into_iter().map(|host| format!("ALTER USER {} {identified}", account(ROOT_ACCOUNT, host))).collect();
        self.run_secret_statements(&clients, &statements).await?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use lampo_domain::ports::{CommandOutput, NoMysqlRootPasswords};
    use lampo_domain::project::ProjectId;

    use super::super::credential_tests::StoredRootPassword;
    use super::super::tests::{FakeRunner, endpoint};
    use super::*;

    fn out(code: i32, stdout: &str, stderr: &str) -> CommandOutput {
        CommandOutput { exit_code: Some(code), stdout: stdout.into(), stderr: stderr.into() }
    }

    const DENIED: &str = "ERROR 1045 (28000): Access denied for user 'root'@'localhost' (using password: YES)";

    fn password() -> MysqlPassword {
        MysqlPassword::new("Gen3ratedPassw0rd").unwrap()
    }

    fn grant(project: &str, databases: &[&str]) -> ProjectAccountGrant {
        ProjectAccountGrant {
            user: MysqlUserName::for_project(&ProjectId::new(project).unwrap()),
            password: MysqlPassword::new(format!("{project}-pass")).unwrap(),
            databases: databases.iter().map(|d| DatabaseName::new(d).unwrap()).collect(),
            auth: None,
        }
    }

    /// Every argument of every command run: the passwords must never be among them.
    fn assert_no_password_on_any_command_line(fake: &FakeRunner, secrets: &[&str]) {
        for spec in fake.calls.lock().unwrap().iter().flatten() {
            assert!(!spec.args.iter().any(|a| secrets.iter().any(|s| a.contains(s))), "{:?}", spec.args);
        }
    }

    #[tokio::test]
    async fn root_that_already_accepts_the_password_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let fake = Arc::new(FakeRunner::default());
        let admin = MysqlCliAdmin::new(fake.clone(), fake.clone(), Arc::new(NoMysqlRootPasswords));
        assert_eq!(admin.secure_root(&endpoint(dir.path()), &password()).await.unwrap(), RootPasswordChange::AlreadySet);
        let calls = fake.calls.lock().unwrap();
        assert_eq!(calls.len(), 1, "only the probe");
        assert!(calls[0][0].args[0].starts_with("--defaults-file="), "the probe authenticates with the password, from a file");
        assert_eq!(fake.option_files.lock().unwrap()[0].1, "[client]\npassword=\"Gen3ratedPassw0rd\"\n");
    }

    #[tokio::test]
    async fn root_without_a_password_gets_it_on_every_root_account_through_standard_input() {
        let dir = tempfile::tempdir().unwrap();
        let fake = Arc::new(FakeRunner::default());
        // Replies are popped from the end: probe denied, hosts listed, the script runs.
        fake.reply.lock().unwrap().extend([out(0, "", ""), out(0, "localhost\n127.0.0.1\n", ""), out(1, "", DENIED)]);
        let admin = MysqlCliAdmin::new(fake.clone(), fake.clone(), Arc::new(NoMysqlRootPasswords));
        assert_eq!(admin.secure_root(&endpoint(dir.path()), &password()).await.unwrap(), RootPasswordChange::Applied);

        let calls = fake.calls.lock().unwrap();
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[1][0].args[0], "--no-defaults", "root is reached with no password");
        assert_eq!(calls[2][0].args[0], "--no-defaults");
        drop(calls);
        let stdins = fake.stdins.lock().unwrap();
        assert_eq!(stdins[0].1, "ALTER USER 'root'@'localhost' IDENTIFIED BY 'Gen3ratedPassw0rd';\nALTER USER 'root'@'127.0.0.1' IDENTIFIED BY 'Gen3ratedPassw0rd';\n");
        assert!(!stdins[0].0.exists(), "the script is deleted after the run");
        assert_no_password_on_any_command_line(&fake, &["Gen3ratedPassw0rd"]);
    }

    #[tokio::test]
    async fn root_accepting_neither_password_is_an_error_and_nothing_changes() {
        let dir = tempfile::tempdir().unwrap();
        let fake = Arc::new(FakeRunner::default());
        fake.reply.lock().unwrap().extend([out(1, "", DENIED), out(1, "", DENIED)]);
        let admin = MysqlCliAdmin::new(fake.clone(), fake.clone(), Arc::new(NoMysqlRootPasswords));
        let err = admin.secure_root(&endpoint(dir.path()), &password()).await.unwrap_err();
        assert!(err.to_string().contains("accepts neither the stored password nor an empty one"), "{err}");
        assert!(fake.stdins.lock().unwrap().is_empty(), "no ALTER USER ran");
    }

    #[tokio::test]
    async fn a_server_that_does_not_answer_is_not_mistaken_for_a_wrong_password() {
        let dir = tempfile::tempdir().unwrap();
        let fake = Arc::new(FakeRunner::default());
        fake.reply.lock().unwrap().push(out(1, "", "ERROR 2003 (HY000): Can't connect to MySQL server on '127.0.0.1:3306' (10061)"));
        let admin = MysqlCliAdmin::new(fake.clone(), fake.clone(), Arc::new(NoMysqlRootPasswords));
        let err = admin.secure_root(&endpoint(dir.path()), &password()).await.unwrap_err();
        assert!(err.to_string().contains("ERROR 2003"), "{err}");
        assert_eq!(fake.calls.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn project_accounts_get_exactly_their_databases_and_stale_ones_are_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let fake = Arc::new(FakeRunner::default());
        let listed = "lampo_shop\tlocalhost\nlampo_shop\t127.0.0.1\nlampo_gone\tlocalhost\nlampoX\t%\n";
        fake.reply.lock().unwrap().extend([out(0, "", ""), out(0, listed, "")]);
        let admin = MysqlCliAdmin::new(fake.clone(), fake.clone(), Arc::new(NoMysqlRootPasswords));
        admin.sync_project_accounts(&endpoint(dir.path()), &[grant("shop", &["shop", "shop_wt_fix"])]).await.unwrap();

        let calls = fake.calls.lock().unwrap();
        assert!(calls[0][0].args.contains(&r"--execute=SELECT User, Host FROM mysql.user WHERE User LIKE 'lampo\\_%'".to_string()), "{:?}", calls[0][0].args);
        drop(calls);
        let expected = "\
DROP USER IF EXISTS 'lampo_gone'@'localhost';
CREATE USER IF NOT EXISTS 'lampo_shop'@'localhost' IDENTIFIED BY 'shop-pass';
ALTER USER 'lampo_shop'@'localhost' IDENTIFIED BY 'shop-pass';
REVOKE ALL PRIVILEGES, GRANT OPTION FROM 'lampo_shop'@'localhost';
GRANT ALL PRIVILEGES ON `shop`.* TO 'lampo_shop'@'localhost';
GRANT ALL PRIVILEGES ON `shop\\_wt\\_fix`.* TO 'lampo_shop'@'localhost';
CREATE USER IF NOT EXISTS 'lampo_shop'@'127.0.0.1' IDENTIFIED BY 'shop-pass';
ALTER USER 'lampo_shop'@'127.0.0.1' IDENTIFIED BY 'shop-pass';
REVOKE ALL PRIVILEGES, GRANT OPTION FROM 'lampo_shop'@'127.0.0.1';
GRANT ALL PRIVILEGES ON `shop`.* TO 'lampo_shop'@'127.0.0.1';
GRANT ALL PRIVILEGES ON `shop\\_wt\\_fix`.* TO 'lampo_shop'@'127.0.0.1';
";
        assert_eq!(fake.stdins.lock().unwrap()[0].1, expected);
        assert_no_password_on_any_command_line(&fake, &["shop-pass"]);
    }

    #[tokio::test]
    async fn nothing_to_create_or_drop_runs_no_script() {
        let dir = tempfile::tempdir().unwrap();
        let fake = Arc::new(FakeRunner::default());
        let admin = MysqlCliAdmin::new(fake.clone(), fake.clone(), Arc::new(NoMysqlRootPasswords));
        admin.sync_project_accounts(&endpoint(dir.path()), &[]).await.unwrap();
        assert_eq!(fake.calls.lock().unwrap().len(), 1, "only the listing");
        assert!(fake.stdins.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_failing_statement_fails_the_sync() {
        let dir = tempfile::tempdir().unwrap();
        let fake = Arc::new(FakeRunner::default());
        fake.reply.lock().unwrap().extend([out(1, "", "ERROR 1396 (HY000) at line 2: Operation ALTER USER failed"), out(0, "", "")]);
        let admin = MysqlCliAdmin::new(fake.clone(), fake.clone(), Arc::new(NoMysqlRootPasswords));
        let err = admin.sync_project_accounts(&endpoint(dir.path()), &[grant("shop", &["shop"])]).await.unwrap_err();
        assert!(err.to_string().contains("ERROR 1396"), "{err}");
    }

    #[tokio::test]
    async fn an_account_for_old_php_is_created_and_kept_on_native_password() {
        let dir = tempfile::tempdir().unwrap();
        let fake = Arc::new(FakeRunner::default());
        fake.reply.lock().unwrap().extend([out(0, "", ""), out(0, "lampo_legacy\tlocalhost\n", "")]);
        let admin = MysqlCliAdmin::new(fake.clone(), fake.clone(), Arc::new(NoMysqlRootPasswords));
        let legacy = ProjectAccountGrant { auth: Some(MysqlAuthPlugin::Native), ..grant("legacy", &["legacy"]) };
        let modern = ProjectAccountGrant { auth: Some(MysqlAuthPlugin::CachingSha2), ..grant("shop", &["shop"]) };
        admin.sync_project_accounts(&endpoint(dir.path()), &[legacy, modern]).await.unwrap();

        let stdins = fake.stdins.lock().unwrap();
        let script = &stdins[0].1;
        for host in ACCOUNT_HOSTS {
            assert!(script.contains(&format!("CREATE USER IF NOT EXISTS 'lampo_legacy'@'{host}' IDENTIFIED WITH mysql_native_password BY 'legacy-pass';\n")), "{script}");
            assert!(script.contains(&format!("ALTER USER 'lampo_legacy'@'{host}' IDENTIFIED WITH mysql_native_password BY 'legacy-pass';\n")), "{script}");
            assert!(script.contains(&format!("ALTER USER 'lampo_shop'@'{host}' IDENTIFIED WITH caching_sha2_password BY 'shop-pass';\n")), "switched back with the PHP: {script}");
        }
        assert!(!script.contains("DROP USER"), "{script}");
        drop(stdins);
        assert_no_password_on_any_command_line(&fake, &["legacy-pass", "shop-pass"]);
    }

    #[tokio::test]
    async fn root_moves_to_native_password_keeping_its_stored_password_on_the_hosts_not_there_yet() {
        let dir = tempfile::tempdir().unwrap();
        let fake = Arc::new(FakeRunner::default());
        let listed = "localhost\tcaching_sha2_password\n127.0.0.1\tmysql_native_password\n::1\tcaching_sha2_password\n";
        fake.reply.lock().unwrap().extend([out(0, "", ""), out(0, listed, "")]);
        let admin = MysqlCliAdmin::new(fake.clone(), fake.clone(), Arc::new(StoredRootPassword));
        assert!(admin.use_native_password_for_root(&endpoint(dir.path())).await.unwrap());

        let calls = fake.calls.lock().unwrap();
        assert!(calls[0][0].args.contains(&"--execute=SELECT Host, plugin FROM mysql.user WHERE User = 'root'".to_string()), "{:?}", calls[0][0].args);
        assert!(calls.iter().flatten().all(|spec| spec.args[0].starts_with("--defaults-file=")), "root authenticates with its stored password");
        drop(calls);
        let expected = "\
ALTER USER 'root'@'localhost' IDENTIFIED WITH mysql_native_password BY 's3cret';
ALTER USER 'root'@'::1' IDENTIFIED WITH mysql_native_password BY 's3cret';
";
        assert_eq!(fake.stdins.lock().unwrap()[0].1, expected);
        assert_no_password_on_any_command_line(&fake, &["s3cret"]);
    }

    #[tokio::test]
    async fn root_without_a_stored_password_stays_without_one_on_native_password() {
        let dir = tempfile::tempdir().unwrap();
        let fake = Arc::new(FakeRunner::default());
        fake.reply.lock().unwrap().extend([out(0, "", ""), out(0, "localhost\tcaching_sha2_password\n", "")]);
        let admin = MysqlCliAdmin::new(fake.clone(), fake.clone(), Arc::new(NoMysqlRootPasswords));
        assert!(admin.use_native_password_for_root(&endpoint(dir.path())).await.unwrap());
        assert_eq!(fake.stdins.lock().unwrap()[0].1, "ALTER USER 'root'@'localhost' IDENTIFIED WITH mysql_native_password BY '';\n");
    }

    #[tokio::test]
    async fn root_already_on_native_password_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let fake = Arc::new(FakeRunner::default());
        fake.reply.lock().unwrap().push(out(0, "localhost\tmysql_native_password\n127.0.0.1\tmysql_native_password\n", ""));
        let admin = MysqlCliAdmin::new(fake.clone(), fake.clone(), Arc::new(NoMysqlRootPasswords));
        assert!(!admin.use_native_password_for_root(&endpoint(dir.path())).await.unwrap());
        assert_eq!(fake.calls.lock().unwrap().len(), 1, "only the listing");
        assert!(fake.stdins.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_root_listing_that_fails_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let fake = Arc::new(FakeRunner::default());
        fake.reply.lock().unwrap().push(out(1, "", DENIED));
        let admin = MysqlCliAdmin::new(fake.clone(), fake.clone(), Arc::new(NoMysqlRootPasswords));
        let err = admin.use_native_password_for_root(&endpoint(dir.path())).await.unwrap_err();
        assert!(err.to_string().contains("ERROR 1045"), "{err}");
        assert!(fake.stdins.lock().unwrap().is_empty());
    }

    #[test]
    fn quoting_escapes_what_sql_would_read_otherwise() {
        assert_eq!(sql_string(r"it's \ x"), r"'it\'s \\ x'");
        assert_eq!(like_prefix("lampo_"), r"lampo\_%");
        assert_eq!(grant_database(&DatabaseName::new("a_b").unwrap()), r"`a\_b`");
    }
}
