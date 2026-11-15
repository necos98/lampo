//! Maps parsed commands to ports: argument -> DTO, call, DTO -> output. No business logic.
//!
//! Read-only commands call the local role traits; commands that change state go through
//! [`Cmd::app`], which starts the app headless unless `--no-start`.

mod db;
mod diagnostics;
mod packages;
mod projects;
mod services;
mod toolchain;
mod worktrees;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use lampo_application::control::ControlRequest;
use lampo_application::dto::{ResolveRequestDto, ResolvedSiteDto};
use lampo_application::{AppError, AppResult, CallContext, ErrorCode};
use serde::de::DeserializeOwned;

use crate::cli::*;
use crate::context::CliContext;
use crate::io::Io;
use crate::progress::ProgressPrinter;

/// Everything a handler needs: the ports, the global flags and the output.
pub struct Cmd<'a> {
    pub ctx: &'a CliContext,
    pub g: &'a GlobalArgs,
    pub io: &'a Io,
}

impl Cmd<'_> {
    pub fn resolve_request(&self) -> ResolveRequestDto {
        self.ctx.env.resolve_request(self.g)
    }

    pub fn resolved_site(&self) -> AppResult<ResolvedSiteDto> {
        self.ctx.sites.resolve(&self.resolve_request())
    }

    /// The site (`shop`, `shop@fix-1`) the command acts on; `ProjectNotResolved` for the global
    /// default (it has no database, URL or logs of its own).
    pub fn site_id(&self) -> AppResult<String> {
        self.resolved_site()?.site.ok_or_else(|| AppError::new(ErrorCode::ProjectNotResolved, "this folder is not inside a Lampo project (only the global default PHP applies)"))
    }

    /// An explicit project id, else the project of the resolved site.
    pub fn project_id(&self, explicit: Option<&String>) -> AppResult<String> {
        if let Some(id) = explicit {
            return Ok(id.clone());
        }
        self.resolved_site()?.project_id.ok_or_else(|| AppError::new(ErrorCode::ProjectNotResolved, "no project for this folder: pass the project id or use --project"))
    }

    /// Sends a request to the running app and decodes the payload. Starts the app headless first
    /// unless `--no-start` (then `AppUnavailable`, exit 9). Progress goes to stderr.
    pub async fn app<T: DeserializeOwned>(&self, request: ControlRequest) -> AppResult<T> {
        let gateway = &self.ctx.app;
        if !gateway.is_running() {
            if self.g.no_start {
                return Err(AppError::new(ErrorCode::AppUnavailable, "the Lampo app is not running and --no-start was given"));
            }
            gateway.ensure_running().await?;
        }
        self.send(request).await
    }

    /// Like [`Cmd::app`] but never starts the app.
    pub async fn app_if_running<T: DeserializeOwned>(&self, request: ControlRequest) -> AppResult<T> {
        self.send(request).await
    }

    async fn send<T: DeserializeOwned>(&self, request: ControlRequest) -> AppResult<T> {
        let printer = Arc::new(ProgressPrinter::new(self.io.err_sink()));
        let on_progress = {
            let printer = printer.clone();
            move |p: lampo_application::events::ProgressDto| printer.report(&p)
        };
        let payload = self.ctx.app.send(request, CallContext::cli(self.g.allow_elevation), &on_progress).await?;
        serde_json::from_value(payload).map_err(|e| AppError::new(ErrorCode::ProtocolMismatch, format!("unexpected reply from the app: {e}")))
    }
}

/// Runs one command and returns the process exit code.
pub async fn run(cli: Cli, ctx: &CliContext, io: &Io) -> AppResult<i32> {
    let cmd = Cmd { ctx, g: &cli.global, io };
    match cli.command {
        Command::Status => services::status(&cmd).await,
        Command::Project(c) => projects::run(&cmd, c).await,
        Command::Worktree(c) => worktrees::run(&cmd, c).await,
        Command::Up(a) => services::control(&cmd, services::Action::Up, a).await,
        Command::Down(a) => services::control(&cmd, services::Action::Down, a).await,
        Command::Restart(a) => services::control(&cmd, services::Action::Restart, a).await,
        Command::Quit => services::quit(&cmd).await,
        Command::Logs(a) => services::logs(&cmd, a).await,
        Command::Package(c) => packages::run(&cmd, c).await,
        Command::Db(c) => db::run(&cmd, c).await,
        Command::Exec(a) => toolchain::exec(&cmd, a),
        Command::Env(a) => toolchain::env(&cmd, a),
        Command::Doctor => diagnostics::doctor(&cmd).await,
        Command::Agent(AgentCommand::Context(a)) if a.hook => diagnostics::agent_context_hook(&cmd).await,
        Command::Agent(AgentCommand::Context(_)) => diagnostics::agent_context(&cmd),
        Command::Shims(c) => diagnostics::shims(&cmd, c),
    }
}

pub(crate) fn validation(message: impl Into<String>) -> AppError {
    AppError::new(ErrorCode::Validation, message)
}
