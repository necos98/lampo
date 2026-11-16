//! Processes and services. Process supervision and port probing are implemented by
//! `lampo-infra-services`; service runners by `lampo-infra-services` (MySQL, Redis) and
//! `lampo-infra-web` (Apache), all on top of [`ProcessSupervisor`] / [`CommandRunner`].

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::PortResult;
use crate::logs::{LogChunk, LogQuery};
use crate::mysql::{DatabaseName, MysqlSeries};
use crate::ports::events::ProgressSink;
use crate::progress::CancelFlag;
use crate::service::{LogStream, ServiceId, ServiceKind, ServiceLaunch, ServiceStatus};
use crate::tcp_port::Port;
use crate::sql_dump::compat::DumpCompat;
use crate::sql_dump::rewrite::DumpRewrite;

/// A program invocation.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CommandSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub working_dir: Option<PathBuf>,
    /// Added to (not replacing) the inherited environment.
    pub env: Vec<(String, String)>,
    /// Directories put in front of the inherited `PATH` (in order) for the child. The program
    /// itself is not searched there: give it as an absolute path.
    pub path_prepend: Vec<PathBuf>,
}

/// Handle of a supervised child, unique for the supervisor's lifetime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ChildId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpawnedChild {
    pub id: ChildId,
    pub pid: u32,
}

/// Callbacks for a supervised child. Called from the supervisor's reader tasks: keep them cheap.
pub trait ProcessObserver: Send + Sync {
    fn on_output(&self, stream: LogStream, line: &str);
    /// The child exited (on its own or after `terminate`). `code` is `None` if killed.
    fn on_exit(&self, code: Option<i32>);
}

/// Long-running children, all assigned to one Windows Job Object
/// (`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`) so nothing survives the app. No console windows.
/// Each child is the root of a process tree (httpd → worker → php-cgi, mysqld's monitor →
/// server): a kill takes the whole tree, and what the tree still runs when its root exits is
/// killed too, so nothing keeps the service's ports.
#[async_trait]
pub trait ProcessSupervisor: Send + Sync {
    async fn spawn(&self, spec: CommandSpec, observer: Arc<dyn ProcessObserver>) -> PortResult<SpawnedChild>;
    /// Waits up to `grace` for the child to exit (after the runner's own shutdown request),
    /// then kills its whole process tree. Ok if it already exited.
    async fn terminate(&self, child: ChildId, grace: Duration) -> PortResult<()>;
    fn is_alive(&self, child: ChildId) -> bool;
    /// Whether the TCP listener on `port` belongs to the child's process tree, i.e. the port
    /// is served by this child and not by somebody else's server.
    fn owns_listener(&self, child: ChildId, port: Port) -> bool;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl CommandOutput {
    pub fn success(&self) -> bool {
        self.exit_code == Some(0)
    }
}

/// Commands run to completion with their standard streams connected like a shell pipeline
/// (`a < in | b > out`), streamed without buffering the data in memory (SQL dumps).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PipedCommand {
    /// At least one stage; stage `i`'s stdout feeds stage `i + 1`'s stdin.
    pub stages: Vec<CommandSpec>,
    /// Streamed into the first stage's stdin (`None`: empty stdin).
    pub stdin_file: Option<PathBuf>,
    /// Created/truncated and filled with the last stage's stdout (`None`: captured into the last
    /// [`CommandOutput::stdout`]; meant for small outputs).
    pub stdout_file: Option<PathBuf>,
    /// How `stdin_file` is decoded on its way into the first stage (streamed, never buffered).
    pub stdin_compression: StreamCompression,
    /// How the last stage's stdout is encoded into `stdout_file` (`Zip` is not supported).
    pub stdout_compression: StreamCompression,
    /// Rewrites applied to the decoded `stdin_file` on its way into the first stage (line by line,
    /// constant memory; the file itself is never changed). Default: none.
    pub stdin_rewrite: DumpRewrite,
}

