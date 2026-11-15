//! MySQL instances, one per series. `prepare` writes a low-RAM `my.ini` (see
//! [`MysqlRunner::render_my_ini`]) and runs `mysqld --initialize-insecure` if the data dir is
//! empty (into a temporary sibling directory renamed on success, so an interrupted
//! initialisation never leaves a half-made data dir behind). `start` waits until the server
//! answers the MySQL handshake on its port; `stop` asks for a clean InnoDB shutdown (the
//! `MYSQLShutdown<pid>` event mysqld listens for on Windows, needing no credentials, then
//! `mysqladmin shutdown` as a fallback) before the supervisor's forced termination.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use lampo_domain::mysql::MysqlSeries;
use lampo_domain::ports::{CommandRunner, CommandSpec, EventPublisher, ProcessSupervisor, ServiceRunner};
use lampo_domain::service::{ServiceId, ServiceKind, ServiceLaunch, ServiceStatus};
use lampo_domain::{PortError, PortResult};
use tokio::io::AsyncReadExt;
use windows_sys::Win32::System::Threading::{EVENT_MODIFY_STATE, OpenEventW, SetEvent};

use crate::service_host::{MAX_LOG_BYTES, ServiceDriver, ServiceHost, create_dir, find_executable, rotate_if_large, write_if_changed};
use crate::win::{owned, raw, wide};

const INITIALIZE_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const MYSQLADMIN_TIMEOUT: Duration = Duration::from_secs(30);

fn series_of(service: &ServiceId) -> MysqlSeries {
    service.version().and_then(|v| MysqlSeries::try_from(v).ok()).unwrap_or(MysqlSeries::DEFAULT)
}

/// Option-file paths: forward slashes (backslashes are escape characters in option files) and
/// double quotes (spaces).
fn ini_path(p: &Path) -> String {
    format!("\"{}\"", p.display().to_string().replace('\\', "/"))
}

fn data_dir_of(launch: &ServiceLaunch) -> PathBuf {
    launch.data_dir.clone().unwrap_or_else(|| launch.config_dir.join("data"))
}

fn error_log(launch: &ServiceLaunch) -> PathBuf {
    launch.log_dir.join("error.log")
}

fn my_ini(launch: &ServiceLaunch) -> PathBuf {
    launch.config_dir.join("my.ini")
}

fn file_len(p: &Path) -> u64 {
    std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}

/// `[ERROR]` lines appended to the error log since `offset` (or its last lines if none).
fn error_lines_since(log: &Path, offset: u64) -> Vec<String> {
    let Ok(mut f) = std::fs::File::open(log) else { return Vec::new() };
    let offset = if file_len(log) < offset { 0 } else { offset };
    let mut text = String::new();
    if f.seek(SeekFrom::Start(offset)).is_err() || f.take(4 * 1024 * 1024).read_to_string(&mut text).is_err() {
        return Vec::new();
    }
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let errors: Vec<String> = lines.iter().filter(|l| l.contains("[ERROR]")).take(15).map(|l| l.to_string()).collect();
    if errors.is_empty() { lines[lines.len().saturating_sub(10)..].iter().map(|l| l.to_string()).collect() } else { errors }
}

fn is_empty_dir(p: &Path) -> bool {
    std::fs::read_dir(p).map(|mut d| d.next().is_none()).unwrap_or(true)
}

/// Signals the named event mysqld creates on Windows to request a normal shutdown
/// (`MYSQLShutdown<pid>`, verified on 8.4: "Normal shutdown … Shutdown complete").
fn signal_shutdown_event(pid: u32) -> bool {
    let name = wide(format!("MYSQLShutdown{pid}"));
    // SAFETY: valid NUL-terminated name.
    let Ok(event) = owned(unsafe { OpenEventW(EVENT_MODIFY_STATE, 0, name.as_ptr()) }) else { return false };
    // SAFETY: valid event handle.
    unsafe { SetEvent(raw(&event)) != 0 }
}

/// Reads the server greeting: protocol 10 handshake (or an error packet, which still proves a
/// MySQL server is accepting connections).
async fn mysql_greets(port: u16) -> bool {
    let attempt = async {
        let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.ok()?;
        let mut header = [0u8; 5];
        s.read_exact(&mut header).await.ok()?;
        Some(header[4] == 10 || header[4] == 0xFF)
    };
    tokio::time::timeout(Duration::from_secs(3), attempt).await.ok().flatten().unwrap_or(false)
}

/// mysqld 8.0.17+ on Windows forks a monitor process (only needed for the `RESTART` statement)
/// costing ~29 MB working set; `--no-monitor` is command-line only (ignored in `my.ini`). The
/// patch level of an 8.0 install is unknown here, so 8.0 keeps the monitor.
fn uses_monitor_process(series: MysqlSeries) -> bool {
    series.major() > 8 || (series.major() == 8 && series.minor() >= 1)
}

