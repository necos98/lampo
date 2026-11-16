//! Mailpit: `mailpit.exe --smtp 127.0.0.1:<port> --listen 127.0.0.1:<ui_port> --database
//! <data>\mailpit.db …`, configured entirely on the command line (no config file). Loopback
//! only, any SMTP credentials accepted (frameworks that always send a username keep working),
//! no update check, no reverse DNS. The database in Lampo's data dir keeps the messages across
//! restarts; the log goes to stdout (captured into `output.log`). Readiness is the SMTP `220`
//! greeting plus the web UI accepting connections. Mailpit has no shutdown command: stopping
//! terminates the process (SQLite's journal keeps the database consistent).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use lampo_domain::ports::{CommandSpec, EventPublisher, ProcessSupervisor, ServiceRunner};
use lampo_domain::service::{ServiceId, ServiceKind, ServiceLaunch, ServiceStatus};
use lampo_domain::tcp_port::Port;
use lampo_domain::PortResult;
use lampo_infra_fs::ensure_dir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::service_host::{ServiceDriver, ServiceHost, find_executable};

/// The web UI port when the launch has none (Mailpit always listens on one).
const DEFAULT_UI_PORT: Port = Port::of(8025);

/// Where the messages are stored.
fn database_file(launch: &ServiceLaunch) -> PathBuf {
    launch.data_dir.clone().unwrap_or_else(|| launch.config_dir.join("data")).join("mailpit.db")
}

/// The command-line arguments for `launch` (pure).
pub fn mailpit_args(launch: &ServiceLaunch) -> Vec<String> {
    let ui = launch.ui_port.unwrap_or(DEFAULT_UI_PORT);
    vec![
        "--smtp".into(),
        format!("127.0.0.1:{}", launch.port),
        "--listen".into(),
        format!("127.0.0.1:{ui}"),
        "--database".into(),
        database_file(launch).display().to_string(),
        "--smtp-auth-accept-any".into(),
        "--smtp-auth-allow-insecure".into(),
        "--smtp-disable-rdns".into(),
        "--disable-version-check".into(),
    ]
}

/// The SMTP greeting (`220 …`), or `None` when nothing answered in time.
async fn smtp_greeting(port: Port) -> Option<String> {
    let attempt = async {
        let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port.get())).await.ok()?;
        let mut buf = [0u8; 256];
        let n = s.read(&mut buf).await.ok()?;
        let _ = s.write_all(b"QUIT\r\n").await;
        Some(String::from_utf8_lossy(&buf[..n]).into_owned())
    };
    tokio::time::timeout(Duration::from_secs(3), attempt).await.ok().flatten()
}

async fn accepts(port: Port) -> bool {
    tokio::time::timeout(Duration::from_secs(3), tokio::net::TcpStream::connect(("127.0.0.1", port.get()))).await.is_ok_and(|r| r.is_ok())
}

struct MailpitDriver;

#[async_trait]
impl ServiceDriver for MailpitDriver {
    fn program(&self) -> &'static str {
        "mailpit"
    }

    fn command(&self, launch: &ServiceLaunch) -> PortResult<CommandSpec> {
        let exe = find_executable(&launch.install_dir, "mailpit.exe")?;
        Ok(CommandSpec { working_dir: Some(launch.config_dir.clone()), program: exe, args: mailpit_args(launch), env: Vec::new(), path_prepend: Vec::new() })
    }

    fn startup_timeout(&self) -> Duration {
        Duration::from_secs(20)
    }

    /// Nothing to wait for: there is no graceful shutdown request.
    fn stop_grace(&self) -> Duration {
        Duration::ZERO
    }

    async fn probe(&self, launch: &ServiceLaunch) -> bool {
        smtp_greeting(launch.port).await.is_some_and(|g| g.starts_with("220")) && accepts(launch.ui_port.unwrap_or(DEFAULT_UI_PORT)).await
    }

    async fn request_shutdown(&self, _launch: &ServiceLaunch, _pid: u32) {}
}

pub struct MailpitRunner {
    host: ServiceHost,
    driver: MailpitDriver,
}

impl MailpitRunner {
    pub fn new(supervisor: Arc<dyn ProcessSupervisor>, events: Arc<dyn EventPublisher>) -> Self {
        Self { host: ServiceHost::new(supervisor, events), driver: MailpitDriver }
    }
}

#[async_trait]
impl ServiceRunner for MailpitRunner {
    fn kind(&self) -> ServiceKind {
        ServiceKind::MAILPIT
    }

    async fn prepare(&self, launch: &ServiceLaunch) -> PortResult<()> {
        find_executable(&launch.install_dir, "mailpit.exe")?;
        ensure_dir(&launch.config_dir)?;
        ensure_dir(&launch.log_dir)?;
        let db = database_file(launch);
        ensure_dir(db.parent().expect("the database file has a folder"))
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
    use std::path::Path;

    fn launch(install_dir: PathBuf) -> ServiceLaunch {
        ServiceLaunch {
            service: ServiceId::new(ServiceKind::MAILPIT, None),
            install_dir,
            port: Port::of(2025),
            ui_port: Some(Port::of(9025)),
            config_dir: r"C:\gen\mailpit".into(),
            data_dir: Some(r"C:\Data Dir\mailpit\data".into()),
            log_dir: r"C:\logs\mailpit".into(),
        }
    }

    #[test]
    fn args_bind_loopback_on_both_ports_and_persist_to_the_data_dir() {
        let args = mailpit_args(&launch(r"C:\pk\mailpit".into()));
        assert_eq!(
            args,
            [
                "--smtp",
                "127.0.0.1:2025",
                "--listen",
                "127.0.0.1:9025",
                "--database",
                r"C:\Data Dir\mailpit\data\mailpit.db",
                "--smtp-auth-accept-any",
                "--smtp-auth-allow-insecure",
                "--smtp-disable-rdns",
                "--disable-version-check",
            ]
        );
        let defaults = mailpit_args(&ServiceLaunch { ui_port: None, data_dir: None, ..launch(r"C:\pk".into()) });
        assert!(defaults.contains(&"127.0.0.1:8025".to_string()), "{defaults:?}");
        assert!(defaults.contains(&r"C:\gen\mailpit\data\mailpit.db".to_string()), "{defaults:?}");
    }

    #[test]
    fn command_runs_the_packaged_executable() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("mailpit.exe"), b"").unwrap();
        let spec = MailpitDriver.command(&launch(dir.path().to_path_buf())).unwrap();
        assert_eq!(spec.program, dir.path().join("mailpit.exe"));
        assert_eq!(spec.working_dir.as_deref(), Some(Path::new(r"C:\gen\mailpit")));
        assert!(MailpitDriver.command(&launch(dir.path().join("missing"))).is_err());
    }
}
