use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use lampo_domain::mysql::{DatabaseName, is_system_schema};
use lampo_domain::ports::{CommandOutput, CommandRunner, CommandSpec, DatabaseAdmin, DumpImportOptions, MysqlEndpoint, PipedCommand, PipedCommandRunner, ProgressSink};
use lampo_domain::progress::{CancelFlag, Progress, ProgressStage};
use lampo_domain::sql_dump::compat::{DumpCompat, suggest_compat};
use lampo_domain::sql_dump::{DUMP_HEAD_BYTES, find_database_switch};
use lampo_domain::{PortError, PortResult};

use crate::compression;
use crate::mysql::client_args;

mod compat;

const STATEMENT_TIMEOUT: Duration = Duration::from_secs(120);
/// Imports/exports of big databases; the user cancels with the `CancelFlag`.
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(12 * 60 * 60);

/// `DatabaseAdmin` via `mysql.exe` / `mysqldump.exe` of the endpoint's series (no MySQL client
/// library: works identically for 5.7 and 8.x auth plugins). Import/export/clone stream through
/// [`PipedCommandRunner::run_piped`]: plain dump files are read/written by the clients themselves,
/// compressed ones (detected by [`crate::compression`]) are (de)compressed chunk by chunk on the
/// way, and a clone pipes `mysqldump` straight into `mysql`, so nothing is held in memory.
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