/// Client arguments common to `mysqladmin` / `mysql` / `mysqldump` on a local instance.
pub(crate) fn client_args(port: u16, user: &str) -> Vec<String> {
    vec!["--no-defaults".into(), "--protocol=TCP".into(), "--host=127.0.0.1".into(), format!("--port={port}"), format!("--user={user}")]
}

struct MysqlDriver {
    commands: Arc<dyn CommandRunner>,
}

#[async_trait]
impl ServiceDriver for MysqlDriver {
    fn program(&self) -> &'static str {
        "mysqld"
    }

    fn command(&self, launch: &ServiceLaunch) -> PortResult<CommandSpec> {
        let mysqld = find_executable(&launch.install_dir, "mysqld.exe")?;
        let basedir = mysqld.parent().and_then(Path::parent).map(Path::to_path_buf);
        // `--defaults-file` must be the first option.
        let mut args = vec![format!("--defaults-file={}", my_ini(launch).display())];
        if uses_monitor_process(series_of(&launch.service)) {
            args.push("--no-monitor".into());
        }
        Ok(CommandSpec { program: mysqld, args, working_dir: basedir, env: Vec::new(), path_prepend: Vec::new() })
    }

    fn startup_timeout(&self) -> Duration {
        Duration::from_secs(120)
    }

    fn stop_grace(&self) -> Duration {
        Duration::from_secs(60)
    }

    async fn probe(&self, launch: &ServiceLaunch) -> bool {
        mysql_greets(launch.port).await
    }

    async fn request_shutdown(&self, launch: &ServiceLaunch, pid: u32) {
        // mysqld 8 on Windows: `pid` is a monitor process, the server is the port's listener.
        let server = crate::win::tcp_listener_pid(launch.port).ok().flatten().filter(|&p| crate::win::is_same_or_descendant(p, pid));
        if server.into_iter().chain([pid]).any(signal_shutdown_event) {
            return;
        }
        let Ok(admin) = find_executable(&launch.install_dir, "mysqladmin.exe") else { return };
        let mut args = client_args(launch.port, "root");
        args.push("shutdown".into());
        match self.commands.run(CommandSpec { program: admin, args, ..Default::default() }, MYSQLADMIN_TIMEOUT).await {
            Ok(out) if out.success() => {}
            Ok(out) => tracing::warn!(service = %launch.service, stderr = %out.stderr.trim(), "mysqladmin shutdown failed"),
            Err(e) => tracing::warn!(service = %launch.service, error = %e, "mysqladmin shutdown failed"),
        }
    }

    fn start_marker(&self, launch: &ServiceLaunch) -> u64 {
        file_len(&error_log(launch))
    }

    fn failure_lines(&self, launch: &ServiceLaunch, marker: u64) -> Vec<String> {
        error_lines_since(&error_log(launch), marker)
    }
}

pub struct MysqlRunner {
    host: ServiceHost,
    driver: MysqlDriver,
    commands: Arc<dyn CommandRunner>,
}

impl MysqlRunner {
    pub fn new(supervisor: Arc<dyn ProcessSupervisor>, commands: Arc<dyn CommandRunner>, events: Arc<dyn EventPublisher>) -> Self {
        Self { host: ServiceHost::new(supervisor, events), driver: MysqlDriver { commands: commands.clone() }, commands }
    }

