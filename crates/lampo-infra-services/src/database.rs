use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use lampo_domain::mysql::DatabaseName;
use lampo_domain::ports::{CommandOutput, CommandRunner, CommandSpec, DatabaseAdmin, MysqlEndpoint, PipedCommand, PipedCommandRunner, ProgressSink};
use lampo_domain::progress::{CancelFlag, Progress, ProgressStage};
use lampo_domain::{PortError, PortResult};

use crate::mysql::client_args;

const STATEMENT_TIMEOUT: Duration = Duration::from_secs(120);
/// Imports/exports of big databases; the user cancels with the `CancelFlag`.
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(12 * 60 * 60);
const SYSTEM_SCHEMAS: [&str; 4] = ["information_schema", "mysql", "performance_schema", "sys"];

/// `DatabaseAdmin` via `mysql.exe` / `mysqldump.exe` of the endpoint's series (no MySQL client
/// library: works identically for 5.7 and 8.x auth plugins). Import/export/clone stream through
/// [`PipedCommandRunner::run_piped`]: dump files are read/written by the clients themselves and a
/// clone pipes `mysqldump` straight into `mysql`, so nothing is held in memory.
///
/// Safety against odd names: names are already restricted by [`DatabaseName`]; they are also
/// backtick-quoted (with backticks doubled) in SQL, passed as separate arguments (never through
/// a shell), and `--no-defaults` keeps stray `my.ini` files from changing the connection.
pub struct MysqlCliAdmin {
    /// Single statements (`CREATE DATABASE`, `SHOW DATABASES`).
    commands: Arc<dyn CommandRunner>,
    /// Import / export / clone streams.
    pipes: Arc<dyn PipedCommandRunner>,
}

impl MysqlCliAdmin {
    pub fn new(commands: Arc<dyn CommandRunner>, pipes: Arc<dyn PipedCommandRunner>) -> Self {
        Self { commands, pipes }
    }
}

fn quote_ident(name: &DatabaseName) -> String {
    format!("`{}`", name.as_str().replace('`', "``"))
}

fn client(endpoint: &MysqlEndpoint, exe: &str) -> PortResult<PathBuf> {
    let path = endpoint.client_dir.join(exe);
    if path.is_file() { Ok(path) } else { Err(PortError::not_found("executable", path.display().to_string())) }
}

fn base_args(endpoint: &MysqlEndpoint) -> Vec<String> {
    let mut args = client_args(endpoint.port, &endpoint.user);
    args.push("--default-character-set=utf8mb4".into());
    args
}

fn mysql_spec(endpoint: &MysqlEndpoint, extra: &[String]) -> PortResult<CommandSpec> {
    let mut args = base_args(endpoint);
    args.extend_from_slice(extra);
    Ok(CommandSpec { program: client(endpoint, "mysql.exe")?, args, ..Default::default() })
}

fn dump_spec(endpoint: &MysqlEndpoint, name: &DatabaseName) -> PortResult<CommandSpec> {
    let mut args = base_args(endpoint);
    args.extend(["--single-transaction", "--routines", "--triggers", "--events", "--hex-blob", "--set-gtid-purged=OFF"].map(String::from));
    args.push(name.as_str().to_string());
    Ok(CommandSpec { program: client(endpoint, "mysqldump.exe")?, args, ..Default::default() })
}

fn restore_spec(endpoint: &MysqlEndpoint, name: &DatabaseName) -> PortResult<CommandSpec> {
    mysql_spec(endpoint, &["--max-allowed-packet=1073741824".into(), format!("--database={}", name.as_str())])
}

fn failure(program: &str, out: &CommandOutput) -> PortError {
    let detail = out.stderr.trim();
    PortError::process(program, if detail.is_empty() { format!("exit code {:?}", out.exit_code) } else { detail.to_string() })
}

/// The last failing stage is the most telling (a failed restore makes the dump fail with a
/// broken pipe, not the other way round).
fn ensure_ok(program: &str, outputs: &[CommandOutput]) -> PortResult<()> {
    match outputs.iter().rev().find(|o| !o.success()) {
        Some(out) => Err(failure(program, out)),
        None => Ok(()),
    }
}

fn reporter<'a>(progress: &'a dyn ProgressSink, total: Option<u64>, subject: String) -> impl Fn(u64) + Send + Sync + 'a {
    // No dedicated stage exists for database transfers; `Configuring` with a subject.
    move |done| progress.report(Progress::new(ProgressStage::Configuring, done, total).with_subject(subject.clone()))
}

fn partial_path(sql_file: &Path) -> PathBuf {
    let mut p = sql_file.as_os_str().to_owned();
    p.push(".partial");
    PathBuf::from(p)
}

impl MysqlCliAdmin {
    async fn execute(&self, endpoint: &MysqlEndpoint, statement: &str) -> PortResult<CommandOutput> {
        let spec = mysql_spec(endpoint, &["--batch".into(), "--skip-column-names".into(), format!("--execute={statement}")])?;
        self.commands.run(spec, STATEMENT_TIMEOUT).await
    }

