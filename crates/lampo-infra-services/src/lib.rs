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
//! | [`TcpPortProbe`]             | `PortProbe`                                  |
//! | [`FileLogReader`]            | `LogFileReader`                              |
//! | [`MysqlCliAdmin`]            | `DatabaseAdmin` (compressed dumps via [`compression`]) |
//! | [`NamedPipeControlServer`]   | `ControlChannelServer`                       |
//! | [`NamedPipeControlClient`]   | `ControlChannelClient`                       |
//! | [`HeadlessAppLauncher`]      | `AppLauncher`                                |
//! | [`TokioTimer`]               | `Timer`                                      |
//! | [`ClaudeCliRunner`]          | `ClaudeCodeCli` (`claude` via the `CommandRunner`) |
//!
//! Internal modules: `process` (process creation inside the job, IOCP stdio pipes),
//! `service_host` (what every supervised runner shares; runners only describe their service),
//! `win` (thin Win32 wrappers). Windows only.

#[cfg(not(windows))]
compile_error!("lampo-infra-services targets Windows only");

pub mod claude_code;
pub mod compression;
pub mod control_pipe;
pub mod database;
pub mod launcher;
pub mod logs;
pub mod mailpit;
pub mod mysql;
pub mod ports;
mod process;
pub mod redis;
mod service_host;
pub mod supervisor;
pub mod timer;
mod win;

pub use claude_code::ClaudeCliRunner;
pub use control_pipe::{NamedPipeControlClient, NamedPipeControlServer};
pub use database::MysqlCliAdmin;
pub use launcher::HeadlessAppLauncher;
pub use logs::FileLogReader;
pub use mailpit::MailpitRunner;
pub use mysql::MysqlRunner;
pub use ports::TcpPortProbe;
pub use redis::RedisRunner;
pub use supervisor::JobObjectSupervisor;
pub use timer::TokioTimer;