/// Compression of a file streamed by a [`PipedCommand`] (compressed SQL dumps). The domain only
/// names the format; adapters detect and (de)compress it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StreamCompression {
    #[default]
    None,
    /// gzip (`.sql.gz`), concatenated members included.
    Gzip,
    /// A zip archive whose single `.sql` entry is the stream (decoding only).
    Zip,
}

/// Short-lived commands run to completion (`httpd -t`, `mysqld --initialize-insecure`).
/// Also inside the Job Object, no console window.
#[async_trait]
pub trait CommandRunner: Send + Sync {
    /// Runs and captures output; a non-zero exit is *not* an error here (inspect the output).
    async fn run(&self, spec: CommandSpec, timeout: Duration) -> PortResult<CommandOutput>;
}

/// Streams data through short-lived commands (SQL dumps): a separate capability from
/// [`CommandRunner`], so an adapter that needs it (`MysqlCliAdmin`) cannot be given a runner that
/// lacks it. Also inside the Job Object, no console window.
#[async_trait]
pub trait PipedCommandRunner: Send + Sync {
    /// Runs a [`PipedCommand`]; returns one output per stage (non-zero exits are not errors).
    /// `on_bytes` receives, throttled, the running total of bytes streamed: read from
    /// `stdin_file` when set, otherwise written by the first stage. On cancel/timeout every
    /// stage is killed and `Cancelled`/`Process` is returned.
    async fn run_piped(&self, command: PipedCommand, on_bytes: &(dyn Fn(u64) + Send + Sync), cancel: &CancelFlag, timeout: Duration) -> PortResult<Vec<CommandOutput>>;
}

/// Runs one command to completion while streaming its output lines (stdout and stderr, split
/// at LF and CR so progress counters arrive live), for long user-visible tools (`git clone`,
/// Composer, the Laravel Installer). On cancel the **whole process tree** is killed (a Job
/// Object per run, nested in the app's job) and `Cancelled` is returned. No console window.
#[async_trait]
pub trait StreamingCommandRunner: Send + Sync {
    /// The exit code (`None` if it could not be read); a non-zero exit is not an error here.
    async fn run_streaming(&self, spec: CommandSpec, on_line: &(dyn for<'l> Fn(LogStream, &'l str) + Send + Sync), cancel: &CancelFlag) -> PortResult<Option<i32>>;
}

/// Controls every instance of one [`ServiceKind`]. One implementation per kind; the
/// application routes by `kind()` (open-closed: a new service kind is a new runner).
///
/// Runners publish `DomainEvent::ServiceStatusChanged` / `ServiceLog` through the
/// `EventPublisher` they receive at construction, including on unexpected exits.
#[async_trait]
pub trait ServiceRunner: Send + Sync {
    fn kind(&self) -> ServiceKind;
    /// Idempotent one-time setup plus config rendering before a start: write `my.ini` /
    /// `redis.conf`, run `mysqld --initialize-insecure` if the data dir is empty
    /// (reporting `ProgressStage::Initializing` via events is optional).
    async fn prepare(&self, launch: &ServiceLaunch) -> PortResult<()>;
    /// Starts the instance and returns once it is accepting connections (or failed).
    async fn start(&self, launch: &ServiceLaunch) -> PortResult<()>;
    async fn stop(&self, service: &ServiceId) -> PortResult<()>;
    /// Applies new configuration. Default: stop then start.
    async fn restart(&self, launch: &ServiceLaunch) -> PortResult<()> {
        self.stop(&launch.service).await?;
        self.start(launch).await
    }
    fn status(&self, service: &ServiceId) -> ServiceStatus;
}

/// Reads log files (any [`crate::logs::LogSource`], resolved to a path by the caller).
/// File-based so it works without the app; a missing file yields an empty chunk.
pub trait LogFileReader: Send + Sync {
    /// Last lines matching `query`, oldest first; `next_offset` = current end of file.
    fn tail(&self, file: &Path, query: &LogQuery) -> PortResult<LogChunk>;
    /// Complete lines appended since byte `offset` (for `--follow` polling). If the file was
    /// truncated/rotated (shorter than `offset`) reading restarts from 0.
    fn read_from(&self, file: &Path, offset: u64) -> PortResult<LogChunk>;
}