    async fn execute_ok(&self, endpoint: &MysqlEndpoint, statement: &str) -> PortResult<String> {
        let out = self.execute(endpoint, statement).await?;
        if out.success() { Ok(out.stdout) } else { Err(failure("mysql", &out)) }
    }
}

#[async_trait]
impl DatabaseAdmin for MysqlCliAdmin {
    async fn list(&self, endpoint: &MysqlEndpoint) -> PortResult<Vec<String>> {
        let out = self.execute_ok(endpoint, "SHOW DATABASES").await?;
        Ok(out.lines().map(str::trim).filter(|l| !l.is_empty() && !SYSTEM_SCHEMAS.contains(&l.to_ascii_lowercase().as_str())).map(String::from).collect())
    }

    async fn create(&self, endpoint: &MysqlEndpoint, name: &DatabaseName) -> PortResult<()> {
        let out = self.execute(endpoint, &format!("CREATE DATABASE {} CHARACTER SET utf8mb4", quote_ident(name))).await?;
        if out.success() {
            Ok(())
        } else if out.stderr.contains("ERROR 1007") {
            Err(PortError::AlreadyExists { entity: "database", id: name.to_string() })
        } else {
            Err(failure("mysql", &out))
        }
    }

    async fn drop_database(&self, endpoint: &MysqlEndpoint, name: &DatabaseName) -> PortResult<()> {
        self.execute_ok(endpoint, &format!("DROP DATABASE IF EXISTS {}", quote_ident(name))).await.map(drop)
    }

    async fn import(&self, endpoint: &MysqlEndpoint, name: &DatabaseName, sql_file: &Path, progress: &dyn ProgressSink, cancel: &CancelFlag) -> PortResult<()> {
        let total = std::fs::metadata(sql_file).map_err(|_| PortError::not_found("file", sql_file.display().to_string()))?.len();
        cancel.check()?;
        self.execute_ok(endpoint, &format!("CREATE DATABASE IF NOT EXISTS {} CHARACTER SET utf8mb4", quote_ident(name))).await?;
        let command = PipedCommand { stages: vec![restore_spec(endpoint, name)?], stdin_file: Some(sql_file.to_path_buf()), stdout_file: None };
        let on_bytes = reporter(progress, Some(total), format!("import {name}"));
        let outputs = self.pipes.run_piped(command, &on_bytes, cancel, TRANSFER_TIMEOUT).await?;
        ensure_ok("mysql", &outputs)
    }

    async fn export(&self, endpoint: &MysqlEndpoint, name: &DatabaseName, sql_file: &Path, progress: &dyn ProgressSink, cancel: &CancelFlag) -> PortResult<()> {
        cancel.check()?;
        let partial = partial_path(sql_file);
        let command = PipedCommand { stages: vec![dump_spec(endpoint, name)?], stdin_file: None, stdout_file: Some(partial.clone()) };
        let on_bytes = reporter(progress, None, format!("export {name}"));
        let result = match self.pipes.run_piped(command, &on_bytes, cancel, TRANSFER_TIMEOUT).await {
            Ok(outputs) => ensure_ok("mysqldump", &outputs),
            Err(e) => Err(e),
        };
        match result {
            Ok(()) => std::fs::rename(&partial, sql_file).map_err(|e| PortError::io(format!("writing {}", sql_file.display()), e)),
            Err(e) => {
                let _ = std::fs::remove_file(&partial);
                Err(e)
            }
        }
    }