/// `InvalidData` if the head of the dump has a `USE` / `CREATE DATABASE`: `--database=` only sets
/// the initial database, so the statements after it would not land in `target`.
fn refuse_database_switch(head: &[u8], sql_file: &Path, target: &DatabaseName) -> PortResult<()> {
    match find_database_switch(head) {
        None => Ok(()),
        Some(switch) => Err(PortError::invalid_data(
            sql_file.display().to_string(),
            format!(
                "line {} switches or creates a database (`{}`): MySQL would run the statements after it in the database it names, not in `{target}`, \
                 possibly overwriting another database. Remove the USE / CREATE DATABASE statements (or export without `mysqldump --databases`), \
                 or import anyway with --allow-use if that is intended",
                switch.line, switch.statement
            ),
        )),
    }
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
        Ok(out.lines().map(str::trim).filter(|l| !l.is_empty() && !is_system_schema(l)).map(String::from).collect())
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

    async fn import(&self, endpoint: &MysqlEndpoint, name: &DatabaseName, sql_file: &Path, options: DumpImportOptions, progress: &dyn ProgressSink, cancel: &CancelFlag) -> PortResult<()> {
        let total = std::fs::metadata(sql_file).map_err(|_| PortError::not_found("file", sql_file.display().to_string()))?.len();
        // Checked before touching the server: a bad archive or a dump that would write elsewhere
        // fails without creating the database.
        let stdin_compression = compression::detect_input(sql_file)?;
        let head = compression::read_head(sql_file, stdin_compression, DUMP_HEAD_BYTES)?;
        if !options.allow_database_switch {
            refuse_database_switch(&head, sql_file, name)?;
        }
        let suggestion = if options.compat.is_none() { suggest_compat(&head, endpoint.series) } else { None };
        cancel.check()?;
        let collation = compat::database_collation_clause(options.compat);
        self.execute_ok(endpoint, &format!("CREATE DATABASE IF NOT EXISTS {} CHARACTER SET utf8mb4{collation}", quote_ident(name))).await?;
        let mut restore = restore_spec(endpoint, name)?;
        compat::add_session_settings(&mut restore, endpoint, options.compat);
        let stdin_rewrite = options.compat.map(DumpCompat::rewrite).unwrap_or_default();
        let command = PipedCommand { stages: vec![restore], stdin_file: Some(sql_file.to_path_buf()), stdin_compression, stdin_rewrite, ..Default::default() };
        let on_bytes = reporter(progress, Some(total), format!("import {name}"));
        // From here on statements may already have run: whatever stops the import leaves them.
        let outputs = self.pipes.run_piped(command, &on_bytes, cancel, TRANSFER_TIMEOUT).await.map_err(PortError::interrupted)?;
        ensure_ok("mysql", &outputs).map_err(|e| compat::with_hint(e, suggestion.as_ref()).interrupted())
    }

    async fn export(&self, endpoint: &MysqlEndpoint, name: &DatabaseName, sql_file: &Path, progress: &dyn ProgressSink, cancel: &CancelFlag) -> PortResult<()> {
        let stdout_compression = compression::detect_output(sql_file)?;
        cancel.check()?;
        let partial = partial_path(sql_file);
        let command = PipedCommand { stages: vec![dump_spec(endpoint, name)?], stdout_file: Some(partial.clone()), stdout_compression, ..Default::default() };
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
        let command = PipedCommand { stages, ..Default::default() };
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
    use lampo_domain::tcp_port::Port;
    use lampo_domain::ports::{NullProgressSink, StreamCompression};
    use lampo_domain::sql_dump::rewrite::DumpRewrite;
    use std::sync::Mutex;

    /// Records invocations and answers with canned outputs.
    #[derive(Default)]
    struct FakeRunner {
        calls: Mutex<Vec<Vec<CommandSpec>>>,
        /// `(stdin_compression, stdout_compression)` of every piped command.
        compressions: Mutex<Vec<(StreamCompression, StreamCompression)>>,
        /// `stdin_rewrite` of every piped command.
        rewrites: Mutex<Vec<DumpRewrite>>,
        reply: Mutex<Vec<CommandOutput>>,
        /// Returned by the next piped command after recording it (a cancellation mid-stream).
        pipe_error: Mutex<Option<PortError>>,
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
            if let Some(out) = &command.stdout_file {
                std::fs::write(out, b"").unwrap();
            }
            self.compressions.lock().unwrap().push((command.stdin_compression, command.stdout_compression));
            self.rewrites.lock().unwrap().push(command.stdin_rewrite);
            self.calls.lock().unwrap().push(command.stages);
            if let Some(error) = self.pipe_error.lock().unwrap().take() {
                return Err(error);
            }
            Ok((0..n).map(|_| self.next()).collect())
        }
    }

    fn endpoint(dir: &Path) -> MysqlEndpoint {
        std::fs::write(dir.join("mysql.exe"), b"").unwrap();
        std::fs::write(dir.join("mysqldump.exe"), b"").unwrap();
        MysqlEndpoint { series: MysqlSeries::DEFAULT, client_dir: dir.to_path_buf(), port: Port::of(3306), user: "root".into() }
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
    async fn list_hides_system_schemas_whatever_their_case() {
        let dir = tempfile::tempdir().unwrap();
        let fake = Arc::new(FakeRunner::default());
        fake.reply.lock().unwrap().push(CommandOutput { exit_code: Some(0), stdout: "INFORMATION_SCHEMA\nMySQL\nPerformance_Schema\nSYS\nmysql_app\n".into(), stderr: String::new() });
        let admin = MysqlCliAdmin::new(fake.clone(), fake.clone());
        assert_eq!(admin.list(&endpoint(dir.path())).await.unwrap(), ["mysql_app"]);
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

    fn gzipped(bytes: &[u8]) -> Vec<u8> {
        use std::io::Write;
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        e.write_all(bytes).unwrap();
        e.finish().unwrap()
    }

    #[tokio::test]
    async fn import_streams_compressed_dumps_with_their_detected_format() {
        let dir = tempfile::tempdir().unwrap();
        let fake = Arc::new(FakeRunner::default());
        let admin = MysqlCliAdmin::new(fake.clone(), fake.clone());
        let name = DatabaseName::new("shop").unwrap();
        let plain = dir.path().join("dump.sql");
        std::fs::write(&plain, b"SELECT 1;").unwrap();
        let gz = dir.path().join("dump.sql.gz");
        std::fs::write(&gz, gzipped(b"SELECT 1;")).unwrap();
        for file in [&plain, &gz] {
            admin.import(&endpoint(dir.path()), &name, file, DumpImportOptions::default(), &NullProgressSink, &CancelFlag::new()).await.unwrap();
        }
        let compressions = fake.compressions.lock().unwrap();
        assert_eq!(*compressions, [(StreamCompression::None, StreamCompression::None), (StreamCompression::Gzip, StreamCompression::None)]);
        let calls = fake.calls.lock().unwrap();
        assert!(calls[1][0].args.contains(&"--database=shop".to_string()), "restores into the target");
    }

    #[tokio::test]
    async fn import_rejects_a_bad_archive_before_touching_the_server() {
        let dir = tempfile::tempdir().unwrap();
        let fake = Arc::new(FakeRunner::default());
        let admin = MysqlCliAdmin::new(fake.clone(), fake.clone());
        let zip_path = dir.path().join("two.zip");
        let mut w = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
        for entry in ["a.sql", "b.sql"] {
            w.start_file(entry, zip::write::SimpleFileOptions::default()).unwrap();
        }
        w.finish().unwrap();
        let err = admin.import(&endpoint(dir.path()), &DatabaseName::new("shop").unwrap(), &zip_path, DumpImportOptions::default(), &NullProgressSink, &CancelFlag::new()).await.unwrap_err();
        assert!(matches!(&err, PortError::InvalidData { message, .. } if message.contains("2 .sql files")), "{err}");
        assert!(fake.calls.lock().unwrap().is_empty(), "no CREATE DATABASE, no mysql");
    }

    const MULTI_DATABASE_DUMP: &[u8] = b"-- MySQL dump 10.13\n\nCREATE DATABASE /*!32312 IF NOT EXISTS*/ `other_db`;\n\nUSE `other_db`;\nCREATE TABLE t (id INT);\n";

    #[tokio::test]
    async fn import_refuses_a_dump_that_switches_database_before_touching_the_server() {
        let dir = tempfile::tempdir().unwrap();
        let fake = Arc::new(FakeRunner::default());
        let admin = MysqlCliAdmin::new(fake.clone(), fake.clone());
        let name = DatabaseName::new("shop").unwrap();
        let plain = dir.path().join("all.sql");
        std::fs::write(&plain, b"-- dump\nUSE `other_db`;\nDROP TABLE IF EXISTS users;\n").unwrap();
        let gz = dir.path().join("all.sql.gz");
        std::fs::write(&gz, gzipped(MULTI_DATABASE_DUMP)).unwrap();
        for (file, line, statement) in [(&plain, 2, "USE `other_db`;"), (&gz, 3, "CREATE DATABASE /*!32312 IF NOT EXISTS*/ `other_db`;")] {
            let err = admin.import(&endpoint(dir.path()), &name, file, DumpImportOptions::default(), &NullProgressSink, &CancelFlag::new()).await.unwrap_err();
            let PortError::InvalidData { message, .. } = &err else { panic!("{err}") };
            assert!(message.contains(&format!("line {line} switches or creates a database (`{statement}`)")), "{message}");
            assert!(message.contains("not in `shop`") && message.contains("--allow-use"), "{message}");
        }
        assert!(fake.calls.lock().unwrap().is_empty(), "no CREATE DATABASE, no mysql");
    }

    #[tokio::test]
    async fn import_runs_a_dump_that_switches_database_when_allowed() {
        let dir = tempfile::tempdir().unwrap();
        let fake = Arc::new(FakeRunner::default());
        let admin = MysqlCliAdmin::new(fake.clone(), fake.clone());
        let file = dir.path().join("all.sql");
        std::fs::write(&file, MULTI_DATABASE_DUMP).unwrap();
        let allow = DumpImportOptions { allow_database_switch: true, ..Default::default() };
        admin.import(&endpoint(dir.path()), &DatabaseName::new("shop").unwrap(), &file, allow, &NullProgressSink, &CancelFlag::new()).await.unwrap();
        assert_eq!(fake.calls.lock().unwrap().len(), 2, "CREATE DATABASE, then the restore");
    }

    #[tokio::test]
    async fn an_import_stopped_mid_stream_is_reported_as_interrupted() {
        let dir = tempfile::tempdir().unwrap();
        let fake = Arc::new(FakeRunner::default());
        let admin = MysqlCliAdmin::new(fake.clone(), fake.clone());
        let (ep, name, file) = (endpoint(dir.path()), DatabaseName::new("shop").unwrap(), dir.path().join("dump.sql"));
        std::fs::write(&file, b"SELECT 1;").unwrap();
        let import = |cancel: CancelFlag| {
            let (admin, ep, name, file) = (&admin, &ep, &name, &file);
            async move { admin.import(ep, name, file, DumpImportOptions::default(), &NullProgressSink, &cancel).await.unwrap_err() }
        };

        *fake.pipe_error.lock().unwrap() = Some(PortError::Cancelled);
        assert_eq!(import(CancelFlag::new()).await, PortError::Interrupted(Box::new(PortError::Cancelled)));

        // Replies are popped from the end: restore fails, CREATE DATABASE ok.
        fake.reply.lock().unwrap().extend([
            CommandOutput { exit_code: Some(1), stdout: String::new(), stderr: "ERROR 1064 at line 1".into() },
            CommandOutput { exit_code: Some(0), stdout: String::new(), stderr: String::new() },
        ]);
        let err = import(CancelFlag::new()).await;
        assert!(matches!(&err, PortError::Interrupted(cause) if cause.to_string().contains("ERROR 1064")), "{err}");

        let cancelled = CancelFlag::new();
        cancelled.cancel();
        assert_eq!(import(cancelled).await, PortError::Cancelled, "cancelled before reaching the server: nothing was written");
    }

    #[tokio::test]
    async fn export_gzips_a_gz_destination_and_refuses_zip() {
        let dir = tempfile::tempdir().unwrap();
        let fake = Arc::new(FakeRunner::default());
        let admin = MysqlCliAdmin::new(fake.clone(), fake.clone());
        let name = DatabaseName::new("shop").unwrap();
        let out = dir.path().join("shop.sql.gz");
        admin.export(&endpoint(dir.path()), &name, &out, &NullProgressSink, &CancelFlag::new()).await.unwrap();
        assert!(out.exists() && !partial_path(&out).exists());
        assert_eq!(*fake.compressions.lock().unwrap(), [(StreamCompression::None, StreamCompression::Gzip)]);
        let err = admin.export(&endpoint(dir.path()), &name, &dir.path().join("shop.zip"), &NullProgressSink, &CancelFlag::new()).await.unwrap_err();
        assert!(err.to_string().contains("use .sql.gz"), "{err}");
        assert_eq!(fake.calls.lock().unwrap().len(), 1);
    }

    const MARIADB_HEADER: &[u8] =
        b"/*M!999999\\- enable the sandbox mode */\n-- MariaDB dump 10.19-11.4.2-MariaDB, for Win64 (AMD64)\n-- Server version\t11.4.2-MariaDB\nCREATE TABLE t (id INT);\n";

    fn endpoint_of(dir: &Path, series: &str) -> MysqlEndpoint {
        MysqlEndpoint { series: MysqlSeries::parse(series).unwrap(), ..endpoint(dir) }
    }

    fn import_args(calls: &[Vec<CommandSpec>]) -> (&[String], &[String]) {
        (&calls[0][0].args, &calls[1][0].args)
    }

    #[tokio::test]
    async fn import_with_compat_sets_the_session_and_rewrites_the_stream() {
        let dir = tempfile::tempdir().unwrap();
        let fake = Arc::new(FakeRunner::default());
        let admin = MysqlCliAdmin::new(fake.clone(), fake.clone());
        let file = dir.path().join("maria.sql.gz");
        std::fs::write(&file, gzipped(MARIADB_HEADER)).unwrap();
        let options = DumpImportOptions { compat: Some(DumpCompat::Mariadb), ..Default::default() };
        admin.import(&endpoint(dir.path()), &DatabaseName::new("shop").unwrap(), &file, options, &NullProgressSink, &CancelFlag::new()).await.unwrap();

        let calls = fake.calls.lock().unwrap();
        let (create, restore) = import_args(&calls);
        assert!(create.contains(&"--execute=CREATE DATABASE IF NOT EXISTS `shop` CHARACTER SET utf8mb4 COLLATE utf8mb4_general_ci".to_string()), "{create:?}");
        let init =
            "--init-command=SET SESSION restrict_fk_on_non_standard_key=OFF, SESSION default_collation_for_utf8mb4=utf8mb4_general_ci, SESSION collation_connection=utf8mb4_general_ci";
        assert_eq!(restore.iter().filter(|a| a.starts_with("--init-command=")).collect::<Vec<_>>(), [init]);
        assert!(restore.contains(&"--database=shop".to_string()));
        assert_eq!(*fake.rewrites.lock().unwrap(), [DumpRewrite { drop_sandbox_line: true, strip_definers: true }]);
        assert_eq!(*fake.compressions.lock().unwrap(), [(StreamCompression::Gzip, StreamCompression::None)], "decompressed and rewritten on the way");
    }

    #[tokio::test]
    async fn import_session_settings_follow_the_target_series() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("dump.sql");
        std::fs::write(&file, b"SELECT 1;\n").unwrap();
        let cases = [
            ("8.4", Some(DumpCompat::Mysql80), Some("--init-command=SET SESSION restrict_fk_on_non_standard_key=OFF")),
            ("8.0", Some(DumpCompat::Mysql80), None),
            ("5.7", Some(DumpCompat::Mariadb), None),
            ("8.4", None, None),
        ];
        for (series, compat, expected) in cases {
            let fake = Arc::new(FakeRunner::default());
            let admin = MysqlCliAdmin::new(fake.clone(), fake.clone());
            let options = DumpImportOptions { compat, ..Default::default() };
            admin.import(&endpoint_of(dir.path(), series), &DatabaseName::new("shop").unwrap(), &file, options, &NullProgressSink, &CancelFlag::new()).await.unwrap();
            let calls = fake.calls.lock().unwrap();
            let (create, restore) = import_args(&calls);
            let init: Vec<&str> = restore.iter().filter(|a| a.starts_with("--init-command=")).map(String::as_str).collect();
            assert_eq!(init, expected.into_iter().collect::<Vec<_>>(), "{series} {compat:?}");
            assert!(create.last().unwrap().ends_with("CHARACTER SET utf8mb4") == (compat != Some(DumpCompat::Mariadb)), "{create:?}");
            let rewrite = *fake.rewrites.lock().unwrap().first().unwrap();
            assert_eq!(rewrite.is_noop(), compat.is_none(), "without --compat the stream is untouched");
        }
    }

    #[tokio::test]
    async fn a_failed_import_of_a_dump_from_another_server_suggests_compat() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("maria.sql");
        std::fs::write(&file, MARIADB_HEADER).unwrap();
        let failing = || {
            let fake = Arc::new(FakeRunner::default());
            // Replies are popped from the end: restore fails, CREATE DATABASE ok.
            fake.reply.lock().unwrap().extend([
                CommandOutput { exit_code: Some(1), stdout: String::new(), stderr: "ERROR at line 1: Unknown command '\\-'.".into() },
                CommandOutput { exit_code: Some(0), stdout: String::new(), stderr: String::new() },
            ]);
            MysqlCliAdmin::new(fake.clone(), fake)
        };
        let name = DatabaseName::new("shop").unwrap();

        let err = failing().import(&endpoint(dir.path()), &name, &file, DumpImportOptions::default(), &NullProgressSink, &CancelFlag::new()).await.unwrap_err();
        let PortError::Interrupted(cause) = &err else { panic!("{err}") };
        let message = cause.to_string();
        assert!(message.contains("Unknown command") && message.contains("made by MariaDB (server version 11.4.2-MariaDB)") && message.contains("--compat mariadb"), "{message}");

        let options = DumpImportOptions { compat: Some(DumpCompat::Mariadb), ..Default::default() };
        let err = failing().import(&endpoint(dir.path()), &name, &file, options, &NullProgressSink, &CancelFlag::new()).await.unwrap_err();
        assert!(!err.to_string().contains("--compat"), "already imported with --compat: {err}");
    }
}