/// Keeps log files bounded, for the logs whose writer reopens them for every entry (PHP's
/// `error_log`) and so cannot rotate them itself. The current file stays where
/// [`LogFileReader`] finds it.
pub trait LogRotator: Send + Sync {
    /// Moves `file` aside when it is over the size limit; a missing or small file is left
    /// alone.
    fn rotate_if_large(&self, file: &Path) -> PortResult<()>;
}

/// Where to reach a MySQL instance with its command-line clients.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MysqlEndpoint {
    pub series: MysqlSeries,
    /// Directory with `mysql.exe` / `mysqldump.exe`.
    pub client_dir: PathBuf,
    pub port: Port,
    pub user: String,
}

/// How [`DatabaseAdmin::import`] treats a dump. The default is the safe one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DumpImportOptions {
    /// Import even when the dump has `USE` / `CREATE DATABASE` statements: its statements may
    /// then run in (and create) databases other than the target.
    pub allow_database_switch: bool,
    /// The server the dump comes from, when it is not the target's own: its fixes are applied in
    /// the importing session and in the stream ([`DumpCompat`]). `None`: the dump runs as it is.
    pub compat: Option<DumpCompat>,
}

/// Database administration on a running instance (via the MySQL command-line clients).
#[async_trait]
pub trait DatabaseAdmin: Send + Sync {
    /// User databases (system schemas excluded).
    async fn list(&self, endpoint: &MysqlEndpoint) -> PortResult<Vec<String>>;
    /// `AlreadyExists` if present.
    async fn create(&self, endpoint: &MysqlEndpoint, name: &DatabaseName) -> PortResult<()>;
    /// Ok if absent.
    async fn drop_database(&self, endpoint: &MysqlEndpoint, name: &DatabaseName) -> PortResult<()>;
    /// Runs an SQL dump file into `name` (created if missing): plain SQL, gzip (`.sql.gz`) or a
    /// zip holding exactly one `.sql` file, streamed. Reports bytes of the file read.
    ///
    /// Before touching the server, a dump whose head ([`crate::sql_dump::DUMP_HEAD_BYTES`])
    /// switches or creates a database is refused with `InvalidData` unless `options` allows it.
    /// With `options.compat`, the dump is rewritten on its way and the session adjusted (see
    /// [`crate::sql_dump::compat`]); without it, a failing dump whose header names another server
    /// gets a suggestion of the `--compat` to use.
    /// A failure or cancellation once statements may have run is [`PortError::Interrupted`]:
    /// `name` may be left incomplete.
    ///
    /// [`PortError::Interrupted`]: crate::PortError::Interrupted
    async fn import(&self, endpoint: &MysqlEndpoint, name: &DatabaseName, sql_file: &Path, options: DumpImportOptions, progress: &dyn ProgressSink, cancel: &CancelFlag) -> PortResult<()>;
    /// Dumps `name` to `sql_file` (`--single-transaction --routines --triggers`), gzipped when
    /// its name ends in `.gz`.
    async fn export(&self, endpoint: &MysqlEndpoint, name: &DatabaseName, sql_file: &Path, progress: &dyn ProgressSink, cancel: &CancelFlag) -> PortResult<()>;
    /// Creates `target` as a copy of `source` on the same instance (dump piped into restore).
    async fn clone_database(&self, endpoint: &MysqlEndpoint, source: &DatabaseName, target: &DatabaseName, progress: &dyn ProgressSink, cancel: &CancelFlag) -> PortResult<()>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum PortAvailability {
    Free,
    /// `owner` is a human-readable process description when it can be found (`skype.exe (pid 42)`).
    InUse {
        owner: Option<String>,
    },
}

/// Checks local TCP ports before starting services.
pub trait PortProbe: Send + Sync {
    fn check(&self, port: Port) -> PortResult<PortAvailability>;
}
