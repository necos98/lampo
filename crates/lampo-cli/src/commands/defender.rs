//! `defender exclusion add|remove [--php] [--mysql]`: runs in this process (the UAC prompt belongs
//! to the terminal), never in the app, which owns no state of it.

use lampo_application::dto::{DefenderExclusionRequestDto, ScanExclusionChangeDto, ScanExclusionOutcomeDto};
use lampo_application::{AppResult, CallContext};

use super::Cmd;
use crate::cli::{DefenderCommand, ExclusionCommand};
use crate::exit_code::ExitCode;
use crate::render;

pub async fn run(c: &Cmd<'_>, command: DefenderCommand) -> AppResult<i32> {
    match command {
        DefenderCommand::Exclusion(command) => exclusion(c, command).await,
    }
}

async fn exclusion(c: &Cmd<'_>, command: ExclusionCommand) -> AppResult<i32> {
    let (change, targets) = match command {
        ExclusionCommand::Add(targets) => (ScanExclusionChangeDto::Add, targets),
        ExclusionCommand::Remove(targets) => (ScanExclusionChangeDto::Remove, targets),
    };
    let request = DefenderExclusionRequestDto { change, php: targets.php, mysql: targets.mysql };
    let result = c.ctx.scan_exclusion.change(request, CallContext::cli(c.g.allow_elevation)).await?;
    c.io.emit(&result, render::defender_exclusion);
    Ok(exit_code(result.outcome).code())
}

/// Declined prompt: as for the hosts file. Defender not doing it: an external program failed.
pub(super) fn exit_code(outcome: ScanExclusionOutcomeDto) -> ExitCode {
    match outcome {
        ScanExclusionOutcomeDto::Applied => ExitCode::Success,
        ScanExclusionOutcomeDto::Cancelled => ExitCode::ElevationDenied,
        ScanExclusionOutcomeDto::DefenderUnavailable | ScanExclusionOutcomeDto::Refused | ScanExclusionOutcomeDto::NotConfirmed => ExitCode::External,
    }
}
