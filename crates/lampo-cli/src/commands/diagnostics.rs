//! `doctor`, `agent context`, `shims`.

use lampo_application::control::ControlRequest;
use lampo_application::dto::{AgentContextDto, StatusDto};
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
        ShimsCommand::Uninstall => c.ctx.shims.uninstall()?,
    };
    c.io.emit(&status, render::shim_status);
    Ok(0)
}

/// How long the hook waits for the running app's live service states.
const HOOK_STATUS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// `lampo agent context --hook` (Claude Code `SessionStart`): read-only, never starts the app.
/// Inside a Lampo project or worktree prints the hook JSON with a compact summary; otherwise
/// prints nothing. The caller turns every error into exit 0 without output.
pub async fn agent_context_hook(c: &Cmd<'_>) -> AppResult<i32> {
    let mut request = c.resolve_request();
    if let Some(cwd) = c.ctx.hook_input.read().as_deref().and_then(hook_cwd) {
        request.cwd = cwd;
    }
    let context = c.ctx.agent.context(&request)?;
    if context.project_id.is_none() {
        return Ok(0);
    }
    // Live states only from an app that is already running (never started from a hook).
    let services = if context.app_running {
        match tokio::time::timeout(HOOK_STATUS_TIMEOUT, c.app_if_running::<StatusDto>(ControlRequest::Status)).await {
            Ok(Ok(status)) => Some(status.services),
            _ => None,
        }
    } else {
        None
    };
    let summary = render::session_context(&context, services.as_deref());
    c.io.out_line(&lampo_domain::claude_code::session_start_output(&summary));
    Ok(0)
}

/// The `cwd` of a hook payload (`{"cwd": "C:\work\shop", ...}`), if any.
fn hook_cwd(payload: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(payload.trim()).ok()?;
    value.get("cwd")?.as_str().map(str::to_string).filter(|c| !c.trim().is_empty())
}
