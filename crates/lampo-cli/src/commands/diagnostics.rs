//! `doctor`, `agent context`, `shims`.

use lampo_application::dto::AgentContextDto;
use lampo_application::AppResult;
use serde::Serialize;

use super::Cmd;
use crate::cli::ShimsCommand;
use crate::exit_code::ExitCode;
use crate::render;

/// Exit 14 when the report is not `ok`; the report is still the command's result.
pub async fn doctor(c: &Cmd<'_>) -> AppResult<i32> {
    let report = c.ctx.doctor.run().await?;
    c.io.emit(&report, render::doctor);
    Ok(if report.ok { ExitCode::Success } else { ExitCode::DoctorProblems }.code())
}

/// The application's context plus a `cli` object (exit codes, JSON error shape, rules) so that
/// one call tells an agent everything. The extra key is additive to the stable DTO.
#[derive(Debug, Serialize)]
pub struct AgentContextOutput {
    #[serde(flatten)]
    pub context: AgentContextDto,
    pub cli: serde_json::Value,
}

pub fn agent_context(c: &Cmd<'_>) -> AppResult<i32> {
    let context = c.ctx.agent.context(&c.resolve_request())?;
    let output = AgentContextOutput { context, cli: render::cli_guide() };
    c.io.emit(&output, |o| render::agent_context(&o.context));
    Ok(0)
}

pub fn shims(c: &Cmd<'_>, command: ShimsCommand) -> AppResult<i32> {
    let status = match command {
        ShimsCommand::Status => c.ctx.shims.status()?,
        ShimsCommand::Install => c.ctx.shims.install()?,
    };
    c.io.emit(&status, render::shim_status);
    Ok(0)
}
