//! # lampo-infra-services
//!
//! Processes and local IPC on Windows. One reason to change: *how Lampo runs and talks to local
//! processes* (Job Objects, MySQL/Redis/Mailpit command lines, TCP ports, log files, the control pipe).
//!
//! | Adapter                      | Ports                                        |
//! |------------------------------|----------------------------------------------|
//! | [`JobObjectSupervisor`]      | `ProcessSupervisor`, `CommandRunner`, `PipedCommandRunner`, `StreamingCommandRunner` (nested job per run: cancel kills the tree) |
//! | [`MysqlRunner`]              | `ServiceRunner` for `ServiceKind::Mysql`     |
//! | [`RedisRunner`]              | `ServiceRunner` for `ServiceKind::Redis`     |
//! | [`MailpitRunner`]            | `ServiceRunner` for `ServiceKind::MAILPIT`   |
//! | [`ServiceHost`]              | what every `ServiceRunner` shares; a runner describes its service with a [`ServiceDriver`] (also used by `lampo-infra-web` for Apache) |
//! | [`TcpPortProbe`]             | `PortProbe`                                  |
//! | [`FileLogReader`]            | `LogFileReader`                              |
//! | [`SizeLogRotator`]           | `LogRotator`                                 |
//! | [`MysqlCliAdmin`]            | `DatabaseAdmin` (compressed dumps via [`compression`]) |
//! | [`NamedPipeControlServer`]   | `ControlChannelServer`                       |
//! | [`NamedPipeControlClient`]   | `ControlChannelClient`                       |
//! | [`HeadlessAppLauncher`]      | `AppLauncher`                                |
//! | [`TokioTimer`]               | `Timer`                                      |
//! | [`ClaudeCliRunner`]          | `ClaudeCodeCli` (`claude` via the `CommandRunner`) |
//! | [`WindowsExternalAppLauncher`] | `ExternalAppLauncher` (PowerShell, Explorer, VS Code) |
//!
//! Internal modules: `process` (process creation inside the job, IOCP stdio pipes),
//! `service_host` (behind [`ServiceHost`]),
//! `rotating_log` (captured service output, rotated by size while it is written),
//! `startup_deadline` (when a slow but progressing start gives up), `last_resort` (synchronous
//! clean MySQL shutdown when the app goes away without the graceful stop),
//! `win` (thin Win32 wrappers). Windows only.

#[cfg(not(windows))]
compile_error!("lampo-infra-services targets Windows only");

pub mod claude_code;
pub mod compression;
pub mod control_pipe;
pub mod database;
mod external_apps;
mod last_resort;
pub mod launcher;
pub mod logs;
pub mod mailpit;
pub mod mysql;
pub mod ports;
mod process;
pub mod redis;
mod rotating_log;
mod service_host;
#[cfg(test)]
mod service_host_tests;
mod startup_deadline;
pub mod supervisor;
pub mod timer;
mod win;

pub use claude_code::ClaudeCliRunner;
pub use control_pipe::{NamedPipeControlClient, NamedPipeControlServer};
pub use database::MysqlCliAdmin;
pub use external_apps::WindowsExternalAppLauncher;
pub use last_resort::{LastResortStop, stop_mysql_servers};
pub use launcher::HeadlessAppLauncher;
pub use logs::FileLogReader;
pub use mailpit::MailpitRunner;
pub use mysql::MysqlRunner;
pub use ports::TcpPortProbe;
pub use redis::RedisRunner;
pub use rotating_log::SizeLogRotator;
pub use service_host::{ServiceDriver, ServiceHost};
pub use supervisor::JobObjectSupervisor;
pub use timer::TokioTimer;