    async fn clone_database(&self, endpoint: &MysqlEndpoint, source: &DatabaseName, target: &DatabaseName, progress: &dyn ProgressSink, cancel: &CancelFlag) -> PortResult<()> {
        cancel.check()?;
        let stages = vec![dump_spec(endpoint, source)?, restore_spec(endpoint, target)?];
        self.create(endpoint, target).await?;
        let command = PipedCommand { stages, stdin_file: None, stdout_file: None };
        let on_bytes = reporter(progress, None, format!("clone {source} -> {target}"));
        let result = match self.pipes.run_piped(command, &on_bytes, cancel, TRANSFER_TIMEOUT).await {
            Ok(outputs) => ensure_ok("mysqldump | mysql", &outputs),
            Err(e) => Err(e),
        };
        if result.is_err() {
            let _ = self.drop_database(endpoint, target).await;
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lampo_domain::mysql::MysqlSeries;
    use lampo_domain::ports::NullProgressSink;
    use std::sync::Mutex;

    /// Records invocations and answers with canned outputs.
    #[derive(Default)]
    struct FakeRunner {
        calls: Mutex<Vec<Vec<CommandSpec>>>,
        reply: Mutex<Vec<CommandOutput>>,
    }

    impl FakeRunner {
        fn next(&self) -> CommandOutput {
            self.reply.lock().unwrap().pop().unwrap_or(CommandOutput { exit_code: Some(0), stdout: String::new(), stderr: String::new() })
        }
    }

    #[async_trait]
    impl CommandRunner for FakeRunner {
        async fn run(&self, spec: CommandSpec, _timeout: Duration) -> PortResult<CommandOutput> {
            self.calls.lock().unwrap().push(vec![spec]);
            Ok(self.next())
        }
    }

    #[async_trait]
    impl PipedCommandRunner for FakeRunner {
        async fn run_piped(&self, command: PipedCommand, on_bytes: &(dyn Fn(u64) + Send + Sync), _c: &CancelFlag, _t: Duration) -> PortResult<Vec<CommandOutput>> {
            on_bytes(42);
            let n = command.stages.len();
            self.calls.lock().unwrap().push(command.stages);
            Ok((0..n).map(|_| self.next()).collect())
        }
    }

    fn endpoint(dir: &Path) -> MysqlEndpoint {
        std::fs::write(dir.join("mysql.exe"), b"").unwrap();
        std::fs::write(dir.join("mysqldump.exe"), b"").unwrap();
        MysqlEndpoint { series: MysqlSeries::DEFAULT, client_dir: dir.to_path_buf(), port: 3306, user: "root".into() }
    }

    #[tokio::test]
    async fn list_filters_system_schemas_and_uses_safe_arguments() {
        let dir = tempfile::tempdir().unwrap();
        let fake = Arc::new(FakeRunner::default());
        fake.reply.lock().unwrap().push(CommandOutput { exit_code: Some(0), stdout: "information_schema\nmysql\nperformance_schema\nshop\nsys\nshop_wt\n".into(), stderr: String::new() });
        let admin = MysqlCliAdmin::new(fake.clone(), fake.clone());
        assert_eq!(admin.list(&endpoint(dir.path())).await.unwrap(), ["shop", "shop_wt"]);
        let calls = fake.calls.lock().unwrap();
        let args = &calls[0][0].args;
        assert_eq!(args[0], "--no-defaults", "must be the first option");
        assert!(args.contains(&"--host=127.0.0.1".to_string()));
        assert!(args.contains(&"--port=3306".to_string()));
    }

    #[tokio::test]
    async fn create_maps_existing_database_and_quotes_names() {
        let dir = tempfile::tempdir().unwrap();
        let fake = Arc::new(FakeRunner::default());
        fake.reply.lock().unwrap().push(CommandOutput {
            exit_code: Some(1),
            stdout: String::new(),
            stderr: "ERROR 1007 (HY000) at line 1: Can't create database 'shop'; database exists".into(),
        });
        let admin = MysqlCliAdmin::new(fake.clone(), fake.clone());
        let name = DatabaseName::new("shop").unwrap();
        assert!(matches!(admin.create(&endpoint(dir.path()), &name).await, Err(PortError::AlreadyExists { entity: "database", .. })));
        let calls = fake.calls.lock().unwrap();
        assert!(calls[0][0].args.contains(&"--execute=CREATE DATABASE `shop` CHARACTER SET utf8mb4".to_string()));
    }

    #[tokio::test]
    async fn clone_pipes_dump_into_restore_and_drops_target_on_failure() {
        let dir = tempfile::tempdir().unwrap();
        let fake = Arc::new(FakeRunner::default());
        // Replies are popped from the end: create ok, dump ok, restore fails, drop ok.
        fake.reply.lock().unwrap().extend([
            CommandOutput { exit_code: Some(0), stdout: String::new(), stderr: String::new() },
            CommandOutput { exit_code: Some(1), stdout: String::new(), stderr: "ERROR 1064".into() },
            CommandOutput { exit_code: Some(0), stdout: String::new(), stderr: String::new() },
            CommandOutput { exit_code: Some(0), stdout: String::new(), stderr: String::new() },
        ]);
        let admin = MysqlCliAdmin::new(fake.clone(), fake.clone());
        let (src, dst) = (DatabaseName::new("shop").unwrap(), DatabaseName::new("shop_wt").unwrap());
        let err = admin.clone_database(&endpoint(dir.path()), &src, &dst, &NullProgressSink, &CancelFlag::new()).await.unwrap_err();
        assert!(err.to_string().contains("ERROR 1064"));
        let calls = fake.calls.lock().unwrap();
        assert_eq!(calls.len(), 3);
        let pipeline = &calls[1];
        assert!(pipeline[0].program.ends_with("mysqldump.exe"));
        assert!(pipeline[0].args.iter().any(|a| a == "--single-transaction"));
        assert_eq!(pipeline[0].args.last().unwrap(), "shop");
        assert!(pipeline[1].args.contains(&"--database=shop_wt".to_string()));
        assert!(calls[2][0].args.contains(&"--execute=DROP DATABASE IF EXISTS `shop_wt`".to_string()));
    }

    #[tokio::test]
    async fn export_failure_leaves_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let fake = Arc::new(FakeRunner::default());
        fake.reply.lock().unwrap().push(CommandOutput { exit_code: Some(2), stdout: String::new(), stderr: "mysqldump: Got error: 1049: Unknown database".into() });
        let admin = MysqlCliAdmin::new(fake.clone(), fake);
        let out = dir.path().join("dump.sql");
        assert!(admin.export(&endpoint(dir.path()), &DatabaseName::new("nope").unwrap(), &out, &NullProgressSink, &CancelFlag::new()).await.is_err());
        assert!(!out.exists());
        assert!(!partial_path(&out).exists());
    }
}
