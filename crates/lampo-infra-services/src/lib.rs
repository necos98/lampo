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
//! | [`MysqlCliAdmin`]            | `DatabaseAdmin` (compressed dumps via [`compression`]), `MysqlAccountAdmin` (root password, project accounts) |
//! | [`DpapiMysqlPasswords`]      | `MysqlRootPasswordReader`/`Writer`, `MysqlProjectPasswordReader`/`Writer` (DPAPI, current user) |
//! | [`SystemRandomPasswords`]    | `MysqlPasswordGenerator` (`BCryptGenRandom`) |
//! | [`NamedPipeControlServer`]   | `ControlChannelServer`                       |
//! | [`NamedPipeControlClient`]   | `ControlChannelClient`                       |
//! | [`HeadlessAppLauncher`]      | `AppLauncher`                                |
//! | [`TokioTimer`]               | `Timer`                                      |
//! | [`ClaudeCliRunner`]          | `ClaudeCodeCli` (`claude` via the `CommandRunner`) |
//! | [`WindowsExternalAppLauncher`] | `ExternalAppLauncher` (PowerShell, Explorer, VS Code, browser) |
//!
//! [`ClientOptionFile`] hands a stored password to the MySQL clients (a user-only temporary
//! option file, never the command line): `MysqlCliAdmin`, `mysqladmin shutdown` and the shims.
//!
//! Internal modules: `process` (process creation inside the job, IOCP stdio pipes),
//! `dump_input` (a piped command's input file, decoded and rewritten on the way),
//! `service_host` (behind [`ServiceHost`]), `config_path` (paths inside `my.ini` and `redis.conf`),
//! `rotating_log` (captured service output, rotated by size while it is written),
//! `startup_deadline` (when a slow but progressing start gives up), `last_resort` (synchronous
//! clean MySQL shutdown when the app goes away without the graceful stop),
//! `win` (thin Win32 wrappers). Windows only.

#[cfg(not(windows))]
compile_error!("lampo-infra-services targets Windows only");

pub mod claude_code;
pub mod client_option_file;
pub mod compression;
mod config_path;
pub mod control_pipe;
pub mod database;
mod dump_input;
mod external_apps;
mod last_resort;
pub mod launcher;
pub mod log_export;
pub mod logs;
pub mod mailpit;
pub mod mysql;
pub mod mysql_passwords;
pub mod password_generator;
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
pub use log_export::FileLogExportWriter;
pub use logs::FileLogReader;
pub use mailpit::MailpitRunner;
pub use mysql::MysqlRunner;
pub use mysql_passwords::DpapiMysqlPasswords;
pub use password_generator::SystemRandomPasswords;
pub use client_option_file::ClientOptionFile;
pub use ports::TcpPortProbe;
pub use redis::RedisRunner;
pub use rotating_log::SizeLogRotator;
pub use service_host::{ServiceDriver, ServiceHost};
pub use supervisor::JobObjectSupervisor;
pub use timer::TokioTimer;