    /// Pure `my.ini` rendering (unit-testable).
    ///
    /// Tuned for a development machine: loopback only, no binary log / X Protocol / Performance
    /// Schema, a small buffer pool that is not re-warmed at startup, single background threads
    /// and small caches. Everything else allocated per connection or on demand stays default.
    pub fn render_my_ini(&self, launch: &ServiceLaunch) -> String {
        let series = series_of(&launch.service);
        let basedir =
            find_executable(&launch.install_dir, "mysqld.exe").ok().and_then(|p| p.parent().and_then(Path::parent).map(Path::to_path_buf)).unwrap_or_else(|| launch.install_dir.clone());
        let datadir = data_dir_of(launch);
        let mut ini = format!(
            "# Generated by Lampo before every start: changes are overwritten.\n\
             # MySQL {series} (shared by every project on this series), loopback only, low idle memory.\n\
             [mysqld]\n\
             basedir={basedir}\n\
             datadir={datadir}\n\
             port={port}\n\
             bind-address=127.0.0.1\n\
             log-error={log}\n\
             pid-file={pid}\n\
             character-set-server=utf8mb4\n\
             max_connections=100\n\
             max_allowed_packet=64M\n\
             # Memory: no Performance Schema (the largest idle consumer), small buffer pool not\n\
             # reloaded at startup, small log buffer and caches, one thread per background task.\n\
             performance_schema=OFF\n\
             innodb_buffer_pool_size=32M\n\
             innodb_buffer_pool_load_at_startup=OFF\n\
             innodb_buffer_pool_dump_at_shutdown=OFF\n\
             innodb_log_buffer_size=4M\n\
             innodb_adaptive_hash_index=OFF\n\
             innodb_page_cleaners=1\n\
             innodb_purge_threads=1\n\
             innodb_read_io_threads=1\n\
             innodb_write_io_threads=1\n\
             key_buffer_size=1M\n\
             table_open_cache=256\n\
             table_open_cache_instances=1\n\
             table_definition_cache=400\n\
             thread_cache_size=4\n\
             tmp_table_size=16M\n\
             max_heap_table_size=16M\n\
             # Development durability: flush the redo log once per second (a mysqld crash loses\n\
             # nothing, an OS crash at most ~1s of commits); much faster imports.\n\
             innodb_flush_log_at_trx_commit=2\n\
             # innodb_doublewrite stays ON on purpose: OFF saves ~30 MB but a torn page after a\n\
             # power loss would corrupt a table beyond repair.\n\
             log_error_verbosity=2\n",
            basedir = ini_path(&basedir),
            datadir = ini_path(&datadir),
            port = launch.port,
            log = ini_path(&error_log(launch)),
            pid = ini_path(&datadir.join("mysqld.pid")),
        );
        if series.is_legacy() {
            ini.push_str(
                "# 5.7 defaults that differ from 8.x.\n\
                 collation-server=utf8mb4_unicode_ci\n\
                 explicit_defaults_for_timestamp=ON\n\
                 query_cache_type=0\n\
                 query_cache_size=0\n",
            );
        } else {
            ini.push_str(
                "# 8.x: no X Protocol listener (port 33060), no binary log, small TempTable engine\n\
                 # budget, no dedicated redo-log writer threads.\n\
                 mysqlx=OFF\n\
                 disable-log-bin\n\
                 temptable_max_ram=32M\n\
                 loose-innodb_log_writer_threads=OFF\n",
            );
        }
        ini.push_str(&format!("\n[client]\nhost=127.0.0.1\nport={}\nuser=root\ndefault-character-set=utf8mb4\n", launch.port));
        ini
    }

    async fn initialize(&self, launch: &ServiceLaunch, data_dir: &Path) -> PortResult<()> {
        let mysqld = find_executable(&launch.install_dir, "mysqld.exe")?;
        let staging = data_dir.with_file_name(format!("{}.initializing", data_dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "data".into())));
        if staging.exists() {
            std::fs::remove_dir_all(&staging).map_err(|e| PortError::io(format!("removing {}", staging.display()), e))?;
        }
        let marker = file_len(&error_log(launch));
        let spec = CommandSpec {
            program: mysqld.clone(),
            args: vec![format!("--defaults-file={}", my_ini(launch).display()), "--initialize-insecure".into(), format!("--datadir={}", staging.display())],
            working_dir: mysqld.parent().and_then(Path::parent).map(Path::to_path_buf),
            env: Vec::new(),
            path_prepend: Vec::new(),
        };
        let out = self.commands.run(spec, INITIALIZE_TIMEOUT).await?;
        if !out.success() {
            let _ = std::fs::remove_dir_all(&staging);
            let mut lines = error_lines_since(&error_log(launch), marker);
            lines.extend(out.stderr.lines().filter(|l| !l.trim().is_empty()).map(str::to_string));
            return Err(PortError::process("mysqld --initialize-insecure", format!("exit code {:?}\n{}", out.exit_code, lines.join("\n"))));
        }
        if data_dir.exists() {
            std::fs::remove_dir(data_dir).map_err(|e| PortError::io(format!("replacing {}", data_dir.display()), e))?;
        }
        std::fs::rename(&staging, data_dir).map_err(|e| PortError::io(format!("moving {} to {}", staging.display(), data_dir.display()), e))
    }
}

#[async_trait]
impl ServiceRunner for MysqlRunner {
    fn kind(&self) -> ServiceKind {
        ServiceKind::MYSQL
    }

    async fn prepare(&self, launch: &ServiceLaunch) -> PortResult<()> {
        let Some(data_dir) = launch.data_dir.as_deref() else {
            return Err(PortError::invalid_data(format!("{} launch", launch.service), "a MySQL instance needs a data directory"));
        };
        find_executable(&launch.install_dir, "mysqld.exe")?;
        create_dir(&launch.config_dir)?;
        create_dir(&launch.log_dir)?;
        if let Some(parent) = data_dir.parent() {
            create_dir(parent)?;
        }
        rotate_if_large(&error_log(launch), MAX_LOG_BYTES);
        write_if_changed(&my_ini(launch), &self.render_my_ini(launch))?;
        if is_empty_dir(data_dir) {
            self.initialize(launch, data_dir).await?;
        }
        Ok(())
    }

    async fn start(&self, launch: &ServiceLaunch) -> PortResult<()> {
        self.host.start(&self.driver, launch).await
    }

    async fn stop(&self, service: &ServiceId) -> PortResult<()> {
        self.host.stop(&self.driver, service).await
    }

    fn status(&self, service: &ServiceId) -> ServiceStatus {
        self.host.status(service)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::JobObjectSupervisor;
    use lampo_domain::ports::NullEventPublisher;

    fn launch(series: MysqlSeries) -> ServiceLaunch {
        ServiceLaunch {
            service: series.service_id(),
            install_dir: r"C:\Lampo\packages\mysql".into(),
            port: 3307,
            ui_port: None,
            config_dir: r"C:\Lampo\generated\mysql\5.7".into(),
            data_dir: Some(r"C:\Users\Mario Rossi\Lampo\mysql\5.7\data".into()),
            log_dir: r"C:\Lampo\logs\mysql-5.7".into(),
        }
    }

    fn runner() -> MysqlRunner {
        let sup = Arc::new(JobObjectSupervisor::new());
        MysqlRunner::new(sup.clone(), sup, Arc::new(NullEventPublisher))
    }

    #[test]
    fn my_ini_is_loopback_low_ram_and_series_specific() {
        let r = runner();
        let legacy = r.render_my_ini(&launch(MysqlSeries::LEGACY_5_7));
        assert!(legacy.contains("port=3307\n"));
        assert!(legacy.contains("bind-address=127.0.0.1\n"));
        assert!(legacy.contains("performance_schema=OFF\n"));
        assert!(legacy.contains("datadir=\"C:/Users/Mario Rossi/Lampo/mysql/5.7/data\"\n"));
        assert!(legacy.contains("log-error=\"C:/Lampo/logs/mysql-5.7/error.log\"\n"));
        assert!(legacy.contains("query_cache_size=0\n"));
        assert!(!legacy.contains("mysqlx"), "5.7 refuses unknown options");
        assert!(!legacy.contains('\\'), "backslashes are escapes in option files");

        let current = r.render_my_ini(&launch(MysqlSeries::DEFAULT));
        assert!(current.contains("mysqlx=OFF\n"));
        assert!(current.contains("disable-log-bin\n"));
        assert!(!current.contains("query_cache"), "removed in 8.0");
    }

    #[test]
    fn command_line_disables_the_monitor_process_only_where_supported() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("bin")).unwrap();
        std::fs::write(dir.path().join("bin").join("mysqld.exe"), b"").unwrap();
        let spec = |series| MysqlDriver { commands: Arc::new(JobObjectSupervisor::new()) }.command(&ServiceLaunch { install_dir: dir.path().to_path_buf(), ..launch(series) }).unwrap();
        let current = spec(MysqlSeries::DEFAULT);
        assert!(current.args[0].starts_with("--defaults-file="));
        assert_eq!(current.args[1], "--no-monitor");
        assert_eq!(spec(MysqlSeries::LEGACY_5_7).args.len(), 1);
        assert_eq!(spec(MysqlSeries::parse("8.0").unwrap()).args.len(), 1);
    }

    #[tokio::test]
    async fn prepare_requires_a_data_dir_and_the_binaries() {
        let r = runner();
        let mut l = launch(MysqlSeries::DEFAULT);
        l.data_dir = None;
        assert!(matches!(r.prepare(&l).await, Err(PortError::InvalidData { .. })));
        let dir = tempfile::tempdir().unwrap();
        let l = ServiceLaunch { install_dir: dir.path().join("missing"), data_dir: Some(dir.path().join("data")), ..launch(MysqlSeries::DEFAULT) };
        assert!(matches!(r.prepare(&l).await, Err(PortError::NotFound { .. })));
    }

    #[test]
    fn error_lines_prefer_errors_since_the_marker() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("error.log");
        std::fs::write(&log, "2026-01-01T00:00:00Z 0 [ERROR] [MY-1] old failure\n").unwrap();
        let marker = file_len(&log);
        let mut f = std::fs::OpenOptions::new().append(true).open(&log).unwrap();
        std::io::Write::write_all(
            &mut f,
            b"2026-01-02T00:00:00Z 0 [System] starting\n2026-01-02T00:00:01Z 0 [ERROR] [MY-010262] [Server] Can't start server: Bind on TCP/IP port: Address already in use\n",
        )
        .unwrap();
        let lines = error_lines_since(&log, marker);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("Bind on TCP/IP port"));
    }
}
